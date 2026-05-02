# Git integration


What this doc covers: the three filesystem watchers that keep git state
fresh in the UI, the runtime-side git operations, and the rule that
agents never call `git` directly. For the data side of commits (commit
points), see [data-model.md](./data-model.md) and
[agent-model.md](./agent-model.md).

## Three watchers

The runtime keeps three independent `fs.watch`-based watchers running.
Each cares about a different slice of the project state.

### 1. Workspace watcher

`crates/oxplow-app/src/workspace_watch.rs` — `WorkspaceWatchRegistry`.
One watcher per stream. Rather than registering a single recursive
watch on the worktree root (which would force `notify_debouncer_full`
to walk every subtree — including `target/` and `node_modules/` — at
boot to seed its cache), registration is **scoped**:

- A non-recursive watch on the worktree root, so top-level file
  changes and the appearance/disappearance of top-level dirs still
  fire.
- One recursive watch per top-level directory **except** the names in
  `EXCLUDED_TOP_LEVEL` (`.git`, `.oxplow`, `target`, `node_modules`).

`is_uninteresting` still filters events from those dirs as a
defence-in-depth step (and to drop swap/temp files), but the meaningful
win is at registration: we never seed cache for the dirs we never
care about. `EXCLUDED_TOP_LEVEL` is the single source of truth for
both pieces.

Drives `workspace.changed` events. Consumed by:

- `ProjectPanel` to refresh the file tree.
- `EditorPane` for external-file-changed prompts.
- `runtime.markDirty` (invoked from the watcher callback) to add the
  path to the per-stream in-memory dirty set, which the next snapshot
  flush reads as its optimizer hint.

Source files mutate constantly; this watcher's job is to keep the
file-tree current and to feed the snapshot dirty set.

### 2. Git root watcher

A non-recursive `FsWatcher` on `projectDir` itself, set up inline in
`workspace_watch::spawn_project_context`. Listens only for direntry
changes whose filename is `.git`. Non-recursive is sufficient: we only
need to know whether `.git` appears or disappears at the project root,
and a recursive watch here would re-walk the entire `.git` tree on
boot for nothing.

Fires when the user runs `git init` (or removes `.git`) in the project
root. On change:

- Re-reads `isGitRepo(projectDir)` and updates `gitEnabledCached`.
- Publishes `workspace-context.changed` with the new `gitEnabled` flag
  so UI surfaces (e.g. branch picker, stream creation form) enable or
  disable themselves.
- Re-binds the **git refs watcher** for every stream (starts watching if
  `.git` just appeared, stops if it disappeared).

This is the only watcher that lives at the project-root level rather
than per-stream.

### 3. Git refs watcher

`crates/oxplow-git/src/refs_watch.rs` — `GitRefsWatcher`. The
per-stream registry lives in
`crates/oxplow-app/src/workspace_watch.rs`
(`WorkspaceWatchRegistry`), which spawns one `GitRefsWatcher` and one
`FsWatcher` per stream at boot and bridges their broadcasts onto the
shared `EventBus` as `gitRefsChanged` / `workspaceChanged`. Watchers
debounce ~250ms (a single `git commit` fires a dozen events touching
`HEAD`, `refs/*`, `logs/*`, `index`, `ORIG_HEAD`, …).

When the stream lives in a secondary worktree (the common case — oxplow
creates worktrees as siblings of the main repo), the stream's
`.git` is a pointer file, not a directory. The watcher reads the
`gitdir:` line to find the per-worktree state dir (containing `HEAD`,
`index`, `logs/HEAD`) and also follows the `commondir` pointer to watch
the shared `.git` (where `refs/heads/*` actually update). Both dirs are
watched; without the commondir watch, `git fetch` / ref updates from
outside the worktree would be missed.

Fires `gitRefsChanged` after each debounce. Consumed silently (no
loading spinner) by:

- `HistoryPanel` — reloads the commit log.
- `ProjectPanel` — refreshes the indexed git statuses.
- (Formerly `GitChangesPanel`, now folded into `ProjectPanel`'s filter
  modes.)

The recursive `fs.watch` falls back to per-subdir watching on platforms
that don't support recursive mode.

### 4. Notes watcher

`crates/oxplow-fs-watch/src/lib.rs` — not really a git watcher, but lives next
to the others because it wraps `fs.watch` the same way. Watches
`.oxplow/wiki/` for `.md` file create/change/delete, debounces
~200ms per slug, and calls `syncNoteFromDisk` → `WikiPageStore.upsert`
(or `deleteBySlug`). Captures current HEAD (`readWorktreeHeadSha`)
and per-reference blob SHA-256 hashes as the freshness baseline.

Every write is treated identically — agent and user edits both
re-baseline freshness — so the watcher is the single sync path for
`wiki_page` metadata. See `data-model.md` → `wiki_page`.

