import { useMemo, useState } from "react";
import type { ChangeAnalysisScope, ChangeAnalysisTarget } from "../../tabs/pageRefs.js";
import type { ChangeAnalysisState } from "./useChangeAnalysis.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { DISK, refVersion, type FileVersion } from "../../file-version.js";
import { DuplicationCard } from "./DuplicationCard.js";
import { TestsCard } from "./TestsCard.js";
import { LookHereFirstCard } from "./LookHereFirstCard.js";
import { ChurnCard } from "./ChurnCard.js";
import { ComplexitySpikesCard } from "./ComplexitySpikesCard.js";
import { OtherSmellsCard } from "./OtherSmellsCard.js";
import {
  ALL_STATUSES,
  FilesPanel,
  statusPasses,
  type StatusKey,
} from "./FilesPanel.js";
import {
  buildFilePivots,
  summarizeTests,
  type FunctionChurnRow,
  type FunctionsBuckets,
} from "./analysisHelpers.js";
import { usePageSnapshot } from "../../tabs/usePageSnapshot.js";

export interface ChangeAnalysisDrilldownProps {
  /** Optional drilldown filter; when absent, every panel still
   *  renders against the full file set. */
  scope?: ChangeAnalysisScope;
  target: ChangeAnalysisTarget;
  analysis: ChangeAnalysisState;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec): void;
}

export function ChangeAnalysisDrilldown({
  scope,
  target,
  analysis,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: ChangeAnalysisDrilldownProps) {
  // Initial status filter:
  //  - if the page was opened with a status scope, restrict to it.
  //  - otherwise default to "all three checked".
  const initialStatus: Set<StatusKey> =
    scope?.kind === "status" && (scope.value === "added" || scope.value === "modified" || scope.value === "deleted")
      ? new Set([scope.value as StatusKey])
      : new Set(ALL_STATUSES);
  const [statusFilter, setStatusFilter] = useState<Set<StatusKey>>(initialStatus);

  // Persist the filter selection across reloads.
  usePageSnapshot<{ statusFilter: StatusKey[] }>({
    serialize: () => ({ statusFilter: [...statusFilter] }),
    restore: (snap) => {
      if (Array.isArray(snap.statusFilter)) {
        const valid = snap.statusFilter.filter((k): k is StatusKey =>
          k === "added" || k === "modified" || k === "deleted",
        );
        setStatusFilter(new Set(valid.length > 0 ? valid : ALL_STATUSES));
      }
    },
    deps: [statusFilter],
  });

  const filesAfterStatus = useMemo(
    () => analysis.files.filter((f) => statusPasses(f.status, statusFilter)),
    [analysis.files, statusFilter],
  );

  const pivotsAfterStatus = useMemo(() => buildFilePivots(filesAfterStatus), [filesAfterStatus]);
  const testsAfterStatus = useMemo(() => summarizeTests(filesAfterStatus), [filesAfterStatus]);

  const filteredPathSet = useMemo(
    () => new Set(filesAfterStatus.map((f) => f.path)),
    [filesAfterStatus],
  );
  const functionsAfterStatus = useMemo<FunctionsBuckets>(() => ({
    added: analysis.functions.added.filter((fn) => filteredPathSet.has(fn.path)),
    deleted: analysis.functions.deleted.filter((fn) => filteredPathSet.has(fn.path)),
    modifiedSignature: analysis.functions.modifiedSignature.filter((fn) => filteredPathSet.has(fn.path)),
    modifiedBody: analysis.functions.modifiedBody.filter((fn) => filteredPathSet.has(fn.path)),
  }), [analysis.functions, filteredPathSet]);

  const dupAfterStatus = useMemo(() => ({
    ...analysis.duplication,
    findings: analysis.duplication.findings.filter((f) => filteredPathSet.has(f.path)),
  }), [analysis.duplication, filteredPathSet]);

  const churnAfterStatus = useMemo<FunctionChurnRow[]>(
    () => analysis.functionChurn.filter((c) => filteredPathSet.has(c.path)),
    [analysis.functionChurn, filteredPathSet],
  );

  /**
   * Open the diff for `path` at `line` *in the current tab*. Falls
   * through to plain file-open if no diff handler is wired or refs
   * haven't resolved yet.
   */
  const openDiffAt = (path: string, line: number) => {
    if (!analysis.refs) {
      onOpenFile(path);
      return;
    }
    const { baseRef, headRef } = analysis.refs;
    const rightVersion: FileVersion = headRef ? refVersion(headRef) : DISK;
    const spec: DiffSpec = {
      path,
      leftVersion: refVersion(baseRef),
      rightVersion,
      baseLabel: target === "working" ? "working tree" : `parent of ${target.toString().slice(0, 7)}`,
      revealLine: line,
    };
    if (onOpenDiffInTab) onOpenDiffInTab(spec);
    else if (onOpenDiff) onOpenDiff(spec);
    else onOpenFile(path);
  };

  // The duplication card needs to know which tree version the scan
  // ran against so it can stamp every duplicate-block ref with that
  // version. For working-tree analysis it's `disk`; for a commit it
  // matches the analyzed commit's tree.
  const scanVersion: FileVersion = target === "working" ? DISK : refVersion(target);

  return (
    <>
      <FilesPanel
        files={filesAfterStatus}
        functions={functionsAfterStatus}
        functionChurn={churnAfterStatus}
        pivots={pivotsAfterStatus}
        statusFilter={statusFilter}
        onStatusChange={setStatusFilter}
        target={target}
        onOpenFile={onOpenFile}
        onOpenFunctionDiff={openDiffAt}
        onOpenFileDiff={(path) => openDiffAt(path, 1)}
      />
      <LookHereFirstCard
        files={filesAfterStatus}
        fileScores={analysis.fileScores}
        onOpenFile={onOpenFile}
      />
      <ChurnCard
        files={filesAfterStatus}
        functionChurn={churnAfterStatus}
        onOpenFile={onOpenFile}
      />
      <ComplexitySpikesCard functions={functionsAfterStatus} onOpenFile={onOpenFile} />
      <OtherSmellsCard
        functions={functionsAfterStatus}
        tests={testsAfterStatus}
        onOpenFile={onOpenFile}
      />
      <DuplicationCard duplication={dupAfterStatus} scanVersion={scanVersion} onOpenFile={onOpenFile} />
      <TestsCard tests={testsAfterStatus} onOpenFile={onOpenFile} />
    </>
  );
}
