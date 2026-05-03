import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getBranchChanges,
  getCommitDetail,
  listCodeQualityFindings,
  listCodeQualityScans,
  readFileAtRef,
  readWorkspaceFile,
  runCodeQualityScan,
  subscribeCodeQualityEvents,
  subscribeGitRefsEvents,
  subscribeWorkspaceEvents,
} from "../../api.js";
import type {
  BranchChangeEntry,
  CodeQualityFindingRow,
  CodeQualityScanRow,
} from "../../api-types.js";
import { commands } from "../../tauri-bridge/index.js";
import {
  buildFilePivots,
  diffFunctions,
  indexSides,
  summarizeTests,
  type FilePivots,
  type FunctionsBuckets,
  type TestSummary,
} from "./analysisHelpers.js";

export interface UseChangeAnalysisInput {
  streamId: string | null;
  /** "working" or a commit SHA. */
  target: string;
}

export interface ChangeAnalysisState {
  loading: boolean;
  error: string | null;
  files: BranchChangeEntry[];
  totals: { additions: number; deletions: number };
  pivots: FilePivots;
  functions: FunctionsBuckets;
  duplication: {
    findings: CodeQualityFindingRow[];
    scanAgeMs: number | null;
    scanning: boolean;
    refresh: () => Promise<void>;
  };
  tests: TestSummary;
  refresh: () => Promise<void>;
}

const EMPTY_FILES: BranchChangeEntry[] = [];
const EMPTY_BUCKETS: FunctionsBuckets = {
  added: [],
  deleted: [],
  modifiedSignature: [],
  modifiedBody: [],
};
const EMPTY_PIVOTS: FilePivots = {
  byExtension: [],
  byTopDir: [],
  byStatus: { modified: 0, added: 0, deleted: 0, renamed: 0, untracked: 0 },
};

/**
 * Resolve the (baseRef, headRef) pair for the requested target. For
 * the working-tree variant `headRef` is null — the hook reads workspace
 * file content directly. For a commit SHA, base is the parent SHA and
 * head is the commit itself.
 */
async function resolveRefs(
  streamId: string,
  target: string,
): Promise<{ baseRef: string; headRef: string | null } | { error: string }> {
  if (target === "working") {
    return { baseRef: "HEAD", headRef: null };
  }
  try {
    const detail = await getCommitDetail(streamId, target);
    if (!detail) return { error: `Commit ${target.slice(0, 7)} not found.` };
    // Use the first parent (or the commit itself for the initial commit).
    const parents = (detail as { parents?: string[] | null }).parents ?? [];
    const baseRef = parents.length > 0 ? parents[0]! : `${target}^`;
    return { baseRef, headRef: target };
  } catch (err) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
}

async function fetchFiles(
  streamId: string,
  baseRef: string,
  target: string,
): Promise<BranchChangeEntry[]> {
  // For both working and commit targets, getBranchChanges over baseRef
  // yields the right diff: HEAD vs working tree, or parent vs commit.
  const branchChanges = await getBranchChanges(streamId, baseRef);
  if (target === "working") return branchChanges.files;
  // Commit-mode: getBranchChanges compares baseRef..HEAD which isn't
  // what we want — instead read commit detail's file list.
  const detail = await getCommitDetail(streamId, target);
  if (!detail) return [];
  const files = ((detail as { files?: BranchChangeEntry[] | null }).files ?? []) as BranchChangeEntry[];
  return files;
}

