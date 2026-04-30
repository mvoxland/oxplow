//! Bridges per-stream filesystem and `.git/refs` watchers onto the
//! shared `EventBus`.
//!
//! Spawned at boot. Iterates the stream list, opens an `FsWatcher`
//! against each worktree (excluding `.git`, `node_modules`, etc. via
//! path filtering inside the bridge loop) and a `GitRefsWatcher`
//! against `<worktree>/.git/refs`. Translates the per-watcher
//! broadcasts into `OxplowEvent::WorkspaceChanged` /
//! `OxplowEvent::GitRefsChanged` so the renderer's existing
//! subscribers fire.
//!
//! Also watches the project root for `.git` appearing/disappearing
//! and emits `OxplowEvent::WorkspaceContextChanged` so the renderer
//! can flip the git-aware UI without polling.
//!
//! The registry holds the watcher handles for the lifetime of the
//! process; dropping the handle cancels the watcher.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::stores::StreamStore;
use oxplow_domain::StreamId;
use oxplow_fs_watch::{FsWatcher, WatchEvent, WatchEventKind};
use oxplow_git::GitRefsWatcher;
use tracing::{debug, warn};

use crate::events::{EventBus, OxplowEvent, WorkspaceChangeKind};

/// Holds every per-stream watcher handle. Dropping the registry
/// cancels every watcher in lockstep.
pub struct WorkspaceWatchRegistry {
    _watchers: Vec<StreamWatchers>,
    _project_watcher: Option<FsWatcher>,
}

struct StreamWatchers {
    _fs: FsWatcher,
    _refs: Option<GitRefsWatcher>,
}

impl WorkspaceWatchRegistry {
    /// Boot the registry. Looks up every existing stream, spawns a
    /// watcher pair per stream, and starts the project-root `.git`
    /// presence watcher.
    pub async fn spawn(
        streams: Arc<dyn StreamStore>,
        events: EventBus,
        project_dir: PathBuf,
    ) -> Self {
        let stream_rows = streams.list().await.unwrap_or_default();
        let mut watchers = Vec::new();
        for s in stream_rows {
            if let Some(w) = spawn_for_stream(s.id, PathBuf::from(s.worktree_path), events.clone())
            {
                watchers.push(w);
            }
        }
        let project_watcher = spawn_project_context(project_dir, events);
        Self {
            _watchers: watchers,
            _project_watcher: project_watcher,
        }
    }
}

fn spawn_for_stream(
    stream_id: StreamId,
    worktree: PathBuf,
    events: EventBus,
) -> Option<StreamWatchers> {
    if !worktree.exists() {
        debug!(?worktree, %stream_id, "skipping watcher — worktree missing");
        return None;
    }
    let fs = match FsWatcher::watch(&worktree, Duration::from_millis(250)) {
        Ok(w) => w,
        Err(e) => {
            warn!(error = %e, %stream_id, ?worktree, "fs watcher failed to start");
            return None;
        }
    };
    {
        let mut rx = fs.subscribe();
        let bus = events.clone();
        let id = stream_id.clone();
        let root = worktree.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(WatchEvent { path, kind }) => {
                        if is_uninteresting(&path) {
                            continue;
                        }
                        let rel = path
                            .strip_prefix(&root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .into_owned();
                        bus.emit(OxplowEvent::WorkspaceChanged {
                            stream_id: id.clone(),
                            change_kind: classify(&kind),
                            path: rel,
                        });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "workspace watcher lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    let refs = match GitRefsWatcher::watch(worktree.clone(), Duration::from_millis(250)) {
        Ok(w) => Some(w),
        Err(e) => {
            // A non-git worktree won't have `.git/refs/`; that's
            // fine — drop the refs watcher silently.
            debug!(error = %e, %stream_id, "git refs watcher unavailable");
            None
        }
    };
    if let Some(refs_handle) = refs.as_ref() {
        let mut rx = refs_handle.subscribe();
        let bus = events.clone();
        let id = stream_id.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(_) => bus.emit(OxplowEvent::GitRefsChanged {
                        stream_id: id.clone(),
                    }),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    Some(StreamWatchers { _fs: fs, _refs: refs })
}

/// Watch the project root for `.git` appearing or disappearing and
/// emit `WorkspaceContextChanged` so the renderer flips the git-aware
/// UI without polling. Initial state is reported on the first emit;
/// callers also `getWorkspaceContext` for the first paint.
fn spawn_project_context(project_dir: PathBuf, events: EventBus) -> Option<FsWatcher> {
    let watcher = match FsWatcher::watch(&project_dir, Duration::from_millis(500)) {
        Ok(w) => w,
        Err(e) => {
            warn!(error = %e, ?project_dir, "project context watcher failed to start");
            return None;
        }
    };
    let mut rx = watcher.subscribe();
    let mut last_state = project_dir.join(".git").exists();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(WatchEvent { path, .. }) => {
                    // Only react to events on `.git` itself.
                    let touched_git = path
                        .file_name()
                        .map(|n| n == ".git")
                        .unwrap_or(false)
                        || path
                            .components()
                            .any(|c| c.as_os_str() == ".git");
                    if !touched_git {
                        continue;
                    }
                    let now = project_dir.join(".git").exists();
                    if now != last_state {
                        last_state = now;
                        events.emit(OxplowEvent::WorkspaceContextChanged { git_enabled: now });
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    Some(watcher)
}

fn classify(k: &WatchEventKind) -> WorkspaceChangeKind {
    match k {
        WatchEventKind::Created => WorkspaceChangeKind::Created,
        WatchEventKind::Modified => WorkspaceChangeKind::Updated,
        WatchEventKind::Removed => WorkspaceChangeKind::Deleted,
        WatchEventKind::Other => WorkspaceChangeKind::Updated,
    }
}

/// Drop noisy paths the renderer never needs to react to. Mirrors the
/// `chokidar` ignore list from the renderer-era `WorkspaceWatcher`:
/// `.git/`, `node_modules/`, `target/`, `.oxplow/`, and editor swap
/// files.
fn is_uninteresting(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if s.contains("/.git/") || s.ends_with("/.git") {
        return true;
    }
    if s.contains("/node_modules/") {
        return true;
    }
    if s.contains("/target/") {
        return true;
    }
    if s.contains("/.oxplow/") {
        return true;
    }
    if s.ends_with('~') || s.ends_with(".swp") || s.ends_with(".tmp") {
        return true;
    }
    false
}
