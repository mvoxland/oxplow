# Tauri-migration status

Honest, reviewable feature matrix for the Electron → Tauri 2 + Rust
backend migration. Replaces the optimistic narrative in
`MIGRATION_REVIEW2.md` (gitignored) with a checklist of what works,
what's stubbed, and what's gone. Update this file alongside any
change that flips a row.

## Backend (Rust crates)

| Subsystem | State | Notes |
|---|---|---|
| Stream / thread / work-item lifecycle | ✅ working | `oxplow-app` orchestration, `oxplow-db` stores, full CRUD via Tauri commands. |
| Git ops (sync/refs/blame/branch changes/scopes/search) | ✅ working | `oxplow-git`. Per-stream worktree resolution wired through every git command via `resolve_repo_dir`. |
| Snapshots + content-addressed blob store | ✅ working | `crates/oxplow-app/src/blob_store.rs`; `restore_file_from_snapshot` end-to-end. |
| Hook event ingest + agent-turn lifecycle | ✅ working | `oxplow-runtime`. |
| Agent panes (tmux orchestration) | ✅ working | `oxplow-tmux` with copy-mode helpers. |
| Code-quality scans | ✅ working | lizard + jscpd subprocess; findings store. |
| LSP session manager + 4 LSP MCP tools | ✅ working | `oxplow-lsp`; `oxplow-app::lsp_sessions`. |
| Daemon recovery on boot | ✅ working | `oxplow-app::recovery`. |
| MCP server | ✅ working | 38 tools via rmcp. |
| Tauri command surface | ✅ working | 156 commands across `crates/oxplow-tauri-ipc/src/commands/`. |

## Renderer (Tauri frontend)

| Subsystem | State | Notes |
|---|---|---|
| File-session state (open files / dirty tabs / LRU) | ✅ working | `apps/desktop/src/editor-session.ts` — restored from `main` with 9 unit tests. |
| Editor pane (Monaco + LSP markers + blame) | ✅ working | Reads bindings shapes directly: `BlameLine.author_time`, `LocalBlameEntry.git`. |
| Terminal pane (xterm + tmux attach) | ✅ working | `open_terminal_session` / `send_terminal_message` / `close_terminal_session` Tauri commands; `terminal:event` channel. Tmux history-mode messages dispatch through `oxplow-tmux::copy_mode_*`. |
| LSP bridge (per-language client) | ✅ working | `open_lsp_client` / `send_lsp_message` / `close_lsp_client` Tauri commands; `lsp:event` channel. Echo-server round-trip test in `oxplow-app::lsp_clients`. |
| Native menu (macOS/Windows) | ✅ working | `set_native_menu` translates `MenuGroupSnapshot[]` → `tauri::menu::Menu`; `menu:command` event re-emits activations to renderer. |
| External-URL tabs | ✅ working | `WebviewWindow` spawn via `open_external_url`; sandboxed by the `external-url` capability with **zero** oxplow commands and zero plugin permissions. |
| `getChangeScopes` staged/unstaged | ✅ working | `oxplow-git::collect_working_tree_changes` populates both arrays from `git status --porcelain`. |
| `createStream` / new-stream form | ✅ working | Maps "existing" / "new" source modes to `create_worktree`. Worktree-adoption mode (mode "worktree") still throws — no Rust counterpart yet. |
| Per-stream git scoping | ✅ working | All 22 git/log commands accept `Option<String> stream_id` and resolve the active worktree via `SqliteStreamStore::list`. |
| Editor focus tracking | 🟡 no-op | The renderer pushes editor focus to `desktopBridge().updateEditorFocus`; the bridge currently swallows it because the daemon doesn't consume editor focus yet. Harmless. |
| `legacy-bridge.ts` / `legacy-*` filenames | ✅ gone | All renamed (`api-types.ts`, `editor-session.ts`); `window.oxplowApi` global eliminated. |
| `buildDesktopAdapter` Proxy + `notPorted` runtime | ✅ gone | Replaced by a 13-method typed `DesktopBridge` facade. Missing methods are now compile errors, not deferred runtime crashes. |

## Tooling / packaging

