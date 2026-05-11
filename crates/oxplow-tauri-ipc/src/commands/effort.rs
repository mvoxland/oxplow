//! task effort tracking commands.

use oxplow_db::{EffortFile, TaskEffort, TaskEffortStore as _};
use oxplow_domain::{EffortId, TaskId};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_task_efforts(
    state: tauri::State<'_, AppState>,
    item_id: TaskId,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(state.effort_store.list_for_item(item_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn get_effort_files(
    state: tauri::State<'_, AppState>,
    effort_id: EffortId,
) -> Result<Vec<EffortFile>, IpcError> {
    Ok(state.effort_store.list_files(&effort_id).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_efforts_ending_at_snapshots(
    state: tauri::State<'_, AppState>,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(state
        .effort_store
        .list_ending_at_snapshots(snapshot_ids)
        .await?)
}
