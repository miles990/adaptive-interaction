import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import CompanionApp from "./companion/CompanionApp";
import "./styles.css";

// One frontend bundle, two windows: the control center (default) and the
// desktop companion. In Tauri the window LABEL decides (query strings do not
// survive WebviewUrl::App paths); the ?window=companion query serves dev/E2E.
type TauriInternals = { metadata?: { currentWindow?: { label?: string } } };
const tauriLabel =
  (window as unknown as { __TAURI_INTERNALS__?: TauriInternals }).__TAURI_INTERNALS__?.metadata
    ?.currentWindow?.label;
const isCompanion =
  tauriLabel === "companion" ||
  new URLSearchParams(window.location.search).get("window") === "companion";

if (isCompanion) {
  document.documentElement.classList.add("companion-window");
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{isCompanion ? <CompanionApp /> : <App />}</React.StrictMode>
);
