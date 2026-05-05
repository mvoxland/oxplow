-- Tag every code-quality scan with the tree version (and optional file
-- filter) it ran against. Without this, the duplication scan was
-- implicitly working-tree-only: results from a scan over `HEAD` would
-- be indistinguishable from results over the working tree, and a
-- commit-target analysis page couldn't tell which scan to surface.
--
-- All columns are nullable here for the migration; the runtime always
-- writes them. Existing rows are backfilled to ('disk', null, 'all'),
-- which matches their original semantics.
ALTER TABLE code_quality_scan
    ADD COLUMN tree_version_kind TEXT;
ALTER TABLE code_quality_scan
    ADD COLUMN tree_version_value TEXT;
ALTER TABLE code_quality_scan
    ADD COLUMN file_filter TEXT;

UPDATE code_quality_scan SET tree_version_kind = 'disk' WHERE tree_version_kind IS NULL;
UPDATE code_quality_scan SET file_filter = 'all' WHERE file_filter IS NULL;

CREATE INDEX idx_code_quality_scan_version
    ON code_quality_scan(tool, tree_version_kind, tree_version_value, file_filter);
