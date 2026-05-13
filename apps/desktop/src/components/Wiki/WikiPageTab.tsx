import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  deleteWikiPage,
  listWikiPages,
  readWikiPageBody,
  subscribeWikiPageEvents,
  writeWikiPageBody,
  type Stream,
  type WikiPageSummary,
} from "../../api.js";
import { MarkdownView } from "./MarkdownView.js";
import { recordOpError } from "../opErrorsStore.js";
import { usePageTitle, useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef } from "../../tabs/pageRefs.js";
import { usePageSnapshot } from "../../tabs/usePageSnapshot.js";

type FreshnessStatus = WikiPageSummary["freshness"];

const FRESHNESS_LABEL: Record<FreshnessStatus, string> = {
  "fresh": "fresh",
  "stale": "stale",
  "very-stale": "very stale",
};

const FRESHNESS_COLOR: Record<FreshnessStatus, string> = {
  "fresh": "var(--freshness-fresh)",
  "stale": "var(--freshness-stale)",
  "very-stale": "var(--freshness-very-stale)",
};

interface Props {
  stream: Stream;
  slug: string;
  onClosed: () => void;
  /** Called for plain in-tab wikilink navigation. Routes through the
   *  host's PageNavigationContext so back/forward live in the shared
   *  chrome rather than a per-WikiPageTab history. */
  onNavigateInternalWikiPage: (slug: string) => void;
  onOpenWikiPageInNewTab: (slug: string) => void;
  onOpenFile: (path: string) => void;
  /** Optional handler for directory wikilink clicks — opens the
   *  DirectoryPage tab listing the folder contents. */
  onOpenDirectory?: (path: string) => void;
  /** Optional handler for git-commit wikilink clicks — opens the
   *  GitCommitPage for the SHA. */
  onOpenCommit?: (sha: string) => void;
  /** Optional handler for external (http/https) link clicks — host opens
   *  it as an in-app external-url tab. Falls back to OS browser when
   *  unset. */
  onOpenExternalUrl?: (url: string) => void;
}

