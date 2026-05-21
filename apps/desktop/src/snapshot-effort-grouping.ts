// Group a snapshot's actually-changed files by the effort(s) that claim
// them. The snapshot detail page makes the real diff the core: each
// changed file is attributed to the active effort(s) whose declared
// authorship (task_effort_file) includes it, with an "unclaimed" bucket
// for changes no active effort owns (formatters, parallel actors, or
// capture gaps) and a roster of efforts that were active but claimed
// none of these changes. A file claimed by several efforts appears
// under each, carrying that effort's declared change-kind and the other
// claimers' names.

/// A changed file from the snapshot diff (path + status).
export interface ChangedEntryLike {
  path: string;
  status: string;
}

/// An effort active at the snapshot plus its declared file claims.
export interface EffortClaimLike {
  effortId: string;
  title: string;
  files: { path: string; change: string }[];
}

export interface GroupedFile {
  entry: ChangedEntryLike;
  /// This effort's declared change-kind for the path (created/updated/
  /// deleted), or null in the unclaimed bucket.
  declaredChange: string | null;
  /// Titles of the OTHER efforts that also claim this changed file.
  alsoClaimedBy: string[];
}

export interface EffortGroup {
  effortId: string;
  title: string;
  files: GroupedFile[];
}

export interface GroupedChanges {
  byEffort: EffortGroup[];
  unclaimed: GroupedFile[];
  /// Efforts active at this snapshot that claim none of its changed
  /// files — surfaced compactly so "which efforts were happening" stays
  /// visible without dominating the diff.
  idleEffortIds: string[];
}

export function groupChangesByEffort(
  changed: ChangedEntryLike[],
  efforts: EffortClaimLike[],
): GroupedChanges {
  // path -> claimers (in effort order), each with its declared change.
  const claimersByPath = new Map<string, { effortId: string; title: string; change: string }[]>();
  for (const e of efforts) {
    for (const f of e.files) {
      const arr = claimersByPath.get(f.path) ?? [];
      arr.push({ effortId: e.effortId, title: e.title, change: f.change });
      claimersByPath.set(f.path, arr);
    }
  }

  const filesByEffort = new Map<string, GroupedFile[]>();
  const unclaimed: GroupedFile[] = [];
  for (const entry of changed) {
    const claimers = claimersByPath.get(entry.path) ?? [];
    if (claimers.length === 0) {
      unclaimed.push({ entry, declaredChange: null, alsoClaimedBy: [] });
      continue;
    }
    for (const c of claimers) {
      const alsoClaimedBy = claimers.filter((x) => x.effortId !== c.effortId).map((x) => x.title);
      const arr = filesByEffort.get(c.effortId) ?? [];
      arr.push({ entry, declaredChange: c.change, alsoClaimedBy });
      filesByEffort.set(c.effortId, arr);
    }
  }

  // Preserve the caller's effort order; a group only appears if it owns
  // ≥1 changed file, otherwise the effort is "idle" here.
  const byEffort: EffortGroup[] = [];
  const idleEffortIds: string[] = [];
  for (const e of efforts) {
    const files = filesByEffort.get(e.effortId);
    if (files && files.length > 0) {
      byEffort.push({ effortId: e.effortId, title: e.title, files });
    } else {
      idleEffortIds.push(e.effortId);
    }
  }

  return { byEffort, unclaimed, idleEffortIds };
}
