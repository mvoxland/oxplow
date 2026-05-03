-- V7: Add dir_refs_json column to wiki_page so directory wikilinks
-- ([[src/components/]]) are indexed alongside file_refs / related_notes.
-- Backlinks queries are JSON-string contains checks, mirroring how
-- file_refs / related_notes already work.

ALTER TABLE wiki_page ADD COLUMN dir_refs_json TEXT NOT NULL DEFAULT '[]';
