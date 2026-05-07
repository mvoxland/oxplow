# Install

Oxplow is a desktop app — a Tauri 2 shell wrapping a React /
Monaco / xterm frontend over a Rust backend. There are two paths:
grab a prebuilt installer from the latest CI run, or build from
source.

!!! note
    Oxplow is early. Builds are unsigned. APIs and on-disk shapes
    change between releases. If that's a problem for your setup,
    wait — or pin to a specific commit.

## Option 1: prebuilt installer

CI produces installers for every push to `main`, and tagged
releases (`v<version>`) attach the same bundles to a GitHub
Release.

1. Open the
   [latest successful run](https://github.com/nvoxland/oxplow/actions)
   on the `main` branch (or pick a [release](https://github.com/nvoxland/oxplow/releases)).
2. Scroll to **Artifacts** (CI) or the asset list (releases).
3. Download the bundle for your platform:
    - macOS → `.dmg` (or `.app.tar.gz`)
    - Windows → `.msi` or `.exe` installer
    - Linux → `.deb` or `.AppImage`
4. Install / open it like any other app.

On macOS the first launch is blocked because the build is
unsigned. Right-click the `.app`, choose **Open**, then confirm in
the dialog. After the first launch macOS remembers your choice.
Windows shows a SmartScreen warning — click **More info → Run
anyway**.

## Option 2: build from source

Prerequisites:

- **Bun** ≥ 1.3.9 — package manager + JS runtime.
- **Rust stable** (≥ 1.80) — `rust-toolchain.toml` pins it; rustup
  installs the right toolchain automatically.
- **Platform Tauri deps**:
    - macOS: `xcode-select --install`
    - Linux: `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`,
      `librsvg2-dev`, `patchelf`, `build-essential`
    - Windows: WebView2 (preinstalled on modern Windows; otherwise
      Microsoft's redistributable) plus the MSVC build tools
- **Git** — oxplow expects the workspace root to be a repo.
- **`tmux`** — agent panes are tmux-managed (the suite skips when
  it's missing, but real use needs it).

If you use [mise](https://mise.jdx.dev), `mise install` picks up
bun / node / rust from `mise.toml`.

```bash
git clone https://github.com/nvoxland/oxplow
cd oxplow
bun install --frozen-lockfile     # frontend deps (React, Monaco, xterm)
bun run tauri:dev                 # builds the Rust shell + boots Vite
```

For everything else — split-process dev, packaged builds, the
release flow — see [`DEV.md`](https://github.com/nvoxland/oxplow/blob/main/DEV.md).

## After install

1. Launch oxplow from the directory you want to work in
   (`./bin/oxplow` from the repo if you built from source — it
   treats the current working directory as the project root).
2. The opened directory **is** the workspace. Oxplow does not
   climb upward looking for an enclosing repo (workspace
   isolation rule).
3. Read [Your first stream](first-stream.md) to send a prompt.

Oxplow stores everything project-local under `.oxplow/` inside
the project root: the SQLite database, the wiki pages folder,
per-effort snapshots, the Claude Code plugin oxplow installs, and
the LSP server cache. Worktrees for non-primary streams live as
siblings of the project root. There is no global state to
configure.
