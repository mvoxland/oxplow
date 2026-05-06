//! Singleton git access surface.
//!
//! `GitService` is the one place anything in the app reaches when it
//! needs git data. It owns a per-stream snapshot cache (status,
//! branches, conflict state, ahead/behind) that's refreshed in the
//! background whenever the filesystem watcher reports a change under
//! the worktree or `.git/refs/`. Read methods serve from the cache
//! when possible and fall back to a live query if the snapshot hasn't
//! been hydrated yet (initial paint, brand-new stream). Mutating
//! operations — commit, push, pull, fetch, merge, rebase, branch
//! ops — pass through to `oxplow_git::*` and immediately schedule a
//! refresh + broadcast so the UI catches up without waiting for the
//! watcher's debounce window.
//!
//! Why a singleton across all streams (rather than one service per
//! stream): a single watcher task / debouncer / broadcast subscription
//! is fewer moving parts, and stream creation/deletion is just a
//! register/deregister against this one service. Per-stream snapshots
//! still live inside it, keyed by `StreamId`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxplow_domain::stores::StreamStore;
use oxplow_domain::{StreamId, Timestamp};
use oxplow_git::{
    detect_current_branch, AheadBehind, BlameLine, BranchChanges, BranchRef, ChangeScopes,
    CommitDetail, GitFileStatus, GitLogCommit, GitLogOptions, GitLogResult, GitOpResult,
    GitWorktreeEntry, GroupedGitRefs, LocalBlameEntry, RemoteBranchEntry, RepoConflictState,
    TextSearchHit, WorkspaceEntry, WorkspaceFile, WorkspaceIndexedFile, WorkspaceStatusSummary,
};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, warn};

use crate::events::{EventBus, OxplowEvent};

/// Which slice of a stream's snapshot a refresh request should
/// recompute. Multiple flags can be OR'd in a single request.
#[derive(Debug, Clone, Copy, Default)]
struct RefreshKinds {
    statuses: bool,
    branches: bool,
    conflict: bool,
    /// Recent commit log (top `RECENT_LOG_LIMIT`, HEAD-only).
    log: bool,
    /// Remote branches list (project-wide cache, but scheduled per
    /// stream so the worker dedupes naturally).
    remote_branches: bool,
}

/// How many commits the per-stream `recent_log` cache holds. The
/// dashboard asks for 5; we cache extra so the same snapshot can serve
/// the GitHistory page's first paint without a re-query.
const RECENT_LOG_LIMIT: usize = 50;
/// How many remote branches the project-wide cache holds.
const RECENT_REMOTE_BRANCHES_LIMIT: usize = 50;

impl RefreshKinds {
    fn all() -> Self {
        Self {
            statuses: true,
            branches: true,
            conflict: true,
            log: true,
            remote_branches: true,
        }
    }

    fn merge(&mut self, other: RefreshKinds) {
        self.statuses |= other.statuses;
        self.branches |= other.branches;
        self.conflict |= other.conflict;
        self.log |= other.log;
        self.remote_branches |= other.remote_branches;
    }

    fn any(&self) -> bool {
        self.statuses || self.branches || self.conflict || self.log || self.remote_branches
    }
}

/// What we keep in memory per stream. `None` means "not yet
/// hydrated"; readers fall through to a live query for those slots
/// and write the result back.
#[derive(Default)]
struct StreamSnapshot {
    worktree: PathBuf,
    statuses: Option<HashMap<String, GitFileStatus>>,
    status_summary: Option<WorkspaceStatusSummary>,
    branches: Option<Vec<BranchRef>>,
    conflict_state: Option<RepoConflictState>,
    /// Top `RECENT_LOG_LIMIT` HEAD-only commits.
    recent_log: Option<GitLogResult>,
    /// `(base, head)` -> counts. Cleared whenever refs/statuses change.
    /// The dashboard fans out per-stream `getAheadBehind` calls; this
    /// memoization makes the second-and-later renders free.
    ahead_behind: HashMap<(String, String), AheadBehind>,
    last_refreshed: Option<Instant>,
}

struct Inner {
    snapshots: HashMap<StreamId, Arc<RwLock<StreamSnapshot>>>,
    /// Project-wide cache for `list_recent_remote_branches` (the call
    /// is scoped to the project root, not a worktree). Holds up to
    /// `RECENT_REMOTE_BRANCHES_LIMIT` entries; readers can serve
    /// `limit <= cached.len()` from cache.
    recent_remote_branches: Option<Vec<RemoteBranchEntry>>,
}

/// One entry the refresh worker is asked to handle. The worker
/// coalesces consecutive entries for the same stream within a small
/// debounce window so a flurry of fs-watch events collapses to one
/// `git status` round-trip.
#[derive(Debug, Clone)]
struct RefreshTask {
    stream_id: StreamId,
    kinds: RefreshKinds,
}

