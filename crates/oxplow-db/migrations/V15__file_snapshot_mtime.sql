-- Add mtime to `file_snapshot` so the startup sweep can short-circuit:
-- if a file's current (size, mtime_ms) matches the latest snapshot's,
-- the bytes are presumed identical and we skip the read + sha pass.
-- NULL for rows captured before V15 — the sweep falls back to hashing
-- those, which is correct (just no faster than the old behavior).
ALTER TABLE file_snapshot ADD COLUMN mtime_ms INTEGER;
