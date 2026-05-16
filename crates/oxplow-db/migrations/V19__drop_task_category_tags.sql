-- Remove the `category` and `tags` columns from `task`.
--
-- Both fields were settable via the rail UI ("+ Add category" /
-- "+ Add tags") and the MCP create/update tools, but `tags` was never
-- read by any frontend surface and `category` was only displayed in
-- the legacy `BacklogDrawer` chip. Nothing populated either field
-- automatically; the Backlog page never grouped or filtered by them
-- despite the doc claim. Keeping the columns just left write-only
-- vestigial state, so they're being deleted with the same
-- `ALTER TABLE … DROP COLUMN` mechanic V18 used for
-- `acceptance_criteria`.
--
-- See V18's header for the cascade-delete trap that bans rebuilding
-- the task table — this migration follows the same drop-column path.

ALTER TABLE task DROP COLUMN category;
ALTER TABLE task DROP COLUMN tags;
