import { commands } from "./tauri-bridge/generated/bindings.js";
import { listen } from "@tauri-apps/api/event";
import type { DesktopApi, OxplowEvent } from "./legacy-ipc-contract.js";

// -- Legacy adapter helpers (formerly in legacy-bridge.ts). Inlined
// here so the legacy compatibility layer lives in a single file.

function unwrap<T>(result: { status: "ok"; data: T } | { status: "error"; error: unknown }): T {
  if (result.status === "ok") return result.data;
  const err = result.error as { message?: string; code?: string } | undefined;
  throw new Error(err?.message ?? err?.code ?? "ipc error");
}

function notPorted(name: string): never {
  throw new Error(`oxplow legacy API method "${name}" is not yet ported to Tauri`);
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function adaptStream(s: any): any {
  if (!s) return s;
  return {
    ...s,
    custom_prompt: s.custom_prompt ?? null,
    panes: { working: s.working_pane ?? "", talking: s.talking_pane ?? "" },
    resume: {
      working_session_id: s.working_session_id ?? "",
      talking_session_id: s.talking_session_id ?? "",
    },
  };
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function adaptThread(t: any): any {
  if (!t) return t;
  return {
    ...t,
    status: t.status === "closed" ? "queued" : t.status,
    closed_at: t.closed_at ?? null,
  };
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function adaptBackgroundTask(t: any): any {
  if (!t) return t;
  let result: unknown = undefined;
  if (typeof t.result_json === "string" && t.result_json.length > 0) {
    try {
      result = JSON.parse(t.result_json);
    } catch {
      // ignore
    }
  }
  return {
    ...t,
    startedAt: t.started_at ? Date.parse(t.started_at) : Date.now(),
    endedAt: t.ended_at ? Date.parse(t.ended_at) : null,
    result,
  };
}

function buildLegacyAdapter(): DesktopApi {
  const adapter: Partial<DesktopApi> = {
    ping: async () => unwrap(await commands.ping()),
    listStreams: async () =>
      (unwrap(await commands.listStreams()) as unknown[]).map(adaptStream),
    getCurrentStream: async () => {
      const cur = unwrap(await commands.getCurrentStream());
      if (cur) return adaptStream(cur);
      const primary = unwrap(await commands.getPrimaryStream());
      if (!primary) throw new Error("no primary stream available");
      return adaptStream(primary);
    },
    switchStream: async (id: string) => {
      unwrap(await commands.switchStream(id));
      return adaptStream(unwrap(await commands.getCurrentStream()));
    },
    renameCurrentStream: async (title: string) => {
      const cur = unwrap(await commands.getCurrentStream());
      if (!cur) throw new Error("no current stream to rename");
      return adaptStream(unwrap(await commands.renameStream({ id: cur.id, title })));
    },
    renameStream: async (id: string, title: string) =>
      adaptStream(unwrap(await commands.renameStream({ id, title }))),
    setStreamPrompt: async (id: string, prompt: string | null) =>
      unwrap(await commands.setStreamPrompt({ id, prompt })),
    checkoutStreamBranch: async (id: string, branch: string) =>
      unwrap(await commands.checkoutStreamBranch(id, branch)),
    reorderStreams: async (order: string[]) => unwrap(await commands.reorderStreams(order)),
    reorderThreads: async (streamId: string, order: string[]) =>
      unwrap(await commands.reorderThreadQueue({ streamId, order })),
    reorderThread: async (streamId: string, order: string[]) =>
      unwrap(await commands.reorderThreadQueue({ streamId, order })),
    createStream: async () => {
      throw new Error("createStream is replaced by createWorktree under Tauri");
    },
    closeThread: async (id: string) => adaptThread(unwrap(await commands.closeThread(id))),
    reopenThread: async (id: string) => adaptThread(unwrap(await commands.reopenThread(id))),
    promoteThread: async (id: string) => adaptThread(unwrap(await commands.promoteThread(id))),
    renameThread: async (id: string, title: string) =>
      adaptThread(unwrap(await commands.renameThread({ id, title }))),
    setThreadPrompt: async (id: string, prompt: string | null) =>
      adaptThread(unwrap(await commands.setThreadPrompt({ id, prompt }))),
    listClosedThreads: async (streamId: string) =>
      (unwrap(await commands.listClosedThreads(streamId)) as unknown[]).map(adaptThread),
    selectThread: async (streamId: string, threadId: string | null) =>
      unwrap(await commands.selectThread({ streamId, threadId })),
    createThread: async (streamId: string, title: string, paneTarget?: string) =>
      adaptThread(
        unwrap(await commands.createThread({ streamId, title, paneTarget: paneTarget ?? null })),
      ),
    reorderThreadQueue: async (streamId: string, order: string[]) =>
      unwrap(await commands.reorderThreadQueue({ streamId, order })),
    getThreadState: async (streamId: string) => {
      const raw = unwrap(await commands.getThreadState(streamId)) as {
        threads: unknown[];
        [k: string]: unknown;
      };
      return { ...raw, threads: raw.threads.map(adaptThread) };
    },
    getThreadWorkState: async (_streamId: string, threadId: string) =>
      unwrap(await commands.getThreadWorkState(threadId)),
    createWorkItem: async (
      _streamId: string,
      threadId: string,
      input: Record<string, unknown>,
    ) =>
      unwrap(await commands.createWorkItem({ threadId, input: input as never })),
    updateWorkItem: async (
      _streamId: string,
      _threadId: string,
      itemId: string,
      changes: Record<string, unknown>,
    ) => unwrap(await commands.updateWorkItem({ id: itemId, changes: changes as never })),
    deleteWorkItem: async (_streamId: string, _threadId: string, itemId: string) =>
      unwrap(await commands.deleteWorkItem(itemId)),
    moveWorkItemToBacklog: async (_streamId: string, _fromThreadId: string, itemId: string) =>
      unwrap(await commands.moveWorkItem({ id: itemId, threadId: null })),
    moveWorkItemToThread: async (
      _streamId: string,
      _fromThreadId: string,
      itemId: string,
      toThreadId: string,
    ) => unwrap(await commands.moveWorkItem({ id: itemId, threadId: toThreadId })),
    moveBacklogItemToThread: async (_streamId: string, itemId: string, toThreadId: string) =>
      unwrap(await commands.moveWorkItem({ id: itemId, threadId: toThreadId })),
    reorderWorkItems: async (
      _streamId: string,
      threadId: string,
      orderedItemIds: string[],
    ) => unwrap(await commands.reorderWorkItems({ threadId, order: orderedItemIds })),
    reorderBacklog: async (orderedItemIds: string[]) =>
      unwrap(await commands.reorderWorkItems({ threadId: null, order: orderedItemIds })),
    getBacklogState: async () => unwrap(await commands.getBacklogState()),
    createBacklogItem: async (input: Record<string, unknown>) =>
      unwrap(await commands.createWorkItem({ threadId: null, input: input as never })),
    updateBacklogItem: async (itemId: string, changes: Record<string, unknown>) =>
      unwrap(await commands.updateWorkItem({ id: itemId, changes: changes as never })),
    deleteBacklogItem: async (itemId: string) =>
      unwrap(await commands.deleteWorkItem(itemId)),
    getWorkItemSummaries: async (threadId?: string | null) =>
      unwrap(await commands.getWorkItemSummaries(threadId ?? null)),
    addWorkItemNote: async (
      _streamId: string,
      _threadId: string,
      itemId: string,
      body: string,
      author?: string,
    ) => unwrap(await commands.addWorkNote(itemId, body, author ?? "user")),
    getWorkNotes: async (itemId: string) => unwrap(await commands.listWorkNotes(itemId)),
    listWorkItemEvents: async (
      _streamId: string,
      _threadId: string,
      itemId?: string,
    ) => unwrap(await commands.listWorkItemEvents(itemId ?? null, null)),
    listWorkItemEfforts: async (itemId: string) =>
      unwrap(await commands.listWorkItemEfforts(itemId)),
    getEffortFiles: async (effortId: string) =>
      unwrap(await commands.getEffortFiles(effortId)),
    listEffortsEndingAtSnapshots: async (snapshotIds: number[]) =>
      unwrap(await commands.listEffortsEndingAtSnapshots(snapshotIds)),
    getRepoConflictState: async () => unwrap(await commands.getRepoConflictState()),
    getAheadBehind: async (base: string, head: string) =>
      unwrap(await commands.getAheadBehind(base, head)),
    getCommitsAheadOf: async (base: string, head: string, limit?: number) =>
      unwrap(await commands.getCommitsAheadOf(base, head, limit ?? 200)),
    getDefaultBranch: async () => unwrap(await commands.getDefaultBranch()),
    listBranches: async () => unwrap(await commands.listLocalBranches()),
    renameGitBranch: async (from: string, to: string) =>
      unwrap(await commands.renameBranch(from, to)),
    deleteGitBranch: async (branch: string, force?: boolean) =>
      unwrap(await commands.deleteBranch(branch, force ?? false)),
    gitAppendToGitignore: async (entry: string) =>
      unwrap(await commands.appendToGitignore(entry)),
    gitRestorePath: async (path: string) => unwrap(await commands.restorePath(path)),
    gitFetch: async (remote?: string | null) =>
      unwrap(await commands.gitFetch(remote ?? null)),
    gitPull: async () => unwrap(await commands.gitPull()),
    gitPullRemoteIntoCurrent: async (remote: string, branch: string) =>
      unwrap(await commands.gitPullRemoteIntoCurrent(remote, branch)),
    gitPush: async () => unwrap(await commands.gitPush()),
    gitPushCurrentTo: async (remote: string, branch: string) =>
      unwrap(await commands.gitPushCurrentTo(remote, branch)),
    gitMergeInto: async (source: string) => unwrap(await commands.gitMergeInto(source)),
    gitRebaseOnto: async (onto: string) => unwrap(await commands.gitRebaseOnto(onto)),
    gitCommitAll: async (message: string) => unwrap(await commands.gitCommitAll(message)),
    gitAddPath: async (path: string) => unwrap(await commands.gitAddPath(path)),
    gitBlame: async (path: string) => unwrap(await commands.gitBlame(path)),
    localBlame: async (path: string, diskText: string) =>
      unwrap(await commands.localBlame(path, diskText)),
    listAllRefs: async () => unwrap(await commands.listAllRefs()),
    listGitRefs: async () => {
      const raw = unwrap(await commands.listAllRefs());
      const localBranches = raw.locals.map((r) => ({
        kind: "local" as const,
        name: r.label,
        ref: r.ref,
      }));
      const byRemote = new Map<string, Array<{ kind: "remote"; name: string; ref: string; remote: string }>>();
      for (const r of raw.remotes) {
        const slash = r.label.indexOf("/");
        const remote = slash >= 0 ? r.label.slice(0, slash) : "origin";
        const name = slash >= 0 ? r.label.slice(slash + 1) : r.label;
        if (!byRemote.has(remote)) byRemote.set(remote, []);
        byRemote.get(remote)!.push({ kind: "remote", name, ref: r.ref, remote });
      }
      return {
        local: localBranches,
        remote: Array.from(byRemote.values()).flat(),
        remotes: Array.from(byRemote.entries()).map(([remote, branches]) => ({
          remote,
          branches,
        })),
        tags: raw.tags.map((t) => ({ name: t.label, ref: t.ref })),
        recent: localBranches.slice(0, 5).map((b) => b.name),
      };
    },
    listFileCommits: async (path: string, limit?: number) =>
      unwrap(await commands.listFileCommits(path, limit ?? null)),
    listRecentRemoteBranches: async (limit?: number) =>
      unwrap(await commands.listRecentRemoteBranches(limit ?? null)),
    readFileAtRef: async (ref: string, path: string) =>
      unwrap(await commands.readFileAtRef(ref, path)),
    searchWorkspaceText: async (query: string, limit?: number) =>
      unwrap(await commands.searchWorkspaceText(query, limit ?? null)),
    getBranchChanges: async (baseRef: string) =>
      unwrap(await commands.getBranchChanges(baseRef)),
    getChangeScopes: async () => {
      const raw = unwrap(await commands.getChangeScopes());
      return {
        staged: [] as never[],
        unstaged: [] as never[],
        currentBranch: raw.current_branch ?? undefined,
        branchBase: raw.branch_base ?? undefined,
        upstream: raw.upstream ?? undefined,
        onDefaultBranch: raw.on_default_branch,
      };
    },
    listAdoptableWorktrees: async () => unwrap(await commands.listAdoptableWorktrees()),
    listSiblingWorktrees: async () => unwrap(await commands.listSiblingWorktrees()),
    getCommitDetail: async (sha: string) => unwrap(await commands.getCommitDetail(sha)),
    getGitLog: async (options?: { limit?: number; all?: boolean }) =>
      unwrap(await commands.getGitLog(options?.limit ?? null, options?.all ?? false)),
    listWorkspaceEntries: async (dir: string) =>
      unwrap(await commands.listWorkspaceEntries(dir)),
    listWorkspaceFiles: async () => unwrap(await commands.listWorkspaceFiles()),
    readWorkspaceFile: async (path: string) => unwrap(await commands.readWorkspaceFile(path)),
    writeWorkspaceFile: async (path: string, content: string) =>
      unwrap(await commands.writeWorkspaceFile(path, content)),
    createWorkspaceFile: async (path: string, content: string) =>
      unwrap(await commands.createWorkspaceFile(path, content)),
    createWorkspaceDirectory: async (path: string) =>
      unwrap(await commands.createWorkspaceDirectory(path)),
    renameWorkspacePath: async (from: string, to: string) =>
      unwrap(await commands.renameWorkspacePath(from, to)),
    deleteWorkspacePath: async (path: string) =>
      unwrap(await commands.deleteWorkspacePath(path)),
    getWorkspaceContext: async () => unwrap(await commands.getWorkspaceContext()),
    recordPageVisit: async (input: { pageKind: string; pageId: string; durationMs?: number | null }) =>
      unwrap(await commands.recordPageVisit(input.pageKind, input.pageId, input.durationMs ?? null)),
    listRecentPageVisits: async (opts?: { limit?: number }) =>
      unwrap(await commands.listRecentPageVisits(opts?.limit ?? 50)),
    topVisitedPages: async (opts?: { limit?: number }) =>
      unwrap(await commands.topVisitedPages(opts?.limit ?? 50)),
    forgetPage: async (kind: string, id: string) =>
      unwrap(await commands.forgetPage(kind, id)),
    countPageVisitsByDay: async (opts?: { days?: number }) =>
      unwrap(await commands.countPageVisitsByDay(opts?.days ?? 30)),
    recordUsage: async (input: { kind: string; payload?: unknown }) =>
      unwrap(await commands.recordUsage(input.kind, JSON.stringify(input.payload ?? {}))),
    listRecentUsage: async (limit?: number) =>
      unwrap(await commands.listRecentUsage(limit ?? 50)),
    listFrequentUsage: async (limit?: number) =>
      unwrap(await commands.listFrequentUsage(limit ?? 50)),
    listCurrentlyOpenUsage: async (limit?: number) =>
      unwrap(await commands.listCurrentlyOpenUsage(limit ?? 50)),
    listRecentlyFinished: async (limit?: number) =>
      unwrap(await commands.listRecentlyFinished(limit ?? 50)),
    clearRecentlyFinished: async () => unwrap(await commands.clearRecentlyFinished()),
    listCodeQualityScans: async (limit?: number) =>
      unwrap(await commands.listCodeQualityScans(limit ?? 50)),
    listCodeQualityFindings: async (scanId: number) =>
      unwrap(await commands.listCodeQualityFindings(scanId)),
    listSnapshots: async (path?: string) =>
      unwrap(await commands.listSnapshots(path ?? "")),
    getSnapshotPairDiff: async (beforeId?: number, afterId?: number) =>
      unwrap(await commands.getSnapshotPairDiff(beforeId ?? null, afterId ?? null)),
    getSnapshotSummary: async (streamId?: string, limit?: number) =>
      unwrap(await commands.getSnapshotSummary(streamId ?? null, limit ?? null)),
    restoreFileFromSnapshot: async (snapshotId: number) =>
      unwrap(await commands.restoreFileFromSnapshot(snapshotId)),
    listWikiNotes: async () => unwrap(await commands.listWikiNotes()),
    deleteWikiNote: async (slug: string) => unwrap(await commands.deleteWikiNote(slug)),
    searchWikiNotes: async (query: string, limit?: number) =>
      unwrap(await commands.searchWikiTitles(query, limit ?? 50)),
    readWikiNoteBody: async (slug: string) => unwrap(await commands.readWikiNoteBody(slug)),
    writeWikiNoteBody: async (slug: string, body: string) =>
      unwrap(await commands.writeWikiNoteBody(slug, body)),
    listBackgroundTasks: async () =>
      (unwrap(await commands.listBackgroundTasks()) as unknown[]).map(adaptBackgroundTask),
    getBackgroundTask: async (id: string) =>
      adaptBackgroundTask(unwrap(await commands.getBackgroundTask(id))),
    listHookEvents: async (_streamId?: string) =>
      unwrap(await commands.listHookEvents(null, null)),
    listAgentStatuses: async () => unwrap(await commands.listAgentStatuses()),
    getConfig: async () => unwrap(await commands.getConfig()),
    setAgentPromptAppend: async (text: string) =>
      unwrap(await commands.setAgentPromptAppend(text)),
    setSnapshotRetentionDays: async (days: number) =>
      unwrap(await commands.setSnapshotRetentionDays(days)),
    setSnapshotMaxFileBytes: async (bytes: number) =>
      unwrap(await commands.setSnapshotMaxFileBytes(bytes)),
    setGeneratedDirs: async (dirs: string[]) =>
      unwrap(await commands.setGeneratedDirs(dirs)),
    removeFollowup: async (id: string) => unwrap(await commands.removeFollowup(id)),
    openExternalUrl: async (url: string) => {
      try {
        unwrap(await commands.openExternalUrl(url));
        return { ok: true };
      } catch (e) {
        return { ok: false, reason: e instanceof Error ? e.message : String(e) };
      }
    },
    setNativeMenu: async () => {},
    onMenuCommand: () => () => {},
    updateEditorFocus: async () => {},
    logUi: async (payload: unknown) => {
      // eslint-disable-next-line no-console
      console.log("[ui]", payload);
    },
    runCodeQualityScan: async (
      tool: string,
      scope?: string,
      files?: string[],
    ) =>
      unwrap(await commands.runCodeQualityScan(tool, scope ?? "workspace", files ?? null)),
    openLspClient: async () => {
      throw new Error("openLspClient: LSP session manager not yet ported");
    },
    closeLspClient: async () => {},
    sendLspMessage: async () => {
      throw new Error("sendLspMessage: LSP session manager not yet ported");
    },
    onLspEvent: () => () => {},
    openTerminalSession: async () => {
      throw new Error("openTerminalSession: PTY bridge not yet wired through Tauri events");
    },
    closeTerminalSession: async () => {},
    sendTerminalMessage: async () => {
      throw new Error("sendTerminalMessage: PTY bridge not yet wired through Tauri events");
    },
    onTerminalEvent: () => () => {},
    onOxplowEvent: (handler: (event: unknown) => void) => {
      let stopped = false;
      const unlistenPromise = listen("oxplow:event", (e) => {
        if (stopped) return;
        handler(e.payload);
      });
      return () => {
        stopped = true;
        unlistenPromise.then((u) => u());
      };
    },
  };

  return new Proxy(adapter, {
    get(target, prop, receiver) {
      const v = Reflect.get(target, prop, receiver);
      if (v !== undefined) return v;
      if (typeof prop === "string") {
        return (..._args: unknown[]) => notPorted(prop);
      }
      return undefined;
    },
  }) as DesktopApi;
}

export type { OxplowEvent } from "./legacy-ipc-contract.js";
export type { GitLogResult, GitLogCommit, GitLogRef, CommitDetail, ChangeScopes, TextSearchHit, GitOpResult, RefOption, BlameLine, GroupedGitRefs, GitWorktreeEntry, RemoteBranchEntry } from "./legacy-ipc-contract.js";

// Stream / Thread types re-exported from the Tauri bindings. The
// legacy adapter (`legacy-bridge.ts`) augments runtime values with
// nested `panes` / `resume` sub-objects so any UI code that reads
// those keeps working even though the type itself doesn't model them
// explicitly. New code should reach for the flat fields
// (`working_pane` / `talking_pane` / `*_session_id`) directly.
import type { Stream, Thread } from "./tauri-bridge/index.js";
export type { Stream, Thread };

export interface ThreadState {
  selectedThreadId: string | null;
  activeThreadId: string | null;
  threads: Thread[];
}

// Work-item types now come from the Tauri bindings. The bindings
// emit a `deleted_at` field that the legacy interface didn't model;
// readers either ignore it or filter on it (legacy stores already
// excluded soft-deleted rows in their list queries). New code can
// read `deleted_at` directly when needed.
import type {
  WorkItem,
  WorkItemKind,
  WorkItemStatus,
  WorkItemPriority,
} from "./tauri-bridge/index.js";
export type { WorkItem, WorkItemKind, WorkItemStatus, WorkItemPriority };

export interface WorkNote {
  id: string;
  work_item_id: string;
  body: string;
  author: string;
  created_at: string;
}

import type { WorkItemEvent } from "./tauri-bridge/index.js";
export type { WorkItemEvent };

export type SnapshotSource =
  | "task-start"
  | "task-end"
  | "task-event"
  | "startup";

export interface FileSnapshot {
  id: string;
  stream_id: string;
  worktree_path: string;
  version_hash: string;
  source: SnapshotSource;
  created_at: string;
  label?: string | null;
  label_kind?: "task" | "turn" | "system" | null;
}

export type SnapshotEntryState = "present" | "oversize";

export interface SnapshotEntry {
  hash: string;
  mtime_ms: number;
  size: number;
  state: SnapshotEntryState;
}

export interface SnapshotFileRow {
  entry: SnapshotEntry;
  kind: "created" | "updated" | "deleted";
}

export interface SnapshotSummary {
  snapshot: FileSnapshot;
  previousSnapshotId: string | null;
  files: Record<string, SnapshotFileRow>;
  counts: { created: number; updated: number; deleted: number };
}

export type SnapshotDiffSide = "absent" | SnapshotEntryState;

export interface SnapshotDiffResult {
  before: string | null;
  after: string | null;
  beforeState: SnapshotDiffSide;
  afterState: SnapshotDiffSide;
}

export interface WorkItemEffort {
  id: string;
  work_item_id: string;
  started_at: string;
  ended_at: string | null;
  start_snapshot_id: string | null;
  end_snapshot_id: string | null;
  summary: string | null;
}

export interface EffortDetail {
  effort: WorkItemEffort;
  start_snapshot: FileSnapshot | null;
  end_snapshot: FileSnapshot | null;
  changed_paths: string[];
  counts: { created: number; updated: number; deleted: number };
}

// Followup is bindings.Followup; ThreadWorkState is the bundle the
// Work panel renders. Both are emitted by tauri-specta now.
import type { Followup as ThreadFollowup, ThreadWorkState as TauriThreadWorkState, BacklogState as TauriBacklogState } from "./tauri-bridge/index.js";
export type { ThreadFollowup };
export type ThreadWorkState = TauriThreadWorkState;
export type BacklogState = TauriBacklogState;

export const BACKLOG_SCOPE = "__backlog__";

export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
}

export type GitFileStatus = "modified" | "added" | "deleted" | "renamed" | "untracked";

export interface BranchChangeEntry {
  path: string;
  status: GitFileStatus;
  additions: number | null;
  deletions: number | null;
}

export interface BranchChanges {
  baseRef: string;
  mergeBase: string | null;
  files: BranchChangeEntry[];
}

export interface WorkspaceEntry {
  name: string;
  path: string;
  kind: "file" | "directory";
  gitStatus: GitFileStatus | null;
  hasChanges: boolean;
}

export interface WorkspaceFile {
  path: string;
  content: string;
}

export interface WorkspacePathChange {
  path: string;
}

export interface WorkspaceRenameResult {
  fromPath: string;
  toPath: string;
}

export interface WorkspaceIndexedFile {
  path: string;
  gitStatus: GitFileStatus | null;
}

import type { WorkspaceStatusSummary } from "./tauri-bridge/index.js";
export type { WorkspaceStatusSummary };

export interface WorkspaceContext {
  gitEnabled: boolean;
}

export interface WorkspaceWatchEvent {
  id: number;
  streamId: string;
  path: string;
  kind: "created" | "updated" | "deleted";
  t: number;
}

export async function getCurrentStream(): Promise<Stream> {
  return desktopApi().getCurrentStream();
}

export async function listStreams(): Promise<Stream[]> {
  return desktopApi().listStreams();
}

export async function switchStream(id: string): Promise<Stream> {
  return desktopApi().switchStream(id);
}

export async function renameCurrentStream(title: string): Promise<Stream> {
  return desktopApi().renameCurrentStream(title);
}

export async function renameStream(streamId: string, title: string): Promise<Stream> {
  return desktopApi().renameStream(streamId, title);
}

export async function getConfig(): Promise<import("./legacy-ipc-contract.js").OxplowConfig> {
  return desktopApi().getConfig();
}

export async function setAgentPromptAppend(text: string): Promise<import("./legacy-ipc-contract.js").OxplowConfig> {
  return desktopApi().setAgentPromptAppend(text);
}

export async function setGeneratedDirs(dirs: string[]): Promise<import("./legacy-ipc-contract.js").OxplowConfig> {
  return desktopApi().setGeneratedDirs(dirs);
}

export async function setSnapshotRetentionDays(days: number): Promise<import("./legacy-ipc-contract.js").OxplowConfig> {
  return desktopApi().setSnapshotRetentionDays(days);
}

export async function setSnapshotMaxFileBytes(bytes: number): Promise<import("./legacy-ipc-contract.js").OxplowConfig> {
  return desktopApi().setSnapshotMaxFileBytes(bytes);
}

export async function listBranches(): Promise<BranchRef[]> {
  return desktopApi().listBranches();
}

export async function getDefaultBranch(): Promise<string | null> {
  return desktopApi().getDefaultBranch();
}

export async function listGitRefs(): Promise<import("./legacy-ipc-contract.js").GroupedGitRefs> {
  return desktopApi().listGitRefs();
}

export async function renameGitBranch(from: string, to: string): Promise<import("./legacy-ipc-contract.js").GitOpResult> {
  return desktopApi().renameGitBranch(from, to);
}

export async function deleteGitBranch(branch: string, options?: { force?: boolean }): Promise<import("./legacy-ipc-contract.js").GitOpResult> {
  return desktopApi().deleteGitBranch(branch, options);
}

/**
 * Long-running git ops are kickoff-style — the IPC promise resolves
 * immediately with a `taskId` once the BackgroundTaskStore row is
 * registered, and the actual work runs in the background. Each
 * renderer-side wrapper also exposes an `awaitDone` promise that
 * resolves with the final `BackgroundTask` (status, error, and
 * `result` payload — typically a `GitOpResult`). Pattern:
 *
 *     const { taskId, awaitDone } = await gitRebaseOnto(...);
 *     // mark UI pending using taskId / a label
 *     const task = await awaitDone;
 *     // task.result is the GitOpResult
 *
 * Callers that don't need the final result can ignore `awaitDone`;
 * any other surface watching `subscribeBackgroundTaskEvents` still
 * sees the same in-flight state.
 */
export interface GitOpKickoff {
  taskId: string;
  awaitDone: Promise<BackgroundTask | null>;
}

function attachAwait(taskId: string): GitOpKickoff {
  return { taskId, awaitDone: awaitBackgroundTask(taskId) };
}

export async function gitMergeInto(streamId: string, other: string): Promise<GitOpKickoff> {
  const { taskId } = await desktopApi().gitMergeInto(streamId, other);
  return attachAwait(taskId);
}

export async function gitRebaseOnto(streamId: string, onto: string): Promise<GitOpKickoff> {
  const { taskId } = await desktopApi().gitRebaseOnto(streamId, onto);
  return attachAwait(taskId);
}

export async function getWorkspaceContext(): Promise<WorkspaceContext> {
  return desktopApi().getWorkspaceContext();
}

export async function createStream(input:
  | { title: string; summary?: string; source: "existing"; ref: string }
  | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string }
  | { title: string; summary?: string; source: "worktree"; worktreePath: string },
): Promise<Stream> {
  return desktopApi().createStream(input);
}

export async function listAdoptableWorktrees(): Promise<import("./legacy-ipc-contract.js").GitWorktreeEntry[]> {
  return desktopApi().listAdoptableWorktrees();
}

export async function listSiblingWorktrees(streamId: string): Promise<import("./legacy-ipc-contract.js").GitWorktreeEntry[]> {
  return desktopApi().listSiblingWorktrees(streamId);
}

export async function checkoutStreamBranch(streamId: string, branch: string): Promise<Stream> {
  return desktopApi().checkoutStreamBranch(streamId, branch);
}

export async function getThreadState(streamId: string): Promise<ThreadState> {
  return desktopApi().getThreadState(streamId);
}

export async function createThread(streamId: string, title: string): Promise<ThreadState> {
  return desktopApi().createThread(streamId, title);
}

export async function reorderThread(streamId: string, threadId: string, targetIndex: number): Promise<ThreadState> {
  return desktopApi().reorderThread(streamId, threadId, targetIndex);
}

export async function reorderThreads(streamId: string, orderedThreadIds: string[]): Promise<void> {
  return desktopApi().reorderThreads(streamId, orderedThreadIds);
}

export async function reorderStreams(orderedStreamIds: string[]): Promise<void> {
  return desktopApi().reorderStreams(orderedStreamIds);
}

export async function selectThread(streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().selectThread(streamId, threadId);
}

export async function promoteThread(streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().promoteThread(streamId, threadId);
}

export async function closeThread(streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().closeThread(streamId, threadId);
}

export async function reopenThread(streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().reopenThread(streamId, threadId);
}

export async function listClosedThreads(streamId: string): Promise<Thread[]> {
  return desktopApi().listClosedThreads(streamId);
}

export async function renameThread(streamId: string, threadId: string, title: string): Promise<Thread> {
  return desktopApi().renameThread(streamId, threadId, title);
}

export async function setStreamPrompt(streamId: string, prompt: string | null): Promise<Stream[]> {
  return desktopApi().setStreamPrompt(streamId, prompt);
}

export async function setThreadPrompt(streamId: string, threadId: string, prompt: string | null): Promise<Thread[]> {
  return desktopApi().setThreadPrompt(streamId, threadId, prompt);
}

export async function getThreadWorkState(streamId: string, threadId: string): Promise<ThreadWorkState> {
  return desktopApi().getThreadWorkState(streamId, threadId);
}

export async function createWorkItem(
  streamId: string,
  threadId: string,
  input: {
    kind: WorkItemKind;
    title: string;
    description?: string;
    acceptanceCriteria?: string | null;
    parentId?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
  },
): Promise<ThreadWorkState> {
  return desktopApi().createWorkItem(streamId, threadId, input);
}

export async function updateWorkItem(
  streamId: string,
  threadId: string,
  itemId: string,
  changes: {
    title?: string;
    description?: string;
    acceptanceCriteria?: string | null;
    parentId?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
    category?: string | null;
    tags?: string | null;
  },
): Promise<ThreadWorkState> {
  return desktopApi().updateWorkItem(streamId, threadId, itemId, changes);
}

export async function deleteWorkItem(
  streamId: string,
  threadId: string,
  itemId: string,
): Promise<ThreadWorkState> {
  return desktopApi().deleteWorkItem(streamId, threadId, itemId);
}

export async function reorderWorkItems(
  streamId: string,
  threadId: string,
  orderedItemIds: string[],
): Promise<ThreadWorkState> {
  return desktopApi().reorderWorkItems(streamId, threadId, orderedItemIds);
}

export async function moveWorkItemToThread(
  streamId: string,
  fromThreadId: string,
  itemId: string,
  toThreadId: string,
  toStreamId?: string,
): Promise<{ from: ThreadWorkState; to: ThreadWorkState }> {
  return desktopApi().moveWorkItemToThread(streamId, fromThreadId, itemId, toThreadId, toStreamId);
}

export async function getBacklogState(): Promise<BacklogState> {
  return desktopApi().getBacklogState();
}

export async function createBacklogItem(input: {
  kind: WorkItemKind;
  title: string;
  description?: string;
  acceptanceCriteria?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
  category?: string | null;
  tags?: string | null;
}): Promise<BacklogState> {
  return desktopApi().createBacklogItem(input);
}

export async function updateBacklogItem(
  itemId: string,
  changes: {
    title?: string;
    description?: string;
    acceptanceCriteria?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
    category?: string | null;
    tags?: string | null;
  },
): Promise<BacklogState> {
  return desktopApi().updateBacklogItem(itemId, changes);
}

export async function deleteBacklogItem(itemId: string): Promise<BacklogState> {
  return desktopApi().deleteBacklogItem(itemId);
}

export async function reorderBacklog(orderedItemIds: string[]): Promise<BacklogState> {
  return desktopApi().reorderBacklog(orderedItemIds);
}

export async function moveWorkItemToBacklog(
  streamId: string,
  fromThreadId: string,
  itemId: string,
): Promise<{ from: ThreadWorkState; backlog: BacklogState }> {
  return desktopApi().moveWorkItemToBacklog(streamId, fromThreadId, itemId);
}

export async function moveBacklogItemToThread(
  streamId: string,
  itemId: string,
  toThreadId: string,
): Promise<{ backlog: BacklogState; to: ThreadWorkState }> {
  return desktopApi().moveBacklogItemToThread(streamId, itemId, toThreadId);
}

export async function getGitLog(
  streamId: string,
  options?: { limit?: number; all?: boolean },
): Promise<import("./legacy-ipc-contract.js").GitLogResult> {
  return desktopApi().getGitLog(streamId, options);
}

export async function getCommitDetail(
  streamId: string,
  sha: string,
): Promise<import("./legacy-ipc-contract.js").CommitDetail | null> {
  return desktopApi().getCommitDetail(streamId, sha);
}

export async function getChangeScopes(
  streamId: string,
): Promise<import("./legacy-ipc-contract.js").ChangeScopes> {
  return desktopApi().getChangeScopes(streamId);
}

export async function searchWorkspaceText(
  streamId: string,
  query: string,
  options?: { limit?: number },
): Promise<import("./legacy-ipc-contract.js").TextSearchHit[]> {
  return desktopApi().searchWorkspaceText(streamId, query, options);
}

export async function gitRestorePath(streamId: string, path: string): Promise<import("./legacy-ipc-contract.js").GitOpResult> {
  return desktopApi().gitRestorePath(streamId, path);
}

export async function gitAddPath(streamId: string, path: string): Promise<import("./legacy-ipc-contract.js").GitOpResult> {
  return desktopApi().gitAddPath(streamId, path);
}

export async function gitAppendToGitignore(streamId: string, path: string): Promise<import("./legacy-ipc-contract.js").GitOpResult> {
  return desktopApi().gitAppendToGitignore(streamId, path);
}

export async function gitPush(
  streamId: string,
  options?: { force?: boolean; setUpstream?: boolean; remote?: string; branch?: string },
): Promise<GitOpKickoff> {
  const { taskId } = await desktopApi().gitPush(streamId, options);
  return attachAwait(taskId);
}

export async function gitPull(
  streamId: string,
  options?: { rebase?: boolean; remote?: string; branch?: string },
): Promise<GitOpKickoff> {
  const { taskId } = await desktopApi().gitPull(streamId, options);
  return attachAwait(taskId);
}

export async function gitFetch(
  streamId: string,
  options?: { remote?: string; prune?: boolean; all?: boolean },
): Promise<GitOpKickoff> {
  const { taskId } = await desktopApi().gitFetch(streamId, options);
  return attachAwait(taskId);
}

export async function gitCommitAll(
  streamId: string,
  message: string,
  options?: { includeUntracked?: boolean; paths?: string[] },
): Promise<import("./legacy-ipc-contract.js").GitOpResult & { sha?: string }> {
  return desktopApi().gitCommitAll(streamId, message, options);
}

export async function getAheadBehind(
  streamId: string,
  base: string,
  head?: string,
): Promise<{ ahead: number; behind: number }> {
  return desktopApi().getAheadBehind(streamId, base, head);
}

export async function getCommitsAheadOf(
  streamId: string,
  base: string,
  head: string,
  limit?: number,
): Promise<import("./legacy-ipc-contract.js").GitLogCommit[]> {
  return desktopApi().getCommitsAheadOf(streamId, base, head, limit);
}

export async function listRecentRemoteBranches(
  streamId: string,
  limit?: number,
): Promise<import("./legacy-ipc-contract.js").RemoteBranchEntry[]> {
  return desktopApi().listRecentRemoteBranches(streamId, limit);
}

export async function gitPushCurrentTo(
  streamId: string,
  remote: string,
  branch: string,
): Promise<GitOpKickoff> {
  const { taskId } = await desktopApi().gitPushCurrentTo(streamId, remote, branch);
  return attachAwait(taskId);
}

export async function gitPullRemoteIntoCurrent(
  streamId: string,
  remote: string,
  branch: string,
): Promise<GitOpKickoff> {
  const { taskId } = await desktopApi().gitPullRemoteIntoCurrent(streamId, remote, branch);
  return attachAwait(taskId);
}

export async function listFileCommits(
  streamId: string,
  path: string,
  limit?: number,
): Promise<import("./legacy-ipc-contract.js").GitLogCommit[]> {
  return desktopApi().listFileCommits(streamId, path, limit);
}

export async function gitBlame(
  streamId: string,
  path: string,
): Promise<import("./legacy-ipc-contract.js").BlameLine[]> {
  return desktopApi().gitBlame(streamId, path);
}

export type { LocalBlameEntry } from "./legacy-local-blame.js";

export async function localBlame(
  streamId: string,
  path: string,
): Promise<import("./legacy-local-blame.js").LocalBlameEntry[]> {
  return desktopApi().localBlame(streamId, path);
}

export type WikiNoteSummary = import("./legacy-ipc-contract.js").WikiNoteSummary;
export type WikiNoteSearchHit = import("./legacy-ipc-contract.js").WikiNoteSearchHit;
export type UsageRollup = import("./legacy-ipc-contract.js").UsageRollup;

export async function listWikiNotes(streamId: string): Promise<WikiNoteSummary[]> {
  return desktopApi().listWikiNotes(streamId);
}

export async function readWikiNoteBody(streamId: string, slug: string): Promise<string> {
  return desktopApi().readWikiNoteBody(streamId, slug);
}

export async function writeWikiNoteBody(streamId: string, slug: string, body: string): Promise<void> {
  return desktopApi().writeWikiNoteBody(streamId, slug, body);
}

export async function deleteWikiNote(streamId: string, slug: string): Promise<void> {
  return desktopApi().deleteWikiNote(streamId, slug);
}

export function subscribeWikiNoteEvents(onEvent: () => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type === "wiki-note.changed") onEvent();
  });
}

