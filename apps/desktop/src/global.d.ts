// Globals carried over from the Electron-era preload shim. Both are
// scheduled for removal as the legacy components migrate off the
// `desktopApi()` helper in `api.ts`. `oxplowApi` is now populated
// lazily by `api.ts` on first access (mirrors `cachedAdapter`) so
// any direct `window.oxplowApi.*` call still works even though
// nothing installs it eagerly.

import type { DesktopApi } from "./legacy-ipc-contract";

declare global {
  interface Window {
    oxplowApi?: DesktopApi;
    oxplowDesktop?: { ready: boolean; isElectron?: boolean; [extra: string]: unknown };
  }
}

export {};
