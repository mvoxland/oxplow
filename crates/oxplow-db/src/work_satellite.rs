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
use crate::page_ref_projections::{
    link_edge, note_edges, work_item_link_ref_types, KIND_WORK_ITEM, KIND_WORK_NOTE,
};
use crate::page_ref_store::SqlitePageRefStore;

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts)
        .unwrap()
        .trim_matches('"')
        .to_string()
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
    page_refs: Option<SqlitePageRefStore>,
}

impl SqliteWorkNoteStore {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            page_refs: None,
        }
    }

    /// When set, every add / update_body / delete also re-projects
    /// the note's body into `page_ref` under
    /// `(work-note, <note id>)` (single-owner full replace).
    pub fn with_page_refs(mut self, store: SqlitePageRefStore) -> Self {
        self.page_refs = Some(store);
        self
    }

    /// Iterate every note id + body for the boot-time backfill.
    pub async fn list_all_for_backfill(&self) -> Result<Vec<(String, String)>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT id, body FROM work_notes")?;
                let rows =
                    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn project_note(&self, id: &str, body: &str) -> Result<(), DomainError> {
        let Some(refs) = &self.page_refs else {
            return Ok(());
        };
        let edges = note_edges(id, body);
        refs.replace_source(KIND_WORK_NOTE, id, edges).await
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
        let body_owned = body.to_string();
        let author = author.to_string();
        let note = tokio::task::spawn_blocking(move || -> Result<WorkNote, DomainError> {
            let id = NoteId::new();
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO work_notes (id, work_item_id, body, author, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        id.as_str(),
                        item.as_str(),
                        body_owned,
                        author,
                        ts_to_string(now)
                    ],
                )?;
                Ok(())
            })?;
            Ok(WorkNote {
                id,
                work_item_id: Some(item),
                thread_id: None,
                body: body_owned,
                author,
                created_at: now,
            })
        })
        .await
        .unwrap()?;
        self.project_note(note.id.as_str(), &note.body).await?;
        Ok(note)
    }

    async fn add_for_thread(
        &self,
        thread: &ThreadId,
        body: &str,
        author: &str,
    ) -> Result<WorkNote, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        let body_owned = body.to_string();
        let author = author.to_string();
        let note = tokio::task::spawn_blocking(move || -> Result<WorkNote, DomainError> {
            let id = NoteId::new();
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO work_notes (id, thread_id, body, author, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        id.as_str(),
                        thread.as_str(),
                        body_owned,
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
                body: body_owned,
                author,
                created_at: now,
            })
        })
        .await
        .unwrap()?;
        self.project_note(note.id.as_str(), &note.body).await?;
        Ok(note)
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
        let id_clone = id.clone();
        let body_clone = body.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE work_notes SET body = ?2 WHERE id = ?1",
                    params![id_clone.as_str(), body_clone],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()?;
        self.project_note(id.as_str(), body).await?;
        Ok(())
    }

    async fn delete(&self, id: &NoteId) -> Result<(), DomainError> {
        let db = self.db.clone();
        let id_clone = id.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "DELETE FROM work_notes WHERE id = ?1",
                    params![id_clone.as_str()],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()?;
        if let Some(refs) = &self.page_refs {
            refs.replace_source(KIND_WORK_NOTE, id.as_str(), vec![])
                .await?;
        }
        Ok(())
    }
}

// ---------------- Work item links ----------------

#[derive(Clone)]
pub struct SqliteWorkItemLinkStore {
    db: Database,
    page_refs: Option<SqlitePageRefStore>,
}

