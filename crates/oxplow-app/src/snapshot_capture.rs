//! Request-driven file-snapshot capture.
//!
//! A singleton manager subscribes to `oxplow_fs_watch` events for
//! the project worktree and accumulates a *dirty set* of paths that
//! changed since the last capture. **Nothing is written to the
//! `file_snapshot` table until someone calls `request_snapshot()`.**
//! That call drains the dirty set, captures each path once, and
//! returns the new snapshot ids.
//!
//! Bytes are persisted to a content-addressed blob store under
//! `<project>/.oxplow/snapshots/<aa>/<aaaa...>`, keyed by the
//! SHA-256 hash. The `local_blame` overlay and
//! `restore_file_from_snapshot` both read through `BlobStore::read`
//! to recover past file content.
//!
//! Cheap to clone — the underlying state is held in an `Arc`. Spawn
//! the watcher loop once at boot via `spawn_watcher()`; everything
//! else is method calls on the cloned handle.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tracing::{debug, warn};

use oxplow_db::{FileSnapshot, SqliteSnapshotStore};
use oxplow_domain::{StreamId, Timestamp};
use oxplow_fs_watch::{should_ignore_workspace_watch_path, FsWatcher};

use crate::blob_store::BlobStore;
use crate::events::{EventBus, OxplowEvent, SnapshotSourceKind};

#[derive(Clone)]
pub struct SnapshotCaptureService {
    inner: Arc<Inner>,
}

