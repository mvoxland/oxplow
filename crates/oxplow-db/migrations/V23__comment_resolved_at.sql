-- When a comment was last moved to `resolved` (NULL while open).
-- Distinct from updated_at (auto re-anchoring bumps it) and
-- last_activity_at (messages only), so it's the only reliable signal
-- for the Comments Dashboard's "resolved in the last N days" buckets.
-- See .context/data-model.md.
ALTER TABLE comment ADD COLUMN resolved_at TEXT;

-- Best-effort backfill for rows resolved before this column existed:
-- updated_at is the closest available approximation of resolve time.
UPDATE comment SET resolved_at = updated_at WHERE status = 'resolved';
