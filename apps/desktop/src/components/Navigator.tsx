import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Stream, Thread, ThreadState } from "../api.js";
import { AgentStatusDot, type AgentStatusDotState } from "./AgentStatusDot.js";
import { Kebab } from "./Kebab.js";
import type { MenuItem } from "../menu.js";

interface NavigatorProps {
  streams: Stream[];
  currentStreamId: string | null;
  threadStates: Record<string, ThreadState>;
  streamStatuses: Record<string, AgentStatusDotState>;
  agentStatuses: Record<string, AgentStatusDotState>;
  onSwitchStream(id: string): void | Promise<void>;
  onSelectThread(id: string): void | Promise<void>;
  onCreateThread(streamId: string, title: string): Promise<void>;
  onOpenNewStreamPage?(): void;
  onRenameStream?(streamId: string, title: string): void | Promise<void>;
  onRenameThread?(threadId: string, title: string): void | Promise<void>;
  onPromoteThread?(threadId: string): void | Promise<void>;
  onCloseThread?(threadId: string): void | Promise<void>;
  onOpenStreamSettings?(streamId: string): void;
  onOpenThreadSettings?(threadId: string): void;
  gitEnabled: boolean;
}

type RenameTarget = { kind: "stream" | "thread"; id: string };

/**
 * Combined stream + thread navigator.
 *
 * Layout: a thin always-visible vertical strip on the left holding a
 * letter glyph per stream and per thread. Clicking a glyph navigates
 * directly. Clicking the toggle (or the empty space below the last
 * glyph) slides an overlay panel out to the right that re-renders the
 * same rows with full titles — y-positions are identical between the
 * strip and the overlay so items don't move when switching modes.
 *
 * Visual hierarchy:
 *   - Stream rows: 16px letter, weight 600, with a subtle underline
 *     (border-bottom) so they read as section headers for the threads
 *     beneath them.
 *   - Thread rows: 16px letter, weight 400, slightly indented in the
 *     overlay; in the strip they share the same column.
 *   - The active stream's writer thread renders its letter inside an
 *     accent-soft-bg pill with an accent border. Activity dot is
 *     overlaid in the top-right corner of the icon cell — same
 *     position in strip and overlay.
 *   - Selection treatment: accent-soft-bg row background + 3px accent
 *     left stripe.
 *   - 8px gap separates the last thread of one stream from the next
 *     stream's row.
 */
