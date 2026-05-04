import type { BranchChangeEntry, GitFileStatus } from "../../api-types.js";

export interface AnalyzedFunctionSummary {
  name: string;
  paramCount: number;
  complexity: number;
  length: number;
  startLine: number;
  /** Outer-to-inner container ancestors (class/impl/module/namespace). */
  containerPath: string[];
}

export interface FilePivotRow {
  key: string;
  files: number;
  additions: number;
  deletions: number;
}

export interface FilePivots {
  byExtension: FilePivotRow[];
  byTopDir: FilePivotRow[];
  byStatus: Record<GitFileStatus, number>;
}

export interface FunctionsBuckets {
  added: Array<{ path: string; name: string; containerPath: string[]; startLine: number; paramCount: number; complexity: number }>;
  deleted: Array<{ path: string; name: string; containerPath: string[]; startLine: number }>;
  modifiedSignature: Array<{ path: string; name: string; containerPath: string[]; startLine: number; before: number; after: number }>;
  modifiedBody: Array<{ path: string; name: string; containerPath: string[]; startLine: number; complexityDelta: number; lengthDelta: number }>;
}

const TEST_PATTERNS: RegExp[] = [
  /\.test\.[a-zA-Z]+$/,
  /\.spec\.[a-zA-Z]+$/,
  /(^|\/)tests?\//,
  /(^|\/)test_[^/]+\.(py|rs)$/,
  /_test\.go$/,
];

/** True if `path` looks like a test file by convention. */
export function isTestPath(path: string): boolean {
  return TEST_PATTERNS.some((re) => re.test(path));
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
    incPivot(byExt, ext, f.additions ?? 0, f.deletions ?? 0);
    incPivot(byDir, dir, f.additions ?? 0, f.deletions ?? 0);
  }
  const sortDesc = (a: FilePivotRow, b: FilePivotRow) =>
    b.files - a.files || b.additions + b.deletions - (a.additions + a.deletions);
  return {
    byExtension: [...byExt.values()].sort(sortDesc),
    byTopDir: [...byDir.values()].sort(sortDesc),
    byStatus,
  };
}

function incPivot(map: Map<string, FilePivotRow>, key: string, add: number, del: number): void {
  const existing = map.get(key);
  if (existing) {
    existing.files += 1;
    existing.additions += add;
    existing.deletions += del;
    return;
  }
  map.set(key, { key, files: 1, additions: add, deletions: del });
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
        });
        continue;
      }
      if (before && !after) {
        out.deleted.push({
          path,
          name: before.name,
          containerPath: before.containerPath,
          startLine: before.startLine,
        });
        continue;
      }
      if (!before || !after) continue;
      const containerPath = after.containerPath;
      if (before.paramCount !== after.paramCount) {
        out.modifiedSignature.push({
          path,
          name: after.name,
          containerPath,
          startLine: after.startLine,
          before: before.paramCount,
          after: after.paramCount,
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
