//! Stores for the satellites of `work_items`: notes, links, events.
//!
//! Each is small enough to share this module rather than warranting
//! its own file. They share the timestamp + helper plumbing the
//! main work_item_store already establishes.

use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::{WorkItemEventStore, WorkItemLinkStore, WorkNoteStore};
use oxplow_domain::{
    DomainError, NoteId, ThreadId, Timestamp, WorkItemActorKind, WorkItemEvent, WorkItemId,
    WorkItemLink, WorkItemLinkType, WorkNote,
};

use crate::database::Database;

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts).unwrap().trim_matches('"').to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

fn link_type_to_str(t: WorkItemLinkType) -> &'static str {
    match t {
        WorkItemLinkType::Blocks => "blocks",
        WorkItemLinkType::RelatesTo => "relates_to",
        WorkItemLinkType::DiscoveredFrom => "discovered_from",
        WorkItemLinkType::Duplicates => "duplicates",
        WorkItemLinkType::Supersedes => "supersedes",
        WorkItemLinkType::RepliesTo => "replies_to",
    }
}

fn str_to_link_type(s: &str) -> Result<WorkItemLinkType, DomainError> {
    match s {
        "blocks" => Ok(WorkItemLinkType::Blocks),
        "relates_to" => Ok(WorkItemLinkType::RelatesTo),
        "discovered_from" => Ok(WorkItemLinkType::DiscoveredFrom),
        "duplicates" => Ok(WorkItemLinkType::Duplicates),
        "supersedes" => Ok(WorkItemLinkType::Supersedes),
        "replies_to" => Ok(WorkItemLinkType::RepliesTo),
        other => Err(DomainError::Invalid(format!("unknown link type: {other}"))),
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

// ---------------- Work notes ----------------

#[derive(Clone)]
pub struct SqliteWorkNoteStore {
    db: Database,
}

impl SqliteWorkNoteStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkNote> {
    let id: String = row.get("id")?;
    let work_item_id: Option<String> = row.get("work_item_id")?;
    let thread_id: Option<String> = row.get("thread_id")?;
    let body: String = row.get("body")?;
    let author: String = row.get("author")?;
    let created_at: String = row.get("created_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(WorkNote {
        id: NoteId::from(id),
        work_item_id: work_item_id.map(WorkItemId::from),
        thread_id: thread_id.map(ThreadId::from),
        body,
        author,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

#[async_trait]
impl WorkNoteStore for SqliteWorkNoteStore {
    async fn add_for_item(
        &self,
        item: &WorkItemId,
        body: &str,
        author: &str,
    ) -> Result<WorkNote, DomainError> {
        let db = self.db.clone();
        let item = item.clone();
        let body = body.to_string();
        let author = author.to_string();
        tokio::task::spawn_blocking(move || {
            let id = NoteId::new();
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO work_notes (id, work_item_id, body, author, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id.as_str(), item.as_str(), body, author, ts_to_string(now)],
                )?;
                Ok(())
            })?;
            Ok(WorkNote {
                id,
                work_item_id: Some(item),
                thread_id: None,
                body,
                author,
                created_at: now,
            })
        })
        .await
        .unwrap()
    }

    async fn add_for_thread(
        &self,
        thread: &ThreadId,
        body: &str,
        author: &str,
    ) -> Result<WorkNote, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        let body = body.to_string();
        let author = author.to_string();
        tokio::task::spawn_blocking(move || {
            let id = NoteId::new();
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO work_notes (id, thread_id, body, author, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        id.as_str(),
                        thread.as_str(),
                        body,
                        author,
                        ts_to_string(now)
                    ],
                )?;
                Ok(())
            })?;
            Ok(WorkNote {
                id,
                work_item_id: None,
                thread_id: Some(thread),
                body,
                author,
                created_at: now,
            })
        })
        .await
        .unwrap()
    }

    async fn list_for_item(&self, item: &WorkItemId) -> Result<Vec<WorkNote>, DomainError> {
        let db = self.db.clone();
        let item = item.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM work_notes WHERE work_item_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![item.as_str()], row_to_note)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<WorkNote>, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM work_notes WHERE thread_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![thread.as_str()], row_to_note)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn update_body(&self, id: &NoteId, body: &str) -> Result<(), DomainError> {
        let db = self.db.clone();
        let id = id.clone();
        let body = body.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE work_notes SET body = ?2 WHERE id = ?1",
                    params![id.as_str(), body],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn delete(&self, id: &NoteId) -> Result<(), DomainError> {
        let db = self.db.clone();
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute("DELETE FROM work_notes WHERE id = ?1", params![id.as_str()])?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }
}

