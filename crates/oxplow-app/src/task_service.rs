//! TaskService — orchestration over the Task store.
//!
//! Encapsulates the create/update/reorder/move use-cases. The store
//! itself is a thin row-CRUD layer; everything that requires composing
//! reads and writes (e.g. computing the next sort_index, transitioning
//! status with the associated timestamp side-effects, moving a task
//! between thread and backlog) lives here.
//!
//! The service does not emit events itself — the Tauri command layer
//! does, after a successful service call. That keeps `oxplow-app`
//! independent of the tauri-specta layering and lets the MCP surface
//! reuse the same service without paying for renderer notifications.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_db::SqliteTaskStore;
use oxplow_db::{EffortFileChange, SqliteTaskEffortStore, TaskEffortStore};
use oxplow_domain::stores::{TaskLinkStore, TaskStore};
use oxplow_domain::{
    DomainError, Task, TaskActorKind, TaskAuthor, TaskId, TaskImpact, TaskLinkType, TaskPriority,
    TaskStatus, ThreadId, Timestamp,
};

#[derive(Debug, Error)]
pub enum TaskServiceError {
    #[error("task not found: {0}")]
    NotFound(TaskId),
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

async fn item_is_blocked(
    id: TaskId,
    link_store: &dyn TaskLinkStore,
    by_id: &std::collections::HashMap<TaskId, Task>,
) -> Result<bool, DomainError> {
    let incoming = link_store.list_incoming(id).await?;
    for link in incoming {
        if !matches!(link.link_type, TaskLinkType::Blocks) {
            continue;
        }
        if let Some(blocker) = by_id.get(&link.from_item_id) {
            if !matches!(
                blocker.status,
                TaskStatus::Done | TaskStatus::Canceled | TaskStatus::Archived
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Discriminated result for `read_task_options`. The shape mirrors
/// main's TS contract so the agent-side skill text stays accurate
/// without a translation layer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "mode", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ReadWorkOptionsResult {
    Empty,
    Epic { epic: Task, children: Vec<Task> },
    Standalone { items: Vec<Task> },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CreateTaskInput {
    pub title: String,
    pub description: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub parent_id: Option<TaskId>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub category: Option<String>,
    pub tags: Option<String>,
    pub author: Option<TaskAuthor>,
}

/// Partial-patch for `update_task`. Each `Option` follows
/// "missing -> keep, present -> replace" semantics. `category` and
/// `tags` use a wrapping `Option<Option<…>>`-via-helper pattern to
/// distinguish "keep" from "clear"; in this struct, `null` clears and
/// missing keeps.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct UpdateTaskChanges {
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance_criteria: Option<Option<String>>,
    pub parent_id: Option<Option<TaskId>>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub category: Option<Option<String>>,
    pub tags: Option<Option<String>>,
}

#[derive(Clone)]
pub struct TaskService {
    store: Arc<SqliteTaskStore>,
    /// Optional. When set, `update()` opens/closes an effort row on
    /// `in_progress` entry/exit. Held as an `Option` so test paths
    /// that construct a TaskService without the full Services boot
    /// still work — they just skip the lifecycle effort.
    effort_store: Option<Arc<SqliteTaskEffortStore>>,
    /// Optional. When set alongside `effort_store`, `update()` calls
    /// `request_snapshot()` on in_progress transitions and stamps
    /// the returned id onto the effort row.
    snapshot_capture: Option<Arc<crate::snapshot_capture::SnapshotCaptureService>>,
}

/// Returns true iff any item in `items` has this id as its parent_id.
fn is_epic(item: &Task, items: &[Task]) -> bool {
    items.iter().any(|c| c.parent_id == Some(item.id))
}

impl TaskService {
    pub fn new(store: Arc<SqliteTaskStore>) -> Self {
        Self {
            store,
            effort_store: None,
            snapshot_capture: None,
        }
    }

    /// Attach the effort store. Required (together with
    /// `with_snapshot_capture`) for automatic effort lifecycle on
    /// in_progress transitions.
    pub fn with_effort_store(mut self, store: Arc<SqliteTaskEffortStore>) -> Self {
        self.effort_store = Some(store);
        self
    }

    /// Attach the snapshot manager. When present alongside
    /// `effort_store`, `update()` triggers `request_snapshot()` on
    /// in_progress entry / exit and stamps the result onto the
    /// effort row.
    pub fn with_snapshot_capture(
        mut self,
        svc: Arc<crate::snapshot_capture::SnapshotCaptureService>,
    ) -> Self {
        self.snapshot_capture = Some(svc);
        self
    }

    /// Create a task attached to `thread` (or to the backlog if
    /// `thread` is `None`). Allocates a fresh id and sort_index.
    pub async fn create(
        &self,
        thread: Option<ThreadId>,
        input: CreateTaskInput,
    ) -> Result<Task, TaskServiceError> {
        let next_sort = self.next_sort_index(thread.as_ref()).await?;
        let now = Timestamp::now();
        let mut item = Task {
            // id assigned by store.insert
            id: TaskId::placeholder(),
            thread_id: thread,
            parent_id: input.parent_id,
            title: input.title,
            description: input.description.unwrap_or_default(),
            acceptance_criteria: input.acceptance_criteria,
            status: input.status.unwrap_or(TaskStatus::Ready),
            priority: input.priority.unwrap_or(TaskPriority::Medium),
            sort_index: next_sort,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: input.author.or(Some(TaskAuthor::User)),
            category: input.category,
            tags: input.tags,
        };
        let id = self.store.insert(&item).await?;
        item.id = id;
        // Filing directly in `in_progress` (the path CLAUDE.md
        // recommends to "start the work in the same call") needs the
        // same lifecycle hook that update() runs on a Ready →
        // InProgress transition — otherwise complete_task's EffortEnd
        // snapshot has no open effort to land on and gets orphaned.
        if matches!(item.status, TaskStatus::InProgress) {
            self.apply_lifecycle_snapshot(&item, true).await;
        }
        Ok(item)
    }

    /// Apply a partial-patch to an existing task. Returns the
    /// post-patch row.
    pub async fn update(
        &self,
        id: TaskId,
        changes: UpdateTaskChanges,
    ) -> Result<Task, TaskServiceError> {
        let mut item = self.load(id).await?;
        let prior_status = item.status;
        if let Some(t) = changes.title {
            item.title = t;
        }
        if let Some(d) = changes.description {
            item.description = d;
        }
        if let Some(ac) = changes.acceptance_criteria {
            item.acceptance_criteria = ac;
        }
        if let Some(p) = changes.parent_id {
            item.parent_id = p;
        }
        if let Some(s) = changes.status {
            // Transitioning to/from `done` flips completed_at.
            if matches!(s, TaskStatus::Done) && item.status != TaskStatus::Done {
                item.completed_at = Some(Timestamp::now());
            } else if matches!(item.status, TaskStatus::Done) && !matches!(s, TaskStatus::Done) {
                item.completed_at = None;
            }
            item.status = s;
        }
        if let Some(p) = changes.priority {
            item.priority = p;
        }
        if let Some(c) = changes.category {
            item.category = c;
        }
        if let Some(t) = changes.tags {
            item.tags = t;
        }
        item.updated_at = Timestamp::now();
        self.store.update(&item).await?;

        // Effort lifecycle: when a task crosses the `in_progress`
        // boundary, request a snapshot and open/close an effort row
        // pinned to it. The snapshot+store hooks are optional so
        // bare TaskService tests (no Services boot) skip this path.
        let crossed_in =
            prior_status != TaskStatus::InProgress && item.status == TaskStatus::InProgress;
        let crossed_out =
            prior_status == TaskStatus::InProgress && item.status != TaskStatus::InProgress;
        if crossed_in || crossed_out {
            self.apply_lifecycle_snapshot(&item, crossed_in).await;
        }
        Ok(item)
    }

    /// Triggered from `update()` when the task just crossed the
    /// in_progress boundary. On entry: request a snapshot and open
    /// a new effort row anchored to it. On exit: request a snapshot,
    /// find the still-open effort for this task, and finish it with
    /// the end snapshot id. All errors are logged + swallowed —
    /// status persistence already succeeded and we don't want a
    /// snapshot failure to roll that back.
    async fn apply_lifecycle_snapshot(&self, item: &Task, entering: bool) {
        let (Some(snapshot), Some(effort_store)) =
            (self.snapshot_capture.as_ref(), self.effort_store.as_ref())
        else {
            return;
        };
        // Lifecycle efforts need a thread to attach to; tasks on
        // the project-wide backlog skip snapshot pinning.
        let Some(thread_id) = item.thread_id.clone() else {
            return;
        };
        let source = if entering {
            crate::events::SnapshotSourceKind::EffortStart
        } else {
            crate::events::SnapshotSourceKind::EffortEnd
        };
        let snap_id = match snapshot.request_snapshot(source).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, task = %item.id, "effort lifecycle: snapshot failed");
                return;
            }
        };
        if entering {
            if let Err(e) = effort_store.start(item.id, &thread_id, snap_id).await {
                tracing::warn!(error = %e, task = %item.id, "effort lifecycle: start failed");
            }
        } else {
            let open = match effort_store.find_open_for_task(item.id).await {
                Ok(open) => open,
                Err(e) => {
                    tracing::warn!(error = %e, task = %item.id, "effort lifecycle: open lookup failed");
                    return;
                }
            };
            if let Some(open) = open {
                if let Err(e) = effort_store.finish(&open.id, snap_id, None).await {
                    tracing::warn!(error = %e, task = %item.id, "effort lifecycle: finish failed");
                }
            } else {
                tracing::debug!(task = %item.id, "effort lifecycle: no open effort to finish");
            }
        }
    }

    /// Rewrite sort_index across the items in `thread` (or backlog if
    /// `thread` is None) according to the supplied order. Items not
    /// included keep their existing sort_index.
    pub async fn reorder(
        &self,
        thread: Option<&ThreadId>,
        order: &[TaskId],
    ) -> Result<(), TaskServiceError> {
        let now = Timestamp::now();
        for (idx, id) in order.iter().enumerate() {
            let mut item = self.load(*id).await?;
            // Only reorder items in the right scope.
            if item.thread_id.as_ref() != thread {
                continue;
            }
            item.sort_index = idx as i64;
            item.updated_at = now;
            self.store.update(&item).await?;
        }
        Ok(())
    }

    /// Move a task to a different thread (or to the backlog with
    /// `dest = None`). Reallocates sort_index at the destination tail.
    pub async fn move_to(
        &self,
        id: TaskId,
        dest: Option<ThreadId>,
    ) -> Result<Task, TaskServiceError> {
        let mut item = self.load(id).await?;
        let next_sort = self.next_sort_index(dest.as_ref()).await?;
        item.thread_id = dest;
        item.sort_index = next_sort;
        item.updated_at = Timestamp::now();
        self.store.update(&item).await?;
        Ok(item)
    }

    pub async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<Task>, TaskServiceError> {
        Ok(self.store.list_for_thread(thread).await?)
    }

    /// Open + record + close an effort for `item` against `thread`.
    /// Declared `impacts` are persisted before finish so the
    /// page_ref projection runs once with the full payload.
    ///
    /// `worktree_root`, when supplied, lets the store classify each
    /// touched file as `Deleted` (file no longer on disk) vs.
    /// `Updated` (file still present). Without a baseline snapshot
    /// "Created" can't be distinguished from "Updated" by stat
    /// alone, so callers needing that signal should declare it via
    /// `impacts` (`{kind:"file", action:"created"}`). Pass `None`
    /// from tests / callers that don't have a worktree handle — the
    /// store falls back to `Updated` for every path, matching the
    /// pre-change behavior.
    // Each parameter is doing distinct semantic work — bundling
    // into a struct would hide that without buying anything.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_effort(
        &self,
        effort_store: &SqliteTaskEffortStore,
        item: TaskId,
        thread: &ThreadId,
        touched_files: &[String],
        summary: Option<String>,
        impacts: &[TaskImpact],
        worktree_root: Option<&Path>,
    ) -> Result<(), TaskServiceError> {
        // Attach to the most-recent effort row for this task — that's
        // the lifecycle effort that `update()` opened on in_progress
        // entry and (typically) closed on exit. If none exists (e.g.
        // a task filed directly into `done` with touched_files), open
        // a fresh atomic effort.
        let existing = effort_store.most_recent_for_task(item).await?;
        let effort = match existing {
            Some(e) => e,
            None => effort_store.start(item, thread, None).await?,
        };
        for path in touched_files {
            if path.is_empty() {
                continue;
            }
            let change = classify_change(worktree_root, path);
            effort_store.record_file(&effort.id, path, change).await?;
        }
        if !impacts.is_empty() {
            effort_store.set_impacts(&effort.id, impacts).await?;
        }
        if effort.ended_at.is_none() {
            // Still open (no lifecycle close happened, or this is
            // the freshly-started fallback). Close it now with the
            // summary; end_snapshot_id stays NULL because record_effort
            // is summary/files attribution, not a status transition.
            effort_store.finish(&effort.id, None, summary).await?;
        } else if summary.is_some() {
            // Lifecycle finish already closed the row but left
            // summary NULL — backfill it.
            effort_store.set_summary(&effort.id, summary).await?;
        }
        Ok(())
    }

