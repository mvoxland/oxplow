use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::WorkItemStore;
use oxplow_domain::{
    DomainError, ThreadId, Timestamp, WorkItem, WorkItemActorKind, WorkItemAuthor, WorkItemId,
    WorkItemKind, WorkItemPriority, WorkItemStatus,
};

use crate::database::Database;

#[derive(Clone)]
pub struct SqliteWorkItemStore {
    db: Database,
}

impl SqliteWorkItemStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn kind_to_str(k: WorkItemKind) -> &'static str {
    match k {
        WorkItemKind::Epic => "epic",
        WorkItemKind::Task => "task",
        WorkItemKind::Subtask => "subtask",
        WorkItemKind::Bug => "bug",
        WorkItemKind::Note => "note",
    }
}

fn str_to_kind(s: &str) -> Result<WorkItemKind, DomainError> {
    match s {
        "epic" => Ok(WorkItemKind::Epic),
        "task" => Ok(WorkItemKind::Task),
        "subtask" => Ok(WorkItemKind::Subtask),
        "bug" => Ok(WorkItemKind::Bug),
        "note" => Ok(WorkItemKind::Note),
        other => Err(DomainError::Invalid(format!("unknown work item kind: {other}"))),
    }
}

fn status_to_str(s: WorkItemStatus) -> &'static str {
    match s {
        WorkItemStatus::Ready => "ready",
        WorkItemStatus::InProgress => "in_progress",
        WorkItemStatus::Blocked => "blocked",
        WorkItemStatus::Done => "done",
        WorkItemStatus::Canceled => "canceled",
        WorkItemStatus::Archived => "archived",
    }
}

fn str_to_status(s: &str) -> Result<WorkItemStatus, DomainError> {
    match s {
        "ready" => Ok(WorkItemStatus::Ready),
        "in_progress" => Ok(WorkItemStatus::InProgress),
        "blocked" => Ok(WorkItemStatus::Blocked),
        "done" => Ok(WorkItemStatus::Done),
        "canceled" => Ok(WorkItemStatus::Canceled),
        "archived" => Ok(WorkItemStatus::Archived),
        other => Err(DomainError::Invalid(format!(
            "unknown work item status: {other}"
        ))),
    }
}

fn priority_to_str(p: WorkItemPriority) -> &'static str {
    match p {
        WorkItemPriority::Low => "low",
        WorkItemPriority::Medium => "medium",
        WorkItemPriority::High => "high",
        WorkItemPriority::Urgent => "urgent",
    }
}

fn str_to_priority(s: &str) -> Result<WorkItemPriority, DomainError> {
    match s {
        "low" => Ok(WorkItemPriority::Low),
        "medium" => Ok(WorkItemPriority::Medium),
        "high" => Ok(WorkItemPriority::High),
        "urgent" => Ok(WorkItemPriority::Urgent),
        other => Err(DomainError::Invalid(format!(
            "unknown work item priority: {other}"
        ))),
    }
}

fn actor_to_str(a: WorkItemActorKind) -> &'static str {
    match a {
        WorkItemActorKind::User => "user",
        WorkItemActorKind::Agent => "agent",
        WorkItemActorKind::System => "system",
    }
}

fn str_to_actor(s: &str) -> Result<WorkItemActorKind, DomainError> {
    match s {
        "user" => Ok(WorkItemActorKind::User),
        "agent" => Ok(WorkItemActorKind::Agent),
        "system" => Ok(WorkItemActorKind::System),
        other => Err(DomainError::Invalid(format!("unknown actor kind: {other}"))),
    }
}

fn author_to_str(a: WorkItemAuthor) -> &'static str {
    match a {
        WorkItemAuthor::User => "user",
        WorkItemAuthor::Agent => "agent",
    }
}

fn str_to_author(s: &str) -> Result<WorkItemAuthor, DomainError> {
    match s {
        "user" => Ok(WorkItemAuthor::User),
        "agent" => Ok(WorkItemAuthor::Agent),
        // Pre-v29 legacy values map to None at the row level — they
        // shouldn't reach this function, but protect anyway.
        other => Err(DomainError::Invalid(format!(
            "unknown work item author: {other}"
        ))),
    }
}

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts).unwrap().trim_matches('"').to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

