// Desktop character companion window: honest runtime-state presentation + quick actions.
//
// This window is presentation + input entry ONLY. It holds no authority:
// every action goes through the same backend commands/policy as everything
// else, and safety states (emergency/blocked/unknown) use fixed standard
// wording that packs cannot override.
//
// CPP（docs/character-protocol/README.md）：視窗內有一個 in-process CharacterGateway。
// 角色由 /characters/index.json＋prefs.companionPack 選出 → builtin entrypoint
// （shu-rig／sprite／text）→ adapter → gateway.registerInstance。Runtime 有
// characterProtocol 時，`character.intent` 事件 → gateway.dispatch → adapter 演出，
// 回執經 /v1/character/receipts、輸入經 /v1/character/events；舊 daemon（沒有
// characterProtocol）維持 mapRuntimeEvent 的相容路徑。角色載入失敗／崩潰時退回
// 文字角色，固定文案顯示在**可信 DOM 元素**上（不在任何 adapter 裡）。

import React from "react";
import { api, RuntimeEvent } from "../api";
import { bootstrapSupervisor, desktop, isTauri, type DesktopPrefs } from "../desktop";
import { onRuntimeEvent } from "../api";
import {
  DRAG_HOLD_MS,
  DRAG_RENEW_MS,
  initial,
  LegacyEventArt,
  MachineEvent,
  MachineState,
  mapRuntimeEvent,
  MixerPort,
  NEUTRAL_EVENT_ART,
  pose,
  reduce,
  wasPreempted,
  wasReplacedByPerforming,
} from "./machine";
import { PackManifest, RendererBackend, SpriteRenderer, validateManifest } from "./renderer";
import { machineStageFlags, StageRenderer } from "./rig/stage";
import {
  HitRegion,
  mergeHitRegions,
  prepareHitRegions,
  sendHitRegions,
  translateRegions,
} from "./hitRegions";
import { ToyKind } from "./playfield";
import { directorTickGate, EMPTY_DIRECTOR_TABLES, InteractionDirector } from "./director";
import { activeConversationProvider, ConversationContext } from "./conversation";
import { applyPresence, cancelEffects, planPresentationCommand } from "./presentationCommands";
import {
  BehaviorState as CompanionBehaviorState,
  initialBehavior,
  layeredMicroMotion,
  noteEvent,
  noteInterruption,
  noteUserInteraction,
  seededRng,
  stepBehavior,
} from "./behavior";
import {
  behaviorFor,
  BehaviorTuning,
  nextChapter,
  PersonaPack,
  resolveLine,
  StoryPack,
  validatePersonaPack,
  validateStoryPack,
} from "./packs";
import { createReceiptDedup, knowledgeReceiptLine } from "./knowledgeReceipts";
import {
  DEFAULT_TUNING,
  personalityFor,
  PersonalityProfile,
  PersonalityTuning,
  tuningFor,
} from "./personality";
import {
  approachAllowed,
  bubbleAllowed,
  bubbleOutcome,
  hoverBubblePolicy,
  proactiveQuietActive,
  proactiveQuietUntil,
  quietBase,
  soundOutcome,
} from "./attention";
import { LandingTable, pickLanding, planClickReaction } from "./gameFeel";
import {
  emptyMemory,
  InteractionMemory,
  notePlay,
  noteSession,
  sanitizeMemory,
} from "./interactionMemory";
import { MixerRenderer } from "./mixerRenderer";
import { companionSensorLabel } from "./sensorLabels";
import {
  adapterReconfigureFor,
  CHARACTER_LOAD_FAILED_LINE,
  charNameFor,
  CharacterSource,
  companionReloadPlan,
  cssClassForEntrypoint,
  dropDestinationLines,
  dropItemLine,
  dropPreviewItems,
  DropPreviewItem,
  DropPreviewSession,
  entrypointKindOf,
  EntrypointKind,
  envelopeForInstance,
  ForwardDecision,
  helloFor,
  HelloTracker,
  importedRigPack,
  INITIAL_HELLO_TRACKER,
  inputEventFor,
  interactionMemoryFromPrefs,
  isImageDataUrl,
  personaIdFor,
  PRIMARY_INSTANCE_ID,
  receiptForRuntime,
  rehelloOnInstanceEvent,
  rehelloOnStatus,
  resolveCharacterSource,
  rigPaletteFor,
  rigPaletteForImported,
  RuntimeFeed,
  selectRuntimeFeed,
  storyPackIdFor,
  summarizeForwardDecisions,
  systemTextFromEvent,
} from "./gatewayWiring";
import { CharacterGateway } from "../character/gateway";
import type { CharacterAdapter } from "../character/adapter";
import { loadCharacterIndex } from "../character/registry";
import { ShuCharacterAdapter } from "../character/adapters/shu";
import { SpriteCharacterAdapter } from "../character/adapters/sprite";
import { TextCharacterAdapter } from "../character/adapters/text";
import type { CharacterManifest, CommandReceipt, IntentEnvelope } from "../character/protocol";
import type { ToyCatalogEntry } from "../character/adapters/shuTables";

// v0.4: itemized Presentation Provider receptors — each interaction kind
// routes to its own receptor so consent/enable is per-capability. Unknown
// kinds fall back to the legacy combined receptor (kept for compat).
// （舊 daemon 路徑；有 characterProtocol 時輸入一律走 /v1/character/events。）
const RECEPTOR_FOR_KIND: Record<string, string> = {
  "companion-clicked": "companion.click",
  "companion-dragged": "companion.click",
  "text-submitted": "companion.text-input",
  "action-selected": "companion.quick-action",
  "pointer-approached": "companion.pointer",
  "pointer-left": "companion.pointer",
  "companion-dropped": "companion.drag-drop",
  "drop-entered": "companion.drag-drop",
  "drop-left": "companion.drag-drop",
  "drop-cancelled": "companion.drag-drop",
  "bubble-shown": "companion.bubble-events",
  "bubble-dismissed": "companion.bubble-events",
  "animation-completed": "companion.animation-events",
  "animation-interrupted": "companion.animation-events",
};

const LOCALE = "zh-TW";

/** Play one of three registered, generated-in-memory cues. No file path, URL,
 * arbitrary oscillator program or AI-provided code crosses this boundary. */
async function playRegisteredSound(sound: "chime" | "soft-pop" | "tick"): Promise<void> {
  const AudioContextCtor = window.AudioContext;
  if (!AudioContextCtor) throw new Error("this system has no Web Audio output");
  const context = new AudioContextCtor();
  const sequence =
    sound === "chime"
      ? [[660, 0], [880, 0.11]]
      : sound === "soft-pop"
        ? [[440, 0]]
        : [[960, 0]];
  const duration = sound === "tick" ? 0.045 : sound === "soft-pop" ? 0.1 : 0.24;
  try {
    // WebKit can create the context in `suspended` state until a consented
    // presentation command reaches the visible surface. Resume explicitly;
    // if the platform still blocks output this rejects and the Runtime gets an
    // honest `failed` ACK instead of a false `completed` receipt.
    if (context.state === "suspended") await context.resume();
    for (const [frequency, delay] of sequence) {
      const oscillator = context.createOscillator();
      const gain = context.createGain();
      oscillator.frequency.value = frequency;
      oscillator.type = "sine";
      const start = context.currentTime + delay;
      gain.gain.setValueAtTime(0.0001, start);
      gain.gain.exponentialRampToValueAtTime(0.08, start + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + duration);
      oscillator.connect(gain).connect(context.destination);
      oscillator.start(start);
      oscillator.stop(start + duration);
    }
    await new Promise((resolve) => setTimeout(resolve, Math.ceil((duration + 0.12) * 1000)));
  } finally {
    await context.close();
  }
}

/**
 * Stop any speech that is currently being spoken.
 *
 * 緊急停止／取消必須真的讓角色閉嘴：清氣泡不會停掉已經送進語音服務的句子，
 * 那句話會在「緊急停止中」的畫面上繼續講完。沒有語音服務時是 no-op。
 */
export function stopSpeech(): void {
  try {
    const synth = (window as unknown as { speechSynthesis?: SpeechSynthesis }).speechSynthesis;
    synth?.cancel();
  } catch {
    /* 平台不允許：沒有正在播的語音可停 */
  }
}

/** Speak bounded plain text through the OS/browser speech service. A 9-second
 * cap stays inside the Runtime presentation ACK lease; overlong/blocked speech
 * is cancelled and reported as failed, never acknowledged as completed. */
async function speakText(text: string): Promise<void> {
  if (!("speechSynthesis" in window) || typeof SpeechSynthesisUtterance === "undefined") {
    throw new Error("this system has no speech synthesis service");
  }
  await new Promise<void>((resolve, reject) => {
    const utterance = new SpeechSynthesisUtterance(text);
    utterance.lang = LOCALE;
    utterance.rate = 1;
    let settled = false;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      if (error) reject(error);
      else resolve();
    };
    utterance.onend = () => finish();
    utterance.onerror = (event) => finish(new Error(`speech synthesis failed: ${event.error}`));
    const timeout = window.setTimeout(() => {
      window.speechSynthesis.cancel();
      finish(new Error("speech synthesis exceeded the 9 second presentation lease"));
    }, 9000);
    window.speechSynthesis.speak(utterance);
  });
}

/**
 * 落點是否貼近螢幕邊緣（決定「滑倒裝沒事」而不是站穩）。
 * 只讀視窗與螢幕幾何，不讀游標、不保存任何位置。
 */
async function nearScreenEdge(
  win: { outerSize: () => Promise<{ width: number; height: number }> },
  pos: { x: number; y: number }
): Promise<boolean> {
  try {
    const { currentMonitor } = await import("@tauri-apps/api/window");
    const monitor = await currentMonitor();
    if (!monitor) return false;
    const size = await win.outerSize();
    const margin = 48;
    const left = pos.x - monitor.position.x;
    const top = pos.y - monitor.position.y;
    const right = monitor.size.width - (left + size.width);
    const bottom = monitor.size.height - (top + size.height);
    return left < margin || top < margin || right < margin || bottom < margin;
  } catch {
    // 取不到螢幕資訊就當作不在邊緣（不猜、不假裝知道）。
    return false;
  }
}

/** Director 的 ambient 動作在 App 這一側的鏡像（對抗審查 director-pipeline-018）。 */
export interface AmbientActionMirror {
  animation?: string;
  durationMs: number;
  startedAt: number;
}

/** 被點擊／拖曳中斷的長 ambient 的恢復計畫（App 側）。 */
export interface CompanionResumePlan {
  animation?: string;
  remainingMs: number;
  expiresAt: number;
}

/** 恢復計畫的有效期（與 InteractionDirector.notePreempted 一致）。 */
const RESUME_PLAN_TTL_MS = 20_000;

/**
 * 中斷前先留一份恢復計畫（對抗審查 director-pipeline-018）。
 *
 * 點擊／拖曳的反應會走 `InteractionDirector.reactDetailed()`，而它會把 Director 自己的
 * `interrupted` 計畫清掉（新反應取消舊的恢復計畫），因此「先 react 再 notePreempted」
 * 與「先 notePreempted 再 react」都留不住計畫——§6.1「動作可中斷、可恢復」在這兩條
 * 最常見的中斷路徑上永遠不成立。App 這一側在反應**之前**依同樣的門檻留一份，
 * 短反應播完後再接回去。
 *
 * 門檻與 Director 相同：原動作 >= 4000ms 且還剩 > 1500ms 才值得恢復。
 */
export function resumePlanFor(
  current: AmbientActionMirror | null,
  nowMs: number
): CompanionResumePlan | null {
  if (!current) return null;
  const remaining = current.durationMs - (nowMs - current.startedAt);
  if (!(remaining > 1_500 && current.durationMs >= 4_000)) return null;
  return {
    animation: current.animation,
    remainingMs: remaining,
    expiresAt: nowMs + RESUME_PLAN_TTL_MS,
  };
}

/**
 * 取用恢復計畫：過期就放棄；安靜／Reduced Motion 期間先不演（但保留計畫，
 * 與 Director 的恢復分支同樣的情境判斷）；其餘取用一次後計畫就用掉。
 */
export function takeResumePlan(
  plan: CompanionResumePlan | null,
  ctx: { nowMs: number; quiet: boolean; reducedMotion: boolean }
): { plan: CompanionResumePlan | null; action: CompanionResumePlan | null } {
  if (!plan) return { plan: null, action: null };
  if (ctx.nowMs > plan.expiresAt) return { plan: null, action: null };
  if (ctx.quiet || ctx.reducedMotion) return { plan, action: null };
  return { plan: null, action: plan };
}

/** 500ms behavior pump 在目前可見性下該做的事（對抗審查 perf-claims-017）。
 *
 *  隱藏 ≠ 靜音：CPP Gateway 的看門狗（acknowledged→uncertain 的誠實階梯）與
 *  Behavior Runtime 的記帳（presence 心跳照實回報 activation／attention）不能停；
 *  但沒有觀眾的演出——micro-motion、姿勢刷新、互動框回報、Director 的 ambient
 *  排程與 hover 氣泡——在隱藏期間全部停下。 */
export function companionPumpWork(hidden: boolean): {
  sweep: boolean;
  behavior: boolean;
  present: boolean;
} {
  return { sweep: true, behavior: true, present: !hidden };
}

