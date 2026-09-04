// CPP §12 `shape`：第二個 Reference Character（`ref-shape`）——一個幾何圓形。
//
// 存在的理由是「證明協定不綁角色」：它跟 rig、sprite、文字角色沒有任何共用程式，
// 加它也沒有改到協定核心。它只做三件事：
//   - 顏色跟著 intent 家族（安靜／注意／玩耍／工作）；
//   - `play` 縮放脈衝一次、`notice` 輕微位移、其餘（rest／idle／…）靜止；
//   - Reduced Motion 時完全不動，只變色。
// 不支援 audio／gaze／particles／遊玩；安全 intent 不宣告，依 CPP 規則由 Gateway
// 落到可信 `system.text`（呈現層對安全訊息沒有否決權，adapter 也不假裝演過）。
//
// 回執：accepted → started → completed；有 durationHint.ms 時等 tick(now) 到期才 completed
// （不自帶 timer、不自帶 rAF——所有動畫都是 CSS class，dispose 後不留任何 handle）。

import type { AdapterHost, AdapterInputEvent, CharacterAdapter, ReceiptSink } from "../adapter";
import { validateCharacterManifest } from "../manifest";
import { presentedIntent } from "../negotiate";
import {
  CharacterIntent,
  CharacterManifest,
  Hello,
  IntentEnvelope,
  IntentResolution,
  Negotiate,
  PROTOCOL_VERSION,
} from "../protocol";

/** 這個角色會演的 intent（其餘 20 − 9 = 11 個安全 intent 一律落 system.text）。 */
export const SHAPE_INTENTS = [
  "idle",
  "notice",
  "acknowledge",
  "think",
  "work",
  "greet",
  "play",
  "rest",
  "sleep",
] as const;

/** manifest 宣告的 `visual.expression` 變體（含安全 intent `wait` 的名字，供協商參考）。 */
export const SHAPE_EXPRESSION_VARIANTS = [
  "idle",
  "notice",
  "play",
  "rest",
  "work",
  "think",
  "acknowledge",
  "wait",
  "greet",
  "sleep",
] as const;

/** 動作只有三種：靜止、脈衝一次、輕微位移。 */
export type ShapeMotion = "still" | "pulse" | "nudge";

/** intent 家族 → 顏色（有界的固定表；adapter 不會臨時算色）。 */
const SHAPE_COLORS: Readonly<Record<string, string>> = {
  calm: "#5b6b7a",
  alert: "#2f7fd1",
  play: "#d1552f",
  work: "#2f9d76",
  safety: "#8a8f98",
};

function familyOf(intent: CharacterIntent): keyof typeof SHAPE_COLORS {
  switch (intent) {
    case "idle":
    case "rest":
    case "sleep":
      return "calm";
    case "notice":
    case "acknowledge":
    case "greet":
      return "alert";
    case "play":
      return "play";
    case "work":
    case "think":
      return "work";
    default:
      // 安全 intent 走 system.text；真的收到也只用中性色，絕不演成慶祝。
      return "safety";
  }
}

function motionOf(intent: CharacterIntent, reducedMotion: boolean): ShapeMotion {
  if (reducedMotion) return "still";
  if (intent === "play") return "pulse";
  if (intent === "notice") return "nudge";
  return "still";
}

export interface ShapeRenderedState {
  readonly messageId: string;
  readonly intent: CharacterIntent;
  readonly color: string;
  readonly motion: ShapeMotion;
  /** Gateway 協商出的 resolution（adapter 只轉述，不升級）。 */
  readonly resolution: IntentResolution["resolution"];
}

export interface ShapeAdapterOptions {
  /** DOM 宿主；省略時只用 onRender（headless 測試）。 */
  container?: HTMLElement;
  onRender?: (state: ShapeRenderedState | null) => void;
}

