// CPP §12 `sprite`：舊 Character Pack（character-pack 1.0／1.1）相容層。
//
// 包住既有 SpriteRenderer（companion/renderer.ts），manifest 由 migratePackToManifest
// 產生：只宣告 sheet 真的有的東西（visual.expression variants = 動畫名；有 anchors 才
// 有 visual.gaze）。intent → 動畫走 spriteIntents.ts 的對照＋安全退階鏈；claim-completed
// 只點頭（frameSlice [0,1]），verified-success 且 truthState verified 才播完整 success。
// 回執：accepted → started → completed（第一輪播完或 durationHint 到期；由 tick(now) 推進，
// 不自帶 timer）；negotiated 為 unsupported 的 intent 回 unsupported。
// 輸入：宣告 input.click／drag／drop／text／fileDrop，但指標接線留給 host——host 透過
// emitInput() 把事件餵進來（Gateway 再正規化）。

import { SpriteRenderer, type PackManifest, type RendererBackend } from "../../companion/renderer";
import type { AdapterHost, AdapterInputEvent, CharacterAdapter, ReceiptSink } from "../adapter";
import { migratePackToManifest, resolveAssetUrl } from "../manifest";
import {
  CharacterManifest,
  Hello,
  IntentEnvelope,
  IntentResolution,
  Negotiate,
  PROTOCOL_VERSION,
} from "../protocol";
import { resolveSpriteAnimation, type SpriteResolution } from "../spriteIntents";

export interface SpriteAdapterOptions {
  pack: PackManifest;
  /** 同源資產根（例如 /packs/shu-standard）；sheet URL = assetBase + "/" + sheet。 */
  assetBase: string;
  canvas?: HTMLCanvasElement;
  /** 測試／其他後端可注入；省略且有 canvas 時建立 SpriteRenderer。 */
  renderer?: RendererBackend;
  scale?: number;
}

interface ActiveCommand {
  messageId: string;
  completeAt: number;
  sink: ReceiptSink;
  resolution: SpriteResolution;
}

export class SpriteCharacterAdapter implements CharacterAdapter {
  readonly manifest: CharacterManifest;
  private readonly pack: PackManifest;
  private readonly sheetUrl: string;
  private readonly canvas: HTMLCanvasElement | null;
  private renderer: RendererBackend | null;
  private readonly ownsRenderer: boolean;
  private readonly scale: number;
  private host: AdapterHost | null = null;
  private listeners = new Set<(e: AdapterInputEvent) => void>();
  private active: ActiveCommand | null = null;
  private lastPlayed: { messageId: string; resolution: SpriteResolution } | null = null;
  private disposed = false;
  private visible = true;
  private suspended = false;

  constructor(opts: SpriteAdapterOptions) {
    const migrated = migratePackToManifest(opts.pack, { assetBase: opts.assetBase });
    if (!migrated.ok) throw new Error(`sprite pack rejected: ${migrated.errors.join("; ")}`);
    this.manifest = migrated.manifest;
    this.pack = opts.pack;
    const sheet = this.manifest.assets.find((a) => a.id === "sheet");
    this.sheetUrl = sheet ? resolveAssetUrl(opts.assetBase, sheet) : `${opts.assetBase}/${opts.pack.sheet}`;
    this.canvas = opts.canvas ?? null;
    this.renderer = opts.renderer ?? null;
    this.ownsRenderer = !opts.renderer;
    this.scale = opts.scale ?? 1;
  }

  async initialize(host: AdapterHost): Promise<void> {
    this.host = host;
    if (!this.renderer && this.canvas) {
      this.renderer = new SpriteRenderer(this.canvas, this.pack, this.sheetUrl, this.scale);
    }
    this.renderer?.setReducedMotion(host.reducedMotion());
    this.renderer?.setAnimation("idle");
  }

  negotiate(hello: Hello): Negotiate {
    this.renderer?.setReducedMotion(hello.reducedMotion);
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
    if (this.canvas) this.canvas.style.visibility = "visible";
    if (!this.suspended) this.renderer?.resume?.();
  }

