use oxplow_git::{
    GitFileStatus, WorkspaceEntry, WorkspaceFile, WorkspaceIndexedFile, WorkspaceStatusSummary,
};

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn list_workspace_entries(
    state: tauri::State<'_, AppState>,
    relative_path: String,
) -> Result<Vec<WorkspaceEntry>, IpcError> {
    let root = state.layout.project_dir.clone();
    let entries = tokio::task::spawn_blocking(move || {
        let statuses = oxplow_git::list_git_statuses(&root);
        oxplow_git::list_workspace_entries(&root, &relative_path, &statuses)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(entries)
}

#[tauri::command]
#[specta::specta]
pub async fn list_workspace_files(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WorkspaceIndexedFile>, IpcError> {
    let root = state.layout.project_dir.clone();
    let files = tokio::task::spawn_blocking(move || {
        let statuses = oxplow_git::list_git_statuses(&root);
        oxplow_git::list_workspace_files(&root, &statuses, "")
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(files)
}

#[tauri::command]
#[specta::specta]
pub async fn read_workspace_file(
    state: tauri::State<'_, AppState>,
    relative_path: String,
) -> Result<WorkspaceFile, IpcError> {
    let root = state.layout.project_dir.clone();
    let file = tokio::task::spawn_blocking(move || {
        oxplow_git::read_workspace_file(&root, &relative_path)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(file)
}

#[tauri::command]
#[specta::specta]
pub async fn write_workspace_file(
    state: tauri::State<'_, AppState>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    let root = state.layout.project_dir.clone();
    let file = tokio::task::spawn_blocking(move || {
        oxplow_git::write_workspace_file(&root, &relative_path, &content)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(file)
}

#[tauri::command]
#[specta::specta]
pub async fn create_workspace_file(
    state: tauri::State<'_, AppState>,
    relative_path: String,
    content: String,
) -> Result<WorkspaceFile, IpcError> {
    let root = state.layout.project_dir.clone();
    let file = tokio::task::spawn_blocking(move || {
        oxplow_git::create_workspace_file(&root, &relative_path, &content)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(file)
}

#[tauri::command]
#[specta::specta]
pub async fn create_workspace_directory(
    state: tauri::State<'_, AppState>,
    relative_path: String,
) -> Result<String, IpcError> {
    let root = state.layout.project_dir.clone();
    let path = tokio::task::spawn_blocking(move || {
        oxplow_git::create_workspace_directory(&root, &relative_path)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(path)
}

#[tauri::command]
#[specta::specta]
pub async fn rename_workspace_path(
    state: tauri::State<'_, AppState>,
    from_path: String,
    to_path: String,
) -> Result<(String, String), IpcError> {
    let root = state.layout.project_dir.clone();
    let pair = tokio::task::spawn_blocking(move || {
        oxplow_git::rename_workspace_path(&root, &from_path, &to_path)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(pair)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_workspace_path(
    state: tauri::State<'_, AppState>,
    relative_path: String,
) -> Result<String, IpcError> {
    let root = state.layout.project_dir.clone();
    let path = tokio::task::spawn_blocking(move || {
        oxplow_git::delete_workspace_path(&root, &relative_path)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(path)
}

#[tauri::command]
#[specta::specta]
pub async fn get_workspace_status_summary(
    state: tauri::State<'_, AppState>,
) -> Result<WorkspaceStatusSummary, IpcError> {
    let root = state.layout.project_dir.clone();
    let summary = tokio::task::spawn_blocking(move || {
        let statuses = oxplow_git::list_git_statuses(&root);
        oxplow_git::summarize_git_statuses(&statuses)
    })
    .await
    .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(summary)
}

/// Re-export so the binding for GitFileStatus is generated.
pub fn _capture_git_file_status() -> GitFileStatus {
    GitFileStatus::Modified
}
