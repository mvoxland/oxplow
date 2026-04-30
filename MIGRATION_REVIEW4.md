# Independent code review #4 — `tauri-migration` vs `main`

> Fresh, unaffiliated review (post-MIGRATION_REVIEW3, post-STATUS.md
> rewrite). Findings only — no fixes applied. This file is the only
> artifact.

## TL;DR

The migration has **substantially closed the gaps** identified in
MIGRATION_REVIEW3. The headline regressions — editor open-file state,
LSP bridge, terminal/PTY bridge, `<webview>` external tabs, native
menu, `getChangeScopes` staged/unstaged, `createStream` IPC, per-stream
git scoping — are now **genuinely fixed**, not papered over. STATUS.md
is honest: items it marks ✅ working actually are; items it marks
🟡/❌ deferred actually are deferred. That is a real change vs the
optimistic narrative in MIGRATION_REVIEW2.

What remains is a **smaller, narrower set of issues**, mostly:

1. **Workspace file IPC is still project-wide** (not per-stream) — the
   one corner of the §1.5 finding that the `resolve_repo_dir` fix did
   not reach. `listWorkspaceEntries` / `readWorkspaceFile` /
   `writeWorkspaceFile` etc. resolve `state.layout.project_dir`
   directly, ignoring the `_streamId` the renderer still passes.
2. **`oxplow-tauri-ipc/src/commands/*` is uncovered** — every command
   module shows 0% line coverage in `cargo llvm-cov`. Workspace
   coverage clears the 65% floor (~71.5% lines) only because the
   library crates carry it. Two-test smoke suites for IPC and MCP
   landed (12 + 10 tests respectively), but they barely touch the
   command surface.
3. **`.context/agent-model.md` still references `*.ts` filenames that
   don't exist.** The MIGRATION_REVIEW3 §4 sed sweep was incomplete:
   `filing-enforcement.ts`, `runtime.ts`, `mcp-server.ts` still appear
   throughout `agent-model.md` — the actual files are
   `crates/oxplow-runtime/src/filing.rs`,
   `crates/oxplow-runtime/src/lib.rs`,
   `crates/oxplow-mcp/src/lib.rs`.
4. **No Tauri e2e harness yet.** The Electron suite was honestly
   archived to `tests-e2e.electron-archive/` with a README that names
   three plausible paths forward. Nothing built. App-level renderer
   coverage remains zero.
5. **Auto-update + macOS/Windows code signing remain deferred.** Both
   are tracked in `ideas/signing-and-auto-update.md` per the user's
   note; not blocking review, but worth flagging that a non-CI signing
   step is the gap between "it builds" and "users can install it
   without warnings."

Verified against `git log main..tauri-migration` — 109 commits,
+34,418 / −32,527 LOC; `cargo test --workspace` reports the 268 tests
STATUS.md claims; `cargo llvm-cov --workspace --summary-only` reports
71.53% lines / 56.08% functions / 67.68% regions; the previously
documented stub strings in `apps/desktop/src/api.ts` are gone.

---

## 1. Functionality lost — what's actually fixed vs still broken

This re-checks each item from MIGRATION_REVIEW3 §1.

