import { useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import type { EffortDetail, Task, TaskPriority, TaskStatus } from "../../api.js";
import { MarkdownView } from "../Wiki/MarkdownView.js";
import { RichTextField } from "../RichText/RichTextField.js";
import { inputStyle, miniButtonStyle } from "./plan-utils.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef } from "../../tabs/pageRefs.js";

/**
 * One entry in the tasks Activity timeline. Each effort
 * (in_progress → done/blocked/canceled cycle) carries a free-form
 * `summary` field that the runtime fills in via `complete_task`, so
 * efforts double as the "what happened on this item" log. Per-item
 * notes were retired — they recorded the same thing.
 */
export type ActivityRow =
  { kind: "effort"; id: string; timestamp: string; active: boolean; detail: EffortDetail };

export function buildActivityTimeline(efforts: EffortDetail[]): ActivityRow[] {
  const rows: ActivityRow[] = [];
  for (const detail of efforts) {
    const active = !detail.effort.ended_at;
    const timestamp = detail.effort.ended_at ?? detail.effort.started_at;
    rows.push({ kind: "effort", id: detail.effort.id, timestamp, active, detail });
  }
  rows.sort((a, b) => (a.timestamp < b.timestamp ? 1 : a.timestamp > b.timestamp ? -1 : 0));
  return rows;
}

export interface TaskDetailChanges {
  title?: string;
  description?: string;
  parentId?: number | null;
  status?: TaskStatus;
  priority?: TaskPriority;
  /** Backlog grooming bucket. Pass `null` to clear, omit to keep. */
  category?: string | null;
  /** Backlog grooming tags (comma-separated; normalized server-side). */
  tags?: string | null;
}

const STATUS_OPTIONS_BASE: TaskStatus[] = [
  "blocked", "ready", "done", "archived", "canceled",
];
const PRIORITY_OPTIONS: TaskPriority[] = ["low", "medium", "high", "urgent"];

function statusOptionsFor(current: TaskStatus): TaskStatus[] {
  return current === "in_progress" ? [...STATUS_OPTIONS_BASE, "in_progress"] : STATUS_OPTIONS_BASE;
}

/**
 * Body half of the tasks detail view — title + description. Status /
 * priority / category / tags / timestamps / destructive actions live
 * in `TaskDetailRail`. Acceptance criteria are no longer a separate
 * field; agents and users are nudged to include a "## Acceptance
 * criteria" subsection inside the description when it would be
 * helpful (the description is rendered as markdown anyway).
 */
export function TaskDetail({
  item,
  onUpdateTask,
}: {
  item: Task;
  onUpdateTask: (itemId: number, changes: TaskDetailChanges) => Promise<void>;
}) {
  return (
    <div
      className="task-detail-body"
      style={{ display: "flex", flexDirection: "column", gap: 18 }}
      onClick={(event) => event.stopPropagation()}
    >
      <TitleField
        key={`title-${item.id}`}
        value={item.title}
        onCommit={(value) => {
          const trimmed = value.trim();
          if (!trimmed || trimmed === item.title) return;
          void onUpdateTask(item.id, { title: trimmed });
        }}
      />
      <RichTextField
        key={`desc-${item.id}`}
        value={item.description}
        placeholder="Add a description… include a ## Acceptance criteria section if helpful."
        onCommit={(value) => {
          if (value === item.description) return;
          void onUpdateTask(item.id, { description: value });
        }}
      />
    </div>
  );
}

/**
 * Big inline-editable title for the task page header. Plain auto-sizing
 * textarea styled as an H1; Enter blurs/commits, Escape reverts.
 *
 * Tiptap would be overkill for a single-line text field — and rich
 * formatting on a title is a UX anti-pattern (titles in lists need to
 * be plain strings).
 */
function TitleField({
  value,
  onCommit,
}: {
  value: string;
  onCommit(value: string): void;
}) {
  const [draft, setDraft] = useState(value);
  const cancelRequested = useRef(false);
  useEffect(() => { setDraft(value); }, [value]);
  return (
    <textarea
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      placeholder="Untitled"
      rows={1}
      onBlur={() => {
        if (cancelRequested.current) {
          cancelRequested.current = false;
          setDraft(value);
          return;
        }
        if (draft.trim() && draft !== value) onCommit(draft);
        else setDraft(value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          cancelRequested.current = true;
          (e.target as HTMLTextAreaElement).blur();
        } else if (e.key === "Enter") {
          e.preventDefault();
          (e.target as HTMLTextAreaElement).blur();
        }
      }}
      className="task-title-field"
      style={{
        width: "100%",
        background: "transparent",
        border: "none",
        outline: "none",
        resize: "none",
        color: "var(--text-primary)",
        fontFamily: "var(--font-ui)",
        fontSize: "var(--text-2xl)",
        fontWeight: "var(--weight-bold)",
        lineHeight: "var(--leading-tight)",
        padding: "4px 0",
        overflow: "hidden",
      }}
      ref={(el) => {
        if (!el) return;
        // Auto-grow textarea to fit content (single visual line per
        // wrap). Run on next frame so the value has rendered.
        el.style.height = "auto";
        el.style.height = `${el.scrollHeight}px`;
      }}
    />
  );
}

/**
 * Rail half of the tasks detail view — status + priority pickers,
 * category, tags, timestamps, created-by, and an overflow menu for
 * destructive / scope actions. Mirror image of `TaskDetail` body fields.
 */
export function TaskDetailRail({
  item,
  onUpdateTask,
  onRequestDelete,
  extraMenuItems,
  formatTimestamp = (iso) => new Date(iso).toLocaleString(),
}: {
  item: Task;
  onUpdateTask: (itemId: number, changes: TaskDetailChanges) => Promise<void>;
  onRequestDelete(): void;
  /** Additional items rendered above Delete in the overflow menu —
   *  e.g. "Send to backlog" / "Bring to this thread". */
  extraMenuItems?: Array<{ label: string; onSelect(): void }>;
  formatTimestamp?(iso: string): string;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  return (
    <div
      style={{ display: "flex", flexDirection: "column", gap: 14, fontSize: "var(--text-xs)" }}
      onClick={(event) => event.stopPropagation()}
    >
      <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: -8, position: "relative" }}>
        <button
          type="button"
          onClick={() => setMenuOpen((v) => !v)}
          aria-label="More actions"
          style={{
            background: "transparent",
            border: "none",
            color: "var(--text-secondary)",
            cursor: "pointer",
            padding: "2px 6px",
            fontSize: 16,
            lineHeight: 1,
          }}
          title="More actions"
        >
          ⋯
        </button>
        {menuOpen ? (
          <div
            role="menu"
            onMouseLeave={() => setMenuOpen(false)}
            style={{
              position: "absolute",
              top: 24,
              right: 0,
              background: "var(--surface-elevated, var(--surface-card))",
              border: "1px solid var(--border-subtle)",
              borderRadius: 4,
              padding: 4,
              display: "flex",
              flexDirection: "column",
              minWidth: 160,
              boxShadow: "0 4px 12px rgba(0, 0, 0, 0.25)",
              zIndex: 10,
            }}
          >
            {extraMenuItems?.map((entry) => (
              <button
                key={entry.label}
                type="button"
                onClick={() => { setMenuOpen(false); entry.onSelect(); }}
                style={menuItemStyle}
              >
                {entry.label}
              </button>
            ))}
            <button
              type="button"
              onClick={() => { setMenuOpen(false); onRequestDelete(); }}
              style={{ ...menuItemStyle, color: "var(--severity-critical)" }}
            >
              Delete
            </button>
          </div>
        ) : null}
      </div>

      <RailPillRow label="Status">
        <PillSelect
          value={item.status}
          options={statusOptionsFor(item.status)}
          color={statusColor(item.status)}
          onChange={(value) => void onUpdateTask(item.id, { status: value as TaskStatus })}
        />
      </RailPillRow>
      <RailPillRow label="Priority">
        <PillSelect
          value={item.priority}
          options={PRIORITY_OPTIONS}
          color={priorityColor(item.priority)}
          onChange={(value) => void onUpdateTask(item.id, { priority: value as TaskPriority })}
        />
      </RailPillRow>
      <RailPillRow label="Category">
        <InlineChipText
          value={item.category ?? ""}
          placeholder="+ Add category"
          onCommit={(value) => {
            const trimmed = value.trim();
            const next = trimmed.length === 0 ? null : trimmed;
            if (next === (item.category ?? null)) return;
            void onUpdateTask(item.id, { category: next });
          }}
        />
      </RailPillRow>
      <RailPillRow label="Tags">
        <TagChipList
          value={item.tags ?? ""}
          onCommit={(value) => {
            const trimmed = value.trim();
            const next = trimmed.length === 0 ? null : trimmed;
            if (next === (item.tags ?? null)) return;
            void onUpdateTask(item.id, { tags: next });
          }}
        />
      </RailPillRow>

      <div style={{
        display: "flex",
        flexDirection: "column",
        gap: 3,
        borderTop: "1px solid var(--border-subtle)",
        paddingTop: 12,
        marginTop: 4,
        color: "var(--text-muted)",
        fontSize: "var(--text-xs)",
      }}>
        <RailMetaRow label="Created">{formatTimestamp(item.created_at)}</RailMetaRow>
        <RailMetaRow label="Updated">{formatTimestamp(item.updated_at)}</RailMetaRow>
        <RailMetaRow label="By">{item.created_by}</RailMetaRow>
      </div>
    </div>
  );
}

