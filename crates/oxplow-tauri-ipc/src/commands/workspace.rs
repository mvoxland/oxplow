use oxplow_git::{
    GitFileStatus, WorkspaceEntry, WorkspaceFile, WorkspaceIndexedFile, WorkspaceStatusSummary,
};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_workspace_entries(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<Vec<WorkspaceEntry>, IpcError> {
    state
        .git
        .list_workspace_entries(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn list_workspace_files(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<Vec<WorkspaceIndexedFile>, IpcError> {
    state
        .git
        .list_workspace_files(stream_id.as_deref())
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn read_workspace_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<WorkspaceFile, IpcError> {
    state
        .git
        .read_workspace_file(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn write_workspace_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    state
        .git
        .write_workspace_file(stream_id.as_deref(), relative_path, content)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn create_workspace_file(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    state
        .git
        .create_workspace_file(stream_id.as_deref(), relative_path, content)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn create_workspace_directory(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    state
        .git
        .create_workspace_directory(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn rename_workspace_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    from_path: String,
    to_path: String,
) -> Result<(String, String), IpcError> {
    state
        .git
        .rename_workspace_path(stream_id.as_deref(), from_path, to_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn delete_workspace_path(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
    relative_path: String,
) -> Result<String, IpcError> {
    state
        .git
        .delete_workspace_path(stream_id.as_deref(), relative_path)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn get_workspace_status_summary(
    state: tauri::State<'_, AppState>,
    stream_id: Option<String>,
) -> Result<WorkspaceStatusSummary, IpcError> {
    Ok(state.git.status_summary(stream_id.as_deref()).await)
}

/// Re-export so the binding for GitFileStatus is generated.
pub fn _capture_git_file_status() -> GitFileStatus {
    GitFileStatus::Modified
}
