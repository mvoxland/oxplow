import { commands } from "./tauri-bridge/generated/bindings.js";
import { listen } from "@tauri-apps/api/event";
import type { DesktopApi, OxplowEvent } from "./api-types.js";

// -- Legacy adapter helpers (now lives here). Inlined
// here so the renderer-side compatibility layer lives in a single file.

function unwrap<T>(result: { status: "ok"; data: T } | { status: "error"; error: unknown }): T {
  if (result.status === "ok") return result.data;
  const err = result.error as { message?: string; code?: string } | undefined;
  throw new Error(err?.message ?? err?.code ?? "ipc error");
}

/// Synthesize a success-shaped GitOpResult for void-returning Tauri
/// commands (gitAddPath / gitRestorePath / gitAppendToGitignore).
/// Renderer code expects a {success, stdout, stderr, status} shape
/// to decide whether to surface a toast. Since these commands either
/// succeed or throw, success here is unconditional.
function synthOk(): import("./tauri-bridge/index.js").GitOpResult {
  return { success: true, stdout: "", stderr: "", status: 0 };
}

/// Pure slug derivation: lowercase ASCII alphanumerics, runs of any
/// other character collapse to a single hyphen, leading/trailing
/// hyphens trimmed. Worktree slug is fixed at creation and never
/// changes, so the formatting needs to be conservative.
function slugifyTitle(title: string): string {
  const base = title
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return base.length > 0 ? base : `stream-${Date.now()}`;
}

// adaptStream / adaptThread were dropped in the api.ts migration:
// - Stream's synthesized `panes` / `resume` objects had zero
//   readers (renderer code only reads working_pane / talking_pane
//   directly).
// - Thread's "closed" → "queued" status coercion was wrong; the
//   rail only ever lists active+queued threads, and closed ones
//   come from a separate listClosedThreads fetch.

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