function statusColor(status: TaskStatus): string {
  switch (status) {
    case "in_progress": return "var(--status-running)";
    case "ready": return "var(--status-ready)";
    case "done": return "var(--status-done)";
    case "blocked": return "var(--status-waiting)";
    case "canceled":
    case "archived":
      return "var(--status-canceled)";
    default:
      return "var(--status-ready)";
  }
}

function priorityColor(priority: TaskPriority): string {
  switch (priority) {
    case "urgent": return "var(--priority-urgent)";
    case "high":   return "var(--priority-high)";
    case "medium": return "var(--priority-medium)";
    case "low":    return "var(--priority-low)";
    default:       return "var(--priority-medium)";
  }
}

function RailPillRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{
        textTransform: "uppercase",
        letterSpacing: 0.5,
        fontSize: 10,
        color: "var(--text-secondary)",
        fontWeight: 500,
      }}>{label}</div>
      <div>{children}</div>
    </div>
  );
}

/**
 * Colored pill that opens a native `<select>` on click. The native
 * select stays transparent over the pill so keyboard navigation +
 * accessibility come for free; the pill chrome is purely visual.
 */
function PillSelect({
  value,
  options,
  color,
  onChange,
}: {
  value: string;
  options: readonly string[];
  color: string;
  onChange(value: string): void;
}) {
  return (
    <span
      style={{
        position: "relative",
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
        padding: "3px 10px",
        borderRadius: 999,
        background: "var(--surface-card)",
        border: `1px solid ${color}`,
        color: "var(--text-primary)",
        fontSize: "var(--text-xs)",
        cursor: "pointer",
        minWidth: 0,
      }}
    >
      <span
        style={{
          width: 8,
          height: 8,
          borderRadius: "50%",
          background: color,
          flexShrink: 0,
        }}
        aria-hidden
      />
      <span>{value.replace(/_/g, " ")}</span>
      <select
        value={value}
        onChange={(event) => onChange(event.target.value)}
        style={{
          position: "absolute",
          inset: 0,
          opacity: 0,
          cursor: "pointer",
          width: "100%",
          height: "100%",
          font: "inherit",
        }}
      >
        {options.map((option) => (
          <option key={option} value={option}>{option.replace(/_/g, " ")}</option>
        ))}
      </select>
    </span>
  );
}

