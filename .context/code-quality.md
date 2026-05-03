# Code quality scans


Deterministic, language-agnostic flagging of complexity hotspots and
duplicated code, driven by external CLIs (`lizard` and `jscpd`) so
oxplow doesn't have to maintain per-language metric definitions.

This is a deliberate first-iteration: ship cheap signals that work
across most languages today, learn which ones are useful, and only
then decide whether to invest in a tree-sitter-based custom metric
layer that would give us cross-language consistency at the cost of
more code to maintain.

## What gets measured

**lizard** (cyclomatic complexity, function length, parameter count
— ~20 languages). For each function in the scan target, we emit
three findings:

- `complexity` — cyclomatic complexity number (CCN). Higher = more
  branching paths through the function.
- `function-length` — line count of the function body.
- `parameter-count` — number of declared parameters.

`extra.functionName` carries the function identifier so the UI can
group all three back together; `extra.nloc` carries the
non-comment line count.

**jscpd** (token-based duplicate-block detection — ~150 languages
via its tokenizer set). For each duplicate-pair, we emit two
findings (one per side):

- `duplicate-block` — `metric_value` is the duplicated line count.
  `extra.peerPath` / `extra.peerStartLine` / `extra.peerEndLine`
  point at the other side so the UI can show
  "duplicates X lines from Y:Lstart-Lend" without re-querying.

## Normalized finding shape

The store and IPC contract speak in normalized findings, not raw
tool output:

```ts
interface CodeQualityFinding {
  path: string;          // repo-relative
  startLine: number;
  endLine: number;
  kind: "complexity" | "function-length"
      | "parameter-count" | "duplicate-block";
  metricValue: number;
  extra: Record<string, unknown> | null;
}
```

Subprocess functions (`runLizard`, `runJscpd`) are responsible for
parsing the tool's native format and converting to this shape.
That isolation means adding a third tool only touches the subprocess
module — the store, runtime, IPC, and UI are tool-agnostic.

Parser functions (`parseLizardCsv`, `parseJscpdReport`) are
exported separately from the subprocess runners so they can be
unit-tested without the CLI installed.

## Scope: codebase vs diff

Scans run in one of two scopes:

- `codebase` — pass the project root as the only argument; lizard
  recursively walks the tree, jscpd uses its default discovery.
- `diff` — call `listBranchChanges(worktree, baseRef)` first and
  pass that file list to the tool. Files with status `deleted` are
  filtered out (the tool would error on them). If the diff is
  empty, we skip the subprocess entirely and write a
  zero-findings completed scan.

Both scopes are persisted independently per `(stream, tool)`, so
the panel can show "what's complex in the whole repo" and "what's
complex in just my branch's changes" at the same time without one
overwriting the other.

## Adding a third tool

1. Define a new normalized parser + runner in
   `crates/oxplow-app/src/code_quality_runner.rs` (or split it out — the file
   stays single-purpose for now).
2. Extend the `CodeQualityTool` union in
   `crates/oxplow-db/src/analytics_stores.rs` and
   `crates/oxplow-tauri-ipc/src/commands/` (the union is duplicated
   intentionally — store and contract have separate type
   identities).
3. Add a branch in `Services.runCodeQualityScan` that
   dispatches to the new runner.
4. Add the tool to the `TOOLS` array in
   `apps/desktop/src/components/CodeQuality/CodeQualityPanel.tsx` so the
   "Run" buttons render.

No migration needed; the existing tables don't care which tool
produced a finding as long as the `kind` is recognized.

## `analyze_functions_at_refs` — before/after lizard for Change Analysis

The Change Analysis Dashboard (`apps/desktop/src/pages/ChangeAnalysisPage.tsx`)
needs per-function metadata for each changed file at *both* the base and
head sides of a diff so it can bucket functions into added / deleted /
signature-changed / body-changed. The IPC command
`analyze_functions_at_refs` (in `crates/oxplow-tauri-ipc/src/commands/code_quality.rs`)
takes a list of `{ path, base_content, head_content }` specs, writes
each side to `<tmp>/<side>/<path>` so lizard's extension-driven
language detection still works, runs `lizard --csv` once over the temp
root, then routes findings back to `(side, path, function)` tuples.

This is **not** persisted — every call re-runs lizard against the
provided contents. It's also **separate from the scan store**: results
do not appear in the Code Quality panel or share scan IDs. Callers
that want persistent rollups should use `runCodeQualityScan` instead.

If lizard isn't on PATH, the command returns `{ tool_missing: "lizard",
sides: [] }` rather than erroring, so the renderer can show an inline
install hint without losing the rest of the dashboard.

## Tool installation

Tools are user-installed and assumed to be on `PATH`. `lizard`
ships via pip (`pip install lizard`); `jscpd` ships via npm
(`npm install -g jscpd`). When ENOENT is hit, the runtime
surfaces a friendly "X is not installed" via
`CodeQualityToolMissingError` and writes it to
`code_quality_scan.error_message`; the UI's scan-status strip
shows the message inline.

We don't bundle either tool — keeping subprocess dependencies
optional means a fresh oxplow install works without forcing users
to install Python or another npm global.
