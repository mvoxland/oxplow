import type { BranchChangeEntry, GitFileStatus } from "../../api-types.js";

export type FunctionVisibility = "public" | "private" | "unknown";

export interface AnalyzedFunctionSummary {
  name: string;
  paramCount: number;
  complexity: number;
  length: number;
  startLine: number;
  /** Outer-to-inner container ancestors (class/impl/module/namespace). */
  containerPath: string[];
  /** Heuristic public/private classification per language. */
  visibility: FunctionVisibility;
}

export interface FilePivotRow {
  key: string;
  files: number;
  additions: number;
  deletions: number;
  /** File counts split by status bucket — drives the stacked bar
   *  on the extension pivot. `added` includes untracked; `modified`
   *  includes renamed; `deleted` is just deleted. */
  byStatus: { added: number; modified: number; deleted: number };
}

export interface FilePivots {
  byExtension: FilePivotRow[];
  byTopDir: FilePivotRow[];
  byStatus: Record<GitFileStatus, number>;
}

/** Per-function line churn (from the IPC), keyed by qualified key. */
export interface FunctionChurnRow {
  path: string;
  name: string;
  containerPath: string[];
  startLineHead: number;
  addedLines: number;
  deletedLines: number;
  modifiedLines: number;
}

/** Optional churn decoration on a bucket row. Absent when the file
 *  was not modified-with-content (added/deleted/binary). */
export interface ChurnDecoration {
  addedLines: number;
  deletedLines: number;
  modifiedLines: number;
  /** (added + deleted) / total diff churn — 0 if total is zero. */
  churnPercent: number;
}

export interface FunctionsBuckets {
  added: Array<{
    path: string;
    name: string;
    containerPath: string[];
    startLine: number;
    paramCount: number;
    complexity: number;
    /** Length in lines on the head side. Filled by `diffFunctions`
     *  for added functions; used by interestingness scoring. */
    length: number;
    visibility: FunctionVisibility;
    churn?: ChurnDecoration | null;
  }>;
  deleted: Array<{ path: string; name: string; containerPath: string[]; startLine: number; visibility: FunctionVisibility }>;
  modifiedSignature: Array<{ path: string; name: string; containerPath: string[]; startLine: number; before: number; after: number; visibility: FunctionVisibility }>;
  modifiedBody: Array<{
    path: string;
    name: string;
    containerPath: string[];
    startLine: number;
    complexityDelta: number;
    lengthDelta: number;
    visibility: FunctionVisibility;
    churn?: ChurnDecoration | null;
  }>;
}

const TEST_PATTERNS: RegExp[] = [
  /\.test\.[a-zA-Z]+$/,
  /\.spec\.[a-zA-Z]+$/,
  /(^|\/)tests?\//,
  /(^|\/)test_[^/]+\.(py|rs)$/,
  /_test\.go$/,
  /_test\.cljc?$/, // Clojure / cljc; .cljs not conventionally suffixed
];

/** True if `path` looks like a test file by convention. */
export function isTestPath(path: string): boolean {
  return TEST_PATTERNS.some((re) => re.test(path));
}

/** Per-language function-name conventions for tests:
 *   - Python / Rust: `test_*`
 *   - Go: `Test*` / `Benchmark*` / `Example*` followed by an
 *     uppercase letter or end-of-name.
 *   - Java: `test*` (older convention; @Test annotations aren't
 *     carried in our function metrics so we lean on the prefix).
 *   - JS/TS: `it`, `test`, `describe` blocks live inside test
 *     files, which `isTestPath` already covers.
 */
const TEST_NAME_PATTERNS: RegExp[] = [
  /^test_/, // Python, Rust
  /^test[A-Z_]/, // older Java / JS convention
  /^Test([A-Z_]|$)/, // Go
  /^Benchmark([A-Z_]|$)/,
  /^Example([A-Z_]|$)/,
];

/** Treat any container named `tests`, `test`, `FooTest`,
 *  `FooTests`, or `Test*` as a test container. The Rust idiom
 *  `#[cfg(test)] mod tests { #[test] fn parses() { ... } }`
 *  produces functions whose name doesn't match the prefix
 *  conventions but whose containerPath includes "tests". */