/** 狀態輪詢週期（對抗審查 perf-claims-017）。隱藏時降頻而**不是**關掉：
 *  SSE 斷線時這條輪詢是緊急停止／暫停的唯一後盾（關掉它會弱化安全網），
 *  回到可見時另外立刻補一次。 */
export function statusPollIntervalMs(hidden: boolean): number {
  return hidden ? 30_000 : 5_000;
}

/** 可信文字元素上的一行（system.text／載入失敗文案）。 */
interface TrustedText {
  text: string;
  marker: "verified" | "none";
}

/** 同源 fetch（角色索引／manifest／pack 只從本機資料夾載入）。 */
const sameOriginFetch = (url: string) => fetch(url);

export default function CompanionApp() {
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  /** 視窗根節點：量 UI 面（快捷選單／氣泡／可信文字）的 hit region 用。 */
  const rootRef = React.useRef<HTMLDivElement>(null);
  /** hit-region IPC 是否在飛（同時只送一個；丟掉的由下一次回報補上）。 */
  const hitRegionsBusyRef = React.useRef(false);
  const textHostRef = React.useRef<HTMLDivElement>(null);
  const rendererRef = React.useRef<RendererBackend | null>(null);
  /** 遊玩場（只有 shu-rig adapter 有；sprite／text 無遊玩）。 */
  const stageRef = React.useRef<StageRenderer | null>(null);
  /** 目前註冊在 gateway 的 adapter（任何 entrypoint）。 */
  const adapterRef = React.useRef<CharacterAdapter | null>(null);
  /** shu-rig adapter（roll call／玩具目錄／角色表）；其他角色為 null。 */
  const shuRef = React.useRef<ShuCharacterAdapter | null>(null);
  const gatewayRef = React.useRef<CharacterGateway | null>(null);
  const manifestRef = React.useRef<CharacterManifest | null>(null);
  /** 最新一次讀到的 host 偏好（companion-reload 時比對：能就地套用的就不整頁重載）。 */
  const prefsSnapshotRef = React.useRef<DesktopPrefs | null>(null);
  /** runtime feed：有 characterProtocol → protocol；否則 legacy。null＝還沒問過 status。 */
  const feedRef = React.useRef<RuntimeFeed | null>(null);
  /** Runtime hello 回給我們的世代（轉送回執時要帶）。 */
  const runtimeGenerationRef = React.useRef<number | null>(null);
  /** hello 追蹤：成功與否、上次嘗試時間、daemon 實例身分（startedAt）——斷線／重啟後要重新 hello。 */
  const helloTrackerRef = React.useRef<HelloTracker>(INITIAL_HELLO_TRACKER);
  /** 最近一批送去 Runtime 的輸入事件（拖放一檔一事件；確認流程要等**全部**的結果）。 */
  const forwardBatchRef = React.useRef<Promise<ForwardDecision | null>[]>([]);
  /** 讀取上一批轉送結果的總結（任何一則沒送到＝null；任何一則被丟＝dropped）。 */
  const awaitForwardBatch = React.useCallback(async (): Promise<ForwardDecision | null> => {
    const batch = forwardBatchRef.current;
    forwardBatchRef.current = [];
    return summarizeForwardDecisions(await Promise.all(batch));
  }, []);
  const approachEnabledRef = React.useRef(true);
  const charNameRef = React.useRef("角色");
  const [charName, setCharName] = React.useState("角色");
  // v0.5 呈現偏好（勿擾/氣泡/音效/拖曳）＋個性 tuning＋互動記憶。
  const dndRef = React.useRef(false);
  const bubblesEnabledRef = React.useRef(true);
  const soundEnabledRef = React.useRef(false);
  const dragEnabledRef = React.useRef(true);
  const personalityRef = React.useRef<PersonalityProfile>(personalityFor("natural"));
  const tuningRef = React.useRef<PersonalityTuning>(DEFAULT_TUNING);
  const memoryRef = React.useRef<InteractionMemory>(emptyMemory());
  const quietHoursRef = React.useRef(false);
  /** 使用者要求的本機安靜期到期時間（epoch ms；0＝沒有）。
   *  存進 DesktopPrefs：角色視窗會因設定變更而重載，只放記憶體會失效。 */
  const quietUntilRef = React.useRef(0);
  // hover 短氣泡：只記「從什麼時候開始停在角色上」，不保存游標軌跡。
  const hoverSinceRef = React.useRef(0);
  /** reduced-motion 媒體查詢監聽器的解除函式。 */
  const motionCleanup = React.useRef<(() => void) | null>(null);
  const reducedMotionRef = React.useRef(false);
  const machineRef = React.useRef<MachineState>(initial);
  const lastBubbleAt = React.useRef(0);
  const [bubble, setBubble] = React.useState<string | null>(null);
  /** 目前氣泡是不是安全文字（cancel 不得清掉它；只有 clear-all 可以）。 */
  const bubbleSafetyRef = React.useRef(false);
  /** 上一句 hover 短句（防連續同句）。 */
  const lastHoverLineRef = React.useRef<string | null>(null);
  const [menuOpen, setMenuOpen] = React.useState(false);
  const [characterId, setCharacterId] = React.useState("");
  const characterIdRef = React.useRef("");
  const [entrypointKind, setEntrypointKind] = React.useState<EntrypointKind | null>(null);
  const [toyCatalog, setToyCatalog] = React.useState<readonly ToyCatalogEntry[]>([]);
  const [ready, setReady] = React.useState(false);
  const [inputOpen, setInputOpen] = React.useState(false);
  const [sensorLabel, setSensorLabel] = React.useState<string | null>(null);
  /** 可信文字元素：system.text 與角色載入失敗文案（永遠不是 adapter 畫的）。 */
  const [trustedText, setTrustedText] = React.useState<TrustedText | null>(null);
  const trustedTextTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  // Mirror the machine base into React state so the pack-immune 緊急停止中 label
  // re-renders even when estop is detected only by the status poll (no event).
  const [baseState, setBaseState] = React.useState<string>("offline");
  const bubbleTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const personaRef = React.useRef<PersonaPack | null>(null);
  const storyRef = React.useRef<StoryPack | null>(null);
  const behaviorRef = React.useRef<BehaviorTuning>(behaviorFor("natural"));
  const storyProgress = React.useRef<Record<string, boolean>>({});
  // §17：同一 knowledge receipt 只說一次（SSE 重連會重放舊事件）。
  const receiptFirstTime = React.useRef(createReceiptDedup());
  /** 角色表（由 adapter 注入；文字角色是空表）。 */
  const landingTableRef = React.useRef<LandingTable>({});
  const eventArtRef = React.useRef<LegacyEventArt>(NEUTRAL_EVENT_ART);

  /** Resolve a bubble line: safety keys fixed, others persona-styled；`{name}` 代成角色名。 */
  const line = React.useCallback((key: string): string | null => {
    return resolveLine(key, personaRef.current, undefined, { name: charNameRef.current });
  }, []);

  /** Show a bubble for `ms` (0 = sticky). Clears any prior timer so an older
   *  bubble's timeout can never erase a newer safety bubble early.
   *  氣泡開關關掉後只剩 `safety` 文字（緊急停止/被擋下/未知）。 */
  const showBubble = React.useCallback(
    (text: string | null, ms: number, opts?: { safety?: boolean }) => {
      if (text !== null && !bubbleAllowed({ enabled: bubblesEnabledRef.current, safety: opts?.safety === true })) {
        return;
      }
      bubbleSafetyRef.current = text !== null && opts?.safety === true;
      showBubbleRaw(text, ms);
    },
    []
  );

  const showBubbleRaw = React.useCallback((text: string | null, ms: number) => {
    if (bubbleTimer.current) {
      clearTimeout(bubbleTimer.current);
      bubbleTimer.current = null;
    }
    setBubble(text);
    if (text && ms > 0) {
      bubbleTimer.current = setTimeout(() => setBubble(null), ms);
    }
  }, []);

  /** 可信文字元素（system.text／載入失敗）：ms=0 為 sticky。 */
  const showTrustedText = React.useCallback((t: TrustedText | null, ms: number) => {
    if (trustedTextTimer.current) {
      clearTimeout(trustedTextTimer.current);
      trustedTextTimer.current = null;
    }
    setTrustedText(t);
    if (t && ms > 0) {
      trustedTextTimer.current = setTimeout(() => setTrustedText(null), ms);
    }
  }, []);

  /** Show a story chapter once, then persist the progress. */
  const playChapter = React.useCallback(
    (trigger: "first-meeting" | "first-verified-success") => {
      const ch = nextChapter(storyRef.current, trigger, storyProgress.current);
      if (!ch) return;
      storyProgress.current = { ...storyProgress.current, [ch.id]: true };
      showBubble(ch.line, 9000);
      void desktop
        .prefsPatch({ storyProgress: storyProgress.current })
        .catch(() => {});
    },
    []
  );

  /**
   * 就地套用 host 偏好（companion-reload 的 "live" 路徑）：更新呈現用的 ref、名字，
   * 並把既有欄位＋各角色偏好（companionPreferences[characterId] → preferences／variant／palette）
   * 一起交給 adapter.reconfigure。不碰角色／persona／尺寸（那些走整頁重載）。
   */
  const applyLivePrefs = React.useCallback((next: DesktopPrefs) => {
    prefsSnapshotRef.current = next;
    storyProgress.current = next.storyProgress ?? {};
    // 互動記憶：host 那一份是唯一真相。控制中心按「忘記這些」（或關掉某個反應）
    // 之後，這個視窗手上的副本必須跟著換掉——否則下一次玩玩具會以陳舊副本整包
    // 寫回，把使用者已經刪掉的記憶復活（memory-ui-001）。
    memoryRef.current = interactionMemoryFromPrefs(next);
    dndRef.current = next.companionDoNotDisturb === true;
    quietUntilRef.current = next.companionProactiveQuietUntil ?? 0;
    bubblesEnabledRef.current = next.companionBubbles !== false;
    soundEnabledRef.current = next.companionSound === true;
    dragEnabledRef.current = next.companionDragEnabled !== false;
    approachEnabledRef.current = next.companionApproach !== false;
    const gateway = gatewayRef.current;
    const manifest = manifestRef.current;
    if (!gateway || !manifest) return;
    const name = charNameFor(next.companionName ?? null, manifest, LOCALE);
    charNameRef.current = name;
    setCharName(name);
    gateway.reconfigure(
      PRIMARY_INSTANCE_ID,
      adapterReconfigureFor(next, {
        name,
        characterId: manifest.characterId,
        entrypoint: entrypointKindOf(manifest),
        tuning: tuningRef.current,
      })
    );
  }, []);

  /**
   * companion-reload（host 每次 companion_apply_prefs 都會發）：重讀偏好，只動了可就地套用的
   * 鍵就 reconfigure，否則維持整頁重載；任何失敗也整頁重載（既有行為）。
   */
  const onCompanionReload = React.useCallback(async () => {
    const prev = prefsSnapshotRef.current;
    if (!prev || !gatewayRef.current || !adapterRef.current) {
      window.location.reload();
      return;
    }
    let next: DesktopPrefs;
    try {
      next = await desktop.prefsGet();
    } catch {
      window.location.reload();
      return;
    }
    const plan = companionReloadPlan(prev, next);
    if (plan.action === "reload") {
      window.location.reload();
      return;
    }
    if (plan.changed.length === 0) {
      prefsSnapshotRef.current = next;
      return;
    }
    try {
      applyLivePrefs(next);
    } catch (e) {
      console.error("live prefs apply failed; reloading", e);
      window.location.reload();
    }
  }, [applyLivePrefs]);
  const onReloadRef = React.useRef(onCompanionReload);
  onReloadRef.current = onCompanionReload;

  // Settings changes: live-applicable prefs reconfigure in place; pack/persona/expressiveness/size reload this window.
  React.useEffect(() => {
    if (!isTauri) return;
    let unReload: (() => void) | null = null;
    let unOpacity: (() => void) | null = null;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      unReload = await listen("companion-reload", () => void onReloadRef.current());
      unOpacity = await listen<number>("companion-opacity", (event) => {
        document.documentElement.style.opacity = String(event.payload);
      });
    })();
    return () => {
      if (unReload) unReload();
      if (unOpacity) unOpacity();
    };
  }, []);

  // ---- machine driving ----
  // Behavior Runtime（生命底層）狀態：平滑量＋Interaction Director（統一
  // 行為導演：注意力/冷卻/防重複/中斷恢復）、seeded RNG。
  const behaviorState = React.useRef<CompanionBehaviorState>(initialBehavior(Date.now()));
  const directorRef = React.useRef(new InteractionDirector(DEFAULT_TUNING, EMPTY_DIRECTOR_TABLES));
  /** 上一個 tick 是否正在表演（用來判斷「自然播完」）。 */
  const performingRef = React.useRef(false);
  /** Director 目前 ambient 動作的 App 側鏡像；點擊／拖曳的反應會清掉 Director
   *  自己的恢復計畫（reactDetailed），所以中斷前要靠它留一份。 */
  const ambientActionRef = React.useRef<AmbientActionMirror | null>(null);
  /** 被點擊／拖曳中斷的長 ambient 的恢復計畫（對抗審查 director-pipeline-018）。 */
  const resumePlanRef = React.useRef<CompanionResumePlan | null>(null);
  const microRng = React.useRef(seededRng(Date.now() >>> 0));

  const apply = React.useCallback((ev: Parameters<typeof reduce>[1]) => {
    const now = Date.now();
    const before = machineRef.current.transient;
    const beforeBase = machineRef.current.base;
    machineRef.current = reduce(machineRef.current, ev, now);
    // 表演被真實事件搶佔 → 記打斷（收斂主動表現）＋讓 Director 之後恢復。
    // 已經自然播完的表演不算被搶（wasPreempted 把兩者分開）。
    const after = machineRef.current.transient;
    if (wasPreempted(before, after, now)) {
      behaviorState.current = noteInterruption(behaviorState.current);
      directorRef.current.notePreempted(now);
    } else if (wasReplacedByPerforming(before, after, now)) {
      // 同優先的另一個表演把它換掉（裝置上線、CPP notice、落地…）：Director 的動作已下台，
      // 不排恢復——否則之後的真實搶佔會拿早已下台的動作排一個假的恢復計畫。
      directorRef.current.noteFinished();
    }
    // 緊急停止：進行中的語音也要停（machine 那邊已清掉非安全 transient）。
    if (machineRef.current.base === "emergency" && beforeBase !== "emergency") {
      stopSpeech();
    }
    setBaseState(machineRef.current.base);
    syncPose();
  }, []);

  const syncPose = React.useCallback(() => {
    const now = Date.now();
    const m = machineRef.current;
    const p = pose(m, now);
    rendererRef.current?.setAnimation(p.animation, p.frameSlice);
    // 舞台旗標必須跟 machine 同步生效。只在 500ms pump 更新的話，緊急停止後
    // 角色還會追球最多半秒，而躺臥／打盹這類姿勢也會被步行覆蓋。
    const t = m.transient && m.transient.untilMs > now ? m.transient : null;
    stageRef.current?.setMachineFlags(machineStageFlags(m.base, t, p.animation, p.ambient));
  }, []);

  /** 給 adapter 的混音器入口：intent 演出與本機互動在同一台 machine 上競爭。 */
  const mixerPort = React.useMemo<MixerPort>(
    () => ({
      apply: (ev: MachineEvent) => {
        apply(ev);
        return machineRef.current;
      },
      state: () => machineRef.current,
    }),
    [apply]
  );

  // ---- 拖曳期間的「持續」transient（放下才結束） ----
  const dragHoldRef = React.useRef<ReturnType<typeof setInterval> | null>(null);

  /** 抱起來：dragged 用續期表示「還在手上」，不是 1.5 秒後自己過期。
   *  表情走 Director 的 `lifted` 變體池（懸空／好奇張望／掙扎；冷卻＋防重複）；
   *  沒有角色表就是 canonical dragged（alias → lifted）。續期沿用同一個變體。 */
  const beginDragHold = React.useCallback(() => {
    const now = Date.now();
    // director-pipeline-018：reactDetailed 會清掉 Director 的恢復計畫，所以在
    // 反應**之前**先把「被抱起來時中斷的長 ambient」記在 App 這一側。
    const preempted = resumePlanFor(ambientActionRef.current, now);
    const decision = directorRef.current.reactDetailed("lifted", now, DRAG_HOLD_MS, microRng.current, { cooldownMs: 1_500 });
    if (decision.action) {
      // 反應成立＝Director 的 interrupted 已被清掉，由 App 接手恢復；
      // 沒反應時 Director 自己還留著計畫，不能兩邊都排（會恢復兩次）。
      if (preempted) resumePlanRef.current = preempted;
      ambientActionRef.current = null;
    }
    const animation = decision.action?.expression;
    apply({ type: "transient", kind: "dragged", durationMs: DRAG_HOLD_MS, animation });
    if (dragHoldRef.current) clearInterval(dragHoldRef.current);
    dragHoldRef.current = setInterval(() => {
      apply({ type: "transient", kind: "dragged", durationMs: DRAG_HOLD_MS, animation });
    }, DRAG_RENEW_MS);
  }, [apply]);

  /** 放下：停止續期並讓出舞台（安全狀態若已搶佔就不動它）。 */
  const endDragHold = React.useCallback(() => {
    if (dragHoldRef.current) {
      clearInterval(dragHoldRef.current);
      dragHoldRef.current = null;
    }
    if (machineRef.current.transient?.kind === "dragged") {
      apply({ type: "clear-transient" });
    }
  }, [apply]);

  React.useEffect(() => () => {
    if (dragHoldRef.current) clearInterval(dragHoldRef.current);
  }, []);

  // 低風險遙測（點擊／滑鼠靠近等）才可 fire-and-forget：失敗即丟棄，
  // 不重試也不誤報成功。凡是會回覆使用者「已記錄」的流程（如拖放）
  // 必須改走 recordDroppedItems 等待實際結果。
  //
  // CPP：有 characterProtocol 時**只**經 gateway.ingestInput → /v1/character/events
  // （Gateway 正規化、節流、隱私）；舊 daemon 才直接 push receptor observation。
  const pushInteraction = React.useCallback((kind: string, extra?: Record<string, unknown>) => {
    behaviorState.current = noteUserInteraction(behaviorState.current, Date.now());
    if (feedRef.current === "protocol" && gatewayRef.current) {
      const ev = inputEventFor(kind, extra);
      if (ev) gatewayRef.current.ingestInput(PRIMARY_INSTANCE_ID, ev);
      return;
    }
    const receptor = RECEPTOR_FOR_KIND[kind] ?? "desktop.companion.interaction";
    void api.pushObservation(receptor, { kind, ...extra }, 1.0).catch(() => {
      /* receptor disabled, companion hidden, or runtime offline: dropped */
    });
  }, []);

  /** Runtime hello（協定模式）：帶 manifest 摘要＋協商結果＋behaviorState。 */
  const sendHello = React.useCallback(async (visible: boolean) => {
    const gateway = gatewayRef.current;
    const adapter = adapterRef.current;
    const manifest = manifestRef.current;
    if (!gateway || !adapter || !manifest || feedRef.current !== "protocol") return;
    const hello = helloFor(PRIMARY_INSTANCE_ID, "primary-companion", {
      reducedMotion: reducedMotionRef.current,
      locale: LOCALE,
    });
    let negotiate;
    try {
      negotiate = adapter.negotiate(hello);
    } catch {
      return; // adapter 壞了：Gateway 的 crash 流程會處理，這裡不硬送
    }
    const state = behaviorState.current;
    helloTrackerRef.current = { ...helloTrackerRef.current, lastAttemptAt: Date.now() };
    try {
      const result = await api.characterHello({
        instanceId: PRIMARY_INSTANCE_ID,
        role: "primary-companion",
        manifest,
        negotiate,
        visible,
        // Reduced Motion 只有一個主人：視窗的實際值一路送到 Runtime 協商，
        // Runtime 才能誠實把 resolution 記成 reduced（不是永遠 exact）。
        reducedMotion: reducedMotionRef.current,
        packId: manifest.characterId,
        behaviorState: {
          activation: state.activation,
          attention: state.attention,
          taskLoad: state.taskLoad,
          interactionReadiness: state.interactionReadiness,
          familiarity: state.familiarity,
          recentInterruptions: state.recentInterruptions,
          currentFocus: state.currentFocus,
          lastInteractionAt: Math.round(state.lastInteractionAt),
          base: machineRef.current.base,
          transient: machineRef.current.transient?.kind ?? null,
        },
      });
      if (typeof result?.generation === "number") runtimeGenerationRef.current = result.generation;
      helloTrackerRef.current = { ...helloTrackerRef.current, sent: true };
    } catch {
      helloTrackerRef.current = { ...helloTrackerRef.current, sent: false }; // 下次狀態輪詢再試
    }
  }, []);

  // ---- boot: transport, character, adapter, gateway ----
  React.useEffect(() => {
    let disposed = false;
    (async () => {
      await bootstrapSupervisor();
      let preferredPack: string | null = null;
      let prefsPersona: string | null = null;
      let prefsName: string | null = null;
      let renderScale = 1.1;
      let prefsSnapshot: Awaited<ReturnType<typeof desktop.prefsGet>> | null = null;
      try {
        const prefs = await desktop.prefsGet();
        prefsSnapshot = prefs;
        preferredPack = prefs.companionPack ?? null;
        prefsPersona = prefs.companionPersona ?? null;
        prefsName = prefs.companionName ?? null;
        behaviorRef.current = behaviorFor(prefs.companionExpressiveness ?? "natural");
        storyProgress.current = prefs.storyProgress ?? {};
        document.documentElement.style.opacity = String(prefs.companionOpacity ?? 1);
        renderScale = 1.1 * ((prefs.companionSize?.[0] ?? 200) / 200);
        dndRef.current = prefs.companionDoNotDisturb === true;
        quietUntilRef.current = prefs.companionProactiveQuietUntil ?? 0;
        bubblesEnabledRef.current = prefs.companionBubbles !== false;
        soundEnabledRef.current = prefs.companionSound === true;
        dragEnabledRef.current = prefs.companionDragEnabled !== false;
        approachEnabledRef.current = prefs.companionApproach !== false;
        // 角色互動記憶：同一天只算一次（熟悉度隨天數緩升，不因單一事件跳動）。
        const mem = noteSession(sanitizeMemory(prefs.companionInteractionMemory), Date.now());
        memoryRef.current = mem;
        void desktop.prefsPatch({ companionInteractionMemory: mem }).catch(() => {});
      } catch {
        /* browser mode */
      }

      // 先問一次 status：決定走 CPP 還是舊路徑（之後輪詢持續更新）。
      try {
        feedRef.current = selectRuntimeFeed(await api.status());
      } catch {
        feedRef.current = null;
      }

      // 角色索引 → 選角色（8 個舊 id 永遠有效；索引壞掉退回文字角色；偏好是匯入角色時
      // 才問 host 清單——只有桌面版有本機角色資料夾，瀏覽器模式跳過，永不擲例外）。
      const indexResult = await loadCharacterIndex(sameOriginFetch);
      const index = indexResult.ok ? indexResult.index : null;
      const resolved = await resolveCharacterSource({
        index,
        preferred: preferredPack,
        tauri: isTauri,
        listImported: () => desktop.characterListImported(),
      });
      const source = resolved.source;
      if (resolved.importedLookup === "failed") console.error("imported character list unavailable", resolved.detail);
      if (source.kind === "text" && source.failed) console.error("character fell back to text", source.reason);
      prefsSnapshotRef.current = prefsSnapshot;
      const entry = source.kind === "index" ? source.entry : null;
      const personaId = personaIdFor(entry, prefsPersona);
      const storyId = storyPackIdFor(entry, personaId);

      // Persona + story packs are data-only; invalid packs fall back to the
      // built-in default lines instead of breaking the companion.
      if (personaId) {
        try {
          const persona = (await fetch(`/packs/${personaId}.json`).then((r) => r.json())) as unknown;
          if (validatePersonaPack(persona).length === 0) {
            personaRef.current = persona as PersonaPack;
          } else {
            console.error("invalid persona pack", validatePersonaPack(persona));
          }
        } catch {
          /* keep defaults */
        }
      }
      if (storyId) {
        try {
          const story = (await fetch(`/packs/${storyId}.json`).then((r) => r.json())) as unknown;
          if (validateStoryPack(story).length === 0) {
            storyRef.current = story as StoryPack;
          }
        } catch {
          /* no story */
        }
      }
      if (disposed || !canvasRef.current) return;

      // 個性：表現度＋persona → profile；tuning 在 adapter 決定後再算（變體權重是角色表）。
      personalityRef.current = personalityFor(prefsSnapshot?.companionExpressiveness ?? "natural", personaId);

      const gateway = new CharacterGateway({
        now: () => Date.now(),
        locale: LOCALE,
        reducedMotion: () => reducedMotionRef.current,
        // 沒有任何呈現能力時的最後退路：安全文字顯示在可信元素上，不會遺失。
        onSystemText: (m) => {
          if (m.instanceId !== null && m.instanceId !== PRIMARY_INSTANCE_ID) return;
          showTrustedText({ text: m.text, marker: m.marker }, m.intent === "emergency" ? 0 : 8000);
        },
        onReceipt: (r: CommandReceipt) => {
          if (feedRef.current !== "protocol") return;
          const forward = receiptForRuntime(r, PRIMARY_INSTANCE_ID, runtimeGenerationRef.current);
          if (!forward) return;
          void api.characterReceipt(PRIMARY_INSTANCE_ID, forward).catch(() => {
            /* runtime 離線：Rust 端看門狗會把它記成 uncertain */
          });
        },
        onInput: (event) => {
          if (feedRef.current !== "protocol") return;
          forwardBatchRef.current.push(
            api
              .characterEvent(PRIMARY_INSTANCE_ID, event)
              .then((r) => ({ decision: String(r?.decision ?? "queued"), reason: r?.reason }))
              .catch(() => null)
          );
          // 遙測類事件沒人等結果：批次有界，不讓 promise 堆積。
          if (forwardBatchRef.current.length > 32) forwardBatchRef.current.splice(0, forwardBatchRef.current.length - 32);
        },
        onLifecycle: (instanceId, state, detail) => {
          if (instanceId !== PRIMARY_INSTANCE_ID || state !== "crashed") return;
          // adapter crash：pending 已是 uncertain、資源已釋放；退回文字角色。
          // 延後一個 microtask：這個回呼是在 Gateway 的 tearDown 裡同步呼叫的，
          // 不在裡面直接 reattach（避免重入）。
          console.error("character adapter crashed", detail);
          queueMicrotask(() => {
            if (disposed) return;
            void fallbackToText(gateway, renderScale, `adapter crashed: ${detail ?? ""}`);
          });
        },
      });
      gatewayRef.current = gateway;

      const canvasEl = canvasRef.current;
      let registered = false;
      try {
        const built = await buildAdapter(source, canvasEl, renderScale, mixerPort, prefsName);
        if (disposed) {
          built.renderer?.destroy();
          return;
        }
        manifestRef.current = built.adapter.manifest;
        await gateway.registerInstance(built.adapter, "primary-companion", { instanceId: PRIMARY_INSTANCE_ID });
        adapterRef.current = built.adapter;
        rendererRef.current = built.renderer;
        stageRef.current = built.stage;
        shuRef.current = built.shu;
        setEntrypointKind(built.kind);
        registered = true;
      } catch (e) {
        console.error("character adapter failed to load", e);
      }
      if (disposed) return;
      if (!registered) {
        await fallbackToText(gateway, renderScale, "load failed");
        if (disposed) return;
      } else if (source.kind === "text" && source.failed) {
        // 選角階段就已經退回文字（匯入角色壞掉／清單讀不到）：固定文案照樣顯示在可信元素上。
        showTrustedText({ text: CHARACTER_LOAD_FAILED_LINE, marker: "none" }, 0);
      }
      wireAdapter(gateway, prefsSnapshot, prefsName);

      // Reduced Motion：由 hello 協商；執行中改系統設定也要立刻生效並重新協商。
      const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
      reducedMotionRef.current = motionQuery.matches;
      gateway.reconfigure(PRIMARY_INSTANCE_ID, { reducedMotion: motionQuery.matches });
      rendererRef.current?.setReducedMotion(motionQuery.matches);
      try {
        gateway.renegotiate(PRIMARY_INSTANCE_ID);
      } catch {
        /* 尚未 live：registerInstance 已協商過 */
      }
      const onMotionChange = (e: MediaQueryListEvent) => {
        reducedMotionRef.current = e.matches;
        rendererRef.current?.setReducedMotion(e.matches);
        gatewayRef.current?.reconfigure(PRIMARY_INSTANCE_ID, { reducedMotion: e.matches });
        try {
          gatewayRef.current?.renegotiate(PRIMARY_INSTANCE_ID);
        } catch {
          /* 實例不在線：下次 hello 再協商 */
        }
        void sendHello(!document.hidden);
      };
      motionQuery.addEventListener?.("change", onMotionChange);
      motionCleanup.current = () => motionQuery.removeEventListener?.("change", onMotionChange);

      await sendHello(!document.hidden);
      if (disposed) return;
      setReady(true);
    })();
    return () => {
      disposed = true;
      motionCleanup.current?.();
      motionCleanup.current = null;
      const gateway = gatewayRef.current;
      if (gateway) gateway.disposeInstance(PRIMARY_INSTANCE_ID, "window closed");
      gatewayRef.current = null;
      adapterRef.current = null;
      shuRef.current = null;
      stageRef.current = null;
      rendererRef.current?.destroy();
      rendererRef.current = null;
    };
  }, []);

  /** 依角色來源建 adapter（builtin entrypoint 白名單：shu-rig／sprite／text）。 */
  async function buildAdapter(
    source: CharacterSource,
    canvasEl: HTMLCanvasElement,
    renderScale: number,
    mixer: MixerPort,
    prefsName: string | null
  ): Promise<{
    adapter: CharacterAdapter;
    renderer: RendererBackend | null;
    stage: StageRenderer | null;
    shu: ShuCharacterAdapter | null;
    kind: EntrypointKind;
  }> {
    if (source.kind === "text") {
      const adapter = new TextCharacterAdapter({ container: textHostRef.current ?? undefined });
      return { adapter, renderer: null, stage: null, shu: null, kind: "text" };
    }
    if (source.kind === "legacy-pack") {
      // 索引不可用但偏好是舊 id：直接由 /packs/<id> 遷移（CPP §2.2）。
      const legacy = (await fetch(`/packs/${source.characterId}/manifest.json`).then((r) => r.json())) as PackManifest & {
        kind?: string;
      };
      if (legacy.kind === "character-rig") {
        const shu = new ShuCharacterAdapter({ legacyRig: legacy, canvas: canvasEl, scale: renderScale, mixer, charName: prefsName ?? undefined });
        return { adapter: shu, renderer: null, stage: null, shu, kind: "shu-rig" };
      }
      const assetBase = `/packs/${source.characterId}`;
      return buildSprite(legacy, assetBase, `${assetBase}/${legacy.sheet}`, canvasEl, renderScale, mixer);
    }
    if (source.kind === "imported") {
      // 匯入角色（host 本機角色資料夾）：只有 builtin 白名單；資產只經 host 讀成 data URL，
      // 不 fetch 任何遠端、不執行任何東西。摘要沒有 manifest 本文時 text／shu-rig 仍可由摘要建出。
      const { entry, entrypoint, manifest } = source;
      if (entrypoint === "text") {
        const adapter = new TextCharacterAdapter({
          container: textHostRef.current ?? undefined,
          characterId: entry.characterId,
          displayName: manifest?.displayName ?? entry.displayName,
          description: manifest?.description,
        });
        return { adapter, renderer: null, stage: null, shu: null, kind: "text" };
      }
      if (entrypoint === "shu-rig") {
        const palette = rigPaletteForImported(manifest);
        const shu = new ShuCharacterAdapter({
          ...(manifest ? { manifest } : { legacyRig: importedRigPack(entry, palette) }),
          palette,
          canvas: canvasEl,
          scale: renderScale,
          mixer,
          charName: prefsName ?? undefined,
        });
        return { adapter: shu, renderer: null, stage: null, shu, kind: "shu-rig" };
      }
      if (!source.sprite) throw new Error(`imported sprite ${entry.characterId}: no pack shape`);
      const sheetUrl = await desktop.characterAsset(entry.characterId, source.sprite.sheetAssetId);
      if (!isImageDataUrl(sheetUrl)) throw new Error(`imported sprite ${entry.characterId}: sheet asset is not an image data URL`);
      return buildSprite(source.sprite.pack, `imported:${entry.characterId}`, sheetUrl, canvasEl, renderScale, mixer);
    }
    const manifest = source.entry.manifest;
    const kind = entrypointKindOf(manifest);
    if (kind === "shu-rig") {
      const shu = new ShuCharacterAdapter({
        manifest,
        palette: rigPaletteFor(manifest),
        canvas: canvasEl,
        scale: renderScale,
        mixer,
        charName: prefsName ?? undefined,
      });
      return { adapter: shu, renderer: null, stage: null, shu, kind };
    }
    if (kind === "sprite") {
      const assetBase = source.entry.assetBase ?? `/packs/${source.characterId}`;
      const legacy = (await fetch(`${assetBase}/manifest.json`).then((r) => r.json())) as PackManifest;
      return buildSprite(legacy, assetBase, `${assetBase}/${legacy.sheet}`, canvasEl, renderScale, mixer);
    }
    if (kind === "text") {
      const adapter = new TextCharacterAdapter({
        container: textHostRef.current ?? undefined,
        characterId: manifest.characterId,
        displayName: manifest.displayName,
        description: manifest.description,
      });
      return { adapter, renderer: null, stage: null, shu: null, kind };
    }
    throw new Error(`unsupported entrypoint for ${source.characterId}`);
  }

  /** sheetUrl：同源 `/packs/<id>/<sheet>`，或匯入角色經 host 讀出的 data URL。 */
  function buildSprite(
    legacy: PackManifest,
    assetBase: string,
    sheetUrl: string,
    canvasEl: HTMLCanvasElement,
    renderScale: number,
    mixer: MixerPort
  ) {
    const issues = validateManifest(legacy);
    if (issues.length > 0) throw new Error(`invalid character pack: ${issues.join("; ")}`);
    // 真正的 SpriteRenderer 由 host 擁有並由 syncPose 驅動；adapter 拿到的是
    // MixerRenderer 門面，它的 setAnimation 進入同一台 machine（不互搶畫面）。
    const real = new SpriteRenderer(canvasEl, legacy, sheetUrl, renderScale);
    const adapter = new SpriteCharacterAdapter({ pack: legacy, assetBase, renderer: new MixerRenderer(real, mixer), scale: renderScale });
    return { adapter, renderer: real as RendererBackend, stage: null, shu: null, kind: "sprite" as const };
  }

  /** 角色載入失敗／崩潰：文字角色＋可信元素上的固定文案。 */
  async function fallbackToText(gateway: CharacterGateway, _renderScale: number, reason: string) {
    void _renderScale;
    try {
      adapterRef.current = null;
      shuRef.current = null;
      stageRef.current = null;
      rendererRef.current?.destroy();
      rendererRef.current = null;
      const adapter = new TextCharacterAdapter({ container: textHostRef.current ?? undefined });
      manifestRef.current = adapter.manifest;
      const info = gateway.getInstance(PRIMARY_INSTANCE_ID);
      if (info && (info.state === "crashed" || info.state === "reconnecting" || info.state === "disposed")) {
        await gateway.reattach(PRIMARY_INSTANCE_ID, adapter);
      } else if (!info) {
        await gateway.registerInstance(adapter, "primary-companion", { instanceId: PRIMARY_INSTANCE_ID });
      } else {
        gateway.disposeInstance(PRIMARY_INSTANCE_ID, reason);
        await gateway.reattach(PRIMARY_INSTANCE_ID, adapter);
      }
      adapterRef.current = adapter;
      setEntrypointKind("text");
      setToyCatalog([]);
      directorRef.current.setTables(EMPTY_DIRECTOR_TABLES);
      landingTableRef.current = {};
      eventArtRef.current = NEUTRAL_EVENT_ART;
      const name = charNameFor(null, adapter.manifest, LOCALE);
      charNameRef.current = name;
      setCharName(name);
      setCharacterId(adapter.manifest.characterId);
      characterIdRef.current = adapter.manifest.characterId;
    } catch (e) {
      console.error("text fallback failed", e);
    }
    // 固定文案，顯示在可信 DOM 元素上（不是 adapter 說的）。
    showTrustedText({ text: CHARACTER_LOAD_FAILED_LINE, marker: "none" }, 0);
    helloTrackerRef.current = { ...helloTrackerRef.current, sent: false };
    void sendHello(!document.hidden);
  }

  /**
   * 真的需要接住游標的 UI 面（視窗相對 CSS px）。
   *
   * 由 JSX 上的 `data-hit-region="…"` 標出來：快捷選單（有按鈕）、氣泡、可信
   * 系統文字、緊急停止／感測標籤（訊息面不該被點穿）、文字角色的本體。
   * 文字輸入與拖放確認**不在**這裡——它們需要原生文字選取／OS drop target，
   * 走 `companion_set_interactive`（整窗）。其餘透明區一律留給桌面。
   */
  function uiHitRegions(): HitRegion[] {
    const root = rootRef.current;
    if (!root) return [];
    const out: HitRegion[] = [];
    root.querySelectorAll("[data-hit-region]").forEach((el) => {
      const id = el.getAttribute("data-hit-region");
      if (!id) return;
      const r = el.getBoundingClientRect();
      if (!(r.width > 0) || !(r.height > 0)) return;
      out.push({ id: `ui:${id}`, x: r.left, y: r.top, w: r.width, h: r.height });
    });
    return out;
  }

  /**
   * 送出 bounded hit regions（companion-gameplay-032）。
   *
   * stage 給的是 canvas 相對座標 → 平移成視窗相對 → 併入 UI 面（角色第一、UI
   * 其次）→ 依 Rust 同一組上限先截斷 → `companion_hit_regions`。
   * `stageRegions=null` 表示「現在就重算一次」（UI 面剛出現／消失時）。
   * 空清單不送：Rust 會拒絕空報告並保留上一份，這是刻意的 fail-closed。
   */
  const pushHitRegions = React.useCallback(async (stageRegions: HitRegion[] | null) => {
    if (!isTauri) return;
    // 同時只有一個 IPC 在飛：不排隊、不堆積，也不會有舊報告後到把新的蓋掉。
    // 丟掉的那一次由下一次回報補上（節流政策保證 ≤60ms 一定會再來一次）。
    if (hitRegionsBusyRef.current) return;
    const stage = stageRegions ?? stageRef.current?.interactiveRegions() ?? [];
    const box = canvasRef.current?.getBoundingClientRect();
    const moved = box ? translateRegions(stage, box.left, box.top) : stage;
    const merged = mergeHitRegions(moved, uiHitRegions());
    const regions = prepareHitRegions(merged, window.innerWidth, window.innerHeight);
    if (regions.length === 0) return;
    hitRegionsBusyRef.current = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await sendHitRegions(invoke, regions);
    } finally {
      hitRegionsBusyRef.current = false;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** 註冊成功後：角色表、名字、遊玩場回呼、偏好套用。 */
  function wireAdapter(
    gateway: CharacterGateway,
    prefs: Awaited<ReturnType<typeof desktop.prefsGet>> | null,
    prefsName: string | null
  ) {
    const adapter = adapterRef.current;
    const manifest = manifestRef.current;
    if (!adapter || !manifest) return;
    const shu = shuRef.current;
    const name = charNameFor(prefsName, manifest, LOCALE);
    charNameRef.current = name;
    setCharName(name);
    setCharacterId(manifest.characterId);
    characterIdRef.current = manifest.characterId;
    // 角色表注入：Director／落地／舊路徑事件美術；文字角色一律空表。
    directorRef.current.setTables(shu?.directorTables ?? EMPTY_DIRECTOR_TABLES);
    landingTableRef.current = shu?.landingTable ?? {};
    eventArtRef.current = shu?.eventArt ?? NEUTRAL_EVENT_ART;
    setToyCatalog(shu?.toyCatalog ?? []);
    tuningRef.current = tuningFor(personalityRef.current, shu?.variantWeights ?? {});
    directorRef.current.setTuning(tuningRef.current);
    // 既有欄位＋各角色偏好（prefs.companionPreferences[characterId] → preferences／variant／palette）。
    gateway.reconfigure(
      PRIMARY_INSTANCE_ID,
      adapterReconfigureFor(prefs, {
        name,
        characterId: manifest.characterId,
        entrypoint: entrypointKindOf(manifest),
        tuning: tuningRef.current,
      })
    );
    const stage = shu?.stageRenderer() ?? null;
    stageRef.current = stage;
    if (stage) {
      rendererRef.current = stage;
      stage.onExpressionEvent((id, durationMs) => {
        apply({ type: "transient", kind: "performing", animation: id, durationMs });
      });
      // 互動區：由 stage 每幀依節流政策回報（角色會走動、玩具會滾，
      // 只靠 500ms pump 的話 Rust 會用過期的框判定點擊穿透）。
      // 回報的是**多個 bounded regions**（角色／使魔／玩具／UI 面各一個），
      // 不是一個聯集矩形——聯集內的空白區要能點穿到桌面
      // （對抗審查 companion-gameplay-032）。
      if (isTauri && canvasRef.current) {
        stage.onHitRegions((regions) => {
          void pushHitRegions(regions);
        });
      }
    }
    gateway.show(PRIMARY_INSTANCE_ID);
    syncPose();
  }

  // ---- presentation commands: runtime → this surface → honest ack ----
  const handlePresentationCommand = React.useCallback(
    async (payload: Record<string, unknown>) => {
      const command = String(payload["command"] ?? "");
      const actionId = typeof payload["actionId"] === "string" ? payload["actionId"] : null;
      if (command === "cancel" || command === "clear-all") {
        stopSpeech(); // 進行中的語音也要停：清氣泡不會讓已經在講的句子閉嘴。
        // Cancelled/estopped: drop any non-safety visual (safety poses are
        // driven by base state, not by presentation commands) — including a
        // still-running `performing` transient, which used to survive here.
        // `cancel`（單一 action，AI 可送）不動安全訊息：被擋下／失敗／未知的 transient 與
        // 固定安全氣泡留著；只有 estop 的 `clear-all` 才全清（基態 emergency 仍由 runtime 擁有）。
        const cancelPlan = planPresentationCommand(command, {}, isTauri);
        const effects = cancelEffects(command);
        if (effects.clearSafetyBubble || !bubbleSafetyRef.current) {
          bubbleSafetyRef.current = false;
          showBubbleRaw(null, 0);
        }
        if (cancelPlan.transient === null) apply({ type: "clear-transient", force: effects.forceClear });
        return;
      }
      if (!actionId) return;
      // CPP：有 characterProtocol 時 state-present／animation-play 由 Runtime
      // 投影成 character.intent（受 floor≤50／truthState none 管制）再送來；
      // 這裡不重演、也不 ack——回執走 /v1/character/receipts。
      if (feedRef.current === "protocol" && (command === "state-present" || command === "animation-play")) {
        return;
      }
      const params = (payload["params"] as Record<string, unknown> | undefined) ?? {};
      const plan = planPresentationCommand(command, params, isTauri);
      if (plan.transient !== undefined) {
        if (plan.transient === null) apply({ type: "clear-transient" });
        else apply({ type: "transient", kind: plan.transient, animation: plan.animation });
      }
      if (plan.bubble) {
        // 使用者關掉氣泡時不顯示，而且誠實回報沒顯示（不假裝 displayed）。
        const decision = bubbleOutcome(bubblesEnabledRef.current);
        if (decision.show) {
          showBubbleRaw(plan.bubble.text, plan.bubble.ms);
          pushInteraction("bubble-shown", { source: "presentation-command" });
        } else {
          plan.outcome = decision.outcome ?? "failed";
          plan.detail = decision.detail;
        }
      }
      if (plan.presence !== undefined && isTauri) {
        // 真的 show／hide：host 命令 resolve 才算「發生了」；拒絕就是 failed，不是 completed。
        await applyPresence(plan, (visible) => desktop.companionSetVisible(visible));
      }
      try {
        if (plan.sound) {
          // 音效預設關閉：關著就不播，而且誠實回 failed（不假裝播過）。
          const decision = soundOutcome(soundEnabledRef.current);
          if (decision.play) {
            await playRegisteredSound(plan.sound);
          } else {
            plan.outcome = decision.outcome ?? "failed";
            plan.detail = decision.detail;
          }
        }
        if (plan.speech) await speakText(plan.speech);
        if (plan.window) {
          if (!isTauri) throw new Error("window adjustment needs the desktop shell");
          const applied = (await desktop.companionWindowAdjust(actionId)) as Record<string, unknown>;
          if (typeof applied.opacity === "number") {
            document.documentElement.style.opacity = String(applied.opacity);
          }
        }
      } catch (error) {
        plan.outcome = "failed";
        plan.detail = String(error);
      }
      await api.presentationAck(actionId, plan.outcome, plan.detail).catch(() => {
        /* runtime gone: the watchdog will honestly mark it uncertain */
      });
    },
    [apply, showBubble, pushInteraction]
  );

  // ---- presence heartbeat: the runtime's honest availability source ----
  React.useEffect(() => {
    if (!ready) return;
    let stopped = false;
    const beat = () => {
      if (stopped) return;
      const state = behaviorState.current;
      // Roll Call（人話）：機器狀態優先；ambient 時由遊玩場描述。
      const m = machineRef.current;
      const t = m.transient && m.transient.untilMs > Date.now() ? m.transient : null;
      const machineLabel =
        m.base === "emergency"
          ? "緊急停止中"
          : m.base === "offline"
            ? "離線"
            : m.base === "paused"
              ? "暫停中"
              : m.base === "quiet"
                ? "在安靜陪伴"
                : t == null || t.kind === "performing"
                  ? null
                  : {
                      listening: "在留意動靜",
                      thinking: "在思考",
                      routing: "在找資料",
                      "requesting-consent": "在等你確認",
                      acting: "在工作",
                      "waiting-for-receipt": "在等結果",
                      succeeded: "剛完成一件事",
                      blocked: "被安全規則擋下",
                      unknown: "結果還不確定",
                      failed: "剛遇到失敗",
                      clicked: "在跟你互動",
                      dragged: "被抱起來了",
                    }[t.kind] ?? null;
      const rollCall = shuRef.current
        ? shuRef.current.rollCallNow(machineLabel)
        : [{ name: charNameRef.current, activity: machineLabel ?? "在休息" }];
      // presence 心跳維持舊路由（packId = characterId）；CPP hello 另走 /v1/character/hello。
      void api
        .presentationHello(!document.hidden, characterIdRef.current || undefined, {
          activation: state.activation,
          attention: state.attention,
          taskLoad: state.taskLoad,
          interactionReadiness: state.interactionReadiness,
          familiarity: state.familiarity,
          recentInterruptions: state.recentInterruptions,
          currentFocus: state.currentFocus,
          lastInteractionAt: Math.round(state.lastInteractionAt),
          base: machineRef.current.base,
          transient: machineRef.current.transient?.kind ?? null,
          rollCall: rollCall.slice(0, 4),
        })
        .catch(() => {});
    };
    const onVisibility = () => {
      const gateway = gatewayRef.current;
      if (gateway) {
        // 隱藏 → suspend（停 rAF／物理／計時）；回來 → resume。先切狀態再 beat，
        // 「剛隱藏」那一拍回報的才是暫停後的事實，不是暫停前的活動。
        if (document.hidden) gateway.suspend(PRIMARY_INSTANCE_ID);
        else gateway.resume(PRIMARY_INSTANCE_ID);
      }
      beat();
      if (gateway) {
        if (feedRef.current === "protocol") {
          gateway.ingestInput(PRIMARY_INSTANCE_ID, inputEventFor("visibility-changed", { visible: !document.hidden })!);
        }
      }
    };
    beat();
    const t = setInterval(beat, 10_000);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stopped = true;
      clearInterval(t);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [ready, characterId]);

  // Runtime events → gateway intents（CPP）或 machine transients（legacy）；status poll → base state.
  React.useEffect(() => {
    if (!ready) return;
    let stopped = false;
    const un = onRuntimeEvent((e: RuntimeEvent) => {
      if (e.eventType === "presentation.command") {
        void handlePresentationCommand(e.payload);
        return;
      }
      if (e.eventType === "knowledge.updated") {
        maybeKnowledgeBubble(e);
        return;
      }
      if (e.eventType === "character.intent") {
        if (feedRef.current !== "protocol") return;
        const gateway = gatewayRef.current;
        const envelope = envelopeForInstance(e.payload, PRIMARY_INSTANCE_ID);
        if (!gateway || !envelope) return;
        const importance = envelope.intent === "emergency" ? 1 : envelope.priority >= 50 ? 0.5 : 0.3;
        behaviorState.current = noteEvent(behaviorState.current, `intent:${envelope.intent}`, importance);
        gateway.dispatch(envelope, "runtime");
        maybeBubbleForIntent(envelope);
        return;
      }
      if (e.eventType === "character.system-text") {
        const st = systemTextFromEvent(e.payload);
        if (st && (st.instanceId === null || st.instanceId === PRIMARY_INSTANCE_ID)) {
          showTrustedText({ text: st.text, marker: st.marker }, 8000);
        }
        return;
      }
      if (e.eventType === "character.instance") {
        // Runtime 把我們標成 connected:false（presence 逾時 sweep、撤銷…）：心跳不會把
        // 實例接回來，只有 hello 會——重新 hello（節流），否則角色永遠收不到 character.intent。
        const d = rehelloOnInstanceEvent(helloTrackerRef.current, e.payload, PRIMARY_INSTANCE_ID, Date.now());
        helloTrackerRef.current = d.tracker;
        if (d.hello) void sendHello(!document.hidden);
        return;
      }
      if (e.eventType.startsWith("character.")) return; // receipt：控制中心的事
      // legacy：runtime 未提供 character.intent 時的相容映射（transient 由事件直接驅動）。
      const mapped = mapRuntimeEvent(e, eventArtRef.current);
      if (mapped) {
        // 注意力：事件推高喚起度（平滑，不會 0→1）。
        const importance = e.eventType === "emergency.stop" ? 1 : e.eventType.startsWith("action.") ? 0.5 : 0.3;
        behaviorState.current = noteEvent(behaviorState.current, e.eventType, importance);
        // protocol 模式：transient 由 character.intent 驅動，這裡只收基態（緊急／暫停）
        // ——安全狀態必須立刻反映，不等 5 秒輪詢。
        if (feedRef.current !== "protocol" || mapped.type === "base") apply(mapped);
        maybeBubble(e);
      }
    });
    const poll = async () => {
      try {
        const s = await api.status();
        if (stopped) return;
        // feed 出現／hello 沒成功／daemon 重啟（startedAt 變了）→（重新）hello。
        const re = rehelloOnStatus(helloTrackerRef.current, feedRef.current, s, Date.now());
        feedRef.current = re.feed;
        helloTrackerRef.current = re.tracker;
        if (re.hello) void sendHello(!document.hidden);
        const estop = Boolean(s["emergencyStop"]);
        const paused = Boolean(
          (s["proactivePause"] as Record<string, unknown> | undefined)?.["paused"]
        );
        quietHoursRef.current = Boolean(s["quietHours"]);
        // quiet 基態 = runtime quiet hours 或使用者的勿擾開關。
        const quiet = quietBase({
          quietHours: quietHoursRef.current,
          doNotDisturb: dndRef.current,
        });
        const sensors = (s["activeSensors"] as { kind: string }[] | undefined) ?? [];
        // 感測不靜默，但不外洩原始 id：與 tray／首頁／host overlay 共用同一份投影
        // （iphone.mic-level 也算麥克風；認不得的種類說「其他感測器」）。
        setSensorLabel(companionSensorLabel(sensors));
        apply({
          type: "base",
          base: estop ? "emergency" : paused ? "paused" : quiet ? "quiet" : "idle",
        });
        if (!firstOnline.current && !estop) {
          firstOnline.current = true;
          playChapter("first-meeting");
        }
      } catch {
        if (!stopped) apply({ type: "base", base: "offline" });
      }
    };
    // perf-claims-017：隱藏時降頻（不是關掉——SSE 斷線時這條輪詢是緊急停止／
    // 暫停的唯一後盾），回到可見時立刻補一次並讓姿勢跟上。
    let pollTimer: ReturnType<typeof setTimeout> | null = null;
    // 世代編號：回到可見時插隊的那一次會作廢還在飛的舊鏈，避免兩條輪詢鏈並存。
    let pollGen = 0;
    const schedulePoll = (gen: number) => {
      if (stopped || gen !== pollGen) return;
      pollTimer = setTimeout(() => runPoll(), statusPollIntervalMs(document.hidden));
    };
    const runPoll = () => {
      const gen = ++pollGen;
      void poll().finally(() => schedulePoll(gen));
    };
    const pollVisibility = () => {
      if (document.hidden) return;
      if (pollTimer) clearTimeout(pollTimer);
      syncPose();
      runPoll();
    };
    runPoll();
    document.addEventListener("visibilitychange", pollVisibility);
    // Pose re-evaluation for transient expiry + ambient blink.
    // Behavior Runtime tick（500ms）：平滑步進 → 姿勢刷新 → 微動作排程。
    // 觸發間隔由 hazard 抽樣決定（幾何分布）——絕不是固定週期同一動畫。
    const pump = setInterval(() => {
      const now = Date.now();
      // perf-claims-017：隱藏 ≠ 靜音。看門狗（誠實階梯）與行為記帳照跑，
      // 沒有觀眾的演出（micro-motion／姿勢刷新／互動框回報／hover 氣泡／
      // Director ambient 排程）在隱藏期間全部停下。
      const work = companionPumpWork(document.hidden);
      // CPP Gateway sweep（看門狗、acknowledged→uncertain、adapter.tick、佇列推進）。
      if (work.sweep) gatewayRef.current?.sweep(now);
      const m = machineRef.current;
      const t = m.transient && m.transient.untilMs > now ? m.transient : null;
      // 表演自然播完（不是被搶佔）→ 告訴 Director 舞台空了。
      if (performingRef.current && t?.kind !== "performing") {
        performingRef.current = false;
        ambientActionRef.current = null;
        directorRef.current.noteFinished();
      } else if (t?.kind === "performing") {
        performingRef.current = true;
      }
      const busy =
        t != null && ["acting", "waiting-for-receipt", "routing", "thinking"].includes(t.kind);
      const waitingForHuman = t?.kind === "requesting-consent";
      // 行為記帳在隱藏期間也要走：presence 心跳照實回報 activation／attention，
      // 凍結在「隱藏那一刻」的值等於對 Runtime 說謊。
      if (work.behavior) {
        behaviorState.current = stepBehavior(behaviorState.current, {
          busy,
          waitingForHuman,
          msSinceInteraction: now - behaviorState.current.lastInteractionAt,
        });
      }
      if (!work.present) return;
      const reducedMotion = reducedMotionRef.current;
      rendererRef.current?.setMicroMotion(
        layeredMicroMotion(
          behaviorState.current,
          now,
          reducedMotion,
          ["emergency", "offline", "paused"].includes(m.base)
        )
      );
      // syncPose 同時把 machine 旗標推給 stage（真相狀態即時凍結遊玩）。
      syncPose();
      const p = pose(m, now);
      const stage = stageRef.current;
      // 心跳：rAF 被系統節流／視窗隱藏時，至少每 500ms 還有一次互動框回報。
      stage?.reportHitRect(true);
      // 使用者要求的本機安靜期（「一小時內不要主動說話」）。
      const localQuiet = proactiveQuietActive(quietUntilRef.current, now);
      // quiet 基態的 pose.ambient 是 false——用它當唯一閘門會讓 Director 的
      // quiet 分支（偶爾眨眼）永遠到不了。閘門改由 directorTickGate 決定。
      const gate = directorTickGate({
        poseAmbient: p.ambient,
        base: m.base,
        hasActiveTransient: t !== null,
        localQuiet,
      });
      if (!gate.tick) return;
      // Hover 短氣泡（§5.1-3）：停在角色身上超過 700ms、冷卻過了才說一句。
      // 本機模板選句，不呼叫 AI、不保存游標位置。
      if (hoverSinceRef.current > 0) {
        const decision = hoverBubblePolicy({
          hoverMs: now - hoverSinceRef.current,
          nowMs: now,
          lastBubbleAt: lastBubbleAt.current,
          bubblesEnabled: bubblesEnabledRef.current,
          approachEnabled: approachEnabledRef.current,
          quiet: m.base === "quiet" || quietHoursRef.current || dndRef.current || localQuiet,
          personality: personalityRef.current,
          rand: microRng.current(),
          lastText: lastHoverLineRef.current,
        });
        if (decision.show && decision.text) {
          lastBubbleAt.current = now;
          hoverSinceRef.current = now; // 同一次停留不再重複說
          lastHoverLineRef.current = decision.text;
          showBubble(decision.text, 3200);
        }
      }
      // 遊玩中（追逐/叼回）不再疊 Director 的 ambient 動作。
      if (stage?.worldBusy()) return;
      // §6.1 動作可中斷、可恢復：被點擊／拖曳打斷的長 ambient 接回去
      // （Director 的 interrupted 已被那次反應清掉，計畫留在 App 這側）。
      const resume = takeResumePlan(resumePlanRef.current, {
        nowMs: now,
        quiet: gate.quiet,
        reducedMotion,
      });
      resumePlanRef.current = resume.plan;
      if (resume.action) {
        ambientActionRef.current = {
          animation: resume.action.animation,
          durationMs: resume.action.remainingMs,
          startedAt: now,
        };
        apply({
          type: "transient",
          kind: "performing",
          animation: resume.action.animation,
          durationMs: resume.action.remainingMs,
        });
        return;
      }
      const expr = behaviorRef.current.allowCasualBubbles
        ? behaviorRef.current.bubbleCooldownMs < 60_000
          ? 1.5
          : 1
        : 0.5;
      // Interaction Director：ambient 變體選擇（hazard 抽樣、冷卻、防重複、
      // 中斷後恢復）。真相狀態不經此路（Director 排程端過濾）。
      const action = directorRef.current.tick(
        {
          nowMs: now,
          ambient: true,
          quiet: gate.quiet,
          reducedMotion,
          expressiveness: expr,
          msSinceInteraction: now - behaviorState.current.lastInteractionAt,
          behavior: behaviorState.current,
        },
        microRng.current
      );
      if (action) {
        // 安靜時只允許眨眼，而且要「就地眨」：套成一般表演的話，角色會從安靜
        // 陪伴的坐姿彈回中性站姿。rig 收得下這個提示就不換表情。
        // 「就地眨眼」靠 Director 的 source 標記認出來，不比對表情 id——眨眼
        // 的 id 由角色 adapter 的 tables 注入，host 不該知道它叫什麼
        // （對抗審查 director-pipeline-045）。
        const rig = rendererRef.current as { blinkNow?: () => boolean } | null;
        const blinkedInPlace =
          gate.quiet && action.source === "blink" && rig?.blinkNow?.() === true;
        if (!blinkedInPlace) {
          // 新的 ambient 上台：記下它（中斷時要靠這份鏡像留恢復計畫），
          // 並取代任何還沒用掉的舊恢復計畫。
          ambientActionRef.current = {
            animation: action.expression,
            durationMs: action.durationMs,
            startedAt: now,
          };
          resumePlanRef.current = null;
          apply({
            type: "transient",
            kind: "performing",
            animation: action.expression,
            durationMs: action.durationMs,
          });
        }
      }
    }, 500);
    return () => {
      stopped = true;
      if (pollTimer) clearTimeout(pollTimer);
      document.removeEventListener("visibilitychange", pollVisibility);
      clearInterval(pump);
      un.then((f) => f()).catch(() => {});
    };
  }, [ready, apply, syncPose, handlePresentationCommand]);

  /** §17 知識收據六句固定文案：依 payload 確定性選句（selector 見
   *  knowledgeReceipts.ts）、同一 receipt 只說一次。知識進度不是安全警示，
   *  所以尊重既有頻率與 quiet 設定——被安靜／冷卻吃掉的 receipt 也標記為
   *  已處理，不在之後回放（控制中心的收據頁永遠完整呈現）。 */
  function maybeKnowledgeBubble(e: RuntimeEvent) {
    const text = knowledgeReceiptLine(e.payload);
    if (!text) return; // 六句沒有誠實對應者：沉默，不硬湊
    if (!receiptFirstTime.current(String(e.payload["updateId"] ?? ""))) return;
    const base = machineRef.current.base;
    if (base === "quiet" || base === "paused" || base === "emergency") return;
    if (!behaviorRef.current.allowCasualBubbles) return;
    const now = Date.now();
    // 使用者要求「不要主動說話」：知識收據不是安全警示，安靜期內不說。
    if (proactiveQuietActive(quietUntilRef.current, now)) return;
    if (now - lastBubbleAt.current < behaviorRef.current.bubbleCooldownMs) return;
    lastBubbleAt.current = now;
    showBubble(text, 5000);
  }

  function maybeBubble(e: RuntimeEvent) {
    const now = Date.now();
    const base = machineRef.current.base;
    // Safety lines are FIXED (packs cannot restyle them) and always show.
    if (e.eventType === "emergency.stop" && e.payload["cleared"] !== true) {
      showBubble(line("emergency"), 0, { safety: true }); // sticky
      return;
    }
    // Blocked/unknown/failed are safety-relevant fixed wording: shown even in
    // quiet expressiveness (but not while paused/quiet-hours/estop — the state
    // itself already explains the silence).
    if (base === "quiet" || base === "paused" || base === "emergency") return;
    const safetyText =
      e.eventType === "plan.blocked"
        ? line("blocked")
        : e.eventType === "action.uncertain"
          ? line("unknown")
          : e.eventType === "action.failed"
            ? line("failed")
            : null;
    if (safetyText) {
      lastBubbleAt.current = now;
      showBubble(safetyText, 5000, { safety: true });
      return;
    }
    // Casual lines: persona-styled, expressiveness-gated, cooldown-limited.
    // 使用者要求的安靜期只擋這些隨口話——上面的安全文字不受影響。
    if (proactiveQuietActive(quietUntilRef.current, now)) return;
    if (!behaviorRef.current.allowCasualBubbles) return;
    if (now - lastBubbleAt.current < behaviorRef.current.bubbleCooldownMs) return;
    let text: string | null = null;
    if (e.eventType === "action.observed") {
      text = line("succeeded-verified");
      playChapter("first-verified-success");
    } else if (e.eventType === "action.completed") text = line("succeeded");
    if (text) {
      lastBubbleAt.current = now;
      showBubble(text, 4000);
    }
  }

  /** CPP intent 的隨口氣泡：只有 verified-success（且 truthState 真的是 verified）
   *  才觸發故事章節；claim 只是聲稱，不慶祝。安全文字由事件路徑（maybeBubble）負責。 */
  function maybeBubbleForIntent(envelope: IntentEnvelope) {
    if (envelope.intent !== "verified-success" || envelope.truthState !== "verified") return;
    const base = machineRef.current.base;
    if (base === "quiet" || base === "paused" || base === "emergency") return;
    playChapter("first-verified-success");
  }

  // ---- semantic interaction events (NEVER raw coordinates to the runtime) ----
  // 遊玩場內的指標座標只活在本視窗 canvas，供角色的遊玩擴充（光點／玩具拖曳）；
  // 不推送 runtime、不持久化。
  const lastApproachAt = React.useRef(0);
  const firstOnline = React.useRef(false);

  function stagePoint(e: React.PointerEvent): { x: number; y: number } {
    const rect = (canvasRef.current ?? (e.target as HTMLElement)).getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }

  function onPointerEnterCanvas(e: React.PointerEvent) {
    const stage = stageRef.current;
    if (stage) {
      const p = stagePoint(e);
      stage.pointerMove(p.x, p.y);
      hoverSinceRef.current = stage.hitTestChar(p.x, p.y) ? Date.now() : 0;
    }
    // 勿擾/安靜時段：不主動靠近、不主動看過來。
    if (
      !approachAllowed({
        approachEnabled: approachEnabledRef.current,
        quietHours: quietHoursRef.current,
        doNotDisturb: dndRef.current,
      })
    ) {
      return;
    }
    const now = Date.now();
    if (now - lastApproachAt.current > 30_000) {
      lastApproachAt.current = now;
      hoverEnteredRef.current = true;
      pushInteraction("pointer-approached");
      // 游標靠近時看過來（不追蹤、不記錄座標）。
      apply({ type: "transient", kind: "listening", durationMs: 1200 });
    }
  }

  /** 這次停留有沒有送過 hover-entered（離開時才對應送 hover-left；成對、不多送）。 */
  const hoverEnteredRef = React.useRef(false);

  function onPointerLeaveCanvas() {
    stageRef.current?.pointerLeave();
    toyDragRef.current = false;
    hoverSinceRef.current = 0;
    if (hoverEnteredRef.current) {
      hoverEnteredRef.current = false;
      // 游標離開：§5.1 靠近／進入／離開都是輸入事件（Gateway 限 4/s；不帶座標）。
      pushInteraction("pointer-left");
    }
  }

  // ---- coarse activity summary (this app's windows only) ----
  const lastActiveRef = React.useRef(Date.now());
  React.useEffect(() => {
    if (!ready) return;
    const mark = () => {
      lastActiveRef.current = Date.now();
    };
    window.addEventListener("pointermove", mark, { passive: true });
    let wasActive = true;
    const t = setInterval(() => {
      const idle = Date.now() - lastActiveRef.current;
      const active = idle < 60_000;
      if (active !== wasActive) {
        wasActive = active;
        void api
          .pushObservation(
            "desktop.pointer.activity",
            { activeRecently: active, idleForMs: idle },
            1.0
          )
          .catch(() => {});
      }
    }, 15_000);
    return () => {
      window.removeEventListener("pointermove", mark);
      clearInterval(t);
    };
  }, [ready]);

  // ---- file drop: preview first, never auto-ingest ----
  /** 拖放預覽：路徑（記錄用，不顯示）＋顯示用項目（檔名／大小／類型；不知道就說不知道）。 */
  const [dropPreview, setDropPreview] = React.useState<{ paths: string[]; items: DropPreviewItem[] } | null>(null);
  /** 可讀的 AI 工作階段清單（null＝還沒拿到／拿不到；明說，不當成「沒有」）。 */
  const [dropSessions, setDropSessions] = React.useState<DropPreviewSession[] | null>(null);
  React.useEffect(() => {
    if (!isTauri || !ready) return;
    let un: (() => void) | null = null;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      un = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "drop") {
          const payload = event.payload as { paths?: string[]; files?: unknown[] };
          const paths = payload.paths ?? [];
          // Tauri 2 的 drop 事件只有 paths＋position：大小／類型沒有來源就顯示「未知」，不補假值。
          if (paths.length > 0) setDropPreview({ paths, items: dropPreviewItems(paths, payload.files ?? []) });
        }
      });
    })();
    return () => {
      if (un) un();
    };
  }, [ready]);
  React.useEffect(() => {
    if (!dropPreview) return;
    let cancelled = false;
    setDropSessions(null);
    api
      .agentSessionsList()
      .then((list) => {
        if (!cancelled) setDropSessions(list as DropPreviewSession[]);
      })
      .catch(() => {
        /* 清單拿不到：預覽明說「暫時拿不到」 */
      });
    return () => {
      cancelled = true;
    };
  }, [dropPreview]);

  /** 拖放確認流程的 push：protocol 模式經 gateway（只有檔名＋短效 grant）並等 Runtime 的決定。 */
  const dropPush = React.useCallback(
    async (receptorId: string, facts: Record<string, unknown>, confidence?: number): Promise<unknown> => {
      const gateway = gatewayRef.current;
      if (feedRef.current === "protocol" && gateway) {
        const ev = inputEventFor("companion-dropped", facts);
        if (!ev) throw new Error("no file names to record");
        forwardBatchRef.current = [];
        // Gateway 一檔一事件（README §6 扁平形狀）：等**全部**的決定才能說「記下了」。
        if (!gateway.ingestInput(PRIMARY_INSTANCE_ID, ev)) throw new Error("input rejected locally");
        const result = await awaitForwardBatch();
        if (!result) throw new Error("runtime unreachable");
        if (result.decision === "dropped") throw new Error(result.reason ?? "runtime dropped the event");
        return result;
      }
      return api.pushObservation(receptorId, facts, confidence);
    },
    [awaitForwardBatch]
  );

  // ---- pointer: toy drag / click vs window drag ----
  const dragState = React.useRef<{ x: number; y: number; dragging: boolean } | null>(null);
  const toyDragRef = React.useRef(false);
  /** 這次按下是落在互動框的空白處（不是角色身上）。 */
  const bgPressRef = React.useRef(false);
  const clickTimes = React.useRef<number[]>([]);

  async function onPointerDown(e: React.PointerEvent) {
    const stage = stageRef.current;
    if (stage) {
      const p = stagePoint(e);
      const hit = stage.pointerDown(p.x, p.y);
      if (hit === "toy") {
        // 玩具拖曳：不做視窗拖曳、不開選單。
        toyDragRef.current = true;
        (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
        return;
      }
      // 互動框（角色 ∪ 玩具的包圍盒）內的空白：以前直接 return，於是那一大條
      // 區域既不穿透桌面、點下去也毫無反應（對抗審查 companion-gameplay-032）。
      // 現在當成一般的視窗互動：可以拖視窗、放開時開選單，但不算「戳到角色」。
      if (hit === "none") return; // 互動框外（Rust 端已讓它穿透，正常收不到）
      bgPressRef.current = hit === "stage";
    }
    dragState.current = { x: e.clientX, y: e.clientY, dragging: false };
  }

  async function onPointerMove(e: React.PointerEvent) {
    const stage = stageRef.current;
    if (stage) {
      const p = stagePoint(e);
      stage.pointerMove(p.x, p.y);
      // hover 計時：離開角色身上就重新計（座標只留在本視窗）。
      if (stage.hitTestChar(p.x, p.y)) {
        if (hoverSinceRef.current === 0) hoverSinceRef.current = Date.now();
      } else {
        hoverSinceRef.current = 0;
      }
    }
    if (toyDragRef.current) return;
    if (!dragEnabledRef.current) return; // 使用者關掉了「可拖曳角色」
    const d = dragState.current;
    if (!d || d.dragging) return;
    if (Math.abs(e.clientX - d.x) + Math.abs(e.clientY - d.y) > 5) {
      d.dragging = true;
      // 被抱起：懸空反應（rig 有專屬 lifted 演出）。整段拖曳期間都是
      // 「被抱著」——用續期的持續 transient，不是 1.5 秒後自己過期，
      // 否則角色會在半空中回 idle、遊玩場也重新啟動。
      beginDragHold();
      pushInteraction("companion-dragged");
      if (isTauri) {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        const startedAt = Date.now();
        const from = await win.outerPosition().catch(() => null);
        await win.startDragging().catch(() => {});
        // startDragging blocks until drop: persist the new position + 落地演出。
        endDragHold();
        try {
          const pos = await win.outerPosition();
          // 速度估算：原生拖曳期間沒有指標事件，只能用整段位移÷耗時算
          // 平均速度（下界估算，不是瞬時速度）。高度＝實際下降量。
          const elapsed = Math.max(1, Date.now() - startedAt);
          const dx = from ? pos.x - from.x : 0;
          const dy = from ? pos.y - from.y : 0;
          const speedPxPerSec = (Math.hypot(dx, dy) / elapsed) * 1000;
          const landing = pickLanding(
            {
              speedPxPerSec,
              heightPx: Math.max(0, dy),
              nearEdge: await nearScreenEdge(win, pos),
            },
            landingTableRef.current
          );
          if (landing.expression && landing.durationMs > 0) {
            apply({
              type: "transient",
              kind: "performing",
              animation: landing.expression,
              durationMs: landing.durationMs,
            });
          }
          await desktop.prefsPatch({
            companionPosition: [pos.x, pos.y],
          } as never);
        } catch {
          /* best effort */
        }
      } else {
        endDragHold(); // 瀏覽器模式沒有原生拖曳：立刻結束
      }
      dragState.current = null;
    }
  }

  function onPointerUp() {
    const stage = stageRef.current;
    if (toyDragRef.current) {
      stage?.pointerUp();
      toyDragRef.current = false;
      bgPressRef.current = false;
      return;
    }
    const d = dragState.current;
    dragState.current = null;
    const bgPress = bgPressRef.current;
    bgPressRef.current = false;
    if (d?.dragging) endDragHold();
    if (d && !d.dragging && bgPress) {
      // 互動框空白處的單擊：一般視窗互動——開/關選單，不戳角色、不送互動事件。
      setMenuOpen((v) => !v);
      setInputOpen(false);
      return;
    }
    if (d && !d.dragging) {
      const now = Date.now();
      clickTimes.current = [...clickTimes.current.filter((t) => now - t < 1_400), now];
      pushInteraction("companion-clicked");
      // 單擊／連戳都走 Director（真相狀態白名單＋變體池＋冷卻＋防重複＋個性）。
      // 連戳冷卻中或文字角色 → 退回一般單擊（有反應、開選單），不是靜默。
      // director-pipeline-018：同上——先留計畫，再讓 Director 建立短反應。
      const preempted = resumePlanFor(ambientActionRef.current, now);
      const plan = planClickReaction({
        rapid: clickTimes.current.length >= 3,
        nowMs: now,
        director: directorRef.current,
        rng: microRng.current,
      });
      if (plan.kind !== "fallback") {
        if (preempted) resumePlanRef.current = preempted;
        ambientActionRef.current = null;
      }
      if (plan.kind === "rapid") {
        // 連戳先清場再套演出，讓下一段連戳接得上。不帶 force：被擋下／失敗／
        // 未知／「在等你確認」都在 machine 的 CLEAR_PROTECTED_TRANSIENTS 裡，
        // 清不掉；接著的 performing（25）也搶不過它們的優先度，所以連戳不會把
        // 真相狀態換成玩鬧姿勢（對抗審查 director-pipeline-044）。
        apply({ type: "clear-transient" });
        apply({ type: "transient", kind: "performing", animation: plan.animation, durationMs: plan.durationMs });
        return;
      }
      apply({ type: "transient", kind: "clicked", animation: plan.animation, durationMs: plan.durationMs });
      if (plan.toggleMenu) {
        setMenuOpen((v) => !v);
        setInputOpen(false);
      }
    }
  }

  /** 本機安靜期（分鐘）：Director quiet ＋ hover/隨口氣泡停用，安全文字不受影響。 */
  function setLocalQuiet(minutes: number) {
    const until = proactiveQuietUntil(minutes, Date.now());
    quietUntilRef.current = until;
    void desktop.prefsPatch({ companionProactiveQuietUntil: until }).catch(() => {});
  }

  async function quick(action: string) {
    setMenuOpen(false);
    pushInteraction("action-selected", { action });
    try {
      switch (action) {
        case "talk":
          setInputOpen(true);
          break;
        case "open-cc":
          if (isTauri) {
            const { invoke } = await import("@tauri-apps/api/core");
            await invoke("companion_open_control_center", { tab: null });
          }
          break;
        case "tasks":
          if (isTauri) {
            const { invoke } = await import("@tauri-apps/api/core");
            await invoke("companion_open_control_center", { tab: "activity" });
          }
          break;
        case "pause-1h":
          await api.pauseSet(60, "companion quick action");
          showBubble(line("pause-ack"), 3500);
          break;
        case "quiet-1h":
          // 主動式對話安靜一小時（≠ 暫停主動行動，也 ≠ emergency stop）。
          // 同時關掉角色自己的隨口氣泡與 ambient 表演——只叫 runtime 閉嘴
          // 的話，角色照樣會自己冒話。安全文字不受影響。
          await api.proactiveDialogueQuiet(60);
          setLocalQuiet(60);
          showBubble("好，一小時內我不主動說話。", 3500);
          break;
        case "quiet-today":
          await api.proactiveDialogueQuiet(12 * 60);
          setLocalQuiet(12 * 60);
          showBubble("好，今天我會安靜一點。", 3500);
          break;
        case "estop":
          await api.emergencyStop("companion quick action");
          break;
      }
    } catch (e) {
      showBubble(`失敗：${e}`, 4000);
    }
  }

  // Click-through coordination: only a focused text input and the drop
  // confirmation need the WHOLE window (native text selection / OS drop
  // target). Passive or self-contained surfaces — speech bubble, safety
  // labels, the quick menu — report their own bounded hit region instead, so
  // the transparent space around them still clicks through to the desktop
  // (companion-gameplay-032). This mirrors the contract documented on
  // `companion_set_interactive` in src-tauri/src/lib.rs.
  React.useEffect(() => {
    if (!isTauri) return;
    void (async () => {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("companion_set_interactive", {
        interactive: inputOpen || dropPreview !== null,
      }).catch(() => {});
    })();
  }, [inputOpen, dropPreview]);

  // UI 面剛出現／消失（選單、氣泡、可信文字、安全與感測標籤）：立刻補一份
  // regions，不要等下一次 stage 回報（≤60ms）。stage 不在場（sprite／text）時
  // 這也是唯一的回報路徑。
  React.useEffect(() => {
    if (!isTauri || !ready) return;
    let cancelled = false;
    // 版面要先算完才量得到 UI 面的矩形。
    const id = requestAnimationFrame(() => {
      if (!cancelled) void pushHitRegions(null);
    });
    return () => {
      cancelled = true;
      cancelAnimationFrame(id);
    };
  }, [ready, entrypointKind, menuOpen, bubble, trustedText, sensorLabel, baseState, pushHitRegions]);

  React.useEffect(() => {
    if (!isTauri || !ready || !canvasRef.current) return;
    // 遊玩場 adapter 的 regions 由 stage 動態更新（角色會走動）；
    // sprite／text 沒有遊玩場，整個 canvas 就是唯一的互動面。
    if (stageRef.current) return;
    const rect = canvasRef.current.getBoundingClientRect();
    if (!(rect.width > 0) || !(rect.height > 0)) return;
    // canvas 相對座標（pushHitRegions 會平移成視窗相對）。
    void pushHitRegions([{ id: "surface", x: 0, y: 0, w: rect.width, h: rect.height }]);
  }, [ready, entrypointKind, pushHitRegions]);

  /** 玩具快捷：由玩家丟玩具進遊玩場（純本機，不經 runtime；gameplay 擴充）。 */
  function quickToy(kind: ToyKind | "clear") {
    setMenuOpen(false);
    const gameplay = adapterRef.current?.gameplay;
    if (!gameplay) return;
    // 凍結（緊急停止／離線／暫停）：停住的系統不生成也不收玩具（stage 也會拒絕；這裡不多做記憶）。
    if (["emergency", "offline", "paused"].includes(machineRef.current.base)) return;
    if (kind === "clear") {
      gameplay.clearToys();
      return;
    }
    if (gameplay.spawnToy(kind) === null) return;
    // 角色互動記憶：記「玩過什麼」，不推論人格（熟悉度只看天數）。
    const mem = notePlay(memoryRef.current, kind, Date.now());
    memoryRef.current = mem;
    void desktop.prefsPatch({ companionInteractionMemory: mem }).catch(() => {});
  }

  /** 文字輸入的實際送出（路由由使用者選；protocol 模式輸入事件只走 characterEvent）。 */
  const submitText = React.useCallback(async (text: string, target: string) => {
    if (target !== "local") {
      await api.agentSessionSend(target, "task", { task: text, source: "desktop-companion" });
      pushInteraction("text-submitted", { text });
      return;
    }
    if (feedRef.current === "protocol" && gatewayRef.current) {
      // CPP：文字本身在 text-submitted 事件裡（privacy personal），Runtime 轉成 companion.text-input。
      const ev = inputEventFor("text-submitted", { text });
      forwardBatchRef.current = [];
      if (!ev || !gatewayRef.current.ingestInput(PRIMARY_INSTANCE_ID, ev)) {
        throw new Error("input rejected");
      }
      behaviorState.current = noteUserInteraction(behaviorState.current, Date.now());
      const result = await awaitForwardBatch();
      if (!result) throw new Error("runtime unreachable");
      if (result.decision === "dropped") throw new Error(result.reason ?? "runtime dropped the input");
      return;
    }
    await api.pushObservation("session.input", { text, source: "desktop-companion" }, 1.0);
    await api
      .pushObservation("companion.text-input", { kind: "text-submitted", modality: "text" }, 1.0)
      .catch(() => {});
  }, [awaitForwardBatch]);

  const estop = baseState === "emergency";
  const frozen = baseState === "emergency" || baseState === "offline" || baseState === "paused";
  const canvasClass = cssClassForEntrypoint(entrypointKind);

  return (
    // `data-hit-region` 的元素會各自成為一個 bounded hit region（見 uiHitRegions）：
    // 訊息面不該被點穿，但它們周圍的透明區仍然屬於桌面。
    <div className="companion-root" ref={rootRef}>
      {bubble && (
        <div
          className={estop ? "companion-bubble danger" : "companion-bubble"}
          role="status"
          data-hit-region="bubble"
        >
          {bubble}
        </div>
      )}
      {estop && (
        <div className="companion-estop-label" data-hit-region="estop-label">
          緊急停止中
        </div>
      )}
      {!estop && sensorLabel && (
        <div className="companion-sensor-label" role="status" data-hit-region="sensor-label">
          {sensorLabel}
        </div>
      )}
      {trustedText && (
        // 可信 host 元素：system.text 與角色載入失敗文案。adapter 碰不到這裡。
        <div
          className="companion-system-text"
          role="status"
          data-marker={trustedText.marker}
          data-hit-region="system-text"
        >
          {trustedText.marker === "verified" ? "✓ " : ""}
          {trustedText.text}
        </div>
      )}
      <canvas
        ref={canvasRef}
        className={canvasClass}
        hidden={entrypointKind === "text"}
        aria-label={`桌面角色（${charName}）`}
        role="img"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerEnter={onPointerEnterCanvas}
        onPointerLeave={onPointerLeaveCanvas}
      />
      <div
        ref={textHostRef}
        className="companion-text-host"
        hidden={entrypointKind !== "text"}
        data-hit-region="text-host"
        aria-label={`桌面角色（${charName}）`}
        onClick={() => {
          setMenuOpen((v) => !v);
          setInputOpen(false);
        }}
      />
      {dropPreview && (
        <div className="companion-menu" role="dialog" aria-label="拖放預覽">
          <div style={{ fontSize: 12, padding: "4px 8px" }}>
            <strong>收到 {dropPreview.items.length} 個項目</strong>
            <ul style={{ margin: "4px 0", paddingLeft: 16, maxHeight: 80, overflow: "auto" }}>
              {dropPreview.items.slice(0, 5).map((item, i) => (
                <li key={`${i}-${item.name}`} style={{ wordBreak: "break-all" }}>
                  {dropItemLine(item)}
                </li>
              ))}
              {dropPreview.items.length > 5 && <li>…等 {dropPreview.items.length} 個</li>}
            </ul>
            <div className="muted" style={{ fontSize: 11, opacity: 0.8 }}>
              {dropDestinationLines(dropSessions).map((l) => (
                <div key={l}>{l}</div>
              ))}
              <div>確認前不會做任何事。</div>
            </div>
          </div>
          <button
            onClick={() => {
              // 確認式流程：等待 push 實際結果，成功才說「記下了」，
              // 失敗誠實回報（誠實階梯：送出≠已記錄）。
              behaviorState.current = noteUserInteraction(behaviorState.current, Date.now());
              const items = dropPreview;
              setDropPreview(null);
              void recordDroppedItems(items.paths, dropPush, showBubble, line, items.items);
            }}
          >
            記錄這些項目
          </button>
          <button onClick={() => setDropPreview(null)}>取消（不做任何事）</button>
        </div>
      )}
      {menuOpen && (
        <div className="companion-menu" role="menu" aria-label="快捷操作" data-hit-region="menu">
          {toyCatalog.length > 0 && !frozen && (
            <div className="companion-toy-row" role="group" aria-label="玩具">
              {toyCatalog.map((toy) => (
                <button key={toy.kind} role="menuitem" title={toy.label} onClick={() => quickToy(toy.kind)}>
                  {toy.emoji}
                </button>
              ))}
              <button role="menuitem" title="收走玩具" onClick={() => quickToy("clear")}>🧹</button>
            </div>
          )}
          <button role="menuitem" onClick={() => quick("talk")}>
            對{charName}說話…
          </button>
          <button role="menuitem" onClick={() => quick("tasks")}>
            查看目前工作
          </button>
          <button role="menuitem" onClick={() => quick("pause-1h")}>
            暫停一小時
          </button>
          <button role="menuitem" onClick={() => quick("quiet-1h")}>
            一小時內不要主動說話
          </button>
          <button role="menuitem" onClick={() => quick("quiet-today")}>
            今天安靜一點
          </button>
          <button role="menuitem" onClick={() => quick("open-cc")}>
            開啟控制中心
          </button>
          <button role="menuitem" className="danger" onClick={() => quick("estop")}>
            緊急停止
          </button>
        </div>
      )}
      {inputOpen && (
        <CompanionInput
          name={charName}
          onClose={() => setInputOpen(false)}
          // 氣泡一律走 showBubble：同一個 bubbleTimer 主人，才不會有沒被追蹤的
          // 計時器提早抹掉更新的安全氣泡（對抗審查 companion-gameplay-032）。
          onBubble={(text, ms, opts) => showBubble(text, ms ?? 0, opts)}
          line={line}
          submit={submitText}
          conversationCtx={() => ({
            openAgentSessions: 0, // CompanionInput 以實際 sessions 數覆蓋
            msSinceInteraction: Date.now() - behaviorState.current.lastInteractionAt,
            expressiveness: behaviorRef.current.allowCasualBubbles ? "natural" : "quiet",
          })}
          onIntent={(intent) => {
            // L1 語意意圖也要經過 Director：truthState 永遠不可點播，
            // 冷卻中就不重播（playable() 白名單由角色 tables 提供）。
            // 回 null 不是靜默：原因記在 Director.lastDecision()（no-mapping 代表表缺項）。
            const decision = directorRef.current.reactDetailed(intent, Date.now(), 2500, microRng.current);
            if (decision.action) {
              apply({
                type: "transient",
                kind: "performing",
                animation: decision.action.expression,
                durationMs: decision.action.durationMs,
              });
            } else if (decision.reason === "no-mapping") {
              console.warn(`director: L1 intent "${intent}" has no reaction in this character's tables`);
            }
          }}
        />
      )}
    </div>
  );
}

