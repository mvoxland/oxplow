// Ambient declarations for globals the legacy UI code expects.
// `window.oxplowApi` was the Electron preload's injected facade;
// under Tauri it doesn't exist, but the typecheck still references
// it through `desktopApi()`. The shim throws at runtime — declarations
// here only quiet TS.

import type { DesktopApi } from "./legacy-ipc-contract";

declare global {
  interface Window {
    oxplowApi?: DesktopApi;
    oxplowDesktop?: { ready: boolean; isElectron?: boolean; [extra: string]: unknown };
  }
}

export {};
