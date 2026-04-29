// Legacy local-blame types — kept for the editor's blame margin
// types until the blame surface is ported to Rust.

export interface LocalBlameEntry {
  line: number;
  capturedAt: string;
  path: string;
  preview: string;
  kind: "local" | "git";
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  git?: any;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  workItem?: any;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}