export async function searchWikiNotes(
  streamId: string,
  query: string,
  limit?: number,
): Promise<WikiNoteSearchHit[]> {
  return desktopApi().searchWikiNotes(streamId, query, limit);
}

export async function recordUsage(input: {
  kind: string;
  key: string;
  event?: string;
  streamId?: string | null;
  threadId?: string | null;
}): Promise<void> {
  return desktopApi().recordUsage(input);
}

export async function listRecentUsage(input: {
  kind: string;
  streamId?: string | null;
  threadId?: string | null;
  limit?: number;
  since?: string;
}): Promise<UsageRollup[]> {
  return desktopApi().listRecentUsage(input);
}

export async function listFrequentUsage(input: {
  kind: string;
  streamId?: string | null;
  threadId?: string | null;
  limit?: number;
  since?: string;
}): Promise<UsageRollup[]> {
  return desktopApi().listFrequentUsage(input);
}

export async function listCurrentlyOpenUsage(input: {
  kind: string;
  streamId?: string | null;
  threadId?: string | null;
}): Promise<string[]> {
  return desktopApi().listCurrentlyOpenUsage(input);
}

export type CodeQualityTool = import("./legacy-ipc-contract.js").CodeQualityTool;
export type CodeQualityScope = import("./legacy-ipc-contract.js").CodeQualityScope;
export type CodeQualityScanStatus = import("./legacy-ipc-contract.js").CodeQualityScanStatus;
export type CodeQualityFindingKind = import("./legacy-ipc-contract.js").CodeQualityFindingKind;
export type CodeQualityScanRow = import("./legacy-ipc-contract.js").CodeQualityScanRow;
export type CodeQualityFindingRow = import("./legacy-ipc-contract.js").CodeQualityFindingRow;

