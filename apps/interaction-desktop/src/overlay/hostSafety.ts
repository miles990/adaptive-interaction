// 可信 host overlay 的資料模型與固定文案。
//
// 這裡的型別鏡射 src-tauri/src/host_safety.rs 的 `HostSafetyView`（camelCase）。
// overlay 是「不信任角色 renderer」的最後一道指示：內容只來自 Rust 的
// `host-safety` 事件，文案固定、每一行都是「圖示＋文字」，永遠不只靠顏色。

export interface SensorView {
  kind: string;
  startedBy?: string;
  purpose?: string;
  autoStopAt?: string;
}

export interface HostSafetyView {
  reachable: boolean;
  /** 啟動寬限內（不可達但尚不算離線）。 */
  starting?: boolean;
  estop: boolean;
  paused: boolean;
  micActive: boolean;
  cameraActive: boolean;
  sensors: SensorView[];
  /**
   * 「離開了使用中清單、但沒有人確認它停了」的筆數。
   *
   * overlay **刻意不顯示**它：這一區只講「此刻正在發生」的事（緊急停止、正在
   * 感測、連不上），未解決停止是沒有結論的紀錄，它的家在狀態列與「連接與權限」。
   * 型別留著是為了跟 Rust 那一份保持鏡射，不是給這裡渲染用的。
   */
  unresolvedStops?: number;
  /** Rust 算好的「該不該顯示」；缺席時以同一規則在此端推導。 */
  active?: boolean;
  at: string;
}

export type OverlayLineId = "estop" | "offline" | "mic" | "camera" | "sensor";

export interface OverlayLine {
  id: OverlayLineId;
  /** 純裝飾 glyph（aria-hidden）；文字才是語意。 */
  icon: string;
  text: string;
  /** 次要說明（誰啟動、用途），可省略。 */
  detail?: string;
}

/** 固定安全文案：不可被角色 pack／adapter 改寫。 */
export const OVERLAY_TEXT = {
  estop: "緊急停止中",
  offline: "Runtime 離線",
  mic: "麥克風使用中",
  camera: "攝影機使用中",
  sensor: "感測使用中",
} as const;

export const OVERLAY_ICON: Record<OverlayLineId, string> = {
  estop: "⛔",
  offline: "⚠",
  mic: "🎙",
  camera: "📷",
  sensor: "📡",
};

/** 與 host_safety.rs 同一規則：本機 microphone 與手機 iphone.mic-level 都算麥克風。 */
export function isMicKind(kind: string): boolean {
  return kind === "microphone" || kind.includes("mic");
}

export function isCameraKind(kind: string): boolean {
  return kind.includes("camera");
}

/** 是否該顯示 overlay：estop ∨ 有感測 ∨（不可達 ∧ 非啟動寬限）。 */
export function isOverlayActive(view: HostSafetyView): boolean {
  if (typeof view.active === "boolean") return view.active;
  const sensors = view.sensors ?? [];
  return view.estop || sensors.length > 0 || (!view.reachable && !view.starting);
}

function describe(sensors: SensorView[]): string | undefined {
  const who = sensors
    .map((s) => s.startedBy?.trim())
    .filter((s): s is string => Boolean(s));
  if (who.length === 0) return undefined;
  return Array.from(new Set(who)).join("、");
}

/**
 * 把視圖展開成要畫的行（順序固定：estop → 離線 → 麥克風 → 攝影機 → 其他感測）。
 * 未知的感測 kind 也要有一行——感測不靜默。
 */
export function deriveOverlayLines(view: HostSafetyView): OverlayLine[] {
  const lines: OverlayLine[] = [];
  const sensors = view.sensors ?? [];
  if (view.estop) {
    lines.push({ id: "estop", icon: OVERLAY_ICON.estop, text: OVERLAY_TEXT.estop });
  }
  if (!view.reachable && !view.starting) {
    lines.push({ id: "offline", icon: OVERLAY_ICON.offline, text: OVERLAY_TEXT.offline });
  }
  const mics = sensors.filter((s) => isMicKind(s.kind));
  if (view.micActive || mics.length > 0) {
    lines.push({
      id: "mic",
      icon: OVERLAY_ICON.mic,
      text: OVERLAY_TEXT.mic,
      detail: describe(mics),
    });
  }
  const cameras = sensors.filter((s) => isCameraKind(s.kind));
  if (view.cameraActive || cameras.length > 0) {
    lines.push({
      id: "camera",
      icon: OVERLAY_ICON.camera,
      text: OVERLAY_TEXT.camera,
      detail: describe(cameras),
    });
  }
  const others = sensors.filter((s) => !isMicKind(s.kind) && !isCameraKind(s.kind));
  if (others.length > 0) {
    const kinds = Array.from(new Set(others.map((s) => s.kind))).join("、");
    lines.push({
      id: "sensor",
      icon: OVERLAY_ICON.sensor,
      text: `${OVERLAY_TEXT.sensor}：${kinds}`,
      detail: describe(others),
    });
  }
  return lines;
}
