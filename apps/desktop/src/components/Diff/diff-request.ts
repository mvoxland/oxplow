import type { FileVersion } from "../../file-version.js";

export interface DiffRequest {
  path: string;
  leftVersion: FileVersion;
  rightVersion: FileVersion;
  baseLabel: string;
}
