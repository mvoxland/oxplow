// Legacy → Tauri bridge adapter.
//
// Builds a `window.oxplowApi`-shaped object at module load, mapping
// each legacy method name to a real `commands.*` call from the
// tauri-bridge. Methods we haven't ported yet throw a clear "not
// yet ported" error so the renderer surfaces a friendly toast
// instead of `undefined is not a function`.
//
// This is a *temporary* shim. Each call site in `api.ts` should
// migrate to `commands.*` directly; once the last one does, this
// file (and `legacy-ipc-contract.ts` and `api.ts`) can be deleted.

import { commands } from "./tauri-bridge/generated/bindings";
import type { DesktopApi } from "./legacy-ipc-contract";
import { listen } from "@tauri-apps/api/event";

function unwrap<T>(result: { status: "ok"; data: T } | { status: "error"; error: unknown }): T {
  if (result.status === "ok") return result.data;
  // The Rust IpcError is `{ code, message, cause? }`; surface the
  // message so error toasts read clearly.
  const err = result.error as { message?: string; code?: string } | undefined;
  throw new Error(err?.message ?? err?.code ?? "ipc error");
}

function notPorted(name: string): never {
  throw new Error(`oxplow legacy API method "${name}" is not yet ported to Tauri`);
}

/**
 * Build the legacy adapter. Returns a frozen object suitable for
 * `window.oxplowApi`. Methods backed by a real Tauri command call
 * through; the rest throw `notPorted(name)` lazily.
 */
