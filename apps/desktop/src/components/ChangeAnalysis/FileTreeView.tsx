import { useMemo, useState } from "react";
import type { BranchChangeEntry, WorkspaceEntry } from "../../api.js";
import { TreeEntries } from "../LeftPanel/FileTree.js";
import type { ContextMenuTarget } from "../LeftPanel/shared.js";

interface Props {
  files: BranchChangeEntry[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

/**
 * Hierarchical file list for the Change Analysis drilldown. Wraps the
 * shared `TreeEntries` component with a synthesized entriesByDir map
 * so the existing StatusBadge / row chrome / sibling-navigation
 * plumbing reuses 1:1.
 *
 * Each `BranchChangeEntry` becomes a leaf `WorkspaceEntry` carrying
 * its `gitStatus`; intermediate directories are synthesized with
 * `gitStatus: null` (the existing StatusBadge skips null). Adding the
 * search filter prunes paths that don't match (case-insensitive,
 * substring); ancestors of matched paths stay so the hit context
 * remains visible.
 */
export function ChangeAnalysisFileTree({ files, onOpenFile }: Props) {
  const [search, setSearch] = useState("");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const filtered = useMemo(() => {
    if (!search.trim()) return files;
    const needle = search.toLowerCase();
    return files.filter((f) => f.path.toLowerCase().includes(needle));
  }, [files, search]);

  const { entriesByDir, allDirs } = useMemo(() => buildEntriesByDir(filtered), [filtered]);

  // Default: every directory expanded. The user's explicit collapse
  // overrides via the `collapsed` set. Search-filtered trees stay
  // fully expanded so matches are visible in context.
  const expandedDirs = useMemo(() => {
    const map: Record<string, boolean> = { "": true };
    for (const d of allDirs) map[d] = !collapsed.has(d);
    return map;
  }, [allDirs, collapsed]);

  const toggleDir = (path: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const handleExpandAll = () => setCollapsed(new Set());
  const handleCollapseAll = () => setCollapsed(new Set(allDirs));

  const rootEntries = entriesByDir[""] ?? [];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={toolbarRow}>
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Filter by path…"
          data-testid="change-analysis-file-search"
          style={searchInput}
        />
        <button
          type="button"
          onClick={handleExpandAll}
          data-testid="change-analysis-file-expand-all"
          style={smallButton}
          title="Expand all directories"
        >
          Expand all
        </button>
        <button
          type="button"
          onClick={handleCollapseAll}
          data-testid="change-analysis-file-collapse-all"
          style={smallButton}
          title="Collapse all directories"
        >
          Collapse all
        </button>
        <span style={{ color: "var(--text-muted)", fontSize: 11, marginLeft: "auto" }}>
          {filtered.length} of {files.length} file{files.length === 1 ? "" : "s"}
        </span>
      </div>
      {rootEntries.length === 0 ? (
        <div style={{ color: "var(--text-muted)", fontSize: 12, padding: 8 }}>
          {search.trim() ? "No files match the filter." : "No files."}
        </div>
      ) : (
        <TreeEntries
          parentPath=""
          entries={rootEntries}
          entriesByDir={entriesByDir}
          expandedDirs={expandedDirs}
          loadingDirs={{}}
          selectedFilePath={null}
          generatedSet={EMPTY_SET}
          onToggleDirectory={toggleDir}
          onOpenFile={onOpenFile}
          onOpenMenu={noopMenu}
        />
      )}
    </div>
  );
}

const EMPTY_SET = new Set<string>();
function noopMenu(_target: ContextMenuTarget | null) {}

/**
 * Build a `{ dirPath -> WorkspaceEntry[] }` map from a flat list of
 * changed-file entries. Files keep their actual gitStatus so the
 * existing StatusBadge renders A/M/D/R/U; directories are synthesized
 * with `gitStatus: null`. Returned `allDirs` is the set of every
 * directory path that appears anywhere in the synthesized tree.
 */
function buildEntriesByDir(files: BranchChangeEntry[]): {
  entriesByDir: Record<string, WorkspaceEntry[]>;
  allDirs: string[];
} {
  const entriesByDir: Record<string, WorkspaceEntry[]> = {};
  const dirSet = new Set<string>();
  // Make sure every directory entry appears as a child of its parent
  // exactly once. Files dedupe by path.
  const seenInDir = new Map<string, Set<string>>();

  function ensureSlot(dir: string) {
    if (!entriesByDir[dir]) entriesByDir[dir] = [];
    if (!seenInDir.has(dir)) seenInDir.set(dir, new Set());
  }

  for (const file of files) {
    const segments = file.path.split("/");
    // Synthesize each ancestor directory entry up the chain.
    let parentPath = "";
    for (let i = 0; i < segments.length - 1; i++) {
      const seg = segments[i]!;
      const fullPath = parentPath ? `${parentPath}/${seg}` : seg;
      ensureSlot(parentPath);
      const seen = seenInDir.get(parentPath)!;
      if (!seen.has(fullPath)) {
        seen.add(fullPath);
        entriesByDir[parentPath]!.push({
          name: seg,
          path: fullPath,
          kind: "directory",
          gitStatus: null,
          hasChanges: true,
        });
      }
      dirSet.add(fullPath);
      parentPath = fullPath;
    }
    // File leaf.
    ensureSlot(parentPath);
    const fileName = segments[segments.length - 1] ?? file.path;
    const seen = seenInDir.get(parentPath)!;
    if (!seen.has(file.path)) {
      seen.add(file.path);
      entriesByDir[parentPath]!.push({
        name: fileName,
        path: file.path,
        kind: "file",
        gitStatus: file.status,
        hasChanges: true,
      });
    }
  }
  // Sort each directory's children: directories first, then files,
  // alphabetical within each kind.
  for (const dir of Object.keys(entriesByDir)) {
    entriesByDir[dir]!.sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === "directory" ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }
  return { entriesByDir, allDirs: [...dirSet] };
}

const toolbarRow: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  paddingBottom: 4,
};
const searchInput: React.CSSProperties = {
  flex: 1,
  minWidth: 120,
  maxWidth: 320,
  padding: "4px 8px",
  background: "var(--surface-app)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  fontSize: 12,
};
const smallButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 12,
};
