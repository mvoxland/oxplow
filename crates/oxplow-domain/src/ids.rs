//! Newtype IDs.
//!
//! Each table that has an external identity gets its own newtype so
//! that mismatched IDs are a compile error.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::fmt;

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type,
        )]
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
id_type!(WorkItemId, "wi");
id_type!(NoteId, "n");
id_type!(AgentTurnId, "at");
id_type!(HookEventId, "he");
id_type!(EffortId, "ef");

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
        assert!(WorkItemId::new().as_str().starts_with("wi-"));
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
}
