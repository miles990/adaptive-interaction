// 可信 host overlay 視窗（label "overlay"）。
//
// 邊界（CPP README §9「隱藏感測指示／改寫安全固定文字」那一列）：
// - 內容**只**來自 Rust 的 `host-safety` 事件；這裡不呼叫 api.*、不讀 status、
//   不碰 transport，也不 import 任何角色／companion 程式碼。
// - 視窗由 Rust 建立為 click-through、置頂、不取焦點；React 只負責把固定文案畫出來。
// - 每一行都是圖示＋文字（圖示 aria-hidden），永遠不只靠顏色。
// - 唯一的 IPC 是 `overlay_attach`：告訴 host「listener 掛好了」，host 只會把
//   它快取的視圖重送一次（而且只接受 overlay 視窗呼叫）。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { deriveOverlayLines, isOverlayActive, type HostSafetyView } from "./hostSafety";
import "./overlay.css";

/** Rust `host_safety::HOST_SAFETY_EVENT`。 */
export const HOST_SAFETY_EVENT = "host-safety";

export default function OverlayApp() {
  const [view, setView] = useState<HostSafetyView | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    listen<HostSafetyView>(HOST_SAFETY_EVENT, (event) => {
      if (!disposed) setView(event.payload);
    })
      .then((un) => {
        if (disposed) {
          un();
          return;
        }
        unlisten = un;
        // listener 已就緒 → 請 host 重送快取的視圖（建立視窗時的那一次 emit 可能趕在前面）。
        return invoke("overlay_attach").catch(() => undefined);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const active = view !== null && isOverlayActive(view);
  const lines = active && view ? deriveOverlayLines(view) : [];

  return (
    <div
      className="overlay-root"
      data-testid="overlay-root"
      data-active={active ? "true" : "false"}
      hidden={!active}
      role="status"
      aria-live="polite"
    >
      {lines.map((line) => (
        <div
          key={line.id}
          className={`overlay-line overlay-line-${line.id}`}
          data-testid={`overlay-line-${line.id}`}
        >
          <span className="overlay-icon" aria-hidden="true" data-testid={`overlay-icon-${line.id}`}>
            {line.icon}
          </span>
          <span className="overlay-text" data-testid={`overlay-text-${line.id}`}>
            {line.text}
          </span>
          {line.detail ? (
            <span className="overlay-detail" data-testid={`overlay-detail-${line.id}`}>
              {line.detail}
            </span>
          ) : null}
        </div>
      ))}
    </div>
  );
}
