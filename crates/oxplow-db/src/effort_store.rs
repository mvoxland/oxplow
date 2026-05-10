//! Work-item effort tracking.
//!
//! An "effort" is one continuous push of agent work on a single work
//! item, bounded by snapshots at start and end. This module owns:
//!
//! - `work_item_effort` (the effort row)
//! - `work_item_effort_file` (per-effort file changes)
//! - `work_item_effort_turn` (link to agent_turn rows)
//!
//! Powers the original `listWorkItemEfforts` / `getEffortFiles` /
//! `listEffortsEndingAtSnapshots` IPC surface.

use async_trait::async_trait;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, EffortId, ThreadId, Timestamp, WorkItemId};

use crate::database::Database;
use crate::page_ref_projections::{effort_ref_types, effort_touched_file_edges, KIND_WORK_ITEM};
use crate::page_ref_store::SqlitePageRefStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EffortFileChange {
    Created,
    Updated,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WorkItemEffort {
    pub id: EffortId,
    pub work_item_id: WorkItemId,
    pub thread_id: ThreadId,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub start_snapshot_id: Option<i64>,
    pub end_snapshot_id: Option<i64>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct EffortFile {
    pub effort_id: EffortId,
    pub path: String,
    pub change: EffortFileChange,
}

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

fn change_to_str(c: EffortFileChange) -> &'static str {
    match c {
        EffortFileChange::Created => "created",
        EffortFileChange::Updated => "updated",
        EffortFileChange::Deleted => "deleted",
    }
}

fn str_to_change(s: &str) -> Result<EffortFileChange, DomainError> {
    Ok(match s {
        "created" => EffortFileChange::Created,
        "updated" => EffortFileChange::Updated,
        "deleted" => EffortFileChange::Deleted,
        other => {
            return Err(DomainError::Invalid(format!(
                "unknown effort file change kind: {other}"
            )))
        }
    })
}

fn row_to_effort(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItemEffort> {
    let id: String = row.get("id")?;
    let work_item_id: String = row.get("work_item_id")?;
    let thread_id: String = row.get("thread_id")?;
    let started_at: String = row.get("started_at")?;
    let ended_at: Option<String> = row.get("ended_at")?;
    let start_snapshot_id: Option<i64> = row.get("start_snapshot_id")?;
    let end_snapshot_id: Option<i64> = row.get("end_snapshot_id")?;
    let summary: Option<String> = row.get("summary")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(WorkItemEffort {
        id: EffortId::from(id),
        work_item_id: WorkItemId::from(work_item_id),
        thread_id: ThreadId::from(thread_id),
        started_at: string_to_ts(&started_at).map_err(map_err)?,
        ended_at: ended_at
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(map_err)?,
        start_snapshot_id,
        end_snapshot_id,
        summary,
    })
}

#[async_trait]
pub trait WorkItemEffortStore: Send + Sync {
    async fn start(
        &self,
        work_item: &WorkItemId,
        thread: &ThreadId,
        start_snapshot_id: Option<i64>,
    ) -> Result<WorkItemEffort, DomainError>;
    async fn finish(
        &self,
        id: &EffortId,
        end_snapshot_id: Option<i64>,
        summary: Option<String>,
    ) -> Result<(), DomainError>;
    async fn list_for_item(&self, item: &WorkItemId) -> Result<Vec<WorkItemEffort>, DomainError>;
    async fn list_files(&self, id: &EffortId) -> Result<Vec<EffortFile>, DomainError>;
    async fn record_file(
        &self,
        id: &EffortId,
        path: &str,
        change: EffortFileChange,
    ) -> Result<(), DomainError>;
    /// Efforts whose end_snapshot_id is in the given list. Powers the
    /// "which efforts captured this snapshot?" rail.
    async fn list_ending_at_snapshots(
        &self,
        snapshot_ids: Vec<i64>,
    ) -> Result<Vec<WorkItemEffort>, DomainError>;
}

#[derive(Clone)]
pub struct SqliteWorkItemEffortStore {
    db: Database,
    page_refs: Option<SqlitePageRefStore>,
}

impl SqliteWorkItemEffortStore {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            page_refs: None,
        }
    }

    pub fn with_page_refs(mut self, store: SqlitePageRefStore) -> Self {
        self.page_refs = Some(store);
        self
    }

    /// Re-emit the `touched_file` slice for `work_item_id` from the
    /// union of all efforts attached to it. We project per work-item
    /// rather than per effort so removing an effort cleans up its
    /// orphaned files automatically. Only the `touched_file` slice
    /// is replaced; body-mention and link slices survive.
    async fn project_touched_files(&self, work_item_id: &WorkItemId) -> Result<(), DomainError> {
        let Some(refs) = &self.page_refs else {
            return Ok(());
        };
        let db = self.db.clone();
        let wi = work_item_id.clone();
        let paths: Vec<String> = tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT f.path FROM work_item_effort_file f
                       JOIN work_item_effort e ON e.id = f.effort_id
                      WHERE e.work_item_id = ?1
                      ORDER BY f.path",
                )?;
                let rows = stmt.query_map(params![wi.as_str()], |r| r.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()?;
        let edges = effort_touched_file_edges(work_item_id.as_str(), &paths);
        refs.replace_source_for_ref_types(
            KIND_WORK_ITEM,
            work_item_id.as_str(),
            effort_ref_types(),
            edges,
        )
        .await
    }

    /// Look up the work_item_id for an effort id (small helper used
    /// by `record_file` so we know which work-item to re-project).
    async fn work_item_for_effort(
        &self,
        effort_id: &EffortId,
    ) -> Result<Option<WorkItemId>, DomainError> {
        let db = self.db.clone();
        let id = effort_id.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt =
                    conn.prepare("SELECT work_item_id FROM work_item_effort WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.as_str()], |r| r.get::<_, String>(0))?;
                Ok(rows.next().transpose()?.map(WorkItemId::from))
            })
        })
        .await
        .unwrap()
    }
}

