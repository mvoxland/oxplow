use oxplow_db::UsageEvent;

use crate::error::IpcError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn record_usage(
    state: tauri::State<'_, AppState>,
    kind: String,
    payload_json: String,
) -> Result<UsageEvent, IpcError> {
    let payload: serde_json::Value =
        serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);
    Ok(state.usage_store.record(&kind, payload).await?)
}

#[tauri::command]
#[specta::specta]
pub async fn list_recent_usage(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<UsageEvent>, IpcError> {
    Ok(state.usage_store.list_recent(limit as usize).await?)
}
