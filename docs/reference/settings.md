# Settings

Oxplow stores per-project state in `.oxplow/state.sqlite` and a
small amount of per-user state in your OS user-config directory.
Most settings are toggles in the UI; almost nothing requires
editing a file.

## Where state lives

- **Per-project** — everything that matters lives under
  `.oxplow/` inside the project root:

    ```
    .oxplow/
      state.sqlite        # work items, threads, snapshots, settings
      wiki/               # wiki pages (markdown)
      snapshots/          # per-effort file snapshots (Local History)
      runtime/            # Claude Code plugin oxplow installs per project
      lsp/                # cached LSP server binaries (Mason packages)
    ```

    Add `.oxplow/` to `.gitignore`; oxplow does not commit
    project state.

- **Per-user** — the Tauri app's data dir under your OS config
  location: `~/Library/Application Support/oxplow/` (macOS),
  `%APPDATA%/oxplow/` (Windows), or `~/.config/oxplow/` (Linux).
  Window position, recent projects, theme preference. Wipe
  freely — it rebuilds.

There is no global config file you need to edit to get started.
Sensible defaults; opinionated product.

## Settings worth knowing

### Writer thread

Per-stream. Exactly one thread is the writer. Other threads are
read-only. Switch the writer from the thread tab kebab.
Switching kicks any in-flight write attempt on the old writer
back through the hook (which fails it cleanly).

### Stream and thread custom prompts

Each stream and thread has its own settings page (open from the
tab kebab → Settings) with a custom prompt field appended to the
agent's system prompt at launch. Use it for stream-specific
framing ("you're on the migration branch, priority is not
breaking schema") or thread-specific framing ("research only —
never edit").

### Agent kind

Per-thread. Default is Claude Code. `copilot` is also supported
but skips the oxplow plugin plumbing — no filing-enforcement, no
Stop directives, no MCP tools.

### tmux mode

Per-thread. Default on. The agent process runs inside a tmux
session so it survives oxplow restarts. Toggle from the agent
tab kebab. The tmux session name is requested when you switch a
thread into tmux mode.

### Snapshot retention

Snapshots from closed work items are pruned on a configurable
schedule. Tune from the project's settings page if defaults
don't fit (most users never touch this).

### LSP servers

Auto-managed. Oxplow's bundled LSP installer fetches Mason
packages on first use, caches them under `.oxplow/lsp/`, and the
proxy hands the right binary to whichever stream asked. There is
no `lsp.json` to maintain — supported languages are
auto-detected from project content (file extensions, root
markers).

If a server you need isn't yet supported, file a work item; the
installer's manifest lives in
`crates/oxplow-lsp/src/installer/`.

### Theme

Dark only. Oxplow is dark-only on purpose — Monaco is pinned to
`vs-dark` and the UI tokens are calibrated for it.

### Telemetry

Off. Always. There is no telemetry to configure.

## Settings the agent can change

None. The MCP surface deliberately does not expose product
settings — the agent operates on intent, files, work items, and
wiki pages. Configuration is the human's job.

## Resetting

Wipe `.oxplow/` to reset a project. Wipe the user-config
directory to reset everything. Both are safe; oxplow rebuilds
what it needs on next launch (your work-item history goes with
the project state, though, so don't do it casually).
