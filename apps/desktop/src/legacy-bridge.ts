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

/// Adapt a bindings-shape Stream to the legacy api.ts Stream by
/// synthesizing the nested `panes` / `resume` sub-objects from the
/// flat working_pane / talking_pane / *_session_id fields.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function adaptStream(s: any): any {
  if (!s) return s;
  return {
    ...s,
    custom_prompt: s.custom_prompt ?? null,
    panes: {
      working: s.working_pane ?? "",
      talking: s.talking_pane ?? "",
    },
    resume: {
      working_session_id: s.working_session_id ?? "",
      talking_session_id: s.talking_session_id ?? "",
    },
  };
}

/// Adapt a bindings-shape Thread. Legacy `status` was `"active" |
/// "queued"`; the bindings add `"closed"`. Map closed threads to
/// queued for legacy callers (closed threads aren't normally in
/// the active list anyway).
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function adaptThread(t: any): any {
  if (!t) return t;
  return {
    ...t,
    status: t.status === "closed" ? "queued" : t.status,
    closed_at: t.closed_at ?? null,
  };
}

/// Adapt a bindings-shape BackgroundTask to the legacy api.ts shape:
/// - started_at / ended_at (RFC3339 strings) → startedAt / endedAt
///   (epoch ms). Legacy callers do arithmetic on the timestamps.
/// - result_json (JSON-encoded string) → result (parsed value).
/// - Pass-through label/detail/error/progress/kind/status/id.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function adaptBackgroundTask(t: any): any {
  if (!t) return t;
  let result: unknown = undefined;
  if (typeof t.result_json === "string" && t.result_json.length > 0) {
    try {
      result = JSON.parse(t.result_json);
    } catch {
      // ignore; legacy callers tolerate undefined.
    }
  }
  return {
    ...t,
    startedAt: t.started_at ? Date.parse(t.started_at) : Date.now(),
    endedAt: t.ended_at ? Date.parse(t.ended_at) : null,
    result,
  };
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
    listStreams: async () =>
      (unwrap(await commands.listStreams()) as unknown[]).map(adaptStream),
    getCurrentStream: async () => {
      const cur = unwrap(await commands.getCurrentStream());
      if (cur) return adaptStream(cur);
      // Fallback to primary if no current pointer is set.
      const primary = unwrap(await commands.getPrimaryStream());
      if (!primary) throw new Error("no primary stream available");
      return adaptStream(primary);
    },
    switchStream: async (id: string) => {
      unwrap(await commands.switchStream(id));
      const got = unwrap(await commands.getCurrentStream());
      return adaptStream(got);
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
    reorderStreams: async (order: string[]) =>
      unwrap(await commands.reorderStreams(order)),
    // Legacy alias used by some UI code paths.
    reorderThreads: async (streamId: string, order: string[]) =>
      unwrap(await commands.reorderThreadQueue({ streamId, order })),
    reorderThread: async (streamId: string, order: string[]) =>
      unwrap(await commands.reorderThreadQueue({ streamId, order })),
    // No corresponding Tauri command — surface a helpful error.
    createStream: async () => {
      throw new Error("createStream is replaced by createWorktree under Tauri");
    },

    // -- threads --
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
        unwrap(
          await commands.createThread({ streamId, title, paneTarget: paneTarget ?? null }),
        ),
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
    getWorkItemSummaries: async (threadId?: string | null) =>
      unwrap(await commands.getWorkItemSummaries(threadId ?? null)),

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
    listWorkItemEvents: async (
      _streamId: string,
      _threadId: string,
      itemId?: string,
    ) =>
      unwrap(await commands.listWorkItemEvents(itemId ?? null, null)),
    listWorkItemEfforts: async (itemId: string) =>
      unwrap(await commands.listWorkItemEfforts(itemId)),
    getEffortFiles: async (effortId: string) =>
      unwrap(await commands.getEffortFiles(effortId)),
    listEffortsEndingAtSnapshots: async (snapshotIds: number[]) =>
      unwrap(await commands.listEffortsEndingAtSnapshots(snapshotIds)),

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
    localBlame: async (path: string, diskText: string) =>
      unwrap(await commands.localBlame(path, diskText)),
    listAllRefs: async () => unwrap(await commands.listAllRefs()),
    /// The legacy `listGitRefs` shape was a UI-flavored grouping
    /// (local branches as BranchRef[], remotes grouped per remote,
    /// recent branches by name). The new `commands.listAllRefs`
    /// returns the raw RefOption flat lists. Transform here so
    /// existing UI code keeps working — once each consumer migrates
    /// to the new shape this transform can go away.
    listGitRefs: async () => {
      const raw = unwrap(await commands.listAllRefs());
      const localBranches = raw.locals.map((r) => ({
        kind: "local" as const,
        name: r.label,
        ref: r.ref,
      }));
      // Group remotes by the leading "<remote>/" segment.
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
        tags: raw.tags.map((t) => ({
          name: t.label,
          ref: t.ref,
        })),
        // Naive "recent" = first 5 local branches; real recency would
        // need a separate API. The picker treats this as a hint.
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
      // Map snake_case from the bindings to the camelCase legacy
      // shape the renderer expects, plus zero-fill the legacy arrays
      // (which now only get populated by `getBranchChanges`).
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
    listFrequentUsage: async (limit?: number) =>
      unwrap(await commands.listFrequentUsage(limit ?? 50)),
    listCurrentlyOpenUsage: async (limit?: number) =>
      unwrap(await commands.listCurrentlyOpenUsage(limit ?? 50)),
    listRecentlyFinished: async (limit?: number) =>
      unwrap(await commands.listRecentlyFinished(limit ?? 50)),
    clearRecentlyFinished: async () =>
      unwrap(await commands.clearRecentlyFinished()),

    // -- code quality --
    listCodeQualityScans: async (limit?: number) =>
      unwrap(await commands.listCodeQualityScans(limit ?? 50)),
    listCodeQualityFindings: async (scanId: number) =>
      unwrap(await commands.listCodeQualityFindings(scanId)),

    // -- snapshots --
    listSnapshots: async (path?: string) =>
      unwrap(await commands.listSnapshots(path ?? "")),
    getSnapshotPairDiff: async (beforeId?: number, afterId?: number) =>
      unwrap(
        await commands.getSnapshotPairDiff(beforeId ?? null, afterId ?? null),
      ),
    getSnapshotSummary: async (streamId?: string, limit?: number) =>
      unwrap(
        await commands.getSnapshotSummary(streamId ?? null, limit ?? null),
      ),
    restoreFileFromSnapshot: async (snapshotId: number) =>
      unwrap(await commands.restoreFileFromSnapshot(snapshotId)),

    // -- wiki notes --
    listWikiNotes: async () => unwrap(await commands.listWikiNotes()),
    deleteWikiNote: async (slug: string) => unwrap(await commands.deleteWikiNote(slug)),
    searchWikiNotes: async (query: string, limit?: number) =>
      unwrap(await commands.searchWikiTitles(query, limit ?? 50)),
    readWikiNoteBody: async (slug: string) =>
      unwrap(await commands.readWikiNoteBody(slug)),
    writeWikiNoteBody: async (slug: string, body: string) =>
      unwrap(await commands.writeWikiNoteBody(slug, body)),

    // -- background tasks --
    listBackgroundTasks: async () =>
      (unwrap(await commands.listBackgroundTasks()) as unknown[]).map(adaptBackgroundTask),
    getBackgroundTask: async (id: string) =>
      adaptBackgroundTask(unwrap(await commands.getBackgroundTask(id))),

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

    // -- legacy UI plumbing — Tauri has native equivalents or
    // doesn't need these. Stubs so the renderer doesn't crash. --
    setNativeMenu: async () => {
      // Tauri menus are configured via the Rust shell, not the
      // renderer. No-op silently.
    },
    onMenuCommand: () => {
      // No menu-command channel under Tauri yet; return an unsubscribe
      // that does nothing.
      return () => {};
    },
    updateEditorFocus: async () => {
      // Editor-focus telemetry was Electron-only.
    },
    logUi: async (payload: unknown) => {
      // Forward to console; the daemon has its own tracing.
      // eslint-disable-next-line no-console
      console.log("[ui]", payload);
    },
    runCodeQualityScan: async (
      tool: string,
      scope?: string,
      files?: string[],
    ) =>
      unwrap(
        await commands.runCodeQualityScan(tool, scope ?? "workspace", files ?? null),
      ),
    openLspClient: async () => {
      throw new Error("openLspClient: LSP session manager not yet ported");
    },
    closeLspClient: async () => {
      // Tolerate close-of-non-existent — UI calls this in cleanup paths.
    },
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

