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

const selectStyle: React.CSSProperties = {
  background: "transparent",
  color: "var(--text)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "2px 6px",
  fontSize: 12,
};

/**
 * Inline scope selector rendered above the Tasks list. The first dropdown
 * picks the overall scope kind; the secondary dropdowns appear only when
 * the kind needs them (specific thread / specific stream). Persistence
 * is handled by the parent via tasks-scope.ts.
 */
export function TasksScopePicker({
  scope,
  onChange,
  streams,
  threadsByStream,
  onRequestThreads,
}: TasksScopePickerProps) {
  function setKind(kind: TasksScope["kind"]) {
    if (kind === "currentThread") onChange({ kind: "currentThread" });
    else if (kind === "all") onChange({ kind: "all" });
    else if (kind === "stream") {
      const first = streams[0]?.id ?? "";
      onChange({ kind: "stream", streamId: first });
    } else if (kind === "thread") {
      const firstStream = streams[0]?.id ?? "";
      const threads = threadsByStream[firstStream] ?? [];
      const firstThread = threads[0]?.id ?? "";
      if (firstStream && !threads.length) onRequestThreads(firstStream);
      onChange({ kind: "thread", streamId: firstStream, threadId: firstThread });
    }
  }

  const streamId = scope.kind === "stream" || scope.kind === "thread" ? scope.streamId : "";
  const threads = streamId ? (threadsByStream[streamId] ?? []) : [];

  return (
    <div
      data-testid="tasks-scope-picker"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 10px",
        borderBottom: "1px solid var(--border)",
        fontSize: 12,
      }}
    >
      <span style={{ color: "var(--muted)" }}>Show:</span>
      <select
        data-testid="tasks-scope-kind"
        value={scope.kind}
        onChange={(e) => setKind(e.target.value as TasksScope["kind"])}
        style={selectStyle}
      >
        <option value="currentThread">Current thread</option>
        <option value="thread">Specific thread</option>
        <option value="stream">Stream (all threads merged)</option>
        <option value="all">Everything (all streams)</option>
      </select>

      {(scope.kind === "stream" || scope.kind === "thread") ? (
        <select
          data-testid="tasks-scope-stream"
          value={streamId}
          onChange={(e) => {
            const next = e.target.value;
            if (scope.kind === "stream") onChange({ kind: "stream", streamId: next });
            else {
              onRequestThreads(next);
              const firstThread = threadsByStream[next]?.[0]?.id ?? "";
              onChange({ kind: "thread", streamId: next, threadId: firstThread });
            }
          }}
          style={selectStyle}
        >
          {streams.length === 0 ? <option value="">(no streams)</option> : null}
          {streams.map((s) => (
            <option key={s.id} value={s.id}>{s.title}</option>
          ))}
        </select>
      ) : null}

      {scope.kind === "thread" ? (
        <select
          data-testid="tasks-scope-thread"
          value={scope.threadId}
          onChange={(e) =>
            onChange({ kind: "thread", streamId: scope.streamId, threadId: e.target.value })
          }
          style={selectStyle}
        >
          {threads.length === 0 ? <option value="">(loading…)</option> : null}
          {threads.map((t) => (
            <option key={t.id} value={t.id}>{t.title || t.id.slice(0, 8)}</option>
          ))}
        </select>
      ) : null}
    </div>
  );
}