export function Navigator({
  streams,
  currentStreamId,
  threadStates,
  streamStatuses,
  agentStatuses,
  onSwitchStream,
  onSelectThread,
  onCreateThread,
  onOpenNewStreamPage,
  onRenameStream,
  onRenameThread,
  onPromoteThread,
  onCloseThread,
  onOpenStreamSettings,
  onOpenThreadSettings,
  gitEnabled,
}: NavigatorProps) {
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [pendingNewThreadFor, setPendingNewThreadFor] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<RenameTarget | null>(null);
  const overlayRef = useRef<HTMLDivElement | null>(null);

  const orderedStreams = useMemo(() => {
    return streams.slice().sort((a, b) => {
      if (a.kind === "primary" && b.kind !== "primary") return -1;
      if (b.kind === "primary" && a.kind !== "primary") return 1;
      return 0;
    });
  }, [streams]);

  // Close overlay on Escape, or click outside.
  useEffect(() => {
    if (!overlayOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOverlayOpen(false);
    };
    const onClick = (e: MouseEvent) => {
      const root = overlayRef.current;
      if (!root) return;
      if (e.target instanceof Node && !root.contains(e.target)) {
        setOverlayOpen(false);
      }
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onClick);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onClick);
    };
  }, [overlayOpen]);

  const handleSwitchStream = (id: string) => {
    if (id !== currentStreamId) void onSwitchStream(id);
    // Streams are containers, not destinations — clicking a stream
    // toggles the overlay: pops it open from the closed strip so the
    // user can pick a thread, and closes it again when clicked from
    // the open overlay. Selecting a thread auto-closes regardless.
    setOverlayOpen((v) => !v);
  };
  const handleSelectThread = (streamId: string, threadId: string) => {
    if (streamId !== currentStreamId) void onSwitchStream(streamId);
    void onSelectThread(threadId);
    setOverlayOpen(false);
  };

  // Build the list of "rows" so the strip and overlay can both walk
  // the same sequence — guaranteeing matching y-positions row-by-row.
  // `add-thread` rows are flyout-only (skipped in the strip render).
  type Row =
    | { kind: "stream"; stream: Stream }
    | { kind: "thread"; stream: Stream; thread: Thread; isWriter: boolean }
    | { kind: "add-thread"; stream: Stream }
    | { kind: "gap"; afterStreamId: string };
  const rows: Row[] = useMemo(() => {
    const out: Row[] = [];
    for (const s of orderedStreams) {
      out.push({ kind: "stream", stream: s });
      const ts = threadStates[s.id];
      const writerId = ts?.activeThreadId ?? null;
      const threads = ts?.threads ?? [];
      for (const t of threads) {
        out.push({ kind: "thread", stream: s, thread: t, isWriter: t.id === writerId });
      }
      out.push({ kind: "add-thread", stream: s });
      out.push({ kind: "gap", afterStreamId: s.id });
    }
    return out;
  }, [orderedStreams, threadStates]);

  return (
    <div
      ref={overlayRef}
      style={{
        position: "relative",
        display: "flex",
        height: "100%",
        flexShrink: 0,
      }}
    >
      {/* Always-visible strip */}
      <aside
        data-testid="navigator-strip"
        style={{
          width: STRIP_WIDTH,
          flexShrink: 0,
          height: "100%",
          // Darker than the HUD rail so the gutter reads as a
          // separate, recessed control surface — not the same
          // background as the pane to its right.
          background: "var(--surface-app)",
          borderRight: "1px solid var(--border-strong)",
          boxShadow: "inset -1px 0 0 rgba(0, 0, 0, 0.35)",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          minHeight: 0,
        }}
      >
        <div
          onClick={() => {
            // Any click that bubbles up to the strip wrapper opens
            // the overlay. StripRow's onClick stops propagation, so
            // direct clicks on items navigate without opening — only
            // gap-rows and padding around items reach this handler.
            setOverlayOpen(true);
          }}
          style={{ flex: 1, overflowY: "auto", paddingTop: STRIP_PADDING_Y, cursor: "pointer" }}
        >
          {rows.map((row, idx) => {
            if (row.kind === "gap") {
              return <div key={`gap-${row.afterStreamId}-${idx}`} style={{ height: GAP_HEIGHT }} />;
            }
            if (row.kind === "add-thread") {
              // The slot only takes vertical space when the inline
              // title input is rendered in the overlay; keep the
              // strip in lock-step so subsequent glyphs stay aligned.
              if (pendingNewThreadFor !== row.stream.id) return null;
              return (
                <div
                  key={`strip-addthread-${row.stream.id}`}
                  style={{ height: ADD_ROW_HEIGHT }}
                />
              );
            }
            if (row.kind === "stream") {
              const isCurrent = row.stream.id === currentStreamId;
              return (
                <StripRow
                  key={`s-${row.stream.id}`}
                  letter={firstLetter(row.stream.title)}
                  isStream
                  isWriter={false}
                  selected={isCurrent}
                  status={undefined}
                  title={row.stream.title}
                  onClick={() => handleSwitchStream(row.stream.id)}
                />
              );
            }
            const isSelected =
              row.stream.id === currentStreamId &&
              threadStates[row.stream.id]?.selectedThreadId === row.thread.id;
            return (
              <StripRow
                key={`t-${row.thread.id}`}
                letter={firstLetter(row.thread.title)}
                isStream={false}
                isWriter={row.isWriter}
                selected={isSelected}
                status={agentStatuses[row.thread.id]}
                title={`${row.stream.title} · ${row.thread.title}${row.isWriter ? " (writer)" : ""}`}
                onClick={() => handleSelectThread(row.stream.id, row.thread.id)}
              />
            );
          })}
        </div>
        <div style={{ borderTop: "1px solid var(--border-subtle)", padding: 4, display: "flex", flexDirection: "column", gap: 4, alignItems: "center" }}>
          <button
            type="button"
            data-testid="navigator-toggle-overlay"
            onClick={() => setOverlayOpen((v) => !v)}
            title={overlayOpen ? "Close navigator" : "Open navigator"}
            style={stripIconButton}
          >
            {overlayOpen ? "«" : "»"}
          </button>
        </div>
      </aside>

      {/* Overlay panel — anchored at left:0 so it covers the strip
          rather than sitting beside it. The icon column inside each
          overlay row is sized identically to the strip's width, so the
          glyphs render in the same x-position before and after open. */}
      {overlayOpen ? (
        <div
          data-testid="navigator-overlay"
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            height: "100%",
            width: STRIP_WIDTH + OVERLAY_WIDTH,
            background: "var(--surface-rail)",
            borderRight: "1px solid var(--border-strong)",
            boxShadow: "8px 0 24px rgba(0, 0, 0, 0.45)",
            display: "flex",
            flexDirection: "column",
            zIndex: 30,
          }}
        >
          <div style={{ flex: 1, overflowY: "auto", paddingTop: STRIP_PADDING_Y }}>
            {rows.map((row, idx) => {
              if (row.kind === "gap") {
                return <div key={`o-gap-${row.afterStreamId}-${idx}`} style={{ height: GAP_HEIGHT }} />;
              }
              if (row.kind === "stream") {
                const isCurrent = row.stream.id === currentStreamId;
                const streamMenu: MenuItem[] = [
                  {
                    id: "stream.add-thread",
                    label: "Add thread",
                    enabled: true,
                    run: () => setPendingNewThreadFor(row.stream.id),
                  },
                  {
                    id: "stream.rename",
                    label: "Rename…",
                    enabled: !!onRenameStream,
                    run: () => setRenaming({ kind: "stream", id: row.stream.id }),
                  },
                  {
                    id: "stream.settings",
                    label: "Settings…",
                    enabled: !!onOpenStreamSettings,
                    run: () => onOpenStreamSettings?.(row.stream.id),
                  },
                ];
                return (
                  <OverlayRow
                    key={`o-s-${row.stream.id}`}
                    letter={firstLetter(row.stream.title)}
                    label={row.stream.title}
                    isStream
                    isWriter={false}
                    selected={isCurrent}
                    status={undefined}
                    onClick={() => handleSwitchStream(row.stream.id)}
                    renaming={renaming?.kind === "stream" && renaming.id === row.stream.id}
                    onCommitRename={async (next) => {
                      setRenaming(null);
                      if (next && next !== row.stream.title) {
                        await onRenameStream?.(row.stream.id, next);
                      }
                    }}
                    onCancelRename={() => setRenaming(null)}
                    menu={streamMenu}
                    menuTestId={`navigator-stream-kebab-${row.stream.id}`}
                  />
                );
              }
              if (row.kind === "add-thread") {
                // Only show the inline title input when the user has
                // explicitly chosen "Add thread" from the stream's
                // kebab. Otherwise the slot is empty in the overlay
                // (and the strip).
                if (pendingNewThreadFor !== row.stream.id) return null;
                return (
                  <InlineNewThread
                    key={`o-addinput-${row.stream.id}`}
                    onSubmit={async (title) => {
                      await onCreateThread(row.stream.id, title);
                      setPendingNewThreadFor(null);
                    }}
                    onCancel={() => setPendingNewThreadFor(null)}
                  />
                );
              }
              const isSelected =
                row.stream.id === currentStreamId &&
                threadStates[row.stream.id]?.selectedThreadId === row.thread.id;
              const threadMenu: MenuItem[] = [
                {
                  id: "thread.rename",
                  label: "Rename…",
                  enabled: !!onRenameThread,
                  run: () => setRenaming({ kind: "thread", id: row.thread.id }),
                },
                {
                  id: "thread.promote",
                  label: row.isWriter ? "Already the writer" : "Make writer",
                  enabled: !!onPromoteThread && !row.isWriter,
                  run: () => onPromoteThread?.(row.thread.id),
                },
                {
                  id: "thread.settings",
                  label: "Settings…",
                  enabled: !!onOpenThreadSettings,
                  run: () => onOpenThreadSettings?.(row.thread.id),
                },
                {
                  id: "thread.close",
                  label: "Close thread",
                  enabled: !!onCloseThread,
                  run: () => onCloseThread?.(row.thread.id),
                },
              ];
              return (
                <OverlayRow
                  key={`o-t-${row.thread.id}`}
                  letter={firstLetter(row.thread.title)}
                  label={row.thread.title}
                  isStream={false}
                  isWriter={row.isWriter}
                  selected={isSelected}
                  status={agentStatuses[row.thread.id]}
                  onClick={() => handleSelectThread(row.stream.id, row.thread.id)}
                  renaming={renaming?.kind === "thread" && renaming.id === row.thread.id}
                  onCommitRename={async (next) => {
                    setRenaming(null);
                    if (next && next !== row.thread.title) {
                      await onRenameThread?.(row.thread.id, next);
                    }
                  }}
                  onCancelRename={() => setRenaming(null)}
                  menu={threadMenu}
                  menuTestId={`navigator-thread-kebab-${row.thread.id}`}
                />
              );
            })}
            <AddStreamButton
              gitEnabled={gitEnabled && !!onOpenNewStreamPage}
              onClick={() => onOpenNewStreamPage?.()}
            />
          </div>
          {/* Footer mirrors the strip's footer position so the close
              button stays where the open button was. */}
          <div
            style={{
              borderTop: "1px solid var(--border-subtle)",
              padding: 4,
              display: "flex",
              alignItems: "center",
            }}
          >
            <div style={{ width: STRIP_WIDTH - 8, display: "flex", justifyContent: "center" }}>
              <button
                type="button"
                onClick={() => setOverlayOpen(false)}
                title="Close navigator"
                style={stripIconButton}
              >
                «
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

/** Single row inside the strip — letter + status, fixed height. */
function StripRow({
  letter,
  isStream,
  isWriter,
  selected,
  status,
  title,
  onClick,
}: {
  letter: string;
  isStream: boolean;
  isWriter: boolean;
  selected: boolean;
  status: AgentStatusDotState | undefined;
  title: string;
  onClick(): void;
}) {
  const rowRef = useRef<HTMLDivElement | null>(null);
  const [tipPos, setTipPos] = useState<{ left: number; top: number } | null>(null);

  function showTip() {
    const rect = rowRef.current?.getBoundingClientRect();
    if (!rect) return;
    setTipPos({ left: rect.right + 6, top: rect.top + rect.height / 2 });
  }
  function hideTip() {
    setTipPos(null);
  }

  return (
    <>
      <div
        ref={rowRef}
        role="button"
        tabIndex={0}
        onClick={(e) => {
          // Stop bubbling so the strip wrapper's onClick (which opens
          // the overlay) doesn't also fire when the user clicked a row
          // directly.
          e.stopPropagation();
          hideTip();
          onClick();
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            e.stopPropagation();
            onClick();
          }
        }}
        onMouseEnter={showTip}
        onMouseLeave={hideTip}
        onFocus={showTip}
        onBlur={hideTip}
        style={{
          height: ROW_HEIGHT,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          cursor: "pointer",
          background: selected ? "var(--accent-soft-bg)" : "transparent",
          borderLeft: selected ? "3px solid var(--accent)" : "3px solid transparent",
          borderBottom: isStream ? "1px solid var(--border-subtle)" : "1px solid transparent",
          position: "relative",
          transition: "background 120ms ease",
        }}
      >
        <IconCell letter={letter} isStream={isStream} isWriter={isWriter} status={status} />
      </div>
      {/* Hover label — fixed-positioned so it can float outside the
          strip's `overflow: hidden` clip. Anchored to the row's
          live bounding rect (recomputed on each enter). */}
      {tipPos ? (
        <span
          role="tooltip"
          style={{
            position: "fixed",
            left: tipPos.left,
            top: tipPos.top,
            transform: "translateY(-50%)",
            background: "var(--surface-elevated)",
            color: "var(--text-primary)",
            border: "1px solid var(--border-strong)",
            borderRadius: 4,
            padding: "4px 8px",
            fontSize: 12,
            fontWeight: isStream ? 600 : 400,
            whiteSpace: "nowrap",
            boxShadow: "2px 2px 8px rgba(0, 0, 0, 0.4)",
            pointerEvents: "none",
            zIndex: 25,
          }}
        >
          {title}
        </span>
      ) : null}
    </>
  );
}

