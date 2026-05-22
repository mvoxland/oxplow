import { expect, test } from "bun:test";
import { act, render } from "@testing-library/react";

import { openFileInSession, updateFileDraft } from "./editor-session.js";
import { type FileSessionsHandle, useFileSessions } from "./useFileSessions.js";

// Capture the hook handle from a rendered component so we can drive it.
function capture(): { current: FileSessionsHandle } {
  const ref: { current: FileSessionsHandle } = { current: null as never };
  function Probe() {
    ref.current = useFileSessions();
    return null;
  }
  render(<Probe />);
  return ref;
}

test("getFileSession returns a fresh empty session for an unknown stream", () => {
  const h = capture();
  const session = h.current.getFileSession("s-unknown");
  expect(session.selectedPath).toBeNull();
  expect(Object.keys(session.files)).toHaveLength(0);
});

test("mutateFileSession applies a reducer to the stream's slice", () => {
  const h = capture();
  act(() => {
    h.current.mutateFileSession("s-1", (s) => openFileInSession(s, "a.ts", "", true));
  });
  const session = h.current.getFileSession("s-1");
  expect(session.files["a.ts"]).toBeDefined();
  expect(h.current.fileSessions["s-1"]).toBe(session);
});

test("mutateFileSession isolates streams from each other", () => {
  const h = capture();
  act(() => {
    h.current.mutateFileSession("s-1", (s) => openFileInSession(s, "a.ts", "x", false));
    h.current.mutateFileSession("s-2", (s) => openFileInSession(s, "b.ts", "y", false));
  });
  expect(h.current.getFileSession("s-1").files["a.ts"]).toBeDefined();
  expect(h.current.getFileSession("s-1").files["b.ts"]).toBeUndefined();
  expect(h.current.getFileSession("s-2").files["b.ts"]).toBeDefined();
});

test("mutateFileSession composes successive reducers on the same slice", () => {
  const h = capture();
  act(() => {
    h.current.mutateFileSession("s-1", (s) => openFileInSession(s, "a.ts", "orig", false));
  });
  act(() => {
    h.current.mutateFileSession("s-1", (s) => updateFileDraft(s, "a.ts", "edited"));
  });
  expect(h.current.getFileSession("s-1").files["a.ts"]?.draftContent).toBe("edited");
});
