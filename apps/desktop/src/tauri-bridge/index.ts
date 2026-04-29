// Frontend facade over the Tauri IPC.
//
// Re-exports the tauri-specta-generated bindings (every Rust command
// becomes a typed async TS function) plus a small ergonomics layer
// for events. UI code imports from here, never directly from
// @tauri-apps/api — that way swapping transports later (sidecar
// binary, remote daemon, etc.) doesn't ripple through the UI.

import { listen } from "@tauri-apps/api/event";
import { commands } from "./generated/bindings";

export { commands };
export * as oxplow from "./generated/bindings";

// Re-export every type the renderer typically reaches for. Kept in
// alphabetical order so additions stay merge-friendly.
export type {
  AgentStatus,
  AgentStatusState,
  AgentTurn,
  AgentTurnId,
  AppVersion,
  BacklogState,
  BlameLine,
  BranchChangeEntry,
  BranchChanges,
  ChangeKind,
  CreateWorkItemInput,
  CreateWorktreeRequest,
  GitOpResult,
  GitWorktreeEntry,
  GroupedGitRefs,
  HookEvent,
  HookEventId,
  HookKind,
  IpcError,
  RemoteBranchEntry,
  Stream,
  StreamId,
  StreamKind,
  TextSearchHit,
  Thread,
  ThreadId,
  ThreadStatus,
  Timestamp,
  WorkspaceContext,
  WorkspaceStatusSummary,
  UpdateWorkItemChanges,
  WorkItem,
  WorkItemActorKind,
  WorkItemAuthor,
  WorkItemId,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
} from "./generated/bindings";

/// Discriminant kinds for the cross-store event bus. Mirrors the
/// `OxplowEvent` enum on the Rust side.
export type OxplowEventKind =
  | "streamsChanged"
  | "currentStreamChanged"
  | "threadsChanged"
  | "selectedThreadChanged"
  | "workItemsChanged"
  | "workNotesChanged"
  | "wikiNotesChanged"
  | "followupsChanged"
  | "backgroundTasksChanged"
  | "hookEventsChanged"
  | "agentStatusChanged"
  | "agentTurnsChanged";

/// Subscribe to all oxplow events on the cross-store bus. Returns an
/// unlisten callback. Each event is the raw `OxplowEvent` payload —
/// the renderer normally branches on the `kind` field and refetches
/// the affected bucket via the matching `commands.*` call.
export function subscribeOxplowEvents(
  onEvent: (event: { kind: OxplowEventKind } & Record<string, unknown>) => void,
): () => Promise<void> {
  let cleanup: (() => void) | null = null;
  const promise = listen<{ kind: OxplowEventKind } & Record<string, unknown>>(
    "oxplow:event",
    (e) => {
      onEvent(e.payload);
    },
  ).then((un) => {
    cleanup = un;
  });
  return async () => {
    await promise;
    cleanup?.();
  };
}

/// Filtered helper: only fire `onEvent` for events matching `kinds`.
export function subscribeOxplowEventsOfKind(
  kinds: OxplowEventKind[],
  onEvent: (event: { kind: OxplowEventKind } & Record<string, unknown>) => void,
): () => Promise<void> {
  return subscribeOxplowEvents((event) => {
    if (kinds.includes(event.kind)) onEvent(event);
  });
}
