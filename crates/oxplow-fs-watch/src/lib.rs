//! Debounced filesystem watcher.
//!
//! Wraps `notify::RecommendedWatcher` and exposes a single broadcast
//! channel of `WatchEvent`s with built-in debouncing. Reused by
//! oxplow-git for `.git/refs` watching, and (in a future pass) by
//! the analysis pipeline.
//!
//! Replaces the `chokidar` usage scattered through the TS codebase.
//! The contract:
//!   - Subscribers see at most one event per (path, kind) pair within
//!     the debounce window.
//!   - The watcher is cancelled by dropping the `FsWatcher` handle —
//!     internal threads exit cleanly, no zombie threads.

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::RecommendedWatcher;
pub use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Debug, Error)]
pub enum FsWatchError {
    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),
}

/// What happened to a watched path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEventKind {
    Created,
    Modified,
    Removed,
    /// `notify` couldn't classify the event but the path is implicated.
    Other,
}

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: WatchEventKind,
}

/// Active filesystem watcher.
///
/// Hold this for as long as you want to watch. Drop it to cancel.
pub struct FsWatcher {
    // Holding the debouncer alive keeps the watcher running. Drop
    // releases all OS handles.
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    sender: broadcast::Sender<WatchEvent>,
}

impl FsWatcher {
    /// Watch `path` recursively, debouncing events within `debounce_window`.
    pub fn watch(path: impl AsRef<Path>, debounce_window: Duration) -> Result<Self, FsWatchError> {
        Self::watch_paths(
            vec![(path.as_ref().to_path_buf(), RecursiveMode::Recursive)],
            debounce_window,
        )
    }

    /// Watch a set of paths with per-path recursion modes, debouncing
    /// events within `debounce_window`. A single OS-level debouncer is
    /// shared across every entry, so an event on any registered path
    /// flows through the same broadcast channel.
    pub fn watch_paths(
        paths: Vec<(PathBuf, RecursiveMode)>,
        debounce_window: Duration,
    ) -> Result<Self, FsWatchError> {
        let (tx, _) = broadcast::channel(256);
        let tx_clone = tx.clone();

        let mut debouncer =
            new_debouncer(debounce_window, None, move |res: DebounceEventResult| {
                let Ok(events) = res else { return };
                for evt in events {
                    let kind = classify(&evt.event);
                    for path in evt.event.paths.iter() {
                        let _ = tx_clone.send(WatchEvent {
                            path: path.clone(),
                            kind: kind.clone(),
                        });
                    }
                }
            })?;

        for (p, mode) in paths {
            debouncer.watch(&p, mode)?;
        }

        Ok(Self {
            _debouncer: debouncer,
            sender: tx,
        })
    }

    /// Subscribe to events. Subscribers can connect at any time; events
    /// emitted before subscription are dropped.
    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.sender.subscribe()
    }
}

fn classify(event: &notify::Event) -> WatchEventKind {
    use notify::EventKind;
    match event.kind {
        EventKind::Create(_) => WatchEventKind::Created,
        EventKind::Modify(_) => WatchEventKind::Modified,
        EventKind::Remove(_) => WatchEventKind::Removed,
        _ => WatchEventKind::Other,
    }
}

/// Path segments that ALWAYS trigger a watch-ignore, regardless of
/// user config. Limited to `.git` — git's internal write spool
/// churns faster than we can capture and produces ephemeral
/// lock/tmp files that always race the watcher to a NotFound, so
/// watching it is never useful. Everything else (build outputs,
/// language caches, IDE state) is the user's call via the
/// `generated` config; we don't pretend to know which dirs each
/// project actually treats as generated. `.oxplow` is handled
/// separately above with a `.oxplow/wiki/` carve-out, not via this
/// segment list.
const DEFAULT_IGNORED_SEGMENTS: &[&str] = &[".git"];

/// Workspace-relative path filter. Constructed once at app startup
/// from the project's `generated` config and shared (by value clone —
/// the entry vec is short) across snapshot capture, code-quality
/// scans, fs-watch consumers, etc.
///
/// Match semantics:
/// - A **single-segment entry** (no `/`) matches if any path component
///   equals it. So `target` filters `target/`, `crates/foo/target/`,
///   etc. Mirrors the legacy hardcoded behavior for build dirs.
/// - A **multi-segment entry** (contains `/`) matches the path exactly
///   OR as a prefix (`apps/desktop/dist` filters that directory and
///   everything under it, but NOT `crates/foo/apps/desktop/dist`).
///
/// The always-on `.git` ignore and `.oxplow/` (with `.oxplow/wiki/`
/// carve-out) handling apply regardless of user config; everything
/// else — build outputs, IDE state, language caches — must be
/// listed in `generated` explicitly.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceFilter {
    user_entries: Vec<FilterEntry>,
}

#[derive(Debug, Clone)]
enum FilterEntry {
    Segment(String),
    Path(PathBuf),
}

