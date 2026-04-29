//! Store traits.
//!
//! Service crates depend on these traits, never on concrete impls.
//! `oxplow-db` implements them against rusqlite; tests can supply
//! in-memory fakes. The traits are async even though the SQLite impl
//! is sync — this matches Tauri's tokio-multi-thread runtime where DB
//! calls go through `spawn_blocking`. From the caller's POV the
//! await point is the same regardless of impl.

use async_trait::async_trait;

use crate::ids::{NoteId, StreamId, ThreadId, WorkItemId};
use crate::stream::Stream;
use crate::thread::Thread;
use crate::work_item::{WorkItem, WorkItemEvent, WorkItemLink, WorkItemLinkType, WorkNote};
use crate::DomainError;

#[async_trait]
pub trait StreamStore: Send + Sync {
    async fn list(&self) -> Result<Vec<Stream>, DomainError>;
    async fn get(&self, id: &StreamId) -> Result<Option<Stream>, DomainError>;
    async fn upsert(&self, stream: &Stream) -> Result<(), DomainError>;
    async fn delete(&self, id: &StreamId) -> Result<(), DomainError>;
    async fn primary(&self) -> Result<Option<Stream>, DomainError>;
    /// Returns the runtime-state pointer to the currently-selected
    /// stream id, if any. Survives restarts; null until set.
    async fn current_id(&self) -> Result<Option<StreamId>, DomainError>;
    /// Sets (or clears) the current-stream pointer.
    async fn set_current(&self, id: Option<&StreamId>) -> Result<(), DomainError>;
}

#[async_trait]
pub trait ThreadStore: Send + Sync {
    async fn list_for_stream(&self, stream: &StreamId) -> Result<Vec<Thread>, DomainError>;
    async fn get(&self, id: &ThreadId) -> Result<Option<Thread>, DomainError>;
    async fn upsert(&self, thread: &Thread) -> Result<(), DomainError>;
    async fn delete(&self, id: &ThreadId) -> Result<(), DomainError>;
    /// Per-stream selected-thread pointer. None means nothing selected.
    async fn selected_for_stream(
        &self,
        stream: &StreamId,
    ) -> Result<Option<ThreadId>, DomainError>;
    async fn set_selected_for_stream(
        &self,
        stream: &StreamId,
        thread: Option<&ThreadId>,
    ) -> Result<(), DomainError>;
}

#[async_trait]
pub trait WorkItemStore: Send + Sync {
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<WorkItem>, DomainError>;
    async fn list_backlog(&self) -> Result<Vec<WorkItem>, DomainError>;
    async fn get(&self, id: &WorkItemId) -> Result<Option<WorkItem>, DomainError>;
    async fn upsert(&self, item: &WorkItem) -> Result<(), DomainError>;
    async fn soft_delete(&self, id: &WorkItemId) -> Result<(), DomainError>;
}

#[async_trait]
pub trait WorkNoteStore: Send + Sync {
    async fn add_for_item(
        &self,
        item: &WorkItemId,
        body: &str,
        author: &str,
    ) -> Result<WorkNote, DomainError>;
    async fn add_for_thread(
        &self,
        thread: &ThreadId,
        body: &str,
        author: &str,
    ) -> Result<WorkNote, DomainError>;
    async fn list_for_item(&self, item: &WorkItemId) -> Result<Vec<WorkNote>, DomainError>;
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<WorkNote>, DomainError>;
    async fn delete(&self, id: &NoteId) -> Result<(), DomainError>;
}

#[async_trait]
pub trait WorkItemLinkStore: Send + Sync {
    async fn create(
        &self,
        thread: &ThreadId,
        from: &WorkItemId,
        to: &WorkItemId,
        link_type: WorkItemLinkType,
    ) -> Result<WorkItemLink, DomainError>;
    async fn list_outgoing(&self, item: &WorkItemId) -> Result<Vec<WorkItemLink>, DomainError>;
    async fn list_incoming(&self, item: &WorkItemId) -> Result<Vec<WorkItemLink>, DomainError>;
    async fn delete(&self, id: &str) -> Result<(), DomainError>;
}

#[async_trait]
pub trait WorkItemEventStore: Send + Sync {
    async fn append(&self, event: &WorkItemEvent) -> Result<(), DomainError>;
    async fn list_for_item(&self, item: &WorkItemId) -> Result<Vec<WorkItemEvent>, DomainError>;
    async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<WorkItemEvent>, DomainError>;
}
