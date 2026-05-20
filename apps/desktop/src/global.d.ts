/// <reference types="vite/client" />

// The Electron preload globals (`window.oxplowApi`, `window.oxplowDesktop`)
// are gone after the Tauri switch. Any module that needs the desktop
// adapter imports `desktopBridge()` from `./api.js` instead.

export {};
