//! WorkItemService — orchestration over the WorkItem store.
//!
//! Encapsulates the create/update/reorder/move use-cases that the
//! original `work-item-api.ts` exposed. The store itself is a thin
//! row-CRUD layer; everything that requires composing reads and writes
//! (e.g. computing the next sort_index, transitioning status with the
//! associated timestamp side-effects, moving an item between thread
//! and backlog) lives here.
//!
//! The service does not emit events itself — the Tauri command layer
//! does, after a successful service call. That keeps `oxplow-app`
//! independent of the tauri-specta layering and lets the MCP surface
//! reuse the same service without paying for renderer notifications.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_db::{EffortFileChange, SqliteWorkItemEffortStore, WorkItemEffortStore};
use oxplow_db::SqliteWorkItemStore;
use oxplow_domain::stores::{WorkItemLinkStore, WorkItemStore};
use oxplow_domain::{
    DomainError, ThreadId, Timestamp, WorkItem, WorkItemActorKind, WorkItemAuthor, WorkItemId,
    WorkItemKind, WorkItemLinkType, WorkItemPriority, WorkItemStatus,
};

#[derive(Debug, Error)]
pub enum WorkItemServiceError {
    #[error("work item not found: {0}")]
    NotFound(WorkItemId),
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

async fn item_is_blocked(
    id: &WorkItemId,
    link_store: &dyn WorkItemLinkStore,
    by_id: &std::collections::HashMap<WorkItemId, WorkItem>,
) -> Result<bool, DomainError> {
    let incoming = link_store.list_incoming(id).await?;
    for link in incoming {
        if !matches!(link.link_type, WorkItemLinkType::Blocks) {
            continue;
        }
        if let Some(blocker) = by_id.get(&link.from_item_id) {
            if !matches!(
                blocker.status,
                WorkItemStatus::Done | WorkItemStatus::Canceled | WorkItemStatus::Archived
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Discriminated result for `read_work_options`. The shape mirrors
/// main's TS contract so the agent-side skill text stays accurate
/// without a translation layer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ReadWorkOptionsResult {
    Empty,
    Epic {
        epic: WorkItem,
        children: Vec<WorkItem>,
    },
    Standalone {
        items: Vec<WorkItem>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CreateWorkItemInput {
    pub kind: Option<WorkItemKind>,
    pub title: String,
    pub description: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub parent_id: Option<WorkItemId>,
    pub status: Option<WorkItemStatus>,
    pub priority: Option<WorkItemPriority>,
    pub category: Option<String>,
    pub tags: Option<String>,
    pub author: Option<WorkItemAuthor>,
}

/// Partial-patch for `update_work_item`. Each `Option` follows
/// "missing -> keep, present -> replace" semantics. `category` and
/// `tags` use a wrapping `Option<Option<…>>`-via-helper pattern to
/// distinguish "keep" from "clear"; in this struct, `null` clears and
/// missing keeps.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct UpdateWorkItemChanges {
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance_criteria: Option<Option<String>>,
    pub parent_id: Option<Option<WorkItemId>>,
    pub status: Option<WorkItemStatus>,
    pub priority: Option<WorkItemPriority>,
    pub category: Option<Option<String>>,
    pub tags: Option<Option<String>>,
}

#[derive(Clone)]
pub struct WorkItemService {
    store: Arc<SqliteWorkItemStore>,
}

impl WorkItemService {
    pub fn new(store: Arc<SqliteWorkItemStore>) -> Self {
        Self { store }
    }

    /// Create a work item attached to `thread` (or to the backlog if
    /// `thread` is `None`). Allocates a fresh id and sort_index.
    pub async fn create(
        &self,
        thread: Option<ThreadId>,
        input: CreateWorkItemInput,
    ) -> Result<WorkItem, WorkItemServiceError> {
        let next_sort = self.next_sort_index(thread.as_ref()).await?;
        let now = Timestamp::now();
        let item = WorkItem {
            id: WorkItemId::new(),
            thread_id: thread,
            parent_id: input.parent_id,
            kind: input.kind.unwrap_or(WorkItemKind::Task),
            title: input.title,
            description: input.description.unwrap_or_default(),
            acceptance_criteria: input.acceptance_criteria,
            status: input.status.unwrap_or(WorkItemStatus::Ready),
            priority: input.priority.unwrap_or(WorkItemPriority::Medium),
            sort_index: next_sort,
            created_by: WorkItemActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: input.author.or(Some(WorkItemAuthor::User)),
            category: input.category,
            tags: input.tags,
        };
        self.store.upsert(&item).await?;
        Ok(item)
    }

    /// Apply a partial-patch to an existing work item. Returns the
    /// post-patch row.
    pub async fn update(
        &self,
        id: &WorkItemId,
        changes: UpdateWorkItemChanges,
    ) -> Result<WorkItem, WorkItemServiceError> {
        let mut item = self.load(id).await?;
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
            if matches!(s, WorkItemStatus::Done) && item.status != WorkItemStatus::Done {
                item.completed_at = Some(Timestamp::now());
            } else if matches!(item.status, WorkItemStatus::Done)
                && !matches!(s, WorkItemStatus::Done)
            {
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
        self.store.upsert(&item).await?;
        Ok(item)
    }

    /// Rewrite sort_index across the items in `thread` (or backlog if
    /// `thread` is None) according to the supplied order. Items not
    /// included keep their existing sort_index.
    pub async fn reorder(
        &self,
        thread: Option<&ThreadId>,
        order: &[WorkItemId],
    ) -> Result<(), WorkItemServiceError> {
        let now = Timestamp::now();
        for (idx, id) in order.iter().enumerate() {
            let mut item = self.load(id).await?;
            // Only reorder items in the right scope.
            if item.thread_id.as_ref() != thread {
                continue;
            }
            item.sort_index = idx as i64;
            item.updated_at = now;
            self.store.upsert(&item).await?;
        }
        Ok(())
    }

    /// Move a work item to a different thread (or to the backlog with
    /// `dest = None`). Reallocates sort_index at the destination tail.
    pub async fn move_to(
        &self,
        id: &WorkItemId,
        dest: Option<ThreadId>,
    ) -> Result<WorkItem, WorkItemServiceError> {
        let mut item = self.load(id).await?;
        let next_sort = self.next_sort_index(dest.as_ref()).await?;
        item.thread_id = dest;
        item.sort_index = next_sort;
        item.updated_at = Timestamp::now();
        self.store.upsert(&item).await?;
        Ok(item)
    }

    pub async fn list_for_thread(
        &self,
        thread: &ThreadId,
    ) -> Result<Vec<WorkItem>, WorkItemServiceError> {
        Ok(self.store.list_for_thread(thread).await?)
    }

    /// Open + record + close an effort for `item` against `thread`,
    /// attributing every path in `touched_files` as Updated. The
    /// `summary` is stored on the effort row for the Local History
    /// panel. Idempotent only at the effort-row level: each call
    /// creates a new effort row, even for the same item — that's the
    /// shape main expects (one effort per close + per redo).
    pub async fn record_effort(
        &self,
        effort_store: &SqliteWorkItemEffortStore,
        item: &WorkItemId,
        thread: &ThreadId,
        touched_files: &[String],
        summary: Option<String>,
    ) -> Result<(), WorkItemServiceError> {
        let effort = effort_store.start(item, thread, None).await?;
        for path in touched_files {
            if path.is_empty() {
                continue;
            }
            effort_store
                .record_file(&effort.id, path, EffortFileChange::Updated)
                .await?;
        }
        effort_store.finish(&effort.id, None, summary).await?;
        Ok(())
    }

    pub async fn list_backlog(&self) -> Result<Vec<WorkItem>, WorkItemServiceError> {
        Ok(self.store.list_backlog().await?)
    }

    /// Return the next dispatch unit for the orchestrator. Mirrors
    /// `readWorkOptions` from `src/persistence/work-item-store.ts`:
    ///
    /// 1. Filter to `ready`, sort by `sort_index` ascending.
    /// 2. Honor `blocks` links — an item is hidden while any blocker
    ///    isn't yet `done`/`canceled`/`archived`.
    /// 3. If the head is an epic, recursively gather its `ready`
    ///    descendants (also blocks-aware).
    /// 4. Otherwise return all ready non-epic items so the caller can
    ///    pick one.
    pub async fn read_work_options(
        &self,
        thread: &ThreadId,
        link_store: &dyn WorkItemLinkStore,
    ) -> Result<ReadWorkOptionsResult, WorkItemServiceError> {
        let all = self.store.list_for_thread(thread).await?;
        let by_id: std::collections::HashMap<WorkItemId, WorkItem> =
            all.iter().map(|i| (i.id.clone(), i.clone())).collect();

        let mut ready: Vec<WorkItem> = all
            .iter()
            .filter(|i| i.status == WorkItemStatus::Ready)
            .cloned()
            .collect();
        ready.sort_by_key(|i| (i.sort_index, i.created_at.clone()));

        let mut unblocked_ready: Vec<WorkItem> = Vec::new();
        for item in &ready {
            if !item_is_blocked(&item.id, link_store, &by_id).await? {
                unblocked_ready.push(item.clone());
            }
        }

        let Some(head) = unblocked_ready.first().cloned() else {
            return Ok(ReadWorkOptionsResult::Empty);
        };

        if head.kind == WorkItemKind::Epic {
            let mut children: Vec<WorkItem> = Vec::new();
            let mut frontier = vec![head.id.clone()];
            while let Some(parent_id) = frontier.pop() {
                for it in &all {
                    if it.parent_id.as_ref() == Some(&parent_id) {
                        if it.status == WorkItemStatus::Ready
                            && !item_is_blocked(&it.id, link_store, &by_id).await?
                        {
                            children.push(it.clone());
                        }
                        frontier.push(it.id.clone());
                    }
                }
            }
            children.sort_by_key(|i| (i.sort_index, i.created_at.clone()));
            return Ok(ReadWorkOptionsResult::Epic {
                epic: head,
                children,
            });
        }

        let standalone: Vec<WorkItem> = unblocked_ready
            .into_iter()
            .filter(|i| i.kind != WorkItemKind::Epic)
            .collect();
        Ok(ReadWorkOptionsResult::Standalone { items: standalone })
    }

    pub async fn soft_delete(&self, id: &WorkItemId) -> Result<(), WorkItemServiceError> {
        self.store.soft_delete(id).await?;
        Ok(())
    }

    async fn load(&self, id: &WorkItemId) -> Result<WorkItem, WorkItemServiceError> {
        self.store
            .get(id)
            .await?
            .ok_or_else(|| WorkItemServiceError::NotFound(id.clone()))
    }

    async fn next_sort_index(
        &self,
        thread: Option<&ThreadId>,
    ) -> Result<i64, WorkItemServiceError> {
        let items = match thread {
            Some(t) => self.store.list_for_thread(t).await?,
            None => self.store.list_backlog().await?,
        };
        Ok(items.iter().map(|i| i.sort_index).max().unwrap_or(-1) + 1)
    }
}

/// The bucketed view the Backlog page renders.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BacklogState {
    pub items: Vec<WorkItem>,
    pub waiting: Vec<WorkItem>,
    pub in_progress: Vec<WorkItem>,
    pub done: Vec<WorkItem>,
}

impl BacklogState {
    pub fn from_rows(rows: Vec<WorkItem>) -> Self {
        let mut items = Vec::new();
        let mut waiting = Vec::new();
        let mut in_progress = Vec::new();
        let mut done = Vec::new();
        for r in rows {
            match r.status {
                WorkItemStatus::InProgress => in_progress.push(r),
                WorkItemStatus::Done | WorkItemStatus::Canceled | WorkItemStatus::Archived => {
                    done.push(r)
                }
                WorkItemStatus::Blocked => waiting.push(r),
                WorkItemStatus::Ready => items.push(r),
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

    async fn fixture() -> (WorkItemService, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let store = Arc::new(SqliteWorkItemStore::new(db));
        let s = Stream {
            id: StreamId::from("s-1"),
            kind: StreamKind::Primary,
            title: "p".into(),
            summary: String::new(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
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
        (WorkItemService::new(store), t.id)
    }

    #[tokio::test]
    async fn create_assigns_increasing_sort_index() {
        let (svc, tid) = fixture().await;
        let a = svc
            .create(
                Some(tid.clone()),
                CreateWorkItemInput {
                    title: "a".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let b = svc
            .create(
                Some(tid.clone()),
                CreateWorkItemInput {
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
                CreateWorkItemInput {
                    title: "before".into(),
                    description: Some("desc".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = svc
            .update(
                &it.id,
                UpdateWorkItemChanges {
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
                CreateWorkItemInput {
                    title: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(it.completed_at.is_none());
        let done = svc
            .update(
                &it.id,
                UpdateWorkItemChanges {
                    status: Some(WorkItemStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(done.completed_at.is_some());
        let reopened = svc
            .update(
                &done.id,
                UpdateWorkItemChanges {
                    status: Some(WorkItemStatus::InProgress),
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
                CreateWorkItemInput {
                    title: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let moved = svc.move_to(&it.id, None).await.unwrap();
        assert!(moved.thread_id.is_none());
        let bl = svc.list_backlog().await.unwrap();
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].id, it.id);
    }

    #[tokio::test]
    async fn reorder_rewrites_indices() {
        let (svc, tid) = fixture().await;
        let a = svc
            .create(Some(tid.clone()), CreateWorkItemInput { title: "a".into(), ..Default::default() })
            .await
            .unwrap();
        let b = svc
            .create(Some(tid.clone()), CreateWorkItemInput { title: "b".into(), ..Default::default() })
            .await
            .unwrap();
        let c = svc
            .create(Some(tid.clone()), CreateWorkItemInput { title: "c".into(), ..Default::default() })
            .await
            .unwrap();
        // c, a, b
        svc.reorder(Some(&tid), &[c.id.clone(), a.id.clone(), b.id.clone()])
            .await
            .unwrap();
        let list = svc.list_for_thread(&tid).await.unwrap();
        let order: Vec<_> = list.iter().map(|i| i.id.clone()).collect();
        assert_eq!(order, vec![c.id, a.id, b.id]);
    }

    #[test]
    fn backlog_state_buckets_by_status() {
        let now = Timestamp::from_unix_ms(1);
        let mk = |id: &str, status| WorkItem {
            id: WorkItemId::from(id),
            thread_id: None,
            parent_id: None,
            kind: WorkItemKind::Task,
            title: id.into(),
            description: String::new(),
            acceptance_criteria: None,
            status,
            priority: WorkItemPriority::Medium,
            sort_index: 0,
            created_by: WorkItemActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(WorkItemAuthor::User),
            category: None,
            tags: None,
        };
        let rows = vec![
            mk("a", WorkItemStatus::Ready),
            mk("b", WorkItemStatus::InProgress),
            mk("c", WorkItemStatus::Done),
            mk("d", WorkItemStatus::Blocked),
        ];
        let st = BacklogState::from_rows(rows);
        assert_eq!(st.items.len(), 1);
        assert_eq!(st.in_progress.len(), 1);
        assert_eq!(st.done.len(), 1);
        assert_eq!(st.waiting.len(), 1);
    }
}
