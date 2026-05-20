import { useEffect, useRef, useState } from "react";

import type { CommentIntent } from "../../tauri-bridge/generated/bindings.js";
import { CommentComposer } from "./CommentComposer.js";

const CARD_WIDTH = 460;

/// The new-comment composer shown directly (no intermediate button)
/// after the user picks "Add comment" from the right-click menu. The
/// owning surface already knows the selection's `quote` + anchor; this
/// collects the first message + intent and hands them back.
export function NewCommentPopover({
  rect,
  onCreate,
  onDismiss,
}: {
  rect: DOMRect;
  onCreate: (input: { body: string; intent: CommentIntent }) => void | Promise<void>;
  onDismiss: () => void;
}) {
  const [intent, setIntent] = useState<CommentIntent>("note");
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onDismiss();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [onDismiss]);

  const left = Math.min(Math.max(8, rect.left), window.innerWidth - CARD_WIDTH - 8);
  const top = Math.min(rect.bottom + 6, window.innerHeight - 260);

  return (
    <div
      ref={ref}
      data-testid="new-comment-popover"
      // Don't let pointer events bubble to a host editor wrapper that
      // would refocus the editor and steal focus from the composer.
      onMouseDown={(e) => e.stopPropagation()}
      onMouseUp={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      style={{
        position: "fixed",
        left,
        top,
        zIndex: 1000,
        width: CARD_WIDTH,
        background: "var(--surface-elevated)",
        border: "1px solid var(--border-strong)",
        borderRadius: 8,
        boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
        padding: 12,
        fontFamily: "var(--font-ui)",
      }}
    >
      <CommentComposer
        submitLabel="Comment"
        placeholder="Add a comment…"
        showIntent
        intent={intent}
        onIntentChange={setIntent}
        testIdPrefix="new-comment"
        onSubmit={(body) => onCreate({ body, intent })}
        onCancel={onDismiss}
      />
    </div>
  );
}