export async function runCodeQualityScan(input: {
  streamId: string;
  tool: CodeQualityTool;
  scope: CodeQualityScope;
  baseRef?: string | null;
}): Promise<CodeQualityScanRow> {
  return desktopApi().runCodeQualityScan(input);
}

export async function listCodeQualityFindings(input: {
  streamId: string;
  tool?: CodeQualityTool;
  paths?: string[];
}): Promise<CodeQualityFindingRow[]> {
  return desktopApi().listCodeQualityFindings(input);
}

export async function listCodeQualityScans(input: {
  streamId: string;
  limit?: number;
}): Promise<CodeQualityScanRow[]> {
  return desktopApi().listCodeQualityScans(input);
}

export function subscribeCodeQualityEvents(
  streamId: string,
  fn: (event: { scanId: number; tool: CodeQualityTool; scope: CodeQualityScope; status: CodeQualityScanStatus }) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "code-quality.scanned") return;
    if (event.streamId !== streamId) return;
    fn({ scanId: event.scanId, tool: event.tool, scope: event.scope, status: event.status });
  });
}

export async function getWorkItemSummaries(ids: string[]): Promise<Array<{
  id: string;
  title: string;
  status: import("./legacy-ipc-contract.js").WorkItemStatus;
  thread_id: string | null;
}>> {
  return desktopApi().getWorkItemSummaries(ids);
}