/** 純資料建構 manifest（bundled `/characters/ref-shape/manifest.json` 用同一份定義）。 */
export function buildShapeCharacterManifest(): CharacterManifest {
  const raw = {
    schemaVersion: "1.0",
    characterId: "ref-shape",
    displayName: { "zh-TW": "參考形狀", en: "Reference Shape" },
    author: "adaptive-interaction",
    description: {
      "zh-TW":
        "第二個 Reference Character：一個幾何圓形。顏色隨 intent 家族改變，play 縮放脈衝一次、notice 輕微位移，其餘靜止。沒有音訊、沒有視線、沒有粒子；安全 intent 一律落到可信系統文字。",
      en: "The second reference character: one geometric circle. Colour follows the intent family, play pulses once, notice nudges, everything else rests. No audio, gaze or particles; safety intents always fall back to trusted system text.",
    },
    version: "1.0.0",
    adapterKind: "in-process",
    entrypoint: { kind: "builtin", id: "shape" },
    assets: [],
    capabilities: {
      "visual.presence": {
        supported: true,
        interruptible: true,
        resumable: true,
        qualityLevel: "minimal",
        reducedMotionBehavior: "static",
      },
      "visual.expression": {
        supported: true,
        maxConcurrent: 1,
        interruptible: true,
        resumable: false,
        variants: [...SHAPE_EXPRESSION_VARIANTS],
        durationRange: { minMs: 0, maxMs: 4000 },
        qualityLevel: "minimal",
        reducedMotionBehavior: "static",
      },
    },
    inputCapabilities: { "input.click": { supported: true } },
    channels: ["transform", "expression"],
    states: ["shape"],
    intents: [...SHAPE_INTENTS],
    variants: [],
    locales: ["zh-TW", "en"],
    securityRequirements: {
      network: false,
      executable: false,
      fileAccess: "none",
      audioOutput: false,
      microphone: false,
      camera: false,
    },
    resourceLimits: { maxAssetBytes: 0, maxConcurrentCommands: 1, maxQueue: 16, maxFps: 30 },
    fallbacks: {},
    compatibility: { protocol: "1.x", runtime: ">=0.6.0" },
  };
  const v = validateCharacterManifest(raw);
  if (!v.ok) throw new Error(`shape manifest invalid: ${v.errors.join("; ")}`);
  return v.manifest;
}

interface Active {
  messageId: string;
  completeAt: number | null;
  sink: ReceiptSink;
}

/** 最小幾何角色 adapter：一個圓，沒有 timer、沒有 rAF、沒有音訊。 */
export class ShapeCharacterAdapter implements CharacterAdapter {
  readonly manifest: CharacterManifest;
  private host: AdapterHost | null = null;
  private readonly container: HTMLElement | null;
  private readonly onRender: ((state: ShapeRenderedState | null) => void) | null;
  private shapeEl: HTMLElement | null = null;
  private domCleanup: (() => void) | null = null;
  private listeners = new Set<(e: AdapterInputEvent) => void>();
  private active: Active | null = null;
  private current: ShapeRenderedState | null = null;
  private visible = true;
  private suspended = false;
  private disposed = false;

  constructor(opts: ShapeAdapterOptions = {}) {
    this.manifest = buildShapeCharacterManifest();
    this.container = opts.container ?? null;
    this.onRender = opts.onRender ?? null;
  }

  async initialize(host: AdapterHost): Promise<void> {
    this.host = host;
    if (this.container && typeof document !== "undefined") {
      const root = document.createElement("div");
      root.setAttribute("data-cpp-shape-character", this.manifest.characterId);
      root.setAttribute("role", "img");
      root.setAttribute("aria-label", this.manifest.displayName["zh-TW"] ?? this.manifest.characterId);
      const shape = document.createElement("div");
      shape.setAttribute("data-cpp-shape", "circle");
      root.appendChild(shape);
      const onClick = () => this.emitInput({ kind: "character.clicked", payload: {} });
      root.addEventListener("click", onClick);
      this.container.appendChild(root);
      this.shapeEl = shape;
      this.domCleanup = () => {
        root.removeEventListener("click", onClick);
        root.remove();
      };
    }
    this.paint();
  }

  negotiate(hello: Hello): Negotiate {
    void hello;
    return {
      type: "negotiate",
      protocolVersion: PROTOCOL_VERSION,
      characterId: this.manifest.characterId,
      manifestVersion: this.manifest.version,
      capabilities: this.manifest.capabilities,
      inputCapabilities: this.manifest.inputCapabilities,
      channels: this.manifest.channels,
      intents: this.manifest.intents,
      variants: this.manifest.variants.map((v) => v.id),
      generation: 0,
      fallbacks: this.manifest.fallbacks,
    };
  }