// ---------------- Work item links ----------------

#[derive(Clone)]
pub struct SqliteWorkItemLinkStore {
    db: Database,
}

impl SqliteWorkItemLinkStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn row_to_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItemLink> {
    let id: String = row.get("id")?;
    let thread_id: String = row.get("thread_id")?;
    let from_item_id: String = row.get("from_item_id")?;
    let to_item_id: String = row.get("to_item_id")?;
    let link_type: String = row.get("link_type")?;
    let created_at: String = row.get("created_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(WorkItemLink {
        id,
        thread_id: ThreadId::from(thread_id),
        from_item_id: WorkItemId::from(from_item_id),
        to_item_id: WorkItemId::from(to_item_id),
        link_type: str_to_link_type(&link_type).map_err(map_err)?,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

#[async_trait]
impl WorkItemLinkStore for SqliteWorkItemLinkStore {
    async fn create(
        &self,
        thread: &ThreadId,
        from: &WorkItemId,
        to: &WorkItemId,
        link_type: WorkItemLinkType,
    ) -> Result<WorkItemLink, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        let from = from.clone();
        let to = to.clone();
        tokio::task::spawn_blocking(move || {
            let id = format!("wil-{}", uuid::Uuid::new_v4().simple());
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO work_item_links (id, thread_id, from_item_id, to_item_id, link_type, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        id,
                        thread.as_str(),
                        from.as_str(),
                        to.as_str(),
                        link_type_to_str(link_type),
                        ts_to_string(now),
                    ],
                )?;
                Ok(())
            })?;
            Ok(WorkItemLink {
                id,
                thread_id: thread,
                from_item_id: from,
                to_item_id: to,
                link_type,
                created_at: now,
            })
        })
        .await
        .unwrap()
    }

    async fn list_outgoing(&self, item: &WorkItemId) -> Result<Vec<WorkItemLink>, DomainError> {
        let db = self.db.clone();
        let item = item.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM work_item_links WHERE from_item_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![item.as_str()], row_to_link)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn list_incoming(&self, item: &WorkItemId) -> Result<Vec<WorkItemLink>, DomainError> {
        let db = self.db.clone();
        let item = item.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM work_item_links WHERE to_item_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![item.as_str()], row_to_link)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn delete(&self, id: &str) -> Result<(), DomainError> {
        let db = self.db.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute("DELETE FROM work_item_links WHERE id = ?1", params![id])?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }
}

// ---------------- Work item events ----------------

#[derive(Clone)]
pub struct SqliteWorkItemEventStore {
    db: Database,
}

impl SqliteWorkItemEventStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItemEvent> {
    let id: String = row.get("id")?;
    let thread_id: String = row.get("thread_id")?;
    let item_id: Option<String> = row.get("item_id")?;
    let event_type: String = row.get("event_type")?;
    let actor_kind: String = row.get("actor_kind")?;
    let actor_id: String = row.get("actor_id")?;
    let payload_json: String = row.get("payload_json")?;
    let created_at: String = row.get("created_at")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(WorkItemEvent {
        id,
        thread_id: ThreadId::from(thread_id),
        item_id: item_id.map(WorkItemId::from),
        event_type,
        actor_kind: str_to_actor(&actor_kind).map_err(map_err)?,
        actor_id,
        payload_json,
        created_at: string_to_ts(&created_at).map_err(map_err)?,
    })
}

#[async_trait]
impl WorkItemEventStore for SqliteWorkItemEventStore {
    async fn append(&self, event: &WorkItemEvent) -> Result<(), DomainError> {
        let db = self.db.clone();
        let event = event.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO work_item_events
                       (id, thread_id, item_id, event_type, actor_kind, actor_id, payload_json, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        event.id,
                        event.thread_id.as_str(),
                        event.item_id.as_ref().map(|i| i.as_str()),
                        event.event_type,
                        actor_to_str(event.actor_kind),
                        event.actor_id,
                        event.payload_json,
                        ts_to_string(event.created_at),
                    ],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn list_for_item(&self, item: &WorkItemId) -> Result<Vec<WorkItemEvent>, DomainError> {
        let db = self.db.clone();
        let item = item.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM work_item_events WHERE item_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![item.as_str()], row_to_event)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<WorkItemEvent>, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM work_item_events WHERE thread_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![thread.as_str()], row_to_event)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
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
    use crate::work_item_store::SqliteWorkItemStore;
    use oxplow_domain::stores::{StreamStore, ThreadStore, WorkItemStore};
    use oxplow_domain::{
        Stream, StreamId, StreamKind, Thread, ThreadStatus, WorkItem, WorkItemAuthor, WorkItemKind,
        WorkItemPriority, WorkItemStatus,
    };

