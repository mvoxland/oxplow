//! Tauri command + event adapter.
//!
//! Each `#[tauri::command]` is a thin wrapper around an `oxplow-app`
//! service method. Errors convert at this boundary into the
//! frontend-facing `IpcError`. `tauri-specta` exports the typed JS
//! bindings consumed by `apps/desktop/src/tauri-bridge/`.

pub mod error;
pub mod state;

pub use error::IpcError;
pub use state::AppState;