/// Singleton handle. Held inside `Services` as `Arc<GitService>`.
pub struct GitService {
    project_dir: PathBuf,
    streams: Arc<dyn StreamStore>,
    events: EventBus,
    inner: Arc<RwLock<Inner>>,
    refresh_tx: mpsc::UnboundedSender<RefreshTask>,
    /// Internal broadcast — emitted alongside every cache update.
    /// Most consumers subscribe to `EventBus` instead; this exists so
    /// inside-process modules can wait on a specific stream/kind
    /// without going through the json-typed `OxplowEvent` payload.
    snapshot_tx: broadcast::Sender<SnapshotChanged>,
}

#[derive(Debug, Clone)]
pub struct SnapshotChanged {
    pub stream_id: StreamId,
    pub statuses: bool,
    pub branches: bool,
    pub conflict: bool,
    pub log: bool,
    pub remote_branches: bool,
}

impl GitService {
    /// Build the service and start its background refresh worker.
    /// Must run inside an entered tokio runtime (the workers are
    /// spawned via `tokio::spawn`). Existing streams are seeded
    /// asynchronously from `streams.list()` so callers don't have to
    /// await; reads against unseeded streams fall back to a live
    /// query and write the result back into the cache.
    pub fn spawn(
        project_dir: PathBuf,
        streams: Arc<dyn StreamStore>,
        events: EventBus,
    ) -> Arc<Self> {
        let (refresh_tx, refresh_rx) = mpsc::unbounded_channel::<RefreshTask>();
        let (snapshot_tx, _) = broadcast::channel::<SnapshotChanged>(256);
        let svc = Arc::new(Self {
            project_dir,
            streams: streams.clone(),
            events: events.clone(),
            inner: Arc::new(RwLock::new(Inner {
                snapshots: HashMap::new(),
                recent_remote_branches: None,
            })),
            refresh_tx,
            snapshot_tx,
        });

        // Refresh worker — owns the cache mutation. Lives for the
        // process lifetime.
        Self::spawn_refresh_worker(svc.clone(), refresh_rx);
        // Bus listener — translates fs-watch + refs events into
        // refresh tasks.
        Self::spawn_bus_listener(svc.clone());
        // Seed snapshots for existing streams in the background, and
        // reconcile each stream's branch field against the live HEAD
        // — covers the case where the user switched branches in the
        // worktree while oxplow was not running.
        let seed_svc = svc.clone();
        tokio::spawn(async move {
            if let Ok(rows) = streams.list().await {
                for s in rows {
                    let path = resolve_worktree(&seed_svc.project_dir, &s.worktree_path);
                    seed_svc.register_internal(&s.id, path.clone()).await;
                    seed_svc.reconcile_branch(&s.id, &path).await;
                }
            }
        });

        svc
    }

    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// Subscribe to fine-grained snapshot updates. Most callers should
    /// prefer the renderer-facing `EventBus` instead — this is for
    /// rust-internal consumers that want to block on "branches just
    /// re-listed for this specific stream".
    pub fn subscribe(&self) -> broadcast::Receiver<SnapshotChanged> {
        self.snapshot_tx.subscribe()
    }

    /// Resolve a stream id (or `None` → project root) to a worktree
    /// path. Mirrors what the IPC layer used to do inline.
    pub async fn resolve_repo_dir(&self, stream_id: Option<&str>) -> PathBuf {
        let Some(id) = stream_id else {
            return self.project_dir.clone();
        };
        let map = self.inner.read().await;
        if let Some(snap) = map.snapshots.get(&StreamId::from(id)) {
            return snap.read().await.worktree.clone();
        }
        drop(map);
        // Stream may exist in the store but not yet registered (e.g.
        // an IPC call landing before boot finished seeding). Look it
        // up directly so we still hand back a useful path.
        if let Ok(rows) = self.streams.list().await {
            if let Some(s) = rows.into_iter().find(|s| s.id.as_str() == id) {
                return resolve_worktree(&self.project_dir, &s.worktree_path);
            }
        }
        self.project_dir.clone()
    }

    /// Register a stream's worktree with the service. Idempotent.
    /// Triggers an initial full refresh asynchronously.
    pub async fn register(&self, stream_id: &StreamId, worktree: PathBuf) {
        self.register_internal(stream_id, worktree).await;
    }

    async fn register_internal(&self, stream_id: &StreamId, worktree: PathBuf) {
        {
            let mut inner = self.inner.write().await;
            inner.snapshots.entry(stream_id.clone()).or_insert_with(|| {
                Arc::new(RwLock::new(StreamSnapshot {
                    worktree: worktree.clone(),
                    ..Default::default()
                }))
            });
        }
        // Best-effort kick — failure means the worker shut down.
        let _ = self.refresh_tx.send(RefreshTask {
            stream_id: stream_id.clone(),
            kinds: RefreshKinds::all(),
        });
    }

