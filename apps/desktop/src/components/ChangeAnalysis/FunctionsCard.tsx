import type { FunctionsBuckets } from "./analysisHelpers.js";

interface FunctionsCardProps {
  functions: FunctionsBuckets;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

export function FunctionsCard({ functions, onOpenFile }: FunctionsCardProps) {
  const empty =
    functions.added.length === 0 &&
    functions.deleted.length === 0 &&
    functions.modifiedSignature.length === 0 &&
    functions.modifiedBody.length === 0;
  return (
    <section data-testid="change-analysis-functions" style={card}>
      <div style={header}>Functions</div>
      {empty ? (
        <div style={muted}>
          No function-level changes detected (lizard may not be installed, or the changes are
          outside lizard-supported languages).
        </div>
      ) : (
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
          <Bucket
            title={`Added (${functions.added.length})`}
            testId="functions-added"
            rows={functions.added.map((fn) => ({
              path: fn.path,
              text: `${fn.name} (cc=${fn.complexity}, p=${fn.paramCount})`,
            }))}
            onOpenFile={onOpenFile}
          />
          <Bucket
            title={`Deleted (${functions.deleted.length})`}
            testId="functions-deleted"
            rows={functions.deleted.map((fn) => ({ path: fn.path, text: fn.name }))}
            onOpenFile={onOpenFile}
          />
          <Bucket
            title={`Signature changed (${functions.modifiedSignature.length})`}
            testId="functions-sig"
            rows={functions.modifiedSignature.map((fn) => ({
              path: fn.path,
              text: `${fn.name} : ${fn.before} → ${fn.after} params`,
            }))}
            onOpenFile={onOpenFile}
          />
          <Bucket
            title={`Body changed (${functions.modifiedBody.length})`}
            testId="functions-body"
            rows={functions.modifiedBody.map((fn) => ({
              path: fn.path,
              text: `${fn.name} (Δcc ${fn.complexityDelta >= 0 ? "+" : ""}${fn.complexityDelta}, Δlen ${
                fn.lengthDelta >= 0 ? "+" : ""
              }${fn.lengthDelta})`,
            }))}
            onOpenFile={onOpenFile}
          />
        </div>
      )}
    </section>
  );
}

function Bucket({
  title,
  testId,
  rows,
  onOpenFile,
}: {
  title: string;
  testId: string;
  rows: Array<{ path: string; text: string }>;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}) {
  return (
    <div data-testid={`change-analysis-${testId}`}>
      <div style={{ fontSize: 12, color: "var(--text-muted)", marginBottom: 4 }}>{title}</div>
      {rows.length === 0 ? (
        <div style={muted}>—</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {rows.slice(0, 50).map((row, i) => (
            <button
              key={`${row.path}:${row.text}:${i}`}
              type="button"
              onClick={(e) => onOpenFile(row.path, { newTab: e.metaKey || e.ctrlKey })}
              style={fnRow}
            >
              <span style={{ color: "var(--text-muted)", marginRight: 6 }}>{row.path}</span>
              <span>{row.text}</span>
            </button>
          ))}
          {rows.length > 50 ? (
            <div style={muted}>…and {rows.length - 50} more</div>
          ) : null}
        </div>
      )}
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
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 12 };
const fnRow: React.CSSProperties = {
  textAlign: "left",
  background: "transparent",
  border: "none",
  cursor: "pointer",
  padding: "2px 4px",
  fontSize: 12,
  color: "var(--text-primary)",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