export function buildLegacyAdapter(): DesktopApi {
  const adapter: Partial<DesktopApi> = {
    // -- liveness --
    ping: async () => unwrap(await commands.ping()),

    // -- streams --
    listStreams: async () => unwrap(await commands.listStreams()),
    getCurrentStream: async () => {
      const cur = unwrap(await commands.getCurrentStream());
      if (cur) return cur;
      // Fallback to primary if no current pointer is set.
      const primary = unwrap(await commands.getPrimaryStream());
      if (!primary) throw new Error("no primary stream available");
      return primary;
    },
    switchStream: async (id: string) => {
      unwrap(await commands.switchStream(id));
      const got = unwrap(await commands.getCurrentStream());
      return got;
    },
    renameCurrentStream: async (title: string) => {
      const cur = unwrap(await commands.getCurrentStream());
      if (!cur) throw new Error("no current stream to rename");
      return unwrap(await commands.renameStream({ id: cur.id, title }));
    },
    renameStream: async (id: string, title: string) =>
      unwrap(await commands.renameStream({ id, title })),

    // -- threads --
    closeThread: async (id: string) => unwrap(await commands.closeThread(id)),
    reopenThread: async (id: string) => unwrap(await commands.reopenThread(id)),
    promoteThread: async (id: string) => unwrap(await commands.promoteThread(id)),
    renameThread: async (id: string, title: string) =>
      unwrap(await commands.renameThread({ id, title })),
    setThreadPrompt: async (id: string, prompt: string | null) =>
      unwrap(await commands.setThreadPrompt({ id, prompt })),
    listClosedThreads: async (streamId: string) =>
      unwrap(await commands.listClosedThreads(streamId)),
    selectThread: async (streamId: string, threadId: string | null) =>
      unwrap(await commands.selectThread({ streamId, threadId })),
    createThread: async (streamId: string, title: string, paneTarget?: string) =>
      unwrap(await commands.createThread({ streamId, title, paneTarget: paneTarget ?? null })),
    reorderThreadQueue: async (streamId: string, order: string[]) =>
      unwrap(await commands.reorderThreadQueue({ streamId, order })),

    // -- work items --
    createWorkItem: async (
      _streamId: string,
      threadId: string,
      input: Record<string, unknown>,
    ) =>
      unwrap(
        await commands.createWorkItem({
          threadId,
          input: input as never,
        }),
      ),
    updateWorkItem: async (
      _streamId: string,
      _threadId: string,
      itemId: string,
      changes: Record<string, unknown>,
    ) =>
      unwrap(
        await commands.updateWorkItem({
          id: itemId,
          changes: changes as never,
        }),
      ),
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

    // -- backlog --
    getBacklogState: async () => unwrap(await commands.getBacklogState()),
    createBacklogItem: async (input: Record<string, unknown>) =>
      unwrap(
        await commands.createWorkItem({
          threadId: null,
          input: input as never,
        }),
      ),
    updateBacklogItem: async (itemId: string, changes: Record<string, unknown>) =>
      unwrap(await commands.updateWorkItem({ id: itemId, changes: changes as never })),
    deleteBacklogItem: async (itemId: string) =>
      unwrap(await commands.deleteWorkItem(itemId)),

    // -- work notes --
    addWorkItemNote: async (
      _streamId: string,
      _threadId: string,
      itemId: string,
      body: string,
      author?: string,
    ) =>
      unwrap(await commands.addWorkNote(itemId, body, author ?? "user")),
    getWorkNotes: async (itemId: string) => unwrap(await commands.listWorkNotes(itemId)),

    // -- git ops --
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
    listAllRefs: async () => unwrap(await commands.listAllRefs()),
    listGitRefs: async () => unwrap(await commands.listAllRefs()),
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
    listAdoptableWorktrees: async () => unwrap(await commands.listAdoptableWorktrees()),
    listSiblingWorktrees: async () => unwrap(await commands.listSiblingWorktrees()),
    getCommitDetail: async (sha: string) => unwrap(await commands.getCommitDetail(sha)),
    getGitLog: async (options?: { limit?: number; all?: boolean }) =>
      unwrap(await commands.getGitLog(options?.limit ?? null, options?.all ?? false)),

    // -- workspace files --
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

    // -- usage / page-visit --
    recordPageVisit: async (input: { pageKind: string; pageId: string; durationMs?: number | null }) =>
      unwrap(
        await commands.recordPageVisit(input.pageKind, input.pageId, input.durationMs ?? null),
      ),
    listRecentPageVisits: async (opts?: { limit?: number }) =>
      unwrap(await commands.listRecentPageVisits(opts?.limit ?? 50)),
    topVisitedPages: async (opts?: { limit?: number }) =>
      unwrap(await commands.topVisitedPages(opts?.limit ?? 50)),
    forgetPage: async (kind: string, id: string) =>
      unwrap(await commands.forgetPage(kind, id)),
    countPageVisitsByDay: async (opts?: { days?: number }) =>
      unwrap(await commands.countPageVisitsByDay(opts?.days ?? 30)),
    recordUsage: async (input: { kind: string; payload?: unknown }) =>
      unwrap(
        await commands.recordUsage(input.kind, JSON.stringify(input.payload ?? {})),
      ),
    listRecentUsage: async (limit?: number) =>
      unwrap(await commands.listRecentUsage(limit ?? 50)),

    // -- code quality --
    listCodeQualityScans: async (limit?: number) =>
      unwrap(await commands.listCodeQualityScans(limit ?? 50)),
    listCodeQualityFindings: async (scanId: number) =>
      unwrap(await commands.listCodeQualityFindings(scanId)),

    // -- snapshots --
    listSnapshots: async (path?: string) =>
      unwrap(await commands.listSnapshots(path ?? "")),

    // -- wiki notes --
    listWikiNotes: async () => unwrap(await commands.listWikiNotes()),
    deleteWikiNote: async (slug: string) => unwrap(await commands.deleteWikiNote(slug)),
    searchWikiNotes: async (query: string, limit?: number) =>
      unwrap(await commands.searchWikiTitles(query, limit ?? null)),

    // -- background tasks --
    listBackgroundTasks: async () => unwrap(await commands.listBackgroundTasks()),
    getBackgroundTask: async (id: string) => unwrap(await commands.getBackgroundTask(id)),

    // -- hook events --
    listHookEvents: async (_streamId?: string) => {
      // Legacy signature was streamId-scoped; new one is thread-scoped.
      // Drop the streamId filter — the renderer can scope on the
      // server side once the new schema lands.
      return unwrap(await commands.listHookEvents(null, null));
    },
    listAgentStatuses: async () => unwrap(await commands.listAgentStatuses()),

    // -- config --
    getConfig: async () => unwrap(await commands.getConfig()),
    setAgentPromptAppend: async (text: string) =>
      unwrap(await commands.setAgentPromptAppend(text)),
    setSnapshotRetentionDays: async (days: number) =>
      unwrap(await commands.setSnapshotRetentionDays(days)),
    setSnapshotMaxFileBytes: async (bytes: number) =>
      unwrap(await commands.setSnapshotMaxFileBytes(bytes)),
    setGeneratedDirs: async (dirs: string[]) =>
      unwrap(await commands.setGeneratedDirs(dirs)),

    // -- followups --
    removeFollowup: async (id: string) => unwrap(await commands.removeFollowup(id)),

    // -- external URL --
    openExternalUrl: async (url: string) => {
      try {
        unwrap(await commands.openExternalUrl(url));
        return { ok: true };
      } catch (e) {
        return { ok: false, reason: e instanceof Error ? e.message : String(e) };
      }
    },

    // -- bridge from oxplow:event channel onto legacy onOxplowEvent --
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

  // Wrap with a Proxy so any unported method throws a clear error
  // instead of `adapter.foo is undefined`.
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

/**
 * Install the legacy adapter on `window.oxplowApi`. Call once at app
 * boot, before any module that calls `desktopApi()` runs. Idempotent.
 */
export function installLegacyAdapter(): void {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  if (w.oxplowApi) return;
  w.oxplowApi = buildLegacyAdapter();
}