function isTestContainer(name: string): boolean {
  if (name === "tests" || name === "test") return true;
  if (/Tests?$/.test(name)) return true;
  if (/^Test([A-Z_]|$)/.test(name)) return true;
  // Clojure namespace convention: `foo.bar-test`
  if (/-test$/.test(name)) return true;
  return false;
}

/** True if a function should be classified as a test — either it
 *  lives in a test file, its name matches a per-language test
 *  convention, or any of its container ancestors is a test
 *  module/class (Rust `mod tests`, Java `FooTest` class, etc.). */
export function isTestFunction(
  path: string,
  name: string,
  containerPath: readonly string[] = [],
): boolean {
  if (isTestPath(path)) return true;
  if (TEST_NAME_PATTERNS.some((re) => re.test(name))) return true;
  if (containerPath.some(isTestContainer)) return true;
  return false;
}

/** Extract the file extension (no dot). Empty string if none. */
export function fileExtension(path: string): string {
  const base = path.includes("/") ? path.slice(path.lastIndexOf("/") + 1) : path;
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return "";
  return base.slice(dot + 1);
}

/** First path segment, or "." for files at the root. */
export function topDirectory(path: string): string {
  const idx = path.indexOf("/");
  return idx < 0 ? "." : path.slice(0, idx);
}

/** Build the three file-side pivots for the dashboard. */
export function buildFilePivots(files: BranchChangeEntry[]): FilePivots {
  const byExt = new Map<string, FilePivotRow>();
  const byDir = new Map<string, FilePivotRow>();
  const byStatus: Record<GitFileStatus, number> = {
    modified: 0,
    added: 0,
    deleted: 0,
    renamed: 0,
    untracked: 0,
  };
  for (const f of files) {
    byStatus[f.status] += 1;
    const ext = fileExtension(f.path) || "(none)";
    const dir = topDirectory(f.path);
    incPivot(byExt, ext, f.additions ?? 0, f.deletions ?? 0, f.status);
    incPivot(byDir, dir, f.additions ?? 0, f.deletions ?? 0, f.status);
  }
  const sortDesc = (a: FilePivotRow, b: FilePivotRow) =>
    b.files - a.files || b.additions + b.deletions - (a.additions + a.deletions);
  return {
    byExtension: [...byExt.values()].sort(sortDesc),
    byTopDir: [...byDir.values()].sort(sortDesc),
    byStatus,
  };
}

function incPivot(
  map: Map<string, FilePivotRow>,
  key: string,
  add: number,
  del: number,
  status: GitFileStatus,
): void {
  const bucket = statusBucket(status);
  const existing = map.get(key);
  if (existing) {
    existing.files += 1;
    existing.additions += add;
    existing.deletions += del;
    existing.byStatus[bucket] += 1;
    return;
  }
  map.set(key, {
    key,
    files: 1,
    additions: add,
    deletions: del,
    byStatus: {
      added: bucket === "added" ? 1 : 0,
      modified: bucket === "modified" ? 1 : 0,
      deleted: bucket === "deleted" ? 1 : 0,
    },
  });
}

function statusBucket(status: GitFileStatus): "added" | "modified" | "deleted" {
  if (status === "added" || status === "untracked") return "added";
  if (status === "deleted") return "deleted";
  // modified, renamed
  return "modified";
}

export interface SidedFunctionMap {
  /** path -> qualifiedKey -> summary, where qualifiedKey is
   *  `container/path/joined::functionName` so methods with the same
   *  short name in sibling classes don't collide. */
  base: Map<string, Map<string, AnalyzedFunctionSummary>>;
  head: Map<string, Map<string, AnalyzedFunctionSummary>>;
}

function qualifiedKey(fn: AnalyzedFunctionSummary): string {
  return fn.containerPath.length === 0
    ? fn.name
    : `${fn.containerPath.join("::")}::${fn.name}`;
}

export interface SideEntry {
  path: string;
  side: string;
  functions: AnalyzedFunctionSummary[];
}

