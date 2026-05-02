-- agent_status was a per-instance transient state cache that recovery
-- reset to "stopped" on every boot anyway, so persisting it added a
-- sync surface to drift from without delivering durability. Status is
-- now derived from the in-memory hook ring buffer
-- (oxplow_app::thread_runtime::ThreadRuntimeRegistry).
--
-- hook_event recorded the verbatim envelopes Claude Code's plugin
-- POSTs at us. They drive state changes when they fire and are
-- uninteresting after; main models them as an in-memory ring per
-- stream and we now do the same.
--
-- agent_turn stays — it's the durable record of agent turns, useful
-- for historical reporting and effort attribution.

DROP INDEX IF EXISTS idx_agent_status_state;
DROP TABLE IF EXISTS agent_status;

DROP INDEX IF EXISTS idx_hook_event_kind_time;
DROP INDEX IF EXISTS idx_hook_event_thread_time;
DROP TABLE IF EXISTS hook_event;
