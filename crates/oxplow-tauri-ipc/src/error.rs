use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

/// Frontend-facing error envelope.
///
/// All `#[tauri::command]` functions return `Result<T, IpcError>`.
/// Internal errors from the service layer are converted here so the
/// JS side never has to reason about Rust-specific error types.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct IpcError {
    pub code: String,
    pub message: String,
    pub cause: Option<String>,
}

impl IpcError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: "INTERNAL".into(),
            message: msg.into(),
            cause: None,
        }
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Self {
            code: "INVALID".into(),
            message: msg.into(),
            cause: None,
        }
    }

    pub fn not_found() -> Self {
        Self {
            code: "NOT_FOUND".into(),
            message: "not found".into(),
            cause: None,
        }
    }

    pub fn with_cause(mut self, cause: impl ToString) -> Self {
        self.cause = Some(cause.to_string());
        self
    }
}
