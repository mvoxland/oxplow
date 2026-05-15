use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::config_service::{mutate_config, read_config};
use oxplow_config::OxplowConfig;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_config(state: tauri::State<'_, AppState>) -> Result<OxplowConfig, IpcError> {
    Ok(read_config(&state.config))
}

#[tauri::command]
#[specta::specta]
pub async fn set_agent_prompt_append(
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<OxplowConfig, IpcError> {
    let project = state.layout.project_dir.clone();
    mutate_config(&state.config, &project, |c| c.agent_prompt_append = text)
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn set_snapshot_retention_days(
    state: tauri::State<'_, AppState>,
    days: u32,
) -> Result<OxplowConfig, IpcError> {
    let project = state.layout.project_dir.clone();
    mutate_config(&state.config, &project, |c| {
        c.snapshot_retention_days = days
    })
    .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn set_snapshot_max_file_bytes(
    state: tauri::State<'_, AppState>,
    bytes: u64,
) -> Result<OxplowConfig, IpcError> {
    let project = state.layout.project_dir.clone();
    mutate_config(&state.config, &project, |c| {
        c.snapshot_max_file_bytes = bytes
    })
    .map_err(|e| IpcError::internal(e.to_string()))
}

#[tauri::command]
#[specta::specta]
pub async fn set_generated(
    state: tauri::State<'_, AppState>,
    entries: Vec<String>,
) -> Result<OxplowConfig, IpcError> {
    let project = state.layout.project_dir.clone();
    mutate_config(&state.config, &project, |c| c.generated = entries)
        .map_err(|e| IpcError::internal(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkspaceContext {
    pub project_dir: String,
    pub default_branch: Option<String>,
    pub is_git_repo: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn get_workspace_context(
    state: tauri::State<'_, AppState>,
) -> Result<WorkspaceContext, IpcError> {
    let project = state.layout.project_dir.clone();
    let project_str = project.to_string_lossy().into_owned();
    let is_git_repo = tokio::task::spawn_blocking(move || oxplow_git::is_git_repo(&project))
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    let default_branch = if is_git_repo {
        state.git.detect_default_branch().await
    } else {
        None
    };
    Ok(WorkspaceContext {
        project_dir: project_str,
        default_branch,
        is_git_repo,
    })
}
