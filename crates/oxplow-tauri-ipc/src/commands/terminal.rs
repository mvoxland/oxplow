use oxplow_app::terminal_sessions::TerminalSessionError;

use crate::error::IpcError;
use crate::state::AppState;

impl From<TerminalSessionError> for IpcError {
    fn from(value: TerminalSessionError) -> Self {
        match value {
            TerminalSessionError::NotFound(id) => {
                IpcError::invalid(format!("terminal session not found: {id}"))
            }
            TerminalSessionError::Pty(e) => IpcError::internal(e.to_string()),
            TerminalSessionError::InvalidMessage(msg) => IpcError::invalid(msg),
            TerminalSessionError::Base64(msg) => IpcError::invalid(format!("base64: {msg}")),
        }
    }
}

/// Spawn `tmux attach-session -t <pane_target>` and return a handle
/// the renderer addresses by `session_id`. `transport_mode` is
/// accepted for protocol compatibility (the original Electron build
/// used the same value to choose direct vs tmux flows) but oxplow's
/// model always runs through tmux today, so the parameter is recorded
/// for future use but does not branch.
#[tauri::command]
#[specta::specta]
pub async fn open_terminal_session(
    state: tauri::State<'_, AppState>,
    pane_target: String,
    cols: u16,
    rows: u16,
    transport_mode: String,
) -> Result<String, IpcError> {
    let _ = transport_mode;
    let id = state
        .terminal_sessions
        .open(pane_target, cols.max(20), rows.max(5))
        .await?;
    Ok(id)
}

/// Forward a JSON-encoded protocol message from the renderer to the
/// session backing `session_id`. See
/// `oxplow_app::terminal_sessions` for the message shapes.
#[tauri::command]
#[specta::specta]
pub async fn send_terminal_message(
    state: tauri::State<'_, AppState>,
    session_id: String,
    message: String,
) -> Result<(), IpcError> {
    state.terminal_sessions.send(&session_id, &message).await?;
    Ok(())
}

/// Tear down the PTY and forwarder backing `session_id`. Idempotent.
#[tauri::command]
#[specta::specta]
pub async fn close_terminal_session(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), IpcError> {
    let _ = state.terminal_sessions.close(&session_id).await;
    Ok(())
}
