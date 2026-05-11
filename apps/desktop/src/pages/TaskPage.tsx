import { useEffect, useMemo, useState } from "react";
import type { EffortDetail, Stream, Thread, ThreadWorkState, Task, TaskPriority, TaskStatus } from "../api.js";
import {
  getTask,
  listTaskEfforts,
  moveBacklogItemToThread,
  moveTaskToBacklog,
  subscribeOxplowEvents,
  updateTask,
} from "../api.js";
import { miniButtonStyle } from "../components/Plan/plan-utils.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, taskRef } from "../tabs/pageRefs.js";
import { ActivityTimeline, TaskDetail } from "../components/Plan/TaskDetail.js";
import { BacklinksList, type SnapshotBacklinkEntry } from "../tabs/BacklinksList.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { SnapshotDetailSlideover } from "../components/Snapshots/SnapshotDetailSlideover.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";

export interface WorkItemPageProps {
  stream: Stream | null;
  thread: Thread | null;
  itemId: string;
  /** Live snapshot of all work items in the current thread (used to find this one). */
  items: Task[];
  threadWork: ThreadWorkState | null;
  onOpenPage(ref: TabRef): void;
  onOpenFile?(path: string): void;
  onShowInHistory?(snapshotId: string): void;
  /** Forwarded to the embedded SnapshotDetailSlideover so its file rows
   *  can ask the host to open a diff editor. */
  onOpenDiff?(spec: DiffSpec): void;
}

/**
 * Single-record page for a work item. Shows the full editable detail
 * (title, description, acceptance, status, priority) plus the merged
 * activity timeline (notes + efforts). Phase 4 entry point — replaces
 * the modal-only edit flow when callers route via `onOpenPage`.
 *
 * Read-only fallback: if the item isn't in the loaded thread state
 * (e.g. it lives in another thread), the page renders just the title
 * row and a hint to open it from its owning thread.
 */
