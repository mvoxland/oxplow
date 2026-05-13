import { useEffect, useState } from "react";
import type { Stream } from "../tauri-bridge/index.js";
import { listWorkspaceEntries, type WorkspaceEntry } from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { directoryRef, fileRef } from "../tabs/pageRefs.js";

export interface DirectoryPageProps {
  stream: Stream | null;
  /** Workspace-relative path to the directory (no trailing slash). */
  path: string;
  /** Open another page (file or sibling directory) in the current
   *  tab's nav slot — wired to `handleNavigateInTab` upstream so
   *  back/forward stays inside the directory tab. */
  onOpenPage(ref: TabRef): void;
}

interface SortedEntries {
  dirs: WorkspaceEntry[];
  files: WorkspaceEntry[];
}

function partitionAndSort(rows: ReadonlyArray<WorkspaceEntry>): SortedEntries {
  const dirs: WorkspaceEntry[] = [];
  const files: WorkspaceEntry[] = [];
  for (const r of rows) {
    if (r.kind === "directory") dirs.push(r);
    else files.push(r);
  }
  const cmp = (a: WorkspaceEntry, b: WorkspaceEntry) => a.name.localeCompare(b.name);
  dirs.sort(cmp);
  files.sort(cmp);
  return { dirs, files };
}

/**
 * Renders the contents of a workspace directory as a flat list of
 * subdirectories and files. Reached via the `[[dir:<path>]]` wikilink
 * shape — `MarkdownView` parses the `dir:` href into a `directory`
 * page kind and `App` opens it as a per-thread page tab.
 */
export function DirectoryPage({ stream, path, onOpenPage }: DirectoryPageProps) {
  const [entries, setEntries] = useState<SortedEntries>({ dirs: [], files: [] });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!stream) {
      setEntries({ dirs: [], files: [] });
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    listWorkspaceEntries(stream.id, path)
      .then((rows) => {
        if (cancelled) return;
        setEntries(partitionAndSort(rows));
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e instanceof Error ? e.message : e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [stream, path]);

  const total = entries.dirs.length + entries.files.length;

  return (
    <Page testId="page-directory" title={path || "/"}>
      <div style={{ padding: "16px 20px", maxWidth: 720 }}>
        <p
          style={{
            color: "var(--text-secondary)",
            margin: "0 0 16px",
            fontSize: "var(--text-sm)",
          }}
        >
          Workspace directory <code>{path || "/"}</code>.
        </p>
        {loading ? (
          <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>Loading…</div>
        ) : null}
        {error ? (
          <div
            data-testid="page-directory-error"
            style={{ color: "var(--severity-critical)", fontSize: "var(--text-xs)" }}
          >
            {error}
          </div>
        ) : null}
        {!loading && !error && total === 0 ? (
          <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
            (empty)
          </div>
        ) : null}
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {entries.dirs.map((entry) => (
            <DirectoryEntryRow
              key={entry.path}
              entry={entry}
              onClick={() => onOpenPage(directoryRef(entry.path))}
            />
          ))}
          {entries.files.map((entry) => (
            <DirectoryEntryRow
              key={entry.path}
              entry={entry}
              onClick={() => onOpenPage(fileRef(entry.path))}
            />
          ))}
        </div>
      </div>
    </Page>
  );
}

function DirectoryEntryRow({
  entry,
  onClick,
}: {
  entry: WorkspaceEntry;
  onClick(): void;
}) {
  const isDir = entry.kind === "directory";
  return (
    <button
      type="button"
      data-testid={`page-directory-entry-${entry.name}`}
      onClick={onClick}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "8px 10px",
        background: "var(--surface-tab-inactive)",
        color: "var(--text-primary)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        cursor: "pointer",
        fontSize: "var(--text-sm)",
        textAlign: "left",
      }}
    >
      <span aria-hidden style={{ fontSize: "var(--text-base)" }}>{isDir ? "📁" : "📄"}</span>
      <span>{entry.name}{isDir ? "/" : ""}</span>
    </button>
  );
}
