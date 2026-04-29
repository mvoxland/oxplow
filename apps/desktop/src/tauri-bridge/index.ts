// Frontend facade over the Tauri IPC.
//
// Re-exports the tauri-specta-generated bindings (every Rust command
// becomes a typed async TS function) plus a small ergonomics layer
// for events. UI code imports from here, never directly from
// @tauri-apps/api — that way swapping transports later (sidecar
// binary, remote daemon, etc.) doesn't ripple through the UI.

export * as oxplow from "./generated/bindings";
// Re-export types that are reachable from the registered commands.
// Adding a new command surfaces additional types automatically.
export type {
  Stream,
  StreamKind,
  Thread,
  ThreadStatus,
  WorkItem,
  WorkItemKind,
  WorkItemStatus,
  WorkItemPriority,
  WorkItemActorKind,
  WorkItemAuthor,
  StreamId,
  ThreadId,
  WorkItemId,
  CreateWorktreeRequest,
  AppVersion,
  IpcError,
} from "./generated/bindings";
