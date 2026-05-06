import type { ChangeAnalysisScope } from "../../tabs/pageRefs.js";

interface ScopeFilterBannerProps {
  scope: ChangeAnalysisScope;
  /** Clear-filter handler. Hosts wire this to navigate back to
   *  their own scope-less ref so the user lands on the same page
   *  unfiltered. */
  onClear(): void;
}

/**
 * Banner shown at the very top of a host page when a drilldown
 * scope is active. Tells the user what's being filtered and gives
 * them a single click to clear it. Kept reusable since both the
 * commit page and the uncommitted-changes page render the same
 * scoped views.
 */
export function ScopeFilterBanner({ scope, onClear }: ScopeFilterBannerProps) {
  return (
    <div data-testid="change-analysis-scope-banner" style={banner}>
      <span style={{ color: "var(--text-muted)" }}>Filtering:</span>
      <span style={{ color: "var(--text-primary)", fontWeight: 600 }}>
        {describeScope(scope)}
      </span>
      <span style={{ flex: 1 }} />
      <button
        type="button"
        data-testid="change-analysis-scope-clear"
        onClick={onClear}
        style={clearButton}
      >
        Clear filter
      </button>
    </div>
  );
}

function describeScope(scope: ChangeAnalysisScope): string {
  if (scope.kind === "ext") return `.${scope.value} files`;
  if (scope.kind === "dir") return `${scope.value}/`;
  return `${scope.value} files`;
}

const banner: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "8px 12px",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderLeft: "3px solid var(--text-link, #2563eb)",
  borderRadius: 6,
  fontSize: 12,
};
const clearButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: 12,
};