    pub async fn list_backlog(&self) -> Result<Vec<Task>, TaskServiceError> {
        Ok(self.store.list_backlog().await?)
    }

    /// Return the next dispatch unit for the orchestrator.
    pub async fn read_task_options(
        &self,
        thread: &ThreadId,
        link_store: &dyn TaskLinkStore,
    ) -> Result<ReadWorkOptionsResult, TaskServiceError> {
        let all = self.store.list_for_thread(thread).await?;
        let by_id: std::collections::HashMap<TaskId, Task> =
            all.iter().map(|i| (i.id, i.clone())).collect();

        let mut ready: Vec<Task> = all
            .iter()
            .filter(|i| i.status == TaskStatus::Ready)
            .cloned()
            .collect();
        ready.sort_by_key(|i| (i.sort_index, i.created_at));

        let mut unblocked_ready: Vec<Task> = Vec::new();
        for item in &ready {
            if !item_is_blocked(item.id, link_store, &by_id).await? {
                unblocked_ready.push(item.clone());
            }
        }

        let Some(head) = unblocked_ready.first().cloned() else {
            return Ok(ReadWorkOptionsResult::Empty);
        };

        if is_epic(&head, &all) {
            let mut children: Vec<Task> = Vec::new();
            let mut frontier = vec![head.id];
            while let Some(parent_id) = frontier.pop() {
                for it in &all {
                    if it.parent_id == Some(parent_id) {
                        if it.status == TaskStatus::Ready
                            && !item_is_blocked(it.id, link_store, &by_id).await?
                        {
                            children.push(it.clone());
                        }
                        frontier.push(it.id);
                    }
                }
            }
            children.sort_by_key(|i| (i.sort_index, i.created_at));
            return Ok(ReadWorkOptionsResult::Epic {
                epic: head,
                children,
            });
        }

        let standalone: Vec<Task> = unblocked_ready
            .into_iter()
            .filter(|i| !is_epic(i, &all))
            .collect();
        Ok(ReadWorkOptionsResult::Standalone { items: standalone })
    }

