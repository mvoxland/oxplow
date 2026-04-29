//! Stores for hook_event, agent_status, agent_turn.
//!
//! Each table gets a thin sqlite-backed `Store` impl mirroring the
//! pattern used by stream_store / thread_store.

use async_trait::async_trait;
use rusqlite::params;

use oxplow_domain::stores::{AgentStatusStore, AgentTurnStore, HookEventStore};
use oxplow_domain::{
    AgentStatus, AgentStatusState, AgentTurn, AgentTurnId, DomainError, HookEvent, HookEventId,
    HookKind, StreamId, ThreadId, Timestamp, WorkItemId,
};

use crate::database::Database;

fn ts_to_string(ts: Timestamp) -> String {
    serde_json::to_string(&ts).unwrap().trim_matches('"').to_string()
}

fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DomainError::Invalid(format!("bad timestamp: {e}")))
}

fn map_err_text(e: DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

// -- HookEvent ---------------------------------------------------------

fn hook_kind_to_str(k: HookKind) -> &'static str {
    match k {
        HookKind::UserPromptSubmit => "user_prompt_submit",
        HookKind::PreToolUse => "pre_tool_use",
        HookKind::PostToolUse => "post_tool_use",
        HookKind::Stop => "stop",
        HookKind::SubagentStop => "subagent_stop",
        HookKind::Interrupt => "interrupt",
        HookKind::AgentBoot => "agent_boot",
    }
}

fn str_to_hook_kind(s: &str) -> Result<HookKind, DomainError> {
    Ok(match s {
        "user_prompt_submit" => HookKind::UserPromptSubmit,
        "pre_tool_use" => HookKind::PreToolUse,
        "post_tool_use" => HookKind::PostToolUse,
        "stop" => HookKind::Stop,
        "subagent_stop" => HookKind::SubagentStop,
        "interrupt" => HookKind::Interrupt,
        "agent_boot" => HookKind::AgentBoot,
        other => return Err(DomainError::Invalid(format!("unknown hook kind: {other}"))),
    })
}

fn row_to_hook(row: &rusqlite::Row<'_>) -> rusqlite::Result<HookEvent> {
    let id: String = row.get("id")?;
    let thread_id: Option<String> = row.get("thread_id")?;
    let stream_id: Option<String> = row.get("stream_id")?;
    let kind: String = row.get("kind")?;
    let session_id: Option<String> = row.get("session_id")?;
    let payload_json: String = row.get("payload_json")?;
    let received_at: String = row.get("received_at")?;
    Ok(HookEvent {
        id: HookEventId::from(id),
        thread_id: thread_id.map(ThreadId::from),
        stream_id: stream_id.map(StreamId::from),
        kind: str_to_hook_kind(&kind).map_err(map_err_text)?,
        session_id,
        payload_json,
        received_at: string_to_ts(&received_at).map_err(map_err_text)?,
    })
}

#[derive(Clone)]
pub struct SqliteHookEventStore {
    db: Database,
}

impl SqliteHookEventStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl HookEventStore for SqliteHookEventStore {
    async fn append(&self, event: &HookEvent) -> Result<(), DomainError> {
        let db = self.db.clone();
        let event = event.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO hook_event (id, thread_id, stream_id, kind, session_id, payload_json, received_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        event.id.as_str(),
                        event.thread_id.as_ref().map(|t| t.as_str()),
                        event.stream_id.as_ref().map(|s| s.as_str()),
                        hook_kind_to_str(event.kind),
                        event.session_id,
                        event.payload_json,
                        ts_to_string(event.received_at),
                    ],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn list_recent(
        &self,
        thread: Option<&ThreadId>,
        limit: usize,
    ) -> Result<Vec<HookEvent>, DomainError> {
        let db = self.db.clone();
        let thread = thread.cloned();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| match thread {
                Some(t) => {
                    let mut stmt = conn.prepare(
                        "SELECT * FROM hook_event WHERE thread_id = ?1 ORDER BY received_at DESC LIMIT ?2",
                    )?;
                    let rows = stmt.query_map(params![t.as_str(), limit as i64], row_to_hook)?;
                    rows.collect()
                }
                None => {
                    let mut stmt = conn.prepare(
                        "SELECT * FROM hook_event ORDER BY received_at DESC LIMIT ?1",
                    )?;
                    let rows = stmt.query_map(params![limit as i64], row_to_hook)?;
                    rows.collect()
                }
            })
        })
        .await
        .unwrap()
    }

    async fn list_by_kind(
        &self,
        kind: HookKind,
        limit: usize,
    ) -> Result<Vec<HookEvent>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM hook_event WHERE kind = ?1 ORDER BY received_at DESC LIMIT ?2",
                )?;
                let rows =
                    stmt.query_map(params![hook_kind_to_str(kind), limit as i64], row_to_hook)?;
                rows.collect()
            })
        })
        .await
        .unwrap()
    }
}

