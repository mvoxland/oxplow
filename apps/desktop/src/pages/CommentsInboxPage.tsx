import { useEffect, useMemo, useState } from "react";

import { listCommentsForStream, subscribeCommentEvents } from "../api.js";
import type { Stream } from "../api.js";
import type { CommentThread } from "../tauri-bridge/generated/bindings.js";
import { CommentPopover } from "../components/Comments/CommentPopover.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import { fileRef, taskRef, wikiPageRef } from "../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";

/// Map a comment's target back to the page that owns it.
function targetRef(kind: string, id: string): TabRef | null {
  if (kind === "file") return fileRef(id);
  if (kind === "wiki") return wikiPageRef(id);
  if (kind === "task") {
    const n = Number(id);
    return Number.isFinite(n) ? taskRef(n) : null;
  }
  return null;
}

function targetLabel(kind: string, id: string): string {
  if (kind === "file") return id;
  if (kind === "wiki") return `wiki/${id}`;
  if (kind === "task") return `task #${id}`;
  return `${kind}:${id}`;
}

/// Global Comments inbox — every comment in the current stream, grouped
/// by the page it's anchored to. The holistic "look at them all" view:
/// clicking a comment row opens its full thread popover inline (read,
/// reply, set intent, resolve, delete) so the whole backlog can be
/// triaged from one place; clicking a group header jumps to the target
/// page where the anchored highlight lives. The agent reaches the same
/// data via the `list_comments` MCP tool.
export function CommentsInboxPage({
  stream,
  onOpenPage,
}: {
  stream: Stream | null;
  onOpenPage: (ref: TabRef) => void;
}) {
  usePageTitle("Comments");
  const ctxNav = useOptionalPageNavigation();
  const [threads, setThreads] = useState<CommentThread[]>([]);
  const [active, setActive] = useState<{ id: number; rect: DOMRect } | null>(null);
  const streamId = stream?.id ?? null;

  useEffect(() => {
    if (!streamId) {
      setThreads([]);
      return;
    }
    let active = true;
    const fetch = async () => {
      const list = await listCommentsForStream(streamId);
      if (active) setThreads(list);
    };
    void fetch();
    const unsub = subscribeCommentEvents(() => void fetch());
    return () => {
      active = false;
      unsub();
    };
  }, [streamId]);

  const groups = useMemo(() => {
    const byTarget = new Map<string, { kind: string; id: string; threads: CommentThread[] }>();
    for (const t of threads) {
      const key = `${t.comment.target_kind}:${t.comment.target_id}`;
      let g = byTarget.get(key);
      if (!g) {
        g = { kind: t.comment.target_kind, id: t.comment.target_id, threads: [] };
        byTarget.set(key, g);
      }
      g.threads.push(t);
    }
    return [...byTarget.values()];
  }, [threads]);

  const navigate = (kind: string, id: string) => {
    const ref = targetRef(kind, id);
    if (!ref) return;
    if (ctxNav) ctxNav.navigate(ref, { newTab: false });
    else onOpenPage(ref);
  };

  return (
    <Page title="Comments">
      <div style={{ padding: 24, display: "flex", flexDirection: "column", gap: 20 }} data-testid="page-comments">
        {groups.length === 0 ? (
          <div style={{ color: "var(--text-muted)", fontSize: "var(--text-sm)" }}>
            No comments yet. Select text in a wiki page, file, or task and add one.
          </div>
        ) : (
          groups.map((g) => (
            <section key={`${g.kind}:${g.id}`} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <button
                type="button"
                data-testid={`comments-group-${g.kind}-${g.id}`}
                onClick={(e) => {
                  if (e.metaKey || e.ctrlKey) onOpenPage(targetRef(g.kind, g.id) ?? { id: "", kind: "agent", payload: null });
                  else navigate(g.kind, g.id);
                }}
                style={{
                  textAlign: "left",
                  background: "transparent",
                  border: "none",
                  color: "var(--text-secondary)",
                  fontSize: "var(--text-xs)",
                  textTransform: "uppercase",
                  letterSpacing: "0.04em",
                  cursor: "pointer",
                  padding: "4px 0",
                }}
              >
                {targetLabel(g.kind, g.id)} · {g.threads.length}
              </button>
              {g.threads.map((t) => {
                const lastMsg = t.messages[t.messages.length - 1];
                return (
                  <button
                    key={t.comment.id}
                    type="button"
                    data-testid={`comments-row-${t.comment.id}`}
                    onClick={(e) =>
                      setActive({ id: t.comment.id, rect: e.currentTarget.getBoundingClientRect() })
                    }
                    style={{
                      textAlign: "left",
                      display: "flex",
                      flexDirection: "column",
                      gap: 4,
                      padding: "8px 12px",
                      background: "var(--surface-card)",
                      border: "1px solid var(--border-subtle)",
                      borderRadius: 6,
                      cursor: "pointer",
                    }}
                  >
                    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                      <span
                        style={{
                          fontSize: "var(--text-xs)",
                          padding: "1px 6px",
                          borderRadius: 4,
                          background: t.comment.intent === "followup" ? "var(--accent-soft-bg)" : "transparent",
                          color: t.comment.intent === "followup" ? "var(--accent)" : "var(--text-muted)",
                          border: "1px solid var(--border-subtle)",
                        }}
                      >
                        {t.comment.intent === "followup" ? "follow-up" : "note"}
                      </span>
                      {t.comment.status === "resolved" && (
                        <span style={{ fontSize: "var(--text-xs)", color: "var(--status-done)" }}>resolved</span>
                      )}
                      {t.comment.orphaned && (
                        <span style={{ fontSize: "var(--text-xs)", color: "var(--freshness-stale)" }}>orphaned</span>
                      )}
                      <span style={{ flex: 1 }} />
                      <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
                        {t.messages.length} {t.messages.length === 1 ? "message" : "messages"}
                      </span>
                    </div>
                    <div
                      style={{
                        fontSize: "var(--text-xs)",
                        color: "var(--text-secondary)",
                        fontStyle: "italic",
                        borderLeft: "2px solid var(--comment-highlight)",
                        paddingLeft: 8,
                      }}
                    >
                      “{t.comment.quote}”
                    </div>
                    {lastMsg && (
                      <div style={{ fontSize: "var(--text-sm)", color: "var(--text-primary)", whiteSpace: "pre-wrap" }}>
                        <span style={{ color: lastMsg.author === "agent" ? "var(--accent)" : "var(--text-secondary)", fontWeight: 600 }}>
                          {lastMsg.author}:{" "}
                        </span>
                        {lastMsg.body}
                      </div>
                    )}
                  </button>
                );
              })}
            </section>
          ))
        )}
      </div>
      {active &&
        (() => {
          const t = threads.find((x) => x.comment.id === active.id);
          return t ? (
            <CommentPopover thread={t} anchorRect={active.rect} onClose={() => setActive(null)} />
          ) : null;
        })()}
    </Page>
  );
}