### Why three

They watch overlapping but disjoint things:

- workspace = source files (excluding `.git`)
- root watcher = appearance/disappearance of `.git`
- refs watcher = mutations *inside* `.git`

A single recursive watcher on the root would lump them together and
either spam the UI on every internal git op or miss external changes
that don't touch source files.

### Boot is async

`WorkspaceWatchRegistry::spawn` and `WikiPagesWatcher::spawn` run as
background tasks reported through `BackgroundTaskStore` (kinds `Git`
and `NotesResync`). The desktop boot path does not block on either —
the renderer paints first, and the `BackgroundTaskIndicator` shows
"Starting workspace watchers" / "Initial wiki pages scan" rows until
each scan settles. Filesystem events start arriving once the cache
walk completes.

## GitService — the singleton

Every read of git state and every mutating git op routes through
`oxplow_app::git_service::GitService`, held on `Services` as
`Arc<GitService>`. There is one of these for the whole app, not one
per stream. It owns:

- A per-stream snapshot cache (`HashMap<StreamId, StreamSnapshot>`)
  of the slow-to-recompute slices: `WorkspaceStatusSummary`, the
  `path → GitFileStatus` map, the branch list, and
  `RepoConflictState`. `None` slots mean "not yet hydrated"; a read
  falls back to a live query and writes the result back.
- A debounced refresh worker (200ms window) fed by an unbounded
  mpsc. The worker coalesces consecutive tasks for the same stream
  into a single git walk.
- A bus listener that translates `OxplowEvent::WorkspaceChanged` /
  `GitRefsChanged` into refresh tasks: workspace events refresh
  statuses + conflict state; refs events refresh branches + conflict
  state.
- An internal `broadcast::Sender<SnapshotChanged>` for in-process
  consumers that want fine-grained cache update events; renderer
  clients keep using the existing `OxplowEvent` channel.

`GitService::register(stream_id, worktree)` and `deregister(stream_id)`
are called from the stream lifecycle commands (`create_worktree`,
`adopt_worktree`, `delete_stream`, `archive_stream`) so the snapshot
map stays in sync with the stream list. At boot, `GitService::spawn`
seeds itself from `streams.list()` asynchronously — readers against
unseeded streams just take the live-query path until the seed lands.

### What's cached vs. pass-through

Cached today: `status_summary`, `statuses`, `branches_for`,
`conflict_state`. Pass-through (no cache yet, but routed through the
service so caching can be layered in later without touching call
sites): `git_log`, `commit_detail`, `commits_ahead_of`, `blame`,
`local_blame`, `list_file_commits`, `read_file_at_ref`, `branch_changes`,
`change_scopes`, `ahead_behind`, `search_workspace_text`,
`list_all_refs`, `list_recent_remote_branches`,
`list_existing_worktrees`, `list_adoptable_worktrees`,
`detect_default_branch`. Workspace-file ops
(`list_workspace_entries` / `read_workspace_file` /
`write_workspace_file` / etc.) also go through the service.

### Mutating ops auto-refresh

`commit_all`, `add_path`, `restore_path`, `fetch`, `pull`,
`pull_remote_into_current`, `push`, `push_current_to`, `merge`,
`rebase`, `rename_branch`, `delete_branch`, `append_to_gitignore` are
all pass-through wrappers around `oxplow_git::*` that, on success:

1. Schedule a snapshot refresh of the affected slices (full refresh
   for merge/rebase/pull/fetch; just statuses for add/restore;
   branches+conflict for branch ops; etc.).
2. Emit `OxplowEvent::WorkspaceChanged` and/or `GitRefsChanged` so
   the renderer's existing subscribers update without waiting for the
   fs-watch debounce window.

This is why hitting "Pull" in the UI doesn't sit on the watcher's
~250ms debounce before the rail catches up.

## Runtime git operations

All git invocations go through `crates/oxplow-git/src/lib.rs`. Notable:

- `gitBlame(projectDir, path)` — `git blame --porcelain HEAD` parsed via
  `parseBlamePorcelain`. Powers the editor blame overlay.
- `gitCommitAll(projectDir, message, options?)` — `git add -u` (or
  `git add -A` when `options.includeUntracked` is true) then
  `git commit -m message`, returning the new sha. Only used by the
  Files-panel commit dialog — the runtime never calls it elsewhere
  and no MCP tool invokes git commits. Commits not started from the
  Files dialog are user-driven via `git commit` in the terminal.
- `listBranchChanges`, `getGitLog`, `getCommitDetail`, `getChangeScopes`,
  `searchWorkspaceText`, `restorePath`, `addPath`, `appendToGitignore`,
  `listFileCommits`, `listAllRefs`,
  `readFileAtRef`, `listGitStatuses` — straight `execFileSync` wrappers
  exposed via IPC for UI consumption.
