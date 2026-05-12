-- Pin a snapshot to a git commit when the worktree was clean at
-- capture time. The capture layer sets this immediately after
-- `request_snapshot` inserts the parent row, but only if
-- `git status --porcelain` is empty (gitignored files don't count
-- — they're excluded by default). When the worktree is dirty the
-- column stays NULL.
ALTER TABLE snapshot ADD COLUMN git_commit TEXT;