// -- AgentStatus -------------------------------------------------------

fn state_to_str(s: AgentStatusState) -> &'static str {
    match s {
        AgentStatusState::Idle => "idle",
        AgentStatusState::Running => "running",
        AgentStatusState::AwaitingUser => "awaiting_user",
        AgentStatusState::Stopped => "stopped",
        AgentStatusState::Error => "error",
    }
}

fn str_to_state(s: &str) -> Result<AgentStatusState, DomainError> {
    Ok(match s {
        "idle" => AgentStatusState::Idle,
        "running" => AgentStatusState::Running,
        "awaiting_user" => AgentStatusState::AwaitingUser,
        "stopped" => AgentStatusState::Stopped,
        "error" => AgentStatusState::Error,
        other => return Err(DomainError::Invalid(format!("unknown agent state: {other}"))),
    })
}

fn row_to_status(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentStatus> {
    let thread_id: String = row.get("thread_id")?;
    let pane_target: String = row.get("pane_target")?;
    let state: String = row.get("state")?;
    let detail: Option<String> = row.get("detail")?;
    let updated_at: String = row.get("updated_at")?;
    Ok(AgentStatus {
        thread_id: ThreadId::from(thread_id),
        pane_target,
        state: str_to_state(&state).map_err(map_err_text)?,
        detail,
        updated_at: string_to_ts(&updated_at).map_err(map_err_text)?,
    })
}

#[derive(Clone)]
pub struct SqliteAgentStatusStore {
    db: Database,
}

impl SqliteAgentStatusStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl AgentStatusStore for SqliteAgentStatusStore {
    async fn upsert(
        &self,
        thread: &ThreadId,
        pane_target: &str,
        state: AgentStatusState,
        detail: Option<String>,
    ) -> Result<AgentStatus, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        let pane_target = pane_target.to_string();
        let now = Timestamp::now();
        let now_str = ts_to_string(now);
        let thread_for_sql = thread.clone();
        let pane_for_sql = pane_target.clone();
        let detail_for_sql = detail.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO agent_status (thread_id, pane_target, state, detail, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(thread_id, pane_target) DO UPDATE SET
                        state = excluded.state,
                        detail = excluded.detail,
                        updated_at = excluded.updated_at",
                    params![
                        thread_for_sql.as_str(),
                        pane_for_sql,
                        state_to_str(state),
                        detail_for_sql,
                        now_str,
                    ],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()?;
        Ok(AgentStatus {
            thread_id: thread,
            pane_target,
            state,
            detail,
            updated_at: now,
        })
    }

    async fn get(
        &self,
        thread: &ThreadId,
        pane_target: &str,
    ) -> Result<Option<AgentStatus>, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        let pane_target = pane_target.to_string();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM agent_status WHERE thread_id = ?1 AND pane_target = ?2",
                )?;
                let mut rows = stmt
                    .query_map(params![thread.as_str(), pane_target], row_to_status)?;
                Ok(rows.next().transpose()?)
            })
        })
        .await
        .unwrap()
    }

    async fn list_all(&self) -> Result<Vec<AgentStatus>, DomainError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn
                    .prepare("SELECT * FROM agent_status ORDER BY updated_at DESC")?;
                let rows = stmt.query_map([], row_to_status)?;
                rows.collect()
            })
        })
        .await
        .unwrap()
    }
}

// -- AgentTurn ---------------------------------------------------------

fn row_to_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTurn> {
    let id: String = row.get("id")?;
    let thread_id: String = row.get("thread_id")?;
    let work_item_id: Option<String> = row.get("work_item_id")?;
    let prompt: String = row.get("prompt")?;
    let answer: Option<String> = row.get("answer")?;
    let session_id: Option<String> = row.get("session_id")?;
    let started_at: String = row.get("started_at")?;
    let ended_at: Option<String> = row.get("ended_at")?;
    Ok(AgentTurn {
        id: AgentTurnId::from(id),
        thread_id: ThreadId::from(thread_id),
        work_item_id: work_item_id.map(WorkItemId::from),
        prompt,
        answer,
        session_id,
        started_at: string_to_ts(&started_at).map_err(map_err_text)?,
        ended_at: ended_at.map(|s| string_to_ts(&s)).transpose().map_err(map_err_text)?,
    })
}

#[derive(Clone)]
pub struct SqliteAgentTurnStore {
    db: Database,
}

