import type { DiffSpec } from "./components/Diff/DiffPane.js";
import type { TabRef } from "./tabs/tabState.js";

/** Stable id for a diff tab. Keyed off the path + both side versions
 *  + label override so re-opening the same diff with a new revealLine
 *  reuses the existing tab. */
export function computeDiffId(spec: DiffSpec): string {
  const left =
    spec.leftVersion.kind === "disk"
      ? "disk"
      : spec.leftVersion.kind === "ref"
        ? `ref:${spec.leftVersion.ref}`
        : `snap:${spec.leftVersion.id}`;
  const right =
    spec.rightVersion.kind === "disk"
      ? "disk"
      : spec.rightVersion.kind === "ref"
        ? `ref:${spec.rightVersion.ref}`
        : `snap:${spec.rightVersion.id}`;
  const labelKey = spec.labelOverride ? `:${spec.labelOverride}` : "";
  return `diff:${left}:${right}:${spec.path}${labelKey}`;
}

/** Build the TabRef for a diff page from its spec. */
export function diffRef(spec: DiffSpec): TabRef {
  return {
    id: computeDiffId(spec),
    kind: "diff",
    payload: {
      path: spec.path,
      leftVersion: spec.leftVersion,
      rightVersion: spec.rightVersion,
      labelOverride: spec.labelOverride ?? null,
    },
  };
}
