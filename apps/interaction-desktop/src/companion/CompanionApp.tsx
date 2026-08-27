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
  initial,
  MachineState,
  mapRuntimeEvent,
  pose,
  reduce,
} from "./machine";
import { PackManifest, SpriteRenderer, validateManifest } from "./renderer";
import { planPresentationCommand } from "./presentationCommands";
import {
  BehaviorState as CompanionBehaviorState,
  initialBehavior,
  noteEvent,
  noteInterruption,
  noteUserInteraction,
  scheduleMicroAction,
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

export default function CompanionApp() {
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  const rendererRef = React.useRef<SpriteRenderer | null>(null);
  const machineRef = React.useRef<MachineState>(initial);
  const lastBubbleAt = React.useRef(0);
  const [bubble, setBubble] = React.useState<string | null>(null);
  const [menuOpen, setMenuOpen] = React.useState(false);
  const [packName, setPackName] = React.useState("shu-agile");
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

  /** Resolve a bubble line: safety keys fixed, others persona-styled. */
  const line = React.useCallback((key: string): string | null => {
    return resolveLine(key, personaRef.current);
  }, []);

  /** Show a bubble for `ms` (0 = sticky). Clears any prior timer so an older
   *  bubble's timeout can never erase a newer safety bubble early. */
  const showBubble = React.useCallback((text: string | null, ms: number) => {
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
    let un: (() => void) | null = null;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      un = await listen("companion-reload", () => window.location.reload());
    })();
    return () => {
      if (un) un();
    };
  }, []);

  // ---- boot: transport, pack, renderer ----
  React.useEffect(() => {
    let disposed = false;
    (async () => {
      await bootstrapSupervisor();
      let pack = "shu-agile";
      let personaId = "persona-shu";
      try {
        const prefs = await desktop.prefsGet();
        pack = prefs.companionPack ?? pack;
        personaId = prefs.companionPersona ?? personaId;
        behaviorRef.current = behaviorFor(prefs.companionExpressiveness ?? "natural");
        storyProgress.current = prefs.storyProgress ?? {};
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
      )) as PackManifest;
      const issues = validateManifest(manifest);
      if (issues.length > 0) {
        console.error("invalid character pack", issues);
        return;
      }
      if (disposed || !canvasRef.current) return;
      const renderer = new SpriteRenderer(
        canvasRef.current,
        manifest,
        `/packs/${pack}/sheet.png`,
        1.1
      );
      renderer.setReducedMotion(window.matchMedia("(prefers-reduced-motion: reduce)").matches);
      rendererRef.current = renderer;
      setReady(true);
    })();
    return () => {
      disposed = true;
      rendererRef.current?.destroy();
    };
  }, []);

  // ---- machine driving ----
  // Behavior Runtime（生命底層）狀態：平滑量、微動作反重複、seeded RNG。
  const behaviorState = React.useRef<CompanionBehaviorState>(initialBehavior(Date.now()));
  const recentMicro = React.useRef<string[]>([]);
  const microRng = React.useRef(seededRng(Date.now() >>> 0));

  const apply = React.useCallback((ev: Parameters<typeof reduce>[1]) => {
    const before = machineRef.current.transient;
    machineRef.current = reduce(machineRef.current, ev, Date.now());
    // 微動作被真實事件搶佔 → 記一次打斷（收斂之後的主動表現）。
    const after = machineRef.current.transient;
    if (before?.kind === "performing" && after && after.kind !== "performing") {
      behaviorState.current = noteInterruption(behaviorState.current);
    }
    setBaseState(machineRef.current.base);
    syncPose();
  }, []);

  const syncPose = React.useCallback(() => {
    const p = pose(machineRef.current, Date.now());
    rendererRef.current?.setAnimation(p.animation, p.frameSlice);
  }, []);

  const pushInteraction = React.useCallback((kind: string, extra?: Record<string, unknown>) => {
    behaviorState.current = noteUserInteraction(behaviorState.current, Date.now());
    const receptor = RECEPTOR_FOR_KIND[kind] ?? "desktop.companion.interaction";
    void api.pushObservation(receptor, { kind, ...extra }, 1.0).catch(() => {
      /* receptor disabled, companion hidden, or runtime offline: stays local */
    });
  }, []);

  // ---- presentation commands: runtime → this surface → honest ack ----
  const handlePresentationCommand = React.useCallback(
    (payload: Record<string, unknown>) => {
      const command = String(payload["command"] ?? "");
      const actionId = typeof payload["actionId"] === "string" ? payload["actionId"] : null;
      if (command === "cancel" || command === "clear-all") {
        // Cancelled/estopped: drop any non-safety visual (safety poses are
        // driven by base state, not by presentation commands).
        showBubble(null, 0);
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
        showBubble(plan.bubble.text, plan.bubble.ms);
        pushInteraction("bubble-shown", { source: "presentation-command" });
      }
      if (plan.presence !== undefined && isTauri) {
        void desktop.prefsPatch({ companionVisible: plan.presence }).catch(() => {});
      }
      void api.presentationAck(actionId, plan.outcome, plan.detail).catch(() => {
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
      void api.presentationHello(!document.hidden, packName).catch(() => {});
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
        handlePresentationCommand(e.payload);
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
        const quiet = Boolean(s["quietHours"]);
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
      const busy =
        t != null && ["acting", "waiting-for-receipt", "routing", "thinking"].includes(t.kind);
      const waitingForHuman = t?.kind === "requesting-consent";
      behaviorState.current = stepBehavior(behaviorState.current, {
        busy,
        waitingForHuman,
        msSinceInteraction: now - behaviorState.current.lastInteractionAt,
      });
      syncPose();
      const p = pose(m, now);
      if (!p.ambient) return;
      const expr = behaviorRef.current.allowCasualBubbles
        ? behaviorRef.current.bubbleCooldownMs < 60_000
          ? 1.5
          : 1
        : 0.5;
      const micro = scheduleMicroAction(
        behaviorState.current,
        {
          ambient: true,
          reducedMotion:
            typeof window.matchMedia === "function" &&
            window.matchMedia("(prefers-reduced-motion: reduce)").matches,
          quiet: m.base === "quiet",
          expressiveness: expr,
          msSinceInteraction: now - behaviorState.current.lastInteractionAt,
          recent: recentMicro.current,
        },
        microRng.current
      );
      if (micro) {
        recentMicro.current = [...recentMicro.current.slice(-3), micro.id];
        apply({
          type: "transient",
          kind: "performing",
          animation: micro.animation,
          frameSlice: micro.frameSlice,
          durationMs: micro.durationMs,
        });
      }
    }, 500);
    return () => {
      stopped = true;
      clearInterval(t);
      clearInterval(pump);
      un.then((f) => f()).catch(() => {});
    };
  }, [ready, apply, syncPose, handlePresentationCommand]);

  function maybeBubble(e: RuntimeEvent) {
    const now = Date.now();
    const base = machineRef.current.base;
    // Safety lines are FIXED (packs cannot restyle them) and always show.
    if (e.eventType === "emergency.stop" && e.payload["cleared"] !== true) {
      showBubble(line("emergency"), 0); // sticky
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
      showBubble(safetyText, 5000);
      return;
    }
    // Casual lines: persona-styled, expressiveness-gated, cooldown-limited.
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

  // ---- semantic interaction events (NEVER raw coordinates) ----
  const lastApproachAt = React.useRef(0);
  const firstOnline = React.useRef(false);

  function onPointerEnterCanvas() {
    const now = Date.now();
    if (now - lastApproachAt.current > 30_000) {
      lastApproachAt.current = now;
      pushInteraction("pointer-approached");
      // 游標靠近時看過來（不追蹤、不記錄座標）。
      apply({ type: "transient", kind: "listening", durationMs: 1200 });
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

  // ---- pointer: click vs drag ----
  const dragState = React.useRef<{ x: number; y: number; dragging: boolean } | null>(null);

  async function onPointerDown(e: React.PointerEvent) {
    dragState.current = { x: e.clientX, y: e.clientY, dragging: false };
  }

  async function onPointerMove(e: React.PointerEvent) {
    const d = dragState.current;
    if (!d || d.dragging) return;
    if (Math.abs(e.clientX - d.x) + Math.abs(e.clientY - d.y) > 5) {
      d.dragging = true;
      apply({ type: "transient", kind: "dragged" });
      pushInteraction("companion-dragged");
      if (isTauri) {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        await win.startDragging().catch(() => {});
        // startDragging blocks until drop: persist the new position.
        try {
          const pos = await win.outerPosition();
          await desktop.prefsPatch({
            companionPosition: [pos.x, pos.y],
          } as never);
        } catch {
          /* best effort */
        }
      }
      dragState.current = null;
    }
  }

  function onPointerUp() {
    const d = dragState.current;
    dragState.current = null;
    if (d && !d.dragging) {
      apply({ type: "transient", kind: "clicked" });
      pushInteraction("companion-clicked");
      setMenuOpen((v) => !v);
      setInputOpen(false);
    }
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
        className="companion-canvas"
        aria-label={`桌面角色小樞（${packName}）`}
        role="img"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerEnter={onPointerEnterCanvas}
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
              pushInteraction("companion-dropped", {
                modality: "file-drop",
                attachments: dropPreview,
                mayLeaveDevice: false,
              });
              setDropPreview(null);
              showBubble(line("drop-received"), 3000);
            }}
          >
            記錄這些項目
          </button>
          <button onClick={() => setDropPreview(null)}>取消（不做任何事）</button>
        </div>
      )}
      {menuOpen && (
        <div className="companion-menu" role="menu" aria-label="快捷操作">
          <button role="menuitem" onClick={() => quick("talk")}>
            對小樞說話…
          </button>
          <button role="menuitem" onClick={() => quick("tasks")}>
            查看目前工作
          </button>
          <button role="menuitem" onClick={() => quick("pause-1h")}>
            暫停一小時
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
        <CompanionInput onClose={() => setInputOpen(false)} onBubble={setBubble} line={line} />
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
}: {
  onClose: () => void;
  onBubble: (t: string | null) => void;
  line: (key: string) => string | null;
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
      onBubble(target === "local" ? line("text-received") : line("delegated"));
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
