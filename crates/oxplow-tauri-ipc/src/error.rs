use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_app::WorkItemServiceError;
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

impl From<WorkItemServiceError> for IpcError {
    fn from(value: WorkItemServiceError) -> Self {
        match value {
            WorkItemServiceError::NotFound(_) => IpcError::not_found(),
            WorkItemServiceError::Storage(e) => IpcError::from(e),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_sets_internal_code() {
        let e = IpcError::internal("boom");
        assert_eq!(e.code, "INTERNAL");
        assert_eq!(e.message, "boom");
        assert!(e.cause.is_none());
    }

    #[test]
    fn invalid_sets_invalid_code() {
        let e = IpcError::invalid("bad input");
        assert_eq!(e.code, "INVALID");
        assert_eq!(e.message, "bad input");
    }

    #[test]
    fn not_found_factory() {
        let e = IpcError::not_found();
        assert_eq!(e.code, "NOT_FOUND");
        assert_eq!(e.message, "not found");
    }

    #[test]
    fn with_cause_attaches_string() {
        let inner = std::io::Error::other("io fault");
        let e = IpcError::internal("wrapped").with_cause(inner);
        assert_eq!(e.cause.as_deref(), Some("io fault"));
    }

    #[test]
    fn from_domain_invalid_uses_invalid_code() {
        let e: IpcError = DomainError::Invalid("bad".into()).into();
        assert_eq!(e.code, "INVALID");
        assert_eq!(e.message, "bad");
    }

    #[test]
    fn from_domain_not_found_maps_to_not_found() {
        let e: IpcError = DomainError::NotFound.into();
        assert_eq!(e.code, "NOT_FOUND");
    }

    #[test]
    fn from_domain_invariant_uses_invariant_code() {
        let e: IpcError = DomainError::Invariant("rule".into()).into();
        assert_eq!(e.code, "INVARIANT");
    }

    #[test]
    fn from_session_not_a_repo_maps() {
        let e: IpcError = SessionError::NotARepo("/no/such".into()).into();
        assert_eq!(e.code, "NOT_A_REPO");
        assert!(e.message.contains("/no/such"));
    }

    #[test]
    fn from_session_primary_missing_maps() {
        let e: IpcError = SessionError::PrimaryMissing.into();
        assert_eq!(e.code, "PRIMARY_MISSING");
    }

    #[test]
    fn ipc_error_serde_round_trips() {
        let e = IpcError::internal("hi").with_cause("inner");
        let json = serde_json::to_string(&e).unwrap();
        let back: IpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, "INTERNAL");
        assert_eq!(back.message, "hi");
        assert_eq!(back.cause.as_deref(), Some("inner"));
    }
}
