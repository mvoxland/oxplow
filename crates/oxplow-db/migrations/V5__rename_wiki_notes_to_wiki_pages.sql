-- Wiki entries are pages, not notes. The earlier "note" naming
-- conflated them with the Explore-subagent thread-scoped notes
-- (a different concept) and with the now-retired per-task
-- notes. Rename every wiki_note* table/index/virtual table to
-- the wiki_page* form so the schema, the Rust types, the IPC
-- surface, and the UI all read consistently.
--
-- FTS5 virtual tables can be renamed with ALTER TABLE; SQLite
-- updates the shadow tables (wiki_note_fts_data,
-- wiki_note_fts_idx, etc.) automatically.

ALTER TABLE wiki_note RENAME TO wiki_page;
-- SQLite doesn't support ALTER INDEX RENAME TO; drop and re-create
-- the index against the renamed table.
DROP INDEX IF EXISTS idx_wiki_note_updated;
CREATE INDEX IF NOT EXISTS idx_wiki_page_updated ON wiki_page(updated_at DESC);
ALTER TABLE wiki_note_fts RENAME TO wiki_page_fts;
ALTER TABLE wiki_note_thread_update RENAME TO wiki_page_thread_update;