    pub async fn soft_delete(&self, id: TaskId) -> Result<(), TaskServiceError> {
        self.store.soft_delete(id).await?;
        Ok(())
    }

    async fn load(&self, id: TaskId) -> Result<Task, TaskServiceError> {
        self.store
            .get(id)
            .await?
            .ok_or(TaskServiceError::NotFound(id))
    }

    async fn next_sort_index(&self, thread: Option<&ThreadId>) -> Result<i64, TaskServiceError> {
        let items = match thread {
            Some(t) => self.store.list_for_thread(t).await?,
            None => self.store.list_backlog().await?,
        };
        Ok(items.iter().map(|i| i.sort_index).max().unwrap_or(-1) + 1)
    }
}

/// Classify how a path changed during an effort by stat-ing the
/// worktree. Without a baseline snapshot we can't reliably tell
/// "created" apart from "updated" (the agent might have edited a
/// pre-existing file too), so this returns:
///
///  - `Deleted` if the file is missing on disk now
///  - `Updated` if the file is present (the dominant case)
///
/// Agents that want explicit "created" attribution should declare
/// it via the `impacts` parameter on `complete_task`. Returns
/// `Updated` when `worktree_root` is `None` so test fixtures that
/// don't carry a real worktree keep their old behavior.
fn classify_change(worktree_root: Option<&Path>, path: &str) -> EffortFileChange {
    let Some(root) = worktree_root else {
        return EffortFileChange::Updated;
    };
    let resolved = root.join(path);
    match std::fs::symlink_metadata(&resolved) {
        Ok(_) => EffortFileChange::Updated,
        Err(_) => EffortFileChange::Deleted,
    }
}

