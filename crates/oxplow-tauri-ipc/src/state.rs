use std::sync::Arc;

/// The canonical state type registered with `tauri::Builder::manage`.
///
/// Use this exact alias from every `#[tauri::command]` parameter list:
/// `state: tauri::State<'_, AppState>`. A type mismatch is a runtime
/// panic, so consistency matters.
pub type AppState = Arc<oxplow_app::Services>;

/// Tauri-managed handle to the in-process control plane (axum server
/// hosting hook + MCP routes) plus the per-spawn token. terminal.rs
/// reads it at agent-spawn time so it can materialize the plugin dir
/// and thread the URLs / token into the agent process env.
///
/// Decoupled from `Services` so the boot order doesn't gain a new
/// dependency: control-plane spawn happens after `Services::boot`,
/// inside the Tauri shell, and is registered via `.manage(…)`
/// alongside `AppState`.
#[derive(Clone, Debug)]
pub struct PluginRuntime {
    pub hook_base_url: String,
    pub mcp_endpoint_url: String,
    pub hook_token: String,
}

pub type PluginRuntimeState = Arc<PluginRuntime>;
