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

use oxplow_domain::{AgentStatusState, StreamId, ThreadId, WorkItemId};

/// fs-watch classification mirrored onto the wire so the renderer can
/// distinguish create / modify / delete / rename without re-stating
/// every variant of the upstream `notify` crate.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceChangeKind {
    Created,
    Updated,
    Deleted,
    Renamed,
}

/// Snapshot trigger source. The renderer renders these differently in
/// the Snapshots panel ("startup" rows are dimmer than "task-end").
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotSourceKind {
    TaskStart,
    TaskEnd,
    TaskEvent,
    Startup,
    Manual,
}

/// Code-quality scan lifecycle phase the bus broadcasts. Mirrors the
/// renderer-era enum.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum CodeQualityScanPhase {
    Started,
    Completed,
    Failed,
}

/// What changed. Variants are deliberately broad — the renderer
/// refetches the affected bucket on receipt rather than trying to
/// reconcile diffs from the payload.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
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
    /// Wiki pages (creation, body update, deletion).
    WikiPagesChanged,
    /// Followups for a thread.
    FollowupsChanged { thread_id: ThreadId },
    /// Background task progress.
    BackgroundTasksChanged,
    /// A new hook event landed; renderer refreshes the hook log.
    HookEventsChanged,
    /// Per-thread per-pane agent status changed. `state` carries the
    /// derived status so the renderer can update without a refetch
    /// round-trip — sources that don't have it pre-derived (e.g.
    /// PreToolUse/PostToolUse, where the renderer used to refetch and
    /// re-derive) compute it inline before emitting.
    AgentStatusChanged {
        thread_id: ThreadId,
        pane_target: String,
        state: AgentStatusState,
    },
    /// agent_turn opened or closed.
    AgentTurnsChanged { thread_id: ThreadId },
    /// A page visit was recorded (rail history, recently-finished, etc.).
    /// Coarse — renderer refetches whatever view it cares about.
    PageVisitChanged,
    /// A usage event was recorded. The renderer's filtering uses
    /// `usage_kind` to scope refetches (wiki-note vs editor-file vs
    /// work-item, etc.).
    UsageRecorded {
        usage_kind: String,
        key: String,
        stream_id: Option<StreamId>,
        thread_id: Option<ThreadId>,
    },
    /// A file snapshot landed in the snapshot store. Driven by the
    /// background snapshot capture loop or an explicit task event.
    FileSnapshotCreated {
        stream_id: Option<StreamId>,
        snapshot_id: i64,
        source: SnapshotSourceKind,
        effort_id: Option<String>,
        thread_id: Option<ThreadId>,
    },
    /// A code-quality scan transitioned states (started / completed /
    /// failed). The renderer refreshes scan + finding lists on receipt.
    CodeQualityScanned {
        stream_id: Option<StreamId>,
        scan_id: i64,
        tool: String,
        scope: String,
        phase: CodeQualityScanPhase,
    },
    /// `.git` directory appeared/disappeared at the project root —
    /// "is this a git workspace" flipped. Renderer hides/restores the
    /// git-aware UI on receipt.
    WorkspaceContextChanged { git_enabled: bool },
    /// A worktree file changed on disk. Renderer-wide: file tree, quick
    /// open, project panel, git dashboard, uncommitted changes view all
    /// refresh in response.
    WorkspaceChanged {
        stream_id: StreamId,
        change_kind: WorkspaceChangeKind,
        path: String,
    },
    /// A ref under `.git/refs/` changed. Drives history, branch list,
    /// and ahead/behind refreshes. Coarse per stream.
    GitRefsChanged { stream_id: StreamId },
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
        bus.emit(OxplowEvent::WikiPagesChanged);
    }
}
