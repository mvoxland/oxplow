use oxplow_app::Followup;
use oxplow_domain::ThreadId;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_followups(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<Vec<Followup>, IpcError> {
    Ok(state.followups.list_for_thread(&thread_id))
}

#[tauri::command]
#[specta::specta]
pub async fn add_followup(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
    body: String,
) -> Result<Followup, IpcError> {
    Ok(state.followups.add(thread_id, body))
}

#[tauri::command]
#[specta::specta]
pub async fn remove_followup(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), IpcError> {
    state.followups.remove(&id);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn clear_followups_for_thread(
    state: tauri::State<'_, AppState>,
    thread_id: ThreadId,
) -> Result<(), IpcError> {
    state.followups.clear_for_thread(&thread_id);
    Ok(())
}