export function WorkItemPage({
  stream,
  thread,
  itemId,
  items,
  threadWork,
  onOpenPage,
  onOpenFile,
  onShowInHistory,
  onOpenDiff,
}: WorkItemPageProps) {
  // Fallback for items not in the current thread's loaded buckets — backlog
  // rows and items owned by another thread won't appear in `items`. Fetch
  // the row directly so the page renders the full editor regardless.
  const [fetchedItem, setFetchedItem] = useState<Task | null>(null);
  const item = items.find((i) => i.id === itemId) ?? fetchedItem;
  const refForGraph = taskRef(itemId);
  const backlinkEntries = useBacklinks(refForGraph);
  const outboundEntries = usePageOutbound(refForGraph);
  const [efforts, setEfforts] = useState<EffortDetail[]>([]);
  // Slideover state lives on this host page (the brief calls for it).
  // Single instance — opening another snapshot replaces the current one.
  const [slideoverSnapshot, setSlideoverSnapshot] = useState<{
    snapshotId: string;
    label: string | null;
    source: string;
    workItemId: string | null;
  } | null>(null);
  // Synthesize snapshot backlinks from this item's efforts. Each completed
  // effort's `end_snapshot_id` becomes a clickable row that opens the
  // SnapshotDetailSlideover. Skipped when no end snapshot (effort still
  // in progress) so the row never lands without a target.
  const snapshotBacklinks = useMemo<SnapshotBacklinkEntry[]>(() => {
    return efforts
      .filter((d) => !!d.effort.end_snapshot_id)
      .map((d, i) => ({
        kind: "snapshot" as const,
        snapshotId: d.effort.end_snapshot_id!,
        label: `Effort ${i + 1} end snapshot`,
        source: "task-end",
        snapshotLabel: null,
        workItemId: itemId,
        subtitle: `${d.changed_paths.length} file${d.changed_paths.length === 1 ? "" : "s"}`,
      }));
  }, [efforts, itemId]);

  const backlinks = {
    count: backlinkEntries.length + snapshotBacklinks.length,
    body: (
      <BacklinksList
        entries={backlinkEntries}
        snapshotEntries={snapshotBacklinks}
        onOpenPage={onOpenPage}
        onOpenSnapshot={(payload) => setSlideoverSnapshot({
          snapshotId: payload.snapshotId,
          label: payload.label ?? null,
          source: payload.source ?? "",
          workItemId: payload.workItemId ?? null,
        })}
        onOpenCommit={(payload) => onOpenPage(gitCommitRef(payload.sha))}
      />
    ),
  };
  const outbound =
    outboundEntries.length > 0
      ? {
          count: outboundEntries.length,
          body: <BacklinksList entries={outboundEntries} onOpenPage={onOpenPage} />,
        }
      : undefined;

  // Refresh the fallback row whenever this item id is missing from the live
  // thread state, or when the runtime fires a change event for it. This
  // covers backlog rows, foreign-thread rows, and moves between scopes.
  const inThreadItems = items.some((i) => i.id === itemId);
  useEffect(() => {
    let cancelled = false;
    const refetch = () => {
      void getTask(itemId).then((row) => {
        if (!cancelled) setFetchedItem(row);
      });
    };
    if (!inThreadItems) refetch();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.type !== "task.changed") return;
      const targetId = (event as unknown as { itemId?: string }).itemId;
      if (targetId !== itemId) return;
      refetch();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [itemId, inThreadItems]);

  useEffect(() => {
    if (!item) return;
    let cancelled = false;
    void listTaskEfforts(item.id).then((rows) => {
      if (!cancelled) setEfforts(rows);
    });
    const unsub = subscribeOxplowEvents((event) => {
      if (event.type !== "task.changed") return;
      const targetId = (event as unknown as { itemId?: string }).itemId;
      if (targetId !== item.id) return;
      void listTaskEfforts(item.id).then((rows) => {
        if (!cancelled) setEfforts(rows);
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [item?.id]);

  const handleUpdate = async (
    targetId: string,
    changes: { title?: string; description?: string; acceptanceCriteria?: string | null; status?: TaskStatus; priority?: TaskPriority },
  ) => {
    if (!stream || !thread) return;
    await updateTask(stream.id, thread.id, targetId, changes);
  };

  // Backlog ↔ thread scope toggle. The button label/handler depend on the
  // item's current `thread_id`: a thread-attached row offers "Send to
  // backlog"; a backlog row offers "Bring to thread" against the active
  // thread. Items owned by another thread show no action — the user would
  // need to navigate there to move them.
  const itemThreadId = item?.thread_id ?? null;
  const scopeAction: { label: string; run: () => Promise<void> } | null = (() => {
    if (!item || !stream) return null;
    if (itemThreadId === null && thread) {
      return {
        label: "Bring to this thread",
        run: async () => {
          await moveBacklogItemToThread(stream.id, item.id, thread.id);
        },
      };
    }
    if (thread && itemThreadId === thread.id) {
      return {
        label: "Send to backlog",
        run: async () => {
          await moveTaskToBacklog(stream.id, thread.id, item.id);
        },
      };
    }
    return null;
  })();

  const slideover = (
    <SnapshotDetailSlideover
      open={!!slideoverSnapshot}
      onClose={() => setSlideoverSnapshot(null)}
      stream={stream}
      snapshotId={slideoverSnapshot?.snapshotId ?? null}
      snapshotLabel={slideoverSnapshot?.label ?? null}
      snapshotSource={slideoverSnapshot?.source ?? ""}
      workItemId={slideoverSnapshot?.workItemId ?? null}
      onOpenDiff={onOpenDiff}
      onOpenWorkItem={(targetId) => onOpenPage(taskRef(targetId))}
    />
  );

  if (!item) {
    return (
      <Page testId="page-work-item" title={itemId} kind="work item" backlinks={backlinks} outbound={outbound}>
        <div style={{ padding: "16px 20px", color: "var(--text-secondary)", fontSize: 13 }}>
          Loading work item…
        </div>
        {slideover}
      </Page>
    );
  }

  const chips = [
    { label: item.status },
    { label: `${item.priority} priority` },
  ];

  return (
    <Page testId="page-work-item" title={item.title} kind="work item" chips={chips} backlinks={backlinks} outbound={outbound}>
      <div style={{ padding: "12px 16px", display: "flex", flexDirection: "column", gap: 12 }}>
        <TaskDetail
          item={item}
          onUpdateWorkItem={handleUpdate}
          onRequestDelete={() => {}}
          headerActions={
            scopeAction ? (
              <button
                type="button"
                style={miniButtonStyle}
                onClick={() => void scopeAction.run()}
              >
                {scopeAction.label}
              </button>
            ) : undefined
          }
        />
        <div>
          <div style={{ fontSize: 11, fontWeight: 600, color: "var(--text-secondary)", textTransform: "uppercase", letterSpacing: 0.4, marginBottom: 6 }}>
            Activity
          </div>
          <ActivityTimeline
            efforts={efforts}
            formatTimestamp={(iso) => new Date(iso).toLocaleString()}
            onOpenFile={onOpenFile}
            onShowInHistory={onShowInHistory}
          />
        </div>
      </div>
      {slideover}
    </Page>
  );
}