- `gitPush` / `gitPull` / `gitMerge` / `gitRebase` ship sync wrappers
  plus async siblings `gitPushAsync` / `gitPullAsync` / `gitMergeAsync` /
  `gitRebaseAsync` (and a `gitFetchAsync` helper) backed by
  `child_process.execFile` + `promisify`. The runtime IPC handlers
  use the async variants so the main process stays responsive during
  the network or merge work, and they register a row with the
  `BackgroundTaskStore` so the bottom-bar `BackgroundTaskIndicator`
  shows progress. The sync wrappers stay around for code paths that
  haven't been promoted yet (e.g. `gitCommitAll`'s internal calls,
  unit tests).
- `getGitLog` accepts an `all` option (defaults `true`). Pass
  `{ all: false }` to drop `--all` so the log only walks commits
  reachable from `HEAD`'s branch — used by the Git Dashboard's
  "Recent commits" card so the graph stays scoped to the current
  branch.
- `getAheadBehind(projectDir, base, head?)` — wraps
  `git rev-list --left-right --count base...head` and returns
  `{ ahead, behind }` relative to `base`. `head` defaults to `HEAD`.
  Powers the Git Dashboard branch header and worktree rows.
- `getCommitsAheadOf(projectDir, base, head, limit=50)` — wraps
  `git log base..head` with the same parser used by `getGitLog`, for
  pairwise commit-diff displays.
- `listRecentRemoteBranches(projectDir, limit=20)` — wraps
  `git for-each-ref --sort=-committerdate refs/remotes` and returns
  `RemoteBranchEntry[]` (filters out `<remote>/HEAD`). Drives the
  dashboard's recent-remote-branches card.
- `gitPushCurrentTo` / `gitPushCurrentToAsync(projectDir, remote, branch)`
  — runs `git push <remote> HEAD:refs/heads/<branch>`. Refspec push;
  never touches any local working dir. The runtime IPC handler uses
  the async variant + `BackgroundTaskStore`.
- `gitPullRemoteIntoCurrent(projectDir, remote, branch)` — fetches
  `<remote>/<branch>` then merges it into the current branch of
  `projectDir`. Fetch failure short-circuits the merge.

### Cross-worktree push: deliberately unsupported

There is no helper that pushes the active stream's commits *into*
another worktree's branch. Every available path mutates the other
worktree:

- `git push <other-worktree-path> <branch>` is refused by default for
  the currently-checked-out branch (`receive.denyCurrentBranch`).
- `git merge` / `git pull` inside the other worktree obviously
  mutates its working dir.
- `git update-ref` from our side advances the ref but leaves the
  other worktree's HEAD/index/working tree divergent — it then
  silently appears "dirty".

The supported direction is the inverse: from the other stream, the
Git Dashboard's worktrees card lists *our* branch with a
"Merge into current" action so a human in that stream pulls our
commits in safely. Tests pin this invariant: the gitMerge sibling-
worktree test in `crates/oxplow-git/src/lib.rs` (`#[cfg(test)] mod tests`) asserts byte-equal HEAD,
status, and file content on the sibling after merging *its* branch
into the primary.

`isGitRepo` requires the project root *itself* to be the git toplevel —
nested git repos and parent-dir lookups are explicitly refused (see
`architecture.md`'s "Workspace isolation rule"). `isGitWorktree` rejects
secondary worktrees so oxplow won't try to nest its own worktrees inside
another tool's checkout.

## UI commit affordance

The Files panel (`ProjectPanel`) shows a **Commit (N)** button in its
header toolbar whenever `gitEnabled && uncommittedPaths.length > 0`.
Clicking it opens a small `CommitDialog` with a commit-message
textarea; submitting runs `gitCommitAll` through a dedicated
`oxplow:gitCommitAll` IPC method. This is the UI entry point for
user-driven commits. The agent doesn't drive commits — the Stop-hook
emits no commit directives.

Button carries `data-testid="files-commit"`; the dialog's message
textarea is `files-commit-message` and the submit button is
`files-commit-submit`.

### Non-writer threads still cannot call git

`NON_WRITER_PROMPT_BLOCK` (`crates/oxplow-runtime/src/write_guard.rs`) explicitly
forbids git mutations for non-writer threads — they share the
worktree with the writer and any ref/index change corrupts the
writer's in-progress work. The write-guard hook denies Write/Edit/
MultiEdit/NotebookEdit in those threads, and the prompt block covers
Bash (which the hook can't classify reliably).

## Related

- [data-model.md](./data-model.md) — schema overview.
- [agent-model.md](./agent-model.md) — Stop-hook pipeline (no commit
  branches; commits are user-driven).
- [editor-and-monaco.md](./editor-and-monaco.md) — blame overlay UI.
