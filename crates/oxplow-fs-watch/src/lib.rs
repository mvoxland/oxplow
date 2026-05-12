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

/// Path segments that should never trigger workspace-level watch
/// reactions (snapshot capture, indexing). These are either oxplow's
/// own state directories, git's internal write spool (which churns
/// faster than we can capture and produces ephemeral lock/tmp files
/// that always race the watcher to a NotFound), or common build /
/// cache dirs whose churn is enormous and uninteresting.
const IGNORED_WORKSPACE_SEGMENTS: &[&str] = &[
    ".git",
    ".oxplow",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    ".venv",
    "__pycache__",
];

/// True if any path component of `relative` matches an ignored
/// workspace segment. `relative` should already be made relative to
/// the workspace root; absolute paths still work but match conserva-
/// tively against the same segment list (`.git/...` anywhere in the
/// chain is treated as ignored).
///
/// Exception: paths under `.oxplow/wiki/` pass through. Wiki pages
/// are authored content and the snapshot system tracks their
/// history alongside source files. The rest of `.oxplow/`
/// (`snapshots/`, `state.sqlite*`, `runtime/`, etc.) remains
/// ignored — those churn fast and are oxplow's own internal state.
pub fn should_ignore_workspace_watch_path(path: &Path) -> bool {
    use std::path::Component;
    let mut comps = path.components().peekable();
    while let Some(c) = comps.next() {
        if let Component::Normal(seg) = c {
            let s = match seg.to_str() {
                Some(s) => s,
                None => continue,
            };
            if s == ".oxplow" {
                // Allow `.oxplow/wiki/...` through; ignore everything
                // else under `.oxplow/`.
                let next = comps.peek().and_then(|c| match c {
                    Component::Normal(n) => n.to_str(),
                    _ => None,
                });
                if next == Some("wiki") {
                    return false;
                }
                return true;
            }
            if IGNORED_WORKSPACE_SEGMENTS.contains(&s) {
                return true;
            }
        }
    }
    false
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
    fn should_ignore_filters_oxplow_git_and_build_dirs() {
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".oxplow/snapshots/aa/foo.tmp"
        )));
        // Wiki pages under .oxplow/wiki/ are tracked, not ignored.
        assert!(!should_ignore_workspace_watch_path(Path::new(
            ".oxplow/wiki/local-snapshots.md"
        )));
        // Other .oxplow/* paths stay ignored.
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".oxplow/state.sqlite"
        )));
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".git/index.lock"
        )));
        assert!(should_ignore_workspace_watch_path(Path::new(
            "target/debug/x.bin"
        )));
        assert!(should_ignore_workspace_watch_path(Path::new(
            "node_modules/foo/index.js"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "src/main.rs"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "docs/README.md"
        )));
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
