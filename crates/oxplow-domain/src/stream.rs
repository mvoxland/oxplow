//! Stream domain type.
//!
//! A `Stream` is the project-level workspace context. Exactly one
//! `primary` stream per project (representing the repo root); every
//! other stream is a `worktree` stream with its own
//! `.oxplow/worktrees/<slug>/`. Encoded in `.context/architecture.md`.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::StreamId;
use crate::time::Timestamp;

/// Whether a stream is the project's primary stream or a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Primary,
    Worktree,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Stream {
    pub id: StreamId,
    pub kind: StreamKind,
    pub title: String,
    pub summary: String,
    pub branch: String,
    pub branch_ref: String,
    pub branch_source: String,
    pub worktree_path: String,
    pub working_pane: String,
    pub talking_pane: String,
    pub working_session_id: String,
    pub talking_session_id: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let now = Timestamp::from_unix_ms(1_700_000_000_000);
        let s = Stream {
            id: StreamId::from("s-primary"),
            kind: StreamKind::Primary,
            title: "oxplow".into(),
            summary: String::new(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/repo".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Stream = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn kind_uses_snake_case() {
        assert_eq!(serde_json::to_string(&StreamKind::Primary).unwrap(), "\"primary\"");
        assert_eq!(serde_json::to_string(&StreamKind::Worktree).unwrap(), "\"worktree\"");
    }
}
