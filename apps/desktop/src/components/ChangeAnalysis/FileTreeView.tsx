import { useMemo } from "react";
import type { BranchChangeEntry, GitFileStatus } from "../../api.js";
import { HierarchyView, type HierarchyNode, type HierarchyStatus } from "../HierarchyView/HierarchyView.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef } from "../../tabs/pageRefs.js";

interface Props {
  files: BranchChangeEntry[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  /** When supplied, plain click opens the file's diff in the current
   *  tab (browser-tab semantic, back returns to this list).
   *  Otherwise falls through to in-tab file open. */
  onOpenFileDiff?(path: string): void;
}

/**
 * Tree-style file list for the Change Analysis drilldown. Builds a
 * directory > file hierarchy and renders it through `HierarchyView`
 * so the toolbar (filter + Expand all / Collapse all), the chevron
 * toggle, and the status badges all match the Semantic view exactly.
 */
export function ChangeAnalysisFileTree({ files, onOpenFile, onOpenFileDiff }: Props) {
  const ctxNav = useOptionalPageNavigation();
  // Default click semantic: navigate **in-tab** to the file's diff
  // when an `onOpenFileDiff` is supplied; otherwise fall through to
  // an in-tab file open via the page-context chokepoint. Cmd/Ctrl-
  // click escapes to a new file tab.
  const openFile = (path: string, opts: { newTab: boolean }) => {
    if (opts.newTab) {
      onOpenFile(path, { newTab: true });
      return;
    }
    if (onOpenFileDiff) {
      onOpenFileDiff(path);
      return;
    }
    if (ctxNav) ctxNav.navigate(fileRef(path), { newTab: false });
    else onOpenFile(path, { newTab: false });
  };
  const tree = useMemo(() => buildTree(files, openFile), [files, openFile]);
  const total = files.length;
  return (
    <HierarchyView
      nodes={tree}
      testIdPrefix="change-analysis-file"
      searchPlaceholder="Filter by path…"
      emptyLabel="No files match the filter."
      toolbarExtra={
        <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
          {total} file{total === 1 ? "" : "s"}
        </span>
      }
    />
  );
}

interface RawDirNode {
  name: string;
  path: string;
  files: Array<{ name: string; entry: BranchChangeEntry }>;
  dirs: Map<string, RawDirNode>;
}

function buildTree(
  files: BranchChangeEntry[],
  openFile: (path: string, opts: { newTab: boolean }) => void,
): HierarchyNode[] {
  const root: RawDirNode = { name: "", path: "", files: [], dirs: new Map() };
  for (const file of files) {
    const segments = file.path.split("/");
    const fileName = segments[segments.length - 1] ?? file.path;
    let cursor = root;
    for (let i = 0; i < segments.length - 1; i++) {
      const seg = segments[i]!;
      let next = cursor.dirs.get(seg);
      if (!next) {
        const dirPath = cursor.path ? `${cursor.path}/${seg}` : seg;
        next = { name: seg, path: dirPath, files: [], dirs: new Map() };
        cursor.dirs.set(seg, next);
      }
      cursor = next;
    }
    cursor.files.push({ name: fileName, entry: file });
  }
  return materialize(root, "", openFile);
}

function materialize(
  node: RawDirNode,
  idPrefix: string,
  openFile: (path: string, opts: { newTab: boolean }) => void,
): HierarchyNode[] {
  const out: HierarchyNode[] = [];
  // Directories first, alphabetical.
  const dirsSorted = [...node.dirs.values()].sort((a, b) => a.name.localeCompare(b.name));
  for (const d of dirsSorted) {
    const id = `${idPrefix}/dir:${d.path}`;
    const children = materialize(d, id, openFile);
    const summary = summarize(d);
    out.push({
      id,
      label: `${d.name}/`,
      icon: <FolderIcon />,
      statuses: summary.statuses,
      count: summary.count,
      children,
    });
  }
  // Then files.
  const filesSorted = [...node.files].sort((a, b) => a.name.localeCompare(b.name));
  for (const f of filesSorted) {
    const id = `${idPrefix}/file:${f.entry.path}`;
    out.push({
      id,
      label: f.name,
      icon: <FileIcon />,
      statuses: gitStatusToHierarchy(f.entry.status),
      detail: formatAddDel(f.entry.additions ?? 0, f.entry.deletions ?? 0),
      onDrill: (e) => {
        // Plain click → in-tab navigate (handled by openFile via the
        // PageNavigationContext chokepoint when present). Cmd/Ctrl-
        // click → new-tab escape. Lists never default-open new tabs.
        openFile(f.entry.path, { newTab: e.metaKey || e.ctrlKey });
      },
      drillTitle: `Open ${f.entry.path}`,
      children: [],
    });
  }
  return out;
}

interface DirSummary {
  count: number;
  statuses: Set<HierarchyStatus>;
}

function summarize(node: RawDirNode): DirSummary {
  const statuses = new Set<HierarchyStatus>();
  let count = 0;
  for (const file of node.files) {
    const fileStatuses = gitStatusToHierarchy(file.entry.status);
    for (const s of fileStatuses) statuses.add(s);
    count += 1;
  }
  for (const child of node.dirs.values()) {
    const inner = summarize(child);
    for (const s of inner.statuses) statuses.add(s);
    count += inner.count;
  }
  return { count, statuses };
}

function gitStatusToHierarchy(status: GitFileStatus): Set<HierarchyStatus> {
  if (status === "added" || status === "untracked") return new Set(["added"]);
  if (status === "deleted") return new Set(["deleted"]);
  // modified, renamed → "modified"
  return new Set(["modified"]);
}

function formatAddDel(adds: number, dels: number): string {
  if (adds === 0 && dels === 0) return "";
  return `+${adds} −${dels}`;
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
