import type { FunctionsBuckets } from "./analysisHelpers.js";

interface FunctionsCardProps {
  functions: FunctionsBuckets;
  /** Click target for function leaf rows. Falls back to opening the
   *  file in the editor if the host doesn't supply
   *  `onOpenFunctionDiff`. */
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  /** Preferred click target — opens the file's diff tab and reveals
   *  the function's start line. */
  onOpenFunctionDiff?(path: string, line: number): void;
}

type RowStatus = "added" | "modified" | "deleted";

interface RowEntry {
  path: string;
  containerPath: string[];
  startLine: number;
  /** Function name (no metric annotations). */
  name: string;
  /** Free-form metric tail rendered after the name in muted text. */
  detail: string;
  status: RowStatus;
}

export function FunctionsCard({ functions, onOpenFile, onOpenFunctionDiff }: FunctionsCardProps) {
  const rows = flattenRows(functions);

  return (
    <section data-testid="change-analysis-functions" style={card}>
      <div style={header}>Functions</div>
      {rows.length === 0 ? (
        <div style={muted}>
          No function-level changes detected (the changed files may be in unsupported languages).
        </div>
      ) : (
        <Tree rows={rows} onOpenFile={onOpenFile} onOpenFunctionDiff={onOpenFunctionDiff} />
      )}
    </section>
  );
}

/**
 * Collapse the four FunctionsBuckets into a single status-tagged row
 * list. A function that appears in both `modifiedSignature` and
 * `modifiedBody` shows up once with a combined detail string.
 */
function flattenRows(functions: FunctionsBuckets): RowEntry[] {
  // Key on (path, containerPath joined, name) so the same function in
  // sibling classes stays separate but a sig+body match merges.
  const keyOf = (path: string, container: string[], name: string) =>
    `${path}::${container.join("::")}::${name}`;
  const out = new Map<string, RowEntry>();
  for (const fn of functions.added) {
    out.set(keyOf(fn.path, fn.containerPath, fn.name), {
      path: fn.path,
      containerPath: fn.containerPath,
      startLine: fn.startLine,
      name: fn.name,
      detail: `cc=${fn.complexity}, p=${fn.paramCount}`,
      status: "added",
    });
  }
  for (const fn of functions.deleted) {
    out.set(keyOf(fn.path, fn.containerPath, fn.name), {
      path: fn.path,
      containerPath: fn.containerPath,
      startLine: fn.startLine,
      name: fn.name,
      detail: "",
      status: "deleted",
    });
  }
  // Modified = sig OR body. If both, combine the detail.
  const sigByKey = new Map<string, string>();
  for (const fn of functions.modifiedSignature) {
    sigByKey.set(
      keyOf(fn.path, fn.containerPath, fn.name),
      `${fn.before} → ${fn.after} params`,
    );
  }
  const bodyByKey = new Map<string, string>();
  for (const fn of functions.modifiedBody) {
    bodyByKey.set(
      keyOf(fn.path, fn.containerPath, fn.name),
      `Δcc ${fn.complexityDelta >= 0 ? "+" : ""}${fn.complexityDelta}, Δlen ${fn.lengthDelta >= 0 ? "+" : ""}${fn.lengthDelta}`,
    );
  }
  const allModKeys = new Set<string>([...sigByKey.keys(), ...bodyByKey.keys()]);
  for (const key of allModKeys) {
    if (out.has(key)) continue; // already covered as added/deleted
    const fn = [...functions.modifiedSignature, ...functions.modifiedBody].find(
      (f) => keyOf(f.path, f.containerPath, f.name) === key,
    );
    if (!fn) continue;
    const sig = sigByKey.get(key);
    const body = bodyByKey.get(key);
    const detailParts = [sig, body].filter((s): s is string => Boolean(s));
    out.set(key, {
      path: fn.path,
      containerPath: fn.containerPath,
      startLine: fn.startLine,
      name: fn.name,
      detail: detailParts.join("; "),
      status: "modified",
    });
  }
  return [...out.values()];
}

interface TreeNode {
  /** Display label for this node — directory segment, filename, or
   *  container name. */
  label: string;
  /** Repo-relative file path — non-null on file leaves and container
   *  branches; null on pure directory branches that span multiple
   *  files. */
  path: string | null;
  /** Node kind drives the indent icon + label color. */
  kind: "dir" | "file" | "container";
  children: Map<string, TreeNode>;
  /** Function rows attached at this depth (always inside a container
   *  or directly under a file leaf). */
  rows: RowEntry[];
}

