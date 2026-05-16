-- Version-track every file reference so we can tell how out-of-date
-- a touched-file row or a wiki -> file edge is.
--
-- Two new tables grow these columns:
--   * `task_effort_file`  — per-effort change attribution. Every row
--     IS a file ref, so all three columns are NOT NULL.
--   * `page_ref`          — the cross-page edge graph. Only edges
--     whose target_kind = 'file' (or 'directory' / 'git_commit' if
--     we ever extend to those) carry a meaningful version; the rest
--     leave the columns NULL. We keep them nullable on this table
--     so we don't have to invent fake snapshot ids for the
--     overwhelming majority of edges.
--
-- Columns (consistent shape across both tables):
--   * `local_snapshot_id`   — the `snapshot.id` the reference was
--     captured against. The capture service guarantees a snapshot
--     row exists at capture time.
--   * `closest_git_version` — git commit sha the snapshot is closest
--     to. Filled at capture time with the snapshot's `git_commit`
--     if the worktree was clean (then `git_version_exact = 1`), or
--     with HEAD otherwise (`git_version_exact = 0`). NULL when no
--     git information is available at all.
--   * `git_version_exact`   — 1 when the local snapshot is byte-equal
--     to the recorded commit. Flipped to 1 by the snapshot store
--     whenever `set_snapshot_git_commit` is called on a snapshot
--     that already has refs pointing at it.
--
-- Backfill: existing `task_effort_file` rows pick up the effort's
-- end_snapshot_id (or start_snapshot_id if no end was recorded). A
-- defaulted 0 stays only for rows whose effort has neither snapshot
-- pinned — those are pre-V13 ghosts and we don't care about them.

ALTER TABLE task_effort_file ADD COLUMN local_snapshot_id INTEGER NOT NULL DEFAULT 0;
ALTER TABLE task_effort_file ADD COLUMN closest_git_version TEXT;
ALTER TABLE task_effort_file ADD COLUMN git_version_exact INTEGER NOT NULL DEFAULT 0;

UPDATE task_effort_file
   SET local_snapshot_id = COALESCE(
         (SELECT e.end_snapshot_id   FROM task_effort e WHERE e.id = task_effort_file.effort_id),
         (SELECT e.start_snapshot_id FROM task_effort e WHERE e.id = task_effort_file.effort_id),
         0
       );

ALTER TABLE page_ref ADD COLUMN local_snapshot_id INTEGER;
ALTER TABLE page_ref ADD COLUMN closest_git_version TEXT;
ALTER TABLE page_ref ADD COLUMN git_version_exact INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_task_effort_file_snapshot ON task_effort_file(local_snapshot_id);
CREATE INDEX idx_page_ref_snapshot         ON page_ref(local_snapshot_id);