  hide(): void {
    this.visible = false;
    if (this.canvas) this.canvas.style.visibility = "hidden";
    // CPP §7：看不見就不畫、不排 rAF（與 ShuCharacterAdapter 的 stage.pause() 一致）。
    this.renderer?.pause?.();
  }

  suspend(): void {
    this.suspended = true;
    this.renderer?.setAnimation("idle");
    this.renderer?.pause?.();
  }

  resume(): void {
    this.suspended = false;
    if (this.lastPlayed) {
      this.renderer?.setAnimation(this.lastPlayed.resolution.animation, this.lastPlayed.resolution.frameSlice);
    }
    if (this.visible) this.renderer?.resume?.();
  }

  reconfigure(prefs: Record<string, unknown>): void {
    if (typeof prefs.reducedMotion === "boolean") this.renderer?.setReducedMotion(prefs.reducedMotion);
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
    const intent = resolution?.viaIntent ?? envelope.intent;
    const resolved = resolveSpriteAnimation(this.pack.animations, intent, {
      truthState: envelope.truthState,
      variant: envelope.presentationHints?.variant,
    });
    if (!resolved) {
      sink({ messageId: envelope.messageId, status: "unsupported", resolution: "unsupported", detail: "no animation" });
      return;
    }
    if (this.active && this.active.messageId !== envelope.messageId) {
      const old = this.active;
      this.active = null;
      old.sink({ messageId: old.messageId, status: "cancelled", reason: "replaced" });
    }
    sink({ messageId: envelope.messageId, status: "accepted" });
    this.renderer?.setAnimation(resolved.animation, resolved.frameSlice);
    this.lastPlayed = { messageId: envelope.messageId, resolution: resolved };
    // resolution 只能變差：走 fallback 鏈的動畫回 substituted。
    const reported: IntentResolution["resolution"] =
      resolved.direct ? (resolution?.resolution ?? "exact") : "substituted";
    sink({ messageId: envelope.messageId, status: "started", resolution: reported });
    const hint = envelope.durationHint?.ms;
    const ms = typeof hint === "number" && Number.isFinite(hint) && hint > 0 ? Math.min(hint, 60_000) : resolved.firstLoopMs;
    const now = this.host?.now() ?? 0;
    this.active = { messageId: envelope.messageId, completeAt: now + ms, sink, resolution: resolved };
  }

  tick(now: number): void {
    if (this.active && now >= this.active.completeAt) {
      const done = this.active;
      this.active = null;
      done.sink({ messageId: done.messageId, status: "completed" });
    }
  }

  cancel(messageId: string): void {
    if (this.active && this.active.messageId === messageId) {
      const old = this.active;
      this.active = null;
      this.renderer?.setAnimation("idle");
      old.sink({ messageId, status: "cancelled", reason: "cancel" });
    }
  }

  dispose(): void {
    this.disposed = true;
    this.active = null;
    this.listeners.clear();
    if (this.ownsRenderer) this.renderer?.destroy();
    this.renderer = null;
  }

  onInput(cb: (e: AdapterInputEvent) => void): () => void {
    this.listeners.add(cb);
    return () => {
      this.listeners.delete(cb);
    };
  }

  /** host 把指標／文字／檔案事件餵進來（本 adapter 不自己監聽 DOM）。 */
  emitInput(e: AdapterInputEvent): void {
    if (this.disposed) return;
    for (const cb of [...this.listeners]) cb(e);
  }

  /** 測試／host 檢視最後一次播放（動畫名＋幀切片）。 */
  lastAnimation(): { messageId: string; animation: string; frameSlice?: [number, number] } | null {
    if (!this.lastPlayed) return null;
    return {
      messageId: this.lastPlayed.messageId,
      animation: this.lastPlayed.resolution.animation,
      frameSlice: this.lastPlayed.resolution.frameSlice,
    };
  }
}
