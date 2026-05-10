//! Pure domain types and store traits for oxplow.
//!
//! This crate is the foundation of the workspace — it defines the
//! data shapes (streams, threads, work items, hook events) and the
//! abstract traits that infrastructure crates implement. It contains
//! no IO, no async runtime usage, and no platform-specific code.

pub mod error;
pub mod hook;
pub mod ids;
pub mod refs;
pub mod stores;
pub mod stream;
pub mod thread;
pub mod time;
pub mod work_item;

pub use error::DomainError;
pub use hook::{AgentStatus, AgentStatusState, AgentTurn, HookEvent, HookKind};
pub use ids::{
    classify_id, AgentTurnId, EffortId, HookEventId, IdKind, NoteId, StreamId, ThreadId, WorkItemId,
};
pub use stream::{Stream, StreamKind};
pub use thread::{Thread, ThreadStatus};
pub use time::Timestamp;
pub use work_item::{
    WorkItem, WorkItemActorKind, WorkItemAuthor, WorkItemEvent, WorkItemKind, WorkItemLink,
    WorkItemLinkType, WorkItemPriority, WorkItemStatus, WorkNote,
};
