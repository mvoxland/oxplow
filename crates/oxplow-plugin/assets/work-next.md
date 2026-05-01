---
description: Pick up the next ready oxplow work item and dispatch it.
---

Call `mcp__oxplow__read_work_options` for this thread and dispatch
the resulting unit to a `general-purpose` subagent per the
`oxplow-runtime` skill. The skill carries the protocol (mark
`in_progress` before work, `done` after, never two items
`in_progress` at once); follow it.

If the tool returns `{ mode: "empty" }` there's nothing ready —
report that and stop.