    fn now() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    async fn fixture() -> (Database, ThreadId, WorkItemId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let items = SqliteWorkItemStore::new(db.clone());

        let s = Stream {
            id: StreamId::from("s-1"),
            kind: StreamKind::Primary,
            title: "p".into(),
            summary: String::new(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/r".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            created_at: now(),
            updated_at: now(),
        };
        streams.upsert(&s).await.unwrap();

        let t = Thread {
            id: ThreadId::from("b-1"),
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
            created_at: now(),
            updated_at: now(),
        };
        threads.upsert(&t).await.unwrap();

        let item_id = WorkItemId::from("wi-1");
        let item = WorkItem {
            id: item_id.clone(),
            thread_id: Some(t.id.clone()),
            parent_id: None,
            kind: WorkItemKind::Task,
            title: "x".into(),
            description: String::new(),
            acceptance_criteria: None,
            status: WorkItemStatus::Ready,
            priority: WorkItemPriority::Medium,
            sort_index: 0,
            created_by: WorkItemActorKind::User,
            created_at: now(),
            updated_at: now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(WorkItemAuthor::User),
            category: None,
            tags: None,
        };
        items.upsert(&item).await.unwrap();
        (db, t.id, item_id)
    }

    #[tokio::test]
    async fn note_for_item_round_trips() {
        let (db, _tid, item_id) = fixture().await;
        let store = SqliteWorkNoteStore::new(db);
        let note = store
            .add_for_item(&item_id, "looking good", "user")
            .await
            .unwrap();
        assert_eq!(note.work_item_id.as_ref(), Some(&item_id));
        assert!(note.thread_id.is_none());
        let listed = store.list_for_item(&item_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].body, "looking good");
    }

    #[tokio::test]
    async fn note_for_thread_round_trips() {
        let (db, tid, _item_id) = fixture().await;
        let store = SqliteWorkNoteStore::new(db);
        let note = store
            .add_for_thread(&tid, "thread-level finding", "agent")
            .await
            .unwrap();
        assert!(note.work_item_id.is_none());
        assert_eq!(note.thread_id.as_ref(), Some(&tid));
        let listed = store.list_for_thread(&tid).await.unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn note_delete_removes() {
        let (db, _tid, item_id) = fixture().await;
        let store = SqliteWorkNoteStore::new(db);
        let note = store.add_for_item(&item_id, "x", "u").await.unwrap();
        store.delete(&note.id).await.unwrap();
        assert!(store.list_for_item(&item_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn link_round_trip_and_directionality() {
        let (db, tid, from_id) = fixture().await;
        // Create a second item to link to.
        let items = SqliteWorkItemStore::new(db.clone());
        let to = WorkItem {
            id: WorkItemId::from("wi-2"),
            thread_id: Some(tid.clone()),
            parent_id: None,
            kind: WorkItemKind::Task,
            title: "y".into(),
            description: String::new(),
            acceptance_criteria: None,
            status: WorkItemStatus::Ready,
            priority: WorkItemPriority::Medium,
            sort_index: 1,
            created_by: WorkItemActorKind::User,
            created_at: now(),
            updated_at: now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(WorkItemAuthor::User),
            category: None,
            tags: None,
        };
        items.upsert(&to).await.unwrap();
        let store = SqliteWorkItemLinkStore::new(db);
        store
            .create(&tid, &from_id, &to.id, WorkItemLinkType::Blocks)
            .await
            .unwrap();
        let outgoing = store.list_outgoing(&from_id).await.unwrap();
        let incoming = store.list_incoming(&to.id).await.unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(incoming.len(), 1);
        assert_eq!(outgoing[0].link_type, WorkItemLinkType::Blocks);
    }

    #[tokio::test]
    async fn event_append_and_list() {
        let (db, tid, item_id) = fixture().await;
        let store = SqliteWorkItemEventStore::new(db);
        let evt = WorkItemEvent {
            id: "evt-1".into(),
            thread_id: tid.clone(),
            item_id: Some(item_id.clone()),
            event_type: "transition".into(),
            actor_kind: WorkItemActorKind::Agent,
            actor_id: "claude".into(),
            payload_json: "{\"to\":\"in_progress\"}".into(),
            created_at: now(),
        };
        store.append(&evt).await.unwrap();
        let item_events = store.list_for_item(&item_id).await.unwrap();
        let thread_events = store.list_for_thread(&tid).await.unwrap();
        assert_eq!(item_events.len(), 1);
        assert_eq!(thread_events.len(), 1);
        assert_eq!(thread_events[0].event_type, "transition");
    }
}
