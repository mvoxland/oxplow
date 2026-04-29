import "monaco-editor/min/vs/editor/editor.main.css";
import "@xterm/xterm/css/xterm.css";
import { createRoot } from "react-dom/client";
import { App } from "./App.js";
import { installUiLogging, logUi } from "./logger.js";
import { installLegacyAdapter } from "./legacy-bridge.js";

// Install the legacy `window.oxplowApi` shim before React mounts so
// any module that calls `desktopApi()` during init finds it.
installLegacyAdapter();

installUiLogging();
logUi("info", "ui bootstrapping");

const el = document.getElementById("root")!;
createRoot(el).render(<App />);
