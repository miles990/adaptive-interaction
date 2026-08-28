// 小樞 companion window: honest runtime-state presentation + quick actions.
//
// This window is presentation + input entry ONLY. It holds no authority:
// every action goes through the same backend commands/policy as everything
// else, and safety states (emergency/blocked/unknown) use fixed standard
// wording that packs cannot override.

import React from "react";
import { api, RuntimeEvent } from "../api";
import { bootstrapSupervisor, desktop, isTauri } from "../desktop";
import { onRuntimeEvent } from "../api";
import {
  DRAG_HOLD_MS,
  DRAG_RENEW_MS,
  initial,
  MachineState,
  mapRuntimeEvent,
  pose,
  reduce,
  wasPreempted,
} from "./machine";
import { PackManifest, RendererBackend, SpriteRenderer, validateManifest } from "./renderer";
import { validateRigManifest } from "./rig/renderer";
import { machineStageFlags, StageRenderer } from "./rig/stage";
import { ToyKind } from "./playfield";
import { directorTickGate, InteractionDirector } from "./director";
import { activeConversationProvider, ConversationContext } from "./conversation";
import { planPresentationCommand } from "./presentationCommands";
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
import { pickLanding } from "./gameFeel";
import {
  emptyMemory,
  InteractionMemory,
  notePlay,
  noteSession,
  sanitizeMemory,
} from "./interactionMemory";

// v0.4: itemized Presentation Provider receptors — each interaction kind
// routes to its own receptor so consent/enable is per-capability. Unknown
// kinds fall back to the legacy combined receptor (kept for compat).
const RECEPTOR_FOR_KIND: Record<string, string> = {
  "companion-clicked": "companion.click",
  "companion-dragged": "companion.click",
  "text-submitted": "companion.text-input",
  "action-selected": "companion.quick-action",
  "pointer-approached": "companion.pointer",
  "companion-dropped": "companion.drag-drop",
  "drop-entered": "companion.drag-drop",
  "drop-left": "companion.drag-drop",
  "drop-cancelled": "companion.drag-drop",
  "bubble-shown": "companion.bubble-events",
  "bubble-dismissed": "companion.bubble-events",
  "animation-completed": "companion.animation-events",
  "animation-interrupted": "companion.animation-events",
};

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
 * 緊急停止／取消必須真的讓她閉嘴：清氣泡不會停掉已經送進語音服務的句子，
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
    utterance.lang = "zh-TW";
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