/** Text input with deterministic routing preview: default = local
 *  observation; open agent sessions can be selected as the destination
 *  (mailbox task). The preview always states where the data goes. */
export function CompanionInput({
  name,
  onClose,
  onBubble,
  line,
  submit,
  conversationCtx,
  onIntent,
}: {
  /** 角色顯示名（aria-label 用）。 */
  name: string;
  onClose: () => void;
  /** 顯示氣泡：`ms` 交給氣泡的主人（showBubble）排計時器；這裡不自排計時器，
   *  否則會抹掉期間出現的安全氣泡。送出失敗屬於「不得靜默」的失敗回報，
   *  用 `safety` 讓它不被「關掉氣泡」的偏好吃掉。 */
  onBubble: (t: string | null, ms?: number, opts?: { safety?: boolean }) => void;
  line: (key: string) => string | null;
  /** 實際送出（由 CompanionApp 決定走 CPP 事件或舊 receptor）。 */
  submit: (text: string, target: string) => Promise<void>;
  /** L1 Conversation Provider 的情境（無 Provider 時本機模板降級）。 */
  conversationCtx?: () => ConversationContext;
  onIntent?: (intent: string) => void;
}) {
  const [text, setText] = React.useState("");
  const [sessions, setSessions] = React.useState<
    { sessionId: string; label?: string; agentId: string; dataScope: string[] }[]
  >([]);
  const [target, setTarget] = React.useState("local");
  const inputRef = React.useRef<HTMLInputElement>(null);
  React.useEffect(() => {
    inputRef.current?.focus();
    api
      .agentSessionsList()
      .then((list) => setSessions(list.filter((s) => !s.closedAt)))
      .catch(() => {});
  }, []);

  async function send() {
    const t = text.trim();
    if (!t) return;
    onClose();
    try {
      await submit(t, target);
      if (target === "local") {
        // L1 Conversation Provider：決定是否回話、語氣與 behaviorIntent。
        // 觀察已真實記錄（上面 submit 成功）；模板回覆不冒充理解。
        const ctx = conversationCtx?.() ?? {
          openAgentSessions: sessions.length,
          msSinceInteraction: 0,
          expressiveness: "natural",
        };
        const result = activeConversationProvider().considerReply(t, {
          ...ctx,
          openAgentSessions: sessions.length,
        });
        onBubble(result.reply ?? line("text-received"), 3500);
        if (result.behaviorIntent && onIntent) onIntent(result.behaviorIntent);
      } else {
        onBubble(line("delegated"), 3500);
      }
    } catch (e) {
      // 使用者按了送出卻沒送成：這是失敗結果，不是隨口閒聊，不可被靜默。
      onBubble(`送出失敗：${e}`, 4000, { safety: true });
    }
  }

  const selected = sessions.find((s) => s.sessionId === target);
  return (
    <div className="companion-input" role="dialog" aria-label={`對${name}說話`}>
      {sessions.length > 0 && (
        <select
          aria-label="交給誰"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          style={{ width: "100%", fontSize: 11 }}
        >
          <option value="local">本機 Runtime（觀察紀錄）</option>
          {sessions.map((s) => (
            <option key={s.sessionId} value={s.sessionId}>
              AI 工作階段：{s.label ?? s.agentId}
            </option>
          ))}
        </select>
      )}
      <div className="companion-route-note">
        {target === "local"
          ? "交給：本機 Runtime（觀察紀錄）・資料不離開本機"
          : `交給：${selected?.label ?? selected?.agentId}・可讀範圍：${
              selected?.dataScope.join("、") || "未設定"
            }`}
      </div>
      <input
        ref={inputRef}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void send();
          if (e.key === "Escape") onClose();
        }}
        placeholder="輸入文字…（Enter 送出，Esc 取消）"
        aria-label="訊息內容"
      />
      <button onClick={() => void send()}>送出</button>
      <button onClick={onClose}>取消</button>
    </div>
  );
}