function buildDesktopAdapter(): DesktopApi {
  const adapter: DesktopApi = {
    ping: async () => unwrap(await commands.ping()),
    listStreams: async () =>
      (unwrap(await commands.listStreams()) as unknown[])/* bindings shape */,
    getCurrentStream: async () => {
      const cur = unwrap(await commands.getCurrentStream());
      if (cur) return cur;
      const primary = unwrap(await commands.getPrimaryStream());
      if (!primary) throw new Error("no primary stream available");
      return primary;
    },
    switchStream: async (id: string) => {
      unwrap(await commands.switchStream(id));
      return unwrap(await commands.getCurrentStream());
    },
    renameCurrentStream: async (title: string) => {
      const cur = unwrap(await commands.getCurrentStream());
      if (!cur) throw new Error("no current stream to rename");
      return unwrap(await commands.renameStream({ id: cur.id, title }));
    },
    renameStream: async (id: string, title: string) =>
      unwrap(await commands.renameStream({ id, title })),
    setStreamPrompt: async (id: string, prompt: string | null) =>
      unwrap(await commands.setStreamPrompt({ id, prompt })),
    checkoutStreamBranch: async (id: string, branch: string) =>
      unwrap(await commands.checkoutStreamBranch(id, branch)),
    reorderStreams: async (order: string[]) => unwrap(await commands.reorderStreams(order)),
    reorderThreads: async (streamId: string, order: string[]) =>
      unwrap(await commands.reorderThreadQueue({ streamId, order })),
    reorderThread: async (streamId: string, order: string[]) =>
      unwrap(await commands.reorderThreadQueue({ streamId, order })),
    createStream: async (input: {
      title: string;
      summary?: string;
      source: "existing" | "new" | "worktree";
      ref?: string;
      branch?: string;
      startPointRef?: string;
      worktreePath?: string;
    }) => {
      const slug = slugifyTitle(input.title);
      switch (input.source) {
        case "existing": {
          if (!input.ref) throw new Error("createStream: missing ref for existing source");
          return unwrap(
            await commands.createWorktree({
              slug,
              title: input.title,
              branch: input.ref,
              branchSource: input.ref,
            }),
          );
        }
        case "new": {
          if (!input.branch) throw new Error("createStream: missing branch for new source");
          return unwrap(
            await commands.createWorktree({
              slug,
              title: input.title,
              branch: input.branch,
              branchSource: input.startPointRef ?? input.branch,
            }),
          );
        }
        case "worktree":
          throw new Error(
            "Adopting an existing worktree on disk is not yet ported to Tauri",
          );
      }
    },
    closeThread: async (id: string) => unwrap(await commands.closeThread(id)),
    reopenThread: async (id: string) => unwrap(await commands.reopenThread(id)),
    promoteThread: async (id: string) => unwrap(await commands.promoteThread(id)),
    renameThread: async (id: string, title: string) =>
      unwrap(await commands.renameThread({ id, title })),
    setThreadPrompt: async (id: string, prompt: string | null) =>
      unwrap(await commands.setThreadPrompt({ id, prompt })),
    listClosedThreads: async (streamId: string) =>
      (unwrap(await commands.listClosedThreads(streamId)) as unknown[])/* bindings shape */,
    selectThread: async (streamId: string, threadId: string | null) =>
      unwrap(await commands.selectThread({ streamId, threadId })),
    createThread: async (streamId: string, title: string, paneTarget?: string) =>
      unwrap(await commands.createThread({ streamId, title, paneTarget: paneTarget ?? null })),
    reorderThreadQueue: async (streamId: string, order: string[]) =>
      unwrap(await commands.reorderThreadQueue({ streamId, order })),
    getThreadState: async (streamId: string) => {
      const raw = unwrap(await commands.getThreadState(streamId)) as {
        threads: unknown[];
        [k: string]: unknown;
      };
      return { ...raw, threads: raw.threads/* bindings shape */ };
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
    getRepoConflictState: async (streamId?: string | null) =>
      unwrap(await commands.getRepoConflictState(streamId ?? null)),
    getAheadBehind: async (
      streamId: string | null | undefined,
      base: string,
      head: string,
    ) => unwrap(await commands.getAheadBehind(streamId ?? null, base, head)),
    getCommitsAheadOf: async (
      streamId: string | null | undefined,
      base: string,
      head: string,
      limit?: number,
    ) =>
      unwrap(
        await commands.getCommitsAheadOf(streamId ?? null, base, head, limit ?? 200),
      ),
    getDefaultBranch: async () => unwrap(await commands.getDefaultBranch()),
    listBranches: async () => unwrap(await commands.listLocalBranches()),
    renameGitBranch: async (from: string, to: string) =>
      unwrap(await commands.renameBranch(from, to)),
    deleteGitBranch: async (branch: string, force?: boolean) =>
      unwrap(await commands.deleteBranch(branch, force ?? false)),
    gitAppendToGitignore: async (streamId: string | null | undefined, entry: string) => {
      unwrap(await commands.appendToGitignore(streamId ?? null, entry));
      return synthOk();
    },
    gitRestorePath: async (streamId: string | null | undefined, path: string) => {
      unwrap(await commands.restorePath(streamId ?? null, path));
      return synthOk();
    },
    gitFetch: async (streamId: string | null | undefined, remote?: string | null) =>
      unwrap(await commands.gitFetch(streamId ?? null, remote ?? null)),
    gitPull: async (streamId: string | null | undefined) =>
      unwrap(await commands.gitPull(streamId ?? null)),
    gitPullRemoteIntoCurrent: async (
      streamId: string | null | undefined,
      remote: string,
      branch: string,
    ) =>
      unwrap(
        await commands.gitPullRemoteIntoCurrent(streamId ?? null, remote, branch),
      ),
    gitPush: async (streamId: string | null | undefined) =>
      unwrap(await commands.gitPush(streamId ?? null)),
    gitPushCurrentTo: async (
      streamId: string | null | undefined,
      remote: string,
      branch: string,
    ) => unwrap(await commands.gitPushCurrentTo(streamId ?? null, remote, branch)),
    gitMergeInto: async (streamId: string | null | undefined, source: string) =>
      unwrap(await commands.gitMergeInto(streamId ?? null, source)),
    gitRebaseOnto: async (streamId: string | null | undefined, onto: string) =>
      unwrap(await commands.gitRebaseOnto(streamId ?? null, onto)),
    gitCommitAll: async (streamId: string | null | undefined, message: string) =>
      unwrap(await commands.gitCommitAll(streamId ?? null, message)),
    gitAddPath: async (streamId: string | null | undefined, path: string) => {
      unwrap(await commands.gitAddPath(streamId ?? null, path));
      return synthOk();
    },
    gitBlame: async (streamId: string | null | undefined, path: string) =>
      unwrap(await commands.gitBlame(streamId ?? null, path)),
    localBlame: async (
      streamId: string | null | undefined,
      path: string,
      diskText: string,
    ) => unwrap(await commands.localBlame(streamId ?? null, path, diskText)),
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
    listFileCommits: async (
      streamId: string | null | undefined,
      path: string,
      limit?: number,
    ) =>
      unwrap(await commands.listFileCommits(streamId ?? null, path, limit ?? null)),
    listRecentRemoteBranches: async (limit?: number) =>
      unwrap(await commands.listRecentRemoteBranches(limit ?? null)),
    readFileAtRef: async (ref: string, path: string) =>
      unwrap(await commands.readFileAtRef(ref, path)),
    searchWorkspaceText: async (
      streamId: string | null | undefined,
      query: string,
      limit?: number,
    ) => unwrap(await commands.searchWorkspaceText(streamId ?? null, query, limit ?? null)),
    getBranchChanges: async (streamId: string | null | undefined, baseRef: string) =>
      unwrap(await commands.getBranchChanges(streamId ?? null, baseRef)),
    getChangeScopes: async (streamId?: string | null) => {
      const raw = unwrap(await commands.getChangeScopes(streamId ?? null));
      return {
        staged: raw.staged,
        unstaged: raw.unstaged,
        currentBranch: raw.current_branch ?? undefined,
        branchBase: raw.branch_base ?? undefined,
        upstream: raw.upstream ?? undefined,
        onDefaultBranch: raw.on_default_branch,
      };
    },
    listAdoptableWorktrees: async () => unwrap(await commands.listAdoptableWorktrees()),
    listSiblingWorktrees: async () => unwrap(await commands.listSiblingWorktrees()),
    getCommitDetail: async (streamId: string | null | undefined, sha: string) =>
      unwrap(await commands.getCommitDetail(streamId ?? null, sha)),
    getGitLog: async (
      streamId?: string | null,
      options?: { limit?: number; all?: boolean },
    ) =>
      unwrap(
        await commands.getGitLog(streamId ?? null, options?.limit ?? null, options?.all ?? false),
      ),
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
    setNativeMenu: async (groups: unknown) => {
      try {
        unwrap(await commands.setNativeMenu(groups as never));
      } catch {
        // Don't break the UI if menu installation fails (e.g.
        // platform doesn't support a particular accelerator).
      }
    },
    onMenuCommand: (handler: (commandId: string) => void) => {
      let stopped = false;
      const unlistenPromise = listen("menu:command", (e) => {
        if (stopped) return;
        const payload = e.payload as { id?: string } | null;
        if (payload?.id) handler(payload.id);
      });
      return () => {
        stopped = true;
        void unlistenPromise.then((u) => u());
      };
    },
    updateEditorFocus: async () => {},
    logUi: async (entry: {
      clientId?: string;
      level: string;
      message: string;
      context?: unknown;
      timestamp?: string;
    }) => {
      try {
        unwrap(
          await commands.logUi({
            clientId: entry.clientId ?? null,
            level: entry.level,
            message: entry.message,
            context: entry.context !== undefined ? JSON.stringify(entry.context) : null,
            timestamp: entry.timestamp ?? null,
          }),
        );
      } catch {
        // Don't let a logging failure surface to callers.
      }
    },
    runCodeQualityScan: async (
      tool: string,
      scope?: string,
      files?: string[],
    ) =>
      unwrap(await commands.runCodeQualityScan(tool, scope ?? "workspace", files ?? null)),
    openLspClient: async (streamId: string, languageId: string) =>
      unwrap(await commands.openLspClient(streamId, languageId)),
    closeLspClient: async (clientId: string) => {
      try {
        unwrap(await commands.closeLspClient(clientId));
      } catch {
        // Idempotent: already-closed clients return INVALID; treat as no-op.
      }
    },
    sendLspMessage: async (clientId: string, payload: string) =>
      unwrap(await commands.sendLspMessage(clientId, payload)),
    onLspEvent: (handler: (event: { clientId: string; message: string }) => void) => {
      let stopped = false;
      const unlistenPromise = listen("lsp:event", (e) => {
        if (stopped) return;
        handler(e.payload as { clientId: string; message: string });
      });
      return () => {
        stopped = true;
        void unlistenPromise.then((u) => u());
      };
    },
    openTerminalSession: async (
      paneTarget: string,
      cols: number,
      rows: number,
      transportMode: string,
    ) =>
      unwrap(await commands.openTerminalSession(paneTarget, cols, rows, transportMode)),
    closeTerminalSession: async (sessionId: string) => {
      try {
        unwrap(await commands.closeTerminalSession(sessionId));
      } catch {
        // Idempotent close.
      }
    },
    sendTerminalMessage: async (sessionId: string, message: string) =>
      unwrap(await commands.sendTerminalMessage(sessionId, message)),
    onTerminalEvent: (handler: (event: { sessionId: string; message: string }) => void) => {
      let stopped = false;
      const unlistenPromise = listen("terminal:event", (e) => {
        if (stopped) return;
        handler(e.payload as { sessionId: string; message: string });
      });
      return () => {
        stopped = true;
        void unlistenPromise.then((u) => u());
      };
    },
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

  // The whole DesktopApi surface is filled in above; no Proxy or
  // throw-stub wrapper. Missing methods are a TypeScript error,
  // not a runtime crash on first call.
  return adapter;
}

export type { OxplowEvent } from "./api-types.js";
// Use the tauri-specta-generated shapes directly for the
// snake_case-native bindings (CommitDetail, GitLogCommit,
// RemoteBranchEntry, GitOpResult, BlameLine, …). The api-types
// camelCase legacy definitions were drifting from runtime shape
// and only existed because the original Electron build wrapped
// them in adapters; nothing converts shape today.
// Bindings shapes for the types whose call sites have been
// migrated. Adding more is a per-call-site refactor: each consumer
// has to be updated to the new field names. Types not on this list
// stay on the api-types camelCase legacy shape until their consumers
// are migrated.
export type {
  GitOpResult,
  GitWorktreeEntry,
  RemoteBranchEntry,
  GitLogCommit,
  CommitDetail,
  BlameLine,
} from "./tauri-bridge/index.js";
// The remaining legacy types still come from api-types because
// their consumers read fields that don't exist on the bindings
// shape yet (e.g. GitLogResult.currentBranch / branchHeads / tags,
// RemoteBranchEntry.remote / branch / lastCommitDate, GitWorktreeEntry
// camelCase aliases, BranchChangeEntry.status / additions / deletions
// — bindings expose .change and don't surface line counts here yet).
// Migrating each one is per-call-site work; until then the shape
// the runtime hands the renderer is the bindings shape but the
// renderer's TypeScript believes it's the legacy shape.
export type {
  GitLogRef,
  GitLogResult,
  ChangeScopes,
  TextSearchHit,
  RefOption,
  GroupedGitRefs,
  BranchChangeEntry,
  BranchChanges,
} from "./api-types.js";

// Stream / Thread come straight from the Tauri bindings — the
// renderer reads the flat shape (working_pane / talking_pane /
// custom_prompt) directly; no synthesis happens at the boundary.
import type { Stream, Thread } from "./tauri-bridge/index.js";
export type { Stream, Thread };

export interface ThreadState {
  selectedThreadId: string | null;
  activeThreadId: string | null;
  threads: Thread[];
}

// Work-item types now come from the Tauri bindings. The bindings
// emit a `deleted_at` field that the earlier UI interface didn't model;
// readers either ignore it or filter on it (earlier stores already
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

// Stream + config wrappers. Each call goes straight to the
// tauri-specta `commands` surface — no buildDesktopAdapter
// detour. The unwrap() helper at the top of this file converts
// the {status, data|error} envelope into a plain promise.

export async function listStreams(): Promise<Stream[]> {
  return unwrap(await commands.listStreams());
}

export async function getCurrentStream(): Promise<Stream> {
  const cur = unwrap(await commands.getCurrentStream());
  if (cur) return cur;
  const primary = unwrap(await commands.getPrimaryStream());
  if (!primary) throw new Error("no primary stream available");
  return primary;
}

export async function switchStream(id: string): Promise<Stream> {
  unwrap(await commands.switchStream(id));
  return getCurrentStream();
}

export async function renameStream(streamId: string, title: string): Promise<Stream> {
  return unwrap(await commands.renameStream({ id: streamId, title }));
}

export async function renameCurrentStream(title: string): Promise<Stream> {
  const cur = unwrap(await commands.getCurrentStream());
  if (!cur) throw new Error("no current stream to rename");
  return renameStream(cur.id, title);
}

export async function getConfig(): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.getConfig()) as unknown as import("./api-types.js").OxplowConfig;
}

