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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Cloneable result of an in-flight snapshot capture, published to
/// concurrent waiters via `tokio::sync::watch`. The error is collapsed
/// to an `Arc<str>` because `Box<dyn Error + Send + Sync>` isn't
/// `Clone`; concurrent waiters reconstruct an error from the message.
type SharedSnapshotResult = Result<Option<i64>, Arc<str>>;

use tracing::{debug, info, warn};

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

/// Pre-computed metadata supplied to `mark_dirty_with_staging` by
/// callers that already read + hashed the file (and wrote the blob).
/// When attached to a dirty-set entry, the capture loop skips re-stat
/// / re-read / re-hash / re-write and builds the DB row directly.
///
/// `blob_hash = None` means either an oversize row (metadata only) or
/// a deletion row — distinguish via `oversize`.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureStaging {
    pub size_bytes: i64,
    pub mtime_ms: Option<i64>,
    pub blob_hash: Option<String>,
    pub oversize: bool,
}

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
    /// The watcher loop pushes into this map; `request_snapshot`
    /// drains it. Keyed by path so repeated edits between requests
    /// collapse into a single capture.
    ///
    /// The value carries optional pre-staged metadata: callers that
    /// already read + hashed the file (currently just the startup
    /// sweep, after writing the blob inline) supply
    /// `Some(CaptureStaging)`, letting `request_snapshot` skip the
    /// stat / read / hash / blob.write entirely and just build the DB
    /// row. fs-watch and explicit `mark_dirty` callers store `None`;
    /// those paths go through the full parallel-process pipeline in
    /// `request_snapshot`.
    dirty: Mutex<HashMap<PathBuf, Option<CaptureStaging>>>,
    /// Single-flight slot for `request_snapshot`. When a capture is
    /// running, this holds a `watch` receiver that publishes the
    /// eventual result. Concurrent callers clone the receiver and
    /// await the same result — they neither drain the dirty set nor
    /// start a second capture. The slot is cleared back to `None`
    /// after the running capture publishes its result, so the next
    /// call starts fresh.
    in_flight: Mutex<Option<tokio::sync::watch::Receiver<Option<SharedSnapshotResult>>>>,
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
                dirty: Mutex::new(HashMap::new()),
                in_flight: Mutex::new(None),
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

    /// Add a path to the dirty set. The next `request_snapshot` will
    /// stat + read + hash + blob.write + INSERT for it. fs-watch and
    /// most call sites use this; the startup sweep prefers
    /// [`mark_dirty_with_staging`] to skip the redundant re-read of
    /// bytes it already had in memory.
    pub fn mark_dirty(&self, path: PathBuf) {
        let mut set = self.inner.dirty.lock().unwrap();
        // Don't downgrade an already-staged entry to None — keep the
        // pre-computed metadata so capture stays fast.
        set.entry(path).or_insert(None);
    }

    /// Add a path to the dirty set along with pre-computed staging
    /// metadata. `request_snapshot` will build the DB row from the
    /// staging fields and skip the file-read / blob-write — the
    /// caller must have already written the blob (when applicable).
    /// If the path already has a staging entry from a prior call,
    /// the newer staging wins.
    pub fn mark_dirty_with_staging(&self, path: PathBuf, staging: CaptureStaging) {
        self.inner.dirty.lock().unwrap().insert(path, Some(staging));
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
                let cleanup_started = Instant::now();
                match this.run_cleanup(retention_days).await {
                    Ok((rows, blobs)) => {
                        tracing::info!(
                            rows_pruned = rows,
                            blobs_removed = blobs,
                            retention_days,
                            elapsed_ms = cleanup_started.elapsed().as_millis() as u64,
                            "snapshot cleanup pass",
                        );
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
        let sweep_started = Instant::now();
        let db_started = Instant::now();
        let mut latest = self.inner.store.latest_stat_per_path().await?;
        let prior_rows = latest.len();
        let db_load_ms = db_started.elapsed().as_millis() as u64;
        info!(
            prior_rows,
            db_load_ms, "snapshot startup sweep: loaded latest_stat_per_path",
        );
        let project_dir = self.inner.project_dir.clone();
        let max_bytes = self.inner.max_file_bytes;
        let blobs = self.inner.blobs.clone();

        // Walk + stat off the async runtime — it's all blocking I/O.
        // The walk + per-file stat-shortcircuit stays single-threaded
        // (cheap; ~50 ms / ~17k files). The expensive read+hash+
        // blob-write for paths that fall through is fanned out across
        // the rayon thread pool — embarrassingly parallel.
        //
        // Phase 2 also writes the blob inline (we already have the
        // bytes in memory). The resulting `CaptureStaging` is shipped
        // back across the spawn_blocking boundary and queued via
        // `mark_dirty_with_staging`, so `request_snapshot` can build
        // each `file_snapshot` row without touching disk or hashing
        // again.
        let queued = tokio::task::spawn_blocking(move || -> Vec<(PathBuf, CaptureStaging)> {
            use rayon::prelude::*;

            // Phase 1 (sequential): walk, stat each file, decide
            // which paths fall through to read+hash. Outputs:
            //   - `staged`: oversize-new (already known) +
            //     reverse-deletions (path missing on disk). No
            //     read needed — staging built from stat only.
            //   - `needs_hash`: paths whose (size, mtime) didn't
            //     match the stored stat — fall through to phase 2.
            let mut staged: Vec<(PathBuf, CaptureStaging)> = Vec::new();
            let mut needs_hash: Vec<(PathBuf, i64, Option<i64>, Option<String>)> = Vec::new();
            let mut files_seen: u64 = 0;
            let mut shortcircuit_hits: u64 = 0;
            let mut oversize_new: u64 = 0;
            let phase1_started = Instant::now();
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
                files_seen += 1;
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
                            shortcircuit_hits += 1;
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
                        oversize_new += 1;
                        staged.push((
                            entry.path().to_path_buf(),
                            CaptureStaging {
                                size_bytes: size,
                                mtime_ms,
                                blob_hash: None,
                                oversize: true,
                            },
                        ));
                    }
                    continue;
                }
                needs_hash.push((
                    entry.path().to_path_buf(),
                    size,
                    mtime_ms,
                    prior.and_then(|s| s.blob_hash),
                ));
            }
            let phase1_ms = phase1_started.elapsed().as_millis() as u64;
            let needs_hash_count = needs_hash.len() as u64;
            info!(
                files_seen,
                shortcircuit_hits,
                oversize_new,
                needs_hash = needs_hash_count,
                phase1_ms,
                "snapshot startup sweep: phase 1 (walk + stat) done",
            );

            // Phase 2 (parallel): read + hash + write-blob the
            // fall-through set across the rayon pool. Each worker
            // is independent — we only emit a staging entry when
            // the new hash differs from the stored one (or there
            // was no stored hash). `BlobStore::write` short-circuits
            // when the content-addressed blob is already on disk,
            // so re-runs are cheap.
            let bytes_read = AtomicU64::new(0);
            let blobs_written = AtomicU64::new(0);
            let phase2_started = Instant::now();
            let hashed: Vec<(PathBuf, CaptureStaging)> = needs_hash
                .into_par_iter()
                .filter_map(|(path, size, mtime_ms, prior_hash)| {
                    let bytes = std::fs::read(&path).ok()?;
                    bytes_read.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    let hash = BlobStore::hash(&bytes);
                    if let Some(prior) = prior_hash.as_ref() {
                        if *prior == hash {
                            return None;
                        }
                    }
                    // Persist the blob now — we already have the
                    // bytes in memory. The serial capture path
                    // would otherwise re-read the same bytes off
                    // disk a moment later.
                    match blobs.write(&bytes) {
                        Ok(_) => {
                            blobs_written.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            warn!(?path, error = %e, "snapshot sweep: blob write failed");
                            return None;
                        }
                    }
                    Some((
                        path,
                        CaptureStaging {
                            size_bytes: size,
                            mtime_ms,
                            blob_hash: Some(hash),
                            oversize: false,
                        },
                    ))
                })
                .collect();
            let phase2_ms = phase2_started.elapsed().as_millis() as u64;
            let phase2_bytes = bytes_read.load(Ordering::Relaxed);
            info!(
                hashed_changed = hashed.len() as u64,
                blobs_written = blobs_written.load(Ordering::Relaxed),
                bytes_read = phase2_bytes,
                mb_read = phase2_bytes as f64 / 1_048_576.0,
                phase2_ms,
                throughput_mb_per_s = if phase2_ms == 0 {
                    0.0
                } else {
                    (phase2_bytes as f64 / 1_048_576.0) / (phase2_ms as f64 / 1000.0)
                },
                "snapshot startup sweep: phase 2 (parallel read+hash+blob) done",
            );
            staged.extend(hashed);

            // Any paths still in `latest` had a snapshot but no
            // file on disk now. Re-record deletions only for
            // those whose latest row wasn't already a deletion.
            for (path, stat) in latest {
                if stat.blob_hash.is_some() {
                    staged.push((
                        project_dir.join(path),
                        CaptureStaging {
                            size_bytes: 0,
                            mtime_ms: None,
                            blob_hash: None,
                            oversize: false,
                        },
                    ));
                }
            }
            staged
        })
        .await?;

        let count = queued.len();
        for (path, staging) in queued {
            self.mark_dirty_with_staging(path, staging);
        }
        info!(
            queued = count,
            elapsed_ms = sweep_started.elapsed().as_millis() as u64,
            "snapshot startup sweep: done",
        );
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
    /// already in flight, it awaits the in-flight capture and returns
    /// the same snapshot id. The dirty set is not drained twice; new
    /// paths that land during the wait get picked up by a subsequent
    /// explicit call.
    pub async fn request_snapshot(
        &self,
        source: SnapshotSourceKind,
    ) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
        enum SlotAction {
            Wait(tokio::sync::watch::Receiver<Option<SharedSnapshotResult>>),
            Run(tokio::sync::watch::Sender<Option<SharedSnapshotResult>>),
        }

        let action = {
            let mut slot = self.inner.in_flight.lock().unwrap();
            if let Some(rx) = slot.as_ref() {
                SlotAction::Wait(rx.clone())
            } else {
                let (tx, rx) = tokio::sync::watch::channel(None);
                *slot = Some(rx);
                SlotAction::Run(tx)
            }
        };

        match action {
            SlotAction::Wait(mut rx) => loop {
                if let Some(shared) = rx.borrow().clone() {
                    return shared.map_err(|msg| -> Box<dyn std::error::Error + Send + Sync> {
                        msg.to_string().into()
                    });
                }
                if rx.changed().await.is_err() {
                    return Err(
                        "in-flight snapshot capture was dropped without publishing a result".into(),
                    );
                }
            },
            SlotAction::Run(tx) => {
                let result = self.capture_inner(source).await;
                let shared: SharedSnapshotResult = match &result {
                    Ok(v) => Ok(*v),
                    Err(e) => Err(Arc::from(e.to_string())),
                };
                let _ = tx.send(Some(shared));
                {
                    let mut slot = self.inner.in_flight.lock().unwrap();
                    *slot = None;
                }
                result
            }
        }
    }

    /// Body of `request_snapshot` — runs the actual drain → blob.write
    /// → DB-insert → git-pin pipeline. Callers are expected to have
    /// already taken the single-flight slot in `request_snapshot`.
    async fn capture_inner(
        &self,
        source: SnapshotSourceKind,
    ) -> Result<Option<i64>, Box<dyn std::error::Error + Send + Sync>> {
        let drained: Vec<(PathBuf, Option<CaptureStaging>)> = {
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
        let capture_started = Instant::now();
        let drained_count = drained.len();
        let parent_id = self
            .inner
            .store
            .create_snapshot(self.inner.stream_id.clone())
            .await?;

        // Split: staged entries → build the row directly; unstaged
        // entries → run through the parallel stat/read/hash/blob.write
        // pipeline on the rayon pool, then assemble rows.
        let mut staged_paths: Vec<(PathBuf, CaptureStaging)> = Vec::new();
        let mut unstaged_paths: Vec<PathBuf> = Vec::new();
        for (path, staging) in drained {
            match staging {
                Some(s) => staged_paths.push((path, s)),
                None => unstaged_paths.push(path),
            }
        }
        let staged_count = staged_paths.len() as u64;
        let unstaged_count = unstaged_paths.len() as u64;

        let project_dir = self.inner.project_dir.clone();
        let stream_id = self.inner.stream_id.clone();
        let max_bytes = self.inner.max_file_bytes;
        let blobs = self.inner.blobs.clone();
        let rows: Vec<FileSnapshot> = tokio::task::spawn_blocking(move || -> Vec<FileSnapshot> {
            use rayon::prelude::*;

            fn rel_of(project_dir: &Path, path: &Path) -> String {
                path.strip_prefix(project_dir)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned()
            }

            let now = Timestamp::now();
            let mut rows: Vec<FileSnapshot> = Vec::with_capacity(staged_paths.len() + unstaged_paths.len());

            // Staged: trivial — staging already carries everything
            // the row needs.
            for (path, s) in staged_paths {
                rows.push(FileSnapshot {
                    id: 0,
                    stream_id: stream_id.clone(),
                    path: rel_of(&project_dir, &path),
                    blob_hash: s.blob_hash,
                    size_bytes: s.size_bytes,
                    captured_at: now,
                    oversize: s.oversize,
                    snapshot_id: Some(parent_id),
                    mtime_ms: s.mtime_ms,
                });
            }

            // Unstaged: parallel stat + read + hash + blob.write.
            // Mirrors the cold-sweep phase 2 shape so a branch-switch-
            // driven 5000-file drain runs as fast as the startup sweep.
            let unstaged_rows: Vec<FileSnapshot> = unstaged_paths
                .into_par_iter()
                .filter_map(|path| {
                    let metadata = match std::fs::metadata(&path) {
                        Ok(m) => m,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            return Some(FileSnapshot {
                                id: 0,
                                stream_id: stream_id.clone(),
                                path: rel_of(&project_dir, &path),
                                blob_hash: None,
                                size_bytes: 0,
                                captured_at: now,
                                oversize: false,
                                snapshot_id: Some(parent_id),
                                mtime_ms: None,
                            });
                        }
                        Err(e) => {
                            debug!(?path, error = %e, "snapshot capture: stat failed");
                            return None;
                        }
                    };
                    if !metadata.is_file() {
                        return None;
                    }
                    let size = metadata.len();
                    let mtime_ms = mtime_to_unix_ms(&metadata);
                    let oversize = size > max_bytes;
                    let blob_hash = if oversize {
                        None
                    } else {
                        match std::fs::read(&path) {
                            Ok(bytes) => match blobs.write(&bytes) {
                                Ok(h) => Some(h),
                                Err(e) => {
                                    debug!(?path, error = %e, "snapshot capture: blob write failed");
                                    return None;
                                }
                            },
                            Err(e) => {
                                debug!(?path, error = %e, "snapshot capture: read failed");
                                return None;
                            }
                        }
                    };
                    Some(FileSnapshot {
                        id: 0,
                        stream_id: stream_id.clone(),
                        path: rel_of(&project_dir, &path),
                        blob_hash,
                        size_bytes: size as i64,
                        captured_at: now,
                        oversize,
                        snapshot_id: Some(parent_id),
                        mtime_ms,
                    })
                })
                .collect();
            rows.extend(unstaged_rows);
            rows
        })
        .await?;

        let assembled = rows.len() as u64;
        let insert_started = Instant::now();
        let ids = self.inner.store.capture_batch(rows).await?;
        let insert_ms = insert_started.elapsed().as_millis() as u64;
        self.emit_batch_event(parent_id, ids.len() as u32, source);
        let capture_ms = capture_started.elapsed().as_millis() as u64;
        info!(
            parent_id,
            drained = drained_count as u64,
            staged = staged_count,
            unstaged = unstaged_count,
            inserted = ids.len() as u64,
            assembled,
            insert_ms,
            capture_ms,
            source = ?source,
            "snapshot request: captured drained set",
        );
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
            let pin_started = Instant::now();
            let stream_ref = Some(self.inner.stream_id.as_str());
            let statuses = git.statuses(stream_ref).await;
            let clean = statuses.is_empty();
            if clean {
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
            info!(
                parent_id,
                clean,
                git_pin_ms = pin_started.elapsed().as_millis() as u64,
                "snapshot request: git pin step",
            );
        }
        Ok(Some(parent_id))
    }

    fn emit_batch_event(&self, snapshot_id: i64, file_count: u32, source: SnapshotSourceKind) {
        let guard = self.inner.events.read().unwrap();
        if let Some(bus) = guard.as_ref() {
            bus.emit(OxplowEvent::FileSnapshotsBatchCreated {
                stream_id: Some(self.inner.stream_id.clone()),
                snapshot_id,
                file_count,
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
    async fn request_snapshot_concurrent_callers_share_result() {
        let project = tempdir().unwrap();
        // Seed enough files that the capture takes long enough for a
        // racing caller to land on the in-flight slot.
        for i in 0..50 {
            let p = project.path().join(format!("f{i}.txt"));
            std::fs::write(&p, format!("contents-{i}")).unwrap();
        }
        let (svc, store) = svc_for(project.path()).await;
        for i in 0..50 {
            svc.mark_dirty(project.path().join(format!("f{i}.txt")));
        }

        let svc_a = svc.clone();
        let svc_b = svc.clone();
        let (a, b) = tokio::join!(
            tokio::spawn(async move { svc_a.request_snapshot(SnapshotSourceKind::Startup).await }),
            tokio::spawn(async move { svc_b.request_snapshot(SnapshotSourceKind::Startup).await }),
        );
        let parent_a = a.unwrap().unwrap().expect("parent id a");
        let parent_b = b.unwrap().unwrap().expect("parent id b");
        // Both callers see the same snapshot id.
        assert_eq!(parent_a, parent_b);
        // Only one snapshot row was created — the second caller did
        // not start a fresh capture.
        let all = store
            .list_parent_snapshots_for_stream(TEST_STREAM, 100)
            .await
            .unwrap();
        assert_eq!(
            all.len(),
            1,
            "expected exactly one snapshot row, got {}",
            all.len()
        );
        // Dirty set was drained exactly once.
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
    async fn request_snapshot_uses_staged_metadata_without_reading_disk() {
        // Pre-stage a row for a path whose file doesn't exist on disk.
        // If the capture loop ignored the staging it would either skip
        // the row (stat fails) or record a deletion row. Instead it
        // must emit a row carrying the staged hash + size.
        let project = tempdir().unwrap();
        let (svc, store) = svc_for(project.path()).await;
        let path = project.path().join("phantom.txt");
        // File deliberately not created.
        svc.mark_dirty_with_staging(
            path.clone(),
            CaptureStaging {
                size_bytes: 42,
                mtime_ms: Some(1_700_000_000_000),
                blob_hash: Some("deadbeef".repeat(4)),
                oversize: false,
            },
        );
        let _parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        // list_for_path is the only read API that surfaces mtime_ms;
        // list_files_for_snapshot drops it. Both are real but only
        // the per-path one verifies staging carried mtime through.
        let rows = store.list_for_path("phantom.txt").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].size_bytes, 42);
        assert_eq!(rows[0].mtime_ms, Some(1_700_000_000_000));
        assert_eq!(
            rows[0].blob_hash.as_deref(),
            Some("deadbeefdeadbeefdeadbeefdeadbeef"),
        );
        assert!(!rows[0].oversize);
    }

    #[tokio::test]
    async fn request_snapshot_handles_mixed_staged_and_unstaged() {
        // Half the dirty set is pre-staged; the other half is raw
        // paths that still need stat+read+hash+blob.write. Both must
        // land in the same parent snapshot and produce real rows.
        let project = tempdir().unwrap();
        let (svc, store) = svc_for(project.path()).await;

        let staged = project.path().join("staged.txt");
        std::fs::write(&staged, "staged-body").unwrap();
        let staged_bytes = std::fs::read(&staged).unwrap();
        let staged_hash = svc.inner.blobs.write(&staged_bytes).unwrap();
        svc.mark_dirty_with_staging(
            staged.clone(),
            CaptureStaging {
                size_bytes: staged_bytes.len() as i64,
                mtime_ms: Some(42),
                blob_hash: Some(staged_hash.clone()),
                oversize: false,
            },
        );

        let unstaged = project.path().join("unstaged.txt");
        std::fs::write(&unstaged, "unstaged-body-which-is-longer").unwrap();
        svc.mark_dirty(unstaged.clone());

        let parent = svc
            .request_snapshot(SnapshotSourceKind::Startup)
            .await
            .unwrap()
            .expect("parent id");
        let files = store.list_files_for_snapshot(parent).await.unwrap();
        assert_eq!(files.len(), 2);
        let staged_row = files.iter().find(|f| f.path == "staged.txt").unwrap();
        let unstaged_row = files.iter().find(|f| f.path == "unstaged.txt").unwrap();
        assert_eq!(staged_row.blob_hash.as_deref(), Some(staged_hash.as_str()));
        // Unstaged side actually read+hashed the file and got a real
        // hash from the BlobStore.
        assert!(unstaged_row.blob_hash.is_some());
        assert!(svc
            .inner
            .blobs
            .has(unstaged_row.blob_hash.as_ref().unwrap()));
    }

    #[tokio::test]
    async fn capture_batch_inserts_all_in_one_transaction() {
        // Drive the new store API directly: 100 rows in one call,
        // each gets a distinct id and shows up in latest_stat_per_path.
        let db = Database::in_memory();
        seed_stream(&db).await;
        let store = SqliteSnapshotStore::new(db);
        let parent = store
            .create_snapshot(StreamId::from(TEST_STREAM))
            .await
            .unwrap();
        let snaps: Vec<oxplow_db::FileSnapshot> = (0..100)
            .map(|i| oxplow_db::FileSnapshot {
                id: 0,
                stream_id: StreamId::from(TEST_STREAM),
                path: format!("file_{i:03}.txt"),
                blob_hash: Some(format!("{:032x}", i)),
                size_bytes: i as i64,
                captured_at: Timestamp::now(),
                oversize: false,
                snapshot_id: Some(parent),
                mtime_ms: Some(1000 + i as i64),
            })
            .collect();
        let ids = store.capture_batch(snaps).await.unwrap();
        assert_eq!(ids.len(), 100);
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            100
        );
        let latest = store.latest_stat_per_path().await.unwrap();
        assert_eq!(latest.len(), 100);
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
