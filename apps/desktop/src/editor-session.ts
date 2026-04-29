// Legacy file-session types + helpers — kept for the editor pane
// until the open-file-state subsystem is ported.

export interface OpenFileState {
  path: string;
  content: string;
  dirty: boolean;
  caret: { line: number; column: number } | null;
  selection: {
    startLine: number;
    startColumn: number;
    endLine: number;
    endColumn: number;
  } | null;
  savedContent?: string;
  draftContent?: string;
  loading?: boolean;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

export interface FileSession {
  files: OpenFileState[];
  activePath: string | null;
  openOrder?: string[];
  selectedPath?: string | null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

/// Legacy alias preserved by the old runtime — same shape as
/// FileSession.
export type FileSessionState = FileSession;

export interface TerminalEvent {
  sessionId: string;
  kind: "data" | "exit" | "resize";
  data?: string;
  exitCode?: number;
  cols?: number;
  rows?: number;
  message?: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

// Stub file-session functions — runtime never executes; they exist
// to keep App.tsx etc. typechecking. Each will be ported as the
// open-file-state subsystem is rebuilt.
const NOT_PORTED = "file-session helpers are not yet ported to Tauri";

export function createEmptyFileSession(): FileSession {
  return { files: [], activePath: null };
}

// All file-session helpers accept any number of arguments. The
// runtime never actually executes them — they exist to keep the
// UI typechecking. Each will be ported as the open-file-state
// subsystem is rebuilt.
//
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Stub = (..._args: any[]) => FileSession;

export const openFileInSession: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const closeOpenFile: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const enforceOpenFileLimit: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const markFileSaved: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const removeOpenFiles: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const renameOpenFilePaths: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const reorderOpenFiles: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const selectOpenFile: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const setLoadedFileContent: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const setOpenFileLoading: Stub = () => {
  throw new Error(NOT_PORTED);
};
export const updateFileDraft: Stub = () => {
  throw new Error(NOT_PORTED);
};
