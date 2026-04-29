-- Initial oxplow schema (Rust rewrite).
--
-- This collapses the 50 incremental migrations from the prior TS
-- implementation into a single initial-state schema. The Electron
-- → Tauri rewrite is a clean break — there is no upgrade path from
-- the old SQLite DB. Users start fresh.
--
-- Naming note: the legacy schema used "batch" / "batches" for what
-- the domain model now calls "thread". The Rust rewrite uses the
-- domain name end-to-end, so tables are renamed accordingly.

PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

CREATE TABLE streams (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('primary', 'worktree')),
    title TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    branch TEXT NOT NULL,
    branch_ref TEXT NOT NULL,
    branch_source TEXT NOT NULL,
    worktree_path TEXT NOT NULL,
    working_pane TEXT NOT NULL DEFAULT '',
    talking_pane TEXT NOT NULL DEFAULT '',
    working_session_id TEXT NOT NULL DEFAULT '',
    talking_session_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_streams_one_primary ON streams(kind) WHERE kind = 'primary';
CREATE INDEX idx_streams_branch ON streams(branch);

CREATE TABLE runtime_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    current_stream_id TEXT REFERENCES streams(id) ON DELETE SET NULL
);
INSERT INTO runtime_state (id, current_stream_id) VALUES (1, NULL);

CREATE TABLE threads (
    id TEXT PRIMARY KEY,
    stream_id TEXT NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'queued', 'closed')),
    sort_index INTEGER NOT NULL DEFAULT 0,
    pane_target TEXT NOT NULL DEFAULT 'working',
    resume_session_id TEXT NOT NULL DEFAULT '',
    summary TEXT NOT NULL DEFAULT '',
    summary_updated_at TEXT,
    closed_at TEXT,
    custom_prompt TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_threads_stream_sort ON threads(stream_id, sort_index);
-- At most one ACTIVE (writer) thread per stream — mirrors the TS
-- invariant. Other threads sit in the queued bucket.
CREATE UNIQUE INDEX idx_threads_one_active_per_stream
    ON threads(stream_id) WHERE status = 'active';

CREATE TABLE thread_selection (
    stream_id TEXT PRIMARY KEY REFERENCES streams(id) ON DELETE CASCADE,
    selected_thread_id TEXT REFERENCES threads(id) ON DELETE SET NULL
);

CREATE TABLE work_items (
    id TEXT PRIMARY KEY,
    -- Nullable: null means the item is on the project-wide backlog.
    thread_id TEXT REFERENCES threads(id) ON DELETE CASCADE,
    parent_id TEXT REFERENCES work_items(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('epic', 'task', 'subtask', 'bug', 'note')),
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    acceptance_criteria TEXT,
    status TEXT NOT NULL CHECK (status IN ('ready', 'in_progress', 'blocked', 'done', 'canceled', 'archived')),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high', 'urgent')),
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_by TEXT NOT NULL CHECK (created_by IN ('user', 'agent', 'system')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT,
    deleted_at TEXT,
    -- Semantic origin (vs. created_by which is the writer).
    author TEXT CHECK (author IN ('user', 'agent')),
    category TEXT,
    tags TEXT
);
CREATE INDEX idx_work_items_thread_parent ON work_items(thread_id, parent_id, sort_index);
CREATE INDEX idx_work_items_thread_status ON work_items(thread_id, status, sort_index);
CREATE INDEX idx_work_items_thread_deleted ON work_items(thread_id, deleted_at, sort_index);
CREATE INDEX idx_work_items_backlog ON work_items(deleted_at, sort_index) WHERE thread_id IS NULL;

CREATE TABLE work_item_links (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    from_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    to_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    link_type TEXT NOT NULL CHECK (link_type IN ('blocks', 'relates_to', 'discovered_from', 'duplicates', 'supersedes', 'replies_to')),
    created_at TEXT NOT NULL,
    CHECK (from_item_id <> to_item_id)
);
CREATE INDEX idx_work_links_thread_from ON work_item_links(thread_id, from_item_id);
CREATE INDEX idx_work_links_thread_to ON work_item_links(thread_id, to_item_id);

CREATE TABLE work_item_events (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    item_id TEXT REFERENCES work_items(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('user', 'agent', 'system')),
    actor_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_work_events_thread_item ON work_item_events(thread_id, item_id, created_at);

CREATE TABLE work_notes (
    id TEXT PRIMARY KEY,
    work_item_id TEXT REFERENCES work_items(id) ON DELETE CASCADE,
    thread_id TEXT REFERENCES threads(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    author TEXT NOT NULL,
    created_at TEXT NOT NULL,
    -- Mutually exclusive: a note is attached to either a work item or
    -- a thread, never both, never neither.
    CHECK (
        (work_item_id IS NOT NULL AND thread_id IS NULL)
        OR (work_item_id IS NULL AND thread_id IS NOT NULL)
    )
);
CREATE INDEX idx_work_notes_item ON work_notes(work_item_id, created_at);
CREATE INDEX idx_work_notes_thread ON work_notes(thread_id, created_at);

CREATE TABLE agent_turn (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    work_item_id TEXT REFERENCES work_items(id) ON DELETE SET NULL,
    prompt TEXT NOT NULL,
    answer TEXT,
    session_id TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT
);
CREATE INDEX idx_agent_turn_thread ON agent_turn(thread_id, started_at DESC);
CREATE INDEX idx_agent_turn_item ON agent_turn(work_item_id, started_at DESC);
CREATE INDEX idx_agent_turn_open ON agent_turn(thread_id) WHERE ended_at IS NULL;