  show(): void {
    this.visible = true;
    this.paint();
  }

  hide(): void {
    this.visible = false;
    this.paint();
  }

  suspend(): void {
    this.suspended = true;
    this.paint();
  }

  resume(): void {
    this.suspended = false;
    this.paint();
  }

  reconfigure(prefs: Record<string, unknown>): void {
    // 幾何角色沒有偏好可調（沒有配色、沒有場景、沒有玩具）；保留呼叫以符合契約。
    void prefs;
    this.paint();
  }

  perform(envelope: IntentEnvelope, sink: ReceiptSink, resolution?: IntentResolution): void {
    if (this.disposed) {
      sink({ messageId: envelope.messageId, status: "failed", resolution: "failed", detail: "adapter disposed" });
      return;
    }
    if (resolution?.resolution === "unsupported") {
      sink({ messageId: envelope.messageId, status: "unsupported", resolution: "unsupported" });
      return;
    }
    // 被新的一則取代：舊的（若還在等 durationHint）算 cancelled。
    if (this.active && this.active.messageId !== envelope.messageId) {
      const old = this.active;
      this.active = null;
      old.sink({ messageId: old.messageId, status: "cancelled", resolution: "exact", reason: "replaced" });
    }
    sink({ messageId: envelope.messageId, status: "accepted" });
    // 安全語意以 envelope.intent 為準：非安全的 viaIntent 不能改寫安全 intent 的呈現。
    const intent = presentedIntent(envelope.intent, resolution?.viaIntent);
    const reduced = this.host?.reducedMotion() === true;
    this.current = {
      messageId: envelope.messageId,
      intent,
      color: SHAPE_COLORS[familyOf(intent)] ?? SHAPE_COLORS.safety,
      motion: motionOf(intent, reduced),
      resolution: resolution?.resolution ?? "exact",
    };
    this.paint();
    sink({ messageId: envelope.messageId, status: "started", resolution: resolution?.resolution ?? "exact" });
    const ms = envelope.durationHint?.ms;
    if (typeof ms === "number" && Number.isFinite(ms) && ms > 0 && this.host) {
      this.active = { messageId: envelope.messageId, completeAt: this.host.now() + Math.min(ms, 4_000), sink };
      return;
    }
    this.active = null;
    sink({ messageId: envelope.messageId, status: "completed", resolution: resolution?.resolution ?? "exact" });
  }

  tick(now: number): void {
    if (this.active && this.active.completeAt !== null && now >= this.active.completeAt) {
      const done = this.active;
      this.active = null;
      done.sink({ messageId: done.messageId, status: "completed" });
    }
  }

  cancel(messageId: string): void {
    if (this.active && this.active.messageId === messageId) {
      const old = this.active;
      this.active = null;
      old.sink({ messageId, status: "cancelled", reason: "cancel" });
    }
    if (this.current?.messageId === messageId) {
      this.current = null;
      this.paint();
    }
  }

  dispose(): void {
    this.disposed = true;
    this.active = null;
    this.current = null;
    this.listeners.clear();
    this.domCleanup?.();
    this.domCleanup = null;
    this.shapeEl = null;
    this.onRender?.(null);
  }

  onInput(cb: (e: AdapterInputEvent) => void): () => void {
    this.listeners.add(cb);
    return () => {
      this.listeners.delete(cb);
    };
  }

  /** 測試／host 檢視目前的圓是什麼樣子。 */
  currentState(): ShapeRenderedState | null {
    return this.current;
  }

  private emitInput(e: AdapterInputEvent) {
    if (this.disposed) return;
    for (const cb of [...this.listeners]) cb(e);
  }

  private paint() {
    const shown = this.visible && !this.suspended ? this.current : null;
    if (this.shapeEl) {
      this.shapeEl.setAttribute("data-intent", shown?.intent ?? "");
      this.shapeEl.setAttribute("data-motion", shown?.motion ?? "still");
      this.shapeEl.style.background = shown?.color ?? "transparent";
    }
    this.onRender?.(shown);
  }
}
