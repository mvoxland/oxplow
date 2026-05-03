import type { TestSummary } from "./analysisHelpers.js";

interface TestsCardProps {
  tests: TestSummary;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

export function TestsCard({ tests, onOpenFile }: TestsCardProps) {
  const { added, modified, deleted, riskyUntested, ratio, testFiles, nonTestFiles } = tests;
  return (
    <section data-testid="change-analysis-tests" style={card}>
      <div style={header}>Tests</div>
      <div style={{ fontSize: 12, color: "var(--text-muted)", marginBottom: 8 }}>
        {testFiles} test file{testFiles === 1 ? "" : "s"} touched • {nonTestFiles} non-test file
        {nonTestFiles === 1 ? "" : "s"} touched • ratio {ratio.toFixed(2)}
      </div>
      <Bucket title="Added" paths={added} onOpenFile={onOpenFile} />
      <Bucket title="Modified" paths={modified} onOpenFile={onOpenFile} />
      <Bucket title="Deleted" paths={deleted} onOpenFile={onOpenFile} />
      {riskyUntested.length > 0 ? (
        <div style={{ marginTop: 8 }}>
          <div style={{ fontSize: 12, color: "var(--text-warning, #92400e)", marginBottom: 4 }}>
            Non-test files added lines without matching test changes (heuristic):
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {riskyUntested.map((r) => (
              <button
                key={r.path}
                type="button"
                data-testid="change-analysis-risky-row"
                onClick={(e) => onOpenFile(r.path, { newTab: e.metaKey || e.ctrlKey })}
                style={pathButton}
              >
                {r.path} <span style={{ color: "var(--text-muted)" }}>(+{r.netLines} net)</span>
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </section>
  );
}

function Bucket({
  title,
  paths,
  onOpenFile,
}: {
  title: string;
  paths: string[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}) {
  if (paths.length === 0) return null;
  return (
    <div style={{ marginBottom: 6 }}>
      <div style={{ fontSize: 12, color: "var(--text-muted)" }}>
        {title} ({paths.length})
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        {paths.slice(0, 25).map((p) => (
          <button
            key={p}
            type="button"
            onClick={(e) => onOpenFile(p, { newTab: e.metaKey || e.ctrlKey })}
            style={pathButton}
          >
            {p}
          </button>
        ))}
        {paths.length > 25 ? (
          <div style={{ color: "var(--text-muted)", fontSize: 12 }}>
            …and {paths.length - 25} more
          </div>
        ) : null}
      </div>
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
const pathButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  textAlign: "left",
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: 12,
};
