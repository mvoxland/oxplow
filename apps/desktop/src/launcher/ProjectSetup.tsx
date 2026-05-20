import { useCallback, useEffect, useRef, useState } from "react";

import { abortSetup, setupProject } from "../api.js";
import { logUi } from "../logger.js";

/// First-run confirmation shown when a directory without an `.oxplow/`
/// dir is opened (see the setup launch mode in `.context/
/// architecture.md`). Create initializes the project and relaunches
/// into it; Cancel closes this window. Enter submits, Escape cancels.
export function ProjectSetup({ dir }: { dir: string }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const createRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    createRef.current?.focus();
  }, []);

  const handleCreate = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await setupProject(dir);
      // setupProject relaunches and exits this process; nothing to do.
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      setBusy(false);
      logUi("warn", "project setup failed", { dir, error: message });
    }
  }, [dir]);

  const handleCancel = useCallback(() => {
    void abortSetup();
  }, []);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") handleCancel();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [handleCancel]);

  return (
    <div data-testid="project-setup" style={rootStyle}>
      <form
        style={cardStyle}
        onSubmit={(e) => {
          e.preventDefault();
          void handleCreate();
        }}
      >
        <h1 style={titleStyle}>Create Oxplow Project?</h1>
        <p style={bodyStyle}>
          This folder isn’t an Oxplow project yet:
        </p>
        <code style={pathStyle}>{dir}</code>
        <p style={bodyStyle}>
          Create one here? Oxplow will add a <code>.oxplow</code> directory to
          store its state.
        </p>

        {error ? (
          <div data-testid="project-setup-error" style={errorStyle}>
            {error}
          </div>
        ) : null}

        <div style={actionsStyle}>
          <button
            type="button"
            data-testid="project-setup-cancel"
            onClick={handleCancel}
            disabled={busy}
            style={cancelButtonStyle}
          >
            Cancel
          </button>
          <button
            ref={createRef}
            type="submit"
            data-testid="project-setup-create"
            disabled={busy}
            style={createButtonStyle}
          >
            {busy ? "Creating…" : "Create Project"}
          </button>
        </div>
      </form>
    </div>
  );
}

const rootStyle: React.CSSProperties = {
  height: "100vh",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  background: "var(--surface-app)",
  color: "var(--text-primary)",
  fontSize: "var(--text-sm)",
};

const cardStyle: React.CSSProperties = {
  width: 480,
  maxWidth: "90vw",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 10,
  padding: 28,
  boxSizing: "border-box",
};

const titleStyle: React.CSSProperties = {
  margin: "0 0 12px",
  fontSize: 18,
  fontWeight: "var(--weight-bold)" as unknown as number,
};

const bodyStyle: React.CSSProperties = {
  margin: "8px 0",
  color: "var(--text-secondary)",
  lineHeight: 1.5,
};

const pathStyle: React.CSSProperties = {
  display: "block",
  fontFamily: "var(--font-mono)",
  fontSize: "var(--text-xs)",
  color: "var(--text-primary)",
  background: "var(--surface-app)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "8px 10px",
  margin: "8px 0",
  wordBreak: "break-all",
};

const actionsStyle: React.CSSProperties = {
  display: "flex",
  justifyContent: "flex-end",
  gap: 8,
  marginTop: 20,
};

const baseButtonStyle: React.CSSProperties = {
  padding: "8px 16px",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: "var(--text-sm)",
  fontWeight: "var(--weight-medium)" as unknown as number,
};

const cancelButtonStyle: React.CSSProperties = {
  ...baseButtonStyle,
  background: "transparent",
  color: "var(--text-secondary)",
  border: "1px solid var(--border-subtle)",
};

const createButtonStyle: React.CSSProperties = {
  ...baseButtonStyle,
  background: "var(--accent)",
  color: "var(--accent-on-accent)",
  border: "none",
};

const errorStyle: React.CSSProperties = {
  background: "var(--accent-soft-bg)",
  color: "var(--severity-critical)",
  border: "1px solid var(--severity-critical)",
  borderRadius: 6,
  padding: "8px 12px",
  marginTop: 12,
  fontSize: "var(--text-xs)",
};