/** Index analyzed sides by (side, path, functionName). */
export function indexSides(sides: SideEntry[]): SidedFunctionMap {
  const base = new Map<string, Map<string, AnalyzedFunctionSummary>>();
  const head = new Map<string, Map<string, AnalyzedFunctionSummary>>();
  for (const side of sides) {
    const target = side.side === "base" ? base : side.side === "head" ? head : null;
    if (!target) continue;
    let perFile = target.get(side.path);
    if (!perFile) {
      perFile = new Map();
      target.set(side.path, perFile);
    }
    for (const fn of side.functions) {
      const key = qualifiedKey(fn);
      // If the analyzer reports the same function name twice (overloads,
      // nested closures), keep the first to keep the diff stable.
      if (!perFile.has(key)) perFile.set(key, fn);
    }
  }
  return { base, head };
}

/**
 * Compare base vs head function maps and produce the four buckets the
 * UI renders.
 *
 * Heuristic: a function is "modified body" when its complexity OR
 * line-length changed; "modified signature" when its parameter count
 * changed (also surfaces alongside body changes if both shifted).
 */
export function diffFunctions(index: SidedFunctionMap): FunctionsBuckets {
  const out: FunctionsBuckets = {
    added: [],
    deleted: [],
    modifiedSignature: [],
    modifiedBody: [],
  };
  const allPaths = new Set<string>([...index.base.keys(), ...index.head.keys()]);
  for (const path of allPaths) {
    const baseFns = index.base.get(path) ?? new Map();
    const headFns = index.head.get(path) ?? new Map();
    const allKeys = new Set<string>([...baseFns.keys(), ...headFns.keys()]);
    for (const key of allKeys) {
      const before = baseFns.get(key);
      const after = headFns.get(key);
      if (!before && after) {
        out.added.push({
          path,
          name: after.name,
          containerPath: after.containerPath,
          startLine: after.startLine,
          paramCount: after.paramCount,
          complexity: after.complexity,
          length: after.length,
          visibility: after.visibility,
        });
        continue;
      }
      if (before && !after) {
        out.deleted.push({
          path,
          name: before.name,
          containerPath: before.containerPath,
          startLine: before.startLine,
          visibility: before.visibility,
        });
        continue;
      }
      if (!before || !after) continue;
      const containerPath = after.containerPath;
      const visibility = after.visibility;
      if (before.paramCount !== after.paramCount) {
        out.modifiedSignature.push({
          path,
          name: after.name,
          containerPath,
          startLine: after.startLine,
          before: before.paramCount,
          after: after.paramCount,
          visibility,
        });
      }
      const complexityDelta = after.complexity - before.complexity;
      const lengthDelta = after.length - before.length;
      if (complexityDelta !== 0 || lengthDelta !== 0) {
        out.modifiedBody.push({
          path,
          name: after.name,
          containerPath,
          startLine: after.startLine,
          complexityDelta,
          lengthDelta,
          visibility,
        });
      }
    }
  }
  // Stable order — most-impactful first.
  out.modifiedBody.sort(
    (a, b) => Math.abs(b.complexityDelta) - Math.abs(a.complexityDelta),
  );
  return out;
}

/**
 * Decorate `added` and `modifiedBody` rows with per-function churn.
 * Lookup key is `path::container::name` (matching the IPC churn
 * row's identity). Mutates the input buckets in place AND returns
 * them so callers can chain.
 */
export function attachChurn(
  buckets: FunctionsBuckets,
  churnRows: FunctionChurnRow[],
): FunctionsBuckets {
  let totalChurn = 0;
  for (const c of churnRows) {
    totalChurn += c.addedLines + c.deletedLines;
  }
  const lookup = new Map<string, FunctionChurnRow>();
  for (const c of churnRows) {
    lookup.set(churnLookupKey(c.path, c.containerPath, c.name), c);
  }
  const decorate = (path: string, containerPath: string[], name: string): ChurnDecoration | null => {
    const row = lookup.get(churnLookupKey(path, containerPath, name));
    if (!row) return null;
    const sum = row.addedLines + row.deletedLines;
    return {
      addedLines: row.addedLines,
      deletedLines: row.deletedLines,
      modifiedLines: row.modifiedLines,
      churnPercent: totalChurn === 0 ? 0 : sum / totalChurn,
    };
  };
  for (const row of buckets.added) {
    row.churn = decorate(row.path, row.containerPath, row.name);
  }
  for (const row of buckets.modifiedBody) {
    row.churn = decorate(row.path, row.containerPath, row.name);
  }
  return buckets;
}

function churnLookupKey(path: string, containerPath: string[], name: string): string {
  return containerPath.length === 0
    ? `${path}::${name}`
    : `${path}::${containerPath.join("::")}::${name}`;
}