### 1.1 Editor open-file state — ✅ FIXED
- `apps/desktop/src/editor-session.ts` is now a 207-line **pure data
  module** with the `// Pure data module: tracks open files per stream.
  No IO, no IPC.` header — exactly the shape the prior review
  recommended (port from `main`'s 197-line module).
- `apps/desktop/src/editor-session.test.ts` has 9 unit tests.
- No `not yet ported` throws remain in this file.

### 1.2 LSP bridge — ✅ FIXED
- `crates/oxplow-tauri-ipc/src/commands/lsp.rs` exports
  `open_lsp_client` / `send_lsp_message` / `close_lsp_client` Tauri
  commands.
- `apps/desktop/src/api.ts:118–142` calls `commands.openLspClient(...)`
  etc. directly — the previous "throws not yet ported" stub is gone.
- `lsp:event` channel is wired via `listen("lsp:event", ...)`.
- STATUS.md accurately calls out the echo-server round-trip integration
  test in `oxplow-app::lsp_clients`.

### 1.3 Terminal / PTY bridge — ✅ FIXED
- `crates/oxplow-tauri-ipc/src/commands/terminal.rs` exports
  `open_terminal_session` / `send_terminal_message` /
  `close_terminal_session`.
- `apps/desktop/src/api.ts:143–172` calls them directly; `terminal:event`
  channel is subscribed via `listen("terminal:event", ...)`.
- STATUS.md notes tmux history-mode messages dispatch through
  `oxplow-tmux::copy_mode_*` — verified.

### 1.4 External-URL tabs — ✅ FIXED
- `apps/desktop/src/pages/ExternalUrlPage.tsx` is **no longer a
  `<webview>` element**. It now classifies the URL via
  `classifyExternalUrl`, calls `desktopBridge().openExternalUrl(url)`
  on mount (which routes to the `open_external_url` Tauri command), and
  becomes a status / re-open panel — matching the doc rewrite in
  `.context/external-url-tabs.md`.
- The `external-url` capability targets webview labels
  (`"webviews": ["ext-url-*"]`) — STATUS.md says this is more precise
  than the parent-window pattern; that's correct (the spawned
  WebviewWindowBuilder uses the same label for window and webview, so
  scoping doesn't widen, and matching webviews directly is what the
  permission model expects in Tauri 2).

### 1.5 Per-stream git scoping — ⚠️ MOSTLY FIXED, ONE GAP
- **Git ops**: ✅ fixed. `crates/oxplow-tauri-ipc/src/commands/git.rs:23`
  defines `pub(crate) async fn resolve_repo_dir(state, stream_id)`,
  and every git command (`get_change_scopes`, `git_pull`, `git_push`,
  `git_blame`, `local_blame`, `search_workspace_text`, `git_log`,
  `git_commit_all`, …) takes `Option<String> stream_id` and threads it
  through. Verified: I count 22 call sites in that file invoking
  `resolve_repo_dir(&state, stream_id.as_deref()).await`.
- **Workspace file ops**: ❌ STILL FLATTENED. The renderer still passes
  `_streamId` (e.g. `api.ts:1421` `listWorkspaceEntries(_streamId, …)`,
  `:1436` `readWorkspaceFile(_streamId, …)`, `:1441`
  `writeWorkspaceFile`, `:1457` `createWorkspaceFile`,
  `:1465` `renameWorkspacePath`, `:1474` `deleteWorkspacePath`), but
  the underlying Rust commands in
  `crates/oxplow-tauri-ipc/src/commands/workspace.rs:14, 30, 47, 64`
  unconditionally use `state.layout.project_dir.clone()`. So if a user
  has an active worktree-stream selected and edits a file via the file
  pane, the read/write hits the **primary** worktree, not the active
  one. This is the same regression class as §1.5 in REVIEW3, just
  scoped down to file IO. Less common than git ops but still
  user-visible — for any user who actually uses worktree-per-stream,
  the file pane lies about which checkout it's editing.
- Verification: `grep -n 'state.layout.project_dir' crates/oxplow-tauri-ipc/src/commands/workspace.rs`

### 1.6 Native menu / focus tracking / logUi — ✅ MOSTLY FIXED
- `setNativeMenu` is real: `commands/menu.rs` translates
  `MenuGroupSnapshot[]` to `tauri::menu::Menu`; `menu:command` event
  forwards activations to the renderer (`api.ts:82–93`).
- `logUi` actually ships to the daemon now via `commands.logUi` rather
  than just `console.log` (api.ts:97–117).
