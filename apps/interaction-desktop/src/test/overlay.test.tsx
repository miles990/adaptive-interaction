// 可信 host overlay：內容只來自 Rust 的 host-safety 事件、固定文案、圖示＋文字、
// 沒事就藏起來，而且絕不呼叫 api.*。

import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import * as fs from "node:fs";
import * as path from "node:path";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import OverlayApp, { HOST_SAFETY_EVENT } from "../overlay/OverlayApp";
import {
  deriveOverlayLines,
  isOverlayActive,
  OVERLAY_TEXT,
  type HostSafetyView,
} from "../overlay/hostSafety";
import { resolveWindowKind } from "../overlay/windowKind";

type Handler = (event: { event: string; id: number; payload: HostSafetyView }) => void;

const handlers: Record<string, Handler> = {};

function baseView(overrides: Partial<HostSafetyView> = {}): HostSafetyView {
  return {
    reachable: true,
    starting: false,
    estop: false,
    paused: false,
    micActive: false,
    cameraActive: false,
    sensors: [],
    at: "2026-09-02T00:00:00Z",
    ...overrides,
  };
}

/** 模擬 Rust `emit_to("overlay", "host-safety", view)`。`active` 由 Rust 算好帶過來。 */
function emitHostSafety(view: HostSafetyView) {
  const handler = handlers[HOST_SAFETY_EVENT];
  if (!handler) throw new Error("overlay did not subscribe to host-safety");
  const payload: HostSafetyView = { ...view, active: view.active ?? isOverlayActive(view) };
  act(() => handler({ event: HOST_SAFETY_EVENT, id: 1, payload }));
}

async function renderOverlay() {
  const utils = render(<OverlayApp />);
  await waitFor(() => expect(handlers[HOST_SAFETY_EVENT]).toBeDefined());
  return utils;
}

beforeEach(() => {
  for (const key of Object.keys(handlers)) delete handlers[key];
  vi.mocked(listen).mockReset();
  vi.mocked(listen).mockImplementation(async (name, cb) => {
    handlers[name as string] = cb as unknown as Handler;
    return () => {
      delete handlers[name as string];
    };
  });
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockResolvedValue(undefined);
});

