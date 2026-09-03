// CPP §8.1 In-process transport：TypeScript `CharacterAdapter` 契約。
//
// 桌面視窗內建 adapter（shu-rig／sprite／text）都實作這個介面；Gateway 只透過它
// 溝通，不引用任何 rig 部位、動畫名或 DOM 結構。介面刻意不含 React、不含
// Runtime token；adapter 拿不到 truthState 的決定權（envelope 是唯讀輸入）。
//
// 回執契約：perform() 內必須同步發出至少一則回執（accepted／started／acknowledged／
// unsupported／failed）；之後 started → completed｜failed｜cancelled；或
// acknowledged（Gateway 之後記成 uncertain，不猜 completed）。

import type {
  CharacterInputEvent,
  CharacterManifest,
  CommandReceipt,
  Hello,
  InputEventKind,
  IntentEnvelope,
  IntentResolution,
  Negotiate,
  Negotiated,
  PrivacyClass,
} from "./protocol";

/** adapter 發出的回執：Gateway 補上 characterInstanceId／generation／at，並強制 resolution 只能變差。 */
export type AdapterReceipt = Pick<CommandReceipt, "messageId" | "status"> &
  Partial<Pick<CommandReceipt, "resolution" | "detail" | "reason" | "generation">>;

export type ReceiptSink = (receipt: AdapterReceipt) => void;

/** adapter 送出的原始輸入事件；Gateway 正規化、節流、加 id／時間／世代。 */
export interface AdapterInputEvent {
  kind: InputEventKind;
  payload?: Record<string, unknown>;
  privacyClass?: PrivacyClass;
  /** adapter 自認的世代（可省略）；與 Gateway 不符則丟棄。 */
  generation?: number;
}

export type LogLevel = "debug" | "info" | "warn" | "error";

/** host 提供給 adapter 的環境：時間注入（測試可控）、reduced motion、語言、log。 */
export interface AdapterHost {
  now(): number;
  reducedMotion(): boolean;
  readonly locale: string;
  log(level: LogLevel, message: string, data?: Record<string, unknown>): void;
}

export interface HitRect {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PointerInput {
  type: "down" | "move" | "up" | "cancel";
  /** 視窗相對座標。 */
  x: number;
  y: number;
  buttons?: number;
}

/**
 * 遊玩擴充（選用）：shu-rig 這類有遊玩場的 adapter 才實作。
 * 只宣告介面；本檔不實作。所有方法都是「請求」，adapter 可拒絕（回 null／false）。
 */
export interface GameplayExtension {
  spawnToy(kind: string, opts?: { x?: number; y?: number }): string | null;
  clearToys(): void;
  familiars: {
    summon(id: string): boolean;
    dismiss(id: string): boolean;
    list(): string[];
  };
  scene: {
    set(sceneId: string): boolean;
    current(): string | null;
  };
  rollCall(): boolean;
  onHitRects(cb: (rects: HitRect[]) => void): () => void;
  /** host 把指標事件交給 adapter 路由；回 true 表示已被角色／玩具消費。 */
  routePointer(input: PointerInput): boolean;
}

export interface CharacterAdapter {
  readonly manifest: CharacterManifest;
  initialize(host: AdapterHost): Promise<void>;
  negotiate(hello: Hello): Negotiate;
  /** Gateway 算出協商結果後通知（選用）。 */
  negotiated?(result: Negotiated): void;
  show(): void;
  hide(): void;
  suspend(): void;
  resume(): void;
  reconfigure(prefs: Record<string, unknown>): void;
  /**
   * 執行 intent。`resolution` 是 Gateway 協商出的結果（substituted 時含 viaIntent），
   * adapter 應據此演出；必須在回傳前同步發出至少一則回執。
   */
  perform(envelope: IntentEnvelope, sink: ReceiptSink, resolution?: IntentResolution): void;
  cancel(messageId: string): void;
  dispose(): void;
  onInput(cb: (event: AdapterInputEvent) => void): () => void;
  /** 時間推進（選用）：Gateway 在 sweep(now) 時呼叫；adapter 不得自帶 timer 也能完成演出。 */
  tick?(now: number): void;
  readonly gameplay?: GameplayExtension;
}

/** 正規化後送往 Runtime 的輸入事件附帶資訊。 */
export interface InputMeta {
  instanceId: string;
  characterId: string;
  role: string;
}

export type NormalizedInputSink = (event: CharacterInputEvent, meta: InputMeta) => void;
