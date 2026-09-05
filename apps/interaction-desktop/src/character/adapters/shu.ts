// CPP §12 `shu-rig`：小樞 v3 參數化 rig＋遊玩場的 Reference Adapter（in-process）。
//
// 包住既有 StageRenderer；intent → 表情走 shuTables.ts；演出的混音（優先階梯、
// 緊急凍結、拖曳持續、表演搶佔）走 machine.ts 的 MixerPort：
//   - host（CompanionApp）注入自己的 machine（本機互動／Director 也在同一台機器上競爭）；
//   - 沒注入時用內建 LocalMixer（自己驅動 stage），讓 adapter 可獨立測試。
// 回執由時間軸驅動：accepted → started →（transient 到期或 durationHint 到）completed；
// 被更高優先的本機演出擠掉 → cancelled{reason:"preempted"}；cancel → 回待機。
// hide／suspend 停 rAF 與物理（StageRenderer.pause）；reconfigure 套名字／場景／
// 開關／tuning／使魔／配色。truthState 只讀不改：綠勾只在 Runtime 給 verified 時。

import {
  machineEventForAnimation,
  MachineEvent,
  MachineState,
  MixerPort,
  pose,
  reduce,
  Transient,
} from "../../companion/machine";
import { EXPRESSIONS, OFFICIAL_36 } from "../../companion/rig/expressions";
import { drawExpressionPreview } from "../../companion/rig/renderer";
import { machineStageFlags, STAGE_SCENES, StageRenderer } from "../../companion/rig/stage";
import { DEFAULT_TUNING, PersonalityTuning } from "../../companion/personality";
import { validateRigManifest, type RigManifest } from "../../companion/rig/renderer";
import type { AdapterPersona } from "../adapterRegistry";
import type { AdapterHost, AdapterInputEvent, CharacterAdapter, GameplayExtension, HitRect, PointerInput, ReceiptSink } from "../adapter";
import {
  displayNameOf,
  finalizeMigratedManifest,
  migratePackToManifest,
  MigrationRegistry,
  normalizeLegacyVersion,
  supportedCapability,
  type MigrationContext,
  type MigrationResult,
  type PackMigrator,
} from "../manifest";
import { presentedIntent } from "../negotiate";
import {
  CapabilityDecl,
  CHARACTER_INTENTS,
  CharacterManifest,
  Hello,
  IntentEnvelope,
  IntentResolution,
  LIMITS,
  LocalizedText,
  Negotiate,
  PROTOCOL_VERSION,
  SEMANTIC_CHANNELS,
} from "../protocol";
import {
  isShuToyKind,
  SHU_DIRECTOR_TABLES,
  SHU_EVENT_ART,
  SHU_LANDING,
  SHU_TOYS,
  SHU_VARIANT_WEIGHTS,
  shuEnterMs,
  shuExpressionPlan,
  shuNaturalDurationMs,
  type ShuExpressionPlan,
} from "./shuTables";

/** 這個 adapter 的 in-process builtin entrypoint id（host 白名單裡的同一個字串）。 */
export const SHU_RIG_ENTRYPOINT_ID = "shu-rig";

/** 沒有 bundled manifest 時的 legacy rig pack（character-rig 2.0）——rigPackMigrator 會產生完整能力集。 */
const DEFAULT_LEGACY_RIG = {
  schemaVersion: "2.0",
  kind: "character-rig",
  id: "shu-maid",
  name: { "zh-TW": "小樞", en: "Shu" },
  palette: "maid-classic",
  version: "3.0.0",
  author: "adaptive-interaction",
};

// ---------------------------------------------------------------------------
// 給設定頁用的公開表（Reference Adapter 自己宣告的選項）
//
// 角色頁只認這幾個匯出：表情名、配色 id、說話風格 id 都留在 adapter 裡，頁面不再直接
// 讀 rig 內部表，也不寫死任何屬於這個參考角色的字串（CLAUDE.md 呈現層主權）。
// ---------------------------------------------------------------------------