struct Inner {
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
    events: RwLock<Option<EventBus>>,
    /// Paths that have changed since the last `request_snapshot()`.
    /// The watcher loop pushes into this set; `request_snapshot`
    /// drains it. A HashSet collapses repeated edits to the same
    /// file between requests into a single capture.
    dirty: Mutex<HashSet<PathBuf>>,
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
            inner: Arc::new(Inner {
                store,
                blobs,
                project_dir,
                stream_id,
                max_file_bytes,
                events: RwLock::new(None),
                dirty: Mutex::new(HashSet::new()),
            }),
        }
    }

    /// Attach an `EventBus` so capture emits `FileSnapshotCreated`
    /// after each successful insert.
    pub fn with_events(self, events: EventBus) -> Self {
        *self.inner.events.write().unwrap() = Some(events);
        self
    }

    pub fn project_dir(&self) -> &Path {
        &self.inner.project_dir
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.inner.blobs
    }

    pub fn store(&self) -> &Arc<SqliteSnapshotStore> {
        &self.inner.store
    }

    /// Spawn the fs-watch listener. The listener only updates the
    /// in-memory dirty set; it never writes to the database. Returns
    /// the `JoinHandle` so callers can await teardown if needed.
    pub fn spawn_watcher(&self) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move { this.run_watcher().await })
    }

    async fn run_watcher(self) {
        let watcher =
            match FsWatcher::watch(self.inner.project_dir.clone(), Duration::from_millis(250)) {
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
                    let rel = path.strip_prefix(&self.inner.project_dir).unwrap_or(&path);
                    if should_ignore_workspace_watch_path(rel) {
                        continue;
                    }
                    self.mark_dirty(path);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "snapshot capture: fs-watch lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    /// Add a path to the dirty set. Exposed for the startup sweep
    /// (and any future code paths that want to enqueue captures
    /// without going through fs-watch).
    pub fn mark_dirty(&self, path: PathBuf) {
        self.inner.dirty.lock().unwrap().insert(path);
    }

    /// Capture every path currently in the dirty set. Drains the
    /// set first so concurrent fs-events landing during the capture
    /// loop accumulate for the next request rather than being lost
    /// or double-captured. Returns the inserted snapshot ids.
    pub async fn request_snapshot(
        &self,
        source: SnapshotSourceKind,
    ) -> Result<Vec<i64>, Box<dyn std::error::Error + Send + Sync>> {
        let drained: Vec<PathBuf> = {
            let mut set = self.inner.dirty.lock().unwrap();
            set.drain().collect()
        };
        let mut ids = Vec::with_capacity(drained.len());
        for path in drained {
            match self.capture_path(&path, source).await {
                Ok(Some(id)) => ids.push(id),
                Ok(None) => {}
                Err(e) => debug!(?path, error = %e, "snapshot capture: skipped"),
            }
        }
        Ok(ids)
    }

    async fn capture_path(
        &self,
        path: &Path,
        source: SnapshotSourceKind,
    ) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // File deleted between dirty-set entry and capture —
                // record a deletion row (blob_hash = NULL, size = 0).
                return self.record_deletion(path, source).await.map(Some);
            }
            Err(e) => return Err(Box::new(e)),
        };
        if !metadata.is_file() {
            return Ok(None);
        }
        let size = metadata.len();
        let oversize = size > self.inner.max_file_bytes;
        let blob_hash = if oversize {
            None
        } else {
            let bytes = std::fs::read(path)?;
            Some(self.inner.blobs.write(&bytes)?)
        };
        let rel = path
            .strip_prefix(&self.inner.project_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let snap = FileSnapshot {
            id: 0,
            stream_id: self.inner.stream_id.clone(),
            path: rel,
            blob_hash,
            size_bytes: size as i64,
            captured_at: Timestamp::now(),
            oversize,
        };
        let snapshot_id = self.inner.store.capture(snap).await?;
        self.emit_event(snapshot_id, source);
        Ok(Some(snapshot_id))
    }

    async fn record_deletion(
        &self,
        path: &Path,
        source: SnapshotSourceKind,
    ) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        let rel = path
            .strip_prefix(&self.inner.project_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let snap = FileSnapshot {
            id: 0,
            stream_id: self.inner.stream_id.clone(),
            path: rel,
            blob_hash: None,
            size_bytes: 0,
            captured_at: Timestamp::now(),
            oversize: false,
        };
        let id = self.inner.store.capture(snap).await?;
        self.emit_event(id, source);
        Ok(id)
    }

    fn emit_event(&self, snapshot_id: i64, source: SnapshotSourceKind) {
        let guard = self.inner.events.read().unwrap();
        if let Some(bus) = guard.as_ref() {
            bus.emit(OxplowEvent::FileSnapshotCreated {
                stream_id: self.inner.stream_id.clone(),
                snapshot_id,
                source,
                effort_id: None,
                thread_id: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use tempfile::tempdir;

    fn svc_for(project: &std::path::Path) -> (SnapshotCaptureService, Arc<SqliteSnapshotStore>) {
        let store = Arc::new(SqliteSnapshotStore::new(Database::in_memory()));
        let blobs = BlobStore::new(project.join(".oxplow/snapshots"));
        let svc = SnapshotCaptureService::new(
            store.clone(),
            blobs,
            project.to_path_buf(),
            None,
            1_000_000,
        );
        (svc, store)
    }

    #[tokio::test]
    async fn request_snapshot_captures_dirty_files_and_drains_set() {
        let project = tempdir().unwrap();
        let a = project.path().join("a.txt");
        let b = project.path().join("b.txt");
        std::fs::write(&a, "hello").unwrap();
        std::fs::write(&b, "world").unwrap();
        let (svc, store) = svc_for(project.path());
        svc.mark_dirty(a.clone());
        svc.mark_dirty(b.clone());

        let ids = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(ids.len(), 2);

        // Second request: dirty set was drained, nothing to capture.
        let again = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert!(again.is_empty());

        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 1);
        assert_eq!(store.list_for_path("b.txt").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn request_snapshot_collapses_repeated_dirty_marks() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let (svc, store) = svc_for(project.path());
        for _ in 0..10 {
            svc.mark_dirty(file.clone());
        }
        let ids = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn deleted_file_records_a_deletion_row() {
        let project = tempdir().unwrap();
        let file = project.path().join("ghost.txt");
        let (svc, store) = svc_for(project.path());
        // Never created on disk — mark_dirty + request_snapshot
        // should still record a deletion row.
        svc.mark_dirty(file);
        let ids = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(ids.len(), 1);
        let rows = store.list_for_path("ghost.txt").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].blob_hash.is_none());
        assert_eq!(rows[0].size_bytes, 0);
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
        svc.mark_dirty(file);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        let rows = store.list_for_path("big.bin").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].oversize);
        assert!(rows[0].blob_hash.is_none());
    }
}
