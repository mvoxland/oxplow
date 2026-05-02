# Tauri-migration code review

> Independent review of the work delivered on the `tauri-migration` branch.
> Compares against the original `newui` branch (the pre-migration Electron build).
> Goal: surface lost functionality, correctness issues, and gaps that must close
> before the branch is mergeable.

## Bottom line

The branch establishes the **architectural skeleton** of the Tauri rewrite but
ships only a **fraction of the original product's behavior**. It builds and the
Rust crates' tests pass (102 tests, all green), but:

- **The frontend does not currently typecheck.** 113 TS errors, almost all
  caused by deleted modules the UI still imports (`../electron/ipc-contract`,
  `../git/git`, `../config/config`, `../electron/local-blame`).
- **The Tauri command surface is ~7% of the original IPC surface.** 9 Rust
  commands vs. 127 IPC channels in `src/electron/main.ts`.
- **Major store/feature areas have no Rust counterpart at all** (background
  tasks, followups, hook events, agent statuses, file snapshots, code-quality
  scans, page visits, usage tracking, wiki pages, work-item events, work-item
  links, work-item efforts, threads-work-state, snapshots-effort).
- **The original ~3,500-line `runtime.ts` orchestration is mostly absent**;
  the Rust `oxplow-runtime` crate ships only `WriteGuard` + `FilingEnforcement`
  (the two pure-logic predicates), not the rest of the orchestrator.
- **MCP tool surface ported: 4 of ~30**.
- **Git operations ported: ~10 of 50+**. Notably absent: blame, commit detail,
  log graph, search, push/pull/fetch, ahead/behind, file restore, snapshot diff.

The plan's own estimate ("~12–18 weeks for a single experienced engineer")
matches what's missing. What landed is closer to a **~2-week skeleton** that
proves the architectural choices work and gives a deterministic path to
completion. Treating it as a complete migration would be misleading.

The remainder of this document inventories the gaps so a follow-up plan can
work through them.

---

## Build / test status

| Component | Status |
|---|---|
| `cargo build --workspace` | ✅ green |
| `cargo test --workspace` | ✅ 102 passed, 0 failed |
| `cargo clippy --workspace` | ⚠️ untested in this branch |
| `cargo fmt --check` | ⚠️ untested |
| `apps/desktop` `tsc --noEmit` | ❌ **113 errors** (UI imports deleted modules) |
| `bun test` (frontend) | ❌ blocked by typecheck |
| `cargo tauri build` | ⚠️ untested locally; would fail because the frontend can't build |
| Playwright e2e (`tests-e2e/`) | ⚠️ untested; expects Electron URL scheme |

The branch's CI workflow (`.github/workflows/ci.yml`) was rewritten for the
new toolchain, but the **`bun run typecheck` step would fail today** — the CI
was never run against the actual codebase as it stands.

---

## Frontend breakage (most urgent)

`apps/desktop/src/api.ts` and several of its consumers still import from
deleted TS modules:

```ts
import type { DesktopApi, OxplowEvent } from "../electron/ipc-contract.js";
import type { GitLogResult, ... } from "../git/git.js";
```

These produce 113 TS errors in `apps/desktop` and effectively kill the
frontend build. The migration plan called for `git mv src/ui` plus a
mechanical import-path swap to the bridge (§3 step 16), but the swap was
**only** done in `tauri-bridge/index.ts` itself — the existing `src/ui/api.ts`
(now `apps/desktop/src/api.ts`) was never touched.

**Fix before merge:** rewrite `apps/desktop/src/api.ts` against
`tauri-bridge`, deleting every reference to `../electron/*` and `../git/*`.
Most of its 700+ lines describe IPC methods that don't exist in the new
backend, so this work is bounded by the IPC-surface gap (next section), not
just a find/replace.

