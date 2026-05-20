/* eslint-disable @typescript-eslint/no-explicit-any */
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";

import { createComment, setCommentAnchor } from "../../api.js";
import type { CommentIntent } from "../../tauri-bridge/generated/bindings.js";
import { resolveQuoteOffset } from "./anchor.js";
import { CommentPopover } from "./CommentPopover.js";
import { NewCommentPopover } from "./NewCommentPopover.js";
import { useCommentsForTarget } from "./useCommentsForTarget.js";

interface PendingSel {
  quote: string;
  anchorJson: string;
  rect: DOMRect;
}

/// Imperative surface EditorPane drives from its right-click menu.
export interface MonacoCommentHandle {
  /// The comment id whose decoration covers `position`, or null.
  commentIdAt(position: any): number | null;
  /// Open the composer for the current editor selection (if non-empty).
  addCommentForSelection(): void;
  /// Open the thread popover for `commentId`.
  openComment(commentId: number): void;
}

/// Comment overlay for the Monaco editor. Renders inline highlights and
/// owns the composer + thread popover, but no longer reacts to plain
/// clicks or selection — creation/opening are driven by EditorPane's
/// right-click menu via the imperative handle (so comments don't fight
/// cursor placement or text selection).
export const MonacoCommentLayer = forwardRef<
  MonacoCommentHandle,
  {
    editor: any;
    monaco: any;
    ready: boolean;
    streamId: string;
    threadId: string | null;
    filePath: string;
  }
>(function MonacoCommentLayer({ editor, monaco, ready, streamId, threadId, filePath }, ref) {
  const { threads } = useCommentsForTarget("file", filePath);
  const threadsRef = useRef(threads);
  threadsRef.current = threads;
  const decoIdsRef = useRef<string[]>([]);
  const decoMapRef = useRef<{ decoId: string; commentId: number }[]>([]);
  const [active, setActive] = useState<{ id: number; rect: DOMRect } | null>(null);
  const [pending, setPending] = useState<PendingSel | null>(null);

  // Re-anchor each comment to a Monaco range and paint inline highlights.
  useEffect(() => {
    if (!ready || !editor || !monaco) return;
    const model = editor.getModel?.();
    if (!model) return;
    const text: string = model.getValue();
    const decos: any[] = [];
    const map: { decoId: string; commentId: number }[] = [];

    for (const thread of threads) {
      const c = thread.comment;
      let range: any = null;
      try {
        const a = JSON.parse(c.anchor_json) as Record<string, number>;
        if (typeof a.startLine === "number") {
          const r = new monaco.Range(a.startLine, a.startColumn, a.endLine, a.endColumn);
          if (model.getValueInRange(r) === c.quote) range = r;
        }
      } catch {
        // fall through to quote search
      }
      if (!range) {
        const offset = resolveQuoteOffset(text, c.quote);
        if (offset !== null) {
          const start = model.getPositionAt(offset);
          const end = model.getPositionAt(offset + c.quote.length);
          range = new monaco.Range(start.lineNumber, start.column, end.lineNumber, end.column);
        }
      }
      if (range) {
        decos.push({
          range,
          options: { inlineClassName: "oxplow-comment-highlight", stickiness: 1 },
        });
        const aj = JSON.stringify({
          startLine: range.startLineNumber,
          startColumn: range.startColumn,
          endLine: range.endLineNumber,
          endColumn: range.endColumn,
        });
        if (c.orphaned || c.anchor_json !== aj) void setCommentAnchor(c.id, aj, false);
        map.push({ decoId: "", commentId: c.id });
      } else if (!c.orphaned) {
        void setCommentAnchor(c.id, c.anchor_json, true);
      }
    }

    const ids: string[] = editor.deltaDecorations(decoIdsRef.current, decos);
    decoIdsRef.current = ids;
    decoMapRef.current = ids.map((decoId, i) => ({ decoId, commentId: map[i]?.commentId ?? -1 }));
  }, [ready, editor, monaco, threads, filePath]);

  // Build a viewport rect from a Monaco position (read model/editor live).
  const rectAtPosition = (position: any): DOMRect | null => {
    const vis = editor?.getScrolledVisiblePosition?.(position);
    const dom = editor?.getDomNode?.();
    if (!vis || !dom) return null;
    const host = dom.getBoundingClientRect();
    return new DOMRect(host.left + vis.left, host.top + vis.top, 0, vis.height);
  };

  useImperativeHandle(
    ref,
    (): MonacoCommentHandle => ({
      commentIdAt(position) {
        const model = editor?.getModel?.();
        if (!model || !position) return null;
        for (const { decoId, commentId } of decoMapRef.current) {
          const r = model.getDecorationRange(decoId);
          if (r && r.containsPosition(position)) return commentId;
        }
        return null;
      },
      addCommentForSelection() {
        const sel = editor?.getSelection?.();
        const model = editor?.getModel?.();
        if (!sel || !model || sel.isEmpty()) return;
        const quote: string = model.getValueInRange(sel).trim();
        if (!quote) return;
        const rect = rectAtPosition(sel.getEndPosition());
        if (!rect) return;
        const anchorJson = JSON.stringify({
          startLine: sel.startLineNumber,
          startColumn: sel.startColumn,
          endLine: sel.endLineNumber,
          endColumn: sel.endColumn,
        });
        setActive(null);
        setPending({ quote, anchorJson, rect });
      },
      openComment(commentId) {
        const model = editor?.getModel?.();
        const entry = decoMapRef.current.find((e) => e.commentId === commentId);
        const r = entry && model ? model.getDecorationRange(entry.decoId) : null;
        const rect = r ? rectAtPosition(r.getStartPosition()) : null;
        setPending(null);
        setActive({ id: commentId, rect: rect ?? new DOMRect(120, 120, 0, 0) });
      },
    }),
    [editor],
  );

  const handleCreate = async (input: { body: string; intent: CommentIntent }) => {
    if (!pending) return;
    await createComment({
      streamId,
      threadId,
      targetKind: "file",
      targetId: filePath,
      quote: pending.quote,
      anchorJson: pending.anchorJson,
      intent: input.intent,
      author: "user",
      body: input.body,
    });
    setPending(null);
  };

  const activeThread = active != null ? threads.find((t) => t.comment.id === active.id) : undefined;

  return (
    <>
      {pending && (
        <NewCommentPopover
          rect={pending.rect}
          onCreate={handleCreate}
          onDismiss={() => setPending(null)}
        />
      )}
      {active && activeThread && (
        <CommentPopover
          thread={activeThread}
          anchorRect={active.rect}
          onClose={() => setActive(null)}
        />
      )}
    </>
  );
});
