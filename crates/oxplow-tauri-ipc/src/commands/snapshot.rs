use oxplow_db::FileSnapshot;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_snapshots(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<Vec<FileSnapshot>, IpcError> {
    Ok(state.snapshot_store.list_for_path(&path).await?)
}