fn row_to_work_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
    let id: String = row.get("id")?;
    let thread_id: Option<String> = row.get("thread_id")?;
    let parent_id: Option<String> = row.get("parent_id")?;
    let kind: String = row.get("kind")?;
    let title: String = row.get("title")?;
    let description: String = row.get("description")?;
    let acceptance_criteria: Option<String> = row.get("acceptance_criteria")?;
    let status: String = row.get("status")?;
    let priority: String = row.get("priority")?;
    let sort_index: i64 = row.get("sort_index")?;
    let created_by: String = row.get("created_by")?;
    let created_at: String = row.get("created_at")?;
    let updated_at: String = row.get("updated_at")?;
    let completed_at: Option<String> = row.get("completed_at")?;
    let deleted_at: Option<String> = row.get("deleted_at")?;
    let author: Option<String> = row.get("author")?;
    let category: Option<String> = row.get("category")?;
    let tags: Option<String> = row.get("tags")?;

    // Note count comes from a JOIN'd subquery; if absent, fall back to 0.
    let note_count: i64 = row
        .get::<_, Option<i64>>("note_count")
        .ok()
        .flatten()
        .unwrap_or(0);

    let map_err =
        |e: DomainError| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e));

    Ok(WorkItem {
        id: WorkItemId::from(id),
        thread_id: thread_id.map(ThreadId::from),
        parent_id: parent_id.map(WorkItemId::from),
        kind: str_to_kind(&kind).map_err(map_err)?,
        title,
        description,
        acceptance_criteria,
        status: str_to_status(&status).map_err(map_err)?,
        priority: str_to_priority(&priority).map_err(map_err)?,
        sort_index,
        created_by: str_to_actor(&created_by).map_err(map_err)?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
        updated_at: string_to_ts(&updated_at).map_err(map_err)?,
        completed_at: completed_at.map(|s| string_to_ts(&s)).transpose().map_err(map_err)?,
        deleted_at: deleted_at.map(|s| string_to_ts(&s)).transpose().map_err(map_err)?,
        note_count,
        author: author.and_then(|a| str_to_author(&a).ok()),
        category,
        tags,
    })
}

const SELECT_BASE: &str =
    "SELECT wi.*, COALESCE((SELECT COUNT(*) FROM work_notes wn WHERE wn.work_item_id = wi.id), 0) AS note_count
     FROM work_items wi";