function emptyNode(label: string, kind: TreeNode["kind"], path: string | null): TreeNode {
  return { label, kind, path, children: new Map(), rows: [] };
}

/** Build the unified tree: dir1 > dir2 > file > container > rows. */
function buildTree(rows: RowEntry[]): TreeNode {
  const root: TreeNode = emptyNode("", "dir", null);
  for (const row of rows) {
    const segments = row.path.split("/");
    const dirSegments = segments.slice(0, -1);
    const fileSegment = segments[segments.length - 1] ?? row.path;
    let cursor = root;
    for (const seg of dirSegments) {
      let next = cursor.children.get(seg);
      if (!next) {
        next = emptyNode(seg, "dir", null);
        cursor.children.set(seg, next);
      }
      cursor = next;
    }
    let fileNode = cursor.children.get(fileSegment);
    if (!fileNode) {
      fileNode = emptyNode(fileSegment, "file", row.path);
      cursor.children.set(fileSegment, fileNode);
    }
    let containerCursor = fileNode;
    for (const seg of row.containerPath) {
      let next = containerCursor.children.get(seg);
      if (!next) {
        next = emptyNode(seg, "container", row.path);
        containerCursor.children.set(seg, next);
      }
      containerCursor = next;
    }
    containerCursor.rows.push(row);
  }
  collapseSingleChildDirs(root);
  return root;
}

/**
 * Collapse runs of single-child directory nodes (e.g. `apps > desktop
 * > src > components`) into a single segment so the tree doesn't have
 * a long ladder of one-child branches before the first interesting
 * split. Only collapses dir → dir; files / containers stay as-is.
 */
function collapseSingleChildDirs(node: TreeNode): void {
  for (const [key, child] of node.children) {
    if (
      child.kind === "dir" &&
      child.children.size === 1 &&
      child.rows.length === 0
    ) {
      const [onlyKey, onlyChild] = [...child.children.entries()][0]!;
      if (onlyChild.kind === "dir") {
        const merged = emptyNode(`${child.label}/${onlyChild.label}`, "dir", null);
        merged.children = onlyChild.children;
        merged.rows = onlyChild.rows;
        node.children.set(key, merged);
        // Recurse on the merged node in case it can collapse further.
        collapseSingleChildDirs(node);
        return;
      }
    }
    collapseSingleChildDirs(child);
  }
}

interface NodeSummary {
  count: number;
  statuses: Set<RowStatus>;
}

function summarizeNode(node: TreeNode): NodeSummary {
  const statuses = new Set<RowStatus>();
  let count = 0;
  for (const row of node.rows) {
    statuses.add(row.status);
    count += 1;
  }
  for (const child of node.children.values()) {
    const s = summarizeNode(child);
    for (const st of s.statuses) statuses.add(st);
    count += s.count;
  }
  return { count, statuses };
}

function Tree({
  rows,
  onOpenFile,
  onOpenFunctionDiff,
}: {
  rows: RowEntry[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenFunctionDiff?(path: string, line: number): void;
}) {
  const root = buildTree(rows);
  const topLevel = [...root.children.values()].sort((a, b) => a.label.localeCompare(b.label));
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 1 }}>
      {topLevel.map((node) => (
        <TreeBranch
          key={`${node.kind}:${node.label}`}
          node={node}
          depth={0}
          onOpenFile={onOpenFile}
          onOpenFunctionDiff={onOpenFunctionDiff}
        />
      ))}
    </div>
  );
}

