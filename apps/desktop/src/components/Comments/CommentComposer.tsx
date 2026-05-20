import { useEffect, useRef, useState } from "react";

import type { CommentIntent } from "../../tauri-bridge/generated/bindings.js";

/// Shared textarea composer for both the new-comment flow and replies.
/// Multi-line: Enter inserts a newline, Cmd/Ctrl+Enter submits, Escape
/// cancels (per the usability rules). Submit is disabled while empty.
export function CommentComposer({
  submitLabel,
  placeholder,
  autofocus = true,
  showIntent = false,
  intent = "note",
  onIntentChange,
  onSubmit,
  onCancel,
  testIdPrefix,
}: {
  submitLabel: string;
  placeholder?: string;
  autofocus?: boolean;
  showIntent?: boolean;
  intent?: CommentIntent;
  onIntentChange?: (intent: CommentIntent) => void;
  onSubmit: (body: string) => void | Promise<void>;
  onCancel?: () => void;
  testIdPrefix: string;
}) {
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    if (autofocus) ref.current?.focus();
  }, [autofocus]);

  const canSubmit = body.trim().length > 0 && !busy;

  const submit = async () => {
    if (!canSubmit) return;
    setBusy(true);
    try {
      await onSubmit(body.trim());
      setBody("");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <textarea
        ref={ref}
        value={body}
        placeholder={placeholder ?? "Add a comment…"}
        data-testid={`${testIdPrefix}-input`}
        onChange={(e) => setBody(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void submit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            onCancel?.();
          }
        }}
        rows={8}
        style={{
          resize: "vertical",
          minHeight: 150,
          padding: "8px 10px",
          background: "var(--surface-card)",
          color: "var(--text-primary)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 6,
          font: "inherit",
          fontFamily: "var(--font-ui)",
          fontSize: "var(--text-sm)",
        }}
      />
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        {showIntent && (
          <select
            value={intent}
            data-testid={`${testIdPrefix}-intent`}
            onChange={(e) => onIntentChange?.(e.target.value as CommentIntent)}
            style={{
              padding: "4px 6px",
              background: "var(--surface-card)",
              color: "var(--text-secondary)",
              border: "1px solid var(--border-subtle)",
              borderRadius: 6,
              fontSize: "var(--text-xs)",
            }}
          >
            <option value="note">Note to self</option>
            <option value="followup">Wants follow-up</option>
          </select>
        )}
        <div style={{ flex: 1 }} />
        {onCancel && (
          <button
            type="button"
            data-testid={`${testIdPrefix}-cancel`}
            onClick={onCancel}
            style={secondaryBtn}
          >
            Cancel
          </button>
        )}
        <button
          type="button"
          data-testid={`${testIdPrefix}-submit`}
          onClick={() => void submit()}
          disabled={!canSubmit}
          style={{ ...primaryBtn, opacity: canSubmit ? 1 : 0.5 }}
        >
          {submitLabel}
        </button>
      </div>
    </div>
  );
}

const primaryBtn: React.CSSProperties = {
  padding: "4px 12px",
  background: "var(--button-primary-bg)",
  color: "var(--button-primary-fg)",
  border: "none",
  borderRadius: 6,
  fontSize: "var(--text-sm)",
  cursor: "pointer",
};

const secondaryBtn: React.CSSProperties = {
  padding: "4px 10px",
  background: "transparent",
  color: "var(--text-secondary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  fontSize: "var(--text-sm)",
  cursor: "pointer",
};
