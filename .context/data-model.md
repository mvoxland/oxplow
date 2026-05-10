# Data model


What this doc covers: the SQLite tables, their store classes, and the
`sort_index` queue invariant for work items. If you're adding a new persisted concept, also
read [ipc-and-stores.md](./ipc-and-stores.md).

## Storage

All persistence lives in one SQLite file under `.oxplow/state.sqlite`, opened
through `Database::open` (`crates/oxplow-db/src/database.rs`). Every store is
a thin class wrapping that connection. Schema changes go through versioned
migrations (`crates/oxplow-db/migrations/V1__initial_schema.sql`) gated by `PRAGMA user_version`
— migrations are append-only; never edit a prior version.

## Tables and stores

### `streams` — `StreamStore` (`crates/oxplow-db/src/stream_store.rs`)

Top-level workspace context. Exactly one row per user-facing stream tab.
Each stream owns:

- a `kind` column (migration v34) — `"primary" | "worktree"`:
  - **primary**: the repo itself. `worktree_path === projectDir`,
    `title === projectBase` (never rewritten), created exactly once at
    startup by `Services::boot()` via `StreamStore.findPrimary()`.
    Cannot be deleted (`StreamStore.deleteStream()` throws for `kind === "primary"`).
  - **worktree**: a real git worktree at
    `<parent_of_project>/<project_basename>-<slug>/` (sibling of the
    main repo) created by `createStream()` via `ensureWorktree()`.
    Pre-migration rows that point at the legacy
    `<project>/.oxplow/worktrees/<slug>/` location keep their stored
    `worktree_path`. Title defaults
    to the branch name; the runtime rewrites the title when the branch
    switches only if the old title matched the old branch (preserves
    user renames).
- a `branch` / `branch_ref` pair that is **not** pinned — any stream can
  switch branches. Updated by `StreamStore.setStreamBranch(streamId,
  branch, branchRef)`, which emits `stream.changed` (kind:
  `"branch-changed"`). The runtime drives it from two sites:
  `Services.checkoutStreamBranch(streamId, branch)` (user-triggered
  via "Switch branch…" in the StreamRail context menu) and
  `maybeSyncStreamBranch(streamId)` (fired by every `git-refs.changed`
  event so external `git checkout` in a worktree is picked up live).
  Git-level errors (dirty tree, missing branch, branch already checked
  out in another worktree) bubble through unchanged so the UI inline
  error shows git's own message.
- a worktree path (projectDir for primary; sibling
  `<parent>/<project_basename>-<slug>/` for worktree kind — the
  `<slug>` is fixed at creation and does not rename on branch switch)