/**
 * Subscribe to `usage.recorded` events. Optionally filter by `kind` so a
 * Notes-pane consumer only refetches on wiki-note visits.
 */
export function subscribeUsageEvents(
  onEvent: (e: { kind: string; key: string; streamId: string | null; threadId: string | null }) => void,
  filter?: { kind?: string },
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "usage.recorded") return;
    if (filter?.kind && event.kind !== filter.kind) return;
    onEvent({ kind: event.kind, key: event.key, streamId: event.streamId, threadId: event.threadId });
  });
}

export async function reorderThreadQueue(
  streamId: string,
  threadId: string,
  entries: Array<{ id: string }>,
): Promise<void> {
  return desktopApi().reorderThreadQueue(streamId, threadId, entries);
}

export async function removeFollowup(threadId: string, id: string): Promise<void> {
  return desktopApi().removeFollowup(threadId, id);
}

export type BackgroundTask = import("./legacy-ipc-contract.js").BackgroundTask;

export async function listBackgroundTasks(): Promise<BackgroundTask[]> {
  return desktopApi().listBackgroundTasks();
}

export async function getBackgroundTask(id: string): Promise<BackgroundTask | null> {
  return desktopApi().getBackgroundTask(id);
}

export function subscribeBackgroundTaskEvents(
  onChange: () => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type === "background-task.changed") onChange();
  });
}