impl SqliteWorkItemLinkStore {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            page_refs: None,
        }
    }

    /// Distinct `from_item_id` values across every link row. Used
    /// by the page-ref backfill so we can re-project each owning
    /// item's slice exactly once.
    pub async fn list_distinct_from_items(&self) -> Result<Vec<WorkItemId>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT DISTINCT from_item_id FROM work_item_links")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
                rows.map(|r| r.map(WorkItemId::from))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    /// When set, every link create/delete also re-projects the
    /// affected work-item's outgoing link edges into `page_ref`.
    /// We project the WHOLE outgoing slice for `from_item_id` rather
    /// than just the changed row so removals are clean.
    pub fn with_page_refs(mut self, store: SqlitePageRefStore) -> Self {
        self.page_refs = Some(store);
        self
    }

    /// Re-emit `work_item_link:*` edges for all currently-stored
    /// outgoing links of `from_item`. Called after create/delete
    /// when `page_refs` is attached. Uses the slice variant so
    /// body-mention edges from `work_item_store` survive.
    async fn project_outgoing_links(&self, from_item: &WorkItemId) -> Result<(), DomainError> {
        let Some(refs) = &self.page_refs else {
            return Ok(());
        };
        let db = self.db.clone();
        let from = from_item.clone();
        let links: Vec<WorkItemLink> = tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM work_item_links WHERE from_item_id = ?1 ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![from.as_str()], row_to_link)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()?;
        let edges: Vec<_> = links.iter().map(link_edge).collect();
        refs.replace_source_for_ref_types(
            KIND_WORK_ITEM,
            from_item.as_str(),
            work_item_link_ref_types(),
            edges,
        )
        .await
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
        let thread_clone = thread.clone();
        let from_clone = from.clone();
        let to_clone = to.clone();
        let link = tokio::task::spawn_blocking(move || -> Result<WorkItemLink, DomainError> {
            let id = format!("wil-{}", uuid::Uuid::new_v4().simple());
            let now = Timestamp::now();
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO work_item_links (id, thread_id, from_item_id, to_item_id, link_type, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        id,
                        thread_clone.as_str(),
                        from_clone.as_str(),
                        to_clone.as_str(),
                        link_type_to_str(link_type),
                        ts_to_string(now),
                    ],
                )?;
                Ok(())
            })?;
            Ok(WorkItemLink {
                id,
                thread_id: thread_clone,
                from_item_id: from_clone,
                to_item_id: to_clone,
                link_type,
                created_at: now,
            })
        })
        .await
        .unwrap()?;
        self.project_outgoing_links(from).await?;
        Ok(link)
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
        // Capture the from_item_id BEFORE deletion so we can re-project.
        let db = self.db.clone();
        let id_str = id.to_string();
        let from_item: Option<WorkItemId> = tokio::task::spawn_blocking({
            let db = db.clone();
            let id = id_str.clone();
            move || -> Result<Option<WorkItemId>, DomainError> {
                db.with_conn(|conn| {
                    let mut stmt =
                        conn.prepare("SELECT from_item_id FROM work_item_links WHERE id = ?1")?;
                    let mut rows = stmt.query_map(params![id], |r| r.get::<_, String>(0))?;
                    Ok(rows.next().transpose()?.map(WorkItemId::from))
                })
            }
        })
        .await
        .unwrap()?;
        let id_str2 = id_str.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "DELETE FROM work_item_links WHERE id = ?1",
                    params![id_str2],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()?;
        if let Some(from) = from_item {
            self.project_outgoing_links(&from).await?;
        }
        Ok(())
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
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/r".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: now(),
            updated_at: now(),
            archived_at: None,
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
            archived_at: None,
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

    /// Notes attached to a page_refs store mirror their parsed body
    /// into `page_ref` on add / update / delete.
    #[tokio::test]
    async fn note_with_page_refs_projects_body() {
        use crate::page_ref_store::SqlitePageRefStore;
        let (db, tid, item_id) = fixture().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteWorkNoteStore::new(db).with_page_refs(page_refs.clone());

        let note = store
            .add_for_item(&item_id, "blocked by wi-019zzz-1 see [[src/app.rs]]", "u")
            .await
            .unwrap();
        let inbound_wi = page_refs
            .list_backlinks("work-item", "wi-019zzz-1", None)
            .await
            .unwrap();
        assert!(
            inbound_wi
                .iter()
                .any(|e| e.source_kind == "work-note" && e.source_id == note.id.as_str()),
            "expected note to backlink wi-019zzz-1; got {inbound_wi:?}"
        );
        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound_file.iter().any(|e| e.source_id == note.id.as_str()));

        // Editing the body replaces the slice cleanly.
        store.update_body(&note.id, "no refs").await.unwrap();
        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound_file.iter().all(|e| e.source_id != note.id.as_str()));

        // Delete clears every edge.
        store.delete(&note.id).await.unwrap();
        let inbound_wi = page_refs
            .list_backlinks("work-item", "wi-019zzz-1", None)
            .await
            .unwrap();
        assert!(inbound_wi.iter().all(|e| e.source_id != note.id.as_str()));

        // Threaded notes also work — body parsing doesn't depend on
        // which parent the note attaches to.
        let tnote = store
            .add_for_thread(&tid, "see [[src/lib.rs]]", "u")
            .await
            .unwrap();
        let inbound_lib = page_refs
            .list_backlinks("file", "src/lib.rs", None)
            .await
            .unwrap();
        assert!(inbound_lib.iter().any(|e| e.source_id == tnote.id.as_str()));
    }

    /// When a page-ref store is attached, link create/delete mirrors
    /// the work_item_link slice into `page_ref` and survives
    /// alongside body-mention edges from the work-item store.
    #[tokio::test]
    async fn link_create_delete_projects_page_ref_slice() {
        use crate::page_ref_store::SqlitePageRefStore;
        let (db, tid, from_id) = fixture().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        // Body-mention slice: from_id mentions a file; this should
        // survive link mutations.
        let items = SqliteWorkItemStore::new(db.clone()).with_page_refs(page_refs.clone());
        let mut sender = items.get(&from_id).await.unwrap().unwrap();
        sender.description = "see [[src/app.rs]]".into();
        items.upsert(&sender).await.unwrap();

        // Add a second item to link to.
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

        let links = SqliteWorkItemLinkStore::new(db.clone()).with_page_refs(page_refs.clone());
        let link = links
            .create(&tid, &from_id, &to.id, WorkItemLinkType::Blocks)
            .await
            .unwrap();

        // Backlinks for wi-2 include the link from wi-1.
        let inbound_to = page_refs
            .list_backlinks("work-item", to.id.as_str(), None)
            .await
            .unwrap();
        assert!(inbound_to
            .iter()
            .any(|e| e.source_id == from_id.as_str() && e.ref_type == "work_item_link:blocks"));

        // Body-mention slice still present.
        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(inbound_file.iter().any(|e| e.source_id == from_id.as_str()));

        // Delete the link → only the link slice clears.
        links.delete(&link.id).await.unwrap();
        let inbound_to = page_refs
            .list_backlinks("work-item", to.id.as_str(), None)
            .await
            .unwrap();
        assert!(inbound_to.is_empty(), "link backlink should clear");
        let inbound_file = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(
            inbound_file.iter().any(|e| e.source_id == from_id.as_str()),
            "body-mention slice must survive link deletion"
        );
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