export function useChangeAnalysis(input: UseChangeAnalysisInput): ChangeAnalysisState {
  const { streamId, target } = input;
  const [files, setFiles] = useState<BranchChangeEntry[]>(EMPTY_FILES);
  const [functions, setFunctions] = useState<FunctionsBuckets>(EMPTY_BUCKETS);
  const [duplication, setDuplication] = useState<{
    findings: CodeQualityFindingRow[];
    scanAgeMs: number | null;
  }>({ findings: [], scanAgeMs: null });
  const [scanning, setScanning] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const reqIdRef = useRef(0);

  const refresh = useCallback(async () => {
    if (!streamId) {
      setFiles(EMPTY_FILES);
      setFunctions(EMPTY_BUCKETS);
      setDuplication({ findings: [], scanAgeMs: null });
      setLoading(false);
      return;
    }
    const reqId = ++reqIdRef.current;
    setLoading(true);
    setError(null);
    try {
      const refs = await resolveRefs(streamId, target);
      if ("error" in refs) {
        if (reqId !== reqIdRef.current) return;
        setError(refs.error);
        setFiles(EMPTY_FILES);
        setFunctions(EMPTY_BUCKETS);
        setLoading(false);
        return;
      }
      const fileList = await fetchFiles(streamId, refs.baseRef, target);
      if (reqId !== reqIdRef.current) return;
      setFiles(fileList);

      // Function diff: read base + head content for each non-binary,
      // non-deleted, non-added-only file. (Added files have no base
      // content; deleted files have no head content; both still flow
      // through analyze_functions_at_refs so add/delete buckets work.)
      const limit = 200; // hard cap to keep large changesets tractable
      const analyzable = fileList.slice(0, limit);
      const specs = await Promise.all(
        analyzable.map(async (entry) => {
          const baseContent = entry.status === "added" || entry.status === "untracked"
            ? null
            : await safeReadAtRef(streamId, refs.baseRef, entry.path);
          const headContent = await readHead(streamId, refs.headRef, entry);
          return {
            path: entry.path,
            base_content: baseContent,
            head_content: headContent,
          };
        }),
      );
      const analysis = await commands.analyzeFunctionsAtRefs(specs);
      if (reqId !== reqIdRef.current) return;
      if (analysis.status === "error") {
        setError(typeof analysis.error === "string" ? analysis.error : "Function analysis failed.");
        setFunctions(EMPTY_BUCKETS);
      } else {
        const result = analysis.data;
        const sides = result.sides.map((s) => ({
          path: s.path,
          side: s.side,
          functions: s.functions.map((fn) => ({
            name: fn.name,
            paramCount: fn.parameter_count,
            complexity: fn.complexity,
            length: fn.length,
            startLine: fn.start_line,
          })),
        }));
        setFunctions(diffFunctions(indexSides(sides)));
      }

      // Duplication: read latest jscpd scan + its findings, filter to
      // changed files. No fresh scan unless the user clicks Refresh.
      const scans = await listCodeQualityScans({ streamId, limit: 50 });
      const latestJscpd = scans.find(
        (s) => s.tool === "jscpd" && s.status === "done",
      );
      if (latestJscpd) {
        const findings = await listCodeQualityFindings({
          streamId,
          scanId: latestJscpd.id,
        });
        const changed = new Set(fileList.map((f) => f.path));
        const filtered = findings.filter((f) => changed.has(f.path));
        const scanAgeMs = scanAgeFor(latestJscpd);
        if (reqId === reqIdRef.current) {
          setDuplication({ findings: filtered, scanAgeMs });
        }
      } else if (reqId === reqIdRef.current) {
        setDuplication({ findings: [], scanAgeMs: null });
      }
    } catch (err) {
      if (reqId !== reqIdRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (reqId === reqIdRef.current) setLoading(false);
    }
  }, [streamId, target]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Live refresh when the working tree or refs change (working-tree
  // variant only — a commit SHA is immutable).
  useEffect(() => {
    if (!streamId || target !== "working") return;
    const a = subscribeGitRefsEvents(streamId, () => void refresh());
    const b = subscribeWorkspaceEvents(streamId, () => void refresh());
    return () => {
      a();
      b();
    };
  }, [streamId, target, refresh]);

  // Re-pull duplication when a code-quality scan completes.
  useEffect(() => {
    if (!streamId) return;
    return subscribeCodeQualityEvents(streamId, (event) => {
      if (event.tool !== "jscpd" || event.status !== "done") return;
      void refresh();
    });
  }, [streamId, refresh]);

  const triggerScan = useCallback(async () => {
    if (!streamId) return;
    setScanning(true);
    try {
      await runCodeQualityScan({ streamId, tool: "jscpd", scope: "diff" });
    } finally {
      setScanning(false);
    }
  }, [streamId]);

  const totals = useMemo(() => {
    let add = 0;
    let del = 0;
    for (const f of files) {
      add += f.additions ?? 0;
      del += f.deletions ?? 0;
    }
    return { additions: add, deletions: del };
  }, [files]);

  const pivots = useMemo(() => buildFilePivots(files), [files]);
  const tests = useMemo(() => summarizeTests(files), [files]);

  return {
    loading,
    error,
    files,
    totals,
    pivots,
    functions,
    duplication: {
      findings: duplication.findings,
      scanAgeMs: duplication.scanAgeMs,
      scanning,
      refresh: triggerScan,
    },
    tests,
    refresh,
  };
}

async function safeReadAtRef(
  streamId: string,
  ref: string,
  path: string,
): Promise<string | null> {
  try {
    const result = await readFileAtRef(streamId, ref, path);
    return result.content ?? null;
  } catch {
    return null;
  }
}

async function readHead(
  streamId: string,
  headRef: string | null,
  entry: BranchChangeEntry,
): Promise<string | null> {
  if (entry.status === "deleted") return null;
  if (headRef === null) {
    // Working-tree variant: read the on-disk file.
    try {
      const file = await readWorkspaceFile(streamId, entry.path);
      return file.content ?? null;
    } catch {
      return null;
    }
  }
  return safeReadAtRef(streamId, headRef, entry.path);
}

function scanAgeFor(scan: CodeQualityScanRow): number | null {
  const finishedRaw = (scan as { finished_at?: string | null }).finished_at;
  const startedRaw = scan.started_at;
  const stamp = finishedRaw ?? startedRaw;
  if (!stamp) return null;
  const ms = Date.parse(stamp);
  if (Number.isNaN(ms)) return null;
  return Date.now() - ms;
}
