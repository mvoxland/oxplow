use std::sync::Arc;

/// The canonical state type registered with `tauri::Builder::manage`.
///
/// Use this exact alias from every `#[tauri::command]` parameter list:
/// `state: tauri::State<'_, AppState>`. A type mismatch is a runtime
/// panic, so consistency matters.
pub type AppState = Arc<oxplow_app::Services>;
