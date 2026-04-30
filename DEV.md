# Developing Oxplow

> **Post-Tauri rewrite.** Oxplow is now a Tauri 2 desktop app with a
> Rust backend. The previous Electron/Node shell is gone.

## Prerequisites

- **Bun 1.3.9** and **Node 22.13.1** (frontend toolchain).
- **Rust stable (≥ 1.80)** — `rust-toolchain.toml` pins it; rustup
  installs the right version automatically.
- **Platform Tauri deps**:
  - macOS: `xcode-select --install` (Xcode CLT).
  - Linux: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev
    librsvg2-dev patchelf build-essential`.
  - Windows: WebView2 (preinstalled on modern Windows; Microsoft
    redistributable otherwise) + MSVC build tools.
- **Git** — oxplow's git features expect the workspace root to be a
  repo.
- **`tmux`** — the agent panes are tmux-managed. Optional for tests
  (the runtime tmux suite skips when tmux isn't on PATH).

If you use [mise](https://mise.jdx.dev/), `mise install` picks up
bun/node/rust from `mise.toml`.

## Install

```
bun install --frozen-lockfile
```

This installs only the frontend deps (React, Monaco, xterm,
@tauri-apps/api). Cargo handles Rust deps lazily on first build.

## Run from source

```
./bin/oxplow
```

The launcher prefers `target/release/oxplow` if present, then
`target/debug/oxplow`, then falls back to `cargo tauri dev` from
`apps/desktop/` for the dev loop.

In dev mode Tauri spawns Vite on `http://localhost:5173` and reloads
the frontend on save. Rust changes require a rebuild — `cargo tauri
dev` rebuilds the shell automatically; for crate-internal iteration,
`cargo build --workspace` while the app is running and then restart.

`bin/oxplow` treats the current working directory as the project root.
Oxplow's workspace isolation rule (see
[.context/architecture.md](./.context/architecture.md)) keeps it
from climbing into a parent repo.

## Test

```
bun run test     # runs both Rust and TS suites
cargo test --workspace
bun run --cwd apps/desktop test
```

`cargo test --workspace` is the Rust suite (≈100 tests across the
backend crates). It also regenerates `apps/desktop/src/tauri-bridge/
generated/bindings.ts` via the `oxplow-tauri-ipc` `export_ts_bindings`
test — CI fails if `git diff` of that file is non-empty after the
test run.

Frontend tests still use `bun test` (run from `apps/desktop/`).
The original Electron-era Playwright suite lives under
`tests-e2e.electron-archive/` and **does not** work against the
Tauri build; see that directory's README for the path forward.
There is no current Tauri e2e harness.

### Coverage

CI runs `cargo llvm-cov --workspace --summary-only` on every PR and
uploads an `lcov.info` artifact. To reproduce locally:

```
cargo install cargo-llvm-cov   # one-time
cargo llvm-cov --workspace --summary-only   # per-crate line %
cargo llvm-cov --html --workspace           # HTML report under target/llvm-cov/html
```

No coverage floor is gated yet — the goal is to keep the numbers
visible so the thinnest crates (`oxplow-mcp`, `oxplow-tauri-ipc`,
`oxplow-pty`, `oxplow-tmux`) get backfill before regressions creep
in.

## Build installers

```
bun run tauri:build
```

Runs Vite + cargo to produce platform installers in
`target/release/bundle/`:

- macOS: `.dmg` + `.app.tar.gz` (arm64 and x64)
- Windows: `.msi` / `.exe`
- Linux: `.deb` + `.AppImage`

Builds are unsigned in CI. Add signing certs by setting Tauri's
standard signing env vars; see Tauri docs for `TAURI_PRIVATE_KEY` and
the per-platform keychain integration.

## Documentation site

User-facing docs live under `docs/` and are built with MkDocs
Material — unchanged from the pre-rewrite setup.

Prereqs: Python 3.11+ and [Poetry](https://python-poetry.org/) 2.x.

```
poetry install --with docs
poetry run mkdocs serve         # live preview at http://localhost:8000
poetry run mkdocs build --strict  # one-shot build into site/
```

## CI

`.github/workflows/ci.yml`:

1. **test** (ubuntu-latest) — `bun install`, `bun run typecheck`,
   `cargo test --workspace`, ts-bindings drift guard, `cargo fmt
   --check`, `cargo clippy -- -D warnings`.
2. **package** (matrix: ubuntu, macOS, Windows) — `bun run
   tauri:build`, uploads installer artifacts.

Cargo registry + target dir cached per OS, keyed on `Cargo.lock`.

## Codebase map

- `apps/desktop/` — Tauri 2 desktop product (frontend + shell).
- `apps/desktop/src/` — frontend TS (React/Monaco/xterm).
- `apps/desktop/src/tauri-bridge/` — typed facade over
  `@tauri-apps/api`; UI imports from here, not `@tauri-apps/api`
  directly.
- `apps/desktop/src-tauri/` — Tauri shell crate, `tauri.conf.json`,
  `capabilities/`.
- `crates/` — reusable Rust libraries:
  - `oxplow-domain` — pure types + store traits.
  - `oxplow-db` — rusqlite stores + migrations.
  - `oxplow-config` — YAML config load/validate.
  - `oxplow-fs-watch` — debounced notify wrapper.
  - `oxplow-git` — repo detection, branches, worktrees, conflict state.
  - `oxplow-session` — stream + worktree lifecycle.
  - `oxplow-runtime` — write guard + filing enforcement.
  - `oxplow-tmux` — tmux command wrapper.
  - `oxplow-pty` — owner-task PTY manager (portable-pty).
  - `oxplow-lsp` — JSON-RPC stdio proxy.
  - `oxplow-mcp` — MCP server (rmcp).
  - `oxplow-app` — Services orchestration.
  - `oxplow-tauri-ipc` — `#[tauri::command]` adapters + tauri-specta
    TS-binding export.

Subsystem docs live under [`.context/`](./.context/). Path
references inside point at the current Rust crate layout.

## Capability schema files

`apps/desktop/src-tauri/capabilities/` references
`gen/schemas/<platform>-schema.json` so editors (VS Code,
JetBrains) autocomplete permission identifiers. Those schemas are
regenerated by `tauri-build` on every `cargo build` and are
gitignored. On a fresh clone, your IDE will report
`unresolved $schema` on the capability files until you run
`cargo build` once.

## Conventions

- **Commit messages**: subject line, blank line, bullet list. Never
  `--amend` a shipped commit.
- **Tests**: real DB (`Database::in_memory()` or tempfile-backed),
  real SQLite, no mocking.
- **Work items as durable records**: every Edit/Write to project
  files needs a tracked work item. See CLAUDE.md for filing rules.
