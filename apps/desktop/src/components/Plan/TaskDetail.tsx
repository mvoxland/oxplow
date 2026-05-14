import { useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import type { EffortDetail, Task, TaskPriority, TaskStatus } from "../../api.js";
import { MarkdownView } from "../Wiki/MarkdownView.js";
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
  acceptanceCriteria?: string | null;
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
 * Body half of the tasks detail view — title, description, acceptance.
 * Status / priority / category / tags / timestamps / destructive actions
 * live in `TaskDetailRail`, rendered as the page's right rail.
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
      style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--text-xs)" }}
      onClick={(event) => event.stopPropagation()}
    >
      <EditableField
        key={`title-${item.id}-${item.updated_at}`}
        label="Title"
        value={item.title}
        placeholder="Title"
        multiline={false}
        onCommit={(value) => {
          const trimmed = value.trim();
          if (!trimmed || trimmed === item.title) return;
          void onUpdateTask(item.id, { title: trimmed });
        }}
      />
      <EditableField
        key={`desc-${item.id}-${item.updated_at}`}
        label="Description"
        value={item.description}
        placeholder="Add a description…"
        multiline
        renderMarkdown
        onCommit={(value) => {
          if (value === item.description) return;
          void onUpdateTask(item.id, { description: value });
        }}
      />
      <EditableField
        key={`accept-${item.id}-${item.updated_at}`}
        label="Acceptance"
        value={item.acceptance_criteria ?? ""}
        placeholder="Acceptance criteria, one per line"
        multiline
        renderMarkdown
        onCommit={(value) => {
          const next = value.length === 0 ? null : value;
          if (next === item.acceptance_criteria) return;
          void onUpdateTask(item.id, { acceptanceCriteria: next });
        }}
      />
    </div>
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

      <RailFieldRow label="Status">
        <InlineSelect
          value={item.status}
          options={statusOptionsFor(item.status)}
          onChange={(value) => void onUpdateTask(item.id, { status: value as TaskStatus })}
        />
      </RailFieldRow>
      <RailFieldRow label="Priority">
        <InlineSelect
          value={item.priority}
          options={PRIORITY_OPTIONS}
          onChange={(value) => void onUpdateTask(item.id, { priority: value as TaskPriority })}
        />
      </RailFieldRow>

      <EditableField
        key={`category-${item.id}-${item.updated_at}`}
        label="Category"
        value={item.category ?? ""}
        placeholder="Backlog bucket (e.g. UI, Infra)"
        multiline={false}
        onCommit={(value) => {
          const trimmed = value.trim();
          const next = trimmed.length === 0 ? null : trimmed;
          if (next === (item.category ?? null)) return;
          void onUpdateTask(item.id, { category: next });
        }}
      />
      <EditableField
        key={`tags-${item.id}-${item.updated_at}`}
        label="Tags"
        value={item.tags ?? ""}
        placeholder="Comma-separated tags"
        multiline={false}
        onCommit={(value) => {
          const trimmed = value.trim();
          const next = trimmed.length === 0 ? null : trimmed;
          if (next === (item.tags ?? null)) return;
          void onUpdateTask(item.id, { tags: next });
        }}
      />

      <div style={{
        display: "flex",
        flexDirection: "column",
        gap: 4,
        borderTop: "1px solid var(--border-subtle)",
        paddingTop: 12,
        color: "var(--text-muted)",
      }}>
        <RailMetaRow label="Created">{formatTimestamp(item.created_at)}</RailMetaRow>
        <RailMetaRow label="Updated">{formatTimestamp(item.updated_at)}</RailMetaRow>
        <RailMetaRow label="By">{item.created_by}</RailMetaRow>
      </div>
    </div>
  );
}

function RailFieldRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, color: "var(--muted)" }}>{label}</div>
      <div>{children}</div>
    </div>
  );
}

function RailMetaRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", gap: 8, fontSize: 11 }}>
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
  const ctxNav = useOptionalPageNavigation();
  const openFile = (path: string) => {
    if (ctxNav) ctxNav.navigate(fileRef(path), { newTab: false });
    else void onOpenFile?.(path);
  };
  const rows = buildActivityTimeline(efforts);
  if (rows.length === 0) {
    return (
      <div style={{ color: "var(--muted)", fontSize: "var(--text-xs)", fontStyle: "italic" }}>
        No activity yet — moving this item to "in progress" starts an effort.
      </div>
    );
  }
  return (
    <div
      data-testid="tasks-activity"
      style={{ display: "flex", flexDirection: "column", gap: 8, overflowY: "auto", border: "1px solid var(--border)", borderRadius: 6, padding: 8, background: "var(--bg-1)" }}
    >
      {rows.map((row) => (
        <ActivityEffortRow
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

function ActivityEffortRow({
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
        gap: 4,
        borderLeft: `2px solid ${active ? "var(--accent)" : "var(--border)"}`,
        paddingLeft: 8,
      }}
    >
      <div style={{ fontSize: 11, color: "var(--muted)", display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
        <span style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, fontWeight: 600 }}>Effort</span>
        {active ? (
          <span style={{ color: "var(--accent)", fontWeight: 600, fontSize: 10, textTransform: "uppercase", letterSpacing: 0.4 }}>in progress</span>
        ) : null}
        <span>{formatTimestamp(detail.effort.started_at)}</span>
        {detail.effort.ended_at ? <span>→ {formatTimestamp(detail.effort.ended_at)}</span> : null}
        <span style={{ marginLeft: "auto", display: "flex", gap: 6, alignItems: "baseline" }}>
          {counts.created > 0 ? <span style={{ color: "#86efac" }}>+{counts.created}</span> : null}
          {counts.updated > 0 ? <span style={{ color: "#e5a06a" }}>~{counts.updated}</span> : null}
          {counts.deleted > 0 ? <span style={{ color: "#f87171" }}>−{counts.deleted}</span> : null}
          {!active && totalChanged === 0 ? <span>0 files</span> : null}
        </span>
        {onShowInHistory && !active ? (
          <button
            type="button"
            data-testid={`tasks-show-in-history-${detail.effort.id}`}
            onClick={() => { if (endSnapshotId) onShowInHistory(endSnapshotId); }}
            style={{ ...miniButtonStyle, padding: "1px 6px", fontSize: 10 }}
            disabled={!endSnapshotId}
            title={endSnapshotId ? "Open Local History at this effort's end snapshot" : "Effort is still open — no end snapshot yet"}
          >
            In history
          </button>
        ) : null}
      </div>
      {detail.changed_paths.length > 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {detail.changed_paths.map((path) => (
            <div key={path} style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11 }}>
              {onOpenFile ? (
                <button
                  type="button"
                  onClick={() => openFile(path)}
                  style={{ background: "transparent", border: "none", padding: 0, color: "var(--accent)", cursor: "pointer", textAlign: "left", font: "inherit", textDecoration: "underline", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1, minWidth: 0 }}
                >
                  {path}
                </button>
              ) : (
                <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{path}</span>
              )}
            </div>
          ))}
        </div>
      ) : null}
      {detail.effort.summary && detail.effort.summary.length > 0 ? (
        <div data-testid={`tasks-effort-summary-${detail.effort.id}`} style={{ fontSize: "var(--text-xs)" }}>
          <MarkdownView body={detail.effort.summary} maxHeight={240} />
        </div>
      ) : !active ? (
        <div data-testid={`tasks-effort-summary-${detail.effort.id}`} style={{ fontSize: 11, color: "var(--muted)", fontStyle: "italic" }}>
          No summary recorded for this effort.
        </div>
      ) : null}
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

function InlineSelect({
  value,
  options,
  onChange,
  suffix,
}: {
  value: string;
  options: readonly string[];
  onChange(value: string): void;
  suffix?: string;
}) {
  return (
    <span style={{ position: "relative", display: "inline-block" }}>
      <span style={{ color: "inherit" }}>{value}{suffix ?? ""}</span>
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
          <option key={option} value={option}>{option}</option>
        ))}
      </select>
    </span>
  );
}
