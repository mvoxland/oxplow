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

use tokio::sync::Mutex as AsyncMutex;

use tracing::{debug, warn};

use std::time::UNIX_EPOCH;

use oxplow_db::{FileSnapshot, SqliteSnapshotStore};

/// Extract `mtime` from a `Metadata` and convert to unix
/// milliseconds. Returns `None` when the platform / filesystem
/// doesn't expose mtime (rare) — callers fall back to hashing.
fn mtime_to_unix_ms(m: &std::fs::Metadata) -> Option<i64> {
    m.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}
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
    stream_id: StreamId,
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
    /// Gate that serializes `request_snapshot` so at most one
    /// capture runs at a time. When a second call arrives while a
    /// capture is in flight, `try_lock` fails and the caller returns
    /// without draining the dirty set — its paths get picked up by
    /// the next call after the in-flight one finishes.
    in_flight: AsyncMutex<()>,
    /// Optional handle to the singleton GitService. When set,
    /// `request_snapshot()` uses it to check worktree cleanliness
    /// and look up HEAD — both reads pull from GitService's cache
    /// so we don't re-stat the worktree on every capture. When None
    /// (test paths without a wired-up service), git-commit pinning
    /// is skipped.
    git: RwLock<Option<Arc<crate::git_service::GitService>>>,
}

