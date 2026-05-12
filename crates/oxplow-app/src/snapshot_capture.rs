//! Periodic file-snapshot capture.
//!
//! Subscribes to `oxplow_fs_watch` events for the project worktree
//! and captures a `file_snapshot` row whenever a file changes. Bytes
//! are persisted to a content-addressed blob store under
//! `<project>/.oxplow/snapshots/<aa>/<aaaa...>`, keyed by the SHA-256
//! hash. The `local_blame` overlay and `restore_file_from_snapshot`
//! both read through `BlobStore::read` to recover past file content.
//!
//! Designed as a long-running task spawned at boot. Runs until the
//! receiver channel closes (typically on shutdown).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, warn};

use oxplow_db::{FileSnapshot, SqliteSnapshotStore};
use oxplow_domain::{StreamId, Timestamp};
use oxplow_fs_watch::{should_ignore_workspace_watch_path, FsWatcher};

use crate::blob_store::BlobStore;
use crate::events::{EventBus, OxplowEvent, SnapshotSourceKind};

#[derive(Clone)]
pub struct SnapshotCaptureService {
    store: Arc<SqliteSnapshotStore>,
    blobs: BlobStore,
    project_dir: PathBuf,
    stream_id: Option<StreamId>,
    /// Files larger than this skip blob hashing and are flagged
    /// `oversize`. Pulled from `OxplowConfig::snapshot_max_file_bytes`.
    max_file_bytes: u64,
    /// Optional event bus. When set, each captured snapshot fires a
    /// `FileSnapshotCreated` event so the renderer can refresh the
    /// Snapshots panel without polling.
    events: Option<EventBus>,
}

impl SnapshotCaptureService {
    pub fn new(
        store: Arc<SqliteSnapshotStore>,
        blobs: BlobStore,
        project_dir: PathBuf,
        stream_id: Option<StreamId>,
        max_file_bytes: u64,
    ) -> Self {
        Self {
            store,
            blobs,
            project_dir,
            stream_id,
            max_file_bytes,
            events: None,
        }
    }

    /// Attach an `EventBus` so capture_path emits
    /// `FileSnapshotCreated` after each successful insert.
    pub fn with_events(mut self, events: EventBus) -> Self {
        self.events = Some(events);
        self
    }

    /// Spawn the capture loop. Watches `project_dir` for changes and
    /// inserts a `file_snapshot` row per affected file. Returns the
    /// `JoinHandle` so callers can await teardown if needed.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move { self.run().await })
    }

    async fn run(self) {
        let watcher = match FsWatcher::watch(self.project_dir.clone(), Duration::from_millis(250)) {
            Ok(w) => w,
            Err(e) => {
                warn!(error = %e, "snapshot capture: failed to start fs-watch");
                return;
            }
        };
        let mut rx = watcher.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let path = event.path;
                    // Skip oxplow / git / build-cache dirs before we
                    // even stat the path. These churn fast (lock files,
                    // .tmp blobs, incremental compile artifacts) and
                    // always race the watcher to a NotFound, which
                    // used to spam DEBUG logs.
                    let rel = path.strip_prefix(&self.project_dir).unwrap_or(&path);
                    if should_ignore_workspace_watch_path(rel) {
                        continue;
                    }
                    if let Err(e) = self.capture_path(&path).await {
                        debug!(?path, error = %e, "snapshot capture: skipped");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "snapshot capture: fs-watch lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    async fn capture_path(
        &self,
        path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_file() {
            return Ok(());
        }
        let size = metadata.len();
        let oversize = size > self.max_file_bytes;
        let blob_hash = if oversize {
            None
        } else {
            let bytes = std::fs::read(path)?;
            // Persist the bytes content-addressed; blob_hash is the
            // SHA-256 hex. local_blame and restore_file_from_snapshot
            // read through BlobStore::read using this hash.
            Some(self.blobs.write(&bytes)?)
        };
        let rel = path
            .strip_prefix(&self.project_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let snap = FileSnapshot {
            id: 0,
            stream_id: self.stream_id.clone(),
            path: rel,
            blob_hash,
            size_bytes: size as i64,
            captured_at: Timestamp::now(),
            oversize,
        };
        let snapshot_id = self.store.capture(snap).await?;
        if let Some(bus) = &self.events {
            bus.emit(OxplowEvent::FileSnapshotCreated {
                stream_id: self.stream_id.clone(),
                snapshot_id,
                // The capture loop is the periodic background source —
                // task-start/task-end variants are reserved for explicit
                // hook-triggered captures elsewhere.
                source: SnapshotSourceKind::Startup,
                effort_id: None,
                thread_id: None,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use tempfile::tempdir;

    #[tokio::test]
    async fn capture_path_writes_a_row_and_blob() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        std::fs::write(&file, "hello").unwrap();
        let store = Arc::new(SqliteSnapshotStore::new(Database::in_memory()));
        let blobs = BlobStore::new(project.path().join(".oxplow/snapshots"));
        let svc = SnapshotCaptureService::new(
            store.clone(),
            blobs.clone(),
            project.path().to_path_buf(),
            None,
            1_000_000,
        );
        svc.capture_path(&file).await.unwrap();
        let rows = store.list_for_path("a.txt").await.unwrap();
        assert_eq!(rows.len(), 1);
        let hash = rows[0].blob_hash.as_deref().unwrap();
        assert_eq!(hash.len(), 64);
        // Bytes round-trip through the blob store.
        assert_eq!(blobs.read(hash).unwrap(), b"hello");
    }

    #[tokio::test]
    async fn oversize_file_skips_hash_and_blob() {
        let project = tempdir().unwrap();
        let file = project.path().join("big.bin");
        std::fs::write(&file, vec![0u8; 1024]).unwrap();
        let store = Arc::new(SqliteSnapshotStore::new(Database::in_memory()));
        let blobs = BlobStore::new(project.path().join(".oxplow/snapshots"));
        let svc = SnapshotCaptureService::new(
            store.clone(),
            blobs,
            project.path().to_path_buf(),
            None,
            512, // 512 byte cap → 1KB is oversize
        );
        svc.capture_path(&file).await.unwrap();
        let rows = store.list_for_path("big.bin").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].oversize);
        assert!(rows[0].blob_hash.is_none());
    }
}
