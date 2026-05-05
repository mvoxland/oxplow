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

`FunctionMetrics.visibility` (`Public`/`Private`/`Unknown`, surfaced
on the IPC as `"public"`/`"private"`/`"unknown"`) is a heuristic
public-or-private classification per language: Rust looks for a
`visibility_modifier` child; TS/JS uses `accessibility_modifier`,
`#`-prefixed names, or the enclosing class/`export_statement` for
top-level functions; Java reads the `modifiers` child; C++ tracks the
preceding `access_specifier` within the enclosing class/struct (class
default = private, struct default = public); Go uses identifier
capitalization; Python uses the leading-underscore convention; C
treats `static` storage class as private. The Change Analysis
Semantic view drives a "Show private" toggle from this field
(default on) and colors the function glyph by visibility.

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
`oxplow-code-dup`. **Function-anchored AST subtree-hash detector
(Deckard-style).** Pipeline:

1. Walk the tree-sitter AST of each file, find every function-like
   node (per `Language::spec().function_kinds` — covers Rust
   `function_item` / `closure_expression`, JS/TS function /
   arrow-function / method, Python / Go / Java / C / C++
   equivalents). **Code outside any function body is not in the
   corpus.** This is deliberate — top-level `const` style objects,
   `enum` declarations with thiserror derives, JSX expression trees,
   schema literals, etc. share AST shape across unrelated files,
   and were the dominant false-positive class of the prior detector.
2. For each function node, hash the function body subtree AND every
   sub-subtree large enough to seed a meaningful match. Hash =
   64-bit fold of preorder-normalized kind sequence: identifiers,
   numeric literals, and strings fold to placeholders (`ID`, `NUM`,
   `STR`); imports / use / include / package declarations are
   skipped whole-subtree; comments are skipped; cross-language
   collisions are prevented by salting with `Language::tag()`.
3. Group records by hash. For each (function-A, function-B) pair
   that shares any matching subtree, emit ONE finding for the
   largest matching subtree between them — so a whole-function
   clone subsumes the inner-loop and inner-branch matches that
   would otherwise pile up.
4. Filter by `min_lines` (default 5) and `min_nodes` (default 30
   AST nodes). The line floor is aggressive on purpose — function-
   anchoring + the node-count floor already filter top-level
   boilerplate and trivial expression subtrees, so the line floor
   doesn't have to do that work too.

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

The result also carries a `churn: Vec<AnalyzedFileChurn>` rollup
— one entry per file where both `base_content` and
`head_content` were supplied. Each rollup has `file_added` /
`file_deleted` totals and a `functions[]` breakdown attributing
added / deleted / modified line counts to the head-side function
whose `[start_line, end_line]` interval contains each line.
Deletions on the base side map to the corresponding head-side
function via qualified-name match
(`container::container::name`); base-only functions count toward
`file_deleted` but produce no per-function row. `modified_lines`
= `min(added_lines, deleted_lines)` per function — a cheap,
explainable "edited both ways" signal.

The diff itself is computed inside the IPC via
`similar::TextDiff::from_lines` (no separate `git diff` invocation
needed). Source: `crates/oxplow-tauri-ipc/src/commands/churn.rs`.

## Change Analysis: interestingness scoring

The dashboard's `LookHereFirstCard` ranks files by a CRAP-flavored
multiplicative score so a single hot factor dominates:

```
sizeFactor      = log2(1 + additions + deletions)
complexitySpike = sum(complexityDelta where >0) across this file's modifiedBody
paramSpike      = sum(after-before where >0) across modifiedSignature
longNewFn       = max(0, max(added.length where length>60) - 60) / 40
untestedMul     = hasMatchingTest ? 1.0 : 1.5

base    = 1 + sizeFactor
spike   = (1 + 0.6 * complexitySpike) * (1 + 0.4 * paramSpike) * (1 + longNewFn)
score   = base * spike * untestedMul
```

Each multiplier ≥ 1.2 contributes a hover-readable `reason` —
"complexity +14 across 3 fns", "no test in same dir", etc. All
weights live in `INTERESTINGNESS_WEIGHTS`
(`apps/desktop/src/components/ChangeAnalysis/interestingness.ts`)
so they're tuneable from one place.

Per-function variant `functionInterestingness` uses the same
shape but with churn lines + length on a single function. Used
by `FunctionChurnCard` for tiebreak ordering.

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