impl SnapshotCaptureService {
    pub fn new(
        store: Arc<SqliteSnapshotStore>,
        blobs: BlobStore,
        project_dir: PathBuf,
        stream_id: StreamId,
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
                in_flight: AsyncMutex::new(()),
                git: RwLock::new(None),
            }),
        }
    }

    /// Attach an `EventBus` so capture emits `FileSnapshotCreated`
    /// after each successful insert.
    pub fn with_events(self, events: EventBus) -> Self {
        *self.inner.events.write().unwrap() = Some(events);
        self
    }

    /// Attach the singleton `GitService`. Enables git-commit pinning
    /// on `request_snapshot()` (uses the service's cached status
    /// map rather than re-scanning the worktree).
    pub fn with_git(self, git: Arc<crate::git_service::GitService>) -> Self {
        *self.inner.git.write().unwrap() = Some(git);
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

    /// Prune snapshot rows older than `retention_days` (keeping the
    /// most-recent row per path) and GC any on-disk blobs no longer
    /// referenced. Returns `(rows_pruned, blobs_removed)`.
    pub async fn run_cleanup(
        &self,
        retention_days: u32,
    ) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
        let cutoff = Timestamp::from_unix_ms(
            Timestamp::now().unix_ms() - (retention_days as i64) * 86_400_000,
        );
        let pruned = self.inner.store.prune_older_than(cutoff).await?;
        let referenced = self.inner.store.referenced_blob_hashes().await?;
        let blobs = self.inner.blobs.clone();
        let removed = tokio::task::spawn_blocking(move || blobs.gc(&referenced)).await??;
        Ok((pruned, removed))
    }

    /// Spawn a long-running cleanup loop: runs once shortly after
    /// boot, then every 24h. When `bts` is provided, each iteration
    /// surfaces as a row in the BackgroundTask HUD.
    pub fn spawn_cleanup_loop(
        &self,
        retention_days: u32,
        bts: Option<crate::background_task::BackgroundTaskStore>,
    ) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            // Brief delay so we don't pile cleanup on top of the
            // startup sweep's hashing work.
            tokio::time::sleep(Duration::from_secs(60)).await;
            loop {
                let task = bts.as_ref().map(|s| {
                    s.start(crate::background_task::StartInput {
                        kind: crate::background_task::BackgroundTaskKind::Snapshot,
                        label: "Pruning snapshot history".into(),
                        detail: None,
                        progress: None,
                    })
                });
                match this.run_cleanup(retention_days).await {
                    Ok((rows, blobs)) => {
                        if rows > 0 || blobs > 0 {
                            tracing::info!(
                                rows_pruned = rows,
                                blobs_removed = blobs,
                                retention_days,
                                "snapshot cleanup",
                            );
                        }
                        if let (Some(s), Some(t)) = (bts.as_ref(), task.as_ref()) {
                            s.complete(
                                &t.id,
                                Some(serde_json::json!({
                                    "rowsPruned": rows,
                                    "blobsRemoved": blobs,
                                })),
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "snapshot cleanup failed");
                        if let (Some(s), Some(t)) = (bts.as_ref(), task.as_ref()) {
                            s.fail(&t.id, e.to_string(), None);
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
            }
        })
    }

    /// Walk the worktree and mark every file whose current content
    /// differs from the most recent snapshot. Also marks paths that
    /// had a non-deleted latest snapshot but are no longer on disk —
    /// those get a deletion row when the dirty set is captured.
    ///
    /// Honors `should_ignore_workspace_watch_path`, so build dirs
    /// and `.oxplow/` internals are skipped (wiki pages pass through
    /// because the filter explicitly allows `.oxplow/wiki/`).
    ///
    /// Doesn't write anything itself — call `request_snapshot` after
    /// to flush the dirty set.
    pub async fn enqueue_startup_diff(
        &self,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let mut latest = self.inner.store.latest_stat_per_path().await?;
        let project_dir = self.inner.project_dir.clone();
        let max_bytes = self.inner.max_file_bytes;

        // Walk + stat off the async runtime — it's all blocking I/O.
        // The walk + per-file stat-shortcircuit stays single-threaded
        // (cheap; ~50 ms / ~17k files). The expensive read+hash for
        // paths that fall through is fanned out across the rayon
        // thread pool — embarrassingly parallel and CPU-bound (xxh3
        // hashes saturate memory bandwidth, not CPU, so the gain
        // comes mostly from parallel I/O).
        let queued = tokio::task::spawn_blocking(move || -> Vec<PathBuf> {
            use rayon::prelude::*;

            // Phase 1 (sequential): walk, stat each file, decide
            // which paths need a read+hash. Outputs three buckets:
            //   - `to_capture`: immediate enqueues (oversize-new,
            //     reverse-deletions). No read needed.
            //   - `needs_hash`: paths whose (size, mtime) didn't
            //     match the stored stat — fall through to phase 2.
            let mut to_capture: Vec<PathBuf> = Vec::new();
            let mut needs_hash: Vec<(PathBuf, Option<String>)> = Vec::new();
            for entry in walkdir::WalkDir::new(&project_dir)
                .into_iter()
                .filter_entry(|e| {
                    if e.depth() == 0 {
                        return true;
                    }
                    let rel = e.path().strip_prefix(&project_dir).unwrap_or(e.path());
                    !should_ignore_workspace_watch_path(rel)
                })
                .filter_map(Result::ok)
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let rel = entry
                    .path()
                    .strip_prefix(&project_dir)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .into_owned();
                let prior = latest.remove(&rel);
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let size = metadata.len() as i64;
                let mtime_ms = mtime_to_unix_ms(&metadata);
                // Fast equality check: when both size and mtime
                // match (and we have an mtime to compare against —
                // pre-V15 rows have None), the file hasn't been
                // touched since the last capture. Skip read+hash.
                if let Some(p) = prior.as_ref() {
                    if let (Some(prior_mtime), Some(cur_mtime)) = (p.mtime_ms, mtime_ms) {
                        if p.size_bytes == size && prior_mtime == cur_mtime {
                            continue;
                        }
                    }
                }
                // Oversize files are tracked by metadata-only rows,
                // so we can't hash-compare. Capture only when there's
                // no prior row at all — otherwise the row would be
                // identical to the existing one.
                if size as u64 > max_bytes {
                    if prior.is_none() {
                        to_capture.push(entry.path().to_path_buf());
                    }
                    continue;
                }
                needs_hash.push((entry.path().to_path_buf(), prior.and_then(|s| s.blob_hash)));
            }

            // Phase 2 (parallel): read + hash the fall-through set
            // across the rayon pool. Each worker is independent —
            // we only emit a path when the new hash differs from the
            // stored one (or there was no stored hash).
            let hashed: Vec<PathBuf> = needs_hash
                .into_par_iter()
                .filter_map(|(path, prior_hash)| {
                    let bytes = std::fs::read(&path).ok()?;
                    let hash = BlobStore::hash(&bytes);
                    match prior_hash {
                        Some(prior) if prior == hash => None,
                        _ => Some(path),
                    }
                })
                .collect();
            to_capture.extend(hashed);

            // Any paths still in `latest` had a snapshot but no file
            // on disk now. Re-record deletions only for those whose
            // latest row wasn't already a deletion.
            for (path, stat) in latest {
                if stat.blob_hash.is_some() {
                    to_capture.push(project_dir.join(path));
                }
            }
            to_capture
        })
        .await?;

        let count = queued.len();
        for path in queued {
            self.mark_dirty(path);
        }
        Ok(count)
    }

    /// Capture every path currently in the dirty set. Drains the
    /// set first so concurrent fs-events landing during the capture
    /// loop accumulate for the next request rather than being lost
    /// or double-captured.
    ///
    /// Returns the **parent `snapshot.id`** that groups every
    /// `file_snapshot` row written by this call. When the dirty set
    /// is empty, no new parent row is inserted; the most recent
    /// existing snapshot id for this stream is returned instead (or
    /// `None` if no snapshot has ever been taken for the stream).
    ///
    /// Captures are serialized: if a call arrives while another is
    /// already in flight it coalesces — the dirty set stays intact
    /// and the latest existing snapshot id is returned. The pending
    /// paths get captured by the next call after the in-flight one
    /// finishes.
    pub async fn request_snapshot(
        &self,
        source: SnapshotSourceKind,
    ) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
        // Coalesce concurrent callers: if a capture is already in
        // flight, return the latest existing snapshot id and leave
        // the dirty set untouched so its paths are picked up by the
        // next call after this one finishes.
        let _guard = match self.inner.in_flight.try_lock() {
            Ok(g) => g,
            Err(_) => {
                return Ok(self
                    .inner
                    .store
                    .latest_snapshot_id_for_stream(self.inner.stream_id.clone())
                    .await?);
            }
        };
        let drained: Vec<PathBuf> = {
            let mut set = self.inner.dirty.lock().unwrap();
            set.drain().collect()
        };
        if drained.is_empty() {
            return Ok(self
                .inner
                .store
                .latest_snapshot_id_for_stream(self.inner.stream_id.clone())
                .await?);
        }
        let parent_id = self
            .inner
            .store
            .create_snapshot(self.inner.stream_id.clone())
            .await?;
        for path in drained {
            if let Err(e) = self.capture_path(&path, parent_id, source).await {
                debug!(?path, error = %e, "snapshot capture: skipped");
            }
        }
        // After capture, pin to the current git commit if (and only
        // if) the worktree is clean — gitignored files don't count.
        // The check happens AFTER capture so any in-flight edits
        // were already drained into this snapshot's file rows.
        //
        // All git reads go through GitService so we use its cached
        // status map (invalidated on fs-watch / refs events) rather
        // than re-scanning the worktree each capture. When the
        // service hasn't been attached (test paths without one),
        // pinning is skipped.
        let git = self.inner.git.read().unwrap().clone();
        if let Some(git) = git {
            let stream_ref = Some(self.inner.stream_id.as_str());
            let statuses = git.statuses(stream_ref).await;
            if statuses.is_empty() {
                if let Some(sha) = git.head_commit_sha(stream_ref).await {
                    if let Err(e) = self
                        .inner
                        .store
                        .set_snapshot_git_commit(parent_id, sha)
                        .await
                    {
                        debug!(error = %e, "snapshot: failed to pin git commit");
                    }
                }
            }
        }
        Ok(Some(parent_id))
    }

    async fn capture_path(
        &self,
        path: &Path,
        parent_id: i64,
        source: SnapshotSourceKind,
    ) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // File deleted between dirty-set entry and capture —
                // record a deletion row (blob_hash = NULL, size = 0).
                return self
                    .record_deletion(path, parent_id, source)
                    .await
                    .map(Some);
            }
            Err(e) => return Err(Box::new(e)),
        };
        if !metadata.is_file() {
            return Ok(None);
        }
        let size = metadata.len();
        let mtime_ms = mtime_to_unix_ms(&metadata);
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
            snapshot_id: Some(parent_id),
            mtime_ms,
        };
        let row_id = self.inner.store.capture(snap).await?;
        self.emit_event(row_id, source);
        Ok(Some(row_id))
    }

    async fn record_deletion(
        &self,
        path: &Path,
        parent_id: i64,
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
            snapshot_id: Some(parent_id),
            mtime_ms: None,
        };
        let id = self.inner.store.capture(snap).await?;
        self.emit_event(id, source);
        Ok(id)
    }

    fn emit_event(&self, snapshot_id: i64, source: SnapshotSourceKind) {
        let guard = self.inner.events.read().unwrap();
        if let Some(bus) = guard.as_ref() {
            bus.emit(OxplowEvent::FileSnapshotCreated {
                stream_id: Some(self.inner.stream_id.clone()),
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

    const TEST_STREAM: &str = "s-test";

    async fn seed_stream(db: &Database) {
        use oxplow_domain::stores::StreamStore;
        let streams = oxplow_db::SqliteStreamStore::new(db.clone());
        streams
            .upsert(&oxplow_domain::Stream {
                id: StreamId::from(TEST_STREAM),
                kind: oxplow_domain::StreamKind::Primary,
                title: "t".into(),
                branch: "main".into(),
                branch_ref: "refs/heads/main".into(),
                branch_source: "main".into(),
                worktree_path: "/r".into(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: Timestamp::from_unix_ms(0),
                updated_at: Timestamp::from_unix_ms(0),
                archived_at: None,
            })
            .await
            .unwrap();
    }

    async fn svc_for(
        project: &std::path::Path,
    ) -> (SnapshotCaptureService, Arc<SqliteSnapshotStore>) {
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = Arc::new(SqliteSnapshotStore::new(db));
        let blobs = BlobStore::new(project.join(".oxplow/snapshots"));
        let svc = SnapshotCaptureService::new(
            store.clone(),
            blobs,
            project.to_path_buf(),
            StreamId::from(TEST_STREAM),
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
        let (svc, store) = svc_for(project.path()).await;
        svc.mark_dirty(a.clone());
        svc.mark_dirty(b.clone());

        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        // Both file rows point at the same parent.
        let files = store.list_files_for_snapshot(parent).await.unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.snapshot_id == Some(parent)));

        // Second request: dirty set was drained, nothing to capture —
        // returns the same parent id (no new row inserted).
        let again = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(again, Some(parent));

        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 1);
        assert_eq!(store.list_for_path("b.txt").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn request_snapshot_coalesces_when_already_in_flight() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let (svc, _store) = svc_for(project.path()).await;
        svc.mark_dirty(file);

        // Simulate an in-flight capture by holding the gate.
        let held = svc.inner.in_flight.lock().await;
        let result = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        // No snapshot exists yet, and we didn't run one → None.
        assert!(result.is_none());
        // Dirty set was NOT drained; the path is still queued.
        assert_eq!(svc.inner.dirty.lock().unwrap().len(), 1);
        drop(held);

        // Once the gate is released, a follow-up call captures the
        // pending path.
        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert!(parent.is_some());
        assert_eq!(svc.inner.dirty.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn request_snapshot_collapses_repeated_dirty_marks() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let (svc, store) = svc_for(project.path()).await;
        for _ in 0..10 {
            svc.mark_dirty(file.clone());
        }
        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        assert_eq!(
            store.list_files_for_snapshot(parent).await.unwrap().len(),
            1
        );
        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 1);
    }

    fn git_service_for(project: &std::path::Path) -> Arc<crate::git_service::GitService> {
        let db = Database::in_memory();
        let streams = Arc::new(oxplow_db::SqliteStreamStore::new(db));
        let bus = crate::events::EventBus::new();
        crate::git_service::GitService::spawn(project.to_path_buf(), streams, bus)
    }

    #[tokio::test]
    async fn clean_worktree_pins_snapshot_to_head_commit() {
        let project = tempdir().unwrap();
        let repo = git2::Repository::init(project.path()).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "t").unwrap();
        cfg.set_str("user.email", "t@example.com").unwrap();
        // Real projects gitignore `.oxplow/` so the snapshot
        // manager's own writes don't dirty the worktree. Mirror that
        // here.
        std::fs::write(project.path().join(".gitignore"), ".oxplow\n").unwrap();
        let tracked = project.path().join("tracked.txt");
        std::fs::write(&tracked, "v1").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("tracked.txt")).unwrap();
        idx.add_path(std::path::Path::new(".gitignore")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let head_oid = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
        let head_sha = head_oid.to_string();

        let (svc, store) = svc_for(project.path()).await;
        let svc = svc.with_git(git_service_for(project.path()));

        // Clean tree → snapshot pinned to HEAD.
        svc.mark_dirty(tracked.clone());
        let clean_id = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.get_snapshot_git_commit(clean_id).await.unwrap(),
            Some(head_sha.clone())
        );

        // Mutate the tracked file → worktree now dirty. The next
        // snapshot must NOT carry a git_commit.
        std::fs::write(&tracked, "v2").unwrap();
        svc.mark_dirty(tracked.clone());
        let dirty_id = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .unwrap();
        assert!(store
            .get_snapshot_git_commit(dirty_id)
            .await
            .unwrap()
            .is_none());

        // Gitignored files don't affect cleanliness. Reset the
        // tracked file, then extend .gitignore to also cover junk.log
        // and commit that change so the tree is clean.
        std::fs::write(&tracked, "v1").unwrap();
        std::fs::write(project.path().join(".gitignore"), ".oxplow\njunk.log\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new(".gitignore")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.find_commit(head_oid).unwrap();
        let head_oid2 = repo
            .commit(Some("HEAD"), &sig, &sig, "ignore", &tree, &[&parent])
            .unwrap();
        // Create an ignored file — should not break cleanliness.
        std::fs::write(project.path().join("junk.log"), "noise").unwrap();
        svc.mark_dirty(tracked.clone());
        let with_ignored = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.get_snapshot_git_commit(with_ignored).await.unwrap(),
            Some(head_oid2.to_string())
        );
    }

    #[tokio::test]
    async fn empty_request_returns_latest_snapshot_id() {
        let project = tempdir().unwrap();
        let (svc, store) = svc_for(project.path()).await;

        // No snapshots yet — request with empty dirty set returns None.
        let first = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert!(first.is_none());

        // Take a real snapshot.
        let file = project.path().join("a.txt");
        std::fs::write(&file, "hi").unwrap();
        svc.mark_dirty(file);
        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");

        // Subsequent empty requests reuse the same parent — no new
        // parent row is inserted.
        for _ in 0..3 {
            let again = svc
                .request_snapshot(SnapshotSourceKind::Startup)
                .await
                .unwrap();
            assert_eq!(again, Some(parent));
        }
        // Only one parent row exists.
        let latest = store
            .latest_snapshot_id_for_stream(StreamId::from(TEST_STREAM))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest, parent);
    }

    #[tokio::test]
    async fn deleted_file_records_a_deletion_row() {
        let project = tempdir().unwrap();
        let file = project.path().join("ghost.txt");
        let (svc, store) = svc_for(project.path()).await;
        // Never created on disk — mark_dirty + request_snapshot
        // should still record a deletion row.
        svc.mark_dirty(file);
        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        assert_eq!(
            store.list_files_for_snapshot(parent).await.unwrap().len(),
            1
        );
        let rows = store.list_for_path("ghost.txt").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].blob_hash.is_none());
        assert_eq!(rows[0].size_bytes, 0);
    }

    #[tokio::test]
    async fn startup_sweep_short_circuits_when_size_and_mtime_match() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        std::fs::write(&file, "v1").unwrap();
        let (svc, store) = svc_for(project.path()).await;

        // Prime: capture once so a baseline row exists with mtime.
        svc.mark_dirty(file.clone());
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        let rows = store.list_for_path("a.txt").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].mtime_ms.is_some(), "mtime should be recorded");

        // No changes at all → sweep queues nothing.
        let queued = svc.enqueue_startup_diff().await.unwrap();
        assert_eq!(queued, 0);

        // Real change: write longer content. Size mismatches → falls
        // through to the read+hash path and queues the file.
        std::fs::write(&file, "v3-much-longer").unwrap();
        let queued = svc.enqueue_startup_diff().await.unwrap();
        assert_eq!(queued, 1);
    }

    #[tokio::test]
    async fn startup_sweep_captures_only_changed_files() {
        let project = tempdir().unwrap();
        let a = project.path().join("a.txt");
        let b = project.path().join("b.txt");
        let c = project.path().join("c.txt");
        std::fs::write(&a, "one").unwrap();
        std::fs::write(&b, "two").unwrap();
        std::fs::write(&c, "three").unwrap();
        let (svc, store) = svc_for(project.path()).await;

        // Prime: capture all three so they have a baseline row.
        svc.mark_dirty(a.clone());
        svc.mark_dirty(b.clone());
        svc.mark_dirty(c.clone());
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 1);

        // Mutate `a`, leave `b` alone, delete `c`.
        std::fs::write(&a, "one!").unwrap();
        std::fs::remove_file(&c).unwrap();

        let queued = svc.enqueue_startup_diff().await.unwrap();
        assert_eq!(queued, 2);
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();

        // `a` got a new row, `c` got a deletion row, `b` is unchanged.
        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 2);
        assert_eq!(store.list_for_path("b.txt").await.unwrap().len(), 1);
        let c_rows = store.list_for_path("c.txt").await.unwrap();
        assert_eq!(c_rows.len(), 2);
        assert!(c_rows[0].blob_hash.is_none());
    }

    #[tokio::test]
    async fn cleanup_prunes_old_rows_and_gcs_orphan_blobs() {
        let project = tempdir().unwrap();
        let file = project.path().join("a.txt");
        let (svc, store) = svc_for(project.path()).await;

        // First capture — content "v1".
        std::fs::write(&file, "v1").unwrap();
        svc.mark_dirty(file.clone());
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();

        // Mutate and capture again — content "v2".
        std::fs::write(&file, "v2").unwrap();
        svc.mark_dirty(file.clone());
        svc.request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap();
        assert_eq!(store.list_for_path("a.txt").await.unwrap().len(), 2);

        // Backdate the older row so it falls outside any positive
        // retention window. Then run cleanup with 1 day retention —
        // the older row should be pruned but the newest kept.
        oxplow_db::SqliteSnapshotStore::backdate_for_test(
            store.clone(),
            "a.txt",
            Timestamp::from_unix_ms(0),
        )
        .await;
        let (rows, blobs) = svc.run_cleanup(1).await.unwrap();
        assert_eq!(rows, 1, "old row should be pruned");
        // The pruned row's blob is no longer referenced → GC removes
        // it. The kept row's blob stays.
        assert_eq!(blobs, 1, "orphan blob should be removed");
        let remaining = store.list_for_path("a.txt").await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(svc
            .inner
            .blobs
            .has(remaining[0].blob_hash.as_ref().unwrap()));
    }

    #[tokio::test]
    async fn oversize_file_skips_hash_and_blob() {
        let project = tempdir().unwrap();
        let file = project.path().join("big.bin");
        std::fs::write(&file, vec![0u8; 1024]).unwrap();
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = Arc::new(SqliteSnapshotStore::new(db));
        let blobs = BlobStore::new(project.path().join(".oxplow/snapshots"));
        let svc = SnapshotCaptureService::new(
            store.clone(),
            blobs,
            project.path().to_path_buf(),
            StreamId::from(TEST_STREAM),
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
