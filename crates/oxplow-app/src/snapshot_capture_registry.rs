//! Per-stream snapshot capture services.
//!
//! Snapshot capture is per-worktree: each `Stream` watches its own
//! worktree path and writes `file_snapshot` rows tagged with its
//! `stream_id`. The registry is the single lookup point — callers
//! resolve the right service via `get(&stream_id)` and use it for
//! `request_snapshot`, blob reads, etc.
//!
//! All services share the same `SqliteSnapshotStore` + `BlobStore` so
//! the on-disk representation is unified; only the in-memory dirty
//! set, settle gate, and fs-watch listener are per-stream.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use oxplow_db::SqliteSnapshotStore;
use oxplow_domain::{Stream, StreamId};
use oxplow_fs_watch::WorkspaceFilter;

use crate::blob_store::BlobStore;
use crate::events::EventBus;
use crate::snapshot_capture::SnapshotCaptureService;

/// Build parameters shared across every per-stream service. The
/// registry holds these so `register()` can construct fresh services
/// at runtime (e.g. when a new stream is created) without callers
/// having to re-thread the storage handles each time.
#[derive(Clone)]
pub struct SnapshotCaptureRegistryConfig {
    pub snapshot_store: Arc<SqliteSnapshotStore>,
    pub blobs: BlobStore,
    pub max_file_bytes: u64,
    pub workspace_filter: WorkspaceFilter,
    pub events: EventBus,
}

/// Per-stream registry of [`SnapshotCaptureService`]s. Look up by
/// `StreamId`; mutate via `register` / `unregister`. Cheap to clone —
/// the underlying map sits behind an `Arc<RwLock<…>>`.
#[derive(Clone)]
pub struct SnapshotCaptureRegistry {
    services: Arc<RwLock<HashMap<StreamId, Arc<SnapshotCaptureService>>>>,
    /// `primary_id` lets legacy single-stream callers fetch the
    /// primary service without already knowing its id — the wiki
    /// watcher, MCP file-ref-version resolver, and bootloader logs
    /// all need it. Set once at boot.
    primary_id: Arc<RwLock<Option<StreamId>>>,
    config: SnapshotCaptureRegistryConfig,
}

impl SnapshotCaptureRegistry {
    pub fn new(config: SnapshotCaptureRegistryConfig) -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
            primary_id: Arc::new(RwLock::new(None)),
            config,
        }
    }

    /// Build + insert a service for `stream`. Idempotent: if a service
    /// already exists for this stream id the existing handle is
    /// returned unchanged (caller stays observer of the same fs-watch
    /// pipeline). Streams whose worktree path doesn't exist on disk
    /// (archived, orphaned) are silently skipped — returns `None`.
    pub fn register(&self, stream: &Stream) -> Option<Arc<SnapshotCaptureService>> {
        let worktree = PathBuf::from(&stream.worktree_path);
        if !worktree.is_dir() {
            tracing::debug!(
                stream_id = %stream.id,
                worktree = %stream.worktree_path,
                "snapshot registry: skipping stream — worktree not on disk",
            );
            return None;
        }
        // Fast-path: already registered.
        {
            let services = self.services.read().unwrap();
            if let Some(existing) = services.get(&stream.id) {
                return Some(existing.clone());
            }
        }
        let svc = Arc::new(
            SnapshotCaptureService::new(
                self.config.snapshot_store.clone(),
                self.config.blobs.clone(),
                worktree,
                stream.id.clone(),
                self.config.max_file_bytes,
                self.config.workspace_filter.clone(),
            )
            .with_events(self.config.events.clone()),
        );
        let mut services = self.services.write().unwrap();
        // Double-check after acquiring the write lock — a concurrent
        // register for the same id may have raced us.
        if let Some(existing) = services.get(&stream.id) {
            return Some(existing.clone());
        }
        services.insert(stream.id.clone(), svc.clone());
        Some(svc)
    }

    /// Drop the service for `id`. The fs-watch task spawned by
    /// `spawn_watcher` exits when its broadcast receiver closes (the
    /// `FsWatcher` is held inside the service's `Inner`, so dropping
    /// the last `Arc` ends the watcher).
    pub fn unregister(&self, id: &StreamId) {
        let mut services = self.services.write().unwrap();
        services.remove(id);
    }

    pub fn get(&self, id: &StreamId) -> Option<Arc<SnapshotCaptureService>> {
        self.services.read().unwrap().get(id).cloned()
    }

    /// Snapshot of the currently-registered services. Order is not
    /// guaranteed; callers needing primary-first should query
    /// [`Self::primary`] explicitly.
    pub fn list(&self) -> Vec<Arc<SnapshotCaptureService>> {
        self.services.read().unwrap().values().cloned().collect()
    }

    /// Mark `id` as the primary stream — used by legacy callers that
    /// haven't migrated to a stream-aware lookup yet (wiki watcher,
    /// MCP file-ref-version resolver). Must be called after the
    /// corresponding `register` so the lookup actually resolves.
    pub fn set_primary(&self, id: StreamId) {
        *self.primary_id.write().unwrap() = Some(id);
    }

    /// Fetch the primary stream's service. `None` only before boot
    /// has called [`Self::set_primary`].
    pub fn primary(&self) -> Option<Arc<SnapshotCaptureService>> {
        let id = self.primary_id.read().unwrap().clone()?;
        self.get(&id)
    }

    /// Spawn the fs-watch listener for every currently-registered
    /// service. Safe to call multiple times — each service
    /// idempotently spawns at most one watcher per `Arc` (in practice
    /// only main.rs calls this, once at boot).
    pub fn spawn_all_watchers(&self) -> Vec<tokio::task::JoinHandle<()>> {
        let services = self.services.read().unwrap();
        services.values().map(|s| s.spawn_watcher()).collect()
    }

    /// Test-only: insert a pre-built service under `id`, replacing any
    /// existing entry. Lets tests override the default builder (with
    /// custom settle / predrain durations, for example) without
    /// re-implementing the full `register` path.
    pub fn insert_for_test(&self, id: StreamId, svc: Arc<SnapshotCaptureService>) {
        let mut services = self.services.write().unwrap();
        services.insert(id, svc);
    }
}
