import { useMemo } from "react";
import type { FunctionsBuckets } from "./analysisHelpers.js";
import { useRouteDispatch } from "../../tabs/RouteLink.js";
import { changeAnalysisRef, type ChangeAnalysisTarget } from "../../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import {
  HierarchyView,
  type HierarchyNode,
  type HierarchyStatus,
} from "../HierarchyView/HierarchyView.js";

interface FunctionsCardProps {
  functions: FunctionsBuckets;
  /** Click target for function leaf rows. Falls back to opening the
   *  file in the editor if the host doesn't supply
   *  `onOpenFunctionDiff`. */
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  /** Preferred click target — opens the file's diff tab and reveals
   *  the function's start line. */
  onOpenFunctionDiff?(path: string, line: number): void;
  /** Analysis target. Required so directory-branch clicks can route
   *  to the directory-scoped Change Analysis drilldown. */
  target?: ChangeAnalysisTarget;
}

type RowStatus = "added" | "modified" | "deleted";

interface RowEntry {
  path: string;
  containerPath: string[];
  startLine: number;
  name: string;
  detail: string;
  status: RowStatus;
}

export function FunctionsCard({ functions, onOpenFile, onOpenFunctionDiff, target }: FunctionsCardProps) {
  const ctxNav = useOptionalPageNavigation();
  const rows = useMemo(() => flattenRows(functions), [functions]);
  const nodes = useMemo(
    () => buildNodes(rows, target, onOpenFile, onOpenFunctionDiff, ctxNav),
    [rows, target, onOpenFile, onOpenFunctionDiff, ctxNav],
  );

  return (
    <section data-testid="change-analysis-functions" style={card}>
      <div style={header}>Functions</div>
      {rows.length === 0 ? (
        <div style={muted}>
          No function-level changes detected (the changed files may be in unsupported languages).
        </div>
      ) : (
        <HierarchyView
          nodes={nodes}
          testIdPrefix="change-analysis-fn"
          searchPlaceholder="Filter by name…"
          emptyLabel="No functions match the filter."
        />
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
    if (out.has(key)) continue;
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

interface RawNode {
  label: string;
  /** Cumulative directory path for `kind === "dir"` nodes. */
  dirPath: string | null;
  /** Repo-relative file path on file leaves and container branches. */
  filePath: string | null;
  kind: "dir" | "file" | "container";
  children: Map<string, RawNode>;
  rows: RowEntry[];
}

function emptyRaw(label: string, kind: RawNode["kind"], filePath: string | null, dirPath: string | null = null): RawNode {
  return { label, kind, filePath, dirPath, children: new Map(), rows: [] };
}

function buildRawTree(rows: RowEntry[]): RawNode {
  const root = emptyRaw("", "dir", null, "");
  for (const row of rows) {
    const segments = row.path.split("/");
    const dirSegments = segments.slice(0, -1);
    const fileSegment = segments[segments.length - 1] ?? row.path;
    let cursor = root;
    let cursorDirPath = "";
    for (const seg of dirSegments) {
      let next = cursor.children.get(seg);
      const childDirPath = cursorDirPath ? `${cursorDirPath}/${seg}` : seg;
      if (!next) {
        next = emptyRaw(seg, "dir", null, childDirPath);
        cursor.children.set(seg, next);
      }
      cursor = next;
      cursorDirPath = childDirPath;
    }
    let fileNode = cursor.children.get(fileSegment);
    if (!fileNode) {
      fileNode = emptyRaw(fileSegment, "file", row.path);
      cursor.children.set(fileSegment, fileNode);
    }
    let containerCursor = fileNode;
    for (const seg of row.containerPath) {
      let next = containerCursor.children.get(seg);
      if (!next) {
        next = emptyRaw(seg, "container", row.path);
        containerCursor.children.set(seg, next);
      }
      containerCursor = next;
    }
    containerCursor.rows.push(row);
  }
  collapseSingleChildDirs(root);
  return root;
}

/** Collapse runs of single-child dir nodes into one segment. */
function collapseSingleChildDirs(node: RawNode): void {
  for (const [key, child] of node.children) {
    if (child.kind === "dir" && child.children.size === 1 && child.rows.length === 0) {
      const [, onlyChild] = [...child.children.entries()][0]!;
      if (onlyChild.kind === "dir") {
        const merged = emptyRaw(
          `${child.label}/${onlyChild.label}`,
          "dir",
          null,
          onlyChild.dirPath,
        );
        merged.children = onlyChild.children;
        merged.rows = onlyChild.rows;
        node.children.set(key, merged);
        collapseSingleChildDirs(node);
        return;
      }
    }
    collapseSingleChildDirs(child);
  }
}

interface RawSummary {
  count: number;
  statuses: Set<HierarchyStatus>;
}
function summarize(node: RawNode): RawSummary {
  const statuses = new Set<HierarchyStatus>();
  let count = 0;
  for (const row of node.rows) {
    statuses.add(row.status);
    count += 1;
  }
  for (const child of node.children.values()) {
    const inner = summarize(child);
    for (const s of inner.statuses) statuses.add(s);
    count += inner.count;
  }
  return { count, statuses };
}

function buildNodes(
  rows: RowEntry[],
  target: ChangeAnalysisTarget | undefined,
  onOpenFile: (path: string, opts?: { newTab?: boolean }) => void,
  onOpenFunctionDiff: ((path: string, line: number) => void) | undefined,
  ctxNav: ReturnType<typeof useOptionalPageNavigation>,
): HierarchyNode[] {
  const tree = buildRawTree(rows);
  const top = [...tree.children.values()].sort((a, b) => a.label.localeCompare(b.label));
  return top.map((c) => toHierarchyNode(c, "", target, onOpenFile, onOpenFunctionDiff, ctxNav));
}

function toHierarchyNode(
  node: RawNode,
  idPrefix: string,
  target: ChangeAnalysisTarget | undefined,
  onOpenFile: (path: string, opts?: { newTab?: boolean }) => void,
  onOpenFunctionDiff: ((path: string, line: number) => void) | undefined,
  ctxNav: ReturnType<typeof useOptionalPageNavigation>,
): HierarchyNode {
  const id = `${idPrefix}/${node.kind}:${node.label}`;
  const sortedChildren = [...node.children.values()].sort((a, b) => {
    const rank = (k: RawNode["kind"]) => (k === "dir" ? 0 : k === "file" ? 1 : 2);
    const r = rank(a.kind) - rank(b.kind);
    if (r !== 0) return r;
    return a.label.localeCompare(b.label);
  });
  const childNodes: HierarchyNode[] = sortedChildren.map((c) =>
    toHierarchyNode(c, id, target, onOpenFile, onOpenFunctionDiff, ctxNav),
  );
  // Function leaves attached at this depth.
  const leafRows = node.rows
    .slice()
    .sort((a, b) => a.name.localeCompare(b.name))
    .map<HierarchyNode>((row, i) => ({
      id: `${id}/fn:${row.name}:${i}`,
      label: row.name,
      icon: <FnIcon />,
      statuses: new Set<HierarchyStatus>([row.status]),
      detail: row.detail,
      onDrill: (e) => {
        if (e.metaKey || e.ctrlKey) {
          onOpenFile(row.path, { newTab: true });
          return;
        }
        if (onOpenFunctionDiff && row.startLine > 0) {
          onOpenFunctionDiff(row.path, row.startLine);
        } else {
          onOpenFile(row.path);
        }
      },
      drillTitle: row.startLine > 0 ? `Open diff at line ${row.startLine}` : "Open diff",
      testId: "change-analysis-fn-row",
      children: [],
    }));
  const summary = summarize(node);
  const childrenAll = [...childNodes, ...leafRows];
  const label = node.kind === "dir" ? `${node.label}/` : node.label;

  // Drill behavior:
  //   - file → file diff at line 1
  //   - dir  → directory-scoped Change Analysis page (in-tab via context)
  //   - container → no drill
  let onDrill: ((e: React.MouseEvent) => void) | undefined;
  let drillTitle: string | undefined;
  if (node.kind === "file" && node.filePath) {
    const filePath = node.filePath;
    onDrill = (e) => {
      if (e.metaKey || e.ctrlKey) {
        onOpenFile(filePath, { newTab: true });
        return;
      }
      if (onOpenFunctionDiff) onOpenFunctionDiff(filePath, 1);
      else onOpenFile(filePath);
    };
    drillTitle = `Open diff for ${filePath}`;
  } else if (node.kind === "dir" && node.dirPath != null && target) {
    const dirPath = node.dirPath;
    const ref = changeAnalysisRef(target, { kind: "dir", value: dirPath });
    onDrill = (e) => {
      const newTab = e.metaKey || e.ctrlKey;
      if (ctxNav) {
        ctxNav.navigate(ref, { newTab });
      } else {
        // No page-context fallback — open via onOpenFile is wrong,
        // so silently no-op when used outside a Page.
      }
    };
    drillTitle = `Drill into ${dirPath}/`;
  }

  return {
    id,
    label,
    icon: iconFor(node.kind),
    statuses: summary.statuses,
    count: summary.count,
    onDrill,
    drillTitle,
    children: childrenAll,
  };
}

function iconFor(kind: RawNode["kind"]) {
  if (kind === "dir") return <FolderIcon />;
  if (kind === "file") return <FileIcon />;
  return <ContainerIcon />;
}

function FolderIcon() {
  return (
    <svg viewBox="0 0 16 16" width="1em" height="1em" aria-hidden style={{ display: "block" }}>
      <path
        d="M1.5 3.5 a1 1 0 0 1 1 -1 h3.5 l1.5 1.5 h6.5 a1 1 0 0 1 1 1 v8 a1 1 0 0 1 -1 1 h-11.5 a1 1 0 0 1 -1 -1 z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
      />
    </svg>
  );
}
function FileIcon() {
  return (
    <svg viewBox="0 0 16 16" width="1em" height="1em" aria-hidden style={{ display: "block" }}>
      <path
        d="M3.5 1.5 h6 l3 3 v10 a1 1 0 0 1 -1 1 h-8 a1 1 0 0 1 -1 -1 v-12 a1 1 0 0 1 1 -1 z M9.5 1.5 v3 h3"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
    </svg>
  );
}
function ContainerIcon() {
  // Squarish glyph for class/impl/module/namespace containers.
  return (
    <svg viewBox="0 0 16 16" width="1em" height="1em" aria-hidden style={{ display: "block" }}>
      <rect x="2.5" y="2.5" width="11" height="11" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.2" />
      <path d="M2.5 6 h11 M6 2.5 v11" stroke="currentColor" strokeWidth="0.9" fill="none" opacity="0.7" />
    </svg>
  );
}
function FnIcon() {
  // ƒ glyph for functions — keep it within 1em so the row height
  // doesn't grow.
  return (
    <span style={{ fontFamily: "serif", fontStyle: "italic", fontSize: "1em", lineHeight: 1 }}>
      ƒ
    </span>
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
