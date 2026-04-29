use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppVersion {
    pub version: &'static str,
}

#[tauri::command]
#[specta::specta]
pub async fn app_version() -> Result<AppVersion, IpcError> {
    Ok(AppVersion {
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Liveness check the UI uses to verify the daemon is reachable.
#[tauri::command]
#[specta::specta]
pub async fn ping() -> Result<&'static str, IpcError> {
    Ok("pong")
}
