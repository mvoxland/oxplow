//! Thread domain type (was "batch" in earlier TS code).
//!
//! A thread is a unit of agent work scoped to a stream. Multiple
//! threads can run concurrently per stream; one is "selected" at a
//! time for UI focus.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{StreamId, ThreadId};
use crate::time::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Thread {
    pub id: ThreadId,
    pub stream_id: StreamId,
    pub title: String,
    pub status: ThreadStatus,
    pub sort_index: i64,
    /// Which pane (working/talking) is the agent's primary attach point.
    pub pane_target: String,
    pub resume_session_id: String,
    pub summary: String,
    pub summary_updated_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let now = Timestamp::from_unix_ms(1_700_000_000_000);
        let t = Thread {
            id: ThreadId::from("b-1"),
            stream_id: StreamId::from("s-1"),
            title: "explore".into(),
            status: ThreadStatus::Open,
            sort_index: 0,
            pane_target: "working".into(),
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: Thread = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn status_uses_snake_case() {
        assert_eq!(serde_json::to_string(&ThreadStatus::Open).unwrap(), "\"open\"");
    }
}