| Item | State | Notes |
|---|---|---|
| `tauri-specta` v2 binding generation | ✅ working | `cargo test -p oxplow-tauri-ipc` regenerates `apps/desktop/src/tauri-bridge/generated/bindings.ts`. |
| Bindings drift guard in CI | ✅ working | `.github/workflows/ci.yml` "Verify generated TS bindings are up to date" step fails the PR on a non-empty `git diff` after regeneration. |
| `cargo-llvm-cov` workspace coverage in CI | ✅ working | Floor: 65% lines; current baseline ~70.7% lines / 66.6% regions / 54.7% functions. |
| Capabilities listed explicitly in `tauri.conf.json` | ✅ working | `app.security.capabilities = ["main-window", "external-url"]`. |
| External-URL capability targets webview labels | ✅ working | `external-url.json` uses `webviews: ["ext-url-*"]` (more precise than the parent-window label pattern). |
| `shell:default` replaced with allowlist | ✅ working | tmux + git + typescript-language-server in `main-window.json`. |
| CSP set in `tauri.conf.json` | ✅ working | `unsafe-inline` retained for styles only (Monaco needs it). |
| Auto-update signing key | ❌ deferred | Operational; needs cert generation + secret wiring. Blocks shipping signed updates. |
| macOS / Windows code signing | ❌ deferred | Same — needs Apple Developer cert + Windows EV cert + CI secret integration. |
| `oxplow-config` preserves user comments on write | 🟡 partial | Unknown top-level keys are preserved through writes; YAML comments are not (no comment-aware Rust YAML crate). Documented in the `write_project_config` docstring. |

## Test counts (per `cargo test`)

| Crate | Tests | LOC | LOC/test |
|---|---|---|---|
| `oxplow-git` | 54 | 2,925 | 54 |
| `oxplow-app` | 54 | 3,286 | 61 |
| `oxplow-db` | 46 | 4,321 | 94 |
| `oxplow-runtime` | 29 | 1,051 | 36 |
| `oxplow-session` | 17 | 785 | 46 |
| `oxplow-domain` | 17 | 822 | 48 |
| `oxplow-config` | 12 | 514 | 43 |
| `oxplow-lsp` | 7 | 542 | 77 |
| `oxplow-pty` | 4 | 531 | 133 |
| `oxplow-tmux` | 3 | 371 | 124 |
| `oxplow-fs-watch` | 3 | 178 | 59 |
| `oxplow-tauri-ipc` | 2 | 2,820 | 1,410 |
| `oxplow-mcp` | 2 | 1,161 | 580 |

Total: 250 tests. The thinnest crates (`oxplow-tauri-ipc`,
`oxplow-mcp`, `oxplow-tmux`, `oxplow-pty`) are the obvious backfill
targets — see "still open" below.

## Frontend tests

`apps/desktop/src/editor-session.test.ts` — 9 unit tests for the
file-session state machine. Other renderer tests are colocated
`*.test.ts` files exercised via `bun test`. End-to-end: the
Electron-era Playwright suite at `tests-e2e.electron-archive/`
**does not** work against the Tauri build (see that directory's
README). No Tauri e2e harness exists yet.

## Still open (deferred, not closing in this session)

- **Auto-update signing key + macOS/Windows code signing.** Blocks
  shipping signed installers. Operational, not code.
- **Tauri e2e harness.** Three plausible paths (tauri-driver +
  WebdriverIO, CDP-into-webview via Playwright, hand-rolled HTTP
  probes). None built; see `tests-e2e.electron-archive/README.md`.
- **Backfill `oxplow-tauri-ipc` and `oxplow-mcp` tests.** Both are
  ~1,000–3,000 LOC with 2 tests each. The CI floor (65% workspace
  lines) doesn't trigger on per-crate gaps; raising the floor as
  these get backfilled is the lever.
- **Worktree-adoption mode for `createStream`.** The "worktree"
  source path in `apps/desktop/src/api.ts::createStream` still
  throws — no Rust counterpart for adopting an existing on-disk
  worktree into oxplow's tracking.

## How to verify

```sh
# Rust
cargo test --workspace
# expected: 250 passed, 0 failed

cargo llvm-cov --workspace --summary-only --fail-under-lines 65
# expected: passes; current baseline ~70.7%

# Frontend
cd apps/desktop && bun run typecheck
# expected: clean exit

bun test apps/desktop/src/editor-session.test.ts
# expected: 9 pass

# Bridge wiring
cargo check -p oxplow-desktop
# expected: clean

# No "not yet ported" stubs in api.ts
grep -nE 'not yet ported|not yet wired' apps/desktop/src/api.ts
# expected: no matches
```
