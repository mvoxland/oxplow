use oxplow_app::agent_command::{build_agent_command, AgentCommandOptions, PaneKind};
use oxplow_app::agent_prompt::assemble_system_prompt;
use oxplow_app::terminal_sessions::TerminalSessionError;
use oxplow_domain::stores::ThreadStore;

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

/// Open a renderer-attached terminal session.
///
/// Two transports, mirroring the main-branch design:
/// - `transport_mode == "direct"` — spawn the agent CLI directly via
///   `sh -lc <build_agent_command>` in a PTY; no tmux. The default.
/// - `transport_mode == "tmux"` — `ensure_pane` to create/reuse a
///   tmux session+window running the agent command, then
///   `tmux attach-session -t <resolved-target>`. The target is the
///   `oxplow-<stream-id>:working|talking` form, not the bare slot.
#[tauri::command]
#[specta::specta]
pub async fn open_terminal_session(
    state: tauri::State<'_, AppState>,
    pane_target: String,
    cols: u16,
    rows: u16,
    transport_mode: String,
) -> Result<String, IpcError> {
    let pane_kind = match pane_target.as_str() {
        "working" => PaneKind::Working,
        "talking" => PaneKind::Talking,
        other => {
            return Err(IpcError::invalid(format!(
                "unknown pane target: {other}"
            )))
        }
    };

    // Resolve the stream the user is currently driving. Falls back to
    // the primary so a brand-new project that hasn't called
    // switch_stream still gets a working pane.
    let stream = match state.streams.current().await? {
        Some(s) => s,
        None => state.streams.ensure_primary().await?,
    };

    // Pull the selected thread (if any) so the system prompt the
    // agent sees matches what the renderer is showing.
    let thread_id = state.threads.selected(&stream.id).await?;
    let thread = match thread_id {
        Some(id) => state.thread_store.get(&id).await?,
        None => None,
    };

    let config = state.config.read().expect("config rwlock").clone();
    let cols = cols.max(20);
    let rows = rows.max(5);

    let id = match transport_mode.as_str() {
        "tmux" => {
            let prompt = assemble_system_prompt(
                &state.layout.project_dir,
                &config,
                &stream,
                thread.as_ref(),
            );
            let opts = AgentCommandOptions {
                append_system_prompt: if prompt.is_empty() {
                    None
                } else {
                    Some(prompt)
                },
                ..Default::default()
            };
            let outcome = state
                .agent_panes
                .ensure_pane(&stream, pane_kind, &config, opts)
                .await
                .map_err(|e| IpcError::internal(e.to_string()))?;
            state
                .terminal_sessions
                .open(outcome.target.as_str().to_string(), cols, rows)
                .await?
        }
        // Default to direct.
        _ => {
            let prompt = assemble_system_prompt(
                &state.layout.project_dir,
                &config,
                &stream,
                thread.as_ref(),
            );
            let opts = AgentCommandOptions {
                append_system_prompt: if prompt.is_empty() {
                    None
                } else {
                    Some(prompt)
                },
                ..Default::default()
            };
            let command = build_agent_command(config.agent, &stream, pane_kind, &opts);
            let cwd = std::path::PathBuf::from(&stream.worktree_path);
            state
                .terminal_sessions
                .open_command(pane_target, command, cwd, cols, rows)
                .await?
        }
    };
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
