-- Persist the human-readable label for each page_visit so RailHud
-- History shows the same string the tab strip showed at activation
-- time. Previously the label was synthesized at read time from
-- kind+id (deriveDefaultLabelFromKind in the renderer) which produced
-- generic strings like "Git Commit" disconnected from the rich tab
-- title (commit subject, page title, etc.). Existing rows get a
-- NULL label; the renderer falls back to page_id for those.

ALTER TABLE page_visit ADD COLUMN label TEXT NULL;
