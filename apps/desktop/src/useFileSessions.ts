import { type Dispatch, type SetStateAction, useCallback, useState } from "react";

import { createEmptyFileSession, type FileSessionState } from "./editor-session.js";

/**
 * Owns the per-stream file-session map (`streamId → FileSessionState`),
 * lifted out of App.tsx. The pure transforms live in editor-session.ts;
 * this hook just holds the state and removes the repetitive
 * `setFileSessions((prev) => ({ ...prev, [id]: reducer(prev[id] ??
 * createEmptyFileSession(), …) }))` wrapper that was duplicated ~28 times
 * in App.
 *
 * - `getFileSession(id)` — the stream's slice, or a fresh empty one.
 * - `mutateFileSession(id, fn)` — apply a reducer to that slice,
 *   auto-creating an empty session first.
 * - `fileSessions` / `setFileSessions` — the raw record + setter, for the
 *   few whole-map reads (persistence, the current-file memo) and any
 *   multi-key update.
 */
export interface FileSessionsHandle {
  fileSessions: Record<string, FileSessionState>;
  setFileSessions: Dispatch<SetStateAction<Record<string, FileSessionState>>>;
  getFileSession: (streamId: string) => FileSessionState;
  mutateFileSession: (streamId: string, fn: (session: FileSessionState) => FileSessionState) => void;
}

export function useFileSessions(): FileSessionsHandle {
  const [fileSessions, setFileSessions] = useState<Record<string, FileSessionState>>({});

  const getFileSession = useCallback(
    (streamId: string): FileSessionState => fileSessions[streamId] ?? createEmptyFileSession(),
    [fileSessions],
  );

  const mutateFileSession = useCallback(
    (streamId: string, fn: (session: FileSessionState) => FileSessionState) => {
      setFileSessions((prev) => ({
        ...prev,
        [streamId]: fn(prev[streamId] ?? createEmptyFileSession()),
      }));
    },
    [],
  );

  return { fileSessions, setFileSessions, getFileSession, mutateFileSession };
}
