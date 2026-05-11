use oxplow_app::BacklogState;
use oxplow_domain::stores::TaskStore;
use oxplow_domain::Task;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_backlog(state: tauri::State<'_, AppState>) -> Result<Vec<Task>, IpcError> {
    Ok(state.task_store.list_backlog().await?)
}

/// Bucketed backlog view: ready/blocked/in_progress/done.
#[tauri::command]
#[specta::specta]
pub async fn get_backlog_state(
    state: tauri::State<'_, AppState>,
) -> Result<BacklogState, IpcError> {
    let rows = state.tasks.list_backlog().await?;
    Ok(BacklogState::from_rows(rows))
}
