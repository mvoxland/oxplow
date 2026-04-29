//! Cross-store event bus.
//!
//! Stores and services post `OxplowEvent` values onto a single
//! `tokio::sync::broadcast` channel. The Tauri layer subscribes once
//! and forwards each event to the renderer via `app_handle.emit`. The
//! MCP layer can subscribe independently if it ever needs to surface
//! state changes to the agent.
//!
//! Events are intentionally coarse: the renderer treats them as
//! "something in this bucket changed, refetch" rather than diffs.
//! The flat enum keeps the wire format simple and avoids a
//! per-bucket subscribe API.

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::broadcast;

use oxplow_domain::{StreamId, ThreadId, WorkItemId};

/// What changed. Variants are deliberately broad — the renderer
/// refetches the affected bucket on receipt rather than trying to
/// reconcile diffs from the payload.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OxplowEvent {
    /// Any stream row changed (created, renamed, deleted, panes
    /// updated). Renderer refetches `list_streams`.
    StreamsChanged,
    /// The current-stream pointer in `runtime_state` moved.
    CurrentStreamChanged { stream_id: Option<StreamId> },
    /// Threads on `stream_id` changed (created, status flipped, etc.).
    ThreadsChanged { stream_id: StreamId },
    /// Selected-thread pointer for `stream_id` moved.
    SelectedThreadChanged {
        stream_id: StreamId,
        thread_id: Option<ThreadId>,
    },
    /// Work items on `thread_id` (or backlog if `thread_id` is None).
    WorkItemsChanged { thread_id: Option<ThreadId> },
    /// A note was added or removed against an item or thread.
    WorkNotesChanged {
        item_id: Option<WorkItemId>,
        thread_id: Option<ThreadId>,
    },
    /// Wiki notes (creation, body update, deletion).
    WikiNotesChanged,
    /// Followups for a thread.
    FollowupsChanged { thread_id: ThreadId },
    /// Background task progress.
    BackgroundTasksChanged,
}

/// Cheap-to-clone broadcast hub. Capacity is small — subscribers
/// expected to keep up; lagging readers see `RecvError::Lagged` and
/// refetch.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<OxplowEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(256);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OxplowEvent> {
        self.sender.subscribe()
    }

    /// Post an event. Returns the number of active receivers (which
    /// may be 0 — that's not an error, the bus is fire-and-forget).
    pub fn emit(&self, event: OxplowEvent) {
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_receive_events() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.emit(OxplowEvent::StreamsChanged);
        let got = rx.recv().await.unwrap();
        assert!(matches!(got, OxplowEvent::StreamsChanged));
    }

    #[tokio::test]
    async fn emit_with_no_subscribers_is_noop() {
        let bus = EventBus::new();
        // Should not panic / error.
        bus.emit(OxplowEvent::WikiNotesChanged);
    }
}
