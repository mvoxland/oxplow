import { useEffect, useRef, useState } from "react";
import type { Stream, Thread } from "../../api.js";
import type { TasksScope } from "./tasks-scope.js";

export interface TasksScopePickerProps {
  scope: TasksScope;
  onChange(next: TasksScope): void;
  streams: Stream[];
  /** Threads grouped by stream id. Streams with no entry render no thread
   *  options yet — the page lazy-loads on demand. */
  threadsByStream: Record<string, Thread[]>;
  onRequestThreads(streamId: string): void;
}

const buttonStyle: React.CSSProperties = {
  background: "transparent",
  color: "var(--text)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "2px 8px",
  fontSize: 12,
  cursor: "pointer",
  display: "inline-flex",
  alignItems: "center",
  gap: 4,
};

const menuStyle: React.CSSProperties = {
  position: "absolute",
  top: "100%",
  left: 0,
  marginTop: 2,
  background: "var(--bg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  fontSize: 12,
  zIndex: 50,
  minWidth: 200,
  padding: "4px 0",
  boxShadow: "0 4px 12px rgba(0,0,0,0.25)",
};

const submenuStyle: React.CSSProperties = {
  ...menuStyle,
  position: "absolute",
  top: 0,
  left: "100%",
  marginTop: -4,
  marginLeft: 0,
};

const itemStyle: React.CSSProperties = {
  padding: "4px 10px",
  cursor: "pointer",
  whiteSpace: "nowrap",
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: 12,
};

const itemHoverBg = "var(--hover, rgba(255,255,255,0.06))";

const dividerStyle: React.CSSProperties = {
  height: 1,
  background: "var(--border)",
  margin: "4px 0",
};

const groupHeaderStyle: React.CSSProperties = {
  padding: "4px 10px",
  fontSize: 10,
  textTransform: "uppercase",
  letterSpacing: 0.5,
  color: "var(--muted)",
};

/**
 * Hierarchical scope menu shown above the Tasks list. One button toggles
 * the menu; the menu surfaces a single drill-through structure so the
 * user picks a scope in one path instead of juggling three side-by-side
 * dropdowns:
 *
 *   • Current thread
 *   • Everything (all streams)
 *   ─────────
 *   • Stream X ▶
 *       • All threads in stream X (merged)
 *       • Thread A
 *       • Thread B
 *   • Stream Y ▶ …
 *
 * The submenu opens on hover (and click on touch). Threads are
 * lazy-loaded via onRequestThreads when a stream's submenu first opens.
 */