/** 角色頁的表情預覽清單（官方 36 格：id＋人話標籤，順序即畫面順序）。 */
export function previewExpressions(): { id: string; label: string }[] {
  return OFFICIAL_36.map((id) => ({ id, label: EXPRESSIONS[id]?.label ?? id }));
}

/**
 * 這個角色的配色 variants（id ＋ 雙語顯示名，第一個是預設值）。
 * 鏡射 Rust `interaction_character_shu::ShuRigPack::VARIANTS`；協定核心不認得這些名字。
 */
export const SHU_RIG_VARIANTS: readonly { readonly id: string; readonly displayName: LocalizedText }[] = [
  { id: "maid-classic", displayName: { "zh-TW": "經典", en: "Classic" } },
  { id: "maid-dusk", displayName: { "zh-TW": "暮色", en: "Dusk" } },
  { id: "maid-sakura", displayName: { "zh-TW": "櫻花", en: "Sakura" } },
];

/** 這個角色提供的配色 id（＝ manifest variants；host 用它決定 `palette` 別名要不要送）。 */
export const SHU_RIG_PALETTES: readonly string[] = SHU_RIG_VARIANTS.map((v) => v.id);

/** 舊 pack 沒宣告 palette、或宣告了未知名稱時的預設值。 */
export const DEFAULT_SHU_RIG_PALETTE = "maid-classic";

/** `value` 是不是已知配色（未知名稱不猜、不接受）。 */
export function isShuRigPalette(value: unknown): value is string {
  return typeof value === "string" && SHU_RIG_PALETTES.includes(value);
}

/** 這個角色提供的配色（第一個是預設值）——設定頁用的人話標籤。 */
export const PALETTES: { id: string; label: string }[] = SHU_RIG_VARIANTS.map((v) => ({
  id: v.id,
  label: v.displayName["zh-TW"] ?? v.id,
}));

/**
 * 說話風格。`followsName` 的項目由頁面把目前角色的名字接在前面
 * （名字一律來自 useCharacterName，不由 adapter 決定）。
 *
 * 這是**唯一**一份：host 的設定頁與設定檔匯入驗證都改讀 `SHU_META.personas`
 * （M2 §3.4；以前 `companion/settingsTransfer.ts` 另外硬編了同樣的兩個 id，
 * 而且拿它當全域白名單，不管那份 JSON 說自己屬於哪個角色）。
 */
export const PERSONAS: readonly AdapterPersona[] = [
  { id: "persona-shu", label: "預設", followsName: true },
  { id: "persona-navigator", label: "導航員（世界觀範例）", followsName: false },
];

/** 預覽一格表情：與桌面角色同一套 rig 程式即時繪製，不是靜態圖片。 */
export function drawPreviewExpression(
  ctx: CanvasRenderingContext2D,
  expressionId: string,
  palette: string,
  size: number
): void {
  drawExpressionPreview(ctx, expressionId, palette, size);
}

// ---------------------------------------------------------------------------
// CPP §2.2／§12：這個角色的能力集與 `character-rig` 2.0 遷移
//
// v0.6.0 strangler：以上全部曾經住在協定核心（character/manifest.ts）。核心現在只認識
// 通用 sprite pack；rig 2.0 由這裡實作 PackMigrator，由 host（character/adapters/index.ts）
// 註冊。鏡射 Rust 的 `interaction_character_shu::{ShuRigPack, RigPackMigrator}`。
// ---------------------------------------------------------------------------

/** 這個角色宣告的輸入能力（CPP §3.1 的 7 個 input.*）。 */
const SHU_INPUT_CAPABILITIES = [
  "input.click",
  "input.hover",
  "input.drag",
  "input.drop",
  "input.pointerProximity",
  "input.text",
  "input.fileDrop",
] as const;