impl SqliteAgentTurnStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl AgentTurnStore for SqliteAgentTurnStore {
    async fn open(&self, turn: &AgentTurn) -> Result<(), DomainError> {
        let db = self.db.clone();
        let turn = turn.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO agent_turn (id, thread_id, work_item_id, prompt, answer, session_id, started_at, ended_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET
                        prompt = excluded.prompt,
                        work_item_id = excluded.work_item_id,
                        session_id = excluded.session_id",
                    params![
                        turn.id.as_str(),
                        turn.thread_id.as_str(),
                        turn.work_item_id.as_ref().map(|w| w.as_str()),
                        turn.prompt,
                        turn.answer,
                        turn.session_id,
                        ts_to_string(turn.started_at),
                        turn.ended_at.map(ts_to_string),
                    ],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn close(
        &self,
        id: &AgentTurnId,
        answer: Option<String>,
    ) -> Result<(), DomainError> {
        let db = self.db.clone();
        let id = id.clone();
        let now = ts_to_string(Timestamp::now());
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE agent_turn SET ended_at = ?2, answer = COALESCE(?3, answer)
                     WHERE id = ?1 AND ended_at IS NULL",
                    params![id.as_str(), now, answer],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
    }

    async fn get(&self, id: &AgentTurnId) -> Result<Option<AgentTurn>, DomainError> {
        let db = self.db.clone();
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT * FROM agent_turn WHERE id = ?1")?;
                let mut rows = stmt.query_map(params![id.as_str()], row_to_turn)?;
                Ok(rows.next().transpose()?)
            })
        })
        .await
        .unwrap()
    }

    async fn list_open(&self, thread: &ThreadId) -> Result<Vec<AgentTurn>, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM agent_turn WHERE thread_id = ?1 AND ended_at IS NULL
                     ORDER BY started_at DESC",
                )?;
                let rows = stmt.query_map(params![thread.as_str()], row_to_turn)?;
                rows.collect()
            })
        })
        .await
        .unwrap()
    }

    async fn list_for_thread(
        &self,
        thread: &ThreadId,
        limit: usize,
    ) -> Result<Vec<AgentTurn>, DomainError> {
        let db = self.db.clone();
        let thread = thread.clone();
        tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT * FROM agent_turn WHERE thread_id = ?1
                     ORDER BY started_at DESC LIMIT ?2",
                )?;
                let rows =
                    stmt.query_map(params![thread.as_str(), limit as i64], row_to_turn)?;
                rows.collect()
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
    use oxplow_domain::{Stream, StreamKind, Thread, ThreadStatus};

    async fn fixture() -> (Database, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let now = Timestamp::from_unix_ms(1);
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
            created_at: now,
            updated_at: now,
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
            created_at: now,
            updated_at: now,
        };
        threads.upsert(&t).await.unwrap();
        (db, t.id)
    }

    #[tokio::test]
    async fn hook_event_round_trips_and_lists_recent() {
        let (db, tid) = fixture().await;
        let store = SqliteHookEventStore::new(db);
        let ev = HookEvent {
            id: HookEventId::new(),
            thread_id: Some(tid.clone()),
            stream_id: None,
            kind: HookKind::Stop,
            session_id: Some("sess-1".into()),
            payload_json: r#"{"foo":1}"#.into(),
            received_at: Timestamp::now(),
        };
        store.append(&ev).await.unwrap();
        let recent = store.list_recent(Some(&tid), 10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].kind, HookKind::Stop);
    }

    #[tokio::test]
    async fn agent_status_upsert_replaces_existing() {
        let (db, tid) = fixture().await;
        let store = SqliteAgentStatusStore::new(db);
        store
            .upsert(&tid, "working", AgentStatusState::Running, None)
            .await
            .unwrap();
        store
            .upsert(&tid, "working", AgentStatusState::Idle, Some("done".into()))
            .await
            .unwrap();
        let got = store.get(&tid, "working").await.unwrap().unwrap();
        assert_eq!(got.state, AgentStatusState::Idle);
        assert_eq!(got.detail.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn agent_turn_open_then_close() {
        let (db, tid) = fixture().await;
        let store = SqliteAgentTurnStore::new(db);
        let turn = AgentTurn {
            id: AgentTurnId::new(),
            thread_id: tid.clone(),
            work_item_id: None,
            prompt: "do the thing".into(),
            answer: None,
            session_id: None,
            started_at: Timestamp::now(),
            ended_at: None,
        };
        store.open(&turn).await.unwrap();
        let open = store.list_open(&tid).await.unwrap();
        assert_eq!(open.len(), 1);
        store.close(&turn.id, Some("done".into())).await.unwrap();
        let still_open = store.list_open(&tid).await.unwrap();
        assert!(still_open.is_empty());
        let got = store.get(&turn.id).await.unwrap().unwrap();
        assert!(got.ended_at.is_some());
        assert_eq!(got.answer.as_deref(), Some("done"));
    }
}