- two tmux pane targets (`working` and `talking`)
- per-pane Claude resume session ids (so reconnecting picks up history)
- a `runtime_state.current_stream_id` pointer (singleton row, id=1)
- a `sort_index` column (migration v14) — streams are listed ordered by
  `sort_index, rowid`; drag-to-reorder in the StreamRail calls
  `reorderStreams(orderedStreamIds)` which reassigns sequential indexes.
  The UI enforces **primary-first** regardless of sort_index (the
  primary tab can't be dragged and nothing can drop before it). Emits a
  `stream.changed` event (kind: "reordered") so the UI can refresh.
- a `custom_prompt` column (migration V6, nullable TEXT) — per-stream
  standing instructions appended to the agent's system prompt after the
  global `agentPromptAppend` section. Set via the `set_stream_prompt`
  IPC command (`apps/desktop/src/api.ts`'s `setStreamPrompt`). Emits a
  `StreamsChanged` event so the UI re-fetches the full stream list.
  V6 also dropped a legacy `summary` column that was carried over from
  the TS schema and never displayed; the prompt feature briefly
  piggybacked on that slot before getting its own column.

Streams never look outside the project root for data; see
`architecture.md`'s "Workspace isolation rule."

`archived_at` (migration v4, nullable TEXT) — soft-delete stamp set
by the rail's "Remove…" action. `StreamStore::list` filters
`archived_at IS NULL` so archived streams disappear from the rail
without dropping rows that history (efforts, snapshots, page_visit)
references. Pre-migration rows stay NULL and remain visible. Primary
streams cannot be archived.

### `threads` — `BatchStore` (`crates/oxplow-db/src/thread_store.rs`)

Units of work *within* a stream. Statuses: `active` (writer — may mutate
the worktree) and `queued` (read-only, agents can run but writes are
denied — see [agent-model.md](./agent-model.md)'s write-guard section).
Exactly one thread per stream is `active`; the rest are `queued`. A
newly-seeded stream ships with one thread titled `Default` (pre-v12 DBs
called it `Current Thread`; migration v12 renames the sort_index=0 row).
The rolling `summary` field + `record_batch_summary` MCP tool were
removed in v13 — use the work-item log as the source of truth instead.

`closed_at` (migration v44, nullable TEXT) — closing a thread is
orthogonal to its status. A closed thread keeps its `queued` status but
sits hidden from the rail; it surfaces only on the **Closed Threads**
page (`apps/desktop/src/pages/ClosedThreadsPage.tsx`), which lists each closed
thread with its work items (read-only) and a Reopen action. Closing is
allowed only for non-writer threads with no work items in `ready` /
`blocked` / `in_progress` (`ThreadStore.close()`); promote another
thread or finish/move open items first. `ThreadStore.reopen()` clears
`closed_at` and the thread returns to the rail as a queued read-only
tab. `ThreadStore.list()` already filters closed threads out, so every
existing surface (rail, work panel, dispatch) treats them as gone
without code changes; `listClosed(streamId)` is the dedicated reader
for the Closed Threads page. Migration v44 also retires the legacy
`completed` status — any pre-existing `completed` rows are remapped to
`queued` with `closed_at = updated_at`.

`archived_at` (migration v4, nullable TEXT) — soft-delete stamp set
when the parent stream's "Remove…" action archives every thread under
it (separate from `closed_at`, which is the user-visible "I'm done
with this thread" gesture). `ThreadStore::list_for_stream` filters
`archived_at IS NULL`, so archived threads vanish from every consumer
in lockstep with the stream that owns them.

`custom_prompt` (migration v18, nullable TEXT) — per-thread standing
instructions appended to the agent's system prompt after the stream-level
`custom_prompt`. Set via `setBatchPrompt(threadId, prompt)` on `BatchStore`;
IPC-exposed as `setBatchPrompt(streamId, threadId, prompt)`. Emits a
`thread.changed` event (kind: "prompt-changed") so the UI refreshes thread
state.

**Removed in v42:** the `auto_commit` column (added in v15) and the
`commit_point` / `wait_point` tables (added in v6/v7). Commits are now
user-driven only — the harness has no `git commit` path, no queueable
commit/wait markers, and no auto-commit Stop directive. Consumers
running an older DB get the columns/tables dropped on first launch with
the new binary; existing rows are not migrated forward (no surface
reads them).

### `work_items` — `WorkItemStore` (`crates/oxplow-db/src/work_item_store.rs`)

The actual TODO list. Kinds: `epic`, `task`, `subtask`, `bug`, `note`.
Statuses: `ready`, `in_progress`, `blocked`, `done`, `canceled`,
`archived`. `archived` is a terminal state that hides the item
from the default Work panel view — archived rows fold into the Done
section's bucketing but aren't rendered unless the user flips the "Show
archived (N)" toggle in the Done section header. The same header carries
an "Archive all" action that bulk-archives every visible Done/Canceled
row. `listReady`'s blocker check treats archived the same as done/
canceled. `parent_id` chains items under epics. `acceptance_criteria` is
plain text (one criterion per line). Work-item links express dependencies
(`blocks`, `discovered_from`, `relates_to`, …) via the `work_item_links`
join table.

`thread_id` is nullable — items with `thread_id IS NULL` belong to the
**backlog** (a global, stream-less queue). The store API uses the constant
`BACKLOG_SCOPE` as a sentinel string in event payloads so listeners can
distinguish backlog changes from in-thread changes.

`author` (migration v26, nullable TEXT) — semantic origin of the row,
distinct from `created_by` (which just classifies the SQL writer as
`user`/`agent`/`system`). Values: `'user'` (explicit user-initiated
create), `'agent'` (explicit agent `create_work_item` /
`file_epic_with_children` call), or `NULL` (legacy rows). Pre-v29 DBs
also held `'agent-auto'` rows synthesized by the removed auto-file
listener; migration v29 cancels any such still-in_progress rows, and
the read path maps the legacy string to `null` so older terminal rows
continue to load under the narrowed enum.

`note_count` is a computed column added to every `WorkItem` returned by the
store (via COUNT subquery over `work_note`). It drives the note badge on
list rows and is always 0 when no notes exist.

`category` and `tags` (migration v48, nullable TEXT) — grooming
metadata used by the Backlog page. `category` is a free-text bucket
that drives the default group-by axis on the Backlog grooming surface
(e.g. "UI / UX", "Infra", "Polish"); `tags` is a comma-separated
list normalized on write (split, trim, dedupe, rejoin with `, `) that
doubles as filter chips. Both columns live on every `work_items` row
so a row keeps its grooming metadata when promoted from backlog into
a thread or demoted back — there is no copy across tables. Caps:
`category` 200 chars, `tags` 500 chars total. Pass `null` through
`updateWorkItem` / `updateBacklogItem` to clear; omit to keep.

### `work_note` — thread-scoped notes only (`crates/oxplow-db/src/work_satellite.rs`)

Structured per-thread notes. Each row has `id`, nullable
`work_item_id` (kept for legacy rows), nullable `thread_id`, `body`,
`author` (free-form string, e.g. "agent", "user",
"explore-subagent"), and `created_at`. A CHECK still enforces that
**exactly one** of `work_item_id` / `thread_id` is non-NULL.

**Item-scoped writes were retired** — `work_item_effort.summary` is
the canonical record of what shipped on a task, so a parallel
per-item note table for the same purpose was duplicative. The
`add_work_note` MCP tool, the `add_work_note` / `list_work_notes`
IPC commands, and the work-item modal's "Notes" timeline section
were removed alongside this. Pre-existing item-scoped rows stay in
the table but no surface reads or writes them.

Thread-scoped rows (`thread_id` set, `work_item_id` NULL) are the
landing spot for `oxplow__delegate_query` Explore-subagent findings.
The delegate tool pre-allocates a row with empty body (via
`addThreadNote`), passes the id into the subagent prompt, and the
subagent fills the body by calling `oxplow__record_query_finding`
(store method `updateThreadNoteBody`). The orchestrator reads them
back via `oxplow__list_thread_notes` / `listThreadNotes(threadId)` —
reverse-chronological, capped at 100.

### `work_item_effort` — `WorkItemEffortStore` (`crates/oxplow-db/src/effort_store.rs`)

An **effort** is one `in_progress → done` (or blocked/canceled) cycle
of a work item. Columns: `work_item_id`, `started_at`, `ended_at`,
`start_snapshot_id`, `end_snapshot_id`, `summary` (v35 — free-form text
written by `complete_task` describing what shipped in this effort; one
summary per effort, replaces the old per-item note-history append).
Auto-managed by the runtime on `work-item.changed` status transitions:

- `→ in_progress` opens a new effort; a `task-start` snapshot is flushed
  and linked to `start_snapshot_id`.
- `in_progress → {done, blocked, canceled}` closes the effort; a
  `task-end` snapshot is flushed and linked to `end_snapshot_id`,
  subject to a 5-minute minimum gap between snapshots — if the latest
  snapshot is fresher than that gap, the close path skips flushing a
  new row (the effort's `end_snapshot_id` is left null in that case).

Re-opening a task (done → in_progress) produces a second effort. At most one open effort per work item at a time.

`work_item_effort_file` (v22) records per-effort write paths so parallel
subagents in one thread get distinct file lists instead of the union via
the snapshot pair-diff. Columns: `effort_id`, `path`, `first_seen_at`,
primary key `(effort_id, path)`. Rows come from the `touchedFiles`
payload on the `update_work_item` transition to `done`, not
from the PostToolUse hook (the previous heuristic couldn't attribute
writes when ≥2 efforts were in_progress). See agent-model.md's
"Per-effort write log" for the flow. Consumed by `get_effort_files`
(`crates/oxplow-tauri-ipc/src/commands/effort.rs`) over the
`EffortStore` and `SnapshotStore`: when ≥2 efforts share an end
snapshot AND this effort has ≥1 row here, the pair-diff is filtered
to those paths; 0 rows → fall back to raw pair-diff ("assume all");
1 effort → raw pair-diff.

Read API: `listEffortsForWorkItem(itemId)`, `listOpenEfforts()`,
`listEffortsForSnapshot(snapshotId)`,
`listEffortsForPath(path)` (closed
efforts that touched `path` via `work_item_effort_file`, joined to the
owning work item's title/status, newest-first by `ended_at` — drives
the local-blame overlay described in `.context/editor-and-monaco.md`).
`createWorkItemApi` exposes `listWorkItemEfforts(itemId)` which returns
per-effort rows with pre-joined start/end snapshot metadata and the
list of changed paths (computed from the pair diff).

**Commit↔item attribution is intentionally NOT tracked.** A
`work_item_commit` junction existed briefly (migration v27) but was
removed in v28. Users commit outside oxplow all the time (IDE buttons,
CLI, CI rebases, merges, squashes) and oxplow has no authoritative hook
there. A blame-style feature built on that data would lie more often
than it'd be useful. If a future feature wants "show me commits for
this item," the answer is to scope `git log` by the files
in `work_item_effort_file`.

### `file_snapshot` + `snapshot_entry` — `SnapshotStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Time-ordered, self-contained snapshots. `file_snapshot` columns:
`id, stream_id, worktree_path, version_hash, source, created_at,
effort_id`. `snapshot_entry` holds the per-path rows: `path, hash,
mtime_ms, size, state`.

`effort_id` (nullable, FK → `work_item_effort.id` ON DELETE SET NULL)
ties `task-start` / `task-end` rows back to the effort that produced
them. `startup` snapshots leave it null. The mirror columns on the
effort row — `work_item_effort.start_snapshot_id` and
`work_item_effort.end_snapshot_id` — are the canonical lookup path for
"the snapshots that bracket this effort"; `file_snapshot.effort_id` is
the reverse pointer. The 5-minute minimum gap rule in
`applyStatusTransition` may leave the effort's `end_snapshot_id`
null — when the most recent snapshot is fresher than
`END_SNAPSHOT_MIN_GAP_MS`, the close path skips flushing a new row to
avoid spamming history with near-identical states.

`source` is one of `task-start | task-end | startup | external`.
`version_hash` is a SHA-256 over the canonical
`(path, hash, size, state)` entry set — `mtime_ms` is deliberately
excluded so touching a file without changing its bytes doesn't produce
a new snapshot. Deleted files have no `snapshot_entry` row at all (the
"entry missing" case is the deletion signal); readers collapse
"absent" and the old `state="deleted"` cases into one branch.

**Dedup on flush.** `flushSnapshot()` computes the next snapshot's
`version_hash` (reusing the most recent snapshot's entries for any path
not listed in `dirtyPaths`), and if that hash matches the newest
existing snapshot for the stream, it returns `{ created: false, id:
<existing> }` instead of inserting a new row. `dirtyPaths` is an
optimizer hint — when null the entire worktree is walked.

**No parent chain.** Snapshots have no `parent_snapshot_id`; the
"previous" snapshot for diff purposes is just the most recent
`file_snapshot` row with `created_at < target.created_at` for the same
stream. `getSnapshotSummary(id, previousId?)` returns created/updated/
deleted counts relative to that previous snapshot (or an explicit one
if provided); `getSnapshotPairDiff(beforeId, afterId, path)` serves
arbitrary pair diffs.

**Baseline is hidden from Local History.** The first snapshot per
stream has no predecessor, so there's nothing to diff against and
nothing meaningful to show. `listSnapshotsForStream` excludes it
(subsequent snapshots use it as their "previous" via
`getSnapshotSummary`). The baseline still lives in the DB — only the
UI list skips it.

**Rows come with pre-joined labels.** `listSnapshotsForStream` joins
against `work_item_effort` to populate `label` + `label_kind` on each
`FileSnapshot` (task title + " — start"/" — end"); effort-end wins
over effort-start when the same snapshot is both. Unlinked snapshots
get `label: null` and the UI falls back to the `source` column.

Blobs live on disk at `.oxplow/snapshots/objects/xx/yyyy…` (sha256
addressed, shared across streams for dedup).

Entry states:
- `present`: file captured, `hash` points at a real blob.
- `oversize`: file existed but exceeded `snapshotMaxFileBytes`; no blob,
  but `mtime_ms` and `size` are tracked.

A deleted file has no `snapshot_entry` row — readers treat a missing
entry as the deletion signal. Migration v24 drops any legacy
`state='deleted'` tombstones.

**Retention.** `SnapshotStore.cleanupOldSnapshots(retentionDays)`
deletes snapshots older than the cutoff (default 7 days, configurable
via `oxplow.yaml`'s `snapshotRetentionDays`; `0` disables pruning). The
most recent snapshot per stream is always kept. `gcBlobs()` then sweeps
`.oxplow/snapshots/objects/` and removes any blob whose sha isn't
referenced by a surviving manifest. The blob store is shared across all
streams (`.oxplow/snapshots/objects/`), so GC runs at the project level
and dedupes identical content across branches.

Cleanup runs at runtime startup and again once every 24 hours via
`runtime.runSnapshotCleanup` (wired in `initialize()`, cleared in
`dispose()`).

**Ignoring generated directories.** The fs-watcher and the snapshot
seeder share one filter: `shouldIgnoreWorkspaceWatchPath` in
`crates/oxplow-fs-watch/src/lib.rs`. It covers `.git/`, `.oxplow/logs/`,
`.oxplow/worktrees/`, and a hardcoded list of common build/cache dir
names (`node_modules`, `dist`, `build`, `target`, `.next`, `.turbo`,
`.cache`, `.venv`, `__pycache__`, …). Users can extend the list via
`generatedDirs: [...]` in `oxplow.yaml` — names are single path
segments matched anywhere in the relative path, and apply to both
the workspace watcher and the snapshot store. No changes to
existing snapshots on toggle; newly ignored paths simply stop
appearing in future dirty sets.

### `wiki_page` — `WikiPageStore` (`crates/oxplow-db/src/wiki_page_store.rs`)

User-curated personal knowledgebase — agent-written writeups, diagrams,
and explanations that accumulate per project. **Bodies live on disk**
as plain markdown files at `.oxplow/wiki/<slug>.md` (not committed to
git — this is a personal KB, not team docs). The table only holds
metadata; the filesystem is the source of truth for content.

Columns: `id, slug (UNIQUE), title, body, captured_head_sha,
captured_refs_json, created_at, updated_at`. The `body` column
mirrors the on-disk markdown so MCP can run substring/content
searches without reading every file — the filesystem is still the
source of truth, and the watcher keeps the column in sync on every
upsert.

Workflow: the agent writes/edits note files directly with its
Write/Edit tools (no MCP round-trip for bodies). A dedicated watcher
(`crates/oxplow-fs-watch/src/lib.rs`) picks up every file event, re-parses the
file, and upserts metadata + body. **Every write — agent or user —
re-baselines freshness**, because any write implicitly asserts "this
is current as of now." There is no agent-vs-user distinction.

Freshness is a general indicator, not a proof:
- `captured_head_sha` is HEAD at last write. If HEAD advances, the
  note is flagged `stale`.
- `captured_refs_json` stores `{path, blobSha, mtimeMs}` for every file
  path mentioned in the note (extracted by the wiki-note-refs parser
  inside `crates/oxplow-db/src/wiki_page_store.rs`). `computeFreshness`
  rehashes each referenced file; any mismatch → `stale`; any missing
  file → `very-stale`.

Parsed wikilink targets are split across three JSON columns by the
parser in `crates/oxplow-app/src/wiki_pages.rs`:

- `file_refs_json` — `[[src/foo.ts]]` / `[[src/foo.ts:42]]` style
  references (workspace-relative file paths).
- `dir_refs_json` (migration V7) — `[[dir:src/components]]` style
  references. The `dir:` prefix is the explicit directory marker
  (mirrors `git:` for commit refs); a trailing `/` on the path is
  tolerated and stripped on store. `backlinks_for_dir` is the
  symmetric reader to `backlinks_for_file`.
- `related_notes_json` — `[[some-other-slug]]` cross-note links.

MCP tools (`crates/oxplow-mcp/src/lib.rs`) are metadata-only —
`list_wiki_pages`, `get_wiki_page_metadata`, `resync_wiki_page`, `search_wiki_pages`
(title), `search_wiki_page_bodies` (content + ~200-char snippet),
`find_wiki_pages_for_file` (backlinks via `captured_refs`), `delete_wiki_page`.
There is no `create_note` or `update_note`: the agent Writes the
file, then optionally calls `resync_wiki_page` to pin freshness
immediately (otherwise the watcher catches up within a ~200ms
debounce).

UI: `NotesPane` (`apps/desktop/src/components/Notes/NotesPane.tsx`) is a
left-dock `ToolWindow` with a debounced full-text search input and a
recency-driven TOC ("Recently visited" / "Recently modified" / "All
notes"); each section caps at 8 rows with a "show all" toggle. The
freshness dot + relative-timestamp pattern is shared by all rows.
Selecting a row opens the note as a center tab (`note:<slug>`)
rendered by `NoteTab` (`apps/desktop/src/components/Notes/NoteTab.tsx`), which
owns the view/edit/delete UI . Markdown rendering is delegated to the shared
`MarkdownView` (`apps/desktop/src/components/Notes/MarkdownView.tsx`) which
wraps `react-markdown`
+ `remark-gfm` and post-renders mermaid
fenced blocks into inline SVG when `renderMermaid` is set. The
same component is reused for the Plan work-item description /
acceptance fields (`WorkItemDetail`) so headings, lists, code,
links, and emphasis come through there too — without mermaid.
IPC surface: `listWikiPages`, `readWikiPageBody`,
`writeWikiPageBody`, `deleteWikiPage`, `searchWikiPages`, plus the
`wiki-note.changed` event on the bus. Full-text search is backed by
the `wiki_page_fts` FTS5 virtual table (migration v39); insert/update/
delete triggers keep it in sync, so `WikiPageStore.searchBodies()`
returns ranked results with `<mark>…</mark>`-highlighted snippets.

### `page_ref` — unified cross-page reference graph (`crates/oxplow-db/src/page_ref_store.rs`, migration `V11__page_ref.sql`)

One row per directed edge `(source) --ref_type--> (target)` across
every page kind. Replaces the in-memory `computeBacklinks` indexer
that used to live in the desktop frontend; both backlinks ("what
points at me?") and outbound ("what do I point at?") are SQL
queries against this table.

Columns: `source_kind, source_id, target_kind, target_id, ref_type,
source_extra` (PK on the first five). Indexes on
`(target_kind, target_id)` for backlinks and `(source_kind,
source_id)` for outbound. `kind` is denormalised next to `id` so
kind-filtered queries don't need LIKE on a synthetic combined
column.

Canonical id shapes match the frontend's `TabRef.id`:
- `wiki:<slug>` source kind uses just the slug as the id
- `work-item:wi-…`
- `file:<repo-relative path>` (id is the bare path; the `file:`
  prefix is implicit from the kind)
- `directory:<repo-relative path, no trailing slash>`
- `git-commit:<full sha>`
- `finding:<rowid as string>`

**Writers own slices by `ref_type`.** A single `(source_kind,
source_id)` can have rows from multiple owners — a work-item's
body-mention edges (from `work_item_store`), link edges (from the
link store), and touched-file edges (from the effort store) all
land under `(work-item, wi-X)` but with distinct `ref_type`s.
`SqlitePageRefStore::replace_source_for_ref_types` lets each
writer wipe + re-insert only the rows whose `ref_type` it owns,
so other owners' rows survive.

Writers (one per source kind / slice):

| Owner | Source | Slice (`ref_type`s) |
|---|---|---|
| `wiki_pages.rs` (`oxplow-app`) | `wiki:<slug>` | full source — uses `replace_source` |
| `work_item_store::upsert` | `work-item:wi-X` body slice | `wi_body_mention`, `wikilink`, `wiki_file_ref`, `wiki_dir_ref`, `finding_mention`, `commit_mention` |
| `work_satellite::SqliteWorkItemLinkStore` create/delete | `work-item:wi-X` link slice | `work_item_link:blocks` / `relates_to` / … |
| `effort_store::record_file` | `work-item:wi-X` touched slice | `touched_file` |
| `analytics_stores::SqliteCodeQualityStore::append_finding` | `finding:<id>` | full source |
| `commit_indexer.rs` (`oxplow-app`) | `git-commit:<sha>` | full source — diff yields `touched_file`, message yields the same body-mention set |

The shared extractor `oxplow_domain::refs::extract(body) ->
ExtractedRefs` is the single parser used by every writer that
takes a free-text body, so wiki/work-item/commit-message ref
recognition stays in lock-step. Pure projections live in
`crates/oxplow-db/src/page_ref_projections.rs`.

Reader: `SqlitePageRefStore::list_backlinks(target_kind, target_id,
limit)` and `list_outbound(source_kind, source_id, limit)`. Both
are exposed as Tauri commands (`list_backlinks` / `list_outbound`,
which decorate each row with a best-effort `source_label` from
the source store) and as MCP tools of the same names.

Boot-time backfill: `oxplow_app::page_ref_backfill::run(...)` re-
projects every existing work-item body, link, effort, and finding
into the table on app start, idempotently. Wiki bodies and recent
commits are covered by their own initial-scan paths and don't
need separate backfill.

### `wiki_page_thread_update` — wiki-note thread-update tracking (table in `crates/oxplow-db/migrations/` + helpers in `crates/oxplow-db/src/wiki_page_store.rs`)

Per-thread attribution side table for wiki page edits. Notes themselves
are global (one body per slug, shared across all threads/streams), but
the rail's "Finished" list filters by which thread last touched each
note — mirrors how task efforts attribute via
`work_item_effort.thread_id`.

Columns: `slug, thread_id, updated_at`. PK `(slug, thread_id)` so
repeated edits in the same thread upsert in place. Index on
`(thread_id, updated_at DESC)` drives the rail query.

Writers funnel through two seams, both routing through
`Runtime.wikiPageThreadUpdateStore.recordUpdate(slug, threadId)`:

- **`oxplow__resync_wiki_page` MCP** — accepts `threadId` and records the
  edit when present. The wiki-capture skill always supplies it.
- **PostToolUse hook** in `crates/oxplow-runtime/src/lib.rs` — when
  the agent writes a path matching `.oxplow/wiki/<slug>.md` via
  Write/Edit/MultiEdit, attribution is recorded immediately, no
  waiting on the watcher debounce.

The notes file watcher itself does not record attribution (it has no
thread context). `Runtime.writeWikiPageBody` (the editor save IPC)
also intentionally skips attribution — the notes editor isn't
thread-bound, so guessing would be worse than abstaining.

`Runtime.deleteWikiPage` clears every attribution row for the slug
when the note is deleted, so removed notes don't linger on the rail
under their last author.

### `usage_event` — `UsageStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Generic (kind, key) usage tracking. Append-only event log with columns
`stream_id (nullable), thread_id (nullable), kind, key, event,
occurred_at`. Aggregates (most-recent, most-frequent, currently-open)
are derived by query rather than stored, so adding a new "kind"
(editor file, work item, future surfaces) needs no schema change.
Indexes: `(kind, key, occurred_at DESC)`, `(stream_id, kind,
occurred_at DESC)`, `(thread_id, kind, occurred_at DESC)`. Both scopes
are recorded simultaneously — `stream_id` is the workspace tab,
`thread_id` is the active thread within it — so consumers can roll
up by either dimension or intersect them.

The store coalesces rapid repeats: if the most recent matching
`(kind, key, event, stream_id)` row is younger than `coalesceMs`
(default 30s), `record()` bumps its `occurred_at` instead of inserting
a new row. This keeps history clean when a user re-selects the same
target several times in quick succession.

Current write hookpoints (all in `apps/desktop/src/App.tsx`, all pass both
`streamId` and `threadId`):

- `wiki-note` — `handleOpenNote` records a visit when a note becomes
  the active center tab. Drives the Notes pane's "Recently visited"
  section via `listRecentUsage({ kind: "wiki-note", … })`.
- `editor-file` — `handleOpenFile` records a visit when a file
  becomes the active center tab. Not yet surfaced in UI; collected
  for future "recent files" / "files this thread cares about" views.
- `work-item` — `handleRequestEditWorkItem` records a visit when the
  user opens the edit modal. Not yet surfaced.

UI surfaces consume via `subscribeUsageEvents(listener, { kind })` to
refresh on cross-window visits without polling.

### `code_quality_scan` + `code_quality_finding` — `CodeQualityStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Deterministic, language-agnostic findings produced in-process by
two tree-sitter analyzers (`metrics` and `duplication`). Two
tables, one store, one runtime method (`runCodeQualityScan`). The
store doesn't run analyzers itself — the runtime calls
`crates/oxplow-app/src/code_quality_runner.rs` and hands normalized
findings back via `completeScan`. The legacy values `'lizard'` and
`'jscpd'` are migrated to `'metrics'` / `'duplication'` by V8.

`code_quality_scan` rows: `id, stream_id, tool ('metrics' |
'duplication'), scope ('codebase' | 'diff'), base_ref (nullable, set
when scope = 'diff'), status ('running' | 'completed' | 'failed'),
error_message, started_at, completed_at`. One row per analyzer
invocation per `(stream, tool, scope)` combination. Index on
`(stream_id, tool, started_at DESC)` makes "latest scan per tool"
cheap.

`code_quality_finding` rows: `id, scan_id, path, start_line, end_line,
kind ('complexity' | 'function-length' | 'parameter-count' |
'duplicate-block'), metric_value (REAL), extra_json`. The metrics
analyzer emits three findings per function (one per metric kind)
with `extra.functionName` for grouping. The duplication analyzer
emits two findings per duplicate-pair (one per side) with
`extra.peerPath` / `extra.peerStartLine` / `extra.peerEndLine` so
the UI can render "duplicates X lines from Y:Lstart-Lend" without
re-querying.

Retention is store-driven, not schema-driven: each `completeScan`
prunes old scans for the same `(stream, tool, scope)` triple beyond
`keepLast` (default 10), deleting their findings in the same
transaction. Different scopes retain independently — running the
diff scan many times doesn't evict the codebase scan.

`listLatestFindings({ streamId, tool?, paths? })` joins on the most
recent `completed` scan per `(stream, tool, scope)`, ignoring
running/failed scans entirely so the panel never shows partial
results. The `paths` filter (used by the Diff vs base tab) intersects
findings against `listBranchChanges`'s file list at query time, so
findings persisted by a codebase scan can also drive a focused
"changed files only" view without re-running.

The store publishes `code-quality.scanned` events on start /
complete / fail; `CodeQualityPanel` (`apps/desktop/src/components/CodeQuality/`)
subscribes via `subscribeCodeQualityEvents(streamId, fn)` and
refetches.

### `page_visit` — `PageVisitStore` (`crates/oxplow-db/src/analytics_stores.rs`)

Append-only event log of in-app page navigations. One row per visit
recorded by `App.handleOpenPage` (skipping `agent`, `new-stream`,
`new-work-item`).

Columns: `id`, `page_kind`, `page_id`, `visited_at` (ISO),
`duration_ms?`, `thread_id?` (added in v3 — nullable so legacy rows and
boot-screen visits with no active thread still record). Indexes on
`visited_at DESC`, `(page_kind, page_id)`, and
`(thread_id, visited_at DESC)`. Aggregates derived by query:

- `listRecent({threadId,limit,dedupeByRef,excludeKinds})` — drives the
  rail History (with `dedupeByRef`).
- `topVisited({threadId,sinceT,limit})` — most-visited rollup with
  payload+label of the latest visit per ref. Drives the rail's
  "Most visited" toggle and the Visits dashboard.
- `countByDay({refId,threadId,sinceT,untilT})` — daily bucketed
  counts for behavior-over-time charts.

Insert publishes `page-visit.changed` for renderer-side invalidation.

### `finished_seen` — runtime watermark for the rail's Finished section

`finished_seen (scope TEXT PRIMARY KEY, t TEXT NOT NULL)`. Tiny KV
table holding "mark all as seen" watermarks for the rail's *Finished*
section. Two scope keys are written: `thread:<id>` (filters work-item
closes for that thread) and `notes` (filters wiki-note updates,
globally). `listRecentlyFinished` filters out rows whose timestamp is
≤ the matching watermark; `clearRecentlyFinished` upserts both scopes
to `now()`.

## The `sort_index` queue

`work_items.sort_index` orders work in a single numeric space scoped
per thread. `runtime.reorderThreadQueue(streamId, threadId, entries)`
rewrites the values in one operation; entries are `{ id }` with
`sort_index = position`.

**Visual vs persistence order for Done.** Sections in `WorkGroupList`
render ascending by `sort_index` *except* Done, which renders
descending (newest-finished items surface on top). The underlying
`sort_index` space is still a single ascending line — the section is
only flipped at render time. When a drag-reorder persists a new order,
`finalizeReorderIds` in `plan-utils.ts` reverses each descending run
(`done`/`canceled`/`archived`) so the `reorderItems` /
`reorderThreadQueue` "sort_index = position" rule produces the intended
visual result. The drag handler passes the *effective* new status of a
row whose status is changing as part of the drop so the run detector
sees the new section membership. Dropping any item *into* Done is a
drop-to-top contract: the drag handler inserts the row at the head of
the Done bucket in visual order, and `work-item-store.updateItem`
bumps `sort_index` to `MAX+1` on every non-Done → Done transition, so
the two paths agree on "newest-done on top." Any new section with a
non-ascending display must either do the same reversal dance or get
its own flat list.

## Status diagrams (text)

```
work item:    ready ─► in_progress ─► done ─► archived
                   ╰─────────► blocked ◄─┘
                   ╰─────────► canceled ─► archived
```

### Transitions to `in_progress` (server-side guard)

`WorkItemStore.updateItem` rejects any direct jump into `in_progress`
from `canceled` or `archived` (those are explicit "abandoned" states
the user must re-`ready` first). All other sources are accepted:

- `ready → in_progress` (normal pickup)
- `done → in_progress` (reopen — the redo path when the user pushes
  back on shipped work)
- `blocked → in_progress` (deliberate unblock gesture)
- `in_progress → in_progress` (no-op)

`dispatch_work_item`'s autoStart path only fires when the item is
currently `ready`.

`listReady` / `readWorkOptions` / `list_ready_work` filter to
`status='ready'` only — `blocked` items are never dispatchable until
un-blocked.

## Change events

Cross-store change fan-out is centralized on the typed EventBus
(`crates/oxplow-app/src/events.rs`). Stores call `EventBus::emit(...)`
with a coarse `OxplowEvent` variant (e.g. `WorkItemsChanged`,
`StreamsChanged`); subscribers refetch the affected bucket on
receipt. The bus is a `tokio::sync::broadcast` channel, so a slow
subscriber sees `RecvError::Lagged` rather than blocking publishers,
and adding/removing subscribers is lock-free.

The runtime relays each store's changes onto the typed EventBus
(`crates/oxplow-app/src/events.rs`) as `*.changed` events:

- `workspace.changed`, `git-refs.changed`, `workspace-context.changed`
- `work-item.changed`, `backlog.changed`, `thread.changed`
- `file-snapshot.created`, `agent-status.changed`
- `hook.recorded`, `config.changed`

UI components subscribe via `subscribeOxplowEvents()` (or scoped helpers
like `subscribeWorkspaceEvents`, `subscribeGitRefsEvents`) in
`apps/desktop/src/api.ts`. See [ipc-and-stores.md](./ipc-and-stores.md) for how to
plumb a new event end-to-end.

## UI-only state worth naming

A few UI surfaces hold non-persisted state that callers (probes,
docs, future stores) reference by name. Listed here so renames touch
the docs in one diff:

- **Files-pane filter mode** — `FilterMode = "all" | "uncommitted" |
  "branch" | "unpushed"` lives in `ProjectPanel` state and
  drives the file-tree visibility filter. The eye-icon trigger button
  is `data-testid="files-filter-toggle"`; each popover option is
  `data-testid="files-filter-option-<value>"` (e.g.
  `files-filter-option-uncommitted`). `branch` / `unpushed` auto-
  fall back to `uncommitted` if the underlying scope disappears (no
  branch base, no upstream).

## Related

- [ipc-and-stores.md](./ipc-and-stores.md) — adding new stores and IPC.
- [agent-model.md](./agent-model.md) — how the agent acts on this data.
- [git-integration.md](./git-integration.md) — `gitCommitAll` for the
  Files-panel commit button (user-driven).
