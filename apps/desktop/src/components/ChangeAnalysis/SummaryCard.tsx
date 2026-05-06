import type { GitFileStatus } from "../../api-types.js";
import type {
  TestFunctionCounts,
  TestLineRatio,
  TestSummary,
} from "./analysisHelpers.js";

interface SummaryCardProps {
  fileCount: number;
  additions: number;
  deletions: number;
  byStatus: Record<GitFileStatus, number>;
  /** File-level test summary. Currently passed for the ratio's
   *  fallback when no churn data is available; OtherSmellsCard
   *  reads the file lists. */
  tests: TestSummary;
  /** Per-status test FUNCTION counts. Drives the Tests line. */
  testFunctions: TestFunctionCounts;
  /** Lines-of-tests vs lines-of-production from per-function
   *  churn. Drives the Test/code ratio. */
  testLineRatio: TestLineRatio;
}

export function SummaryCard({
  fileCount,
  additions,
  deletions,
  byStatus,
  tests,
  testFunctions,
  testLineRatio,
}: SummaryCardProps) {
  void tests; // currently unused on this card; kept for callsite stability
  return (
    <section data-testid="change-analysis-summary" style={card}>
      <div style={header}>Summary</div>
      <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
        <Stat label="Files" value={fileCount} />
        <Stat label="+ lines" value={additions} color="var(--text-success, #16a34a)" />
        <Stat label="− lines" value={deletions} color="var(--text-danger, #dc2626)" />
        <Stat label="M" value={byStatus.modified} />
        <Stat label="A" value={byStatus.added + byStatus.untracked} />
        <Stat label="D" value={byStatus.deleted} />
        <Stat label="R" value={byStatus.renamed} />
      </div>
      <div
        style={{ marginTop: 12, fontSize: 12, color: "var(--text-muted)" }}
        title={`${testLineRatio.testLines} test lines / ${testLineRatio.productionLines} production lines (added + deleted, function-anchored)`}
      >
        Tests: {testFunctions.added} added, {testFunctions.modified} modified,{" "}
        {testFunctions.deleted} deleted • Test/code ratio:{" "}
        {testLineRatio.ratio.toFixed(2)}
        {testLineRatio.productionLines > 0 || testLineRatio.testLines > 0 ? (
          <span style={{ marginLeft: 6 }}>
            ({testLineRatio.testLines} / {testLineRatio.productionLines} lines)
          </span>
        ) : null}
      </div>
    </section>
  );
}

function Stat({ label, value, color }: { label: string; value: number; color?: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", minWidth: 64 }}>
      <span style={{ fontSize: 11, color: "var(--text-muted)", textTransform: "uppercase", letterSpacing: 0.5 }}>
        {label}
      </span>
      <span style={{ fontSize: 18, fontWeight: 600, color: color ?? "var(--text-primary)" }}>{value}</span>
    </div>
  );
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600, marginBottom: 8 };
