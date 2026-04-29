//! Pure domain types and store traits for oxplow.
//!
//! This crate is the foundation of the workspace — it defines the
//! data shapes (streams, threads, work items, hook events) and the
//! abstract traits that infrastructure crates implement. It contains
//! no IO, no async runtime usage, and no platform-specific code.

pub mod error;
pub mod ids;
pub mod stores;
pub mod stream;
pub mod thread;
pub mod time;
pub mod work_item;

pub use error::DomainError;
pub use ids::{AgentTurnId, NoteId, StreamId, ThreadId, WorkItemId};
pub use stream::{Stream, StreamKind};
pub use thread::{Thread, ThreadStatus};
pub use time::Timestamp;
pub use work_item::{
    WorkItem, WorkItemActorKind, WorkItemAuthor, WorkItemEvent, WorkItemKind, WorkItemLink,
    WorkItemLinkType, WorkItemPriority, WorkItemStatus, WorkNote,
};
