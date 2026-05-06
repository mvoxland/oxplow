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

/// Classification of an opaque id string by its `<prefix>-` segment.
///
/// Used by error-reporting at the MCP / IPC boundary to turn raw FK
/// failures into "you passed a stream id where a thread id was
/// expected"-style guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    Stream,
    Thread,
    WorkItem,
    Note,
    AgentTurn,
    HookEvent,
    Effort,
    /// Recognised follow-up id (`fu-…`).
    Followup,
    /// Recognised page-visit id (`pv-…`).
    PageVisit,
    /// Recognised usage-event id (`ue-…`).
    UsageEvent,
    /// Recognised work-item-link id (`wil-…`).
    WorkItemLink,
    /// Recognised background-task id (`bg-…`).
    BackgroundTask,
    /// Has a `<word>-` shape but no known prefix.
    UnknownPrefix(&'static str),
    /// Doesn't have a `<prefix>-<rest>` shape at all.
    Unrecognised,
}

impl IdKind {
    /// Human-readable label used in error messages.
    pub fn label(self) -> &'static str {
        match self {
            IdKind::Stream => "stream id (s-…)",
            IdKind::Thread => "thread id (b-…)",
            IdKind::WorkItem => "work-item id (wi-…)",
            IdKind::Note => "note id (n-…)",
            IdKind::AgentTurn => "agent-turn id (at-…)",
            IdKind::HookEvent => "hook-event id (he-…)",
            IdKind::Effort => "effort id (ef-…)",
            IdKind::Followup => "follow-up id (fu-…)",
            IdKind::PageVisit => "page-visit id (pv-…)",
            IdKind::UsageEvent => "usage-event id (ue-…)",
            IdKind::WorkItemLink => "work-item-link id (wil-…)",
            IdKind::BackgroundTask => "background-task id (bg-…)",
            IdKind::UnknownPrefix(_) => "id with an unrecognised prefix",
            IdKind::Unrecognised => "value with no `<prefix>-…` shape",
        }
    }
}

/// Infer the kind of an id from its prefix segment. Cheap, allocation-free.
pub fn classify_id(value: &str) -> IdKind {
    let Some((prefix, _rest)) = value.split_once('-') else {
        return IdKind::Unrecognised;
    };
    match prefix {
        "s" => IdKind::Stream,
        "b" => IdKind::Thread,
        "wi" => IdKind::WorkItem,
        "n" => IdKind::Note,
        "at" => IdKind::AgentTurn,
        "he" => IdKind::HookEvent,
        "ef" => IdKind::Effort,
        "fu" => IdKind::Followup,
        "pv" => IdKind::PageVisit,
        "ue" => IdKind::UsageEvent,
        "wil" => IdKind::WorkItemLink,
        "bg" => IdKind::BackgroundTask,
        // We deliberately leak the &str here only when we already
        // know it's a short prefix segment from the input. Since
        // `UnknownPrefix` is `&'static str`, we can't actually return
        // the borrowed slice — fall through to a generic marker.
        _ => IdKind::UnknownPrefix(""),
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

    #[test]
    fn classify_recognises_canonical_prefixes() {
        assert_eq!(classify_id("s-abc"), IdKind::Stream);
        assert_eq!(classify_id("b-abc"), IdKind::Thread);
        assert_eq!(classify_id("wi-abc"), IdKind::WorkItem);
        assert_eq!(classify_id("n-abc"), IdKind::Note);
        assert_eq!(classify_id("at-abc"), IdKind::AgentTurn);
        assert_eq!(classify_id("he-abc"), IdKind::HookEvent);
        assert_eq!(classify_id("ef-abc"), IdKind::Effort);
        assert_eq!(classify_id("fu-abc"), IdKind::Followup);
        assert_eq!(classify_id("pv-abc"), IdKind::PageVisit);
        assert_eq!(classify_id("ue-abc"), IdKind::UsageEvent);
        assert_eq!(classify_id("wil-abc"), IdKind::WorkItemLink);
        assert_eq!(classify_id("bg-abc"), IdKind::BackgroundTask);
    }

    #[test]
    fn classify_unknown_prefix() {
        assert!(matches!(classify_id("zzz-abc"), IdKind::UnknownPrefix(_)));
    }

    #[test]
    fn classify_unrecognised_when_no_dash() {
        assert_eq!(classify_id("plain"), IdKind::Unrecognised);
        assert_eq!(classify_id(""), IdKind::Unrecognised);
    }

    #[test]
    fn label_mentions_prefix_form() {
        assert!(IdKind::Stream.label().contains("s-"));
        assert!(IdKind::Thread.label().contains("b-"));
        assert!(IdKind::WorkItem.label().contains("wi-"));
    }
}