/**
 * Subscribe to changes for a single background task. The callback
 * receives the change kind ("started" | "updated" | "ended"). Use this
 * to drive in-flight UI off a kickoff IPC's returned `taskId`.
 */
export function subscribeBackgroundTask(
  taskId: string,
  onChange: (kind: "started" | "updated" | "ended") => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type === "background-task.changed" && event.id === taskId) {
      onChange(event.kind);
    }
  });
}

/**
 * Resolve when a background task ends (done or failed). Reads the final
 * task row so callers can inspect `task.status`, `task.error`, and
 * `task.result`. Returns null if the task disappeared (evicted) before
 * we could read it.
 */
export function awaitBackgroundTask(taskId: string): Promise<BackgroundTask | null> {
  return new Promise((resolve) => {
    let settled = false;
    const finish = async () => {
      if (settled) return;
      settled = true;
      unsubscribe();
      resolve(await getBackgroundTask(taskId));
    };
    const unsubscribe = subscribeBackgroundTask(taskId, (kind) => {
      if (kind === "ended") void finish();
    });
    // Race condition: the task may have already ended before we
    // subscribed. Check the current row once on entry.
    void getBackgroundTask(taskId).then((task) => {
      if (task && (task.status === "done" || task.status === "failed")) void finish();
    });
  });
}