impl WorkspaceFilter {
    /// Build a filter from the user's `generated` config list.
    /// Entries may be a single dir/file name (matches anywhere) or a
    /// repo-relative path (matches that exact path + everything
    /// under it).
    pub fn with_user_entries<I, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let user_entries = entries
            .into_iter()
            .filter_map(|raw| {
                let trimmed = raw.as_ref().trim().trim_matches('/');
                if trimmed.is_empty() {
                    return None;
                }
                if trimmed.contains('/') {
                    Some(FilterEntry::Path(PathBuf::from(trimmed)))
                } else {
                    Some(FilterEntry::Segment(trimmed.to_string()))
                }
            })
            .collect();
        Self { user_entries }
    }

    /// True if `path` (workspace-relative) should be ignored.
    pub fn ignore(&self, path: &Path) -> bool {
        use std::path::Component;

        // Always-on defaults: walk components, match by segment with
        // the `.oxplow/wiki/` carve-out.
        let mut comps = path.components().peekable();
        while let Some(c) = comps.next() {
            if let Component::Normal(seg) = c {
                let s = match seg.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                if s == ".oxplow" {
                    let next = comps.peek().and_then(|c| match c {
                        Component::Normal(n) => n.to_str(),
                        _ => None,
                    });
                    if next == Some("wiki") {
                        return false;
                    }
                    return true;
                }
                if DEFAULT_IGNORED_SEGMENTS.contains(&s) {
                    return true;
                }
            }
        }

        // User entries: segments match any component, paths match
        // exact-or-prefix.
        for entry in &self.user_entries {
            match entry {
                FilterEntry::Segment(seg) => {
                    if path.components().any(
                        |c| matches!(c, Component::Normal(n) if n.to_str() == Some(seg.as_str())),
                    ) {
                        return true;
                    }
                }
                FilterEntry::Path(p) => {
                    if path == p.as_path() || path.starts_with(p) {
                        return true;
                    }
                }
            }
        }

        false
    }
}