export default function CompanionApp() {
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  const rendererRef = React.useRef<RendererBackend | null>(null);
  /** 遊玩場（只有 rig packs 有；sprite 相容層無遊玩）。 */
  const stageRef = React.useRef<StageRenderer | null>(null);
  const approachEnabledRef = React.useRef(true);
  const charNameRef = React.useRef("小樞");
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
  const machineRef = React.useRef<MachineState>(initial);
  const lastBubbleAt = React.useRef(0);
  const [bubble, setBubble] = React.useState<string | null>(null);
  const [menuOpen, setMenuOpen] = React.useState(false);
  const [packName, setPackName] = React.useState("shu-maid");
  const [ready, setReady] = React.useState(false);
  const [inputOpen, setInputOpen] = React.useState(false);
  const [sensorLabel, setSensorLabel] = React.useState<string | null>(null);
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

  /** Resolve a bubble line: safety keys fixed, others persona-styled. */
  const line = React.useCallback((key: string): string | null => {
    return resolveLine(key, personaRef.current);
  }, []);

  /** Show a bubble for `ms` (0 = sticky). Clears any prior timer so an older
   *  bubble's timeout can never erase a newer safety bubble early.
   *  氣泡開關關掉後只剩 `safety` 文字（緊急停止/被擋下/未知）。 */
  const showBubble = React.useCallback(
    (text: string | null, ms: number, opts?: { safety?: boolean }) => {
      if (text !== null && !bubbleAllowed({ enabled: bubblesEnabledRef.current, safety: opts?.safety === true })) {
        return;
      }
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

  // Settings changes (pack/persona/expressiveness) reload this window.
  React.useEffect(() => {
    if (!isTauri) return;
    let unReload: (() => void) | null = null;
    let unOpacity: (() => void) | null = null;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      unReload = await listen("companion-reload", () => window.location.reload());
      unOpacity = await listen<number>("companion-opacity", (event) => {
        document.documentElement.style.opacity = String(event.payload);
      });
    })();
    return () => {
      if (unReload) unReload();
      if (unOpacity) unOpacity();
    };
  }, []);

  // ---- boot: transport, pack, renderer ----
  React.useEffect(() => {
    let disposed = false;
    (async () => {
      await bootstrapSupervisor();
      let pack = "shu-maid";
      let personaId = "persona-shu";
      let renderScale = 1.1;
      try {
        const prefs = await desktop.prefsGet();
        pack = prefs.companionPack ?? pack;
        personaId = prefs.companionPersona ?? personaId;
        behaviorRef.current = behaviorFor(prefs.companionExpressiveness ?? "natural");
        storyProgress.current = prefs.storyProgress ?? {};
        document.documentElement.style.opacity = String(prefs.companionOpacity ?? 1);
        renderScale = 1.1 * ((prefs.companionSize?.[0] ?? 200) / 200);
        // 個性：表現度＋persona → profile → tuning（純函式，只影響呈現）。
        personalityRef.current = personalityFor(prefs.companionExpressiveness ?? "natural", personaId);
        tuningRef.current = tuningFor(personalityRef.current);
        directorRef.current.setTuning(tuningRef.current);
        dndRef.current = prefs.companionDoNotDisturb === true;
        quietUntilRef.current = prefs.companionProactiveQuietUntil ?? 0;
        bubblesEnabledRef.current = prefs.companionBubbles !== false;
        soundEnabledRef.current = prefs.companionSound === true;
        dragEnabledRef.current = prefs.companionDragEnabled !== false;
        // 角色互動記憶：同一天只算一次（熟悉度隨天數緩升，不因單一事件跳動）。
        const mem = noteSession(sanitizeMemory(prefs.companionInteractionMemory), Date.now());
        memoryRef.current = mem;
        void desktop.prefsPatch({ companionInteractionMemory: mem }).catch(() => {});
      } catch {
        /* browser mode */
      }
      // Persona + story packs are data-only; invalid packs fall back to the
      // built-in default lines instead of breaking the companion.
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
      try {
        const story = (await fetch(`/packs/story-shu-intro.json`).then((r) => r.json())) as unknown;
        if (validateStoryPack(story).length === 0) {
          storyRef.current = story as StoryPack;
        }
      } catch {
        /* no story */
      }
      if (disposed) return;
      setPackName(pack);
      const manifest = (await fetch(`/packs/${pack}/manifest.json`).then((r) =>
        r.json()
      )) as PackManifest & { kind?: string; palette?: string };
      if (disposed || !canvasRef.current) return;
      let renderer: RendererBackend;
      if (manifest.kind === "character-rig") {
        // v3 執行期參數化 rig（女僕正式版）＋遊玩場。
        const issues = validateRigManifest(manifest);
        if (issues.length > 0) {
          console.error("invalid character rig pack", issues);
          return;
        }
        const canvasEl = canvasRef.current;
        const stage = new StageRenderer(canvasEl, String(manifest.palette), renderScale);
        try {
          const prefs = await desktop.prefsGet();
          stage.setToggles({
            play: prefs.companionPlay !== false,
            cursorPlay: prefs.companionCursorPlay !== false,
            deskMove: prefs.companionDeskMove !== false,
          });
          stage.setScene(prefs.companionScene ?? "none");
          stage.setCharName(prefs.companionName || "小樞");
          charNameRef.current = prefs.companionName || "小樞";
          stage.setFamiliars(prefs.companionFamiliars ?? []);
          approachEnabledRef.current = prefs.companionApproach !== false;
        } catch {
          /* browser mode：預設全部開啟 */
        }
        stage.setTuning(tuningRef.current);
        stage.onExpressionEvent((id, durationMs) => {
          apply({ type: "transient", kind: "performing", animation: id, durationMs });
        });
        // 互動框：由 stage 每幀依節流政策回報（角色會走動、玩具會滾，
        // 只靠 500ms pump 的話 Rust 會用過期的框判定點擊穿透）。
        if (isTauri) {
          void import("@tauri-apps/api/core").then(({ invoke }) => {
            stage.onHitRect((b) => {
              const rect = canvasEl.getBoundingClientRect();
              void invoke("companion_hit_rect", {
                x: rect.left + b.x,
                y: rect.top + b.y,
                w: b.w,
                h: b.h,
              }).catch(() => {});
            });
          });
        }
        stageRef.current = stage;
        renderer = stage;
      } else {
        // v1/v2 sprite-sheet pack 相容層。
        const issues = validateManifest(manifest);
        if (issues.length > 0) {
          console.error("invalid character pack", issues);
          return;
        }
        renderer = new SpriteRenderer(
          canvasRef.current,
          manifest,
          `/packs/${pack}/sheet.png`,
          renderScale
        );
      }
      const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
      renderer.setReducedMotion(motionQuery.matches);
      // 執行中改系統設定也要立刻生效（不必重開視窗）。
      const onMotionChange = (e: MediaQueryListEvent) =>
        rendererRef.current?.setReducedMotion(e.matches);
      motionQuery.addEventListener?.("change", onMotionChange);
      motionCleanup.current = () => motionQuery.removeEventListener?.("change", onMotionChange);
      rendererRef.current = renderer;
      setReady(true);
    })();
    return () => {
      disposed = true;
      motionCleanup.current?.();
      motionCleanup.current = null;
      rendererRef.current?.destroy();
    };
  }, []);

  // ---- machine driving ----
  // Behavior Runtime（生命底層）狀態：平滑量＋Interaction Director（統一
  // 行為導演：注意力/冷卻/防重複/中斷恢復）、seeded RNG。
  const behaviorState = React.useRef<CompanionBehaviorState>(initialBehavior(Date.now()));
  const directorRef = React.useRef(new InteractionDirector());
  /** 上一個 tick 是否正在表演（用來判斷「自然播完」）。 */
  const performingRef = React.useRef(false);
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
    // 角色還會追球最多半秒，而 doze/lie-flat 這類姿勢也會被步行覆蓋。
    const t = m.transient && m.transient.untilMs > now ? m.transient : null;
    stageRef.current?.setMachineFlags(machineStageFlags(m.base, t, p.animation, p.ambient));
  }, []);

  // ---- 拖曳期間的「持續」transient（放下才結束） ----
  const dragHoldRef = React.useRef<ReturnType<typeof setInterval> | null>(null);

  /** 抱起來：dragged 用續期表示「還在手上」，不是 1.5 秒後自己過期。 */
  const beginDragHold = React.useCallback(() => {
    apply({ type: "transient", kind: "dragged", durationMs: DRAG_HOLD_MS });
    if (dragHoldRef.current) clearInterval(dragHoldRef.current);
    dragHoldRef.current = setInterval(() => {
      apply({ type: "transient", kind: "dragged", durationMs: DRAG_HOLD_MS });
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
  const pushInteraction = React.useCallback((kind: string, extra?: Record<string, unknown>) => {
    behaviorState.current = noteUserInteraction(behaviorState.current, Date.now());
    const receptor = RECEPTOR_FOR_KIND[kind] ?? "desktop.companion.interaction";
    void api.pushObservation(receptor, { kind, ...extra }, 1.0).catch(() => {
      /* receptor disabled, companion hidden, or runtime offline: dropped */
    });
  }, []);

  // ---- presentation commands: runtime → this surface → honest ack ----
  const handlePresentationCommand = React.useCallback(
    async (payload: Record<string, unknown>) => {
      const command = String(payload["command"] ?? "");
      const actionId = typeof payload["actionId"] === "string" ? payload["actionId"] : null;
      if (command === "cancel" || command === "clear-all") {
        // Cancelled/estopped: drop any non-safety visual (safety poses are
        // driven by base state, not by presentation commands) — including a
        // still-running `performing` transient, which used to survive here.
        // 進行中的語音也要停：清氣泡不會讓已經在講的句子閉嘴。
        const cancelPlan = planPresentationCommand(command, {}, isTauri);
        stopSpeech();
        showBubbleRaw(null, 0);
        if (cancelPlan.transient === null) apply({ type: "clear-transient" });
        return;
      }
      if (!actionId) return;
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
        try {
          await desktop.prefsPatch({ companionVisible: plan.presence });
        } catch (error) {
          plan.outcome = "failed";
          plan.detail = String(error);
        }
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
      const rollCall = stageRef.current
        ? stageRef.current.rollCallNow(machineLabel)
        : [{ name: charNameRef.current, activity: machineLabel ?? "在休息" }];
      void api
        .presentationHello(!document.hidden, packName, {
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
    beat();
    const t = setInterval(beat, 10_000);
    document.addEventListener("visibilitychange", beat);
    return () => {
      stopped = true;
      clearInterval(t);
      document.removeEventListener("visibilitychange", beat);
    };
  }, [ready, packName]);

  // Runtime events → transients; status poll → base state.
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
      const mapped = mapRuntimeEvent(e);
      if (mapped) {
        // 注意力：事件推高喚起度（平滑，不會 0→1）。
        const importance = e.eventType === "emergency.stop" ? 1 : e.eventType.startsWith("action.") ? 0.5 : 0.3;
        behaviorState.current = noteEvent(behaviorState.current, e.eventType, importance);
        apply(mapped);
        maybeBubble(e);
      }
    });
    const poll = async () => {
      try {
        const s = await api.status();
        if (stopped) return;
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
        setSensorLabel(
          sensors.length > 0
            ? sensors.some((x) => x.kind === "microphone")
              ? "🎙 正在使用麥克風"
              : `使用中：${sensors.map((x) => x.kind).join("、")}`
            : null
        );
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
    void poll();
    const t = setInterval(poll, 5000);
    // Pose re-evaluation for transient expiry + ambient blink.
    // Behavior Runtime tick（500ms）：平滑步進 → 姿勢刷新 → 微動作排程。
    // 觸發間隔由 hazard 抽樣決定（幾何分布）——絕不是固定週期同一動畫。
    const pump = setInterval(() => {
      const now = Date.now();
      const m = machineRef.current;
      const t = m.transient && m.transient.untilMs > now ? m.transient : null;
      // 表演自然播完（不是被搶佔）→ 告訴 Director 舞台空了。
      if (performingRef.current && t?.kind !== "performing") {
        performingRef.current = false;
        directorRef.current.noteFinished();
      } else if (t?.kind === "performing") {
        performingRef.current = true;
      }
      const busy =
        t != null && ["acting", "waiting-for-receipt", "routing", "thinking"].includes(t.kind);
      const waitingForHuman = t?.kind === "requesting-consent";
      behaviorState.current = stepBehavior(behaviorState.current, {
        busy,
        waitingForHuman,
        msSinceInteraction: now - behaviorState.current.lastInteractionAt,
      });
      const reducedMotion =
        typeof window.matchMedia === "function" &&
        window.matchMedia("(prefers-reduced-motion: reduce)").matches;
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
        });
        if (decision.show && decision.text) {
          lastBubbleAt.current = now;
          hoverSinceRef.current = now; // 同一次停留不再重複說
          showBubble(decision.text, 3200);
        }
      }
      // 遊玩中（追逐/叼回）不再疊 Director 的 ambient 動作。
      if (stage?.worldBusy()) return;
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
        // 安靜時只允許眨眼，而且要「就地眨」：套成一般表演的話，她會從安靜
        // 陪伴的坐姿彈回中性站姿。rig 收得下這個提示就不換表情。
        const rig = rendererRef.current as { blinkNow?: () => boolean } | null;
        const blinkedInPlace =
          gate.quiet && action.expression === "blink" && rig?.blinkNow?.() === true;
        if (!blinkedInPlace) {
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
      clearInterval(t);
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

  // ---- semantic interaction events (NEVER raw coordinates to the runtime) ----
  // 遊玩場內的指標座標只活在本視窗 canvas，供光點/逗貓棒/玩具拖曳；
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
      pushInteraction("pointer-approached");
      // 游標靠近時看過來（不追蹤、不記錄座標）。
      apply({ type: "transient", kind: "listening", durationMs: 1200 });
    }
  }

  function onPointerLeaveCanvas() {
    stageRef.current?.pointerLeave();
    toyDragRef.current = false;
    hoverSinceRef.current = 0;
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
  const [dropPreview, setDropPreview] = React.useState<string[] | null>(null);
  React.useEffect(() => {
    if (!isTauri || !ready) return;
    let un: (() => void) | null = null;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      un = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type === "drop") {
          const paths = (event.payload as { paths?: string[] }).paths ?? [];
          if (paths.length > 0) setDropPreview(paths);
        }
      });
    })();
    return () => {
      if (un) un();
    };
  }, [ready]);

  // ---- pointer: toy drag / click vs window drag ----
  const dragState = React.useRef<{ x: number; y: number; dragging: boolean } | null>(null);
  const toyDragRef = React.useRef(false);
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
      if (hit === "none") return; // 擴大互動框內的空白：不反應
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
      // 否則她會在半空中回 idle、遊玩場也重新啟動。
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
          const landing = pickLanding({
            speedPxPerSec,
            heightPx: Math.max(0, dy),
            nearEdge: await nearScreenEdge(win, pos),
          });
          if (landing.durationMs > 0) {
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
      return;
    }
    const d = dragState.current;
    dragState.current = null;
    if (d?.dragging) endDragHold();
    if (d && !d.dragging) {
      const now = Date.now();
      clickTimes.current = [...clickTimes.current.filter((t) => now - t < 1_400), now];
      pushInteraction("companion-clicked");
      if (clickTimes.current.length >= 3) {
        // 被連戳：統一走 Director（真相狀態白名單＋冷卻＋個性）。
        const reaction = directorRef.current.react("poked-rapid", now, 2200, microRng.current);
        if (reaction) {
          apply({ type: "clear-transient" });
          apply({
            type: "transient",
            kind: "performing",
            animation: reaction.expression,
            durationMs: reaction.durationMs,
          });
        }
        return;
      }
      apply({ type: "transient", kind: "clicked" });
      setMenuOpen((v) => !v);
      setInputOpen(false);
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
          // 的話，她照樣會自己冒話。安全文字不受影響。
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

  // Click-through coordination: while a menu/input/bubble is open the whole
  // window must accept the cursor; otherwise only the character hit-rect does.
  React.useEffect(() => {
    if (!isTauri) return;
    void (async () => {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("companion_set_interactive", {
        interactive: menuOpen || inputOpen || bubble !== null || dropPreview !== null,
      }).catch(() => {});
    })();
  }, [menuOpen, inputOpen, bubble, dropPreview]);

  React.useEffect(() => {
    if (!isTauri || !ready || !canvasRef.current) return;
    // 遊玩場 pack 的 hit-rect 由 pump 動態更新（角色會走動）；
    // sprite 相容層維持整個 canvas。
    if (stageRef.current) return;
    const rect = canvasRef.current.getBoundingClientRect();
    void (async () => {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("companion_hit_rect", {
        x: rect.left,
        y: rect.top,
        w: rect.width,
        h: rect.height,
      }).catch(() => {});
    })();
  }, [ready]);

  /** 玩具快捷：由玩家丟玩具進遊玩場（純本機，不經 runtime）。 */
  function quickToy(kind: ToyKind | "clear") {
    setMenuOpen(false);
    const stage = stageRef.current;
    if (!stage) return;
    if (kind === "clear") {
      stage.clearAllToys();
      return;
    }
    stage.spawnToy(kind);
    // 角色互動記憶：記「玩過什麼」，不推論人格（熟悉度只看天數）。
    const mem = notePlay(memoryRef.current, kind, Date.now());
    memoryRef.current = mem;
    void desktop.prefsPatch({ companionInteractionMemory: mem }).catch(() => {});
  }

  const estop = baseState === "emergency";

  return (
    <div className="companion-root">
      {bubble && (
        <div className={estop ? "companion-bubble danger" : "companion-bubble"} role="status">
          {bubble}
        </div>
      )}
      {estop && <div className="companion-estop-label">緊急停止中</div>}
      {!estop && sensorLabel && (
        <div className="companion-sensor-label" role="status">
          {sensorLabel}
        </div>
      )}
      <canvas
        ref={canvasRef}
        className={packName.startsWith("shu-maid") ? "companion-stage" : "companion-canvas"}
        aria-label={`桌面角色（${packName}）`}
        role="img"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerEnter={onPointerEnterCanvas}
        onPointerLeave={onPointerLeaveCanvas}
      />
      {dropPreview && (
        <div className="companion-menu" role="dialog" aria-label="拖放預覽">
          <div style={{ fontSize: 12, padding: "4px 8px" }}>
            <strong>收到 {dropPreview.length} 個項目</strong>
            <ul style={{ margin: "4px 0", paddingLeft: 16, maxHeight: 80, overflow: "auto" }}>
              {dropPreview.slice(0, 5).map((p) => (
                <li key={p} style={{ wordBreak: "break-all" }}>
                  {p.split("/").pop()}
                </li>
              ))}
              {dropPreview.length > 5 && <li>…等 {dropPreview.length} 個</li>}
            </ul>
            <div className="muted" style={{ fontSize: 11, opacity: 0.8 }}>
              只記錄檔案位置，不讀取內容、不上傳、不離開本機。確認前不會做任何事。
            </div>
          </div>
          <button
            onClick={() => {
              // 確認式流程：等待 push 實際結果，成功才說「記下了」，
              // 失敗誠實回報（誠實階梯：送出≠已記錄）。
              behaviorState.current = noteUserInteraction(behaviorState.current, Date.now());
              const items = dropPreview;
              setDropPreview(null);
              void recordDroppedItems(items, api.pushObservation, showBubble, line);
            }}
          >
            記錄這些項目
          </button>
          <button onClick={() => setDropPreview(null)}>取消（不做任何事）</button>
        </div>
      )}
      {menuOpen && (
        <div className="companion-menu" role="menu" aria-label="快捷操作">
          {stageRef.current && (
            <div className="companion-toy-row" role="group" aria-label="玩具">
              <button role="menuitem" title="丟毛球" onClick={() => quickToy("yarn")}>🧶</button>
              <button role="menuitem" title="丟紙團" onClick={() => quickToy("paper")}>🗞️</button>
              <button role="menuitem" title="紙飛機" onClick={() => quickToy("plane")}>✈️</button>
              <button role="menuitem" title="光點" onClick={() => quickToy("light")}>✨</button>
              <button role="menuitem" title="逗貓棒" onClick={() => quickToy("wand")}>🪶</button>
              <button role="menuitem" title="小物件（她只會好奇地看看）" onClick={() => quickToy("trinket")}>🧸</button>
              <button role="menuitem" title="收走玩具" onClick={() => quickToy("clear")}>🧹</button>
            </div>
          )}
          <button role="menuitem" onClick={() => quick("talk")}>
            對小樞說話…
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
          onClose={() => setInputOpen(false)}
          onBubble={setBubble}
          line={line}
          conversationCtx={() => ({
            openAgentSessions: 0, // CompanionInput 以實際 sessions 數覆蓋
            msSinceInteraction: Date.now() - behaviorState.current.lastInteractionAt,
            expressiveness: behaviorRef.current.allowCasualBubbles ? "natural" : "quiet",
          })}
          onIntent={(intent) => {
            // L1 語意意圖也要經過 Director：truthState 永遠不可點播，
            // 冷卻中就不重播（playable() 白名單在 Director 內）。
            const action = directorRef.current.react(intent, Date.now(), 2500, microRng.current);
            if (action) {
              apply({
                type: "transient",
                kind: "performing",
                animation: action.expression,
                durationMs: action.durationMs,
              });
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
function CompanionInput({
  onClose,
  onBubble,
  line,
  conversationCtx,
  onIntent,
}: {
  onClose: () => void;
  onBubble: (t: string | null) => void;
  line: (key: string) => string | null;
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
      if (target === "local") {
        await api.pushObservation("session.input", { text: t, source: "desktop-companion" }, 1.0);
      } else {
        await api.agentSessionSend(target, "task", { task: t, source: "desktop-companion" });
      }
      await api
        .pushObservation("companion.text-input", { kind: "text-submitted", modality: "text" }, 1.0)
        .catch(() => {});
      if (target === "local") {
        // L1 Conversation Provider：決定是否回話、語氣與 behaviorIntent。
        // 觀察已真實記錄（上面 push 成功）；模板回覆不冒充理解。
        const ctx = conversationCtx?.() ?? {
          openAgentSessions: sessions.length,
          msSinceInteraction: 0,
          expressiveness: "natural",
        };
        const result = activeConversationProvider().considerReply(t, {
          ...ctx,
          openAgentSessions: sessions.length,
        });
        onBubble(result.reply ?? line("text-received"));
        if (result.behaviorIntent && onIntent) onIntent(result.behaviorIntent);
      } else {
        onBubble(line("delegated"));
      }
      setTimeout(() => onBubble(null), 3500);
    } catch (e) {
      onBubble(`送出失敗：${e}`);
      setTimeout(() => onBubble(null), 4000);
    }
  }

  const selected = sessions.find((s) => s.sessionId === target);
  return (
    <div className="companion-input" role="dialog" aria-label="對小樞說話">
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
  line: (key: string) => string | null
): Promise<boolean> {
  try {
    await push(
      RECEPTOR_FOR_KIND["companion-dropped"],
      {
        kind: "companion-dropped",
        modality: "file-drop",
        attachments: paths,
        mayLeaveDevice: false,
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
