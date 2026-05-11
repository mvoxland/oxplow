# Local History

Every time the agent works on a task, oxplow snapshots
the files it touches — before and after. The collected
snapshots are **Local History**: a per-file, per-effort
timeline, independent of git, that lets you see exactly what
changed and roll any individual file back without disturbing
the rest.

## Where snapshots come from

A snapshot is captured when:

- A file is about to be written by the agent (the "before"
  snapshot).
- The effort closes and the file is in a different state than
  its before-snapshot (the "after" snapshot).

Snapshots are stored under `.oxplow/` and attributed to the
**effort** that produced them.

## Effort

An effort is one open-and-close cycle of a task. If you
flip a `done` item back to `in_progress` to redo it, that
opens a *second* effort on the same item. The Local History
page groups snapshots by effort, so you can see the full
history of attempts against a single concern.

When the agent closes an item, it passes the list of
repo-relative paths it edited, so attribution is precise even
when multiple tasks run in parallel. If that list is
omitted, oxplow falls back to "assume all" (every file modified
during the effort).

## Opening Local History

- The **Local History** page in the rail's Pages directory
  (project-wide, grouped by item).
- Right-click a file → **Local History** (filtered to that
  file).
- A task's kebab → **Local History** (every file the
  efforts on that item touched).

The page shows a chronological list. Each row is a snapshot:
file path, timestamp, effort it belongs to, and the task
title.

## Comparing and restoring

Click a snapshot to open the diff against the current state.
Click **Restore** to overwrite the current file with the
snapshot's contents.

Restoring is targeted — only that file is affected. The rest
of your working tree is untouched.

This is the main "undo" path for agent work. It is *not* the
same as `git revert` or `git reset`:

- `git reset` rewinds your *whole* working tree (and history,
  if you're not careful).
- Restoring from Local History rewinds *one file* to a known
  state, with no impact on git history.

Use git for committed history. Use Local History for the
working-tree shape between commits.

## Cleanup

Snapshots accumulate. Old efforts under closed items are
pruned on a schedule (configurable; defaults are sensible).
If you want to keep a snapshot indefinitely, copy the file
out — pruning won't ask before it removes data.