export function WikiPageTab({ stream, slug, onClosed, onNavigateInternalWikiPage, onOpenWikiPageInNewTab, onOpenFile, onOpenDirectory, onOpenCommit, onOpenExternalUrl }: Props) {
  const [summary, setSummary] = useState<WikiPageSummary | null>(null);
  const [body, setBody] = useState<string>("");
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<string>("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);

  // Title flows through the shared PageNavigationContext so it surfaces in
  // the chrome header and the tab strip without a duplicate row inside the
  // wiki-page body. Falls back to the slug until the summary loads.
  usePageTitle(summary?.title ?? slug);

  // Persist scroll position across restart. We track scrollTop in a
  // ref (not state) so onScroll doesn't re-render the page — and,
  // critically, so the restore effect can't fight the user's active
  // scrolling. State-driven snap-back created an onScroll → setState
  // → effect → el.scrollTop = stale_value → onScroll loop that
  // visibly flickered the page while the wheel was moving.
  const scrollHostRef = useRef<HTMLDivElement | null>(null);
  const scrollYRef = useRef(0);
  const pendingRestoreRef = useRef<number | null>(null);
  usePageSnapshot<{ scrollY: number }>({
    serialize: () => ({ scrollY: scrollYRef.current }),
    restore: (snap) => {
      if (typeof snap.scrollY === "number") {
        scrollYRef.current = snap.scrollY;
        pendingRestoreRef.current = snap.scrollY;
      }
    },
    // Re-serialize whenever the body changes (so the latest scroll
    // position is captured at the moment the markdown updates) and
    // when the user navigates away (closePageTab triggers this via
    // the surrounding context unmount). We don't tick on every
    // scroll event — the ref already holds the latest value and the
    // snapshot layer reads it on dep change.
    deps: [body],
  });
  // Apply the restored / pending scroll position only when the body
  // changes (initial load + markdown re-renders that may shift
  // offsets briefly). Never as a reaction to the user's own scroll.
  useEffect(() => {
    const el = scrollHostRef.current;
    if (!el) return;
    const target = pendingRestoreRef.current;
    if (target == null) return;
    if (Math.abs(el.scrollTop - target) > 1) el.scrollTop = target;
    pendingRestoreRef.current = null;
  }, [body]);

  const refresh = useCallback(async () => {
    try {
      const all = await listWikiPages(stream.id);
      setSummary(all.find((n) => n.slug === slug) ?? null);
    } catch {}
    try {
      const text = await readWikiPageBody(stream.id, slug);
      setBody(text);
      setNotFound(false);
      setLoadError(null);
    } catch (error) {
      const message = String(error);
      if (/(wiki page|note) not found/i.test(message)) {
        setNotFound(true);
        setLoadError(null);
        setBody("");
      } else {
        setLoadError(message);
        setNotFound(false);
      }
    }
  }, [stream.id, slug]);

  useEffect(() => {
    void refresh();
    setEditing(false);
  }, [refresh]);

  // Stable subscription: hold the latest refresh in a ref and
  // subscribe once. The earlier `[refresh]` dependency tore the
  // subscription down + re-installed it whenever the callback's
  // identity changed (e.g. on slug change inside the same tab). The
  // teardown synchronously sets `stopped = true` on the bridge, but
  // the matching Tauri `listen()` cleanup is awaited from a Promise
  // — so between teardown and the new `listen()` resolving, any
  // `WikiPagesChanged` event the backend emitted was dropped on the
  // floor, leaving the page stuck on the prior body until the user
  // closed and reopened the tab.
  const refreshRef = useRef(refresh);
  useEffect(() => { refreshRef.current = refresh; }, [refresh]);
  useEffect(() => subscribeWikiPageEvents((changedSlug) => {
    if (changedSlug !== slug) return;
    void refreshRef.current();
  }), [slug]);

  const [draftInitialized, setDraftInitialized] = useState(false);

  useEffect(() => {
    if (!draftInitialized) {
      setDraft(body);
      setDraftInitialized(true);
    }
  }, [body, draftInitialized]);

  useEffect(() => {
    setDraftInitialized(false);
  }, [slug]);

  const enterEdit = useCallback(() => {
    if (!draftInitialized) {
      setDraft(body);
      setDraftInitialized(true);
    }
    setEditing(true);
  }, [body, draftInitialized]);

  const enterView = useCallback(() => {
    setEditing(false);
  }, []);

  const handleRevert = useCallback(() => {
    setDraft(body);
  }, [body]);

  const handleSave = useCallback(async () => {
    try {
      await writeWikiPageBody(stream.id, slug, draft);
      setBody(draft);
    } catch (error) {
      recordOpError({
        label: `Save wiki page "${slug}"`,
        message: String(error),
      });
    }
  }, [stream.id, slug, draft]);

  const handleCreate = useCallback(async () => {
    const seed = `# ${slug}\n\n`;
    try {
      await writeWikiPageBody(stream.id, slug, seed);
      setNotFound(false);
      setBody(seed);
      setDraft(seed);
      setDraftInitialized(true);
      setEditing(true);
    } catch (error) {
      recordOpError({
        label: `Create wiki page "${slug}"`,
        message: String(error),
      });
    }
  }, [stream.id, slug]);

  const handleDelete = useCallback(async () => {
    if (!window.confirm(`Delete wiki page "${slug}"? The file will be removed.`)) return;
    try {
      await deleteWikiPage(stream.id, slug);
      onClosed();
    } catch (error) {
      recordOpError({
        label: `Delete wiki page "${slug}"`,
        message: String(error),
      });
    }
  }, [stream.id, slug, onClosed]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 6,
          padding: "4px 12px",
          borderBottom: "1px solid var(--border-subtle)",
          background: "var(--surface-app)",
          fontSize: 12,
          flexShrink: 0,
        }}
      >
        {summary && <FreshnessBadge note={summary} />}
        <div style={{ flex: 1 }} />
        {notFound ? (
          <button type="button" onClick={() => void handleCreate()}>Create page</button>
        ) : (
          <>
            {editing ? (
              <button type="button" onClick={enterView} title="Switch to view mode">View</button>
            ) : (
              <button type="button" onClick={enterEdit} title="Switch to edit mode">Edit</button>
            )}
            <button
              type="button"
              onClick={() => void handleSave()}
              disabled={!editing || draft === body}
              title={draft === body ? "No unsaved changes" : "Save changes"}
            >
              Save
            </button>
            <button
              type="button"
              onClick={handleRevert}
              disabled={!editing || draft === body}
              title="Discard unsaved changes"
            >
              Revert
            </button>
            <button type="button" onClick={() => void handleDelete()} title="Delete wiki page">Delete</button>
          </>
        )}
      </div>
      <div
        ref={scrollHostRef}
        onScroll={(e) => { scrollYRef.current = (e.currentTarget as HTMLDivElement).scrollTop; }}
        style={{ flex: 1, minHeight: 0, overflow: "auto", padding: 12 }}
      >
        {notFound ? (
          <div style={{ color: "var(--text-muted)", fontSize: 13 }}>
            <div style={{ fontSize: 15, marginBottom: 8, color: "var(--text-primary)" }}>Page not found</div>
            <div>No wiki page exists with slug <code>{slug}</code>.</div>
            <div style={{ marginTop: 8 }}>
              Click <strong>Create page</strong> above to start a new wiki page at <code>.oxplow/wiki/{slug}.md</code>.
            </div>
          </div>
        ) : loadError ? (
          <div style={{ color: "var(--severity-critical)" }}>Failed to load wiki page: {loadError}</div>
        ) : editing ? (
          <textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            style={{
              width: "100%",
              height: "100%",
              minHeight: 300,
              fontFamily: "var(--font-mono, monospace)",
              fontSize: 13,
              background: "var(--surface-card)",
              color: "var(--text-primary)",
              border: "1px solid var(--border-subtle)",
              padding: 8,
              resize: "none",
            }}
          />
        ) : (
          <MarkdownView
            className="wiki-page-markdown"
            body={draftInitialized ? draft : body}
            onNavigateInternal={onNavigateInternalWikiPage}
            onOpenInNewTab={onOpenWikiPageInNewTab}
            onOpenFile={(path) => onOpenFile(path)}
            onOpenDirectory={onOpenDirectory}
            onOpenCommit={onOpenCommit}
            onOpenExternalUrl={onOpenExternalUrl}
            renderMermaid
          />
        )}
      </div>
      {!notFound && !loadError && summary && (summary.referenced_files?.length ?? 0) > 0 && (
        <BacklinksFooter
          summary={summary}
          onOpenFile={onOpenFile}
        />
      )}
    </div>
  );
}