    /// Drop a stream from the service. Used when a stream is deleted.
    pub async fn deregister(&self, stream_id: &StreamId) {
        let mut inner = self.inner.write().await;
        inner.snapshots.remove(stream_id);
    }

    fn spawn_refresh_worker(svc: Arc<Self>, mut rx: mpsc::UnboundedReceiver<RefreshTask>) {
        tokio::spawn(async move {
            // Coalesce within a small debounce window so a burst of
            // fs-watch hits doesn't translate into N separate git
            // status walks.
            let debounce = Duration::from_millis(200);
            let mut pending: HashMap<StreamId, RefreshKinds> = HashMap::new();
            loop {
                let task = match rx.recv().await {
                    Some(t) => t,
                    None => break,
                };
                pending
                    .entry(task.stream_id.clone())
                    .or_default()
                    .merge(task.kinds);

                // Drain anything that's already queued, then sleep
                // briefly to let more events coalesce.
                let deadline = tokio::time::Instant::now() + debounce;
                loop {
                    let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if timeout.is_zero() {
                        break;
                    }
                    match tokio::time::timeout(timeout, rx.recv()).await {
                        Ok(Some(t)) => {
                            pending
                                .entry(t.stream_id.clone())
                                .or_default()
                                .merge(t.kinds);
                        }
                        Ok(None) => return,
                        Err(_) => break,
                    }
                }

                let drained: Vec<_> = pending.drain().collect();
                for (stream_id, kinds) in drained {
                    if !kinds.any() {
                        continue;
                    }
                    svc.do_refresh(&stream_id, kinds).await;
                }
            }
        });
    }

