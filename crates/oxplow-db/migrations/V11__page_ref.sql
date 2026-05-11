-- V11: Unified cross-page reference graph.
--
-- Every "X points at Y" relationship across page kinds (wiki ->
-- file, task -> wiki, commit -> task, …) lives here as
-- one row. Per-subsystem writers own their `source_kind` rows
-- (delete-then-insert on save). A single reader joins on
-- (target_kind, target_id) for backlinks and (source_kind,
-- source_id) for outbound.
--
-- `kind` is denormalised next to `id` so kind-filtered queries
-- ("all backlinks where target is a file") don't need LIKE on a
-- combined "kind:id" column. Canonical ids match the frontend's
-- TabRef.id shape (e.g. "wiki:architecture", "task:42",
-- "file:src/app.rs", "git-commit:abc123").
--
-- `ref_type` records HOW the source points at the target so the
-- reader can render a useful label and the writer can replace just
-- one slice of edges (e.g. only the parsed-message edges of a
-- commit, leaving the touched-file edges alone).

CREATE TABLE page_ref (
  source_kind  TEXT NOT NULL,
  source_id    TEXT NOT NULL,
  target_kind  TEXT NOT NULL,
  target_id    TEXT NOT NULL,
  ref_type     TEXT NOT NULL,
  source_extra TEXT,
  PRIMARY KEY (source_kind, source_id, target_kind, target_id, ref_type)
);

CREATE INDEX idx_page_ref_target ON page_ref(target_kind, target_id);
CREATE INDEX idx_page_ref_source ON page_ref(source_kind, source_id);