function BacklinksFooter({
  summary,
  onOpenFile,
}: {
  summary: WikiPageSummary;
  onOpenFile: (path: string) => void;
}) {
  const ctxNav = useOptionalPageNavigation();
  const openFile = (path: string) => {
    if (ctxNav) ctxNav.navigate(fileRef(path), { newTab: false });
    else onOpenFile(path);
  };
  const changed = useMemo(() => new Set(summary.changed_refs ?? []), [summary.changed_refs]);
  const deleted = useMemo(() => new Set(summary.deleted_refs ?? []), [summary.deleted_refs]);
  const referencedFiles = summary.referenced_files ?? [];
  return (
    <footer
      style={{
        borderTop: "1px solid var(--border-subtle)",
        padding: "6px 10px",
        fontSize: 12,
        color: "var(--text-muted)",
        display: "flex",
        flexWrap: "wrap",
        gap: 6,
        alignItems: "center",
      }}
    >
      <span>
        Referenced file{referencedFiles.length === 1 ? "" : "s"} ({referencedFiles.length}):
      </span>
      {referencedFiles.map((path) => {
        const status = deleted.has(path) ? "deleted" : changed.has(path) ? "changed" : "fresh";
        const color =
          status === "deleted"
            ? "var(--severity-critical)"
            : status === "changed"
              ? "var(--status-waiting)"
              : "var(--text-primary)";
        return (
          <button
            key={path}
            type="button"
            onClick={() => {
              if (status === "deleted") return;
              openFile(path);
            }}
            disabled={status === "deleted"}
            title={
              status === "deleted"
                ? `${path} (deleted from workspace)`
                : status === "changed"
                  ? `${path} (changed since this wiki page was written)`
                  : `Open ${path}`
            }
            style={{
              fontFamily: "var(--font-mono, monospace)",
              fontSize: 11,
              padding: "1px 6px",
              borderRadius: 3,
              border: "1px solid var(--border-subtle)",
              background: "transparent",
              color,
              cursor: status === "deleted" ? "not-allowed" : "pointer",
              textDecoration: status === "deleted" ? "line-through" : "none",
            }}
          >
            {path}
          </button>
        );
      })}
    </footer>
  );
}

function FreshnessBadge({ note }: { note: WikiPageSummary }) {
  const reasons = useMemo(() => {
    const r: string[] = [];
    const changedCount = note.changed_refs?.length ?? 0;
    const deletedCount = note.deleted_refs?.length ?? 0;
    if (note.head_advanced) r.push("HEAD advanced");
    if (changedCount > 0) r.push(`${changedCount} ref${changedCount === 1 ? "" : "s"} changed`);
    if (deletedCount > 0) r.push(`${deletedCount} deleted`);
    return r;
  }, [note]);
  const totalRefs = note.total_refs ?? note.referenced_files?.length ?? 0;
  const title = reasons.length > 0 ? reasons.join("; ") : `${totalRefs} referenced files`;
  return (
    <span
      title={title}
      style={{
        fontSize: 11,
        padding: "2px 6px",
        borderRadius: 3,
        background: FRESHNESS_COLOR[note.freshness ?? "fresh"],
        color: "#fff",
      }}
    >
      {FRESHNESS_LABEL[note.freshness ?? "fresh"]}
    </span>
  );
}