/** CPP §12 `shu-rig` 的完整能力集（全部 visual.*、audio.speech／effect、input.*、multiCharacter、scene、rollCall、gameplay.*）。 */
export function shuRigCapabilities(): {
  capabilities: Record<string, CapabilityDecl>;
  inputCapabilities: Record<string, CapabilityDecl>;
} {
  const rm = { interruptible: true, resumable: true, qualityLevel: "full" as const };
  const capabilities: Record<string, CapabilityDecl> = {
    "visual.presence": supportedCapability({ ...rm, reducedMotionBehavior: "static" }),
    "visual.pose": supportedCapability({ ...rm, reducedMotionBehavior: "static", maxConcurrent: 1 }),
    "visual.expression": supportedCapability({
      ...rm,
      reducedMotionBehavior: "static",
      maxConcurrent: 1,
      variants: Object.keys(EXPRESSIONS),
      durationRange: { minMs: 100, maxMs: LIMITS.durationMaxMs },
    }),
    "visual.gaze": supportedCapability({ ...rm, reducedMotionBehavior: "reduced" }),
    "visual.locomotion": supportedCapability({ ...rm, reducedMotionBehavior: "disabled" }),
    "visual.overlay": supportedCapability({ ...rm, reducedMotionBehavior: "reduced" }),
    "visual.particles": supportedCapability({ ...rm, reducedMotionBehavior: "disabled" }),
    "visual.prop": supportedCapability({ ...rm, reducedMotionBehavior: "static" }),
    "visual.textBubble": supportedCapability({ ...rm, reducedMotionBehavior: "unchanged", maxConcurrent: 1 }),
    "audio.speech": supportedCapability({ interruptible: true, resumable: false, requiresAudio: true, reducedMotionBehavior: "unchanged" }),
    "audio.effect": supportedCapability({ interruptible: true, resumable: false, requiresAudio: true, reducedMotionBehavior: "unchanged" }),
    multiCharacter: supportedCapability({ reducedMotionBehavior: "unchanged" }),
    scene: supportedCapability({ reducedMotionBehavior: "reduced" }),
    rollCall: supportedCapability({ reducedMotionBehavior: "reduced", durationRange: { minMs: 500, maxMs: 20_000 } }),
    "gameplay.toys": supportedCapability({ reducedMotionBehavior: "disabled" }),
    "gameplay.autonomy": supportedCapability({ reducedMotionBehavior: "reduced" }),
  };
  const inputCapabilities: Record<string, CapabilityDecl> = {};
  for (const id of SHU_INPUT_CAPABILITIES) inputCapabilities[id] = supportedCapability();
  return { capabilities, inputCapabilities };
}

/** 這個角色接手的舊 pack `kind` 與 `schemaVersion`。 */
export const RIG_PACK_KIND = "character-rig";
export const RIG_PACK_SCHEMA_VERSION = "2.0";

/**
 * `character-rig` 2.0 → CPP manifest。純函式：不讀檔、不執行 entrypoint、不改寫使用者設定；
 * 未知配色不猜（由既有的 rig 驗證器擋下）。
 */
export const rigPackMigrator: PackMigrator = {
  kind: RIG_PACK_KIND,
  schemaVersions: [RIG_PACK_SCHEMA_VERSION],
  migrate(legacy: Record<string, unknown>, ctx: MigrationContext): MigrationResult {
    const issues = validateRigManifest(legacy);
    if (issues.length > 0) return { ok: false, errors: issues.map((i) => `legacy rig: ${i}`) };
    const rig = legacy as unknown as RigManifest;
    // 自己的配色排第一，其餘依宣告順序（未知名稱進不來：驗證器已擋）。
    const ordered = [rig.palette, ...SHU_RIG_PALETTES.filter((p) => p !== rig.palette)];
    const byId = new Map(SHU_RIG_VARIANTS.map((v) => [v.id, v.displayName]));
    const { capabilities, inputCapabilities } = shuRigCapabilities();
    const candidate: Record<string, unknown> = {
      schemaVersion: "1.0",
      characterId: rig.id,
      displayName: rig.name,
      ...(rig.author ? { author: rig.author } : {}),
      ...(rig.description ? { description: rig.description } : {}),
      version: normalizeLegacyVersion(rig.version),
      adapterKind: "in-process",
      entrypoint: { kind: "builtin", id: SHU_RIG_ENTRYPOINT_ID },
      assets: [],
      capabilities,
      inputCapabilities,
      channels: [...SEMANTIC_CHANNELS],
      states: Object.keys(EXPRESSIONS),
      intents: [...CHARACTER_INTENTS],
      variants: ordered.map((id) => ({ id, displayName: byId.get(id) ?? { en: id } })),
      locales: Object.keys(rig.name),
      pronouns: { "zh-TW": "她", en: "she" },
      securityRequirements: {
        network: false,
        executable: false,
        fileAccess: "none",
        audioOutput: true,
        microphone: false,
        camera: false,
      },
      resourceLimits: { maxAssetBytes: 8 * 1024 * 1024, maxConcurrentCommands: 4, maxQueue: 32, maxFps: 60 },
      fallbacks: {},
      compatibility: { protocol: "1.x", runtime: ">=0.5.0" },
      legacy: { kind: RIG_PACK_KIND, schemaVersion: rig.schemaVersion, palette: rig.palette },
    };
    return finalizeMigratedManifest(candidate, RIG_PACK_KIND, ctx);
  },
};

