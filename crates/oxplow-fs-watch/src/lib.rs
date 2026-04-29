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

use notify::{RecommendedWatcher, RecursiveMode};
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
    pub fn watch(
        path: impl AsRef<Path>,
        debounce_window: Duration,
    ) -> Result<Self, FsWatchError> {
        let (tx, _) = broadcast::channel(256);
        let tx_clone = tx.clone();

        let mut debouncer = new_debouncer(
            debounce_window,
            None,
            move |res: DebounceEventResult| {
                let Ok(events) = res else { return };
                for evt in events {
                    let kind = classify(&evt.event);
                    for path in evt.event.paths.iter() {
                        // best-effort send — receivers may be lagging or absent
                        let _ = tx_clone.send(WatchEvent {
                            path: path.clone(),
                            kind: kind.clone(),
                        });
                    }
                }
            },
        )?;

        debouncer.watch(path.as_ref(), RecursiveMode::Recursive)?;

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
        loop {
            match timeout(Duration::from_millis(800), rx.recv()).await {
                Ok(Ok(_)) => count += 1,
                _ => break,
            }
        }

        assert!(count > 0, "expected at least one event");
        assert!(count < 20, "expected debouncing, got {count} events");
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
            Err(_) => {} // timeout — fine
            Ok(Err(broadcast::error::RecvError::Closed)) => {} // closed — fine
            other => panic!("expected closed/timeout, got {other:?}"),
        }
    }
}
