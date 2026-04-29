use oxplow_app::background_task::{StartInput, UpdateInput};
use oxplow_app::{BackgroundTask, BackgroundTaskKind};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_background_tasks(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<BackgroundTask>, IpcError> {
    Ok(state.background_tasks.list_running())
}

#[tauri::command]
#[specta::specta]
pub async fn get_background_task(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<BackgroundTask>, IpcError> {
    Ok(state.background_tasks.get(&id))
}

#[tauri::command]
#[specta::specta]
pub async fn start_background_task(
    state: tauri::State<'_, AppState>,
    kind: BackgroundTaskKind,
    label: String,
    detail: Option<String>,
) -> Result<BackgroundTask, IpcError> {
    Ok(state.background_tasks.start(StartInput {
        kind,
        label,
        detail,
        progress: None,
    }))
}

#[tauri::command]
#[specta::specta]
pub async fn complete_background_task(
    state: tauri::State<'_, AppState>,
    id: String,
    result: Option<serde_json::Value>,
) -> Result<(), IpcError> {
    state.background_tasks.complete(&id, result);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn fail_background_task(
    state: tauri::State<'_, AppState>,
    id: String,
    error: String,
) -> Result<(), IpcError> {
    state.background_tasks.fail(&id, error, None);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_background_task(
    state: tauri::State<'_, AppState>,
    id: String,
    label: Option<String>,
    detail: Option<Option<String>>,
    progress: Option<Option<f64>>,
) -> Result<(), IpcError> {
    state.background_tasks.update(
        &id,
        UpdateInput {
            label,
            detail,
            progress,
        },
    );
    Ok(())
}
