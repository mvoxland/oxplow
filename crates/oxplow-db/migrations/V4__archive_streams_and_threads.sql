-- The stream-rail "Remove…" action soft-deletes a stream and its
-- threads instead of dropping rows, so history (closed efforts,
-- file_snapshots, page_visit attribution) doesn't dangle. Add a
-- nullable `archived_at` to both tables; readers filter to
-- `archived_at IS NULL` so existing surfaces ignore archived rows
-- without code changes elsewhere. Pre-migration rows stay visible.

ALTER TABLE streams ADD COLUMN archived_at TEXT;
ALTER TABLE threads ADD COLUMN archived_at TEXT;
CREATE INDEX idx_streams_archived ON streams(archived_at);
CREATE INDEX idx_threads_archived ON threads(archived_at);