/** Row inside the slide-over overlay — same row height + letter cell
 *  as the strip, plus the full title to the right. */
function OverlayRow({
  letter,
  label,
  isStream,
  isWriter,
  selected,
  status,
  onClick,
  renaming = false,
  onCommitRename,
  onCancelRename,
  menu,
  menuTestId,
}: {
  letter: string;
  label: string;
  isStream: boolean;
  isWriter: boolean;
  selected: boolean;
  status: AgentStatusDotState | undefined;
  onClick(): void;
  renaming?: boolean;
  onCommitRename?(next: string): void | Promise<void>;
  onCancelRename?(): void;
  menu?: MenuItem[];
  menuTestId?: string;
}) {
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={(e) => {
        if (renaming) return;
        // Suppress row navigation when the click originated inside the
        // kebab — the menu manages its own click flow.
        if ((e.target as HTMLElement).closest("[data-navigator-row-kebab]")) return;
        onClick();
      }}
      onKeyDown={(e) => {
        if (renaming) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
      title={label}
      style={{
        height: ROW_HEIGHT,
        display: "flex",
        alignItems: "center",
        gap: 8,
        cursor: renaming ? "default" : "pointer",
        background: selected ? "var(--accent-soft-bg)" : "transparent",
        borderLeft: selected ? "3px solid var(--accent)" : "3px solid transparent",
        borderBottom: isStream ? "1px solid var(--border-subtle)" : "1px solid transparent",
        paddingRight: 6,
        transition: "background 120ms ease",
      }}
    >
      <div style={{ width: STRIP_WIDTH - 3 /* keep the icon column the same width as the strip */, display: "flex", justifyContent: "center" }}>
        <IconCell letter={letter} isStream={isStream} isWriter={isWriter} status={status} />
      </div>
      {renaming ? (
        <RenameInput
          initial={label}
          paddingLeft={isStream ? 0 : 8}
          onCommit={(next) => onCommitRename?.(next)}
          onCancel={() => onCancelRename?.()}
        />
      ) : (
        <span
          style={{
            flex: 1,
            fontSize: 13,
            fontWeight: isStream ? 600 : 400,
            color: selected ? "var(--text-primary)" : "var(--text-secondary)",
            paddingLeft: isStream ? 0 : 8,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {label}
        </span>
      )}
      {menu && !renaming ? (
        <span data-navigator-row-kebab style={{ flexShrink: 0 }}>
          <Kebab items={menu} testId={menuTestId} size={14} />
        </span>
      ) : null}
    </div>
  );
}

function RenameInput({
  initial,
  paddingLeft,
  onCommit,
  onCancel,
}: {
  initial: string;
  paddingLeft: number;
  onCommit(next: string): void | Promise<void>;
  onCancel(): void;
}) {
  const [value, setValue] = useState(initial);
  return (
    <input
      autoFocus
      value={value}
      onClick={(e) => e.stopPropagation()}
      onChange={(e) => setValue(e.target.value)}
      onBlur={() => {
        const next = value.trim();
        if (!next || next === initial) onCancel();
        else void onCommit(next);
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          const next = value.trim();
          if (!next || next === initial) onCancel();
          else void onCommit(next);
        } else if (e.key === "Escape") {
          e.preventDefault();
          onCancel();
        }
      }}
      style={{
        flex: 1,
        background: "var(--surface-card)",
        color: "var(--text-primary)",
        border: "1px solid var(--accent)",
        borderRadius: 4,
        padding: "3px 6px",
        fontSize: 13,
        marginLeft: paddingLeft,
      }}
    />
  );
}

/** The shared icon cell that appears in both strip and overlay. The
 *  letter is centered in a 22px box; for writer threads the box is
 *  filled with `--accent-soft-bg` and bordered with `--accent`. The
 *  activity dot is absolutely positioned in the top-right corner so
 *  its location is identical regardless of writer/non-writer. */
function IconCell({
  letter,
  isStream,
  isWriter,
  status,
}: {
  letter: string;
  isStream: boolean;
  isWriter: boolean;
  status: AgentStatusDotState | undefined;
}) {
  // Writer pill: a soft, dim accent wash + translucent ring instead
  // of the full --accent-soft-bg + --accent treatment, so "writer"
  // still reads as a deliberate state without dominating the icon.
  const writerStyles: CSSProperties = isWriter
    ? {
        background: "rgba(107, 156, 246, 0.08)",
        border: "1px solid rgba(107, 156, 246, 0.35)",
      }
    : {
        background: "transparent",
        border: "1px solid transparent",
      };
  return (
    <span
      style={{
        position: "relative",
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: ICON_BOX,
        height: ICON_BOX,
        borderRadius: 6,
        fontSize: LETTER_FONT,
        lineHeight: 1,
        fontWeight: isStream ? 700 : 600,
        color: "var(--text-primary)",
        ...writerStyles,
      }}
    >
      {letter}
      {/* Threads always show the agent's activity indicator. Mirrors
          the fallback the Agent tab uses (App.tsx — `agentStatuses[id]
          ?? "waiting"`) so a never-attached thread still reads as the
          same red "waiting" dot here as it does on the tab. Stream
          rows omit the dot entirely. */}
      {!isStream ? (
        <span
          style={{
            position: "absolute",
            top: -2,
            right: -2,
          }}
        >
          <AgentStatusDot status={status ?? "waiting"} size={8} />
        </span>
      ) : null}
    </span>
  );
}

function AddStreamButton({ gitEnabled, onClick }: { gitEnabled: boolean; onClick(): void }) {
  return (
    <div
      style={{
        padding: "12px 12px 8px",
        marginTop: GAP_HEIGHT,
        borderTop: "1px solid var(--border-subtle)",
      }}
    >
      <button
        type="button"
        data-testid="navigator-new-stream"
        onClick={() => { if (gitEnabled) onClick(); }}
        disabled={!gitEnabled}
        title={gitEnabled ? "Create a new stream" : "Disabled: workspace root is not its own git repo"}
        style={{
          width: "100%",
          textAlign: "center",
          background: "var(--surface-card)",
          color: "var(--text-primary)",
          border: "1px solid var(--border-strong)",
          borderRadius: 6,
          padding: "6px 10px",
          cursor: gitEnabled ? "pointer" : "not-allowed",
          fontFamily: "inherit",
          fontSize: 12,
          fontWeight: 500,
          letterSpacing: 0.2,
          opacity: gitEnabled ? 1 : 0.5,
        }}
      >
        + Add stream
      </button>
    </div>
  );
}

function InlineNewThread({
  onSubmit,
  onCancel,
}: {
  onSubmit(title: string): Promise<void>;
  onCancel(): void;
}) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <form
      onSubmit={async (e) => {
        e.preventDefault();
        const t = value.trim();
        if (!t) return onCancel();
        setBusy(true);
        try {
          await onSubmit(t);
        } finally {
          setBusy(false);
        }
      }}
      style={{
        height: ADD_ROW_HEIGHT,
        padding: "4px 12px",
        display: "flex",
        alignItems: "center",
      }}
    >
      <input
        autoFocus
        value={value}
        disabled={busy}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => {
          const t = value.trim();
          if (!t) onCancel();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
        placeholder="New thread title"
        style={{
          width: "100%",
          background: "var(--surface-card)",
          color: "var(--text-primary)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 4,
          padding: "4px 6px",
          fontSize: 12,
        }}
      />
    </form>
  );
}

function firstLetter(s: string): string {
  const trimmed = s.trim();
  if (!trimmed) return "?";
  return trimmed.charAt(0).toUpperCase();
}

// Glyph sizing. The letter is bumped above body text (20px vs 14px)
// so the strip reads as a real navigation indicator rather than a
// tooltip-targeted dot. Strip width and row height grow proportionally
// so the icon cell sits comfortably with breathing room on both sides.
const LETTER_FONT = 20;
const ICON_BOX = 30;
const STRIP_WIDTH = 40;
const STRIP_PADDING_Y = 6;
const ROW_HEIGHT = 36;
const GAP_HEIGHT = 10;
// Height reserved (strip) and matched (overlay) for the per-stream
// "+ New thread" row so subsequent items stay y-aligned across the
// two views. Must match the rendered AddThreadRow height.
const ADD_ROW_HEIGHT = 40;
const OVERLAY_WIDTH = 240;

const stripIconButton: CSSProperties = {
  width: 24,
  height: 24,
  borderRadius: 6,
  border: "1px solid var(--border-subtle)",
  background: "transparent",
  color: "var(--text-secondary)",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
};