/** 拖放確認後把項目記錄成觀察。與遙測不同，這裡對使用者承諾「已記錄」，
 *  所以必須等待 push 的實際結果：resolve 才顯示成功語，reject 顯示失敗
 *  （receptor 停用、隱藏中、runtime 離線時項目並沒有被記錄，不得謊稱）。 */
export async function recordDroppedItems(
  paths: string[],
  push: (
    receptorId: string,
    facts: Record<string, unknown>,
    confidence?: number
  ) => Promise<unknown>,
  showBubble: (text: string | null, ms: number) => void,
  line: (key: string) => string | null,
  items?: readonly DropPreviewItem[]
): Promise<boolean> {
  try {
    await push(
      RECEPTOR_FOR_KIND["companion-dropped"],
      {
        kind: "companion-dropped",
        modality: "file-drop",
        attachments: paths,
        mayLeaveDevice: false,
        // 已知的大小／類型（不知道的項目不帶鍵，不補 0）。
        ...(items && items.length > 0
          ? { files: items.map((i) => ({ ...(i.bytes !== null ? { bytes: i.bytes } : {}), ...(i.mediaType !== null ? { mediaType: i.mediaType } : {}) })) }
          : {}),
      },
      1.0
    );
    showBubble(line("drop-received"), 3000);
    return true;
  } catch (e) {
    showBubble(`記錄失敗：${e}（這些項目沒有被記錄）`, 4000);
    return false;
  }
}
