-- Replace the unused `summary` column on streams with a real
-- `custom_prompt` column. Streams previously routed the per-stream
-- prompt feature through the summary slot; now it has its own column
-- and is read by `agent_prompt::assemble_system_prompt`.

ALTER TABLE streams DROP COLUMN summary;
ALTER TABLE streams ADD COLUMN custom_prompt TEXT;
