/**
 * Formatting helpers shared across pages and panels.
 *
 * Kept lib-style (no React, no DOM) so any module can pull them in
 * without dragging UI dependencies. Live here rather than in api.ts
 * to keep the api module focused on IPC wrappers.
 */

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/**
 * Short date+time label used in dashboard rows: "May 13, 14:32".
 * Falls back to the raw string if `Date` can't parse it.
 */
export function formatShortDateTime(input: string): string {
  try {
    return new Date(input).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return input;
  }
}

/**
 * Full date+time label used on detail-page headers: locale long form.
 * Falls back to the raw string if `Date` can't parse it.
 */
export function formatFullDateTime(input: string): string {
  try {
    return new Date(input).toLocaleString();
  } catch {
    return input;
  }
}
