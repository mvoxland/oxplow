-- Remove the `acceptance_criteria` column from `task`.
--
-- The field was only consulted in two places: a synthesized
-- "## Acceptance criteria" section in the agent dispatch brief, and
-- the FTS concat in page_ref_projections. Nothing actually validated
-- AC on close, no UI badge surfaced "AC met / unmet", and the user
-- decided the per-task field added more surface area than it bought.
-- Agents are nudged (via the create_task tool docstring) to write an
-- "## Acceptance criteria" subsection inside the task description
-- when it would be helpful — same affordance, simpler model.
--
-- IMPORTANT: do NOT rebuild the task table via a `task_new` +
-- `INSERT … SELECT` + `DROP TABLE task` + `RENAME` dance. The
-- `task_effort.task_id` and `task_link.from_item_id` / `.to_item_id`
-- columns have `REFERENCES task(id) ON DELETE CASCADE`, and with
-- `PRAGMA foreign_keys = ON` (which oxplow sets per-connection),
-- `DROP TABLE task` cascades and wipes every child row. The very
-- first version of this migration did exactly that and silently
-- deleted every effort row and task link on existing dev DBs. Use
-- `ALTER TABLE … DROP COLUMN` instead (SQLite 3.35+, the bundled
-- rusqlite is 3.51+) — it leaves child tables untouched.
--
-- Migration plan:
--   1. Append any non-empty AC value into description as a
--      "## Acceptance criteria" markdown section so nothing is lost.
--   2. ALTER TABLE task DROP COLUMN acceptance_criteria.

UPDATE task
   SET description = CASE
         WHEN description = '' THEN '## Acceptance criteria' || char(10) || char(10) || acceptance_criteria
         ELSE description || char(10) || char(10) || '## Acceptance criteria' || char(10) || char(10) || acceptance_criteria
       END
 WHERE acceptance_criteria IS NOT NULL
   AND length(trim(acceptance_criteria)) > 0;

ALTER TABLE task DROP COLUMN acceptance_criteria;
