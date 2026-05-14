//! task effort tracking commands.

use oxplow_db::{EffortAtSnapshot, EffortFile, TaskEffort, TaskEffortStore as _};
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
pub async fn list_efforts_at_snapshots(
    state: tauri::State<'_, AppState>,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<EffortAtSnapshot>, IpcError> {
    Ok(state
        .effort_store
        .list_efforts_at_snapshots(snapshot_ids)
        .await?)
}

/// All distinct file paths whose `file_snapshot` rows fall inside
/// this effort's snapshot bracket — the "all changes during this
/// effort" reference list. Returns empty when the effort has no
/// start/end snapshot pin yet. Drives the reference view shown
/// alongside the canonical `task_effort_file` list on
/// `SnapshotDetailPage`.
#[tauri::command]
#[specta::specta]
pub async fn list_changed_paths_for_effort(
    state: tauri::State<'_, AppState>,
    effort_id: EffortId,
) -> Result<Vec<String>, IpcError> {
    Ok(state
        .effort_store
        .list_changed_paths_for_effort(&effort_id)
        .await?)
}
