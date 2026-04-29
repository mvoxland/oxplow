//! Daemon recovery on startup.
//!
//! Reconciles persisted state with the live world after a restart:
//! - Open agent_turn rows whose session is gone get closed with a
//!   synthetic "interrupted by restart" answer.
//! - Background-task in-memory rollups get nothing (they're transient
//!   by design).
//! - Stale agent_status rows referencing now-dead panes get dropped.
//!
//! Called once from `Services::boot` after the DB is open. Idempotent.

use std::sync::Arc;

use tracing::info;

use oxplow_domain::stores::{AgentStatusStore, AgentTurnStore};
use oxplow_domain::{AgentStatusState, DomainError};

use crate::events::{EventBus, OxplowEvent};

#[derive(Clone)]
pub struct RecoveryService {
    turns: Arc<dyn AgentTurnStore>,
    statuses: Arc<dyn AgentStatusStore>,
    events: EventBus,
}

impl RecoveryService {
    pub fn new(
        turns: Arc<dyn AgentTurnStore>,
        statuses: Arc<dyn AgentStatusStore>,
        events: EventBus,
    ) -> Self {
        Self { turns, statuses, events }
    }

    /// Close orphaned `agent_turn` rows + reset every `agent_status`
    /// to Stopped. Returns the number of rows touched per category so
    /// callers can log it.
    pub async fn run(&self) -> Result<RecoveryReport, DomainError> {
        let statuses = self.statuses.list_all().await?;
        let mut closed_turns = 0usize;
        let mut reset_statuses = 0usize;

        // Mark every previously-running pane as stopped. The hook
        // ingest pipeline will move it back to Running on the next
        // UserPromptSubmit.
        for status in statuses {
            if matches!(status.state, AgentStatusState::Running | AgentStatusState::AwaitingUser) {
                self.statuses
                    .upsert(
                        &status.thread_id,
                        &status.pane_target,
                        AgentStatusState::Stopped,
                        Some("recovered_after_restart".into()),
                    )
                    .await?;
                reset_statuses += 1;

                // Close any open turn for that thread — the pane is
                // dead so the turn can't ever Stop on its own.
                let open = self.turns.list_open(&status.thread_id).await?;
                for turn in open {
                    self.turns
                        .close(&turn.id, Some("interrupted_by_restart".into()))
                        .await?;
                    closed_turns += 1;
                }
            }
        }

        if closed_turns > 0 || reset_statuses > 0 {
            self.events.emit(OxplowEvent::HookEventsChanged);
            self.events.emit(OxplowEvent::BackgroundTasksChanged);
        }
        info!(closed_turns, reset_statuses, "daemon recovery complete");
        Ok(RecoveryReport {
            closed_turns,
            reset_statuses,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RecoveryReport {
    pub closed_turns: usize,
    pub reset_statuses: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteAgentStatusStore, SqliteAgentTurnStore, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{AgentTurn, AgentTurnId, Stream, StreamId, StreamKind, Thread, ThreadId, ThreadStatus, Timestamp};

    #[tokio::test]
    async fn closes_open_turn_for_running_pane() {
        let db = Database::in_memory();
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
        };
        SqliteThreadStore::new(db.clone()).upsert(&t).await.unwrap();
        let turns = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let statuses = Arc::new(SqliteAgentStatusStore::new(db.clone()));

        // Seed: a Running pane with an open turn.
        statuses
            .upsert(&t.id, "working", AgentStatusState::Running, None)
            .await
            .unwrap();
        let turn = AgentTurn {
            id: AgentTurnId::new(),
            thread_id: t.id.clone(),
            work_item_id: None,
            prompt: "do".into(),
            answer: None,
            session_id: None,
            started_at: now,
            ended_at: None,
        };
        turns.open(&turn).await.unwrap();

        let svc = RecoveryService::new(turns.clone(), statuses.clone(), EventBus::new());
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_turns, 1);
        assert_eq!(report.reset_statuses, 1);

        let still_open = turns.list_open(&t.id).await.unwrap();
        assert!(still_open.is_empty());
        let status = statuses.get(&t.id, "working").await.unwrap().unwrap();
        assert_eq!(status.state, AgentStatusState::Stopped);
    }

    #[tokio::test]
    async fn idempotent_when_nothing_to_recover() {
        let db = Database::in_memory();
        let turns = Arc::new(SqliteAgentTurnStore::new(db.clone()));
        let statuses = Arc::new(SqliteAgentStatusStore::new(db));
        let svc = RecoveryService::new(turns, statuses, EventBus::new());
        let report = svc.run().await.unwrap();
        assert_eq!(report.closed_turns, 0);
        assert_eq!(report.reset_statuses, 0);
    }
}
