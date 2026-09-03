import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import CompanionApp from "./companion/CompanionApp";
import OverlayApp from "./overlay/OverlayApp";
import { resolveWindowKind } from "./overlay/windowKind";
import "./styles.css";

// One frontend bundle, three windows: the control center (default), the
// desktop companion, and the trusted host safety overlay. In Tauri the window
// LABEL decides (query strings do not survive WebviewUrl::App paths); the
// ?window=companion / ?window=overlay query serves dev/E2E.
type TauriInternals = { metadata?: { currentWindow?: { label?: string } } };
const tauriLabel =
  (window as unknown as { __TAURI_INTERNALS__?: TauriInternals }).__TAURI_INTERNALS__?.metadata
    ?.currentWindow?.label;
const windowKind = resolveWindowKind(tauriLabel, window.location.search);

if (windowKind === "companion") {
  document.documentElement.classList.add("companion-window");
} else if (windowKind === "overlay") {
  document.documentElement.classList.add("overlay-window");
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {windowKind === "overlay" ? (
      <OverlayApp />
    ) : windowKind === "companion" ? (
      <CompanionApp />
    ) : (
      <App />
    )}
  </React.StrictMode>
);