export function TasksScopePicker({
  scope,
  onChange,
  streams,
  threadsByStream,
  onRequestThreads,
}: TasksScopePickerProps) {
  const [open, setOpen] = useState(false);
  const [hoverStream, setHoverStream] = useState<string | null>(null);
  const [hoverItem, setHoverItem] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    function onDocMouseDown(ev: MouseEvent) {
      if (!rootRef.current) return;
      if (rootRef.current.contains(ev.target as Node)) return;
      setOpen(false);
      setHoverStream(null);
    }
    function onKey(ev: KeyboardEvent) {
      if (ev.key === "Escape") {
        setOpen(false);
        setHoverStream(null);
      }
    }
    window.addEventListener("mousedown", onDocMouseDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDocMouseDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  useEffect(() => {
    if (!open || !hoverStream) return;
    if (!threadsByStream[hoverStream]) onRequestThreads(hoverStream);
  }, [open, hoverStream, threadsByStream, onRequestThreads]);

  function pick(next: TasksScope) {
    onChange(next);
    setOpen(false);
    setHoverStream(null);
  }

  const label = describeScope(scope, streams, threadsByStream);

  function rowStyle(key: string): React.CSSProperties {
    return {
      ...itemStyle,
      background: hoverItem === key ? itemHoverBg : "transparent",
    };
  }

  return (
    <div
      ref={rootRef}
      data-testid="tasks-scope-picker"
      style={{
        position: "relative",
        padding: "6px 10px",
        borderBottom: "1px solid var(--border)",
        fontSize: 12,
        display: "flex",
        alignItems: "center",
        gap: 8,
      }}
    >
      <span style={{ color: "var(--muted)" }}>Show:</span>
      <button
        type="button"
        data-testid="tasks-scope-button"
        style={buttonStyle}
        onClick={() => setOpen((v) => !v)}
      >
        <span>{label}</span>
        <span style={{ color: "var(--muted)" }}>▾</span>
      </button>

      {open ? (
        <div data-testid="tasks-scope-menu" style={menuStyle}>
          <div
            data-testid="tasks-scope-current"
            style={rowStyle("current")}
            onMouseEnter={() => { setHoverItem("current"); setHoverStream(null); }}
            onMouseLeave={() => setHoverItem(null)}
            onClick={() => pick({ kind: "currentThread" })}
          >
            Current thread
          </div>
          <div
            data-testid="tasks-scope-all"
            style={rowStyle("all")}
            onMouseEnter={() => { setHoverItem("all"); setHoverStream(null); }}
            onMouseLeave={() => setHoverItem(null)}
            onClick={() => pick({ kind: "all" })}
          >
            Everything (all streams)
          </div>
          {streams.length > 0 ? <div style={dividerStyle} /> : null}
          {streams.length > 0 ? <div style={groupHeaderStyle}>Streams</div> : null}
          {streams.map((s) => {
            const isHover = hoverStream === s.id;
            const threads = threadsByStream[s.id];
            return (
              <div
                key={s.id}
                data-testid={`tasks-scope-stream-${s.id}`}
                style={{
                  ...rowStyle(`stream-${s.id}`),
                  position: "relative",
                  background: isHover ? itemHoverBg : (hoverItem === `stream-${s.id}` ? itemHoverBg : "transparent"),
                }}
                onMouseEnter={() => { setHoverItem(`stream-${s.id}`); setHoverStream(s.id); }}
              >
                <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{s.title}</span>
                <span style={{ color: "var(--muted)" }}>▸</span>
                {isHover ? (
                  <div style={submenuStyle} onMouseLeave={() => setHoverStream(null)}>
                    <div
                      style={rowStyle(`stream-merged-${s.id}`)}
                      onMouseEnter={() => setHoverItem(`stream-merged-${s.id}`)}
                      onMouseLeave={() => setHoverItem(null)}
                      onClick={(e) => { e.stopPropagation(); pick({ kind: "stream", streamId: s.id }); }}
                    >
                      All threads (merged)
                    </div>
                    <div style={dividerStyle} />
                    {threads === undefined ? (
                      <div style={{ ...itemStyle, color: "var(--muted)", cursor: "default" }}>Loading…</div>
                    ) : threads.length === 0 ? (
                      <div style={{ ...itemStyle, color: "var(--muted)", cursor: "default" }}>(no threads)</div>
                    ) : (
                      threads.map((t) => (
                        <div
                          key={t.id}
                          data-testid={`tasks-scope-thread-${t.id}`}
                          style={rowStyle(`thread-${t.id}`)}
                          onMouseEnter={() => setHoverItem(`thread-${t.id}`)}
                          onMouseLeave={() => setHoverItem(null)}
                          onClick={(e) => {
                            e.stopPropagation();
                            pick({ kind: "thread", streamId: s.id, threadId: t.id });
                          }}
                        >
                          {t.title || t.id.slice(0, 8)}
                        </div>
                      ))
                    )}
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

function describeScope(
  scope: TasksScope,
  streams: Stream[],
  threadsByStream: Record<string, Thread[]>,
): string {
  if (scope.kind === "currentThread") return "Current thread";
  if (scope.kind === "all") return "Everything (all streams)";
  const stream = streams.find((s) => s.id === scope.streamId);
  const streamLabel = stream?.title ?? "(unknown stream)";
  if (scope.kind === "stream") return `${streamLabel} — all threads`;
  const thread = threadsByStream[scope.streamId]?.find((t) => t.id === scope.threadId);
  const threadLabel = thread?.title || scope.threadId.slice(0, 8);
  return `${streamLabel} › ${threadLabel}`;
}