/** 只認得自己舊格式的 registry：adapter 自建 manifest 時用，不依賴 host 的載入順序。 */
let ownRegistry: MigrationRegistry | null = null;
function shuMigrationRegistry(): MigrationRegistry {
  if (!ownRegistry) ownRegistry = new MigrationRegistry().register(rigPackMigrator);
  return ownRegistry;
}

// ---------------------------------------------------------------------------
// host 接線用的配色 helper（原本住在 companion/gatewayWiring.ts）
// ---------------------------------------------------------------------------

function plainObject(v: unknown): Record<string, unknown> | null {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

/** bundled manifest 的 legacy.palette，或第一個 variant；都沒有就用預設配色。 */
export function rigPaletteFor(manifest: CharacterManifest): string {
  const legacy = (manifest as unknown as { legacy?: { palette?: unknown } }).legacy;
  if (legacy && typeof legacy.palette === "string") return legacy.palette;
  return manifest.variants[0]?.id ?? DEFAULT_SHU_RIG_PALETTE;
}

/**
 * 匯入角色的初始配色：x-legacy／legacy.palette → preferencesSchema 的 variant／palette
 * 預設值 → variants[0] → 預設配色。只接受宣告過的配色名（未知名稱不猜）。
 */
export function rigPaletteForImported(manifest: CharacterManifest | null): string {
  if (!manifest) return DEFAULT_SHU_RIG_PALETTE;
  for (const key of ["x-legacy", "legacy"]) {
    const ext = plainObject((manifest as unknown as Record<string, unknown>)[key]);
    if (ext && isShuRigPalette(ext.palette)) return ext.palette;
  }
  const props = manifest.preferencesSchema?.properties ?? {};
  for (const key of ["variant", "palette"]) {
    const d = props[key]?.default;
    if (isShuRigPalette(d)) return d;
  }
  const first = manifest.variants[0]?.id;
  if (isShuRigPalette(first)) return first;
  return DEFAULT_SHU_RIG_PALETTE;
}

/**
 * 沒有 manifest 本文時，用清單摘要組一個 `character-rig` 2.0 pack 交給這個 adapter 遷移
 * （characterId＝匯入 id、名字＝清單 displayName）。純資料，不執行任何東西。
 */
export function importedRigPack(
  entry: { characterId: string; displayName?: unknown; version?: unknown },
  palette: string
): Record<string, unknown> {
  const declared = plainObject(entry.displayName);
  const name = declared && Object.keys(declared).length > 0 ? declared : { "zh-TW": "角色" };
  return {
    schemaVersion: RIG_PACK_SCHEMA_VERSION,
    kind: RIG_PACK_KIND,
    id: entry.characterId,
    name,
    palette: isShuRigPalette(palette) ? palette : DEFAULT_SHU_RIG_PALETTE,
    ...(typeof entry.version === "string" && entry.version.length > 0 ? { version: entry.version } : {}),
  };
}

export interface ShuAdapterOptions {
  /** bundled /characters/<characterId>/manifest.json（已驗證）；省略時由 legacyRig 遷移。 */
  manifest?: CharacterManifest;
  /** legacy character-rig 2.0 pack manifest（/packs/<id>/manifest.json，kind character-rig）。 */
  legacyRig?: unknown;
  palette?: string;
  canvas?: HTMLCanvasElement;
  /** 測試／host 注入的舞台；省略且有 canvas 時由 initialize() 建立並擁有。 */
  stage?: StageRenderer;
  scale?: number;
  /** host 的混音器；省略時用內建 LocalMixer。 */
  mixer?: MixerPort;
  charName?: string;
}

interface Watch {
  kind: Transient["kind"];
  animation?: string;
  verified?: boolean;
}

interface ActiveCommand {
  messageId: string;
  completeAt: number;
  sink: ReceiptSink;
  /** transient 命令：要盯著的 transient 特徵（被擠掉＝preempted）。 */
  watch: Watch | null;
  plan: ShuExpressionPlan;
}

/** adapter 自帶的混音器（獨立測試／沒有 host machine 時）：自己驅動舞台。 */
class LocalMixer implements MixerPort {
  private st: MachineState = { base: "idle", transient: null };

  constructor(
    private readonly stage: () => StageRenderer | null,
    private readonly now: () => number
  ) {}

  apply(event: MachineEvent): MachineState {
    this.st = reduce(this.st, event, this.now());
    this.sync();
    return this.st;
  }

  state(): MachineState {
    return this.st;
  }

  sync(now = this.now()): void {
    const stage = this.stage();
    if (!stage) return;
    const p = pose(this.st, now);
    stage.setAnimation(p.animation, p.frameSlice);
    const t = this.st.transient && this.st.transient.untilMs > now ? this.st.transient : null;
    stage.setMachineFlags(machineStageFlags(this.st.base, t, p.animation, p.ambient));
  }
}

function transientMatches(t: Transient | null, w: Watch): boolean {
  if (!t) return false;
  return t.kind === w.kind && (t.animation ?? undefined) === w.animation && (t.verified ?? undefined) === w.verified;
}

export class ShuCharacterAdapter implements CharacterAdapter {
  readonly manifest: CharacterManifest;
  /** Director／gameFeel／personality／舊路徑 machine 需要的角色表（host 注入用）。 */
  readonly directorTables = SHU_DIRECTOR_TABLES;
  readonly landingTable = SHU_LANDING;
  readonly variantWeights = SHU_VARIANT_WEIGHTS;
  readonly eventArt = SHU_EVENT_ART;
  readonly toyCatalog = SHU_TOYS;

  private host: AdapterHost | null = null;
  private stage: StageRenderer | null;
  private readonly ownsStage: boolean;
  private readonly canvas: HTMLCanvasElement | null;
  private readonly scale: number;
  private paletteName: string;
  private mixer: MixerPort | null;
  private local: LocalMixer | null = null;
  private charName: string | null;
  private listeners = new Set<(e: AdapterInputEvent) => void>();
  private active: ActiveCommand | null = null;
  private lastPlan: ShuExpressionPlan | null = null;
  private visible = true;
  private suspended = false;
  private disposed = false;
  private reducedMotion = false;
  private familiars: { id: string; name: string; palette: string }[] = [];
  private currentScene = "none";

  constructor(opts: ShuAdapterOptions = {}) {
    if (opts.manifest) {
      this.manifest = opts.manifest;
    } else {
      const migrated = migratePackToManifest(opts.legacyRig ?? DEFAULT_LEGACY_RIG, { registry: shuMigrationRegistry() });
      if (!migrated.ok) throw new Error(`shu rig pack rejected: ${migrated.errors.join("; ")}`);
      this.manifest = migrated.manifest;
    }
    if (this.manifest.entrypoint.kind !== "builtin" || this.manifest.entrypoint.id !== SHU_RIG_ENTRYPOINT_ID) {
      throw new Error("manifest entrypoint is not builtin shu-rig");
    }
    this.paletteName = opts.palette ?? rigPaletteFor(this.manifest);
    this.canvas = opts.canvas ?? null;
    this.stage = opts.stage ?? null;
    this.ownsStage = !opts.stage;
    this.scale = opts.scale ?? 1;
    this.mixer = opts.mixer ?? null;
    this.charName = opts.charName ?? null;
  }

  // ---- CharacterAdapter ------------------------------------------------------

  async initialize(host: AdapterHost): Promise<void> {
    this.host = host;
    if (!this.stage && this.canvas) {
      this.stage = new StageRenderer(this.canvas, this.paletteName, this.scale);
    }
    if (!this.mixer) {
      this.local = new LocalMixer(() => this.stage, () => host.now());
      this.mixer = this.local;
    }
    this.reducedMotion = host.reducedMotion();
    const stage = this.stage;
    if (stage) {
      stage.setPalette(this.paletteName);
      stage.setReducedMotion(this.reducedMotion);
      stage.setCharName(this.charName ?? displayNameOf(this.manifest, host.locale));
    }
    this.local?.sync();
  }

  negotiate(hello: Hello): Negotiate {
    this.reducedMotion = hello.reducedMotion === true;
    this.stage?.setReducedMotion(this.reducedMotion);
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
    if (!this.suspended) this.stage?.resume();
  }

  hide(): void {
    this.visible = false;
    if (this.canvas) this.canvas.style.visibility = "hidden";
    // 看不見就不畫、不算物理、不回報互動框。
    this.stage?.pause();
  }

  suspend(): void {
    this.suspended = true;
    this.stage?.pause();
  }

  resume(): void {
    this.suspended = false;
    if (this.visible) this.stage?.resume();
  }

  reconfigure(prefs: Record<string, unknown>): void {
    const stage = this.stage;
    if (typeof prefs.name === "string") {
      this.charName = prefs.name;
      stage?.setCharName(prefs.name);
    }
    if (typeof prefs.scene === "string") {
      this.currentScene = (STAGE_SCENES as string[]).includes(prefs.scene) ? prefs.scene : "none";
      stage?.setScene(prefs.scene);
    }
    const toggles: { play?: boolean; cursorPlay?: boolean; deskMove?: boolean; approach?: boolean } = {};
    for (const key of ["play", "cursorPlay", "deskMove", "approach"] as const) {
      if (typeof prefs[key] === "boolean") toggles[key] = prefs[key] as boolean;
    }
    if (Object.keys(toggles).length > 0) stage?.setToggles(toggles);
    if (prefs.tuning && typeof prefs.tuning === "object") {
      stage?.setTuning({ ...DEFAULT_TUNING, ...(prefs.tuning as Partial<PersonalityTuning>) });
    }
    if (Array.isArray(prefs.familiars)) {
      this.familiars = prefs.familiars
        .filter((f): f is { id: string; name: string; palette: string } => {
          const r = f as Record<string, unknown>;
          return !!r && typeof r.id === "string" && typeof r.name === "string" && typeof r.palette === "string";
        })
        .slice(0, 3);
      stage?.setFamiliars(this.familiars);
    }
    if (typeof prefs.palette === "string") {
      this.paletteName = prefs.palette;
      stage?.setPalette(prefs.palette);
    }
    if (typeof prefs.reducedMotion === "boolean") {
      this.reducedMotion = prefs.reducedMotion;
      stage?.setReducedMotion(prefs.reducedMotion);
    }
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
    const mixer = this.mixer;
    if (!mixer || !this.host) {
      sink({ messageId: envelope.messageId, status: "failed", resolution: "failed", detail: "adapter not initialized" });
      return;
    }
    // 安全 intent 的固定文案／安全動畫一律以 envelope.intent 為準：
    // 非安全的 viaIntent 不能改寫安全語意（呈現層沒有權限主權）。
    const intent = presentedIntent(envelope.intent, resolution?.viaIntent);
    const p = shuExpressionPlan(intent, envelope.truthState, envelope.presentationHints?.variant);
    this.lastPlan = p;

    // 同一時間只盯一則命令：新的一則進來，舊的（若還在等）算被取代。
    if (this.active && this.active.messageId !== envelope.messageId) {
      const old = this.active;
      this.active = null;
      old.sink({ messageId: old.messageId, status: "cancelled", reason: "replaced" });
    }
    sink({ messageId: envelope.messageId, status: "accepted" });
    const reported = resolution?.resolution ?? "exact";
    const now = this.host.now();

    if (p.mode === "clear") {
      // Runtime 派送的 idle／cancelled：真相回待機（含安全訊息）；AI 的低優先 idle 到不了這裡（Gateway 擋）。
      mixer.apply({ type: "clear-transient", force: true });
      sink({ messageId: envelope.messageId, status: "started", resolution: reported });
      sink({ messageId: envelope.messageId, status: "completed", resolution: reported });
      return;
    }

    const hint = envelope.durationHint?.ms;
    const hinted = typeof hint === "number" && Number.isFinite(hint) && hint > 0 ? Math.min(hint, LIMITS.durationMaxMs) : null;
    const event = machineEventForAnimation(p.animation, p.frameSlice, hinted ?? undefined);
    let durationMs: number | undefined = hinted ?? undefined;
    if (event.type === "transient" && event.kind === "performing" && durationMs === undefined) {
      durationMs = shuNaturalDurationMs(p.expression);
    }
    const applied: MachineEvent = event.type === "transient" && durationMs !== undefined ? { ...event, durationMs } : event;
    const after = mixer.apply(applied);

    if (applied.type === "base") {
      if (after.base !== applied.base) {
        sink({ messageId: envelope.messageId, status: "cancelled", reason: "preempted", detail: "base state not applied" });
        return;
      }
      sink({ messageId: envelope.messageId, status: "started", resolution: reported });
      // 基態命令「演完」＝enter 段播完；基態本身持續到 Runtime 改變它。
      this.active = { messageId: envelope.messageId, completeAt: now + shuEnterMs(p.expression) + 300, sink, watch: null, plan: p };
      return;
    }

    if (applied.type !== "transient") {
      sink({ messageId: envelope.messageId, status: "started", resolution: reported });
      sink({ messageId: envelope.messageId, status: "completed", resolution: reported });
      return;
    }
    const watch: Watch = { kind: applied.kind, animation: applied.animation, verified: applied.verified };
    const t = after.transient && after.transient.untilMs > now ? after.transient : null;
    if (!transientMatches(t, watch)) {
      // 本機混音器留住了更高優先的演出（或基態是 emergency/offline）：沒上台就誠實說沒上台。
      sink({ messageId: envelope.messageId, status: "cancelled", reason: "preempted", detail: "a higher-priority display kept the stage" });
      return;
    }
    sink({ messageId: envelope.messageId, status: "started", resolution: reported });
    this.active = { messageId: envelope.messageId, completeAt: t!.untilMs, sink, watch, plan: p };
  }

  tick(now: number): void {
    this.local?.sync(now);
    const a = this.active;
    if (!a) return;
    if (now >= a.completeAt) {
      this.active = null;
      a.sink({ messageId: a.messageId, status: "completed" });
      return;
    }
    if (a.watch && this.mixer && !transientMatches(this.mixer.state().transient, a.watch)) {
      this.active = null;
      a.sink({ messageId: a.messageId, status: "cancelled", reason: "preempted" });
    }
  }

  cancel(messageId: string): void {
    const a = this.active;
    if (!a || a.messageId !== messageId) return; // 冪等：不是進行中的那則就不動
    this.active = null;
    if (a.watch && this.mixer && transientMatches(this.mixer.state().transient, a.watch)) {
      // 取消回待機（只對「正在盯的那則命令」；Runtime 撤回自己的 blocked 也算）；
      // 基態命令（emergency／offline）不在這裡解除——那是 Runtime 的真相。
      this.mixer.apply({ type: "clear-transient", force: true });
    }
    a.sink({ messageId, status: "cancelled", reason: "cancel" });
  }

  dispose(): void {
    this.disposed = true;
    this.active = null;
    this.listeners.clear();
    if (this.ownsStage) this.stage?.destroy();
    else this.stage?.pause();
    this.stage = null;
  }

  onInput(cb: (event: AdapterInputEvent) => void): () => void {
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

  // ---- GameplayExtension ------------------------------------------------------

  readonly gameplay: GameplayExtension = {
    spawnToy: (kind) => {
      if (!this.stage || !isShuToyKind(kind)) return null;
      // 舞台凍結（緊急停止／離線／暫停）或玩具已滿時拒絕：沒生成就不說生成了。
      if (!this.stage.spawnToy(kind)) return null;
      this.emitInput({ kind: "character.toy-thrown", payload: { toyId: kind } });
      return kind;
    },
    clearToys: () => {
      this.stage?.clearAllToys();
    },
    familiars: {
      summon: (id) => {
        if (!this.stage || this.familiars.length >= 3 || !/^[a-zA-Z0-9-]{1,32}$/.test(id)) return false;
        if (this.familiars.some((f) => f.id === id)) return true;
        this.familiars = [...this.familiars, { id, name: id, palette: this.paletteName }];
        this.stage.setFamiliars(this.familiars);
        return true;
      },
      dismiss: (id) => {
        if (!this.stage || !this.familiars.some((f) => f.id === id)) return false;
        this.familiars = this.familiars.filter((f) => f.id !== id);
        this.stage.setFamiliars(this.familiars);
        return true;
      },
      list: () => this.familiars.map((f) => f.id),
    },
    scene: {
      set: (sceneId) => {
        if (!this.stage || !(STAGE_SCENES as string[]).includes(sceneId)) return false;
        this.currentScene = sceneId;
        this.stage.setScene(sceneId);
        return true;
      },
      current: () => (this.stage ? this.currentScene : null),
    },
    rollCall: () => this.stage !== null,
    onHitRects: (cb) => {
      const stage = this.stage;
      if (!stage) return () => {};
      stage.onHitRect((r) => cb([{ id: "character", x: r.x, y: r.y, w: r.w, h: r.h } satisfies HitRect]));
      return () => stage.onHitRect(() => {});
    },
    routePointer: (input: PointerInput) => {
      const stage = this.stage;
      if (!stage) return false;
      switch (input.type) {
        case "down":
          return stage.pointerDown(input.x, input.y) === "toy";
        case "move":
          stage.pointerMove(input.x, input.y);
          return stage.isDraggingToy();
        case "up": {
          const was = stage.isDraggingToy();
          stage.pointerUp();
          return was;
        }
        case "cancel":
          stage.pointerLeave();
          return false;
        default:
          return false;
      }
    },
  };

  // ---- host 專用（超出 CharacterAdapter 契約的舞台存取） -------------------------

  /** host 需要直接接指標／hit-rect／表情事件時取得舞台（null＝尚未 initialize 或已 dispose）。 */
  stageRenderer(): StageRenderer | null {
    return this.stage;
  }

  /** Roll Call（人話）：機器狀態優先；ambient 時由遊玩場描述。 */
  rollCallNow(machineLabel: string | null): { name: string; activity: string }[] {
    if (this.stage) return this.stage.rollCallNow(machineLabel);
    return [{ name: this.charName ?? displayNameOf(this.manifest, this.host?.locale ?? "zh-TW"), activity: machineLabel ?? "在休息" }];
  }

  /** 測試／診斷：最後一次 perform 的表情計畫。 */
  lastExpressionPlan(): ShuExpressionPlan | null {
    return this.lastPlan;
  }

  /** 測試／診斷：目前盯著的命令 id。 */
  activeMessageId(): string | null {
    return this.active?.messageId ?? null;
  }

  /** 測試／診斷：協商時記下的 reduced motion。 */
  isReducedMotion(): boolean {
    return this.reducedMotion;
  }

  /** 測試／診斷：內建混音器的狀態（host 注入 mixer 時回 null）。 */
  localMachineState(): MachineState | null {
    return this.local?.state() ?? null;
  }
}