function TreeBranch({
  node,
  depth,
  onOpenFile,
  onOpenFunctionDiff,
}: {
  node: TreeNode;
  depth: number;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenFunctionDiff?(path: string, line: number): void;
}) {
  const summary = summarizeNode(node);
  const sortedChildren = [...node.children.values()].sort((a, b) => {
    // Directories first, then files, then containers — within each
    // kind alphabetical.
    const kindRank = (k: TreeNode["kind"]) => (k === "dir" ? 0 : k === "file" ? 1 : 2);
    const r = kindRank(a.kind) - kindRank(b.kind);
    if (r !== 0) return r;
    return a.label.localeCompare(b.label);
  });
  return (
    <details open style={{ paddingLeft: 0 }}>
      <summary style={summaryStyle(depth)}>
        <StatusBadges statuses={summary.statuses} />
        <span style={labelStyle(node.kind)}>
          {node.label}
          {node.kind === "dir" ? "/" : ""}
        </span>
        <span style={{ color: "var(--text-muted)", marginLeft: 6, fontSize: 11 }}>
          ({summary.count})
        </span>
      </summary>
      {sortedChildren.map((child) => (
        <TreeBranch
          key={`${child.kind}:${child.label}`}
          node={child}
          depth={depth + 1}
          onOpenFile={onOpenFile}
          onOpenFunctionDiff={onOpenFunctionDiff}
        />
      ))}
      {node.rows
        .slice()
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((row, i) => (
          <button
            key={`${row.name}:${i}`}
            type="button"
            data-testid="change-analysis-fn-row"
            onClick={(e) => {
              // Cmd/Ctrl-click → open the file directly. Default →
              // open the diff at the function's line.
              if (e.metaKey || e.ctrlKey) {
                onOpenFile(row.path, { newTab: true });
                return;
              }
              if (onOpenFunctionDiff && row.startLine > 0) {
                onOpenFunctionDiff(row.path, row.startLine);
              } else {
                onOpenFile(row.path);
              }
            }}
            style={{ ...fnRow, paddingLeft: 14 + (depth + 1) * 12 }}
            title={row.startLine > 0 ? `Open diff at line ${row.startLine}` : "Open diff"}
          >
            <StatusBadges statuses={new Set([row.status])} />
            <span style={{ color: "var(--text-primary)" }}>{row.name}</span>
            {row.detail ? (
              <span style={{ color: "var(--text-muted)", marginLeft: 6 }}>{row.detail}</span>
            ) : null}
          </button>
        ))}
    </details>
  );
}

function StatusBadges({ statuses }: { statuses: Set<RowStatus> }) {
  const order: RowStatus[] = ["added", "modified", "deleted"];
  return (
    <span style={{ display: "inline-flex", gap: 2, marginRight: 6, flexShrink: 0 }}>
      {order
        .filter((s) => statuses.has(s))
        .map((s) => (
          <StatusBadge key={s} status={s} />
        ))}
    </span>
  );
}

function StatusBadge({ status }: { status: RowStatus }) {
  const cfg = STATUS_STYLE[status];
  return (
    <span
      title={cfg.title}
      data-testid={`fn-badge-${status}`}
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: 14,
        height: 14,
        borderRadius: 3,
        fontSize: 9,
        fontWeight: 700,
        color: cfg.fg,
        background: cfg.bg,
      }}
    >
      {cfg.glyph}
    </span>
  );
}

const STATUS_STYLE: Record<RowStatus, { glyph: string; fg: string; bg: string; title: string }> = {
  added: {
    glyph: "A",
    fg: "var(--text-success, #16a34a)",
    bg: "rgba(22, 163, 74, 0.18)",
    title: "Added",
  },
  modified: {
    glyph: "M",
    fg: "var(--text-link, #2563eb)",
    bg: "rgba(37, 99, 235, 0.18)",
    title: "Modified",
  },
  deleted: {
    glyph: "D",
    fg: "var(--text-danger, #dc2626)",
    bg: "rgba(220, 38, 38, 0.18)",
    title: "Deleted",
  },
};

function labelStyle(kind: TreeNode["kind"]): React.CSSProperties {
  if (kind === "dir") {
    return { color: "var(--text-muted)", fontSize: 12 };
  }
  if (kind === "file") {
    return { color: "var(--text-primary)", fontSize: 12, fontWeight: 500 };
  }
  // container
  return { color: "var(--text-primary)", fontSize: 12, fontWeight: 500, fontStyle: "italic" };
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600, marginBottom: 8 };
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 12 };
const summaryStyle = (depth: number): React.CSSProperties => ({
  cursor: "pointer",
  fontSize: 12,
  padding: "2px 0",
  paddingLeft: depth * 12,
  userSelect: "none",
  display: "flex",
  alignItems: "center",
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
  display: "flex",
  alignItems: "center",
  width: "100%",
};