    fn spawn_bus_listener(svc: Arc<Self>) {
        let mut rx = svc.events.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(OxplowEvent::WorkspaceChanged { stream_id, .. }) => {
                        // Synchronously invalidate so the next reader
                        // recomputes from disk — without this, the
                        // frontend's debounced refresh can race the
                        // backend's async do_refresh and read the stale
                        // cached value.
                        svc.invalidate_statuses(&stream_id).await;
                        let _ = svc.refresh_tx.send(RefreshTask {
                            stream_id,
                            kinds: RefreshKinds {
                                statuses: true,
                                branches: false,
                                conflict: true,
                                log: false,
                                remote_branches: false,
                            },
                        });
                    }
                    Ok(OxplowEvent::GitRefsChanged { stream_id }) => {
                        // HEAD moving (commit, checkout, reset) changes
                        // the worktree's diff vs HEAD even when files
                        // don't change, so the cached status_summary
                        // must be invalidated and recomputed too.
                        svc.invalidate_statuses(&stream_id).await;
                        let _ = svc.refresh_tx.send(RefreshTask {
                            stream_id,
                            kinds: RefreshKinds {
                                statuses: true,
                                branches: true,
                                conflict: true,
                                log: true,
                                remote_branches: true,
                            },
                        });
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// Compare the worktree's live HEAD against the Stream record's
    /// stored `branch` and persist the new value if they diverged.
    ///
    /// External `git checkout`s — run while oxplow is off, or run from
    /// a terminal during a session — never touch the Stream record on
    /// their own; without this step the bottom-bar branch chip and any
    /// other consumer of `stream.branch` keep showing the branch the
    /// stream was created on. We reconcile both at seed time (covers
    /// the "switched while oxplow was off" case) and after every
    /// branch-refresh task fires (covers the in-session case driven
    /// by GitRefsChanged from the fs-watcher).
    async fn reconcile_branch(&self, stream_id: &StreamId, worktree: &Path) {
        let Some(detected) = detect_current_branch(worktree) else {
            return;
        };
        let Ok(Some(mut stored)) = self.streams.get(stream_id).await else {
            return;
        };
        if stored.branch == detected {
            return;
        }
        stored.branch = detected.clone();
        stored.branch_ref = format!("refs/heads/{detected}");
        stored.updated_at = Timestamp::now();
        if let Err(e) = self.streams.upsert(&stored).await {
            warn!(stream_id = %stream_id, error = %e, "failed to persist reconciled branch");
            return;
        }
        debug!(stream_id = %stream_id, branch = %detected, "reconciled stream branch from HEAD");
        self.events.emit(OxplowEvent::StreamsChanged);
    }

    /// Drop the cached status map / summary for `stream_id` so the next
    /// reader recomputes from disk. Called from the event-bus listener
    /// the moment a workspace or refs change is observed; the async
    /// pre-warm (via `do_refresh`) still runs in parallel to repopulate
    /// the cache before the next reader arrives.
    async fn invalidate_statuses(&self, stream_id: &StreamId) {
        let snapshot = {
            let inner = self.inner.read().await;
            match inner.snapshots.get(stream_id) {
                Some(s) => s.clone(),
                None => return,
            }
        };
        let mut snap = snapshot.write().await;
        snap.statuses = None;
        snap.status_summary = None;
    }

    async fn do_refresh(&self, stream_id: &StreamId, kinds: RefreshKinds) {
        let snapshot = {
            let inner = self.inner.read().await;
            match inner.snapshots.get(stream_id) {
                Some(s) => s.clone(),
                None => return,
            }
        };
        let worktree = snapshot.read().await.worktree.clone();
        let worktree_for_blocking = worktree.clone();
        let project_dir = self.project_dir.clone();
        let RefreshKinds {
            statuses,
            branches,
            conflict,
            log,
            remote_branches,
        } = kinds;

        let (statuses_v, branches_v, conflict_v, log_v, remote_branches_v) =
            tokio::task::spawn_blocking(move || {
                let worktree = worktree_for_blocking;
                let s = if statuses {
                    let map = oxplow_git::list_git_statuses(&worktree);
                    let summary = oxplow_git::summarize_git_statuses(&map);
                    Some((map, summary))
                } else {
                    None
                };
                let b = if branches {
                    Some(oxplow_git::list_branches(worktree.clone()))
                } else {
                    None
                };
                let c = if conflict {
                    Some(oxplow_git::get_repo_conflict_state(&worktree))
                } else {
                    None
                };
                let lg = if log {
                    Some(oxplow_git::get_git_log(
                        &worktree,
                        oxplow_git::GitLogOptions {
                            limit: Some(RECENT_LOG_LIMIT),
                            all: false,
                        },
                    ))
                } else {
                    None
                };
                let rb = if remote_branches {
                    Some(oxplow_git::list_recent_remote_branches(
                        &project_dir,
                        RECENT_REMOTE_BRANCHES_LIMIT,
                    ))
                } else {
                    None
                };
                (s, b, c, lg, rb)
            })
            .await
            .unwrap_or((None, None, None, None, None));

        let mut changed = SnapshotChanged {
            stream_id: stream_id.clone(),
            statuses: false,
            branches: false,
            conflict: false,
            log: false,
            remote_branches: false,
        };
        {
            let mut snap = snapshot.write().await;
            if let Some((map, summary)) = statuses_v {
                snap.statuses = Some(map);
                snap.status_summary = Some(summary);
                changed.statuses = true;
                // Status changes can affect ahead/behind only if HEAD
                // moves; a `git status` walk doesn't move HEAD, so we
                // leave the ahead_behind cache alone here.
            }
            if let Some(b) = branches_v {
                snap.branches = Some(b);
                changed.branches = true;
                // Refs moved → ahead/behind is stale.
                snap.ahead_behind.clear();
            }
            if let Some(c) = conflict_v {
                snap.conflict_state = Some(c);
                changed.conflict = true;
            }
            if let Some(l) = log_v {
                snap.recent_log = Some(l);
                changed.log = true;
                // HEAD moved → ahead/behind is stale.
                snap.ahead_behind.clear();
            }
            snap.last_refreshed = Some(Instant::now());
        }
        if let Some(rb) = remote_branches_v {
            let mut inner = self.inner.write().await;
            inner.recent_remote_branches = Some(rb);
            changed.remote_branches = true;
        }
        if changed.statuses
            || changed.branches
            || changed.conflict
            || changed.log
            || changed.remote_branches
        {
            let _ = self.snapshot_tx.send(changed);
        }

        // Branch-list refresh implies HEAD may have moved. Reconcile
        // the Stream record's stored branch field so consumers
        // (status-bar chip, agent prompt, etc.) follow along when an
        // external `git checkout` happens during a session.
        if branches {
            self.reconcile_branch(stream_id, &worktree).await;
        }
    }

    fn schedule(&self, stream_id: &StreamId, kinds: RefreshKinds) {
        if !kinds.any() {
            return;
        }
        let _ = self.refresh_tx.send(RefreshTask {
            stream_id: stream_id.clone(),
            kinds,
        });
    }

    /// After a write op against `stream_id`, push the renderer-facing
    /// events the affected panels listen for and queue a fresh
    /// snapshot pull so the next read is hot.
    fn announce_write(&self, stream_id: Option<&StreamId>, kinds: RefreshKinds) {
        if let Some(id) = stream_id {
            // Any branches/conflict bump implies refs may have moved,
            // which means recent log + remote branches need a re-fetch
            // too. Layer that in so callers don't have to remember.
            let mut effective = kinds;
            if kinds.branches || kinds.conflict {
                effective.log = true;
                effective.remote_branches = true;
            }
            self.schedule(id, effective);
            self.events.emit(OxplowEvent::WorkspaceChanged {
                stream_id: id.clone(),
                change_kind: crate::events::WorkspaceChangeKind::Updated,
                path: String::new(),
            });
            if kinds.branches || kinds.conflict {
                self.events.emit(OxplowEvent::GitRefsChanged {
                    stream_id: id.clone(),
                });
            }
        }
    }

    // ---------------------------------------------------------------
    // Cached reads
    // ---------------------------------------------------------------

    pub async fn status_summary(&self, stream_id: Option<&str>) -> WorkspaceStatusSummary {
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let guard = snap.read().await;
            if let Some(s) = guard.status_summary.as_ref() {
                return s.clone();
            }
        }
        // Cache miss — compute live, populate.
        let path = self.resolve_repo_dir(stream_id).await;
        let summary = tokio::task::spawn_blocking(move || {
            let m = oxplow_git::list_git_statuses(&path);
            (oxplow_git::summarize_git_statuses(&m), m)
        })
        .await
        .ok();
        let Some((summary, map)) = summary else {
            return WorkspaceStatusSummary::default();
        };
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let mut guard = snap.write().await;
            guard.statuses = Some(map);
            guard.status_summary = Some(summary.clone());
        }
        summary
    }

