import type { FunctionsBuckets } from "./analysisHelpers.js";

interface FunctionsCardProps {
  functions: FunctionsBuckets;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

interface RowEntry {
  path: string;
  containerPath: string[];
  text: string;
}

export function FunctionsCard({ functions, onOpenFile }: FunctionsCardProps) {
  const empty =
    functions.added.length === 0 &&
    functions.deleted.length === 0 &&
    functions.modifiedSignature.length === 0 &&
    functions.modifiedBody.length === 0;

  const added: RowEntry[] = functions.added.map((fn) => ({
    path: fn.path,
    containerPath: fn.containerPath,
    text: `${fn.name} (cc=${fn.complexity}, p=${fn.paramCount})`,
  }));
  const deleted: RowEntry[] = functions.deleted.map((fn) => ({
    path: fn.path,
    containerPath: fn.containerPath,
    text: fn.name,
  }));
  const modSig: RowEntry[] = functions.modifiedSignature.map((fn) => ({
    path: fn.path,
    containerPath: fn.containerPath,
    text: `${fn.name} : ${fn.before} → ${fn.after} params`,
  }));
  const modBody: RowEntry[] = functions.modifiedBody.map((fn) => ({
    path: fn.path,
    containerPath: fn.containerPath,
    text: `${fn.name} (Δcc ${fn.complexityDelta >= 0 ? "+" : ""}${fn.complexityDelta}, Δlen ${
      fn.lengthDelta >= 0 ? "+" : ""
    }${fn.lengthDelta})`,
  }));

  return (
    <section data-testid="change-analysis-functions" style={card}>
      <div style={header}>Functions</div>
      {empty ? (
        <div style={muted}>
          No function-level changes detected (the changed files may be in unsupported languages).
        </div>
      ) : (
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
          <Bucket title={`Added (${added.length})`} testId="functions-added" rows={added} onOpenFile={onOpenFile} />
          <Bucket title={`Deleted (${deleted.length})`} testId="functions-deleted" rows={deleted} onOpenFile={onOpenFile} />
          <Bucket
            title={`Signature changed (${modSig.length})`}
            testId="functions-sig"
            rows={modSig}
            onOpenFile={onOpenFile}
          />
          <Bucket
            title={`Body changed (${modBody.length})`}
            testId="functions-body"
            rows={modBody}
            onOpenFile={onOpenFile}
          />
        </div>
      )}
    </section>
  );
}

interface TreeNode {
  /** Display label for this node. For files this is the full path; for
   *  containers it's the container's own segment. */
  label: string;
  /** Repo-relative file path — the click target when this node names a
   *  file or container row. */
  path: string;
  /** Direct children: files under "no container", or sub-containers. */
  children: TreeNode[];
  /** Leaf rows directly attached at this level. */
  rows: RowEntry[];
}

/** Build a `path > containerPath... > row` tree from a flat row list. */
function buildTree(rows: RowEntry[]): TreeNode[] {
  // path -> file node
  const files = new Map<string, TreeNode>();
  for (const row of rows) {
    let fileNode = files.get(row.path);
    if (!fileNode) {
      fileNode = { label: row.path, path: row.path, children: [], rows: [] };
      files.set(row.path, fileNode);
    }
    let cursor = fileNode;
    for (const segment of row.containerPath) {
      let next = cursor.children.find((c) => c.label === segment);
      if (!next) {
        next = { label: segment, path: row.path, children: [], rows: [] };
        cursor.children.push(next);
      }
      cursor = next;
    }
    cursor.rows.push(row);
  }
  return [...files.values()].sort((a, b) => a.label.localeCompare(b.label));
}

function totalLeafCount(node: TreeNode): number {
  return node.rows.length + node.children.reduce((sum, c) => sum + totalLeafCount(c), 0);
}

function Bucket({
  title,
  testId,
  rows,
  onOpenFile,
}: {
  title: string;
  testId: string;
  rows: RowEntry[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}) {
  const tree = buildTree(rows);
  return (
    <div data-testid={`change-analysis-${testId}`}>
      <div style={{ fontSize: 12, color: "var(--text-muted)", marginBottom: 4 }}>{title}</div>
      {tree.length === 0 ? (
        <div style={muted}>—</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {tree.map((node) => (
            <TreeBranch key={node.path} node={node} depth={0} onOpenFile={onOpenFile} />
          ))}
        </div>
      )}
    </div>
  );
}

function TreeBranch({
  node,
  depth,
  onOpenFile,
}: {
  node: TreeNode;
  depth: number;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}) {
  const hasNested = node.children.length > 0;
  const hasContent = hasNested || node.rows.length > 0;
  if (!hasContent) return null;
  const count = totalLeafCount(node);
  // File-level nodes default open; container nodes also default open
  // so the user sees the hierarchy at a glance. <details> keeps state
  // locally without a re-render store.
  return (
    <details open style={{ paddingLeft: depth === 0 ? 0 : 12 }}>
      <summary style={summaryStyle(depth)}>
        <span style={depth === 0 ? filePathStyle : containerStyle}>{node.label}</span>
        <span style={{ color: "var(--text-muted)", marginLeft: 6 }}>({count})</span>
      </summary>
      {node.children.map((child) => (
        <TreeBranch key={`${child.label}@${child.path}`} node={child} depth={depth + 1} onOpenFile={onOpenFile} />
      ))}
      {node.rows.map((row, i) => (
        <button
          key={`${row.text}:${i}`}
          type="button"
          onClick={(e) => onOpenFile(row.path, { newTab: e.metaKey || e.ctrlKey })}
          style={{ ...fnRow, paddingLeft: 12 + (depth + 1) * 12 }}
        >
          {row.text}
        </button>
      ))}
    </details>
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
const filePathStyle: React.CSSProperties = { color: "var(--text-muted)", fontSize: 12 };
const containerStyle: React.CSSProperties = { color: "var(--text-primary)", fontSize: 12, fontWeight: 500 };
const summaryStyle = (depth: number): React.CSSProperties => ({
  cursor: "pointer",
  fontSize: 12,
  padding: "1px 0",
  paddingLeft: depth * 12,
  userSelect: "none",
});
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
  display: "block",
  width: "100%",
};
