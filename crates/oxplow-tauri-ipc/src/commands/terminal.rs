use oxplow_app::agent_command::{build_agent_command, AgentCommandOptions, PaneKind};
use oxplow_app::agent_prompt::assemble_system_prompt;
use oxplow_app::terminal_sessions::{AttachResult, TerminalSessionError};
use oxplow_domain::stores::ThreadStore;
use oxplow_app::terminal_sessions::SpawnRequest;

use crate::error::IpcError;
use crate::state::{AppState, PluginRuntimeState};

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
    plugin_runtime: tauri::State<'_, PluginRuntimeState>,
    pane_target: String,
    cols: u16,
    rows: u16,
    transport_mode: String,
) -> Result<AttachResult, IpcError> {
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
    let thread = match thread_id.clone() {
        Some(id) => state.thread_store.get(&id).await?,
        None => None,
    };

    let config = state.config.read().expect("config rwlock").clone();
    let cols = cols.max(20);
    let rows = rows.max(5);

    // Identity used to deduplicate sessions so re-attaches resume the
    // same PTY instead of spawning a new one. Includes the thread id
    // when known so per-thread state is isolated.
    let session_key = format!(
        "{}|{}|{}|{}",
        stream.id.0,
        thread_id.as_ref().map(|t| t.0.as_str()).unwrap_or(""),
        pane_target,
        transport_mode,
    );

    // Materialize the Claude Code plugin (.oxplow/runtime/claude-plugin/)
    // every spawn — overwrites in place so live edits to skill content
    // take effect without manual cleanup. The hook URL + token live on
    // the control-plane handle stashed at boot; per-spawn identity rides
    // in env vars below.
    let plugin_paths = oxplow_plugin::write_plugin(
        &state.layout.project_dir,
        &plugin_runtime.hook_base_url,
        &plugin_runtime.mcp_endpoint_url,
    )
    .map_err(|e| IpcError::internal(format!("plugin write failed: {e}")))?;

    let plugin_env = vec![
        (
            "OXPLOW_HOOK_TOKEN".to_string(),
            plugin_runtime.hook_token.clone(),
        ),
        ("OXPLOW_STREAM_ID".to_string(), stream.id.0.clone()),
        (
            "OXPLOW_THREAD_ID".to_string(),
            thread_id
                .as_ref()
                .map(|t| t.0.clone())
                .unwrap_or_default(),
        ),
        ("OXPLOW_PANE".to_string(), pane_target.clone()),
    ];

    let result = match transport_mode.as_str() {
        "tmux" => {
            let prompt = assemble_system_prompt(
                &state.layout.project_dir,
                &config,
                &stream,
                thread.as_ref(),
            );
            let opts = AgentCommandOptions {
                plugin_dir: Some(plugin_paths.plugin_dir.to_string_lossy().into_owned()),
                mcp_config: Some(plugin_paths.mcp_config.to_string_lossy().into_owned()),
                env: plugin_env.clone(),
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
            let target_label = outcome.target.as_str().to_string();
            state
                .terminal_sessions
                .attach_or_create(session_key, target_label.clone(), cols, rows, |c, r| {
                    SpawnRequest {
                        command: "tmux".into(),
                        args: vec!["attach-session".into(), "-t".into(), target_label.clone()],
                        cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                        env: vec![
                            ("TERM".into(), "xterm-256color".into()),
                            ("COLORTERM".into(), "truecolor".into()),
                        ],
                        cols: c,
                        rows: r,
                    }
                })
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
                plugin_dir: Some(plugin_paths.plugin_dir.to_string_lossy().into_owned()),
                mcp_config: Some(plugin_paths.mcp_config.to_string_lossy().into_owned()),
                env: plugin_env.clone(),
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
                .attach_or_create(session_key, pane_target.clone(), cols, rows, |c, r| {
                    SpawnRequest {
                        command: "sh".into(),
                        args: vec!["-lc".into(), command],
                        cwd,
                        env: vec![
                            ("TERM".into(), "xterm-256color".into()),
                            ("COLORTERM".into(), "truecolor".into()),
                        ],
                        cols: c,
                        rows: r,
                    }
                })
                .await?
        }
    };
    Ok(result)
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

/// Detach the renderer from `session_id` without killing the PTY —
/// the agent keeps running in the background so the user can navigate
/// away and come back. Use `terminate_terminal_session` to actually
/// stop the agent.
#[tauri::command]
#[specta::specta]
pub async fn close_terminal_session(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), IpcError> {
    let _ = state.terminal_sessions.detach(&session_id).await;
    Ok(())
}

/// Permanently kill the PTY behind `session_id`. Used when a thread
/// is closed or the user explicitly terminates the agent.
#[tauri::command]
#[specta::specta]
pub async fn terminate_terminal_session(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), IpcError> {
    let _ = state.terminal_sessions.close(&session_id).await;
    Ok(())
}
