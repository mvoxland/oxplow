import type { CSSProperties } from "react";

/// View-state enum for the small per-agent status indicator. Mapped
/// from the bindings `AgentStatusState` ("running" → "working",
/// "awaiting_user" → "waiting") at the call site. Keeping the dot's
/// alphabet narrow lets future state names (idle/error/etc.) be
/// distinguished or hidden without touching every dot caller.
export type AgentStatusDotState = "working" | "waiting";

const COLORS: Record<AgentStatusDotState, string> = {
  working: "#fcd34d",
  waiting: "#fca5a5",
};

const LABELS: Record<AgentStatusDotState, string> = {
  working: "Working",
  waiting: "Waiting for input",
};

export function AgentStatusDot({
  status,
  size = 8,
}: {
  status: AgentStatusDotState;
  size?: number;
}) {
  const style: CSSProperties = {
    display: "inline-block",
    width: size,
    height: size,
    borderRadius: "50%",
    background: COLORS[status],
    flexShrink: 0,
    animation: status === "working" ? "oxplow-pulse 1.4s ease-in-out infinite" : undefined,
    boxShadow: status === "waiting" ? `0 0 0 2px rgba(252, 165, 165, 0.25)` : undefined,
  };
  return (
    <span
      style={style}
      title={LABELS[status]}
      aria-label={`Agent status: ${LABELS[status]}`}
      data-agent-status={status}
      data-agent-label={LABELS[status]}
    />
  );
}