/**
 * Inline-editable single-line chip text — used for Category in the
 * rail. Click the displayed value (or the placeholder) to swap to a
 * borderless input; Enter / blur commits, Escape reverts.
 */
function InlineChipText({
  value,
  placeholder,
  onCommit,
}: {
  value: string;
  placeholder: string;
  onCommit(value: string): void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const cancelRequested = useRef(false);
  useEffect(() => { if (!editing) setDraft(value); }, [value, editing]);
  if (!editing) {
    return (
      <button
        type="button"
        onClick={() => { setDraft(value); setEditing(true); }}
        style={{
          background: "transparent",
          border: "none",
          padding: 0,
          color: value ? "var(--text-primary)" : "var(--text-muted)",
          fontSize: "var(--text-xs)",
          cursor: "text",
          textAlign: "left",
        }}
      >
        {value || placeholder}
      </button>
    );
  }
  return (
    <input
      autoFocus
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => {
        if (cancelRequested.current) {
          cancelRequested.current = false;
          setDraft(value);
          setEditing(false);
          return;
        }
        setEditing(false);
        if (draft !== value) onCommit(draft);
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          cancelRequested.current = true;
          (e.target as HTMLInputElement).blur();
        } else if (e.key === "Enter") {
          e.preventDefault();
          (e.target as HTMLInputElement).blur();
        }
      }}
      style={{
        ...inputStyle,
        width: "100%",
        padding: "2px 6px",
        fontSize: "var(--text-xs)",
      }}
    />
  );
}