- `updateEditorFocus` is **still a no-op** (api.ts:94–96), but STATUS.md
  honestly marks it 🟡 ("the daemon doesn't consume editor focus yet.
  Harmless."). On `main` this drove window title and focus context;
  losing it is a minor regression but not load-bearing.

### 1.7 E2E test suite — ⚠️ HONESTLY ARCHIVED, NOTHING REPLACES IT
- The Electron suite moved to `tests-e2e.electron-archive/` with a
  README naming three Tauri options (tauri-driver+wdio, CDP, hand-rolled
  HTTP). None implemented.
- Result: **app-level click-through coverage is zero**. The
  `oxplow-app` integration tests cover business-logic surface; they do
  not exercise the React tree, so any regression confined to renderer
  state, Monaco wiring, or page chrome ships uncaught.

### 1.8 `createStream` — ✅ FIXED
- `api.ts:576–609` `createStream(...)` now maps the three source modes
  (`existing` / `new` / `worktree`) to `commands.createWorktree(...)`
  and `commands.adoptWorktree(...)`. No more "createStream is
  replaced …" throw.

### 1.9 `getChangeScopes` staged/unstaged — ✅ FIXED
- `api.ts:862–874` returns the bindings shape directly; staged and
  unstaged arrays come from
  `oxplow_git::collect_working_tree_changes` (per STATUS.md). No more
  zeroed-out arrays.

---

## 2. Code quality, organization, duplication

### 2.1 `api.ts` is bigger but no longer "an adapter shim"
- `apps/desktop/src/api.ts` is **1789 lines**, up from 399. The growth
  is real wrappers (one per command, with documented kickoff/awaitDone
  semantics for long-running git ops) rather than a Proxy that throws.
  The file at this size is doing the right thing — it's a typed
  facade — but it's now the heaviest file in the renderer.
- The `notPorted` Proxy is gone (commit `1ffe00b` "phase 3j: drop
  notPorted Proxy"). `desktopBridge()` is a real 13-method facade
  (api.ts:70–189). Missing methods are now compile errors. That's the
  correct move and is a clear improvement over the snapshot the prior
  review saw.
- `api-types.ts` is 455 lines (down from earlier ~1700 reportedly).
  Most types now come from `tauri-bridge/index.js` (i.e. the
  tauri-specta bindings); api-types holds the legacy camelCase shapes
  whose call sites haven't migrated yet. Comments at api.ts:223–232
  enumerate which types are still on the legacy shape — that
  documentation is accurate as far as I checked.

### 2.2 Underscored params: cosmetic at this point
- `_streamId` / `_threadId` count: 33 in api.ts. Mostly legitimate
  wrapper-level signature compatibility (callers still pass the IDs;
  the IPC doesn't need them because the **scope** has been folded into
  another argument or the operation is genuinely project-wide). The
  exceptions are the workspace-file wrappers (§1.5 above), which are
  load-bearing — those callers think they're writing into a stream's
  worktree but aren't.

### 2.3 Crate layout still sound
- 13 crates, balanced. No god-modules (`oxplow-db` 4321 LOC remains
  the largest; nothing pathological).
- Layering rule (`oxplow-domain` pure, infrastructure separate,
  `oxplow-tauri-ipc` thin) holds.

### 2.4 Test counts vs main
- 268 Rust tests (STATUS.md says 268; `cargo test --workspace`
  confirms — sum of per-crate `test result: ok` lines: 54+46+17+3+54+
  7+10+4+29+17+12+3+12+0=268). On `main` the prior review estimated
  ~858 it()/test() calls in the TS suite. Even with multiple-asserts-
  per-test patterns, the rewrite covers far less behavioral surface.
- The rebalance is real but uneven:

  | Crate            | Tests | LOC   | LOC/test |
  |---|---|---|---|
  | oxplow-tauri-ipc | 12    | 2,820 | 235      |
  | oxplow-mcp       | 10    | 1,161 | 116      |
  | oxplow-db        | 46    | 4,321 | 94       |
  | oxplow-tmux      | 3     | 371   | 124      |
  | oxplow-pty       | 4     | 531   | 133      |

  The two thinnest by ratio (`oxplow-tauri-ipc`, `oxplow-mcp`) got
  smoke-test backfills (commit `e7fb8a2`), but `cargo llvm-cov`
  reveals those tests barely move the needle: every command file in
  `oxplow-tauri-ipc/src/commands/` shows **0% line coverage**. Only
  `error.rs` (77%) and `lib.rs` (96%) are exercised. The smoke tests
  hit the IPC error-mapping plumbing, not the actual command bodies.

### 2.5 Coverage: passes the floor, but the floor is loose
- `cargo llvm-cov --workspace --summary-only`: TOTAL 71.53% lines /
  56.08% functions / 67.68% regions.
- The 65% floor in CI passes comfortably, but every IPC command file
  is at 0%. The library crates carry the average.
- Per-crate floors would surface this. STATUS.md's "Still open"
  section already names this as the lever.

### 2.6 Misc cleanup opportunities
- `apps/desktop/src/api.ts:1115–1117`
  `listCurrentlyOpenUsage` accepts `input` but discards it
  (`const _ = input;`) — leftover signature compat. Same pattern at
  `:1178` `getWorkItemSummaries(ids)` returns `[]` unconditionally;
  it's a placeholder.
- `apps/desktop/src/api.ts:638–648` `reorderThread` — comment says
  "Legacy 'single move' call: …; this helper stays for source compat
  but just refetches." That helper exists only to preserve a
  signature; deleting it would force the call sites to `reorderThreads`
  cleanly.
- `apps/desktop/src/api.ts:1339–1349` `getSnapshotSummary` returns
  `null` because the bindings command shape doesn't match
  (`(snapshotId, previousSnapshotId)` vs Rust's
  `(stream_id, limit)`). This is a real missing capability — the
  Local-history pane's empty state is structural, not user-driven.

---

## 3. Tauri 2 best practices

### 3.1 ✅ Capabilities listed explicitly
- `tauri.conf.json` `app.security.capabilities = ["main-window",
  "external-url"]` — closes the directory-auto-enable gap REVIEW3
  flagged.

### 3.2 ✅ Capability targets webview labels
- `external-url.json` uses `"webviews": ["ext-url-*"]` (with a
  thoughtful docstring explaining why webview labels rather than
  window labels). Permissions list is empty; sandbox is real.

### 3.3 ✅ CSP is set, justified
- `default-src 'self'; … style-src 'self' 'unsafe-inline'; …
  connect-src 'self' ipc: http://ipc.localhost`. `unsafe-inline` only
  for styles (Monaco). No `unsafe-eval`, no wildcard scopes.

### 3.4 ✅ Shell allowlist tight
- tmux + git + typescript-language-server only — `shell:default` is
  not used.

### 3.5 ✅ State management
- `Arc<Services>` via `tauri::State`; no top-level `Mutex<Services>`.
- mpsc/broadcast channels rather than `Mutex<HashMap>` for cross-task
  state.

### 3.6 ✅ tauri-specta drift guard in CI
- `.github/workflows/ci.yml:60–70` runs `git status --porcelain
  apps/desktop/src/tauri-bridge/generated` after `cargo test` and
  fails the build on a non-empty diff. This was an open item in
  REVIEW3; it's now closed.

### 3.7 ❌ Auto-update signing key + macOS/Windows code signing
- Still deferred; `ideas/signing-and-auto-update.md` is the
  punch-list. Not blocking for review per the user's note. Worth
  flagging that the CI `Tauri build` job runs unsigned on all three
  OS matrix entries, so installer artifacts are usable for smoke
  testing but not for distribution to users-who-haven't-disabled-
  Gatekeeper.

### 3.8 ❌ No e2e harness
- See §1.7. Not a Tauri-best-practice issue per se, but Tauri 2's
  testing story (tauri-driver, mockruntime) is mature enough that
  *some* harness should exist before the build ships.

---

## 4. `.context/` vs reality

### 4.1 Stale `*.ts` references in `agent-model.md`
- `grep -n 'filing-enforcement.ts\|runtime.ts\|mcp-server.ts' .context/agent-model.md`
  returns 11 hits. None of those files exist. The actual locations
  are:
  - `filing-enforcement.ts` → `crates/oxplow-runtime/src/filing.rs`
  - `runtime.ts` (functions like `buildRefreshedSessionContext`,
    `applyStatusTransition`, `buildBatchAgentPrompt`,
    `buildSessionContextBlock`, `terminalInputIsInterrupt`) →
    `crates/oxplow-runtime/src/lib.rs` and adjacent files
  - `mcp-server.ts` → `crates/oxplow-mcp/src/lib.rs`
- Also lingering: `agent-model.md:411` mentions
  `createElectronPlugin` in `crates/oxplow-app/src/agent_command.rs` —
  the function name should probably be re-read for accuracy (the
  plugin is not "Electron" anymore).
- `editor-and-monaco.md` still says "the native Electron menu (via
  commands.ts → setNativeMenu)" — should be "the native Tauri menu
  (via the desktop bridge → set_native_menu)".
- `usability.md` references `window.prompt()` and "Electron disables
  it"; that's still true under Tauri's webview but the framing should
  switch.
- `ipc-and-stores.md` mentions `ipcRenderer` and "Electron's default
  `MaxListeners=10`" — those are dead concepts under Tauri.

### 4.2 No references to phantom subsystems
- The previously phantom `crates/oxplow-app/src/external-content-lockdown.ts`
  / `external-content-policy.ts` references in REVIEW3 §4 are gone.
- `find crates -name '*.ts'` returns nothing, and
  `grep -rE 'crates/[^.]+\.ts\b' .context/*.md` returns nothing.
- So the **path-based** references have been cleaned. What's left is
  **function-name** references that no longer match the new file
  layout (above §4.1).

---

## 5. Test coverage detail

### 5.1 Numbers
- `cargo test --workspace`: 268 passed, 0 failed.
- `cargo llvm-cov --workspace --summary-only`: 71.53% lines
  (TOTAL row).
- CI floor: 65% lines. Headroom: ~6 points.

### 5.2 Where the 0%-coverage zones are
Every file in `crates/oxplow-tauri-ipc/src/commands/` shows 0% lines:

```
agent_panes.rs       0%   /  88 lines
app.rs               0%   /  32 lines
background.rs        0%   /  55 lines
backlog.rs           0%   /  15 lines
branch.rs            0%   / 109 lines
code_quality.rs      0%   /  75 lines
config.rs            0%   /  99 lines
git.rs               0%   / 510 lines  (largest IPC module — uncovered)
hooks.rs             0%   /  56 lines
log.rs               0%   /  72 lines
lsp.rs               0%   /  60 lines
menu.rs              0%   /  95 lines
notes.rs             0%   /  54 lines
page_visit.rs        0%   / 112 lines
snapshot.rs          0%   / 116 lines
streams.rs           0%   / 218 lines
terminal.rs          0%   /  50 lines
threads.rs           0%   / 252 lines
usage.rs             0%   /  21 lines
webview.rs           0%   /  41 lines
wiki.rs              0%   /  96 lines
work_items.rs        0%   / 121 lines
workspace.rs         0%   / 245 lines
```

These are `#[tauri::command]` adapters — thin by design, but they're
still where `state` plumbing and argument-shape errors land. They
need an integration harness (a Tauri `MockRuntime` or a thin
`AppState`-builder that constructs Services without spawning the
shell) to be exercised. Right now, the only thing that catches a
panicking `state.unwrap()` is at runtime in dev.

### 5.3 `oxplow-mcp` is also under-tested for its size
- 10 tests / 1,161 LOC. The smoke suite covers
  ping/app_version/list_streams/list_backlog/get/upsert/
  delete_work_item/list_notes per STATUS.md. The other 30+ MCP tools
  (subsystem docs, code-quality scans, blame, hook events, snapshots,
  wiki refs, etc.) are unexercised.

### 5.4 `oxplow-tmux` and `oxplow-pty` are subprocess-heavy
- 3 / 4 tests respectively. STATUS.md correctly notes these are
  exercised through `oxplow-app` integration paths. Unit-testing them
  in isolation hits portability problems (Windows ConPTY in
  particular). Reasonable trade.

---

## 6. Strengths

- **Honest STATUS.md.** Replaces MIGRATION_REVIEW2's "functional parity
  achieved" with a feature matrix; rows actually reflect what works.
  Items marked deferred are deferred. Items marked working *are*
  working.
- **Per-stream git scoping is real now**: `resolve_repo_dir` pattern
  is consistent across all 22 git commands; reads the active stream's
  worktree from `SqliteStreamStore::list`.
- **The renderer-side cleanup work in waves 4a–4m is substantive,
  not cosmetic**: dropping the `notPorted` Proxy in favor of a typed
  13-method `DesktopBridge` facade means the TypeScript compiler
  catches missing IPC methods, not the user at runtime. That's the
  right end-state.
- **Long-running git ops have proper `BackgroundTask` plumbing**
  (api.ts:533–557 `runAsBackgroundTask`). `awaitDone` resolves with
  the actual `GitOpResult`; subscribers see a real
  `background-task.changed` event. This is more correct than `main`'s
  fire-and-forget pattern in some cases.
- **Capabilities + CSP + drift guard + coverage floor in CI** form
  a credible quality wall. CI is doing real work.
- **`editor-session.ts` is a clean port** — pure data, 9 tests, no
  IO. Exactly the shape the prior review wanted.
- **Crate boundaries hold up**: `oxplow-domain` is genuinely
  IO-free; `oxplow-tauri-ipc` is genuinely thin; `oxplow-app`
  orchestrates without becoming a god-module.
- **Honest e2e archive**: `tests-e2e.electron-archive/README.md`
  doesn't pretend the archived suite works. It names three Tauri
  options and admits zero are built. Better than silently-broken.

---

## 7. Recommendations (prioritized by user-impact / effort)

1. **Thread `stream_id` through workspace file commands.** Mirror the
   git pattern: `resolve_repo_dir(state, stream_id)` for
   `list_workspace_entries`, `list_workspace_files`,
   `read_workspace_file`, `write_workspace_file`,
   `create_workspace_file`, `create_workspace_directory`,
   `rename_workspace_path`, `delete_workspace_path`. The renderer
   already passes `_streamId`; rename and use it. **~1–2 hours.**
   Closes the only load-bearing item in this review.

2. **Add an integration harness for `oxplow-tauri-ipc` commands.**
   Either `tauri::test::mock_app()` with a constructed `AppState`
   over an in-memory DB, or a thin builder in
   `crates/oxplow-tauri-ipc/tests/` that exercises a representative
   command per module. Goal: take the 0%-coverage rows above to
   ≥40% so a `state.unwrap()` regression fails CI rather than at
   runtime. **Several days; biggest coverage lever available.**

3. **Backfill `.context/agent-model.md` filename references.**
   Rename to `crates/oxplow-runtime/src/filing.rs`,
   `crates/oxplow-runtime/src/lib.rs`, `crates/oxplow-mcp/src/lib.rs`
   etc. as listed in §4.1 above. Drop "Electron" from
   `editor-and-monaco.md`, `usability.md`, `ipc-and-stores.md`.
   **~30 min.**

4. **Pick an e2e path and build the smallest version of it.**
   Tauri-driver + WebdriverIO is officially supported and probably
   the right answer; an MVP that re-implements the
   `dogfood-probe.ts` "boot, see a stream list, see a tab" test
   would re-establish app-level smoke coverage. **Days, not weeks,
   for an MVP.**

5. **Implement `getSnapshotSummary` / hide the local-history pane.**
   `api.ts:1339–1349` returns `null` because the Rust command
   shape doesn't match. Either add a single-snapshot lookup to
   `oxplow-snapshot` (or wherever) and migrate, or hide the pane
   until the data flows. The current state is a structural empty
   state with no user-visible signal.

6. **Per-crate coverage floors.** A single number (65%) lets a
   regression in `oxplow-tauri-ipc` (0%) hide behind a high number
   in `oxplow-git` (~80%). Floors per crate (e.g. `oxplow-mcp` ≥40%,
   `oxplow-tauri-ipc` ≥30%, others ≥60%) would surface the gaps
   directly. **~1 hour CI config.**

7. **Address auto-update signing.** Per the user's note this is
   tracked in `ideas/signing-and-auto-update.md`. Worth doing the
   key generation + secret wiring before the first user-facing
   release; the gap between "CI builds" and "users install without
   warnings" is small in setup but unbounded in support cost
   afterwards. Not in scope for this review.

8. **Delete dead source-compat helpers in `api.ts`.** `reorderThread`
   (just refetches), `listCurrentlyOpenUsage` (discards `input`),
   `getWorkItemSummaries` (returns `[]`). Each is small;
   collectively they remove a handful of lies from the TypeScript
   surface.

---

## 8. Verification

All findings above reproducible from
`/Users/nvoxland/src/nvoxland/oxplow`:

```sh
# Diff size vs main
git log main..HEAD --oneline | wc -l                 # 109
git diff main..HEAD --stat | tail -5                  # 444 files; +34418/-32527

# §1.1 — editor-session is real
wc -l apps/desktop/src/editor-session.ts             # 207
wc -l apps/desktop/src/editor-session.test.ts        # 125
head -3 apps/desktop/src/editor-session.ts            # "Pure data module"

# §1.2 / §1.3 — LSP + terminal IPC exists
grep -n 'pub async fn' crates/oxplow-tauri-ipc/src/commands/lsp.rs
grep -n 'pub async fn' crates/oxplow-tauri-ipc/src/commands/terminal.rs

# §1.4 — ExternalUrlPage no longer uses <webview>
grep -c '<webview' apps/desktop/src/pages/ExternalUrlPage.tsx   # 0
grep -n 'openExternalUrl' apps/desktop/src/pages/ExternalUrlPage.tsx

# §1.5 — git scoping fixed; workspace scoping NOT fixed
grep -c 'resolve_repo_dir' crates/oxplow-tauri-ipc/src/commands/git.rs  # >20
grep -n 'state.layout.project_dir' crates/oxplow-tauri-ipc/src/commands/workspace.rs  # 8 hits

# §1.6 — native menu / logUi real, updateEditorFocus stub
grep -n 'commands.setNativeMenu\|commands.logUi\|updateEditorFocus' apps/desktop/src/api.ts | head

# §1.7 — e2e archive
ls tests-e2e.electron-archive/ | wc -l                # 35
head -10 tests-e2e.electron-archive/README.md

# Stub strings gone from api.ts
grep -nE 'not yet ported|not yet wired' apps/desktop/src/api.ts        # no matches

# §3.1 — capabilities listed explicitly
grep capabilities apps/desktop/src-tauri/tauri.conf.json
# "capabilities": ["main-window", "external-url"]

# §4.1 — stale function references in .context/
grep -nE 'filing-enforcement\.ts|runtime\.ts|mcp-server\.ts' .context/agent-model.md | wc -l  # 11

# §5 — tests + coverage
cargo test --workspace 2>&1 | grep 'test result' | awk '{s+=$4} END {print s}'   # 268
cargo llvm-cov --workspace --summary-only 2>&1 | grep '^TOTAL'
# TOTAL  21966  6254  71.53% …  56.08% …  67.68% …

# Per-IPC-command coverage rows
cargo llvm-cov --workspace --summary-only 2>&1 | grep 'tauri-ipc/src/commands/' | awk '{print $1, $4}'
```
