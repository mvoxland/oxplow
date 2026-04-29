//! WorkItem domain types.
//!
//! Mirrors the TS `src/persistence/work-item-store.ts` types and the
//! schema introduced over migrations 1..N. The work item is the
//! durable unit of authored change in oxplow — every Edit/Write the
//! agent makes must trace back to one.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{ThreadId, WorkItemId};
use crate::time::Timestamp;

/// What kind of work item this is.
///
/// The hierarchy: `epic` parents `task`s parent `subtask`s. `bug` and
/// `note` are flat — they don't normally parent anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemKind {
    Epic,
    Task,
    Subtask,
    Bug,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus {
    Ready,
    InProgress,
    Blocked,
    Done,
    Canceled,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPriority {
    Low,
    Medium,
    High,
    Urgent,
}

/// Who or what wrote a work-item row to the DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemActorKind {
    User,
    Agent,
    System,
}

/// Semantic origin — distinct from `created_by` (the writer).
///
/// Narrowed to user/agent after auto-file was removed in v29+. Legacy
/// `agent-auto` rows get mapped to `None` on read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemAuthor {
    User,
    Agent,
}

/// The relationship type between two work items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemLinkType {
    Blocks,
    RelatesTo,
    DiscoveredFrom,
    Duplicates,
    Supersedes,
    RepliesTo,
}

/// A work item row.
///
/// Field shape mirrors the TS interface so JSON payloads are
/// indistinguishable across the migration. Nullable timestamps stay
/// `Option<Timestamp>` rather than collapsing to a sentinel — a missing
/// `completed_at` and a "completed at the epoch" must be distinguishable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WorkItem {
    pub id: WorkItemId,
    /// `None` when the item is on the project-wide backlog.
    pub thread_id: Option<ThreadId>,
    pub parent_id: Option<WorkItemId>,
    pub kind: WorkItemKind,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: Option<String>,
    pub status: WorkItemStatus,
    pub priority: WorkItemPriority,
    pub sort_index: i64,
    pub created_by: WorkItemActorKind,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub deleted_at: Option<Timestamp>,
    pub note_count: i64,
    /// Legacy rows have `None`; v29+ rows are always populated.
    pub author: Option<WorkItemAuthor>,
    /// Free-text grooming bucket used by the Backlog page's group-by.
    pub category: Option<String>,
    /// Comma-separated tags used by the Backlog page filter chips.
    pub tags: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WorkItemLink {
    pub id: String,
    pub thread_id: ThreadId,
    pub from_item_id: WorkItemId,
    pub to_item_id: WorkItemId,
    pub link_type: WorkItemLinkType,
    pub created_at: Timestamp,
}

/// External-input length caps, kept here so both the validation layer
/// and the DB layer reach for the same constants.
pub mod limits {
    pub const TITLE_MAX_LEN: usize = 500;
    pub const DESCRIPTION_MAX_LEN: usize = 20_000;
    pub const ACCEPTANCE_CRITERIA_MAX_LEN: usize = 20_000;
    pub const NOTE_MAX_LEN: usize = 20_000;
}

/// Sentinel scope used for the project-wide backlog (work items not
/// attached to any thread). Matches the TS `BACKLOG_SCOPE` constant.
pub const BACKLOG_SCOPE: &str = "__backlog__";

/// A note attached to either a work item or a thread (mutually
/// exclusive — enforced at the DB CHECK constraint).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WorkNote {
    pub id: crate::ids::NoteId,
    pub work_item_id: Option<WorkItemId>,
    pub thread_id: Option<ThreadId>,
    pub body: String,
    pub author: String,
    pub created_at: Timestamp,
}

/// Audit-log entry for state changes on a work item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WorkItemEvent {
    pub id: String,
    pub thread_id: ThreadId,
    pub item_id: Option<WorkItemId>,
    pub event_type: String,
    pub actor_kind: WorkItemActorKind,
    pub actor_id: String,
    pub payload_json: String,
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_round_trips_as_snake_case() {
        let ip = WorkItemStatus::InProgress;
        let json = serde_json::to_string(&ip).unwrap();
        assert_eq!(json, "\"in_progress\"");
        let back: WorkItemStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(ip, back);
    }

    #[test]
    fn link_type_uses_snake_case_in_json() {
        let lt = WorkItemLinkType::DiscoveredFrom;
        let json = serde_json::to_string(&lt).unwrap();
        assert_eq!(json, "\"discovered_from\"");
    }

    #[test]
    fn work_item_round_trips() {
        let now = Timestamp::from_unix_ms(1_700_000_000_000);
        let item = WorkItem {
            id: WorkItemId::from("wi-1"),
            thread_id: Some(ThreadId::from("b-1")),
            parent_id: None,
            kind: WorkItemKind::Task,
            title: "ship it".into(),
            description: String::new(),
            acceptance_criteria: None,
            status: WorkItemStatus::Ready,
            priority: WorkItemPriority::Medium,
            sort_index: 0,
            created_by: WorkItemActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(WorkItemAuthor::User),
            category: None,
            tags: None,
        };

        let json = serde_json::to_string(&item).unwrap();
        let back: WorkItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item, back);
    }

    #[test]
    fn backlog_item_has_no_thread() {
        let item: WorkItem = serde_json::from_str(
            r#"{
                "id":"wi-x","thread_id":null,"parent_id":null,"kind":"task",
                "title":"t","description":"","acceptance_criteria":null,
                "status":"ready","priority":"medium","sort_index":0,
                "created_by":"user",
                "created_at":"2026-04-29T12:00:00Z","updated_at":"2026-04-29T12:00:00Z",
                "completed_at":null,"deleted_at":null,"note_count":0,
                "author":"user","category":null,"tags":null
            }"#,
        )
        .unwrap();
        assert!(item.thread_id.is_none());
    }
}