/// Default-only shorthand for callers that don't have a configured
/// filter handy (e.g. the snapshot-sweep example binary). Equivalent
/// to `WorkspaceFilter::default().ignore(path)` — applies the always-
/// on defaults only, no user entries.
pub fn should_ignore_workspace_watch_path(path: &Path) -> bool {
    WorkspaceFilter::default().ignore(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::timeout;

    #[tokio::test]
    async fn detects_new_file() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path(), Duration::from_millis(50)).unwrap();
        let mut rx = watcher.subscribe();

        let target = dir.path().join("hello.txt");
        std::fs::write(&target, b"hello").unwrap();

        let evt = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("event")
            .expect("event ok");
        // macOS resolves /var/folders to /private/var/folders — compare
        // canonicalized paths so the test is stable across platforms.
        assert_eq!(
            evt.path.canonicalize().unwrap(),
            target.canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn debounces_rapid_writes() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path(), Duration::from_millis(150)).unwrap();
        let mut rx = watcher.subscribe();

        let target = dir.path().join("hot.txt");
        for i in 0..20 {
            std::fs::write(&target, format!("{i}")).unwrap();
        }

        // Collect everything that lands within 1s. The debouncer
        // should coalesce 20 writes into a small number of events.
        let mut count = 0;
        while let Ok(Ok(_)) = timeout(Duration::from_millis(800), rx.recv()).await {
            count += 1;
        }

        assert!(count > 0, "expected at least one event");
        assert!(count < 20, "expected debouncing, got {count} events");
    }

    #[tokio::test]
    async fn watch_paths_registers_multiple_dirs() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let watcher = FsWatcher::watch_paths(
            vec![
                (a.path().to_path_buf(), RecursiveMode::Recursive),
                (b.path().to_path_buf(), RecursiveMode::Recursive),
            ],
            Duration::from_millis(50),
        )
        .unwrap();
        let mut rx = watcher.subscribe();

        let ta = a.path().join("a.txt");
        let tb = b.path().join("b.txt");
        std::fs::write(&ta, b"a").unwrap();
        std::fs::write(&tb, b"b").unwrap();

        let mut saw_a = false;
        let mut saw_b = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline && !(saw_a && saw_b) {
            if let Ok(Ok(evt)) = timeout(Duration::from_millis(500), rx.recv()).await {
                let p = evt.path.canonicalize().unwrap_or(evt.path.clone());
                if p == ta.canonicalize().unwrap() {
                    saw_a = true;
                }
                if p == tb.canonicalize().unwrap() {
                    saw_b = true;
                }
            }
        }
        assert!(saw_a, "expected event for {ta:?}");
        assert!(saw_b, "expected event for {tb:?}");
    }

    #[tokio::test]
    async fn non_recursive_only_reports_top_level() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("nested");
        std::fs::create_dir(&sub).unwrap();

        let watcher = FsWatcher::watch_paths(
            vec![(dir.path().to_path_buf(), RecursiveMode::NonRecursive)],
            Duration::from_millis(50),
        )
        .unwrap();
        let mut rx = watcher.subscribe();

        // Write into a subdir — should NOT show up under non-recursive.
        let nested = sub.join("hidden.txt");
        std::fs::write(&nested, b"x").unwrap();

        // Drain for ~600ms; if we see the nested path, fail.
        let drain_deadline = std::time::Instant::now() + Duration::from_millis(600);
        let mut nested_seen = false;
        while std::time::Instant::now() < drain_deadline {
            match timeout(Duration::from_millis(150), rx.recv()).await {
                Ok(Ok(evt)) => {
                    let canon = evt.path.canonicalize().unwrap_or(evt.path.clone());
                    if canon == nested.canonicalize().unwrap() {
                        nested_seen = true;
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(
            !nested_seen,
            "non-recursive watch should not surface nested writes"
        );

        // Top-level write should be reported.
        let top = dir.path().join("top.txt");
        std::fs::write(&top, b"y").unwrap();
        let mut top_seen = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(evt)) => {
                    let canon = evt.path.canonicalize().unwrap_or(evt.path.clone());
                    if canon == top.canonicalize().unwrap() {
                        top_seen = true;
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(top_seen, "expected top-level event for {top:?}");
    }

    #[test]
    fn should_ignore_filters_oxplow_and_git() {
        // Always-on: anything under `.oxplow/` except `.oxplow/wiki/`,
        // and anything under `.git/`. Everything else is the user's
        // call via the `generated` config.
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".oxplow/snapshots/aa/foo.tmp"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            ".oxplow/wiki/local-snapshots.md"
        )));
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".oxplow/state.sqlite"
        )));
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".git/index.lock"
        )));
        // Build dirs are NOT default-ignored; users opt in via `generated`.
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "target/debug/x.bin"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "node_modules/foo/index.js"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "src/main.rs"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "docs/README.md"
        )));
    }

    #[test]
    fn workspace_filter_user_segment_matches_anywhere() {
        let f = WorkspaceFilter::with_user_entries([".idea"]);
        assert!(f.ignore(Path::new(".idea/workspace.xml")));
        assert!(f.ignore(Path::new("nested/.idea/foo")));
        assert!(!f.ignore(Path::new("src/main.rs")));
    }

    #[test]
    fn workspace_filter_user_path_matches_prefix_only() {
        // Use a path entry whose segments don't collide with the
        // always-on defaults (no `dist`, `target`, etc.). That lets
        // us isolate the "matches prefix only" semantics.
        let f = WorkspaceFilter::with_user_entries(["apps/desktop/generated"]);
        assert!(f.ignore(Path::new("apps/desktop/generated")));
        assert!(f.ignore(Path::new("apps/desktop/generated/index.js")));
        assert!(!f.ignore(Path::new("apps/desktop")));
        // Not a free-floating match — only the exact prefix counts.
        assert!(!f.ignore(Path::new("crates/foo/apps/desktop/generated/x")));
    }

    #[test]
    fn workspace_filter_user_file_path_matches_exact() {
        let f = WorkspaceFilter::with_user_entries(["docs/generated/output.txt"]);
        assert!(f.ignore(Path::new("docs/generated/output.txt")));
        assert!(!f.ignore(Path::new("docs/generated/other.txt")));
    }

    #[test]
    fn workspace_filter_defaults_apply_even_with_empty_user_list() {
        // Defaults are `.git` (segment) and `.oxplow/*` (with a
        // `.oxplow/wiki/` carve-out). Build dirs are NOT defaults —
        // they require the user to add them to `generated`.
        let f = WorkspaceFilter::default();
        assert!(f.ignore(Path::new(".git/HEAD")));
        assert!(f.ignore(Path::new("crates/foo/.git/HEAD")));
        assert!(!f.ignore(Path::new("node_modules/x")));
        assert!(!f.ignore(Path::new("target/debug")));
        assert!(!f.ignore(Path::new("src/main.rs")));
    }

    #[test]
    fn workspace_filter_oxplow_wiki_passes_through() {
        let f = WorkspaceFilter::default();
        assert!(!f.ignore(Path::new(".oxplow/wiki/page.md")));
        assert!(f.ignore(Path::new(".oxplow/state.sqlite")));
    }

    #[test]
    fn workspace_filter_empty_entries_are_dropped() {
        let f = WorkspaceFilter::with_user_entries(["", "  ", "/"]);
        assert!(!f.ignore(Path::new("foo.txt")));
    }

    #[tokio::test]
    async fn drop_cancels_watcher() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path(), Duration::from_millis(50)).unwrap();
        let mut rx = watcher.subscribe();
        drop(watcher);
        // After drop, no further events are emitted; channel closes
        // when the sender is dropped (the watcher held the only sender).
        let target = dir.path().join("ignored.txt");
        std::fs::write(&target, b"x").unwrap();
        let recv = timeout(Duration::from_millis(500), rx.recv()).await;
        // Either a timeout (the channel is silent) or a closed channel.
        match recv {
            Err(_) => {}                                       // timeout — fine
            Ok(Err(broadcast::error::RecvError::Closed)) => {} // closed — fine
            other => panic!("expected closed/timeout, got {other:?}"),
        }
    }
}
