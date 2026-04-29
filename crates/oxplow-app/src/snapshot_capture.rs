//! Periodic file-snapshot capture.
//!
//! Subscribes to `oxplow_fs_watch` events for the project worktree
//! and captures a `file_snapshot` row whenever a file changes. Blob
//! storage isn't ported yet (see MIGRATION_REVIEW2 §3 / sharp edge
//! §5), so for now we capture the *metadata* — path, size, sha256
//! of the bytes — without persisting the bytes themselves. The
//! `local_blame` overlay and `restore_file_from_snapshot` will be
//! upgraded once a content-addressed blob store lands.
//!
//! Designed as a long-running task spawned at boot. Runs until the
//! `CancellationToken` fires (typically on shutdown).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use oxplow_db::{FileSnapshot, SqliteSnapshotStore};
use oxplow_domain::{StreamId, Timestamp};
use oxplow_fs_watch::FsWatcher;

#[derive(Clone)]
pub struct SnapshotCaptureService {
    store: Arc<SqliteSnapshotStore>,
    project_dir: PathBuf,
    stream_id: Option<StreamId>,
    /// Files larger than this skip blob hashing and are flagged
    /// `oversize`. Pulled from `OxplowConfig::snapshot_max_file_bytes`.
    max_file_bytes: u64,
}

impl SnapshotCaptureService {
    pub fn new(
        store: Arc<SqliteSnapshotStore>,
        project_dir: PathBuf,
        stream_id: Option<StreamId>,
        max_file_bytes: u64,
    ) -> Self {
        Self {
            store,
            project_dir,
            stream_id,
            max_file_bytes,
        }
    }

    /// Spawn the capture loop. Watches `project_dir` for changes and
    /// inserts a `file_snapshot` row per affected file. Returns the
    /// `JoinHandle` so callers can await teardown if needed.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move { self.run().await })
    }

    async fn run(self) {
        let watcher = match FsWatcher::watch(self.project_dir.clone(), Duration::from_millis(250))
        {
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

    async fn capture_path(&self, path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            Some(format!("{:x}", hasher.finalize()))
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
        self.store.capture(snap).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use tempfile::tempdir;

    #[tokio::test]
    async fn capture_path_writes_a_row() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        std::fs::write(&file, "hello").unwrap();
        let store = Arc::new(SqliteSnapshotStore::new(Database::in_memory()));
        let svc = SnapshotCaptureService::new(
            store.clone(),
            project.path().to_path_buf(),
            None,
            1_000_000,
        );
        svc.capture_path(&file).await.unwrap();
        let rows = store.list_for_path("a.txt").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].blob_hash.as_deref().unwrap().len() == 64);
    }

    #[tokio::test]
    async fn oversize_file_skips_hash() {
        let project = tempdir().unwrap();
        let file = project.path().join("big.bin");
        std::fs::write(&file, vec![0u8; 1024]).unwrap();
        let store = Arc::new(SqliteSnapshotStore::new(Database::in_memory()));
        let svc = SnapshotCaptureService::new(
            store.clone(),
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
