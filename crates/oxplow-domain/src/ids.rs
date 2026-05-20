//! Newtype IDs.
//!
//! Each table that has an external identity gets its own newtype so
//! that mismatched IDs are a compile error.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::fmt;

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(format!("{}-{}", $prefix, uuid::Uuid::now_v7()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
    };
}

id_type!(StreamId, "s");
id_type!(ThreadId, "b"); // "b" matches existing TS convention (b-...)
id_type!(NoteId, "n");
id_type!(AgentTurnId, "at");
id_type!(HookEventId, "he");
id_type!(EffortId, "ef");

/// Task identifier — plain SQLite autoincrement integer.
///
/// The inner field is intentionally private: every construction path
/// goes through one of the named constructors so the "what does this
/// integer mean" question always has a textual answer at the call site.
/// In particular, the `0` value is reserved as the
/// [`TaskId::placeholder`] sentinel that the upsert IPC uses to
/// distinguish "client doesn't know an id yet, allocate one" from
/// "update this row in place". SQLite `AUTOINCREMENT` never issues 0,
/// so the sentinel is unambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct TaskId(i64);

impl TaskId {
    /// Wrap a known id (typically the one SQLite just assigned, or the
    /// one the renderer received from a prior fetch). Don't pass `0`
    /// here — use [`TaskId::placeholder`] when you mean "no id yet".
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// The "I'm about to be inserted" sentinel id. Equal to `TaskId::placeholder()`,
    /// which the SQLite autoincrement allocator never returns.
    pub const fn placeholder() -> Self {
        Self(0)
    }

    /// Returns true iff this id is the placeholder — i.e. the row has
    /// not been persisted yet and the next insert should allocate a
    /// real id.
    pub const fn is_placeholder(self) -> bool {
        self.0 == 0
    }

    pub const fn value(self) -> i64 {
        self.0
    }

    /// Parse from a string (used by the polymorphic TEXT id column in
    /// `page_ref`). Returns `None` if the input isn't all ASCII digits.
    pub fn try_from_str(s: &str) -> Option<Self> {
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        s.parse::<i64>().ok().map(TaskId::new)
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Task-link identifier — plain SQLite autoincrement integer. Private
/// field, same reasoning as [`TaskId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct TaskLinkId(i64);

impl TaskLinkId {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> i64 {
        self.0
    }
}

impl fmt::Display for TaskLinkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Comment identifier — plain SQLite autoincrement integer (no UUIDs).
/// Private field, same reasoning as [`TaskId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct CommentId(i64);

impl CommentId {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> i64 {
        self.0
    }
}

impl fmt::Display for CommentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Comment-message identifier — plain SQLite autoincrement integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct CommentMessageId(i64);

impl CommentMessageId {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> i64 {
        self.0
    }
}

impl fmt::Display for CommentMessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let a = StreamId::new();
        let b = StreamId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn ids_have_prefix() {
        assert!(StreamId::new().as_str().starts_with("s-"));
        assert!(ThreadId::new().as_str().starts_with("b-"));
    }

    #[test]
    fn ids_round_trip_serde() {
        let id = StreamId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: StreamId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn ids_serialize_as_plain_string() {
        let id = StreamId::from("s-fixed");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"s-fixed\"");
    }

    #[test]
    fn task_id_serializes_as_integer() {
        let id = TaskId::new(42);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "42");
        let back: TaskId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn task_id_try_from_str() {
        assert_eq!(TaskId::try_from_str("42"), Some(TaskId::new(42)));
        assert_eq!(TaskId::try_from_str(""), None);
        assert_eq!(TaskId::try_from_str("4a"), None);
        assert_eq!(TaskId::try_from_str("-1"), None);
    }

    #[test]
    fn task_id_placeholder_round_trips_through_predicate() {
        assert!(TaskId::placeholder().is_placeholder());
        assert!(!TaskId::new(1).is_placeholder());
    }
}
