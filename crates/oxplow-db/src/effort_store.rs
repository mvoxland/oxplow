//! Task effort tracking.
//!
//! An "effort" is one continuous push of agent work on a single task,
//! bounded by snapshots at start and end. This module owns:
//!
//! - `task_effort` (the effort row)
//! - `task_effort_file` (per-effort file changes)
//! - `task_effort_turn` (link to agent_turn rows)

use async_trait::async_trait;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::{DomainError, EffortId, TaskId, TaskImpact, ThreadId, Timestamp};

use crate::database::Database;
use crate::page_ref_projections::{
    effort_impact_edges, effort_ref_types, effort_summary_edges, effort_touched_file_edges,
    KIND_TASK,
};
use crate::page_ref_store::SqlitePageRefStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EffortFileChange {
    Created,
    Updated,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct TaskEffort {
    pub id: EffortId,
    pub task_id: TaskId,
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

fn row_to_effort(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskEffort> {
    let id: String = row.get("id")?;
    let task_id: i64 = row.get("task_id")?;
    let thread_id: String = row.get("thread_id")?;
    let started_at: String = row.get("started_at")?;
    let ended_at: Option<String> = row.get("ended_at")?;
    let start_snapshot_id: Option<i64> = row.get("start_snapshot_id")?;
    let end_snapshot_id: Option<i64> = row.get("end_snapshot_id")?;
    let summary: Option<String> = row.get("summary")?;
    let map_err = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    Ok(TaskEffort {
        id: EffortId::from(id),
        task_id: TaskId::new(task_id),
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
pub trait TaskEffortStore: Send + Sync {
    async fn start(
        &self,
        task: TaskId,
        thread: &ThreadId,
        start_snapshot_id: Option<i64>,
    ) -> Result<TaskEffort, DomainError>;
    async fn finish(
        &self,
        id: &EffortId,
        end_snapshot_id: Option<i64>,
        summary: Option<String>,
    ) -> Result<(), DomainError>;
    /// Record the LLM-declared cross-page impacts for an effort.
    /// Replaces any prior list. The store then re-projects the
    /// owning task's effort slice so impact edges show up in
    /// `page_ref` immediately.
    async fn set_impacts(&self, id: &EffortId, impacts: &[TaskImpact]) -> Result<(), DomainError>;
    async fn list_for_item(&self, item: TaskId) -> Result<Vec<TaskEffort>, DomainError>;
    async fn list_files(&self, id: &EffortId) -> Result<Vec<EffortFile>, DomainError>;
    async fn list_impacts(&self, id: &EffortId) -> Result<Vec<TaskImpact>, DomainError>;
    async fn record_file(
        &self,
        id: &EffortId,
        path: &str,
        change: EffortFileChange,
    ) -> Result<(), DomainError>;
    /// Efforts whose end_snapshot_id is in the given list.
    async fn list_ending_at_snapshots(
        &self,
        snapshot_ids: Vec<i64>,
    ) -> Result<Vec<TaskEffort>, DomainError>;
}

#[derive(Clone)]
pub struct SqliteTaskEffortStore {
    db: Database,
    page_refs: Option<SqlitePageRefStore>,
}

impl SqliteTaskEffortStore {
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

    /// Re-emit the full effort-owned slice for `task_id` — the
    /// union of touched-file edges, the parsed wikilink/file/dir/
    /// task/finding/commit refs pulled from every effort's
    /// `summary` body, and the declared `TaskImpact` rows.
    /// Replaces under `effort_ref_types()` so the task-body slice
    /// (owned by `task_store`) is unaffected.
    async fn project_effort_slice(&self, task_id: TaskId) -> Result<(), DomainError> {
        let Some(refs) = &self.page_refs else {
            return Ok(());
        };
        let db = self.db.clone();
        type SliceRows = (Vec<(String, String)>, Vec<String>, Vec<String>);
        let (paths, summaries, impact_jsons): SliceRows = tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                // Pick the most-recent `change_kind` per path across
                // every effort on this task. "Most recent" = the
                // effort with the latest `started_at`. The window
                // function isolates rn=1 so each path appears once.
                let mut path_stmt = conn.prepare(
                    "SELECT path, change_kind FROM (
                       SELECT f.path, f.change_kind,
                              ROW_NUMBER() OVER (
                                PARTITION BY f.path
                                ORDER BY e.started_at DESC
                              ) AS rn
                       FROM task_effort_file f
                       JOIN task_effort e ON e.id = f.effort_id
                       WHERE e.task_id = ?1
                     )
                     WHERE rn = 1
                     ORDER BY path",
                )?;
                let paths: Vec<(String, String)> = path_stmt
                    .query_map(params![task_id.value()], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut sum_stmt = conn.prepare(
                    "SELECT summary FROM task_effort
                      WHERE task_id = ?1
                        AND summary IS NOT NULL
                        AND summary <> ''
                      ORDER BY started_at",
                )?;
                let summaries: Vec<String> = sum_stmt
                    .query_map(params![task_id.value()], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut imp_stmt = conn.prepare(
                    "SELECT impacts_json FROM task_effort
                      WHERE task_id = ?1
                        AND impacts_json IS NOT NULL
                        AND impacts_json <> ''
                      ORDER BY started_at",
                )?;
                let impact_jsons: Vec<String> = imp_stmt
                    .query_map(params![task_id.value()], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok((paths, summaries, impact_jsons))
            })
        })
        .await
        .unwrap()?;
        let mut impacts: Vec<TaskImpact> = Vec::new();
        for j in &impact_jsons {
            match serde_json::from_str::<Vec<TaskImpact>>(j) {
                Ok(rows) => impacts.extend(rows),
                Err(e) => {
                    tracing::warn!(?e, "effort impacts_json deserialize failed; skipping");
                }
            }
        }
        let id_str = task_id.to_string();
        let mut edges = effort_touched_file_edges(&id_str, &paths);
        edges.extend(effort_summary_edges(&id_str, &summaries));
        edges.extend(effort_impact_edges(&id_str, &impacts));
        refs.replace_source_for_ref_types(KIND_TASK, &id_str, effort_ref_types(), edges)
            .await
    }

    async fn task_for_effort(&self, effort_id: &EffortId) -> Result<Option<TaskId>, DomainError> {
        let db = self.db.clone();
        let id = effort_id.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT task_id FROM task_effort WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.as_str()], |r| r.get::<_, i64>(0))?;
                Ok(rows.next().transpose()?.map(TaskId::new))
            })
        })
        .await
        .unwrap()
    }
}

