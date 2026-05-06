import type { FunctionsBuckets } from "./analysisHelpers.js";

interface CodeSmellsCardProps {
  functions: FunctionsBuckets;
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
  /** Plain click on a row → diff at the function's start line.
   *  Cmd/ctrl-click → new-tab file open. */
  onOpenFileDiff?: (path: string, line?: number) => void;
}

const ROW_CAP = 12;
const PARAM_SECTION_CAP = 5;
const LONG_SECTION_CAP = 5;
const COMPLEXITY_DELTA_THRESHOLD = 2;
const ADDED_COMPLEXITY_THRESHOLD = 8;
const LONG_FN_THRESHOLD = 60;
const PARAM_SPIKE_THRESHOLD = 2;

interface SmellRow {
  key: string;
  path: string;
  startLine: number;
  fnLabel: string;
  /** Smell-specific detail rendered as the right-most column. */
  detail: string;
  detailColor: string;
}

/**
 * Combined panel for code-smell signals across the four function
 * buckets. One card with three sections (Complexity spikes /
 * Parameter list growth / Very long new functions); each row
 * follows the same shape: file path → function name → smell
 * detail. Sections are independent — caps and thresholds live
 * on each section separately. The whole card hides when every
 * section is empty.
 */
export function CodeSmellsCard({ functions, onOpenFile, onOpenFileDiff }: CodeSmellsCardProps) {
  const complexitySpikes: SmellRow[] = [
    ...functions.modifiedBody
      .filter((fn) => fn.complexityDelta >= COMPLEXITY_DELTA_THRESHOLD)
      .map<SmellRow>((fn) => ({
        key: `cs-delta::${fn.path}::${fn.containerPath.join("::")}::${fn.name}`,
        path: fn.path,
        startLine: fn.startLine,
        fnLabel: qualified(fn.containerPath, fn.name),
        detail: `+${fn.complexityDelta} cc`,
        detailColor: "var(--text-danger, #dc2626)",
      })),
    ...functions.added
      .filter((fn) => fn.complexity >= ADDED_COMPLEXITY_THRESHOLD)
      .map<SmellRow>((fn) => ({
        key: `cs-new::${fn.path}::${fn.containerPath.join("::")}::${fn.name}`,
        path: fn.path,
        startLine: fn.startLine,
        fnLabel: qualified(fn.containerPath, fn.name),
        detail: `new · cc ${fn.complexity}`,
        detailColor: "var(--text-muted)",
      })),
  ]
    .sort((a, b) => parseWeight(b.detail) - parseWeight(a.detail))
    .slice(0, ROW_CAP);

  const paramSpikes: SmellRow[] = functions.modifiedSignature
    .filter((fn) => fn.after - fn.before >= PARAM_SPIKE_THRESHOLD)
    .sort((a, b) => b.after - b.before - (a.after - a.before))
    .slice(0, PARAM_SECTION_CAP)
    .map<SmellRow>((fn) => ({
      key: `ps::${fn.path}::${fn.containerPath.join("::")}::${fn.name}`,
      path: fn.path,
      startLine: fn.startLine,
      fnLabel: qualified(fn.containerPath, fn.name),
      detail: `+${fn.after - fn.before} params (now ${fn.after})`,
      detailColor: "var(--text-danger, #dc2626)",
    }));

  const longNewFns: SmellRow[] = functions.added
    .filter((fn) => fn.length > LONG_FN_THRESHOLD)
    .sort((a, b) => b.length - a.length)
    .slice(0, LONG_SECTION_CAP)
    .map<SmellRow>((fn) => ({
      key: `lf::${fn.path}::${fn.containerPath.join("::")}::${fn.name}`,
      path: fn.path,
      startLine: fn.startLine,
      fnLabel: qualified(fn.containerPath, fn.name),
      detail: `${fn.length} lines`,
      detailColor: "var(--text-danger, #dc2626)",
    }));

  if (
    complexitySpikes.length === 0 &&
    paramSpikes.length === 0 &&
    longNewFns.length === 0
  ) {
    return null;
  }

  return (
    <section data-testid="change-analysis-code-smells" style={card}>
      <div style={header}>Code smells</div>
      {complexitySpikes.length > 0 ? (
        <Section title="Complexity spikes" rows={complexitySpikes} onOpenFile={onOpenFile} onOpenFileDiff={onOpenFileDiff} />
      ) : null}
      {paramSpikes.length > 0 ? (
        <Section title="Parameter list growth" rows={paramSpikes} onOpenFile={onOpenFile} onOpenFileDiff={onOpenFileDiff} />
      ) : null}
      {longNewFns.length > 0 ? (
        <Section title="Very long new functions" rows={longNewFns} onOpenFile={onOpenFile} onOpenFileDiff={onOpenFileDiff} />
      ) : null}
    </section>
  );
}

function qualified(containerPath: string[], name: string): string {
  return containerPath.length > 0 ? `${containerPath.join("::")}::${name}` : name;
}

/** Pull the leading numeric out of a detail string for sort weight.
 *  "+5 cc" → 5, "new · cc 12" → 12, falls back to 0. */
function parseWeight(detail: string): number {
  const m = detail.match(/-?\d+/);
  return m ? parseInt(m[0]!, 10) : 0;
}

function Section({
  title,
  rows,
  onOpenFile,
  onOpenFileDiff,
}: {
  title: string;
  rows: SmellRow[];
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
  onOpenFileDiff?: (path: string, line?: number) => void;
}) {
  return (
    <div style={section} data-testid="change-analysis-smell-section">
      <div style={sectionTitle}>{title}</div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {rows.map((row) => (
          <Row
            key={row.key}
            row={row}
            onOpenFile={onOpenFile}
            onOpenFileDiff={onOpenFileDiff}
          />
        ))}
      </div>
    </div>
  );
}

function Row({
  row,
  onOpenFile,
  onOpenFileDiff,
}: {
  row: SmellRow;
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
  onOpenFileDiff?: (path: string, line?: number) => void;
}) {
  return (
    <div style={rowOuter}>
      <button
        type="button"
        onClick={(e) => {
          if (e.metaKey || e.ctrlKey) {
            onOpenFile?.(row.path, { newTab: true });
            return;
          }
          if (onOpenFileDiff) onOpenFileDiff(row.path, row.startLine);
          else onOpenFile?.(row.path);
        }}
        style={pathButton}
        title={`${row.path}:${row.startLine}`}
      >
        {row.path}:{row.startLine}
      </button>
      <span style={fnCol} title={row.fnLabel}>
        {row.fnLabel}
      </span>
      <span style={{ ...detailCol, color: row.detailColor }}>{row.detail}</span>
    </div>
  );
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600, marginBottom: 4 };
const section: React.CSSProperties = {
  marginTop: 8,
  paddingTop: 8,
  borderTop: "1px solid var(--border-subtle)",
};
const sectionTitle: React.CSSProperties = {
  fontSize: 11,
  color: "var(--text-muted)",
  textTransform: "uppercase",
  letterSpacing: 0.5,
  marginBottom: 6,
};
const rowOuter: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  fontSize: 12,
};
const pathButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: 12,
  textAlign: "left",
  flexShrink: 0,
  width: 360,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
const fnCol: React.CSSProperties = {
  color: "var(--text-primary)",
  fontWeight: 500,
  flex: 1,
  minWidth: 0,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
const detailCol: React.CSSProperties = {
  fontFamily: "ui-monospace, monospace",
  fontSize: 11,
  flexShrink: 0,
  textAlign: "right",
  minWidth: 140,
};
