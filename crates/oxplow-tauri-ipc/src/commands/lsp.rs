use oxplow_app::lsp_clients::LspClientError;

use crate::error::IpcError;
use crate::state::AppState;

impl From<LspClientError> for IpcError {
    fn from(value: LspClientError) -> Self {
        match value {
            LspClientError::NotFound(id) => IpcError::invalid(format!("lsp client not found: {id}")),
            LspClientError::NoConfig(lang) => {
                IpcError::invalid(format!("no lsp server configured for language `{lang}`"))
            }
            LspClientError::Spawn(msg) => IpcError::internal(msg),
            LspClientError::Dropped => IpcError::internal("lsp client dropped"),
        }
    }
}

/// Spawn a new language-server child for `(stream_id, language_id)`.
/// Returns an opaque `client_id` the renderer uses to address
/// subsequent send/close commands. The cwd is resolved from the
/// stream's worktree path; if the stream isn't found we fall back to
/// the project dir.
#[tauri::command]
#[specta::specta]
pub async fn open_lsp_client(
    state: tauri::State<'_, AppState>,
    stream_id: String,
    language_id: String,
) -> Result<String, IpcError> {
    let cwd = state
        .streams
        .list_streams()
        .await
        .ok()
        .and_then(|streams| {
            streams
                .into_iter()
                .find(|s| s.id.as_str() == stream_id)
                .map(|s| std::path::PathBuf::from(&s.worktree_path))
        })
        .unwrap_or_else(|| state.layout.project_dir.clone());
    let id = state.lsp_clients.open(&language_id, cwd).await?;
    Ok(id)
}

/// Forward a raw JSON-RPC frame body (no headers) from the renderer
/// to the language server addressed by `client_id`.
#[tauri::command]
#[specta::specta]
pub async fn send_lsp_message(
    state: tauri::State<'_, AppState>,
    client_id: String,
    payload: String,
) -> Result<(), IpcError> {
    state.lsp_clients.send(&client_id, payload).await?;
    Ok(())
}

/// Tear down the language server backing `client_id`. Idempotent on
/// already-closed clients (returns `INVALID` rather than panicking).
#[tauri::command]
#[specta::specta]
pub async fn close_lsp_client(
    state: tauri::State<'_, AppState>,
    client_id: String,
) -> Result<(), IpcError> {
    state.lsp_clients.close(&client_id).await?;
    Ok(())
}
