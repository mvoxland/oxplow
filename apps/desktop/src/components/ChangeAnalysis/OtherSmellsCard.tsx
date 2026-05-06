import type { FunctionsBuckets } from "./analysisHelpers.js";

interface OtherSmellsCardProps {
  functions: FunctionsBuckets;
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
  /** Plain click on a row → diff at the function's start line.
   *  Cmd/ctrl-click → new-tab file open. */
  onOpenFileDiff?: (path: string, line?: number) => void;
}

const SECTION_CAP = 5;
const LONG_FN_THRESHOLD = 60;
const PARAM_SPIKE = 2;

/**
 * Multi-section panel for code-smell signals that aren't churn or
 * complexity per se but still tell a reviewer where to look:
 *
 *   - Parameter-count spikes (signature change adding ≥2 params).
 *   - Very long new functions (length > 60).
 *
 * Each section caps at 5; the whole card hides when both sections
 * are empty. Test-coverage signals were removed because the only
 * honest detection at the file level (without per-file pair
 * matching or coverage data) is global, which doesn't differentiate
 * one row from another.
 */
export function OtherSmellsCard({ functions, onOpenFile, onOpenFileDiff }: OtherSmellsCardProps) {
  const paramSpikes = functions.modifiedSignature
    .filter((fn) => fn.after - fn.before >= PARAM_SPIKE)
    .sort((a, b) => b.after - b.before - (a.after - a.before))
    .slice(0, SECTION_CAP);

  const longNewFns = functions.added
    .filter((fn) => fn.length > LONG_FN_THRESHOLD)
    .sort((a, b) => b.length - a.length)
    .slice(0, SECTION_CAP);

  if (paramSpikes.length === 0 && longNewFns.length === 0) {
    return null;
  }

  return (
    <section data-testid="change-analysis-other-smells" style={card}>
      <div style={header}>Other smells</div>

      {paramSpikes.length > 0 ? (
        <Section title="Parameter list growth">
          {paramSpikes.map((fn) => (
            <Row
              key={`p::${fn.path}::${fn.containerPath.join("::")}::${fn.name}`}
              path={fn.path}
              startLine={fn.startLine}
              label={`${fn.containerPath.length > 0 ? `${fn.containerPath.join("::")}::` : ""}${fn.name}`}
              badge={`+${fn.after - fn.before} params (now ${fn.after})`}
              badgeColor="var(--text-danger, #dc2626)"
              onOpen={onOpenFile}
              onOpenDiff={onOpenFileDiff}
            />
          ))}
        </Section>
      ) : null}

      {longNewFns.length > 0 ? (
        <Section title="Very long new functions">
          {longNewFns.map((fn) => (
            <Row
              key={`l::${fn.path}::${fn.containerPath.join("::")}::${fn.name}`}
              path={fn.path}
              startLine={fn.startLine}
              label={`${fn.containerPath.length > 0 ? `${fn.containerPath.join("::")}::` : ""}${fn.name}`}
              badge={`${fn.length} lines`}
              badgeColor="var(--text-danger, #dc2626)"
              onOpen={onOpenFile}
              onOpenDiff={onOpenFileDiff}
            />
          ))}
        </Section>
      ) : null}
    </section>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ marginTop: 8 }} data-testid="change-analysis-smell-section">
      <div style={{ fontSize: 11, color: "var(--text-muted)", textTransform: "uppercase", letterSpacing: 0.5, marginBottom: 4 }}>
        {title}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>{children}</div>
    </div>
  );
}

function Row({
  path,
  startLine,
  label,
  badge,
  badgeColor,
  onOpen,
  onOpenDiff,
}: {
  path: string;
  startLine: number;
  label: string;
  badge: string;
  badgeColor: string;
  onOpen?: (path: string, opts?: { newTab?: boolean }) => void;
  onOpenDiff?: (path: string, line?: number) => void;
}) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
      <span style={{ color: badgeColor, minWidth: 100, fontFamily: "ui-monospace, monospace", fontSize: 11 }}>
        {badge}
      </span>
      <span style={{ color: "var(--text-primary)", fontWeight: 600 }}>{label}</span>
      <button
        type="button"
        onClick={(e) => {
          if (e.metaKey || e.ctrlKey) {
            onOpen?.(path, { newTab: true });
            return;
          }
          if (onOpenDiff) onOpenDiff(path, startLine);
          else onOpen?.(path);
        }}
        style={pathButton}
        title={path}
      >
        {path}:{startLine}
      </button>
    </div>
  );
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600 };
const pathButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: 12,
  marginLeft: "auto",
};