#[async_trait]
impl WorkItemEffortStore for SqliteWorkItemEffortStore {
    async fn start(
        &self,
        work_item: &WorkItemId,
        thread: &ThreadId,
        start_snapshot_id: Option<i64>,
    ) -> Result<WorkItemEffort, DomainError> {
        let db = self.db.clone();
        let work_item = work_item.clone();
        let thread = thread.clone();
        let now = Timestamp::now();
        let id = EffortId::new();
        let id_for_sql = id.clone();
        let work_item_for_sql = work_item.clone();
        let thread_for_sql = thread.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO work_item_effort
                       (id, work_item_id, thread_id, started_at, ended_at,
                        start_snapshot_id, end_snapshot_id, summary)
                     VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, NULL)",
                    params![
                        id_for_sql.as_str(),
                        work_item_for_sql.as_str(),
                        thread_for_sql.as_str(),
                        ts_to_string(now),
                        start_snapshot_id,
                    ],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()?;
        Ok(WorkItemEffort {
            id,
            work_item_id: work_item,
            thread_id: thread,
            started_at: now,
            ended_at: None,
            start_snapshot_id,
            end_snapshot_id: None,
            summary: None,
        })
    }

    async fn finish(
        &self,
        id: &EffortId,
        end_snapshot_id: Option<i64>,
        summary: Option<String>,
    ) -> Result<(), DomainError> {
        let db = self.db.clone();
        let id = id.clone();
        let now = ts_to_string(Timestamp::now());
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE work_item_effort
                     SET ended_at = ?2, end_snapshot_id = ?3, summary = ?4
                     WHERE id = ?1 AND ended_at IS NULL",
                    params![id.as_str(), now, end_snapshot_id, summary],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn list_for_item(&self, item: &WorkItemId) -> Result<Vec<WorkItemEffort>, DomainError> {
        let db = self.db.clone();
        let item = item.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM work_item_effort WHERE work_item_id = ?1
                     ORDER BY started_at DESC",
                )?;
                let rows = stmt.query_map(params![item.as_str()], row_to_effort)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn list_files(&self, id: &EffortId) -> Result<Vec<EffortFile>, DomainError> {
        let db = self.db.clone();
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT effort_id, path, change_kind FROM work_item_effort_file
                     WHERE effort_id = ?1 ORDER BY path ASC",
                )?;
                let rows = stmt.query_map(params![id.as_str()], |r| {
                    let effort_id: String = r.get(0)?;
                    let path: String = r.get(1)?;
                    let kind: String = r.get(2)?;
                    let map_err = |e: DomainError| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    };
                    Ok(EffortFile {
                        effort_id: EffortId::from(effort_id),
                        path,
                        change: str_to_change(&kind).map_err(map_err)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
        })
        .await
        .unwrap()
    }

    async fn record_file(
        &self,
        id: &EffortId,
        path: &str,
        change: EffortFileChange,
    ) -> Result<(), DomainError> {
        let db = self.db.clone();
        let id_clone = id.clone();
        let path_clone = path.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO work_item_effort_file
                       (effort_id, path, change_kind)
                     VALUES (?1, ?2, ?3)",
                    params![id_clone.as_str(), path_clone, change_to_str(change)],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()?;
        if self.page_refs.is_some() {
            if let Some(wi) = self.work_item_for_effort(id).await? {
                self.project_touched_files(&wi).await?;
            }
        }
        Ok(())
    }

    async fn list_ending_at_snapshots(
        &self,
        snapshot_ids: Vec<i64>,
    ) -> Result<Vec<WorkItemEffort>, DomainError> {
        if snapshot_ids.is_empty() {
            return Ok(vec![]);
        }
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                // Build the IN-clause dynamically; rusqlite doesn't bind
                // arrays directly.
                let placeholders: Vec<String> =
                    (1..=snapshot_ids.len()).map(|i| format!("?{i}")).collect();
                let sql = format!(
                    "SELECT * FROM work_item_effort WHERE end_snapshot_id IN ({})
                     ORDER BY ended_at DESC",
                    placeholders.join(",")
                );
                let mut stmt = conn.prepare(&sql)?;
                let params_iter: Vec<&dyn rusqlite::ToSql> = snapshot_ids
                    .iter()
                    .map(|id| id as &dyn rusqlite::ToSql)
                    .collect();
                let rows =
                    stmt.query_map(rusqlite::params_from_iter(params_iter), row_to_effort)?;
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
        Stream, StreamId, StreamKind, Thread, ThreadStatus, WorkItem, WorkItemActorKind,
        WorkItemAuthor, WorkItemKind, WorkItemPriority, WorkItemStatus,
    };

    async fn fixture() -> (SqliteWorkItemEffortStore, WorkItemId, ThreadId) {
        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(1);
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
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteStreamStore::new(db.clone()).upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::from("b-1"),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let wi = WorkItem {
            id: WorkItemId::from("wi-1"),
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
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(WorkItemAuthor::User),
            category: None,
            tags: None,
        };
        SqliteWorkItemStore::new(db.clone())
            .upsert(&wi)
            .await
            .unwrap();
        (SqliteWorkItemEffortStore::new(db), wi.id, t.id)
    }

    #[tokio::test]
    async fn start_then_finish_round_trips() {
        let (store, wi, t) = fixture().await;
        let eff = store.start(&wi, &t, None).await.unwrap();
        assert!(eff.ended_at.is_none());
        store
            .finish(&eff.id, None, Some("done".into()))
            .await
            .unwrap();
        let list = store.list_for_item(&wi).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].ended_at.is_some());
        assert_eq!(list[0].summary.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn record_then_list_files() {
        let (store, wi, t) = fixture().await;
        let eff = store.start(&wi, &t, None).await.unwrap();
        store
            .record_file(&eff.id, "src/a.rs", EffortFileChange::Created)
            .await
            .unwrap();
        store
            .record_file(&eff.id, "src/b.rs", EffortFileChange::Updated)
            .await
            .unwrap();
        let files = store.list_files(&eff.id).await.unwrap();
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn list_ending_at_snapshots_filters() {
        // Captures real snapshots first so the FK on end_snapshot_id is
        // satisfied. Without this the rows would fail to insert.
        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(1);
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
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteStreamStore::new(db.clone()).upsert(&s).await.unwrap();
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
            created_at: now,
            updated_at: now,
            archived_at: None,
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let wi = WorkItem {
            id: WorkItemId::from("wi-1"),
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
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(WorkItemAuthor::User),
            category: None,
            tags: None,
        };
        SqliteWorkItemStore::new(db.clone())
            .upsert(&wi)
            .await
            .unwrap();
        let snap_store = crate::SqliteSnapshotStore::new(db.clone());
        let snap1 = snap_store
            .capture(crate::FileSnapshot {
                id: 0,
                stream_id: Some(s.id.clone()),
                path: "a.txt".into(),
                blob_hash: Some("h1".into()),
                size_bytes: 1,
                captured_at: now,
                oversize: false,
            })
            .await
            .unwrap();
        let snap2 = snap_store
            .capture(crate::FileSnapshot {
                id: 0,
                stream_id: Some(s.id.clone()),
                path: "a.txt".into(),
                blob_hash: Some("h2".into()),
                size_bytes: 2,
                captured_at: now,
                oversize: false,
            })
            .await
            .unwrap();

        let store = SqliteWorkItemEffortStore::new(db);
        let a = store.start(&wi.id, &t.id, None).await.unwrap();
        let b = store.start(&wi.id, &t.id, None).await.unwrap();
        store.finish(&a.id, Some(snap1), None).await.unwrap();
        store.finish(&b.id, Some(snap2), None).await.unwrap();
        let matches = store.list_ending_at_snapshots(vec![snap1]).await.unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, a.id);
    }
}