export interface TestSummary {
  added: string[];
  modified: string[];
  deleted: string[];
  testFiles: number;
  nonTestFiles: number;
  /** test files / non-test files (0 if no non-test changes). */
  ratio: number;
  /** Non-test files that gained ≥1 net line and have no matching test file change. */
  riskyUntested: Array<{ path: string; netLines: number }>;
}

/** Per-status counts of test FUNCTIONS in the diff. "modified" =
 *  unique test functions that appear in either modifiedSignature or
 *  modifiedBody (so a function changed both ways still counts once).
 *  Drives SummaryCard's Tests line. */
export interface TestFunctionCounts {
  added: number;
  modified: number;
  deleted: number;
}

/** Lines-of-tests vs lines-of-production from per-function churn.
 *  `ratio = testLines / productionLines`. Used by SummaryCard's
 *  Test/code ratio line. Both numerator and denominator count
 *  added + deleted lines (i.e. total churn) within their bucket. */
export interface TestLineRatio {
  testLines: number;
  productionLines: number;
  /** 0 when productionLines is 0 (avoids div-by-zero; also natural
   *  reading: "no production code touched" rather than "infinity"). */
  ratio: number;
}

export function summarizeTestLineRatio(churn: FunctionChurnRow[]): TestLineRatio {
  let testLines = 0;
  let productionLines = 0;
  for (const c of churn) {
    const lines = c.addedLines + c.deletedLines;
    if (isTestFunction(c.path, c.name, c.containerPath)) {
      testLines += lines;
    } else {
      productionLines += lines;
    }
  }
  return {
    testLines,
    productionLines,
    ratio: productionLines === 0 ? 0 : testLines / productionLines,
  };
}

export function summarizeTestFunctions(functions: FunctionsBuckets): TestFunctionCounts {
  const out: TestFunctionCounts = { added: 0, modified: 0, deleted: 0 };
  for (const fn of functions.added) {
    if (isTestFunction(fn.path, fn.name, fn.containerPath)) out.added += 1;
  }
  for (const fn of functions.deleted) {
    if (isTestFunction(fn.path, fn.name, fn.containerPath)) out.deleted += 1;
  }
  // modifiedSignature + modifiedBody can both touch the same function.
  // Track qualified keys to avoid double-counting.
  const seen = new Set<string>();
  const tally = (fn: { path: string; name: string; containerPath: string[] }) => {
    if (!isTestFunction(fn.path, fn.name, fn.containerPath)) return;
    const key = `${fn.path}::${fn.containerPath.join("::")}::${fn.name}`;
    if (seen.has(key)) return;
    seen.add(key);
    out.modified += 1;
  };
  for (const fn of functions.modifiedSignature) tally(fn);
  for (const fn of functions.modifiedBody) tally(fn);
  return out;
}

export function summarizeTests(files: BranchChangeEntry[]): TestSummary {
  const added: string[] = [];
  const modified: string[] = [];
  const deleted: string[] = [];
  let testFiles = 0;
  let nonTestFiles = 0;
  const risky: Array<{ path: string; netLines: number }> = [];
  for (const f of files) {
    if (isTestPath(f.path)) {
      testFiles += 1;
      if (f.status === "added" || f.status === "untracked") added.push(f.path);
      else if (f.status === "deleted") deleted.push(f.path);
      else modified.push(f.path);
    } else {
      nonTestFiles += 1;
      const net = (f.additions ?? 0) - (f.deletions ?? 0);
      if (net > 0) risky.push({ path: f.path, netLines: net });
    }
  }
  // A non-test file is only "risky" when no test file in the same top-
  // level directory changed alongside it.
  const touchedDirs = new Set<string>();
  for (const f of files) {
    if (isTestPath(f.path)) touchedDirs.add(topDirectory(f.path));
  }
  const riskyFiltered = risky.filter((r) => !touchedDirs.has(topDirectory(r.path)));
  riskyFiltered.sort((a, b) => b.netLines - a.netLines);
  return {
    added,
    modified,
    deleted,
    testFiles,
    nonTestFiles,
    ratio: nonTestFiles === 0 ? 0 : testFiles / nonTestFiles,
    riskyUntested: riskyFiltered.slice(0, 20),
  };
}
