use oxplow_domain::stores::WorkItemStore;
use oxplow_domain::WorkItem;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_backlog(state: tauri::State<'_, AppState>) -> Result<Vec<WorkItem>, IpcError> {
    Ok(state.work_item_store.list_backlog().await?)
}
