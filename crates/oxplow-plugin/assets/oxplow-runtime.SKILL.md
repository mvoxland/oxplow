---
name: oxplow-runtime
description: Oxplow runtime — task filing, status transitions, and orchestrator dispatch. Loads on mcp__oxplow__create_task, file_epic_with_children, update_task, add_work_note, read_task_options, or dispatch_task calls, and when composing a subagent brief.
---

# Filing oxplow tasks

Active agent turns render as live rows in the Work panel passively —
no synthesized tasks. File durable tasks explicitly when
you want to:

- Split pre-planned or multi-phase work into an epic + children
  (`file_epic_with_children`).
- Pre-queue work the user wants done in a later turn (`create_task`).
- Record a follow-up you noticed but can't fix right now.

## Task vs epic

Pick by structure, not by whether the work was planned first. Plenty
of plan-mode outputs describe a single task.

- **`create_task` with `kind: "task"`** — one coherent change,
  even if it touches a few files. Rename, bug fix, small feature in one
  subsystem. Sequential chores (edit → typecheck → test) are still one
  task, not sub-steps.
- **`file_epic_with_children`** — ≥3 sub-steps a reviewer would
  naturally check off independently: distinct phases, clear handoffs,
  or separable subsystems (e.g. schema → runtime → IPC → UI → docs).
  Each child closes to `done` on its own as it ships.
- Decision test: could a single child close to `done` and
  have the user meaningfully inspect just that piece? If yes, epic.
  If no, it's a task and the bullets are just an execution outline.
- Don't retroactively wrap a task in an epic mid-execution if it turns
  out to be small — just finish it.

## Shaping the row

- `title`: imperative, ≤60 chars (`Fix login redirect loop`).
- `description`: what and why; keep it terse.
- `acceptanceCriteria`: one observable criterion per line.
- `kind`: `epic` only with children (use `file_epic_with_children`);
  otherwise `task`.
- `priority`: `medium` unless the user signalled otherwise.

## One QA-separate concern per row

Siblings under an epic still need to be independently reviewable:
two things a reviewer would accept/reject separately go in two child
tasks, not one "misc" child. Same rule as top-level items.

# task transitions

Mark an explicit item `in_progress` when you start executing it and
`done` (via `update_task` or `complete_task`) when
you finish. Use `blocked` for items parked on user input.

**Close the row in the same turn the work actually ships.** An
`in_progress` row with finished work parked in it looks stuck to the
user. Call `complete_task` the moment the code change lands —
don't wait for a later turn.

**Pass `touchedFiles` when you close.** `complete_task`,
`update_task`, and `create_task` all accept an optional
`touchedFiles: string[]` of repo-relative paths you edited for this
effort. The runtime attaches them to the closing effort so Local
History can attribute writes to this specific item when multiple
items ran in parallel. Skip only if you edited >100 files (the
assume-all fallback handles big change sets).

For retroactive splits or "file and close in one call" rows (where
the edits already shipped and you just want a durable row with
attribution), pass `touchedFiles` directly into `create_task`
along with `status: "done"` (or `"blocked"`) — the server
synthesizes the `in_progress → target` transition so attribution
lands exactly as it would for a normal close. Without
`touchedFiles`, items filed directly into `done` never open
an effort, so attribution is impossible; the Local History panel
falls back to "assume all" for that item.

Legitimate reasons to *stay* `in_progress` across a stop boundary:

- You have a question the user must answer before you can finish.
- The work is genuinely multi-turn and you're pausing partway through.

In either case, leave a note (`add_work_note`) explaining what's
pending so the stop-hook nudge suppresses itself — it only fires for
items the agent didn't touch during the turn.

## Talking about items in chat

When you mention a task to the user, refer to it by its quoted
title (e.g. `"Fix login redirect loop"`), **never** by its `id`
id. The id is an internal handle for tool calls; the user doesn't see
it in their UI and won't know what you're pointing at. This applies
everywhere: confirming a fix, asking whether to proceed, summarizing
what shipped, naming the item you just reopened, etc.

## Redos on a just-shipped item

When the user pushes back on work you just closed to `done`
(asks you to fix, redo, revert, or take a different approach to the
same concern), **reopen the existing item** — don't file a new one.

Flow:

1. `update_task` the item back to `in_progress` (this opens a
   fresh effort; the `done → in_progress` transition is the documented
   reopen path).
2. Do the new round of edits.
3. `complete_task` back to `done` with `touchedFiles` for the new
   effort.

The item row gets a second effort recording the redo, attributed
correctly. Filing a new "Fix the thing I just did" task fragments the
history and makes the Work panel lie about how many concerns the user
actually raised. A *new* concern still gets a new item — the rule is
scoped to "user rejected my last attempt at this same item."

# Dispatch mode

- **Inline**: small fixes (≤20 lines, ≤2 files, no risk). Orchestrator
  edits directly.
- **Subagent**: anything bigger or risky. Call
  `mcp__oxplow__dispatch_task({threadId, itemId})` to get a ready
  brief; pass `prompt` to the general-purpose Agent tool. The brief
  already contains the item fields, AC, recent notes, and the
  subagent protocol preamble.

Subagents return a one-line `oxplow-result: { ok, itemId, … }`.
Record that as a work note via `add_work_note`.