#[async_trait]
impl TaskEffortStore for SqliteTaskEffortStore {
    async fn start(
        &self,
        task: TaskId,
        thread: &ThreadId,
        start_snapshot_id: Option<i64>,
    ) -> Result<TaskEffort, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        let now = Timestamp::now();
        let id = EffortId::new();
        let id_for_sql = id.clone();
        let thread_for_sql = thread.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO task_effort
                       (id, task_id, thread_id, started_at, ended_at,
                        start_snapshot_id, end_snapshot_id, summary)
                     VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, NULL)",
                    params![
                        id_for_sql.as_str(),
                        task.value(),
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
        Ok(TaskEffort {
            id,
            task_id: task,
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
        let id_for_sql = id.clone();
        let summary_has_body = summary
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let now = ts_to_string(Timestamp::now());
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE task_effort
                     SET ended_at = ?2, end_snapshot_id = ?3, summary = ?4
                     WHERE id = ?1 AND ended_at IS NULL",
                    params![id_for_sql.as_str(), now, end_snapshot_id, summary],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()?;
        if summary_has_body && self.page_refs.is_some() {
            if let Some(tid) = self.task_for_effort(id).await? {
                self.project_effort_slice(tid).await?;
            }
        }
        Ok(())
    }

    async fn list_for_item(&self, item: TaskId) -> Result<Vec<TaskEffort>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM task_effort WHERE task_id = ?1
                     ORDER BY started_at DESC",
                )?;
                let rows = stmt.query_map(params![item.value()], row_to_effort)?;
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
                    "SELECT effort_id, path, change_kind FROM task_effort_file
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

    async fn set_impacts(&self, id: &EffortId, impacts: &[TaskImpact]) -> Result<(), DomainError> {
        let db = self.db.clone();
        let id_clone = id.clone();
        let json = if impacts.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(impacts)
                    .map_err(|e| DomainError::Invalid(format!("impacts serialize failed: {e}")))?,
            )
        };
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE task_effort SET impacts_json = ?2 WHERE id = ?1",
                    params![id_clone.as_str(), json],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()?;
        if self.page_refs.is_some() {
            if let Some(tid) = self.task_for_effort(id).await? {
                self.project_effort_slice(tid).await?;
            }
        }
        Ok(())
    }

    async fn list_impacts(&self, id: &EffortId) -> Result<Vec<TaskImpact>, DomainError> {
        let db = self.db.clone();
        let id = id.clone();
        let raw: Option<String> = tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt =
                    conn.prepare("SELECT impacts_json FROM task_effort WHERE id = ?1")?;
                let mut rows =
                    stmt.query_map(params![id.as_str()], |r| r.get::<_, Option<String>>(0))?;
                Ok(rows.next().transpose()?.flatten())
            })
        })
        .await
        .unwrap()?;
        match raw {
            Some(json) if !json.is_empty() => serde_json::from_str(&json)
                .map_err(|e| DomainError::Invalid(format!("impacts deserialize failed: {e}"))),
            _ => Ok(Vec::new()),
        }
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
                    "INSERT OR REPLACE INTO task_effort_file
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
            if let Some(tid) = self.task_for_effort(id).await? {
                self.project_effort_slice(tid).await?;
            }
        }
        Ok(())
    }

    async fn list_ending_at_snapshots(
        &self,
        snapshot_ids: Vec<i64>,
    ) -> Result<Vec<TaskEffort>, DomainError> {
        if snapshot_ids.is_empty() {
            return Ok(vec![]);
        }
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let placeholders: Vec<String> =
                    (1..=snapshot_ids.len()).map(|i| format!("?{i}")).collect();
                let sql = format!(
                    "SELECT * FROM task_effort WHERE end_snapshot_id IN ({})
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
    use crate::task_store::SqliteTaskStore;
    use crate::thread_store::SqliteThreadStore;
    use oxplow_domain::stores::{StreamStore, TaskStore, ThreadStore};
    use oxplow_domain::{
        Stream, StreamId, StreamKind, Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus,
        Thread, ThreadStatus,
    };

    async fn fixture() -> (SqliteTaskEffortStore, TaskId, ThreadId) {
        let (store, _db, tid, thread) = fixture_with_db().await;
        (store, tid, thread)
    }

    async fn fixture_with_db() -> (SqliteTaskEffortStore, Database, TaskId, ThreadId) {
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
        let tid = SqliteTaskStore::new(db.clone())
            .insert(&Task {
                id: TaskId::placeholder(),
                thread_id: Some(t.id.clone()),
                parent_id: None,
                title: "x".into(),
                description: String::new(),
                acceptance_criteria: None,
                status: TaskStatus::Ready,
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
            })
            .await
            .unwrap();
        (SqliteTaskEffortStore::new(db.clone()), db, tid, t.id)
    }

    #[tokio::test]
    async fn start_then_finish_round_trips() {
        let (store, tid, t) = fixture().await;
        let eff = store.start(tid, &t, None).await.unwrap();
        assert!(eff.ended_at.is_none());
        store
            .finish(&eff.id, None, Some("done".into()))
            .await
            .unwrap();
        let list = store.list_for_item(tid).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].ended_at.is_some());
        assert_eq!(list[0].summary.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn record_then_list_files() {
        let (store, tid, t) = fixture().await;
        let eff = store.start(tid, &t, None).await.unwrap();
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
    async fn finish_projects_summary_refs_into_page_ref() {
        use crate::page_ref_store::SqlitePageRefStore;
        let (_, db, tid, t) = fixture_with_db().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteTaskEffortStore::new(db).with_page_refs(page_refs.clone());
        let eff = store.start(tid, &t, None).await.unwrap();
        store
            .finish(
                &eff.id,
                None,
                Some("Filed [[url-schemes]] referencing [[src/foo.rs]] and task:99".into()),
            )
            .await
            .unwrap();

        let wiki_back = page_refs
            .list_backlinks("wiki", "url-schemes", None)
            .await
            .unwrap();
        assert!(
            wiki_back.iter().any(|e| e.source_kind == "task"
                && e.source_id == tid.to_string()
                && e.ref_type == "summary_wikilink"),
            "wiki backlink missing; got {wiki_back:?}"
        );

        let file_back = page_refs
            .list_backlinks("file", "src/foo.rs", None)
            .await
            .unwrap();
        assert!(
            file_back
                .iter()
                .any(|e| e.ref_type == "summary_file_ref" && e.source_id == tid.to_string()),
            "file backlink missing; got {file_back:?}"
        );

        let task_back = page_refs.list_backlinks("task", "99", None).await.unwrap();
        assert!(
            task_back
                .iter()
                .any(|e| e.ref_type == "summary_task_mention" && e.source_id == tid.to_string()),
            "task backlink missing; got {task_back:?}"
        );
    }

    #[tokio::test]
    async fn set_impacts_projects_edges_and_round_trips() {
        use crate::page_ref_store::SqlitePageRefStore;
        use oxplow_domain::TaskImpact;
        let (_, db, tid, t) = fixture_with_db().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteTaskEffortStore::new(db).with_page_refs(page_refs.clone());
        let eff = store.start(tid, &t, None).await.unwrap();
        let impacts = vec![
            TaskImpact {
                kind: "wiki".into(),
                id: "url-schemes".into(),
                action: Some("created".into()),
            },
            TaskImpact {
                kind: "git_commit".into(),
                id: "abc1234".into(),
                action: Some("referenced".into()),
            },
        ];
        store.set_impacts(&eff.id, &impacts).await.unwrap();

        // Round-trip read
        let listed = store.list_impacts(&eff.id).await.unwrap();
        assert_eq!(listed, impacts);

        // Edges projected with normalized target kind + action extra
        let wiki = page_refs
            .list_backlinks("wiki", "url-schemes", None)
            .await
            .unwrap();
        let row = wiki
            .iter()
            .find(|e| e.source_id == tid.to_string())
            .expect("wiki impact edge missing");
        assert_eq!(row.ref_type, "impact");
        assert!(row
            .source_extra
            .as_deref()
            .is_some_and(|s| s.contains("created")));

        let commit = page_refs
            .list_backlinks("git-commit", "abc1234", None)
            .await
            .unwrap();
        assert!(commit
            .iter()
            .any(|e| e.source_id == tid.to_string() && e.ref_type == "impact"));

        // Replacing the impact set clears old edges
        store
            .set_impacts(
                &eff.id,
                &[TaskImpact {
                    kind: "wiki".into(),
                    id: "other-page".into(),
                    action: None,
                }],
            )
            .await
            .unwrap();
        let wiki = page_refs
            .list_backlinks("wiki", "url-schemes", None)
            .await
            .unwrap();
        assert!(
            wiki.iter().all(|e| e.source_id != tid.to_string()),
            "old wiki impact edge wasn't replaced: {wiki:?}"
        );

        // Empty list nulls the column and clears all impact edges
        store.set_impacts(&eff.id, &[]).await.unwrap();
        let wiki = page_refs
            .list_backlinks("wiki", "other-page", None)
            .await
            .unwrap();
        assert!(wiki.iter().all(|e| e.source_id != tid.to_string()));
        assert!(store.list_impacts(&eff.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn record_file_keeps_summary_slice_alive() {
        // Regression: when a later effort records a touched file via
        // `record_file`, the projection helper re-runs and must still
        // include summary edges from earlier-finished efforts.
        use crate::page_ref_store::SqlitePageRefStore;
        let (_, db, tid, t) = fixture_with_db().await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let store = SqliteTaskEffortStore::new(db).with_page_refs(page_refs.clone());
        let first = store.start(tid, &t, None).await.unwrap();
        store
            .finish(&first.id, None, Some("Filed [[url-schemes]]".into()))
            .await
            .unwrap();

        let second = store.start(tid, &t, None).await.unwrap();
        store
            .record_file(&second.id, "src/bar.rs", EffortFileChange::Updated)
            .await
            .unwrap();

        let wiki_back = page_refs
            .list_backlinks("wiki", "url-schemes", None)
            .await
            .unwrap();
        assert!(
            wiki_back.iter().any(|e| e.source_id == tid.to_string()),
            "summary slice was clobbered by record_file: {wiki_back:?}"
        );
    }

    #[tokio::test]
    async fn list_ending_at_snapshots_filters() {
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
        let tid = SqliteTaskStore::new(db.clone())
            .insert(&Task {
                id: TaskId::placeholder(),
                thread_id: Some(t.id.clone()),
                parent_id: None,
                title: "x".into(),
                description: String::new(),
                acceptance_criteria: None,
                status: TaskStatus::Ready,
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
            })
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
                snapshot_id: None,
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
                snapshot_id: None,
            })
            .await
            .unwrap();

        let store = SqliteTaskEffortStore::new(db);
        let a = store.start(tid, &t.id, None).await.unwrap();
        let b = store.start(tid, &t.id, None).await.unwrap();
        store.finish(&a.id, Some(snap1), None).await.unwrap();
        store.finish(&b.id, Some(snap2), None).await.unwrap();
        let matches = store.list_ending_at_snapshots(vec![snap1]).await.unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, a.id);
    }
}
