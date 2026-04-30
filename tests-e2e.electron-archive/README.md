# Archived Electron e2e suite

These 35 Playwright probes exercised the original Electron build of
oxplow via `playwright._electron.launch(...)`. They do **not** work
against the Tauri 2 build that replaces it: Playwright has no first-
party Tauri driver, the `<webview>` tag flows are different, and
several probes pass arguments (`--user-data-dir`, `--project`) that
the Tauri shell doesn't accept.

The directory is preserved under `tests-e2e.electron-archive/` so
that:
- The hand-written probe steps (page selectors, assertions, fixture
  flows) are still readable as a behavior corpus when porting.
- `git blame` history on the probes is not lost.

## Path forward

Tauri 2 e2e options:

1. **`tauri-driver` + WebdriverIO** — Tauri's official approach.
   Spawns the bundled binary, exposes a WebDriver session via WRY's
   embedded webview. Probe rewriting required (Playwright API → wdio).
2. **CDP into a dev-build webview** — Tauri's WebView can be opened
   with remote debugging in dev mode; Playwright can `chromium
   .connectOverCDP(url)` to it. Less mature, macOS-only WebKit issues.
3. **Hand-rolled HTTP probe harness** — for headless smoke tests
   only; would not exercise the React UI.

None of those have been built yet. Until one is, app-level coverage
is **zero** and unit/integration tests in `crates/*/` carry the
weight.

## Removing this directory

If/when a working Tauri e2e harness exists at a new path
(`tests-e2e/` or similar), this archive can be deleted. Do not delete
it before the new harness exists.
