use oxplow_app::agent_status_derive::derive_thread_status;
use oxplow_app::HookEnvelope;
use oxplow_domain::stores::AgentTurnStore;
use oxplow_domain::{AgentStatus, AgentTurn, HookEvent, HookKind, ThreadId};

use crate::error::IpcError;
use crate::state::AppState;

/// Land an envelope from the hook subprocess. Drives the agent_turn /
/// agent_status state machine inside HookIngestService.
#[tauri::command]
#[specta::specta]
pub async fn ingest_hook_event(
    state: tauri::State<'_, AppState>,
    envelope: HookEnvelope,
) -> Result<(), IpcError> {
    state
        .hook_ingest
        .ingest(envelope)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn list_hook_events(
    state: tauri::State<'_, AppState>,
    thread_id: Option<ThreadId>,
    limit: Option<usize>,
) -> Result<Vec<HookEvent>, IpcError> {
    let limit = limit.unwrap_or(200);
    Ok(state
        .hook_event_store
        .list_recent(thread_id.as_ref(), limit)
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_hook_events_by_kind(
    state: tauri::State<'_, AppState>,
    kind: HookKind,
    limit: Option<usize>,
) -> Result<Vec<HookEvent>, IpcError> {
    Ok(state
        .hook_event_store
        .list_by_kind(kind, limit.unwrap_or(200))
        .await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_agent_statuses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgentStatus>, IpcError> {
    // Derive each thread's working/waiting state by replaying its
    // hook event log instead of trusting the agent_status row. The
    // sidecar table can drift (a missed Stop, a mis-routed Subagent
    // Stop, a stale boot row) — the hook log is what Claude Code
    // actually emitted, so deriving from it self-heals against
    // ingest-pipeline bugs. Mirrors `src/session/agent-status.ts`
    // on main, which has the proven state machine for this.
    let mut statuses = state.agent_status_store.list_all().await?;
    for s in &mut statuses {
        let events = state
            .hook_event_store
            .list_recent(Some(&s.thread_id), 200)
            .await?;
        s.state = derive_thread_status(&events);
    }
    Ok(statuses)
}

#[tauri::command]
#[specta::specta]
pub async fn list_open_agent_turns(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<AgentTurn>, IpcError> {
    Ok(state.agent_turn_store.list_open(&thread_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_recent_agent_turns(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
    limit: Option<usize>,
) -> Result<Vec<AgentTurn>, IpcError> {
    Ok(state
        .agent_turn_store
        .list_for_thread(&thread_id, limit.unwrap_or(50))
        .await?)
}

// Derivation logic + its unit tests live in
// oxplow_app::agent_status_derive — list_agent_statuses just wires
// the store calls together.