/// The bucketed view the Backlog page renders.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BacklogState {
    pub items: Vec<Task>,
    pub waiting: Vec<Task>,
    pub in_progress: Vec<Task>,
    pub done: Vec<Task>,
}

impl BacklogState {
    pub fn from_rows(rows: Vec<Task>) -> Self {
        let mut items = Vec::new();
        let mut waiting = Vec::new();
        let mut in_progress = Vec::new();
        let mut done = Vec::new();
        for r in rows {
            match r.status {
                TaskStatus::InProgress => in_progress.push(r),
                TaskStatus::Done | TaskStatus::Canceled | TaskStatus::Archived => done.push(r),
                TaskStatus::Blocked => waiting.push(r),
                TaskStatus::Ready => items.push(r),
            }
        }
        Self {
            items,
            waiting,
            in_progress,
            done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadStatus};

    #[test]
    fn classify_change_defaults_to_updated_without_worktree() {
        // No worktree → caller (test or a path that hasn't plumbed
        // the root yet) gets the same behavior as before the
        // detection landed.
        assert_eq!(
            classify_change(None, "src/anything.rs"),
            EffortFileChange::Updated
        );
    }

    #[test]
    fn classify_change_detects_deletion() {
        let tmp = tempfile::tempdir().unwrap();
        // File doesn't exist → Deleted.
        assert_eq!(
            classify_change(Some(tmp.path()), "missing.rs"),
            EffortFileChange::Deleted
        );
        // File exists → Updated (we can't tell created from
        // modified without a baseline snapshot).
        let real = tmp.path().join("real.rs");
        std::fs::write(&real, "fn main() {}").unwrap();
        assert_eq!(
            classify_change(Some(tmp.path()), "real.rs"),
            EffortFileChange::Updated
        );
    }

    #[test]
    fn classify_change_treats_symlink_as_present() {
        // Even a broken symlink reports via symlink_metadata, so the
        // path is "present" from the agent's point of view —
        // resolving the link is a deletion concern.
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink("nowhere", &link).unwrap();
        #[cfg(not(unix))]
        {
            let _ = link;
            return;
        }
        assert_eq!(
            classify_change(Some(tmp.path()), "link"),
            EffortFileChange::Updated
        );
    }

    async fn fixture() -> (TaskService, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let store = Arc::new(SqliteTaskStore::new(db));
        let s = Stream {
            id: StreamId::from("s-1"),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::from("b-1"),
            stream_id: s.id.clone(),
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        (TaskService::new(store), t.id)
    }

    async fn fixture_with_lifecycle() -> (
        TaskService,
        ThreadId,
        Arc<SqliteTaskEffortStore>,
        tempfile::TempDir,
    ) {
        let project = tempfile::tempdir().unwrap();
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let task_store = Arc::new(SqliteTaskStore::new(db.clone()));
        let effort_store = Arc::new(SqliteTaskEffortStore::new(db.clone()));
        let snapshot_store = Arc::new(oxplow_db::SqliteSnapshotStore::new(db.clone()));
        let blobs = crate::blob_store::BlobStore::new(project.path().join(".oxplow/snapshots"));
        let s = Stream {
            id: StreamId::from("s-1"),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: project.path().to_string_lossy().into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let snapshot_svc = Arc::new(
            crate::snapshot_capture::SnapshotCaptureService::new(
                snapshot_store,
                blobs,
                project.path().to_path_buf(),
                s.id.clone(),
                1_000_000,
            )
            // Tests bypass the settle gate; the gate is independently
            // covered in `snapshot_capture::tests::settle_window_*`.
            .with_settle_duration(std::time::Duration::ZERO),
        );
        let t = Thread {
            id: ThreadId::from("b-life"),
            stream_id: s.id.clone(),
            title: "t".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        let svc = TaskService::new(task_store)
            .with_effort_store(effort_store.clone())
            .with_snapshot_capture(snapshot_svc);
        (svc, t.id, effort_store, project)
    }

    #[tokio::test]
    async fn in_progress_transition_opens_effort_with_start_snapshot() {
        let (svc, tid, effort_store, _project) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "lifecycle".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Ready → InProgress: opens an effort with start_snapshot_id.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let open = effort_store
            .find_open_for_task(item.id)
            .await
            .unwrap()
            .expect("effort should be open");
        // Dirty set is empty in tests (no actual fs writes), so the
        // first snapshot returns None. The effort still opens but
        // start_snapshot_id is None — that's the "nothing to pin"
        // case and is fine. To verify the snapshot path actually
        // ran, write a file first.
        assert!(open.ended_at.is_none());
        assert!(open.start_snapshot_id.is_none());

        // Mark a file dirty so the next request_snapshot produces
        // a non-empty result.
        let svc_for_dirty = svc.snapshot_capture.as_ref().unwrap().clone();
        std::fs::write(_project.path().join("a.txt"), "v").unwrap();
        svc_for_dirty.mark_dirty(
            _project.path().join("a.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );

        // InProgress → Done: closes the open effort with end_snapshot_id.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let efforts = effort_store.list_for_item(item.id).await.unwrap();
        assert_eq!(efforts.len(), 1);
        let closed = &efforts[0];
        assert!(closed.ended_at.is_some());
        assert!(closed.end_snapshot_id.is_some());
        // And no new effort was opened.
        assert!(effort_store
            .find_open_for_task(item.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn create_with_in_progress_opens_lifecycle_effort() {
        // Filing a task directly in `in_progress` (the path CLAUDE.md
        // recommends for "start the work in the same call") must run
        // the lifecycle hook — otherwise complete_task's TaskEnd
        // snapshot has no open effort to attach to and the snapshot
        // is orphaned.
        let (svc, tid, effort_store, _project) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "born running".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let open = effort_store
            .find_open_for_task(item.id)
            .await
            .unwrap()
            .expect("lifecycle effort should be open after in_progress create");
        assert!(open.ended_at.is_none());
    }

    #[tokio::test]
    async fn create_with_done_skips_effort_lifecycle() {
        // Filing directly in a terminal status (e.g. retroactively
        // logging completed work) must NOT open a lifecycle effort —
        // record_effort handles that synthesis itself, with the
        // touched_files payload.
        let (svc, tid, effort_store, _project) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "retro".into(),
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(effort_store
            .find_open_for_task(item.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn record_effort_merges_into_lifecycle_effort() {
        let (svc, tid, effort_store, _project) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "merge".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Open the lifecycle effort.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Close it.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Now record_effort comes in with touched files + summary.
        // It should attach to the already-closed lifecycle effort,
        // NOT create a second row.
        svc.record_effort(
            &effort_store,
            item.id,
            &tid,
            &["src/x.rs".to_string()],
            Some("did the thing".into()),
            &[],
            None,
        )
        .await
        .unwrap();
        let efforts = effort_store.list_for_item(item.id).await.unwrap();
        assert_eq!(efforts.len(), 1, "should still be a single effort row");
        let row = &efforts[0];
        assert_eq!(row.summary.as_deref(), Some("did the thing"));
        let files = effort_store.list_files(&row.id).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/x.rs");
    }

    #[tokio::test]
    async fn record_effort_creates_fresh_effort_when_no_lifecycle() {
        let (svc, tid, effort_store, _project) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "direct".into(),
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // No lifecycle ran — task filed directly as done.
        svc.record_effort(
            &effort_store,
            item.id,
            &tid,
            &["a.rs".to_string()],
            Some("retro".into()),
            &[],
            None,
        )
        .await
        .unwrap();
        let efforts = effort_store.list_for_item(item.id).await.unwrap();
        assert_eq!(efforts.len(), 1);
        assert!(efforts[0].ended_at.is_some());
        assert_eq!(efforts[0].summary.as_deref(), Some("retro"));
    }

    #[tokio::test]
    async fn non_in_progress_transitions_skip_effort_lifecycle() {
        let (svc, tid, effort_store, _project) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "skip".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Ready → Blocked: no effort row.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Blocked),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(effort_store
            .list_for_item(item.id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn create_assigns_increasing_sort_index() {
        let (svc, tid) = fixture().await;
        let a = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "a".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let b = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "b".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(a.sort_index, 0);
        assert_eq!(b.sort_index, 1);
    }

    #[tokio::test]
    async fn update_title_keeps_other_fields() {
        let (svc, tid) = fixture().await;
        let it = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "before".into(),
                    description: Some("desc".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = svc
            .update(
                it.id,
                UpdateTaskChanges {
                    title: Some("after".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.title, "after");
        assert_eq!(updated.description, "desc");
    }

    #[tokio::test]
    async fn transition_to_done_sets_completed_at() {
        let (svc, tid) = fixture().await;
        let it = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(it.completed_at.is_none());
        let done = svc
            .update(
                it.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(done.completed_at.is_some());
        let reopened = svc
            .update(
                done.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(reopened.completed_at.is_none());
    }

    #[tokio::test]
    async fn move_to_backlog_clears_thread_id_and_resorts() {
        let (svc, tid) = fixture().await;
        let it = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let moved = svc.move_to(it.id, None).await.unwrap();
        assert!(moved.thread_id.is_none());
        let bl = svc.list_backlog().await.unwrap();
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].id, it.id);
    }

    #[tokio::test]
    async fn reorder_rewrites_indices() {
        let (svc, tid) = fixture().await;
        let a = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "a".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let b = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "b".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let c = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "c".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // c, a, b
        svc.reorder(Some(&tid), &[c.id, a.id, b.id]).await.unwrap();
        let list = svc.list_for_thread(&tid).await.unwrap();
        let order: Vec<_> = list.iter().map(|i| i.id).collect();
        assert_eq!(order, vec![c.id, a.id, b.id]);
    }

    #[test]
    fn backlog_state_buckets_by_status() {
        let now = Timestamp::from_unix_ms(1);
        let mk = |id: i64, status| Task {
            id: TaskId::new(id),
            thread_id: None,
            parent_id: None,
            title: id.to_string(),
            description: String::new(),
            acceptance_criteria: None,
            status,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
            category: None,
            tags: None,
        };
        let rows = vec![
            mk(1, TaskStatus::Ready),
            mk(2, TaskStatus::InProgress),
            mk(3, TaskStatus::Done),
            mk(4, TaskStatus::Blocked),
        ];
        let st = BacklogState::from_rows(rows);
        assert_eq!(st.items.len(), 1);
        assert_eq!(st.in_progress.len(), 1);
        assert_eq!(st.done.len(), 1);
        assert_eq!(st.waiting.len(), 1);
    }

    #[test]
    fn backlog_state_collapses_canceled_and_archived_into_done() {
        let now = Timestamp::from_unix_ms(1);
        let mk = |id: i64, status| Task {
            id: TaskId::new(id),
            thread_id: None,
            parent_id: None,
            title: id.to_string(),
            description: String::new(),
            acceptance_criteria: None,
            status,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
            category: None,
            tags: None,
        };
        let st = BacklogState::from_rows(vec![
            mk(1, TaskStatus::Done),
            mk(2, TaskStatus::Canceled),
            mk(3, TaskStatus::Archived),
        ]);
        assert_eq!(st.done.len(), 3);
        assert!(st.items.is_empty());
        assert!(st.in_progress.is_empty());
        assert!(st.waiting.is_empty());
    }

    #[test]
    fn backlog_state_empty_input() {
        let st = BacklogState::from_rows(vec![]);
        assert!(
            st.items.is_empty()
                && st.waiting.is_empty()
                && st.in_progress.is_empty()
                && st.done.is_empty()
        );
    }

    // ---- read_task_options edge cases ----

    async fn link_store_fixture() -> (TaskService, oxplow_db::SqliteTaskLinkStore, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let store = Arc::new(SqliteTaskStore::new(db.clone()));
        let link_store = oxplow_db::SqliteTaskLinkStore::new(db.clone());
        let s = Stream {
            id: StreamId::from("s-1"),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::from("b-1"),
            stream_id: s.id.clone(),
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        (TaskService::new(store), link_store, t.id)
    }

    #[tokio::test]
    async fn read_work_options_empty_when_no_ready_items() {
        let (svc, links, tid) = link_store_fixture().await;
        let a = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "in flight".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        svc.update(
            a.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let result = svc.read_task_options(&tid, &links).await.unwrap();
        assert!(matches!(result, ReadWorkOptionsResult::Empty));
    }

    #[tokio::test]
    async fn read_work_options_returns_standalone_for_plain_task() {
        let (svc, links, tid) = link_store_fixture().await;
        svc.create(
            Some(tid.clone()),
            CreateTaskInput {
                title: "ready task".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let result = svc.read_task_options(&tid, &links).await.unwrap();
        match result {
            ReadWorkOptionsResult::Standalone { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].title, "ready task");
            }
            other => panic!("expected Standalone, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_work_options_returns_epic_with_ready_children() {
        let (svc, links, tid) = link_store_fixture().await;
        let epic = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "the epic".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let _child_a = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "child A".into(),
                    parent_id: Some(epic.id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let _child_b = svc
            .create(
                Some(tid.clone()),
                CreateTaskInput {
                    title: "child B".into(),
                    parent_id: Some(epic.id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let result = svc.read_task_options(&tid, &links).await.unwrap();
        match result {
            ReadWorkOptionsResult::Epic { epic: e, children } => {
                assert_eq!(e.id, epic.id);
                assert_eq!(children.len(), 2);
            }
            other => panic!("expected Epic, got {other:?}"),
        }
    }
}
