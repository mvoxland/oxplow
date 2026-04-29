use oxplow_app::BacklogState;
use oxplow_domain::stores::WorkItemStore;
use oxplow_domain::WorkItem;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_backlog(state: tauri::State<'_, AppState>) -> Result<Vec<WorkItem>, IpcError> {
    Ok(state.work_item_store.list_backlog().await?)
}

/// Bucketed backlog view: ready/blocked/in_progress/done.
#[tauri::command]
#[specta::specta]
pub async fn get_backlog_state(
    state: tauri::State<'_, AppState>,
) -> Result<BacklogState, IpcError> {
    let rows = state.work_items.list_backlog().await?;
    Ok(BacklogState::from_rows(rows))
}