#[async_trait]
impl WorkItemStore for SqliteWorkItemStore {
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<WorkItem>, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let sql = format!(
                    "{} WHERE wi.thread_id = ?1 AND wi.deleted_at IS NULL \
                     ORDER BY wi.sort_index ASC, wi.created_at ASC",
                    SELECT_BASE
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![thread.as_str()], row_to_work_item)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn list_backlog(&self) -> Result<Vec<WorkItem>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let sql = format!(
                    "{} WHERE wi.thread_id IS NULL AND wi.deleted_at IS NULL \
                     ORDER BY wi.sort_index ASC, wi.created_at ASC",
                    SELECT_BASE
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], row_to_work_item)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn get(&self, id: &WorkItemId) -> Result<Option<WorkItem>, DomainError> {
        let db = self.db.clone();
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let sql = format!("{} WHERE wi.id = ?1", SELECT_BASE);
                let mut stmt = conn.prepare(&sql)?;
                let mut rows = stmt.query_map(params![id.as_str()], row_to_work_item)?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            })
        })
        .await
        .unwrap()
    }

    async fn upsert(&self, item: &WorkItem) -> Result<(), DomainError> {
        let db = self.db.clone();
        let item = item.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO work_items (
                        id, thread_id, parent_id, kind, title, description, acceptance_criteria,
                        status, priority, sort_index, created_by, created_at, updated_at,
                        completed_at, deleted_at, author, category, tags
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                     ON CONFLICT(id) DO UPDATE SET
                        thread_id = excluded.thread_id,
                        parent_id = excluded.parent_id,
                        kind = excluded.kind,
                        title = excluded.title,
                        description = excluded.description,
                        acceptance_criteria = excluded.acceptance_criteria,
                        status = excluded.status,
                        priority = excluded.priority,
                        sort_index = excluded.sort_index,
                        updated_at = excluded.updated_at,
                        completed_at = excluded.completed_at,
                        deleted_at = excluded.deleted_at,
                        author = excluded.author,
                        category = excluded.category,
                        tags = excluded.tags",
                    params![
                        item.id.as_str(),
                        item.thread_id.as_ref().map(|t| t.as_str()),
                        item.parent_id.as_ref().map(|p| p.as_str()),
                        kind_to_str(item.kind),
                        item.title,
                        item.description,
                        item.acceptance_criteria,
                        status_to_str(item.status),
                        priority_to_str(item.priority),
                        item.sort_index,
                        actor_to_str(item.created_by),
                        ts_to_string(item.created_at),
                        ts_to_string(item.updated_at),
                        item.completed_at.map(ts_to_string),
                        item.deleted_at.map(ts_to_string),
                        item.author.map(author_to_str),
                        item.category,
                        item.tags,
                    ],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn soft_delete(&self, id: &WorkItemId) -> Result<(), DomainError> {
        let db = self.db.clone();
        let id = id.clone();
        let now = ts_to_string(Timestamp::now());
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE work_items SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1",
                    params![id.as_str(), now],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_store::SqliteStreamStore;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadStatus};

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    async fn fixture() -> (SqliteWorkItemStore, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let work = SqliteWorkItemStore::new(db);
        let s = Stream {
            id: StreamId::from("s-1"),
            kind: StreamKind::Primary,
            title: "oxplow".into(),
            summary: String::new(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/repo".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            created_at: ts(),
            updated_at: ts(),
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::from("b-1"),
            stream_id: s.id.clone(),
            title: "explore".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: ts(),
            updated_at: ts(),
        };
        threads.upsert(&t).await.unwrap();
        (work, t.id)
    }

    fn item(id: &str, thread: Option<ThreadId>) -> WorkItem {
        WorkItem {
            id: WorkItemId::from(id),
            thread_id: thread,
            parent_id: None,
            kind: WorkItemKind::Task,
            title: "ship it".into(),
            description: String::new(),
            acceptance_criteria: None,
            status: WorkItemStatus::Ready,
            priority: WorkItemPriority::Medium,
            sort_index: 0,
            created_by: WorkItemActorKind::User,
            created_at: ts(),
            updated_at: ts(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(WorkItemAuthor::User),
            category: None,
            tags: None,
        }
    }

    #[tokio::test]
    async fn upsert_then_get() {
        let (store, tid) = fixture().await;
        let it = item("wi-1", Some(tid));
        store.upsert(&it).await.unwrap();
        let got = store.get(&it.id).await.unwrap().unwrap();
        assert_eq!(got, it);
    }

    #[tokio::test]
    async fn list_for_thread_excludes_deleted() {
        let (store, tid) = fixture().await;
        let alive = item("wi-alive", Some(tid.clone()));
        let dead = item("wi-dead", Some(tid.clone()));
        store.upsert(&alive).await.unwrap();
        store.upsert(&dead).await.unwrap();
        store.soft_delete(&dead.id).await.unwrap();
        let list = store.list_for_thread(&tid).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, alive.id);
    }

    #[tokio::test]
    async fn backlog_items_have_no_thread() {
        let (store, tid) = fixture().await;
        let in_thread = item("wi-thread", Some(tid));
        let on_backlog = item("wi-backlog", None);
        store.upsert(&in_thread).await.unwrap();
        store.upsert(&on_backlog).await.unwrap();

        let bl = store.list_backlog().await.unwrap();
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].id, on_backlog.id);
    }

    #[tokio::test]
    async fn list_orders_by_sort_index() {
        let (store, tid) = fixture().await;
        let mut a = item("wi-a", Some(tid.clone()));
        a.sort_index = 5;
        let mut b = item("wi-b", Some(tid.clone()));
        b.sort_index = 1;
        store.upsert(&a).await.unwrap();
        store.upsert(&b).await.unwrap();
        let list = store.list_for_thread(&tid).await.unwrap();
        assert_eq!(list[0].id, b.id);
        assert_eq!(list[1].id, a.id);
    }

    #[tokio::test]
    async fn upsert_overwrites_existing() {
        let (store, tid) = fixture().await;
        let mut it = item("wi-x", Some(tid));
        store.upsert(&it).await.unwrap();
        it.title = "renamed".into();
        it.status = WorkItemStatus::InProgress;
        store.upsert(&it).await.unwrap();
        let got = store.get(&it.id).await.unwrap().unwrap();
        assert_eq!(got.title, "renamed");
        assert_eq!(got.status, WorkItemStatus::InProgress);
    }
}