    pub async fn statuses(&self, stream_id: Option<&str>) -> HashMap<String, GitFileStatus> {
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let guard = snap.read().await;
            if let Some(s) = guard.statuses.as_ref() {
                return s.clone();
            }
        }
        let path = self.resolve_repo_dir(stream_id).await;
        let map = tokio::task::spawn_blocking(move || oxplow_git::list_git_statuses(&path))
            .await
            .unwrap_or_default();
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let mut guard = snap.write().await;
            guard.statuses = Some(map.clone());
            guard.status_summary = Some(oxplow_git::summarize_git_statuses(&map));
        }
        map
    }

    pub async fn branches_for(&self, stream_id: Option<&str>) -> Vec<BranchRef> {
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let guard = snap.read().await;
            if let Some(b) = guard.branches.as_ref() {
                return b.clone();
            }
        }
        let path = self.resolve_repo_dir(stream_id).await;
        let list = tokio::task::spawn_blocking(move || oxplow_git::list_branches(path))
            .await
            .unwrap_or_default();
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let mut guard = snap.write().await;
            guard.branches = Some(list.clone());
        }
        list
    }

    /// `list_branches` against the project root — used by the shared
    /// branch picker that doesn't sit inside a specific stream.
    pub async fn list_branches_project(&self) -> Vec<BranchRef> {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::list_branches(path))
            .await
            .unwrap_or_default()
    }

    pub async fn conflict_state(&self, stream_id: Option<&str>) -> RepoConflictState {
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let guard = snap.read().await;
            if let Some(c) = guard.conflict_state.as_ref() {
                return c.clone();
            }
        }
        let path = self.resolve_repo_dir(stream_id).await;
        let state = tokio::task::spawn_blocking(move || oxplow_git::get_repo_conflict_state(&path))
            .await
            .expect("conflict_state join");
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let mut guard = snap.write().await;
            guard.conflict_state = Some(state.clone());
        }
        state
    }

    // ---------------------------------------------------------------
    // Pass-through reads (no cache yet — easy to add later because
    // every caller already routes through here)
    // ---------------------------------------------------------------

    pub async fn ahead_behind(
        &self,
        stream_id: Option<&str>,
        base: String,
        head: String,
    ) -> AheadBehind {
        let key = (base.clone(), head.clone());
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let guard = snap.read().await;
            if let Some(v) = guard.ahead_behind.get(&key) {
                return v.clone();
            }
        }
        let path = self.resolve_repo_dir(stream_id).await;
        let base_for_blocking = base.clone();
        let head_for_blocking = head.clone();
        let result = tokio::task::spawn_blocking(move || {
            oxplow_git::get_ahead_behind(&path, &base_for_blocking, &head_for_blocking)
        })
        .await
        .expect("ahead_behind join");
        if let Some(snap) = self.snapshot_for(stream_id).await {
            let mut guard = snap.write().await;
            guard.ahead_behind.insert(key, result.clone());
        }
        result
    }

    pub async fn change_scopes(&self, stream_id: Option<&str>) -> ChangeScopes {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_change_scopes(&path))
            .await
            .expect("change_scopes join")
    }

    pub async fn branch_changes(&self, stream_id: Option<&str>, base_ref: String) -> BranchChanges {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::list_branch_changes(&path, &base_ref))
            .await
            .expect("branch_changes join")
    }

    pub async fn git_log(&self, stream_id: Option<&str>, opts: GitLogOptions) -> GitLogResult {
        // Cache hit: HEAD-only query whose limit fits inside the
        // pre-fetched window. The dashboard asks for limit=5; the
        // GitHistory page's first paint asks for ~50.
        if !opts.all {
            let want = opts.limit.unwrap_or(usize::MAX);
            if want <= RECENT_LOG_LIMIT {
                if let Some(snap) = self.snapshot_for(stream_id).await {
                    let guard = snap.read().await;
                    if let Some(cached) = guard.recent_log.as_ref() {
                        let take = want.min(cached.commits.len());
                        return GitLogResult {
                            commits: cached.commits[..take].to_vec(),
                            branch_heads: cached.branch_heads.clone(),
                            tags: cached.tags.clone(),
                        };
                    }
                }
            }
        }
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_git_log(&path, opts))
            .await
            .expect("git_log join")
    }

    pub async fn commit_detail(
        &self,
        stream_id: Option<&str>,
        sha: String,
    ) -> Option<CommitDetail> {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::get_commit_detail(&path, &sha))
            .await
            .unwrap_or(None)
    }

    pub async fn commits_ahead_of(
        &self,
        stream_id: Option<&str>,
        base: String,
        head: String,
        limit: usize,
    ) -> Vec<GitLogCommit> {
        let path = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || {
            oxplow_git::get_commits_ahead_of(&path, &base, &head, limit)
        })
        .await
        .unwrap_or_default()
    }

    pub async fn blame(&self, stream_id: Option<&str>, path: String) -> Vec<BlameLine> {
        let dir = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::git_blame(&dir, &path))
            .await
            .unwrap_or_default()
    }

    pub async fn local_blame(
        &self,
        stream_id: Option<&str>,
        path: String,
        disk_text: String,
    ) -> Vec<LocalBlameEntry> {
        let dir = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::local_blame(&dir, &path, &disk_text))
            .await
            .unwrap_or_default()
    }

    pub async fn list_file_commits(
        &self,
        stream_id: Option<&str>,
        path: String,
        limit: usize,
    ) -> Vec<GitLogCommit> {
        let dir = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::list_file_commits(&dir, &path, limit))
            .await
            .unwrap_or_default()
    }

    pub async fn read_file_at_ref(&self, r#ref: String, path: String) -> Option<String> {
        let project = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::read_file_at_ref(&project, &r#ref, &path))
            .await
            .unwrap_or(None)
    }

    pub async fn search_workspace_text(
        &self,
        stream_id: Option<&str>,
        query: String,
        limit: Option<usize>,
    ) -> Vec<TextSearchHit> {
        let dir = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::search_workspace_text(&dir, &query, limit))
            .await
            .unwrap_or_default()
    }

    pub async fn list_all_refs(&self) -> GroupedGitRefs {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::list_all_refs(&path))
            .await
            .expect("list_all_refs join")
    }

    pub async fn list_recent_remote_branches(&self, limit: usize) -> Vec<RemoteBranchEntry> {
        {
            let inner = self.inner.read().await;
            if let Some(cached) = inner.recent_remote_branches.as_ref() {
                if limit <= cached.len() || cached.len() < RECENT_REMOTE_BRANCHES_LIMIT {
                    let take = limit.min(cached.len());
                    return cached[..take].to_vec();
                }
            }
        }
        let path = self.project_dir.clone();
        let fetched = tokio::task::spawn_blocking(move || {
            oxplow_git::list_recent_remote_branches(&path, RECENT_REMOTE_BRANCHES_LIMIT.max(limit))
        })
        .await
        .unwrap_or_default();
        {
            let mut inner = self.inner.write().await;
            inner.recent_remote_branches = Some(fetched.clone());
        }
        let take = limit.min(fetched.len());
        fetched[..take].to_vec()
    }

    pub async fn list_existing_worktrees(&self) -> Vec<GitWorktreeEntry> {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::list_existing_worktrees(&path))
            .await
            .unwrap_or_default()
    }

    pub async fn list_adoptable_worktrees(&self, registered: Vec<String>) -> Vec<GitWorktreeEntry> {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || {
            oxplow_git::list_adoptable_worktrees(&path, &registered)
        })
        .await
        .unwrap_or_default()
    }

    pub async fn detect_default_branch(&self) -> Option<String> {
        let path = self.project_dir.clone();
        tokio::task::spawn_blocking(move || oxplow_git::detect_default_branch(&path))
            .await
            .unwrap_or(None)
    }

    // ---------------------------------------------------------------
    // Workspace-file pass-throughs (kept on the service so every git
    // touch goes through one place; cache invalidation rides the
    // fs-watch path on its own).
    // ---------------------------------------------------------------

    pub async fn list_workspace_entries(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<Vec<WorkspaceEntry>, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let statuses = self.statuses(stream_id).await;
        tokio::task::spawn_blocking(move || {
            oxplow_git::list_workspace_entries(&root, &relative_path, &statuses)
        })
        .await
        .expect("workspace entries join")
    }

    pub async fn list_workspace_files(
        &self,
        stream_id: Option<&str>,
    ) -> Result<Vec<WorkspaceIndexedFile>, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let statuses = self.statuses(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::list_workspace_files(&root, &statuses, ""))
            .await
            .expect("workspace files join")
    }

    pub async fn read_workspace_file(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<WorkspaceFile, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || oxplow_git::read_workspace_file(&root, &relative_path))
            .await
            .expect("read workspace file join")
    }

    pub async fn write_workspace_file(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
        content: String,
    ) -> Result<WorkspaceFile, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let result = tokio::task::spawn_blocking(move || {
            oxplow_git::write_workspace_file(&root, &relative_path, &content)
        })
        .await
        .expect("write workspace file join")?;
        if let Some(id) = stream_id_from(stream_id) {
            self.announce_write(
                Some(&id),
                RefreshKinds {
                    statuses: true,
                    branches: false,
                    conflict: false,
                    ..Default::default()
                },
            );
        }
        Ok(result)
    }

    pub async fn create_workspace_file(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
        content: String,
    ) -> Result<WorkspaceFile, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let result = tokio::task::spawn_blocking(move || {
            oxplow_git::create_workspace_file(&root, &relative_path, &content)
        })
        .await
        .expect("create workspace file join")?;
        if let Some(id) = stream_id_from(stream_id) {
            self.announce_write(
                Some(&id),
                RefreshKinds {
                    statuses: true,
                    branches: false,
                    conflict: false,
                    ..Default::default()
                },
            );
        }
        Ok(result)
    }

    pub async fn create_workspace_directory(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<String, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        tokio::task::spawn_blocking(move || {
            oxplow_git::create_workspace_directory(&root, &relative_path)
        })
        .await
        .expect("create workspace dir join")
    }

    pub async fn rename_workspace_path(
        &self,
        stream_id: Option<&str>,
        from_path: String,
        to_path: String,
    ) -> Result<(String, String), oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let result = tokio::task::spawn_blocking(move || {
            oxplow_git::rename_workspace_path(&root, &from_path, &to_path)
        })
        .await
        .expect("rename workspace path join")?;
        if let Some(id) = stream_id_from(stream_id) {
            self.announce_write(
                Some(&id),
                RefreshKinds {
                    statuses: true,
                    branches: false,
                    conflict: false,
                    ..Default::default()
                },
            );
        }
        Ok(result)
    }

    pub async fn delete_workspace_path(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<String, oxplow_git::WorkspaceError> {
        let root = self.resolve_repo_dir(stream_id).await;
        let result = tokio::task::spawn_blocking(move || {
            oxplow_git::delete_workspace_path(&root, &relative_path)
        })
        .await
        .expect("delete workspace path join")?;
        if let Some(id) = stream_id_from(stream_id) {
            self.announce_write(
                Some(&id),
                RefreshKinds {
                    statuses: true,
                    branches: false,
                    conflict: false,
                    ..Default::default()
                },
            );
        }
        Ok(result)
    }

    // ---------------------------------------------------------------
    // Mutating ops — pass through to oxplow_git, then refresh + bus.
    // ---------------------------------------------------------------

    pub async fn commit_all(
        &self,
        stream_id: Option<&str>,
        message: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result = run_blocking(move || oxplow_git::commit_all(&path, &message)).await?;
        self.announce_write(
            stream_id_from(stream_id).as_ref(),
            RefreshKinds {
                statuses: true,
                branches: true,
                conflict: true,
                ..Default::default()
            },
        );
        Ok(result)
    }

    pub async fn add_path(
        &self,
        stream_id: Option<&str>,
        relpath: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result = run_blocking(move || oxplow_git::add_path(&path, &relpath)).await?;
        self.announce_write(
            stream_id_from(stream_id).as_ref(),
            RefreshKinds {
                statuses: true,
                branches: false,
                conflict: false,
                ..Default::default()
            },
        );
        Ok(result)
    }

    pub async fn restore_path(
        &self,
        stream_id: Option<&str>,
        relpath: String,
    ) -> std::io::Result<()> {
        let path = self.resolve_repo_dir(stream_id).await;
        run_blocking(move || oxplow_git::restore_path(&path, &relpath)).await?;
        self.announce_write(
            stream_id_from(stream_id).as_ref(),
            RefreshKinds {
                statuses: true,
                branches: false,
                conflict: false,
                ..Default::default()
            },
        );
        Ok(())
    }

    pub async fn fetch(
        &self,
        stream_id: Option<&str>,
        remote: Option<String>,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result = run_blocking(move || oxplow_git::fetch(&path, remote.as_deref())).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), RefreshKinds::all());
        Ok(result)
    }

    pub async fn pull(&self, stream_id: Option<&str>) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result = run_blocking(move || oxplow_git::pull(&path)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), RefreshKinds::all());
        Ok(result)
    }

    pub async fn pull_remote_into_current(
        &self,
        stream_id: Option<&str>,
        remote: String,
        branch: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result =
            run_blocking(move || oxplow_git::pull_remote_into_current(&path, &remote, &branch))
                .await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), RefreshKinds::all());
        Ok(result)
    }

    pub async fn push(&self, stream_id: Option<&str>) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result = run_blocking(move || oxplow_git::push(&path)).await?;
        // Push doesn't change local refs; just nudge the renderer in
        // case it cares about ahead/behind.
        self.announce_write(
            stream_id_from(stream_id).as_ref(),
            RefreshKinds {
                statuses: false,
                branches: true,
                conflict: false,
                ..Default::default()
            },
        );
        Ok(result)
    }

    pub async fn push_current_to(
        &self,
        stream_id: Option<&str>,
        remote: String,
        branch: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result =
            run_blocking(move || oxplow_git::push_current_to(&path, &remote, &branch)).await?;
        self.announce_write(
            stream_id_from(stream_id).as_ref(),
            RefreshKinds {
                statuses: false,
                branches: true,
                conflict: false,
                ..Default::default()
            },
        );
        Ok(result)
    }

    pub async fn merge(
        &self,
        stream_id: Option<&str>,
        source: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result = run_blocking(move || oxplow_git::merge(&path, &source)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), RefreshKinds::all());
        Ok(result)
    }

    pub async fn rebase(
        &self,
        stream_id: Option<&str>,
        onto: String,
    ) -> std::io::Result<GitOpResult> {
        let path = self.resolve_repo_dir(stream_id).await;
        let result = run_blocking(move || oxplow_git::rebase(&path, &onto)).await?;
        self.announce_write(stream_id_from(stream_id).as_ref(), RefreshKinds::all());
        Ok(result)
    }

    pub async fn rename_branch(
        &self,
        from: String,
        to: String,
    ) -> Result<(), oxplow_git::BranchOpError> {
        let path = self.project_dir.clone();
        run_blocking_branch(move || oxplow_git::rename_branch(&path, &from, &to)).await?;
        // Affects every registered stream (the project root's refs).
        self.broadcast_refs_change_all().await;
        Ok(())
    }

    pub async fn delete_branch(
        &self,
        branch: String,
        force: bool,
    ) -> Result<(), oxplow_git::BranchOpError> {
        let path = self.project_dir.clone();
        run_blocking_branch(move || oxplow_git::delete_branch(&path, &branch, force)).await?;
        self.broadcast_refs_change_all().await;
        Ok(())
    }

    pub async fn append_to_gitignore(
        &self,
        stream_id: Option<&str>,
        entry: String,
    ) -> std::io::Result<()> {
        let path = self.resolve_repo_dir(stream_id).await;
        run_blocking(move || oxplow_git::append_to_gitignore(&path, &entry)).await?;
        self.announce_write(
            stream_id_from(stream_id).as_ref(),
            RefreshKinds {
                statuses: true,
                branches: false,
                conflict: false,
                ..Default::default()
            },
        );
        Ok(())
    }

    async fn snapshot_for(&self, stream_id: Option<&str>) -> Option<Arc<RwLock<StreamSnapshot>>> {
        let id = stream_id?;
        let inner = self.inner.read().await;
        inner.snapshots.get(&StreamId::from(id)).cloned()
    }

    async fn broadcast_refs_change_all(&self) {
        let ids: Vec<StreamId> = {
            let inner = self.inner.read().await;
            inner.snapshots.keys().cloned().collect()
        };
        for id in ids {
            self.schedule(
                &id,
                RefreshKinds {
                    statuses: false,
                    branches: true,
                    conflict: true,
                    ..Default::default()
                },
            );
            self.events
                .emit(OxplowEvent::GitRefsChanged { stream_id: id });
        }
    }
}

fn stream_id_from(s: Option<&str>) -> Option<StreamId> {
    s.map(StreamId::from)
}

fn resolve_worktree(project_dir: &Path, recorded: &str) -> PathBuf {
    let raw = PathBuf::from(recorded);
    if raw.is_absolute() {
        raw
    } else {
        project_dir.join(raw)
    }
}

async fn run_blocking<R>(
    f: impl FnOnce() -> std::io::Result<R> + Send + 'static,
) -> std::io::Result<R>
where
    R: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "git op join failed");
            Err(std::io::Error::other(e.to_string()))
        }
    }
}

async fn run_blocking_branch<R>(
    f: impl FnOnce() -> Result<R, oxplow_git::BranchOpError> + Send + 'static,
) -> Result<R, oxplow_git::BranchOpError>
where
    R: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .expect("branch op join")
}

#[cfg(debug_assertions)]
#[allow(dead_code)]
fn _trace(s: &str) {
    debug!(target: "git_service", "{s}");
}