/**
 * Tag chip list — renders the comma-separated string as a row of
 * small chips when not editing; clicking the row swaps to a single
 * input where the user can edit the raw comma-separated form.
 *
 * Per-chip editing (add/remove individually) is a follow-up; for now
 * we just make the read-state visual.
 */
function TagChipList({
  value,
  onCommit,
}: {
  value: string;
  onCommit(value: string): void;
}) {
  const tags = value
    .split(",")
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      {tags.length > 0 ? (
        <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
          {tags.map((tag) => (
            <span
              key={tag}
              style={{
                fontSize: "var(--text-xs)",
                padding: "1px 8px",
                borderRadius: 999,
                background: "var(--accent-soft-bg)",
                color: "var(--text-primary)",
              }}
            >{tag}</span>
          ))}
        </div>
      ) : null}
      <InlineChipText
        value={value}
        placeholder={tags.length > 0 ? "+ Add tag" : "+ Add tags"}
        onCommit={onCommit}
      />
    </div>
  );
}

function RailMetaRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", gap: 8 }}>
      <span style={{ width: 56, color: "var(--text-muted)" }}>{label}</span>
      <span style={{ color: "var(--text-secondary)" }}>{children}</span>
    </div>
  );
}

const menuItemStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  textAlign: "left",
  padding: "6px 10px",
  fontSize: "var(--text-xs)",
  color: "var(--text-primary)",
  cursor: "pointer",
  borderRadius: 3,
};

/**
 * Single chronological list (newest first) mixing tasks notes and
 * efforts inside the tasks modal. Replaces the previous two-section
 * layout (Notes pane + separate Efforts pane with an "active effort"
 * callout box) so the timeline reads top-to-bottom without overlap.
 *
 * Active effort renders inline at the top with a subtle "in progress"
 * badge — no callout box.
 */
export function ActivityTimeline({
  efforts,
  formatTimestamp,
  onOpenFile,
  onShowInHistory,
}: {
  efforts: EffortDetail[];
  formatTimestamp(iso: string): string;
  onOpenFile?(path: string): void | Promise<void>;
  onShowInHistory?(snapshotId: string): void;
}) {
  const rows = buildActivityTimeline(efforts);
  if (rows.length === 0) {
    return (
      <div style={{
        color: "var(--text-muted)",
        fontSize: "var(--text-xs)",
        fontStyle: "italic",
        padding: "8px 12px",
      }}>
        No activity yet — moving this item to "in progress" starts an effort.
      </div>
    );
  }
  return (
    <div
      data-testid="tasks-activity"
      style={{ display: "flex", flexDirection: "column", gap: 10 }}
    >
      {rows.map((row) => (
        <ActivityEffortCard
          key={`effort-${row.id}`}
          detail={row.detail}
          active={row.active}
          formatTimestamp={formatTimestamp}
          onOpenFile={onOpenFile}
          onShowInHistory={onShowInHistory}
        />
      ))}
    </div>
  );
}

/**
 * Standalone card per effort. Header strip on top (effort label,
 * timestamps, +a ~b −c counts, In-history link) sits on a tinted band;
 * body below holds changed paths + the effort summary (rendered via
 * `MarkdownView` so mermaid + internal links keep working in
 * read-only effort summaries).
 *
 * No pencil affordance on these cards — they're read-only by design.
 */
