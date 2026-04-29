// `window.oxplowDesktop` is a tiny gating flag a few legacy spots
// use to detect "are we running inside the desktop shell". Under
// Tauri the answer is always yes, but the type stays around until
// those spots are cleaned up.
//
// `window.oxplowApi` was the Electron preload's injected facade —
// it's gone now. Any module that needs the legacy adapter imports
// `legacyApi()` from `./api.js` instead.

declare global {
  interface Window {
    oxplowDesktop?: { ready: boolean; isElectron?: boolean; [extra: string]: unknown };
  }
}

export {};
