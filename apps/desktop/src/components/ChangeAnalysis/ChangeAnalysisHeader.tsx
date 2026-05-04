import type { TabRef } from "../../tabs/tabState.js";
import type { ChangeAnalysisTarget } from "../../tabs/pageRefs.js";

export interface ChangeAnalysisHeaderProps {
  target: ChangeAnalysisTarget;
  loading: boolean;
  onRefresh(): void;
  /** Source tab to "Open …" — uncommitted page or git commit page. Null hides the link. */
  sourceLink: TabRef | null;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
}

/**
 * The fixed top row used by every Change Analysis surface (dashboard +
 * any focused drilldown): "Parent vs <target>" caption, Refresh, and
 * Open Source link. Lifted out of `ChangeAnalysisPage` so all variants
 * render the same chrome.
 */
export function ChangeAnalysisHeader({
  target,
  loading,
  onRefresh,
  sourceLink,
  onOpenPage,
}: ChangeAnalysisHeaderProps) {
  return (
    <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
      <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
        {target === "working" ? "Working tree (HEAD vs uncommitted)" : `Parent vs ${target.slice(0, 12)}`}
      </span>
      <button
        type="button"
        data-testid="change-analysis-refresh"
        onClick={onRefresh}
        disabled={loading}
        style={smallButton}
      >
        {loading ? "Loading…" : "Refresh"}
      </button>
      {sourceLink ? (
        <button
          type="button"
          data-testid="change-analysis-open-source"
          onClick={() => onOpenPage(sourceLink)}
          style={linkButton}
        >
          {target === "working" ? "Open Uncommitted →" : "Open Commit →"}
        </button>
      ) : null}
    </div>
  );
}

const smallButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: 12,
};
const linkButton: React.CSSProperties = {
  padding: 0,
  background: "transparent",
  border: "none",
  color: "var(--text-link, #2563eb)",
  fontSize: 12,
  cursor: "pointer",
};