Other affected files:
- `apps/desktop/src/App.tsx`
- `apps/desktop/src/external-file-sync.ts`
- `apps/desktop/src/components/TerminalPane.tsx`
- `apps/desktop/src/lsp.ts` (uses `window.oxplowApi`, the Electron preload's
  injected global — doesn't exist under Tauri).

---

## IPC surface gap (127 → 9)

The original Electron main process registered **127 IPC channels** in
`src/electron/main.ts`. The Rust port (`oxplow-tauri-ipc`) registers **9**.

### Ported (9)
- `app_version`
- `list_streams`, `ensure_primary`, `create_worktree`, `delete_stream`
- `list_threads`, `list_work_items_for_thread`, `list_backlog`
- `open_external_url`

### Unported categories (118)

**Stream management (rest):** `getCurrentStream`, `switchStream`,
`renameStream`, `renameCurrentStream`, `setStreamPrompt`, `reorderStreams`,
`checkoutStreamBranch`.

**Threads (rest):** `getThreadState`, `getThreadWorkState`, `selectThread`,
`createThread`, `closeThread`, `reopenThread`, `renameThread`, `promoteThread`,
`reorderThread`, `reorderThreads`, `reorderThreadQueue`, `setThreadPrompt`,
`listClosedThreads`.

**Work items (rest):** `createWorkItem`, `updateWorkItem`, `deleteWorkItem`,
`reorderWorkItems`, `moveWorkItemToBacklog`, `moveWorkItemToThread`,
`getWorkItemSummaries`, `listWorkItemEvents`, `listWorkItemEfforts`,
`getEffortFiles`, `addWorkItemNote`, `getWorkNotes`, `removeFollowup`.

**Backlog (rest):** `getBacklogState`, `createBacklogItem`,
`updateBacklogItem`, `deleteBacklogItem`, `moveBacklogItemToThread`,
`reorderBacklog`.

**Git (50+):** `listBranches`, `getDefaultBranch`, `listGitRefs`, `listAllRefs`,
`gitMergeInto`, `gitRebaseOnto`, `gitPush`, `gitPushCurrentTo`, `gitPull`,
`gitPullRemoteIntoCurrent`, `gitFetch`, `gitCommitAll`, `gitAddPath`,
`gitRestorePath`, `gitAppendToGitignore`, `gitBlame`, `localBlame`,
`renameGitBranch`, `deleteGitBranch`, `getAheadBehind`, `getCommitsAheadOf`,
`getBranchChanges`, `getChangeScopes`, `getCommitDetail`, `getGitLog`,
`getRepoConflictState`, `listFileCommits`, `listRecentRemoteBranches`,
`listSiblingWorktrees`, `listAdoptableWorktrees`, `readFileAtRef`,
`searchWorkspaceText`.

**Workspace files:** `getWorkspaceContext`, `listWorkspaceEntries`,
`listWorkspaceFiles`, `readWorkspaceFile`, `writeWorkspaceFile`,
`createWorkspaceFile`, `createWorkspaceDirectory`, `deleteWorkspacePath`,
`renameWorkspacePath`.

**Snapshots:** `listSnapshots`, `getSnapshotPairDiff`, `getSnapshotSummary`,
`restoreFileFromSnapshot`, `listEffortsEndingAtSnapshots`.

**LSP / terminal:** `openLspClient`, `closeLspClient`, `sendLspMessage`,
`openTerminalSession`, `closeTerminalSession`, `sendTerminalMessage`.

**Config:** `getConfig`, `setAgentPromptAppend`, `setSnapshotRetentionDays`,
`setSnapshotMaxFileBytes`, `setGeneratedDirs`.

**Hook ingest:** `listHookEvents`, `listAgentStatuses`.

**Background tasks:** `getBackgroundTask`, `listBackgroundTasks`.

**Wiki pages:** `listWikiPages`, `searchWikiPages`, `readWikiPageBody`,
`writeWikiPageBody`, `deleteWikiPage`.

**Code quality:** `runCodeQualityScan`, `listCodeQualityScans`,
`listCodeQualityFindings`.

**Usage / page visits:** `recordPageVisit`, `recordUsage`,
`listRecentPageVisits`, `listRecentUsage`, `listFrequentUsage`,
`listCurrentlyOpenUsage`, `listRecentlyFinished`, `clearRecentlyFinished`,
`countPageVisitsByDay`, `forgetPage`, `topVisitedPages`.

**Misc:** `clipboardReadText`, `setNativeMenu`, `updateEditorFocus`, `logUi`,
`ping`, `subscribeOxplowEvents` and the rest of the event-stream
subscriptions.

Each of these needs a Rust handler; many also need:
- A new domain trait + store impl (e.g., `BackgroundTaskStore`,
  `CodeQualityFindingStore`, `UsageEventStore`, `WikiPageStore`,
  `PageVisitStore`, `WorkItemEffortStore`, `SnapshotStore`).
- A migration entry in `oxplow-db`'s schema.
- Tauri events (the original had 17+ subscription channels — `subscribeWorkItemEvents`,
  `subscribeBackgroundTaskEvents`, etc. — which need
  `app_handle.emit` calls).

---

## Domain model gaps

`oxplow-domain` ships:
- IDs (StreamId, ThreadId, WorkItemId, NoteId, AgentTurnId)
- Timestamp
- `Stream`, `Thread`, `WorkItem`, `WorkItemLink`
- 3 store traits (StreamStore, ThreadStore, WorkItemStore)
- Status / kind / priority enums

Original TS persistence layer covered (and tested):
- `agent_turn` (referenced in schema but no store/trait)
- `agent_status`
- `background_task` + `followup`
- `batch_file_change` (not present in v1 schema)
- `code_quality_scan`, `code_quality_finding`
- `commit_point`, `wait_point`, `finished_seen`
- `file_snapshot`, `snapshot_entry`
- `page_visit`
- `usage_event`
- `wiki_page`, `wiki_page_thread_update`
- `work_item_commit`, `work_item_effort`, `work_item_effort_file`,
  `work_item_effort_turn`
- `work_item_event`, `work_note`
- `runtime_state` (table exists but no service uses it)

The `V1__initial_schema.sql` includes some of these tables (work_notes,
agent_turn, work_item_events, work_item_links) but nothing reads or writes
them — those are dead schema.

**Recommended next pass:** for each missing store, port the TS file
into a `<store>.rs` module under `oxplow-db`, with the same
`spawn_blocking`-wrapped CRUD pattern used by the existing three
stores, and a trait declaration in `oxplow-domain::stores`.

---

## `oxplow-runtime` is far smaller than the TS `runtime.ts`

The crate ships pure-logic predicates:
- `WriteGuard::build_write_guard_response`
- `FilingEnforcement::build_filing_enforcement_pre_tool_deny`

Original `src/electron/runtime.ts` was 3,527 lines. Other functions it
contained:
- Stream lifecycle wiring (delegated to `oxplow-session` here, partial)
- Branch checkout flow (`checkoutStreamBranch` — not ported)
- Agent pane lifecycle (`ensureAgentPane`, `closeStreamPanes`, etc.)
- Stop hook delegation to `decideStopDirective` (the
  `stop-hook-pipeline` crate's logic; **not ported** in any form)
- Hook ingest pipeline
- Worktree adoption (`listAdoptableWorktrees`)
- File-snapshot capture
- Background task supervision
- Daemon recovery
- Agent prompt assembly (loads CLAUDE.md / agent-skills, injects
  session-context block)

The `stop-hook-pipeline.ts` (280 LOC, with its own test file) is **completely
absent** from the Rust workspace. Stop-hook decisions are a load-bearing part
of the agent loop; merging without porting them means the runtime can't
service agent turns at all.

---

## `oxplow-mcp`: 4 of ~30 tools

Ported:
- `ping`
- `app_version`
- `list_streams`
- `list_backlog`

Original tool surface (from `src/mcp/mcp-tools.ts` etc.):
`add_followup`, `add_work_note`, `await_user`, `complete_task`,
`create_work_item`, `delegate_query`, `delete_note`, `delete_work_item`,
`dispatch_work_item`, `file_epic_with_children`, `find_notes_for_file`,
`fork_thread`, `get_note_metadata`, `get_subsystem_doc`, `get_thread_context`,
`get_thread_notes`, `get_work_item`, `link_work_items`, `list_followups`,
`list_notes`, `list_ready_work`, `list_thread_work`, `lsp_definition`,
`lsp_diagnostics`, `lsp_hover`, `lsp_references`, `read_work_options`,
`record_query_finding`, `remove_followup`, `reorder_work_items`,
`resync_note`, `search_note_bodies`, `search_notes`,
`transition_work_items`, `update_work_item`.

Plus the `lsp-mcp-tools.ts` (LSP-bridging tools) and
`wiki-note-mcp-tools.ts` (wiki capture). None of those are ported.

---

## Git surface gap

`oxplow-git` exports: `is_git_repo`, `is_git_worktree`,
`detect_current_branch`, `list_branches`, `ensure_worktree`,
`get_repo_conflict_state`.

Original `src/git/git.ts` exported 50+ functions. Notable absences in the
Rust port:
- Local blame and `git blame` integration (the editor's blame margin
  depends on these).
- Snapshot/diff machinery.
- `getCommitDetail`, `getGitLog`, `getCommitsAheadOf`, `getAheadBehind`.
- `searchWorkspaceText` (ripgrep-equivalent fallback).
- Push / pull / fetch (sync + async variants).
- File restore (`restorePath`).
- Branch rename / delete.
- `git merge` / `git rebase` invocation.
- Refs watcher integration with `oxplow-fs-watch` (the watcher crate
  exists but no `oxplow-git` consumer wires it up).
- Workspace-files surface (the ~190-line `workspace-files.ts`).
- Notes watcher (`notes-watch.ts`).

The `oxplow-fs-watch` crate is fully implemented and tested but
**unused** — no current consumer. The TS git layer used `chokidar` to
watch `.git/refs/heads/`; the Rust layer doesn't yet.

---

## DB schema accuracy

`V1__initial_schema.sql` collapses 50 TS migrations into one initial state,
which is a defensible choice for a clean break. Concerns:

- **Naming drift**: the legacy schema uses `batch` / `batches`; the Rust
  schema renames to `thread` / `threads`. If a user has existing data in
  the Electron DB, none of it migrates. The plan calls this out (no upgrade
  path), but worth flagging in user-facing release notes.
- **Missing tables present in TS but absent from V1**: `batch_file_change`,
  `commit_point`, `file_snapshot`, `snapshot_entry`, `page_visit`,
  `usage_event`, `wiki_page`, `wiki_page_thread_update`, `work_item_commit`,
  `work_item_effort` and friends, `code_quality_scan`,
  `code_quality_finding`, `agent_status`, `wait_point`, `finished_seen`.
  These need to be added before their corresponding stores can be ported.
- **Status enum mismatch on `Thread`**: the TS code uses
  `status TEXT NOT NULL` with values `active | queued`. The Rust schema
  uses `open | closed`. The Rust runtime crate's `WriteGuard` checks
  `thread.status == ThreadStatus::Closed`, which encodes the opposite
  semantics from the TS check (`thread.status === "active"`). On a
  read-only thread the new code returns `None` (no deny), while the TS
  returns the deny body. **This is a regression** that would let
  read-only threads write to the worktree once the wiring is connected.
  The intent in the rewrite was probably to encode "writer / non-writer"
  via a separate stream-level pointer; the rewrite hasn't followed
  through, and the existing `WriteGuard` test asserts
  `Some(deny)` only when `tool_name` is mutating — none of the tests
  exercise the active-vs-closed thread distinction faithfully.

---

## Type fidelity drift

`oxplow_domain::Thread` is missing fields the original `Thread` interface
carried:
- `closed_at: string | null`
- `custom_prompt: string | null`
- The `ThreadState` aggregate (selectedThreadId / activeThreadId / threads)
  doesn't exist anywhere in Rust.

`oxplow_domain::Stream` is missing:
- `custom_prompt: string | null`
- `panes: { working, talking }` (the Rust struct has flat
  `working_pane` / `talking_pane` strings, not the structured pane info
  the TS contract returned).
- `resume: { working_session_id, talking_session_id }` (similar — flat
  strings instead of the structured object).

These flatten when the IPC layer reshapes them, but the JSON wire shape
diverges from the existing TS contract. Frontend code expecting
`stream.panes.working` will break.

---

## Test parity

The plan called for "every TS test that defines current backend behavior
has a Rust counterpart, written first" (Goal 4).

Original TS test files (sampled):
- `runtime.test.ts` — Stop-hook + filing-enforcement integration: NOT ported
- `stop-hook-pipeline.test.ts` — NOT ported
- `mcp-server.test.ts` (~640 LOC) — NOT ported
- `mcp-tools.test.ts`, `lsp-mcp-tools.test.ts`,
  `wiki-note-mcp-tools.test.ts` — NOT ported
- `work-item-store.test.ts`, `thread-store.test.ts`,
  `stream-store.test.ts` — partially ported (basic CRUD only; the TS
  files exercised dozens of corner cases the Rust tests don't)
- `migrations.test.ts` — NOT ported (no migration test exists)
- `claude-plugin.test.ts`, `agent-status.test.ts`, `editor-focus.test.ts`,
  `file-session.test.ts`, `hook-ingest.test.ts`,
  `resume-tracker.test.ts` — NOT ported
- `local-blame.test.ts` — NOT ported
- `external-content-policy.test.ts` — NOT ported
- `code-quality-store.test.ts`, `snapshot-store.test.ts`,
  `usage-store.test.ts`, `page-visit-store.test.ts`,
  `snapshot-effort.test.ts`, `wiki-note-store.test.ts`,
  `wiki-note-thread-update-store.test.ts`,
  `work-item-effort-store.test.ts` — NOT ported

Rust test counts as of this branch:
- 102 tests total, distributed:
  - oxplow-domain: 14
  - oxplow-db: 16
  - oxplow-config: 11
  - oxplow-fs-watch: 3
  - oxplow-git: 16
  - oxplow-session: 8
  - oxplow-runtime: 15
  - oxplow-tmux: 3
  - oxplow-pty: 3
  - oxplow-lsp: 7
  - oxplow-mcp: 2
  - oxplow-tauri-ipc: 2
  - oxplow-app: 2

The original repo had on the order of ~2,000 unit tests across the
backend (matching its ~12k LOC of `*.test.ts`). The branch has ~5% of that.

---

## Specific bugs and concerns

1. **Port leak / no DB closing**: `oxplow-db::Database` holds an
   `Arc<Pool<SqliteConnectionManager>>` with no explicit shutdown. On
   app exit the pool drops naturally, but for tests that open
   tempfile DBs in tight loops this could exhaust file descriptors on
   Windows where SQLite's deletion-while-open is brittle.

2. **`tempfile::TempDir::keep` in tests**: `oxplow-pty/src/lib.rs:431`
   uses `.keep()` on a `TempDir`, which intentionally leaks the
   directory so the spawned process has somewhere to live. Tests pass
   but accumulate orphaned directories under `/tmp` over many runs.
   Should be replaced with an explicit cleanup-on-test-exit pattern.

3. **`oxplow-pty` wait task and child handle**: the spawn task moves
   `child` into the wait-thread inside an `Arc<Mutex<Option<...>>>`,
   then immediately drops the Arc clone on the entry. This means
   `Cmd::Kill` cannot actually `child.kill()` — the entry's
   `child: None` slot stays empty. The tests pass because they kill
   the manager via dropping or because the child exits naturally.
   In production, calling `kill_pane` on a long-running child does
   nothing; the master is freed, the slave dies eventually, but the
   child process is orphaned until OS-level cleanup. This needs a
   redesign (e.g., the wait task signals a done-flag the entry
   inspects on Kill, falling back to `child.kill()` if still running).

4. **`oxplow-tauri-ipc::open_external_url` capability scope**: the
   capability windows pattern `ext-url-*` is parsed by Tauri 2 as a
   glob, but the `capabilities/external-url.json` schema may require
   the literal `windows: ["pattern1", "pattern2"]` syntax with each
   entry as an exact label or a documented-glob. Untested whether
   Tauri 2 honors the wildcard there. Worth verifying with a real
   `cargo tauri dev` run.

5. **`oxplow-config::write_project_config` round-trip drops user
   comments**: this is documented behavior but the TS implementation
   had the same limitation, so it's not a regression — flagging only
   for the user-experience risk.

6. **`oxplow-git::ensure_worktree` shells out to `git`** but only
   handles the new-branch and existing-branch cases; the original TS
   `ensureWorktree` had additional logic for branch detection from a
   remote (`origin/main` style branch sources), worktree adoption,
   and verification. Worth porting before this is used in anger.

7. **`oxplow-tauri-ipc::commands.rs` error mapping is weak**: every
   error becomes `IpcError::internal(e.to_string())`, losing the
   structured error variants. The plan called for "typed errors, not
   stringified anyhow" — current code is closer to the latter.
   Suggest a `From<SessionError> for IpcError` impl that maps each
   variant to a stable `code` string.

8. **No event emission**: the plan's §1 says "PTY/git/fs streaming
   uses Tauri events." No `app_handle.emit` call exists in the entire
   workspace. The frontend's subscribe-to-events code (the 17
   `subscribeXxxEvents` functions in `api.ts`) has nothing to attach
   to.

9. **`tauri-specta` `Json<T>` return type was abandoned in
   `oxplow-mcp`** because of macro friction. The fix (manually
   `serde_json::to_string_pretty` then wrap in `Content::text`)
   produces stringified JSON that loses MCP's structured-output
   schema benefit. Worth revisiting once rmcp's macro story
   stabilizes.

10. **The frontend bridge re-exports `oxplow.*` but the rest of the
    UI calls `window.oxplowApi.*`** (the Electron preload's injected
    name). The migration didn't update those call sites; even if the
    Rust commands existed, the UI would never reach them without a
    follow-up sweep.

---

## Recommended next steps

In priority order:

1. **Fix the frontend typecheck.** Rewrite `apps/desktop/src/api.ts`
   against the bridge; delete every reference to `../electron`,
   `../git`, `../config`, `../mcp`, `../session`, `../persistence`.
   Treat IPC functions backed by unported Rust commands as `throw new
   Error("not yet ported")` placeholders so the UI compiles and
   crashes loudly at runtime instead of silently calling
   `window.oxplowApi.foo` that doesn't exist.

2. **Add `WorkspaceContext`-style commands** so the UI can boot:
   `get_current_stream`, `get_thread_state`, `get_workspace_context`,
   `get_config`, plus the corresponding event subscriptions.

3. **Port the runtime god-object** (`stop-hook-pipeline`,
   `hook-ingest`, agent prompt assembly, the
   `decideStopDirective` decision tree, `agent_turn` lifecycle).
   Without these the Stop hook can't service an agent turn — the app
   may launch but won't function.

4. **Port the workspace-files + git operations** to give the file
   tree / editor a working backend. Without `read_workspace_file`,
   `write_workspace_file`, blame, status, etc., the editor pane is
   inert.

5. **Fix the `oxplow-pty` child-handle bug** (concern #3 above)
   before any pane is spawned in production.

6. **Fix the `Thread` status semantic regression** (concern under
   "DB schema accuracy"): align the Rust enum + `WriteGuard` predicate
   with the TS `active`/`queued` model, OR explicitly redesign with
   tests proving the new model works.

7. **Add migration tests** for the V1 schema so future schema
   evolution has a regression baseline.

8. **Wire `oxplow-fs-watch` into `oxplow-git`** (refs watcher + notes
   watcher) so the UI can observe branch and note changes.

9. **Iteratively port the remaining MCP tools and IPC commands**
   following the established adapter pattern. This is mechanical but
   bounded — each command is a `#[tauri::command]` calling into
   `oxplow-app`; each MCP tool is an `#[tool]` doing the same.

10. **Update `.context/*.md` docs** in lockstep as the corresponding
    Rust subsystems land — currently most of them point at deleted
    paths.

Honest sizing: closing this gap to feature parity is the rest of the
plan's 12–18-week budget.

---

## Items the migration got right

To balance the litany above:

- **The architectural skeleton is clean.** Crate boundaries match the
  plan's layering rule; nothing fights the Tauri 2.x state-management
  guidance.
- **Domain types are faithful** (where they exist). Snake-case enum
  serialization matches the TS wire format bit-for-bit, so once stores
  and commands are ported the JSON shapes will be drop-in.
- **The PTY owner-task pattern is correct in shape**, including the
  Windows `Drop` mitigation for the documented `portable-pty`
  teardown race. Bug aside (concern #3), this is the right design.
- **The LSP proxy** is a clean transport-only layer with codec tests,
  exactly as the plan recommended.
- **`tauri-specta`** is wired and produces typed bindings checked
  into the repo with a CI drift guard. Adding a new command
  automatically updates the TS surface.
- **`oxplow-config`** is a near-1:1 port with the same validation
  semantics and a write-back path that round-trips cleanly.
- **Tests, where they exist, hit real systems**: real SQLite, real
  git repos, real tmux subprocesses, real LSP server (via a python
  fake). No DB mocking — matches the plan's testing rule.
- **The migration commit history is granular** (one commit per step),
  so reverting any one step is mechanical if the next step's design
  needs revisiting.
