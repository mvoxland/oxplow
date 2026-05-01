# Tauri-migration parity backlog

Items the Tauri rewrite still owes parity with the Electron build.
Each one has a short rationale + acceptance criterion so they can be
filed straight into the Work panel via the UI (or via `mcp__oxplow__*`
tools from a session that can reach the new MCP) without re-deriving
context.

Delete entries from this file as they're filed, so the file shrinks
to empty as parity lands.

---

## Plugin / control-plane wiring (rust impl exists, not called)

Three rust modules in `crates/oxplow-runtime/` are direct ports of
the corresponding `src/electron/*.ts` files but the new
`oxplow-control-plane` axum handler never invokes them. Wiring them
into the `POST /hook/{event}` path lights them up.

### 1. Wire write_guard into PreToolUse — **high**

`oxplow-runtime/src/write_guard.rs` is a port of
`src/electron/write-guard.ts`. The control plane's PreToolUse handler
ingests the event but never calls `WriteGuard::evaluate` to short-
circuit with `{ permissionDecision: "deny", reason }`. Effect: read-
only threads can still mutate the worktree.

**AC:** PreToolUse on a non-writer thread returns deny for
Edit/Write/MultiEdit/NotebookEdit; `.oxplow/notes/<slug>.md` paths
still pass through; writer threads see no behavior change; control-
plane integration test covers the deny payload.

### 2. Wire filing_enforcement into PreToolUse — **high**

`oxplow-runtime/src/filing.rs` is a port of
`src/electron/filing-enforcement.ts`. Same problem as write-guard —
exists but unwired. Wire it after write-guard so the order matches
main: read-only deny first, then filing deny if the writer thread
has no `in_progress` item to claim the change.

**AC:** PreToolUse denies Edit/Write/MultiEdit/NotebookEdit on the
writer thread when no `in_progress` work item exists; allows when one
does; exempts edits during a git operation
(MERGE_HEAD/REBASE_HEAD/CHERRY_PICK_HEAD/REVERT_HEAD); ports the
existing TS test suite.

### 3. Wire stop_hook decideStopDirective into Stop — **high**

`oxplow-runtime/src/stop_hook.rs` is a port of
`src/electron/stop-hook-pipeline.ts`. Stop handler currently just
ACKs and lets `HookIngestService` close the turn — no directive
returned, so the runtime no longer nudges about unaudited
in_progress items, surfaces the `/work-next` prompt, etc.

**AC:** Stop response includes `additionalContext` when an
`in_progress` item went unaudited this turn; suppresses the nudge
when `add_work_note` was called during the turn; surfaces `/work-next`
when no in_progress remain and ready queue is non-empty; behavioral
parity with `decideStopDirective` covered by ports of the existing
test suite.

### 4. Restart MCP / control plane on dev source change — **medium**

On main, the runtime watched `src/mcp/**` and respawned the MCP
server on source change so dogfooding picked up tool-surface edits
without an app restart. The Tauri control plane has no equivalent.

**AC:** Editing `crates/oxplow-mcp/**` or `crates/oxplow-control-plane/**`
during `tauri:dev` makes the new tool surface available without
restarting the Tauri shell; agent reconnects to the new MCP transport
on next call.

### 5. Resume-tracker: capture session_id from hooks — **high**

`src/session/resume-tracker.ts` on main listens for the session_id
on any incoming hook and persists it as the thread's
`resume_session_id` (Claude Code drops HTTP hooks for SessionStart,
so we have to learn it from whichever hook fires next). Without
this, every re-attach to a thread starts a fresh Claude session and
loses prior context — `--resume` in `agent_command` is wired but
nothing populates the column. Likely lives best inside
`HookIngestService::ingest`.

**AC:** Any hook with a `session_id` field updates the thread's
`resume_session_id`; subsequent `open_terminal_session` calls
include `--resume <session_id>` and Claude actually resumes;
agent_pane recovery still works after a daemon restart.

---

## MCP tool gaps (vs main's 35 tools)

`oxplow-mcp` is missing several tools that exist on main. Most
notably the new `/work-next` slash command we just shipped depends
on `read_work_options`, which doesn't exist on the Rust side yet.

### 6. `read_work_options` MCP tool — **high (blocks /work-next)**

Returns the next dispatch unit for the orchestrator. If the highest-
priority ready item is an epic, returns the epic + all ready
descendants as one atomic unit; otherwise returns all ready non-epic
items. Lives in `WorkItemStore::readWorkOptions` on main. The plugin
ships `commands/work-next.md` calling
`mcp__oxplow__read_work_options` — without this tool, `/work-next`
errors immediately.

**AC:** Tool returns `{ mode: "epic", epic, children: [...] }`,
`{ mode: "ready", items: [...] }`, or `{ mode: "empty" }` per main's
contract; supports `full=true` flag for verbose payload; reachable
from a Claude session with the new control plane up.

### 7. `delegate_query` + `record_query_finding` MCP tools — **medium**

Used by the wiki-capture skill to fold subagent Explore findings
into wiki notes. The skill text references them; without the tools,
the "folding in Explore findings" branch is a dead reference.

**AC:** `delegate_query` produces a queryable subagent context;
`record_query_finding` persists the finding so `get_thread_notes`
returns it; the wiki-capture skill's documented flow runs end-to-end.

### 8. `reorder_work_items` MCP tool — **low**

Tauri IPC command exists (`commands/work_items.rs::reorder_work_items`)
so the UI can reorder; agent can't because no MCP wrapper. Adding the
MCP shim is mostly a copy-paste from the IPC handler.

**AC:** `mcp__oxplow__reorder_work_items({ orderedItemIds })` matches
the IPC behavior.

---

## Wiki-note infrastructure

The `wiki_note_store` table is migrated and the basic CRUD MCP tools
exist (`list_notes`, `search_notes`, etc.), but the supporting
machinery that makes notes navigable on main is missing.

### 9. Wiki-note refs parser + table + backlinks — **medium**

Main parses `[[wikilink]]` references inside note bodies and stores
them in a `note_refs` table so the UI can show backlinks ("notes
referencing this file" / "notes linking to this note"). See
`src/persistence/wiki-note-refs.ts` on main. Without this, the wiki
section of the rail and the note backlinks panel show nothing.

**AC:** Note body parser extracts `[[…]]` refs (file, file:line,
note, commit forms); `note_refs` table is populated on note
write/update; a `getBacklinks(slug)` API returns the inverse; refs
re-parse on body change.

### 10. Wiki-note thread-update store — **medium**

Per-thread "last touched" attribution for notes. Used by the
RailHud's "Finished" list to surface only notes the *current* thread
authored or revised. See
`src/persistence/wiki-note-thread-update-store.ts`.

**AC:** Writing a note from a thread inserts into
`wiki_note_thread_update`; rail's `recentlyFinished` query merges
note updates with work_item efforts; per-thread filtering works.

### 11. Wiki-notes fs watcher — **medium**

Bodies are markdown files on disk; metadata (title, refs, freshness)
syncs from the file via a watcher with a 200ms debounce. See
`src/git/notes-watch.ts`. Without it, an out-of-band edit to a note
doesn't reflect in the UI until a manual refresh.

**AC:** Editing a `.oxplow/notes/<slug>.md` outside the app
(external editor, git operation) triggers a metadata refresh on the
matching `wiki_note` row within ~250ms; new files are picked up;
deletes drop the row.
