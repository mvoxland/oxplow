use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_domain::DomainError;
use oxplow_session::{SessionError, ThreadError};

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

impl From<DomainError> for IpcError {
    fn from(value: DomainError) -> Self {
        match &value {
            DomainError::Invalid(msg) => Self {
                code: "INVALID".into(),
                message: msg.clone(),
                cause: None,
            },
            DomainError::NotFound => Self::not_found(),
            DomainError::Invariant(msg) => Self {
                code: "INVARIANT".into(),
                message: msg.clone(),
                cause: None,
            },
        }
    }
}

impl From<SessionError> for IpcError {
    fn from(value: SessionError) -> Self {
        match &value {
            SessionError::NotARepo(p) => Self {
                code: "NOT_A_REPO".into(),
                message: format!("not a git repo: {}", p.display()),
                cause: None,
            },
            SessionError::InWorktree(p) => Self {
                code: "IN_WORKTREE".into(),
                message: format!("workspace is a secondary git worktree: {}", p.display()),
                cause: None,
            },
            SessionError::PrimaryExists => Self {
                code: "PRIMARY_EXISTS".into(),
                message: "primary stream already exists".into(),
                cause: None,
            },
            SessionError::PrimaryMissing => Self {
                code: "PRIMARY_MISSING".into(),
                message: "primary stream missing".into(),
                cause: None,
            },
            SessionError::DuplicateWorktreeSlug(slug, sid) => Self {
                code: "DUPLICATE_WORKTREE_SLUG".into(),
                message: format!("worktree slug \"{slug}\" already exists for stream {sid}"),
                cause: None,
            },
            SessionError::Git(e) => Self {
                code: "GIT".into(),
                message: e.to_string(),
                cause: None,
            },
            SessionError::Storage(e) => IpcError::from(e.clone()),
        }
    }
}

impl From<ThreadError> for IpcError {
    fn from(value: ThreadError) -> Self {
        match value {
            ThreadError::NotFound(_) => IpcError::not_found(),
            ThreadError::Closed(id) => Self {
                code: "THREAD_CLOSED".into(),
                message: format!("thread is closed: {id}"),
                cause: None,
            },
            ThreadError::Storage(e) => IpcError::from(e),
        }
    }
}
