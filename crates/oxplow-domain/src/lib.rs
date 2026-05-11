//! Pure domain types and store traits for oxplow.
//!
//! This crate is the foundation of the workspace — it defines the
//! data shapes (streams, threads, tasks, hook events) and the
//! abstract traits that infrastructure crates implement. It contains
//! no IO, no async runtime usage, and no platform-specific code.

pub mod error;
pub mod hook;
pub mod ids;
pub mod refs;
pub mod stores;
pub mod stream;
pub mod task;
pub mod thread;
pub mod time;

pub use error::DomainError;
pub use hook::{AgentStatus, AgentStatusState, AgentTurn, HookEvent, HookKind};
pub use ids::{AgentTurnId, EffortId, HookEventId, NoteId, StreamId, TaskId, TaskLinkId, ThreadId};
pub use stream::{Stream, StreamKind};
pub use task::{
    Task, TaskActorKind, TaskAuthor, TaskEvent, TaskLink, TaskLinkType, TaskPriority, TaskStatus,
    WorkNote,
};
pub use thread::{Thread, ThreadStatus};
pub use time::Timestamp;
