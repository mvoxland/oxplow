---
name: oxplow-subagent-work-protocol
description: Standing protocol for subagents executing a oxplow work item. Loads on any wi-… id in a brief or on mcp__oxplow__update_work_item / add_work_note calls.
---

# Subagent protocol

- Mark the item `in_progress` on entry; `done` on exit.
- Return ONE line: `oxplow-result: {"ok":true,"itemId":"wi-…","…":…}`.
- Keep notes terse: what you did, not how.
- On blocker, set `blocked` and leave a note — do not retry silently.
