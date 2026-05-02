//! SQLite persistence layer for oxplow.
//!
//! Implements the store traits defined in `oxplow-domain` against a
//! `rusqlite` connection pool. Migrations live in `migrations/` as
//! plain SQL and are applied at startup via `refinery`.

pub mod agent_stores;
pub mod analytics_stores;
pub mod effort_store;
mod database;
mod stream_store;
mod thread_store;
pub mod wiki_note_store;
pub mod wiki_note_thread_updates;
mod work_item_store;
mod work_satellite;

pub use agent_stores::SqliteAgentTurnStore;
pub use effort_store::{
    EffortFile, EffortFileChange, SqliteWorkItemEffortStore, WorkItemEffort, WorkItemEffortStore,
};
pub use analytics_stores::{
    CodeQualityFinding, CodeQualityScan, CodeQualityScanStatus, FileSnapshot, PageVisit,
    PageVisitStore, SqliteCodeQualityStore, SqlitePageVisitStore, SqliteSnapshotStore,
    SqliteUsageStore, UsageEvent, UsageRollup,
};
pub use database::{Database, DbInitError};
pub use stream_store::SqliteStreamStore;
pub use thread_store::SqliteThreadStore;
pub use wiki_note_store::{SqliteWikiNoteStore, WikiNote, WikiNoteSearchHit, WikiNoteStore};
pub use wiki_note_thread_updates::{SqliteWikiNoteThreadUpdateStore, WikiNoteThreadUpdate};
pub use work_item_store::SqliteWorkItemStore;
pub use work_satellite::{SqliteWorkItemEventStore, SqliteWorkItemLinkStore, SqliteWorkNoteStore};