export async function listAllRefs(streamId: string): Promise<import("./legacy-ipc-contract.js").RefOption[]> {
  return desktopApi().listAllRefs(streamId);
}

export async function addWorkItemNote(
  streamId: string,
  threadId: string,
  itemId: string,
  note: string,
): Promise<WorkItemEvent[]> {
  return desktopApi().addWorkItemNote(streamId, threadId, itemId, note);
}

export async function listWorkItemEvents(
  streamId: string,
  threadId: string,
  itemId?: string,
): Promise<WorkItemEvent[]> {
  return desktopApi().listWorkItemEvents(streamId, threadId, itemId);
}

export async function getWorkNotes(itemId: string): Promise<WorkNote[]> {
  return desktopApi().getWorkNotes(itemId);
}

export async function getBranchChanges(
  streamId: string,
  baseRef?: string,
): Promise<BranchChanges & { resolvedBaseRef: string | null }> {
  return desktopApi().getBranchChanges(streamId, baseRef);
}

export async function readFileAtRef(
  streamId: string,
  ref: string,
  path: string,
): Promise<{ content: string | null }> {
  return desktopApi().readFileAtRef(streamId, ref, path);
}

export async function listWorkItemEfforts(itemId: string): Promise<EffortDetail[]> {
  return desktopApi().listWorkItemEfforts(itemId);
}