function ActivityEffortCard({
  detail,
  active,
  formatTimestamp,
  onOpenFile,
  onShowInHistory,
}: {
  detail: EffortDetail;
  active: boolean;
  formatTimestamp(iso: string): string;
  onOpenFile?(path: string): void | Promise<void>;
  onShowInHistory?(snapshotId: string): void;
}) {
  const ctxNav = useOptionalPageNavigation();
  const openFile = (path: string) => {
    if (ctxNav) ctxNav.navigate(fileRef(path), { newTab: false });
    else void onOpenFile?.(path);
  };
  const endSnapshotId = detail.effort.end_snapshot_id;
  const counts = detail.counts;
  const totalChanged = counts.created + counts.updated + counts.deleted;
  return (
    <div
      data-testid={active ? "tasks-effort-in-progress" : `tasks-effort-${detail.effort.id}`}
      style={{
        display: "flex",
        flexDirection: "column",
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        overflow: "hidden",
      }}
    >
      <div style={{
        display: "flex",
        gap: 8,
        alignItems: "center",
        flexWrap: "wrap",
        padding: "6px 10px",
        background: "var(--surface-tab-active)",
        borderBottom: "1px solid var(--border-subtle)",
        fontSize: 11,
        color: "var(--text-secondary)",
      }}>
        <span style={{
          textTransform: "uppercase",
          letterSpacing: 0.4,
          fontSize: 10,
          fontWeight: 600,
          color: "var(--text-primary)",
        }}>Effort</span>
        {active ? (
          <span style={{
            color: "var(--accent)",
            fontWeight: 600,
            fontSize: 10,
            textTransform: "uppercase",
            letterSpacing: 0.4,
            background: "var(--accent-soft-bg)",
            padding: "1px 6px",
            borderRadius: 999,
          }}>in progress</span>
        ) : null}
        <span>{formatTimestamp(detail.effort.started_at)}</span>
        {detail.effort.ended_at ? <span>→ {formatTimestamp(detail.effort.ended_at)}</span> : null}
        <span style={{
          marginLeft: "auto",
          display: "flex",
          gap: 8,
          alignItems: "baseline",
          fontFamily: "var(--font-mono)",
        }}>
          {counts.created > 0 ? <span style={{ color: "var(--diff-add-fg)" }}>+{counts.created}</span> : null}
          {counts.updated > 0 ? <span style={{ color: "var(--priority-high)" }}>~{counts.updated}</span> : null}
          {counts.deleted > 0 ? <span style={{ color: "var(--diff-remove-fg)" }}>−{counts.deleted}</span> : null}
          {!active && totalChanged === 0 ? <span style={{ color: "var(--text-muted)" }}>0 files</span> : null}
        </span>
        {onShowInHistory && !active ? (
          <button
            type="button"
            data-testid={`tasks-show-in-history-${detail.effort.id}`}
            onClick={() => { if (endSnapshotId) onShowInHistory(endSnapshotId); }}
            style={{ ...miniButtonStyle, padding: "1px 8px", fontSize: 10 }}
            disabled={!endSnapshotId}
            title={endSnapshotId ? "Open Local History at this effort's end snapshot" : "Effort is still open — no end snapshot yet"}
          >
            In history
          </button>
        ) : null}
      </div>
      <div style={{ padding: "8px 10px", display: "flex", flexDirection: "column", gap: 8 }}>
        {detail.changed_paths.length > 0 ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {detail.changed_paths.map((path) => (
              <div key={path} style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
                {onOpenFile ? (
                  <button
                    type="button"
                    onClick={() => openFile(path)}
                    style={{
                      background: "transparent",
                      border: "none",
                      padding: 0,
                      color: "var(--accent)",
                      cursor: "pointer",
                      textAlign: "left",
                      font: "inherit",
                      fontFamily: "var(--font-mono)",
                      textDecoration: "underline",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                      flex: 1,
                      minWidth: 0,
                    }}
                  >
                    {path}
                  </button>
                ) : (
                  <span style={{
                    flex: 1,
                    minWidth: 0,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    fontFamily: "var(--font-mono)",
                  }}>{path}</span>
                )}
              </div>
            ))}
          </div>
        ) : null}
        {detail.effort.summary && detail.effort.summary.length > 0 ? (
          <div data-testid={`tasks-effort-summary-${detail.effort.id}`}>
            <MarkdownView body={detail.effort.summary} maxHeight={320} />
          </div>
        ) : !active ? (
          <div
            data-testid={`tasks-effort-summary-${detail.effort.id}`}
            style={{ fontSize: 11, color: "var(--text-muted)", fontStyle: "italic" }}
          >
            No summary recorded for this effort.
          </div>
        ) : null}
      </div>
    </div>
  );
}