export async function setAgentPromptAppend(text: string): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setAgentPromptAppend(text)) as unknown as import("./api-types.js").OxplowConfig;
}

export async function setGeneratedDirs(dirs: string[]): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setGeneratedDirs(dirs)) as unknown as import("./api-types.js").OxplowConfig;
}

export async function setSnapshotRetentionDays(days: number): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setSnapshotRetentionDays(days)) as unknown as import("./api-types.js").OxplowConfig;
}

export async function setSnapshotMaxFileBytes(bytes: number): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setSnapshotMaxFileBytes(bytes)) as unknown as import("./api-types.js").OxplowConfig;
}

export async function listBranches(): Promise<BranchRef[]> {
  return desktopApi().listBranches();
}

export async function getDefaultBranch(): Promise<string | null> {
  return unwrap(await commands.getDefaultBranch());
}

export async function listGitRefs(): Promise<import("./api-types.js").GroupedGitRefs> {
  return desktopApi().listGitRefs();
}

export async function renameGitBranch(
  from: string,
  to: string,
): Promise<import("./tauri-bridge/index.js").GitOpResult> {
  unwrap(await commands.renameBranch(from, to));
  return synthOk();
}

export async function deleteGitBranch(
  branch: string,
  options?: { force?: boolean },
): Promise<import("./tauri-bridge/index.js").GitOpResult> {
  unwrap(await commands.deleteBranch(branch, options?.force ?? false));
  return synthOk();
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

export async function listAdoptableWorktrees(): Promise<
  import("./tauri-bridge/index.js").GitWorktreeEntry[]
> {
  return unwrap(await commands.listAdoptableWorktrees());
}

export async function listSiblingWorktrees(
  _streamId: string,
): Promise<import("./tauri-bridge/index.js").GitWorktreeEntry[]> {
  return unwrap(await commands.listSiblingWorktrees());
}

export async function checkoutStreamBranch(streamId: string, branch: string): Promise<Stream> {
  return unwrap(await commands.checkoutStreamBranch(streamId, branch));
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
  unwrap(await commands.reorderStreams(orderedStreamIds));
}

export async function selectThread(streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().selectThread(streamId, threadId);
}

export async function promoteThread(_streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().promoteThread(_streamId, threadId);
}

export async function closeThread(_streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().closeThread(_streamId, threadId);
}

export async function reopenThread(_streamId: string, threadId: string): Promise<ThreadState> {
  return desktopApi().reopenThread(_streamId, threadId);
}

export async function listClosedThreads(streamId: string): Promise<Thread[]> {
  return unwrap(await commands.listClosedThreads(streamId));
}

export async function renameThread(_streamId: string, threadId: string, title: string): Promise<Thread> {
  return unwrap(await commands.renameThread({ id: threadId, title }));
}

export async function setStreamPrompt(streamId: string, prompt: string | null): Promise<Stream[]> {
  unwrap(await commands.setStreamPrompt({ id: streamId, prompt }));
  return listStreams();
}

export async function setThreadPrompt(
  _streamId: string,
  threadId: string,
  prompt: string | null,
): Promise<Thread[]> {
  unwrap(await commands.setThreadPrompt({ id: threadId, prompt }));
  return [];
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
): Promise<import("./api-types.js").GitLogResult> {
  return desktopApi().getGitLog(streamId, options);
}

export async function getCommitDetail(
  streamId: string,
  sha: string,
): Promise<import("./tauri-bridge/index.js").CommitDetail | null> {
  return desktopApi().getCommitDetail(streamId, sha);
}

export async function getChangeScopes(
  streamId: string,
): Promise<import("./api-types.js").ChangeScopes> {
  return desktopApi().getChangeScopes(streamId);
}

export async function searchWorkspaceText(
  streamId: string,
  query: string,
  options?: { limit?: number },
): Promise<import("./api-types.js").TextSearchHit[]> {
  return desktopApi().searchWorkspaceText(streamId, query, options);
}

export async function gitRestorePath(streamId: string, path: string): Promise<import("./tauri-bridge/index.js").GitOpResult> {
  return desktopApi().gitRestorePath(streamId, path);
}

export async function gitAddPath(streamId: string, path: string): Promise<import("./tauri-bridge/index.js").GitOpResult> {
  return desktopApi().gitAddPath(streamId, path);
}

export async function gitAppendToGitignore(streamId: string, path: string): Promise<import("./tauri-bridge/index.js").GitOpResult> {
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
): Promise<import("./tauri-bridge/index.js").GitOpResult & { sha?: string }> {
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
): Promise<import("./tauri-bridge/index.js").GitLogCommit[]> {
  return desktopApi().getCommitsAheadOf(streamId, base, head, limit);
}

export async function listRecentRemoteBranches(
  streamId: string,
  limit?: number,
): Promise<import("./tauri-bridge/index.js").RemoteBranchEntry[]> {
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
): Promise<import("./tauri-bridge/index.js").GitLogCommit[]> {
  return desktopApi().listFileCommits(streamId, path, limit);
}

export async function gitBlame(
  streamId: string,
  path: string,
): Promise<import("./tauri-bridge/index.js").BlameLine[]> {
  return desktopApi().gitBlame(streamId, path);
}

/// Renderer-side LocalBlameEntry: the bindings shape plus an
/// optional `workItem` overlay the editor's blame margin paints
/// when a snapshot/work-item attribution exists. The runtime
/// today only populates {line, source, git}; `workItem` arrives
/// once the snapshot blob store grows attribution lookup. Until
/// then the editor's local-blame branch is dormant but typesafe.
export interface LocalBlameEntry {
  line: number;
  source: string;
  git: import("./tauri-bridge/index.js").BlameLine | null;
  workItem?: {
    id: string;
    title: string;
    endedAt: string;
  };
}

export async function localBlame(
  streamId: string,
  path: string,
): Promise<LocalBlameEntry[]> {
  return desktopApi().localBlame(streamId, path);
}

export type WikiNoteSummary = import("./api-types.js").WikiNoteSummary;
export type WikiNoteSearchHit = import("./api-types.js").WikiNoteSearchHit;
export type UsageRollup = import("./api-types.js").UsageRollup;

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

export type CodeQualityTool = import("./api-types.js").CodeQualityTool;
export type CodeQualityScope = import("./api-types.js").CodeQualityScope;
export type CodeQualityScanStatus = import("./api-types.js").CodeQualityScanStatus;
export type CodeQualityFindingKind = import("./api-types.js").CodeQualityFindingKind;
export type CodeQualityScanRow = import("./api-types.js").CodeQualityScanRow;
export type CodeQualityFindingRow = import("./api-types.js").CodeQualityFindingRow;

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
  status: import("./api-types.js").WorkItemStatus;
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

export type BackgroundTask = import("./api-types.js").BackgroundTask;

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

export async function listAllRefs(streamId: string): Promise<import("./api-types.js").RefOption[]> {
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
): Promise<import("./api-types.js").BranchChanges & { resolvedBaseRef: string | null }> {
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
): Promise<import("./api-types.js").RepoConflictState> {
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

// Lazy-built adapter that maps the DesktopApi method shape to
// real Tauri `commands.*` calls. Constructed once on first access so
// any module that imports this file picks up the same instance.
let cachedAdapter: DesktopApi | null = null;
function desktopApi(): DesktopApi {
  if (!cachedAdapter) {
    cachedAdapter = buildDesktopAdapter();
  }
  return cachedAdapter;
}

/**
 * Exposes the adapter for the few files that historically read
 * `window.oxplowApi` directly (logger, lsp, TerminalPane, App.tsx).
 * Replaces the global; same shape, just imported. Each call site
 * should eventually migrate off this onto `commands.*` from
 * `tauri-bridge`, at which point this export can go away.
 */
export function desktopBridge(): DesktopApi {
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