export async function listSnapshots(streamId: string, limit?: number): Promise<FileSnapshot[]> {
  return desktopApi().listSnapshots(streamId, limit);
}

export async function getSnapshotSummary(
  snapshotId: string,
  previousSnapshotId?: string | null,
): Promise<SnapshotSummary | null> {
  return desktopApi().getSnapshotSummary(snapshotId, previousSnapshotId);
}

export async function getSnapshotPairDiff(
  beforeSnapshotId: string | null,
  afterSnapshotId: string,
  path: string,
): Promise<SnapshotDiffResult> {
  return desktopApi().getSnapshotPairDiff(beforeSnapshotId, afterSnapshotId, path);
}

export async function getEffortFiles(effortId: string): Promise<SnapshotSummary | null> {
  return desktopApi().getEffortFiles(effortId);
}

export async function listEffortsEndingAtSnapshots(
  snapshotIds: string[],
): Promise<Record<string, Array<{ effortId: string; workItemId: string; threadId: string; title: string; status: WorkItemStatus; priority: WorkItemPriority }>>> {
  return desktopApi().listEffortsEndingAtSnapshots(snapshotIds);
}

export async function restoreFileFromSnapshot(
  streamId: string,
  snapshotId: string,
  path: string,
): Promise<void> {
  return desktopApi().restoreFileFromSnapshot(streamId, snapshotId, path);
}

export interface FileSnapshotCreatedEventPayload {
  streamId: string;
  snapshotId: string;
  kind: SnapshotSource;
  effortId: string | null;
  threadId: string | null;
}

export function subscribeSnapshotEvents(
  streamId: string,
  fn: (payload: FileSnapshotCreatedEventPayload) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "file-snapshot.created") return;
    if (event.streamId !== streamId) return;
    fn({
      streamId: event.streamId,
      snapshotId: event.snapshotId,
      kind: event.kind,
      effortId: event.effortId,
      threadId: event.threadId,
    });
  });
}

export async function listWorkspaceEntries(streamId: string, path = ""): Promise<WorkspaceEntry[]> {
  return desktopApi().listWorkspaceEntries(streamId, path);
}

export async function listWorkspaceFiles(streamId: string): Promise<{
  files: WorkspaceIndexedFile[];
  summary: WorkspaceStatusSummary;
}> {
  return desktopApi().listWorkspaceFiles(streamId);
}

export async function readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile> {
  return desktopApi().readWorkspaceFile(streamId, path);
}

export async function writeWorkspaceFile(streamId: string, path: string, content: string): Promise<WorkspaceFile> {
  return desktopApi().writeWorkspaceFile(streamId, path, content);
}

export async function createWorkspaceFile(streamId: string, path: string, content = ""): Promise<WorkspaceFile> {
  return desktopApi().createWorkspaceFile(streamId, path, content);
}

export async function createWorkspaceDirectory(streamId: string, path: string): Promise<WorkspacePathChange> {
  return desktopApi().createWorkspaceDirectory(streamId, path);
}

export async function renameWorkspacePath(
  streamId: string,
  fromPath: string,
  toPath: string,
): Promise<WorkspaceRenameResult> {
  return desktopApi().renameWorkspacePath(streamId, fromPath, toPath);
}

export async function deleteWorkspacePath(streamId: string, path: string): Promise<WorkspacePathChange> {
  return desktopApi().deleteWorkspacePath(streamId, path);
}

export function subscribeOxplowEvents(
  listener: (event: OxplowEvent) => void,
): () => void {
  return desktopApi().onOxplowEvent(listener);
}

export function subscribeWorkspaceContext(
  onEvent: (next: WorkspaceContext) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "workspace-context.changed") return;
    onEvent({ gitEnabled: event.gitEnabled });
  });
}