function EditableField({
  label,
  value,
  placeholder,
  multiline,
  renderMarkdown = false,
  onCommit,
}: {
  label: string;
  value: string;
  placeholder: string;
  multiline: boolean;
  /**
   * When true and the field is not being edited and the value is non-empty,
   * render the value as markdown (headings, lists, code, links, emphasis)
   * instead of as a plain textarea. Click the rendered surface to edit.
   * Long content gets a max-height + internal scroll so the modal/row
   * doesn't grow unbounded.
   */
  renderMarkdown?: boolean;
  onCommit(value: string): void;
}) {
  const [draft, setDraft] = useState(value);
  const [editing, setEditing] = useState(false);
  // When rendering markdown for the value, the textarea is hidden until the
  // user clicks the rendered surface. `revealEditor` swaps the markdown view
  // for the textarea (which then autoFocuses → setEditing(true)).
  const [revealEditor, setRevealEditor] = useState(false);
  // Latch "the user clicked Cancel" across the mousedown → blur → click chain
  // so the blur handler knows to skip auto-commit and revert instead. Using a
  // ref avoids a state update during the mousedown event.
  const cancelRequested = useRef(false);
  const dirty = draft !== value;

  useEffect(() => {
    if (!editing) setDraft(value);
  }, [value, editing]);

  const commit = () => {
    setEditing(false);
    setRevealEditor(false);
    if (draft === value) return;
    onCommit(draft);
  };

  const revert = () => {
    setDraft(value);
    setEditing(false);
    setRevealEditor(false);
  };

  // Show the markdown view when the field has rendered content and the
  // user isn't editing yet. Clicking it reveals the editor.
  const showMarkdown = renderMarkdown && multiline && !editing && !revealEditor && value.length > 0;

  const inputProps = {
    value: draft,
    placeholder,
    autoFocus: revealEditor,
    onChange: (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setDraft(event.target.value),
    onFocus: () => setEditing(true),
    onBlur: () => {
      if (cancelRequested.current) {
        cancelRequested.current = false;
        revert();
      } else {
        commit();
      }
    },
    onKeyDown: (event: React.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancelRequested.current = true;
        (event.target as HTMLElement).blur();
      } else if (event.key === "Enter" && !multiline) {
        event.preventDefault();
        (event.target as HTMLElement).blur();
      } else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        (event.target as HTMLElement).blur();
      }
    },
    style: {
      ...inputStyle,
      width: "100%",
      minHeight: multiline ? 48 : undefined,
      resize: multiline ? ("vertical" as const) : undefined,
      fontFamily: "inherit",
    },
  };

  // Save/Cancel surface while the user is actively editing a dirty draft.
  // Clicking Save would blur the input anyway (→ commit); the button is
  // mostly a visible "here's how to save" affordance. Cancel has to set the
  // cancelRequested latch from mousedown so the blur that follows reverts
  // instead of committing.
  const actions = editing && dirty ? (
    <div style={actionRowStyle}>
      <button
        type="button"
        onMouseDown={(event) => { event.preventDefault(); cancelRequested.current = true; }}
        onClick={revert}
        style={{ ...miniButtonStyle, padding: "3px 10px" }}
        title="Discard changes to this field (Escape)"
      >Cancel</button>
      <button
        type="button"
        onClick={commit}
        style={{ ...miniButtonStyle, padding: "3px 10px", background: "var(--accent)", color: "#fff", borderColor: "var(--accent)" }}
        title={multiline ? "Save changes (Cmd/Ctrl+Enter)" : "Save changes (Enter)"}
      >Save</button>
    </div>
  ) : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, color: "var(--muted)" }}>{label}</div>
      {showMarkdown ? (
        <div
          role="button"
          tabIndex={0}
          onClick={() => setRevealEditor(true)}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              setRevealEditor(true);
            }
          }}
          title="Click to edit"
          style={markdownSurfaceStyle}
        >
          <MarkdownView body={value} maxHeight={320} />
        </div>
      ) : multiline ? (
        <textarea {...inputProps} />
      ) : (
        <input {...inputProps} />
      )}
      {actions}
    </div>
  );
}

const markdownSurfaceStyle: CSSProperties = {
  border: "1px solid transparent",
  borderRadius: 4,
  padding: "4px 6px",
  cursor: "text",
  background: "transparent",
  fontSize: "var(--text-xs)",
  lineHeight: 1.45,
};

const actionRowStyle: CSSProperties = {
  display: "flex",
  justifyContent: "flex-end",
  gap: 6,
  marginTop: 2,
};