describe("OverlayApp", () => {
  it("is hidden before any host event and when nothing is active", async () => {
    await renderOverlay();
    const root = screen.getByTestId("overlay-root");
    expect(root).toHaveAttribute("hidden");
    expect(root).toHaveAttribute("data-active", "false");
    expect(root).not.toBeVisible();

    emitHostSafety(baseView());
    expect(screen.getByTestId("overlay-root")).toHaveAttribute("hidden");
    expect(screen.queryByTestId("overlay-line-estop")).toBeNull();
    expect(screen.queryByTestId("overlay-line-mic")).toBeNull();
  });

  it("shows the fixed emergency-stop text with an icon", async () => {
    await renderOverlay();
    emitHostSafety(baseView({ estop: true }));
    const root = screen.getByTestId("overlay-root");
    expect(root).not.toHaveAttribute("hidden");
    expect(root).toHaveAttribute("data-active", "true");
    expect(root).toHaveAttribute("role", "status");
    expect(screen.getByTestId("overlay-text-estop")).toHaveTextContent("緊急停止中");
    const icon = screen.getByTestId("overlay-icon-estop");
    expect(icon).toHaveAttribute("aria-hidden", "true");
    expect(icon.textContent?.trim()).not.toBe("");
  });

  it("shows microphone use for the local mic and the iPhone mic-level sensor", async () => {
    await renderOverlay();
    emitHostSafety(
      baseView({
        micActive: true,
        sensors: [{ kind: "microphone", startedBy: "user", purpose: "click-to-listen" }],
      })
    );
    expect(screen.getByTestId("overlay-text-mic")).toHaveTextContent("麥克風使用中");
    expect(screen.getByTestId("overlay-detail-mic")).toHaveTextContent("user");

    // 手機麥克風音量：kind 不是 "microphone"，也必須顯示同一行固定文字。
    emitHostSafety(
      baseView({
        micActive: true,
        sensors: [{ kind: "iphone.mic-level", startedBy: "iphone:abc", purpose: "音量" }],
      })
    );
    expect(screen.getByTestId("overlay-text-mic")).toHaveTextContent("麥克風使用中");
    expect(screen.getByTestId("overlay-detail-mic")).toHaveTextContent("iphone:abc");
    expect(screen.queryByTestId("overlay-line-camera")).toBeNull();
  });

  it("shows camera use", async () => {
    await renderOverlay();
    emitHostSafety(
      baseView({ cameraActive: true, sensors: [{ kind: "camera", startedBy: "user" }] })
    );
    expect(screen.getByTestId("overlay-text-camera")).toHaveTextContent("攝影機使用中");
    expect(screen.getByTestId("overlay-icon-camera")).toHaveAttribute("aria-hidden", "true");
    expect(screen.queryByTestId("overlay-line-mic")).toBeNull();
  });

  it("shows Runtime offline, but not during the starting grace", async () => {
    await renderOverlay();
    emitHostSafety(baseView({ reachable: false }));
    expect(screen.getByTestId("overlay-text-offline")).toHaveTextContent("Runtime 離線");

    emitHostSafety(baseView({ reachable: false, starting: true }));
    expect(screen.getByTestId("overlay-root")).toHaveAttribute("hidden");
  });

  it("names unknown sensor kinds instead of staying silent", async () => {
    await renderOverlay();
    emitHostSafety(baseView({ sensors: [{ kind: "lidar", startedBy: "user" }] }));
    expect(screen.getByTestId("overlay-text-sensor")).toHaveTextContent("感測使用中：lidar");
  });

  it("stacks every active state, emergency stop first, never colour-only", async () => {
    await renderOverlay();
    emitHostSafety(
      baseView({
        estop: true,
        reachable: false,
        micActive: true,
        cameraActive: true,
        sensors: [{ kind: "microphone" }, { kind: "camera" }],
      })
    );
    const root = screen.getByTestId("overlay-root");
    const lines = Array.from(root.querySelectorAll(".overlay-line"));
    expect(lines.map((l) => l.getAttribute("data-testid"))).toEqual([
      "overlay-line-estop",
      "overlay-line-offline",
      "overlay-line-mic",
      "overlay-line-camera",
    ]);
    for (const line of lines) {
      const icon = line.querySelector(".overlay-icon");
      const text = line.querySelector(".overlay-text");
      expect(icon).not.toBeNull();
      expect(icon).toHaveAttribute("aria-hidden", "true");
      expect(icon?.textContent?.trim()).not.toBe("");
      expect(text?.textContent?.trim()).not.toBe("");
    }
  });

  it("hides again when the host reports everything clear", async () => {
    await renderOverlay();
    emitHostSafety(baseView({ estop: true }));
    expect(screen.getByTestId("overlay-root")).not.toHaveAttribute("hidden");
    emitHostSafety(baseView());
    expect(screen.getByTestId("overlay-root")).toHaveAttribute("hidden");
  });

  it("paused alone is informational and does not show", async () => {
    await renderOverlay();
    emitHostSafety(baseView({ paused: true }));
    expect(screen.getByTestId("overlay-root")).toHaveAttribute("hidden");
  });

  it("only attaches to the host and never calls api.* / status", async () => {
    await renderOverlay();
    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith("overlay_attach"));
    const commands = vi.mocked(invoke).mock.calls.map((call) => call[0]);
    expect(commands.every((cmd) => cmd === "overlay_attach")).toBe(true);
    // listen 只訂閱 host-safety。
    const subscribed = vi.mocked(listen).mock.calls.map((call) => call[0]);
    expect(subscribed).toEqual([HOST_SAFETY_EVENT]);
  });

  it("keeps working when the attach IPC is unavailable (e.g. dev browser)", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("no tauri"));
    await renderOverlay();
    emitHostSafety(baseView({ estop: true }));
    expect(screen.getByTestId("overlay-text-estop")).toHaveTextContent("緊急停止中");
  });

  it("does not import runtime/api/companion code (static guard)", () => {
    for (const file of ["src/overlay/OverlayApp.tsx", "src/overlay/hostSafety.ts"]) {
      const src = fs.readFileSync(path.resolve(file), "utf8");
      expect(src).not.toMatch(/from\s+["']\.\.\/api["']/);
      expect(src).not.toMatch(/from\s+["']\.\.\/transport["']/);
      expect(src).not.toMatch(/from\s+["']\.\.\/desktop["']/);
      expect(src).not.toMatch(/\.\.\/companion\//);
      expect(src).not.toMatch(/\.\.\/character\//);
      expect(src).not.toMatch(/api\.(status|presentation|character)/);
    }
  });
});

describe("hostSafety helpers", () => {
  it("isOverlayActive mirrors the Rust rule when `active` is absent", () => {
    expect(isOverlayActive(baseView())).toBe(false);
    expect(isOverlayActive(baseView({ estop: true }))).toBe(true);
    expect(isOverlayActive(baseView({ sensors: [{ kind: "microphone" }] }))).toBe(true);
    expect(isOverlayActive(baseView({ reachable: false }))).toBe(true);
    expect(isOverlayActive(baseView({ reachable: false, starting: true }))).toBe(false);
    expect(isOverlayActive(baseView({ paused: true }))).toBe(false);
    // Rust 算好的 active 優先。
    expect(isOverlayActive(baseView({ estop: true, active: false }))).toBe(false);
    expect(isOverlayActive(baseView({ active: true }))).toBe(true);
  });

  it("deriveOverlayLines uses fixed copy and a stable order", () => {
    const lines = deriveOverlayLines(
      baseView({
        estop: true,
        reachable: false,
        sensors: [
          { kind: "iphone.mic-level", startedBy: "iphone:1" },
          { kind: "camera", startedBy: "user" },
          { kind: "lidar" },
        ],
      })
    );
    expect(lines.map((l) => l.id)).toEqual(["estop", "offline", "mic", "camera", "sensor"]);
    expect(lines[0].text).toBe(OVERLAY_TEXT.estop);
    expect(lines[1].text).toBe(OVERLAY_TEXT.offline);
    expect(lines[2].text).toBe(OVERLAY_TEXT.mic);
    expect(lines[2].detail).toBe("iphone:1");
    expect(lines[3].text).toBe(OVERLAY_TEXT.camera);
    expect(lines[4].text).toBe("感測使用中：lidar");
    for (const line of lines) {
      expect(line.icon.trim()).not.toBe("");
      expect(line.text.trim()).not.toBe("");
    }
    expect(deriveOverlayLines(baseView())).toEqual([]);
    // Rust 的 micActive 旗標即使 sensors 為空也要畫麥克風那行。
    expect(deriveOverlayLines(baseView({ micActive: true })).map((l) => l.id)).toEqual(["mic"]);
  });

  it("resolveWindowKind routes by Tauri label first, then ?window=", () => {
    expect(resolveWindowKind("overlay", "")).toBe("overlay");
    expect(resolveWindowKind("companion", "?window=overlay")).toBe("companion");
    expect(resolveWindowKind("main", "")).toBe("main");
    expect(resolveWindowKind(undefined, "?window=overlay")).toBe("overlay");
    expect(resolveWindowKind(undefined, "?window=companion")).toBe("companion");
    expect(resolveWindowKind(undefined, "?window=other")).toBe("main");
    expect(resolveWindowKind(undefined, "")).toBe("main");
  });
});