export function subscribeWorkspaceEvents(
  streamId: string,
  onEvent: (event: WorkspaceWatchEvent) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type === "workspace.changed" && event.streamId === streamId) {
      onEvent({
        id: event.id,
        streamId: event.streamId,
        kind: event.kind,
        path: event.path,
        t: event.t,
      });
    }
  });
}

export function subscribeGitRefsEvents(
  streamId: string,
  onEvent: () => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type === "git-refs.changed" && event.streamId === streamId) {
      onEvent();
    }
  });
}

export type WorkItemChangeKind = "created" | "updated" | "note" | "linked" | "deleted" | "reordered" | "moved";

export interface WorkItemChangeEvent {
  streamId: string;
  threadId: string;
  kind: WorkItemChangeKind;
  itemId: string | null;
}

export type AgentStatus = "working" | "waiting";

export interface AgentStatusEntry {
  streamId: string;
  threadId: string;
  status: AgentStatus;
}

export async function listAgentStatuses(streamId?: string): Promise<AgentStatusEntry[]> {
  return desktopApi().listAgentStatuses(streamId);
}

export type FinishedEntry =
  | { kind: "work-item"; itemId: string; title: string; t: string }
  | { kind: "note"; slug: string; title: string; t: string };

export async function listRecentlyFinished(threadId: string | null, limit: number): Promise<FinishedEntry[]> {
  return desktopApi().listRecentlyFinished(threadId, limit);
}

export async function clearRecentlyFinished(threadId: string | null): Promise<void> {
  return desktopApi().clearRecentlyFinished(threadId);
}

export interface PageVisitInputApi {
  refKind: string;
  refId: string;
  payload: unknown;
  label: string;
  streamId?: string | null;
  threadId?: string | null;
  source?: string | null;
}

export interface PageVisitApi {
  id: number;
  t: string;
  streamId: string | null;
  threadId: string | null;
  refKind: string;
  refId: string;
  payload: unknown;
  label: string;
  source: string | null;
}

export interface TopVisitedRowApi {
  refId: string;
  refKind: string;
  payload: unknown;
  label: string;
  count: number;
  lastT: string;
}

export interface CountByDayRowApi {
  day: string;
  count: number;
}

export async function recordPageVisit(input: PageVisitInputApi): Promise<void> {
  return desktopApi().recordPageVisit(input);
}

export async function listRecentPageVisits(opts: {
  threadId?: string | null;
  limit: number;
  dedupeByRef?: boolean;
  excludeKinds?: string[];
}): Promise<PageVisitApi[]> {
  return desktopApi().listRecentPageVisits(opts);
}

export async function topVisitedPages(opts: {
  threadId?: string | null;
  sinceT?: string | null;
  limit: number;
  excludeKinds?: string[];
}): Promise<TopVisitedRowApi[]> {
  return desktopApi().topVisitedPages(opts);
}

export async function countPageVisitsByDay(opts: {
  refId?: string;
  threadId?: string | null;
  sinceT?: string;
  untilT?: string;
}): Promise<CountByDayRowApi[]> {
  return desktopApi().countPageVisitsByDay(opts);
}

export function subscribePageVisitEvents(onEvent: () => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type === "page-visit.changed") onEvent();
  });
}

/** Drop every visit row for a given page reference. Used when a page
 *  is deleted (real persistent or virtual, e.g. an op-error entry) so
 *  it disappears from rail history. Generic — not tied to any one
 *  page kind. */
export async function forgetPage(refKind: string, refId: string): Promise<void> {
  return desktopApi().forgetPage(refKind, refId);
}

export async function getRepoConflictState(
  streamId: string,
): Promise<import("./legacy-ipc-contract.js").RepoConflictState> {
  return desktopApi().getRepoConflictState(streamId);
}

export function subscribeAgentStatus(
  streamId: string | "all",
  onEvent: (entry: AgentStatusEntry) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "agent-status.changed") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent({ streamId: event.streamId, threadId: event.threadId, status: event.status });
  });
}

export interface BacklogChangeEvent {
  kind: WorkItemChangeKind;
  itemId: string | null;
}

export function subscribeBacklogEvents(onEvent: (event: BacklogChangeEvent) => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "backlog.changed") return;
    onEvent({ kind: event.kind, itemId: event.itemId });
  });
}

export function subscribeWorkItemEvents(
  streamId: string | "all",
  onEvent: (event: WorkItemChangeEvent) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "work-item.changed") return;
    if (!event.streamId || !event.threadId) return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent({
      streamId: event.streamId,
      threadId: event.threadId,
      kind: event.kind,
      itemId: event.itemId,
    });
  });
}

export async function probeDaemon(): Promise<boolean> {
  try {
    return await desktopApi().ping();
  } catch {
    return false;
  }
}

export type NormalizedEvent =
  | { kind: "session-start"; t: number; sessionId?: string; cwd?: string }
  | { kind: "session-end"; t: number; sessionId?: string; reason?: string }
  | { kind: "user-prompt"; t: number; sessionId?: string; prompt: string }
  | {
      kind: "tool-use-start";
      t: number;
      sessionId?: string;
      toolName: string;
      target?: string;
      input?: unknown;
    }
  | {
      kind: "tool-use-end";
      t: number;
      sessionId?: string;
      toolName: string;
      status: "ok" | "error";
    }
  | { kind: "stop"; t: number; sessionId?: string }
  | { kind: "notification"; t: number; sessionId?: string; message: string }
  | { kind: "meta"; t: number; sessionId?: string; hookEventName: string; raw: unknown };

export interface StoredEvent {
  id: number;
  streamId: string;
  threadId?: string;
  pane?: "working" | "talking";
  normalized: NormalizedEvent;
}

export async function listHookEvents(streamId?: string): Promise<StoredEvent[]> {
  return desktopApi().listHookEvents(streamId);
}

export function subscribeHookEvents(
  streamId: string | "all",
  onEvent: (event: StoredEvent) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.type !== "hook.recorded") return;
    if (streamId !== "all" && event.streamId !== streamId) return;
    onEvent(event.event as StoredEvent);
  });
}

// Lazy-built adapter that maps the legacy DesktopApi method shape to
// real Tauri `commands.*` calls. Constructed once on first access so
// any module that imports this file picks up the same instance.
let cachedAdapter: DesktopApi | null = null;
function desktopApi(): DesktopApi {
  if (!cachedAdapter) {
    cachedAdapter = buildLegacyAdapter();
  }
  return cachedAdapter;
}

/**
 * Exposes the legacy adapter for the few files that historically read
 * `window.oxplowApi` directly (logger, lsp, TerminalPane, App.tsx).
 * Replaces the global; same shape, just imported. Each call site
 * should eventually migrate off this onto `commands.*` from
 * `tauri-bridge`, at which point this export can go away.
 */
export function legacyApi(): DesktopApi {
  return desktopApi();
}

/**
 * Open an http(s) URL in the user's OS browser. The main process
 * re-validates the URL against the same scheme allowlist as the
 * renderer; non-allowed URLs return `{ ok: false }` so callers can
 * show a refusal toast.
 */
export async function openExternalUrl(url: string): Promise<{ ok: boolean; reason?: string }> {
  return desktopApi().openExternalUrl(url);
}
