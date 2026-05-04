import { useMemo, useState } from "react";
import type { FunctionsBuckets, FunctionVisibility } from "./analysisHelpers.js";
import { useRouteDispatch } from "../../tabs/RouteLink.js";
import { changeAnalysisRef, fileRef, type ChangeAnalysisTarget } from "../../tabs/pageRefs.js";
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
  visibility: FunctionVisibility;
}

export function FunctionsCard({ functions, onOpenFile, onOpenFunctionDiff, target }: FunctionsCardProps) {
  const ctxNav = useOptionalPageNavigation();
  const [showPrivate, setShowPrivate] = useState(true);
  const allRows = useMemo(() => flattenRows(functions), [functions]);
  const visibleRows = useMemo(
    () => (showPrivate ? allRows : allRows.filter((r) => r.visibility !== "private")),
    [allRows, showPrivate],
  );
  const nodes = useMemo(
    () => buildNodes(visibleRows, target, onOpenFile, onOpenFunctionDiff, ctxNav),
    [visibleRows, target, onOpenFile, onOpenFunctionDiff, ctxNav],
  );
  const privateCount = useMemo(
    () => allRows.filter((r) => r.visibility === "private").length,
    [allRows],
  );

  return (
    <section data-testid="change-analysis-functions" style={card}>
      <div style={headerRow}>
        <span style={{ fontWeight: 600 }}>Functions</span>
        {privateCount > 0 ? (
          <label style={toggleLabel} title="Heuristic per language. See language-specific notes for what 'private' means.">
            <input
              type="checkbox"
              data-testid="change-analysis-show-private"
              checked={showPrivate}
              onChange={(e) => setShowPrivate(e.target.checked)}
            />
            <span>Show private ({privateCount})</span>
          </label>
        ) : null}
      </div>
      {allRows.length === 0 ? (
        <div style={muted}>
          No function-level changes detected (the changed files may be in unsupported languages).
        </div>
      ) : visibleRows.length === 0 ? (
        <div style={muted}>
          All function changes are private. Toggle "Show private" to see them.
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
      visibility: fn.visibility,
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
      visibility: fn.visibility,
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
      visibility: fn.visibility,
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

interface PathSegment {
  label: string;
  /** Filesystem path the segment represents — used for drill (dir
   *  scope or file open). For "dir"-kind intermediate segments this
   *  is the directory; for "file"-kind leaves this is the file. */
  refPath: string;
  kind: "dir" | "file";
}

/**
 * Compute the grouping segments for a file path. For Rust files
 * inside `crates/<crate-name>/src/...`, this returns the crate +
 * module path (e.g. `[oxplow_app, services, stream]`) so the
 * semantic tree groups by language-level structure rather than
 * filesystem layout. Falls back to filesystem segments for every
 * other file.
 */
function pathSegments(filePath: string): PathSegment[] {
  if (filePath.endsWith(".rs")) {
    const rust = rustGroupingSegments(filePath);
    if (rust) return rust;
  }
  const parts = filePath.split("/");
  const out: PathSegment[] = [];
  let cum = "";
  for (let i = 0; i < parts.length - 1; i++) {
    const p = parts[i]!;
    cum = cum ? `${cum}/${p}` : p;
    out.push({ label: p, refPath: cum, kind: "dir" });
  }
  out.push({ label: parts[parts.length - 1] ?? filePath, refPath: filePath, kind: "file" });
  return out;
}

function rustGroupingSegments(filePath: string): PathSegment[] | null {
  const m = /^crates\/([^/]+)\/src\/(.*)$/.exec(filePath);
  if (!m) return null;
  const crateDir = m[1]!;
  const rest = m[2]!;
  const crateName = crateDir.replace(/-/g, "_");
  const restNoExt = rest.replace(/\.rs$/, "");
  // lib.rs / main.rs are the crate root — the file IS the crate.
  if (restNoExt === "lib" || restNoExt === "main") {
    return [{ label: crateName, refPath: filePath, kind: "file" }];
  }
  let modParts = restNoExt.split("/");
  // `foo/mod.rs` is the `foo` module — drop the trailing `mod` so
  // the leaf is the parent directory's name.
  if (modParts[modParts.length - 1] === "mod") modParts = modParts.slice(0, -1);
  if (modParts.length === 0) {
    return [{ label: crateName, refPath: filePath, kind: "file" }];
  }
  const segs: PathSegment[] = [];
  segs.push({ label: crateName, refPath: `crates/${crateDir}`, kind: "dir" });
  let cum = `crates/${crateDir}/src`;
  for (let i = 0; i < modParts.length - 1; i++) {
    const p = modParts[i]!;
    cum = `${cum}/${p}`;
    segs.push({ label: p, refPath: cum, kind: "dir" });
  }
  // Deepest segment = the file itself.
  segs.push({ label: modParts[modParts.length - 1] ?? "", refPath: filePath, kind: "file" });
  return segs;
}

function buildRawTree(rows: RowEntry[]): RawNode {
  const root = emptyRaw("", "dir", null, "");
  for (const row of rows) {
    const segs = pathSegments(row.path);
    let cursor = root;
    let fileNode: RawNode | null = null;
    for (let i = 0; i < segs.length; i++) {
      const seg = segs[i]!;
      const isLast = i === segs.length - 1;
      let next = cursor.children.get(seg.label);
      if (!next) {
        next = isLast
          ? emptyRaw(seg.label, "file", row.path)
          : emptyRaw(seg.label, "dir", null, seg.refPath);
        cursor.children.set(seg.label, next);
      }
      cursor = next;
      if (isLast) fileNode = next;
    }
    if (!fileNode) continue;
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
      icon: <FnIcon visibility={row.visibility} />,
      labelColor: visibilityColor(row.visibility),
      statuses: new Set<HierarchyStatus>([row.status]),
      detail: row.detail,
      onDrill: (e) => {
        // Plain click → open the diff for this file at the function's
        // line, *in the current tab* (browser-tab semantic; back
        // returns to the analysis dashboard). Cmd/Ctrl-click → new
        // tab via the file-open path (escape hatch).
        const newTab = e.metaKey || e.ctrlKey;
        if (newTab) {
          onOpenFile(row.path, { newTab: true });
          return;
        }
        if (onOpenFunctionDiff && row.startLine > 0) {
          onOpenFunctionDiff(row.path, row.startLine);
          return;
        }
        // Fallback: open the file in-tab.
        if (ctxNav) ctxNav.navigate(fileRef(row.path), { newTab: false });
        else onOpenFile(row.path);
      },
      drillTitle: row.startLine > 0 ? `Open diff at line ${row.startLine}` : `Open diff for ${row.path}`,
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
      // File branches open the file's diff at line 1 *in the current
      // tab*. Cmd/Ctrl-click → new tab via the file-open escape.
      const newTab = e.metaKey || e.ctrlKey;
      if (newTab) {
        onOpenFile(filePath, { newTab: true });
        return;
      }
      if (onOpenFunctionDiff) {
        onOpenFunctionDiff(filePath, 1);
        return;
      }
      if (ctxNav) ctxNav.navigate(fileRef(filePath), { newTab: false });
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
function visibilityColor(visibility: FunctionVisibility): string {
  if (visibility === "private") return "var(--text-muted)";
  if (visibility === "public") return "var(--text-primary)";
  return "var(--text-secondary)";
}

function visibilityTitle(visibility: FunctionVisibility): string {
  if (visibility === "private") return "Private (heuristic)";
  if (visibility === "public") return "Public (heuristic)";
  return "Visibility unknown";
}

function FnIcon({ visibility }: { visibility?: FunctionVisibility }) {
  // ƒ glyph for functions — colored by visibility so the user can
  // see at a glance which rows are public vs private. Stays within
  // 1em so the row height doesn't grow.
  const v = visibility ?? "unknown";
  return (
    <span
      title={visibilityTitle(v)}
      style={{
        fontFamily: "serif",
        fontStyle: "italic",
        fontSize: "1em",
        lineHeight: 1,
        color: visibilityColor(v),
      }}
    >
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
const headerRow: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 12,
  marginBottom: 8,
};
const toggleLabel: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 4,
  fontSize: 11,
  color: "var(--text-muted)",
  cursor: "pointer",
  marginLeft: "auto",
};
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: 12 };
