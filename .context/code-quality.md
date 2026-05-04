# Code quality scans

Native, in-process complexity + duplicate-block detection. Both
analysis kinds run directly inside the Rust process via tree-sitter
— no subprocess, no Python or Node dependency, nothing for the user
to install.

The store and IPC contract speak in two analysis kinds — `metrics`
and `duplication` — which is the dimension users pick from in the
panel.

## What gets measured

**Per-function metrics** (tool name `"metrics"`) — handled by
`oxplow-code-metrics`. For each function in each scanned file we
emit three findings:

- `complexity` — cyclomatic complexity (decision-point count + 1).
- `function-length` — line count of the function body.
- `parameter-count` — number of declared parameters.

`extra.functionName` carries the function identifier so the UI can
group all three back together.

`FunctionMetrics.container_path` (and `AnalyzedFunction.container_path`
on the IPC surface) carries the outer-to-inner names of the named-
declaration ancestors a function lives inside (class / impl / trait /
mod / namespace / interface / enum / record). The Change Analysis
Functions card uses it to render a `path > container > … > function`
tree so the user can scan high-level constructs first and drill in.
Top-level functions report an empty `container_path`. The set of
container kinds is per-language — `LanguageSpec.container_kinds` plus
`container_name_fields` in `crates/oxplow-code-metrics/src/spec.rs`.
Go and C have no class-like containers and use an empty list.

Languages: Rust, TypeScript (incl. TSX), JavaScript, Python, Go,
Java, C, C++. Adding a language is one entry in
`crates/oxplow-code-metrics/src/spec.rs` listing the function /
parameter / decision-point / container AST node names plus a grammar
loader. Files in unsupported languages are silently skipped.

**Duplicate blocks** (tool name `"duplication"`) — handled by
`oxplow-code-dup`. Pipeline:

1. Walk the tree-sitter AST of each file, emitting a normalized
   token per leaf (identifiers / numeric literals / strings are
   folded to placeholder kinds so renames and constant tweaks don't
   suppress matches; comments are skipped).
2. Compute rolling 64-bit hashes over k-grams of `K=20` tokens.
3. Winnow with window `W=4` (Schleimer/Aiken 2003) — keep one
   fingerprint per ~5 tokens.
4. Build an inverted index of fingerprint → occurrences and extend
   each multi-occurrence fingerprint forward into the longest
   matching contiguous run.
5. Emit one duplicate per pair where the line span is at least
   `min_lines = 5`.

Output is two `duplicate-block` findings per pair (one per side)
with `extra.peerPath` / `extra.peerStartLine` / `extra.peerEndLine`
so the panel can render the cross-reference inline.

## Normalized finding shape

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

Both runners (`run_metrics_scan` / `run_duplication_scan` in
`crates/oxplow-app/src/code_quality_runner.rs`) produce this shape
directly. The store and the panel UI are tool-agnostic — adding a
third analysis kind only requires defining its `kind` strings.

## Scope: codebase vs diff

Scans run in one of two scopes:

- `codebase` — the runner walks every supported file under the
  project root (skipping `.git`, `target`, `node_modules`, `dist`,
  `build`, and dotdirs).
- `diff` — caller passes a file list (typically from
  `listBranchChanges`); the runner only reads those.

Both scopes are persisted independently per `(stream, tool)`, so
the panel can show "what's complex / duplicated in the whole repo"
and "in just my branch's changes" at the same time without one
overwriting the other.

## `analyze_functions_at_refs` — before/after metrics for Change Analysis

The Change Analysis Dashboard
(`apps/desktop/src/pages/ChangeAnalysisPage.tsx`) needs per-function
metadata at *both* the base and head sides of a diff to bucket
functions into added / deleted / signature-changed / body-changed.

The IPC command `analyze_functions_at_refs`
(`crates/oxplow-tauri-ipc/src/commands/code_quality.rs`) takes a
list of `{ path, base_content, head_content }` specs and calls
`oxplow_code_metrics::analyze_file` directly per side. No tempdir,
no subprocess, no install dependency.

This is **not** persisted — every call re-analyses the provided
contents. It's also **separate from the scan store**: results do
not appear in the Code Quality panel or share scan IDs. Callers
that want persistent rollups should use `runCodeQualityScan`
instead.


## Adding a third analysis kind

1. New crate (or new module) producing `CodeQualityFinding`
   records with a fresh `kind` string.
2. Add a branch in `Services.runCodeQualityScan` (more precisely:
   the `match tool.as_str()` in
   `crates/oxplow-tauri-ipc/src/commands/code_quality.rs`).
3. Add the tool to the `TOOLS` array in
   `apps/desktop/src/components/CodeQuality/CodeQualityPanel.tsx`
   so the Run button renders.

No migration needed; the existing tables don't care which runner
produced a finding as long as the `kind` is recognized.

## Performance notes

Both runners punt their CPU-bound work to a `tokio::task::spawn_blocking`
pool so they don't stall the runtime on large repos. Rough
ballpark on the oxplow checkout (~2k source files): metrics scan
~0.5s, duplicate scan ~2s. Big jumps suggest a tunable
(`DupOptions { k, w, min_lines }`) needs adjusting.
