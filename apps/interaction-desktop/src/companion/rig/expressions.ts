// 小樞 v3 表情庫：36 個正式表情＋基態＋相容別名。
//
// 每個表情四段式（spec §6.1/§7）：enter（一次）→ hold（基準參數）＋
// loop（小循環，疊加在 hold 上反覆）→ exit（離開）。
// 不是靜態圖片：enter/loop 都是關鍵幀時間軸，由 RigRenderer 插值。
//
// 誠實不變量：
//   - truthState=true 的表情（成功/失敗/阻擋/未知/緊急/離線）只能由
//     machine.ts 的真實狀態驅動；Director 與 presentation 白名單不得點播。
//   - success-claimed 絕無綠勾與慶祝粒子；綠勾只在 success-verified。

import { RigParams } from "./params";

export interface ExprKeyframe {
  /** 0..1 相位內時間。 */
  t: number;
  p: Partial<RigParams>;
}

export interface ExprPhase {
  frames: ExprKeyframe[];
  durationMs: number;
}

export interface Expression {
  id: string;
  label: string;
  /** 真相狀態：只能由 runtime 事件驅動，不可被點播。 */
  truthState?: boolean;
  /** 允許 gaze/ear 微動作疊加（生活感）。 */
  ambientOverlay?: boolean;
  /** 允許自動眨眼。 */
  autoBlink?: boolean;
  enter?: ExprPhase;
  hold: Partial<RigParams>;
  loop?: ExprPhase;
  exit?: ExprPhase;
}

const kf = (t: number, p: Partial<RigParams>): ExprKeyframe => ({ t, p });
const phase = (durationMs: number, ...frames: ExprKeyframe[]): ExprPhase => ({
  durationMs,
  frames,
});

/** 站姿呼吸小循環（多數 ambient 表情共用）。 */
const BREATHE = phase(
  3400,
  kf(0, { bodyBob: 0, corePulse: 0 }),
  kf(0.5, { bodyBob: -1.5, corePulse: 0.5 }),
  kf(1, { bodyBob: 0, corePulse: 1 })
);

/** 尾巴慢擺。 */
const TAIL_SWAY = phase(
  4200,
  kf(0, { tailSway: -0.5 }),
  kf(0.5, { tailSway: 0.5 }),
  kf(1, { tailSway: -0.5 })
);

const E = (
  id: string,
  label: string,
  def: Omit<Expression, "id" | "label">
): Expression => ({ id, label, ...def });

// ---------------------------------------------------------------------------
// 基礎生活（§7.1）
// ---------------------------------------------------------------------------

export const EXPRESSIONS: Record<string, Expression> = {};

function add(e: Expression) {
  EXPRESSIONS[e.id] = e;
}

add(
  E("idle", "站立呼吸", {
    ambientOverlay: true,
    autoBlink: true,
    hold: {},
    loop: phase(
      3400,
      kf(0, { bodyBob: 0, tailSway: -0.3, corePulse: 0 }),
      kf(0.5, { bodyBob: -1.6, tailSway: 0.3, corePulse: 0.5 }),
      kf(1, { bodyBob: 0, tailSway: -0.3, corePulse: 1 })
    ),
    // 離開待機：吐一口氣、尾巴歸位（下一個動作從中性姿勢起手）。
    exit: phase(
      160,
      kf(0, {}),
      kf(0.5, { bodyBob: -0.6, tailSway: 0.1 }),
      kf(1, { bodyBob: 0, tailSway: 0, corePulse: 0 })
    ),
  })
);

add(
  E("blink", "眨眼", {
    ambientOverlay: true,
    hold: {},
    enter: phase(300, kf(0, { eyeOpen: 1 }), kf(0.4, { eyeOpen: 0.05 }), kf(1, { eyeOpen: 1 })),
    loop: BREATHE,
  })
);

add(
  E("sit", "端坐", {
    ambientOverlay: true,
    autoBlink: true,
    hold: { pose: "sit" },
    loop: phase(
      3600,
      kf(0, { bodyBob: 0, tailSway: -0.4 }),
      kf(0.5, { bodyBob: -1.2, tailSway: 0.4 }),
      kf(1, { bodyBob: 0, tailSway: -0.4 })
    ),
  })
);

add(
  E("lie-flat", "趴平", {
    ambientOverlay: false,
    autoBlink: true,
    enter: phase(
      600,
      kf(0, { pose: "stand" }),
      kf(0.5, { pose: "crouch", squash: 0.3 }),
      kf(1, { pose: "lie", squash: 0 })
    ),
    hold: { pose: "lie", eyeLid: 0.35, earPerk: 0.25, mouth: "soft" },
    loop: phase(
      4200,
      kf(0, { bodyBob: 0, tailAngle: 35 }),
      kf(0.5, { bodyBob: -1, tailAngle: 45 }),
      kf(1, { bodyBob: 0, tailAngle: 35 })
    ),
  })
);

add(
  E("doze", "打瞌睡", {
    enter: phase(
      900,
      kf(0, { pose: "sit" }),
      kf(1, { pose: "sit", eyeOpen: 0.12, eyeLid: 0.6, headNod: 0.7, earPerk: 0.12 })
    ),
    hold: {
      pose: "sit",
      eyeOpen: 0.1,
      eyeLid: 0.6,
      headNod: 0.7,
      headTilt: -6,
      earPerk: 0.12,
      tailWrap: 1,
      overlay: "zzz",
      armPose: "front",
    },
    loop: phase(
      3000,
      kf(0, { headNod: 0.7, overlayPhase: 0 }),
      kf(0.6, { headNod: 0.9, overlayPhase: 0.6 }),
      kf(0.75, { headNod: 0.55, overlayPhase: 0.75 }), // 點頭驚醒半拍
      kf(1, { headNod: 0.7, overlayPhase: 1 })
    ),
  })
);

add(
  E("sleep", "熟睡", {
    hold: {
      pose: "lie",
      eyeOpen: 0,
      earPerk: 0.08,
      mouth: "soft",
      overlay: "zzz",
      tailWrap: 1,
      coreGlow: 0.15,
    },
    loop: phase(
      4200,
      kf(0, { bodyBob: 0, overlayPhase: 0 }),
      kf(0.5, { bodyBob: -1.4, overlayPhase: 0.5 }),
      kf(1, { bodyBob: 0, overlayPhase: 1 })
    ),
  })
);

add(
  E("startled-awake", "被吵醒", {
    enter: phase(
      450,
      kf(0, { pose: "lie", eyeOpen: 0 }),
      kf(0.35, { pose: "crouch", eyeOpen: 1, pupilScale: 0.8, earPerk: 1, squash: 0.25, hairSway: 1 }),
      kf(0.5, { pose: "crouch", eyeOpen: 1, earPerk: 1, squash: 0.25 }), // hit-stop
      kf(1, { pose: "stand", eyeOpen: 1, earPerk: 0.9, squash: 0, headpieceTilt: 9 })
    ),
    hold: {
      eyeOpen: 1,
      pupilScale: 0.85,
      earPerk: 0.9,
      browL: 0.5,
      browR: 0.5,
      mouth: "open",
      headpieceTilt: 9,
      tailAngle: 55,
    },
    exit: phase(500, kf(0, { headpieceTilt: 9 }), kf(1, { headpieceTilt: 0 })),
  })
);

add(
  E("yawn", "哈欠", {
    enter: phase(
      1500,
      kf(0, {}),
      kf(0.35, { mouth: "open", eyeOpen: 0.15, headNod: -0.6, squash: -0.12, earPerk: 0.2 }),
      kf(0.75, { mouth: "open", eyeOpen: 0.08, headNod: -0.7 }),
      kf(1, { mouth: "soft", eyeOpen: 0.7, headNod: 0 })
    ),
    hold: { eyeLid: 0.3, mouth: "soft" },
    loop: BREATHE,
  })
);

add(
  E("stretch", "伸懶腰", {
    enter: phase(
      1300,
      kf(0, {}),
      kf(0.2, { pose: "crouch", squash: 0.2, armPose: "front" }), // anticipation
      kf(0.65, { pose: "stand", squash: -0.4, armPose: "stretch", armPhase: 1, eyeOpen: 0.12, mouth: "open", headNod: -0.9, earPerk: 0.25 }),
      kf(0.85, { squash: -0.35, armPose: "stretch", armPhase: 0.9, eyeOpen: 0.12 }),
      kf(1, { squash: 0, armPose: "front", armPhase: 0, eyeOpen: 0.9, mouth: "soft" })
    ),
    hold: {},
    loop: BREATHE,
  })
);

add(
  E("groom", "整理儀容", {
    enter: phase(
      1600,
      kf(0, {}),
      kf(0.25, { armPose: "raise", armPhase: 0.5, armSide: 1, headTilt: 6, eyeLid: 0.3 }),
      kf(0.55, { armPose: "raise", armPhase: 0.42, headTilt: 8, hairSway: 0.6 }),
      kf(0.8, { armPose: "raise", armPhase: 0.5, headTilt: 5, hairSway: -0.3, headpieceTilt: -3 }),
      kf(1, { armPose: "front", armPhase: 0, headTilt: 0, headpieceTilt: 0 })
    ),
    hold: {},
    loop: BREATHE,
  })
);

add(
  E("look-around", "左右張望", {
    enter: phase(
      1800,
      kf(0, {}),
      kf(0.2, { pupilX: -2.5, earLTilt: -8 }), // 眼先動
      kf(0.4, { headTurn: -0.7, pupilX: -2 }), // 頭後轉
      kf(0.6, { pupilX: 2.5, headTurn: -0.3, earRTilt: 8 }),
      kf(0.8, { headTurn: 0.7, pupilX: 2 }),
      kf(1, { headTurn: 0, pupilX: 0, earLTilt: 0, earRTilt: 0 })
    ),
    hold: {},
    loop: BREATHE,
  })
);

add(
  E("spaced-out", "放空", {
    ambientOverlay: false,
    hold: {
      eyeLid: 0.42,
      pupilScale: 0.85,
      pupilY: -0.5,
      mouth: "soft",
      earPerk: 0.2,
      tailSway: 0,
      tailAngle: 12,
    },
    loop: phase(
      5200,
      kf(0, { bodyBob: 0, pupilX: 0 }),
      kf(0.5, { bodyBob: -1.2, pupilX: 0.6 }),
      kf(1, { bodyBob: 0, pupilX: 0 })
    ),
    autoBlink: true,
  })
);

add(
  E("legswing", "坐著晃腳", {
    ambientOverlay: true,
    autoBlink: true,
    hold: { pose: "sit", mouth: "smile", armPose: "down", tailAngle: 35 },
    loop: phase(
      2200,
      kf(0, { legPhase: -0.8, tailSway: -0.4 }),
      kf(0.5, { legPhase: 0.8, tailSway: 0.4 }),
      kf(1, { legPhase: -0.8, tailSway: -0.4 })
    ),
  })
);

add(
  E("tailhug", "抱尾巴", {
    autoBlink: true,
    enter: phase(
      800,
      kf(0, { pose: "sit" }),
      kf(1, { pose: "sit", tailWrap: 1, armPose: "hug", eyeLid: 0.3 })
    ),
    hold: { pose: "sit", tailWrap: 1, armPose: "hug", eyeLid: 0.3, mouth: "cat", earPerk: 0.3 },
    loop: phase(
      3800,
      kf(0, { bodyBob: 0 }),
      kf(0.5, { bodyBob: -1.2 }),
      kf(1, { bodyBob: 0 })
    ),
  })
);

// ---------------------------------------------------------------------------
// 玩家互動（§7.3）＋注意力
// ---------------------------------------------------------------------------

add(
  E("notice", "察覺", {
    enter: phase(
      700,
      kf(0, {}),
      kf(0.25, { earPerk: 1, earRTilt: 6 }), // 聰明：先耳
      kf(0.5, { pupilX: 1.8, pupilScale: 1.15 }), // 再眼
      kf(1, { headTurn: 0.4, earPerk: 0.95, pupilX: 1.2 }) // 最後頭
    ),
    hold: { earPerk: 0.95, headTurn: 0.4, pupilX: 1.2, pupilScale: 1.1, mouth: "soft", tailAngle: 45 },
    loop: TAIL_SWAY,
  })
);

add(
  E("curious", "歪頭好奇", {
    enter: phase(
      600,
      kf(0, {}),
      kf(0.6, { headTilt: 15, pupilScale: 1.35, earPerk: 1, earLTilt: -7 }),
      kf(1, { headTilt: 13, pupilScale: 1.3 })
    ),
    hold: {
      headTilt: 13,
      pupilScale: 1.3,
      earPerk: 1,
      earLTilt: -7,
      mouth: "cat",
      tailAngle: 48,
      tailCurl: 0.7,
    },
    loop: phase(
      2600,
      kf(0, { tailCurl: 0.7, headTilt: 13 }),
      kf(0.5, { tailCurl: 0.5, headTilt: 15 }),
      kf(1, { tailCurl: 0.7, headTilt: 13 })
    ),
  })
);

add(
  E("question", "疑問", {
    hold: {
      headTilt: 10,
      browL: -0.35,
      browR: 0.6,
      mouth: "pout",
      overlay: "question",
      earLTilt: -10,
      earRTilt: 6,
      earPerk: 0.75,
    },
    loop: phase(
      1600,
      kf(0, { overlayPhase: 0 }),
      kf(1, { overlayPhase: 1 })
    ),
  })
);

add(
  E("peek", "偷看", {
    enter: phase(
      700,
      kf(0, { headTurn: 0 }),
      kf(0.5, { headTurn: -0.9, bodyLean: -7, eyeLid: 0.25 }),
      kf(1, { headTurn: -0.75, bodyLean: -6 })
    ),
    hold: {
      headTurn: -0.75,
      bodyLean: -6,
      eyeLid: 0.25,
      pupilX: -2.6,
      mouth: "cat",
      earPerk: 0.85,
      tailAngle: 40,
    },
    loop: phase(
      2400,
      kf(0, { pupilX: -2.6 }),
      kf(0.5, { pupilX: -1.8 }),
      kf(1, { pupilX: -2.6 })
    ),
  })
);

add(
  E("lean-in", "探頭", {
    enter: phase(
      550,
      kf(0, {}),
      kf(0.6, { bodyLean: 9, headTurn: 0.6, squash: -0.08, earPerk: 1 }),
      kf(1, { bodyLean: 8 })
    ),
    hold: { bodyLean: 8, headTurn: 0.6, earPerk: 1, pupilX: 2, pupilScale: 1.2, mouth: "soft" },
    loop: TAIL_SWAY,
  })
);

add(
  E("deadpan", "無語", {
    hold: {
      eyeLid: 0.5,
      pupilScale: 0.8,
      mouth: "flat",
      browL: -0.2,
      browR: -0.2,
      earPerk: 0.3,
      earLTilt: -4,
      tailAngle: 8,
      sweat: 0.25,
    },
    loop: phase(5000, kf(0, { bodyBob: 0 }), kf(0.5, { bodyBob: -0.8 }), kf(1, { bodyBob: 0 })),
  })
);

add(
  E("poked", "被點", {
    enter: phase(
      450,
      kf(0, {}),
      kf(0.25, { squash: 0.3, eyeOpen: 1, pupilScale: 0.85, earPerk: 1 }),
      kf(0.4, { squash: 0.3 }), // hit-stop
      kf(0.7, { squash: -0.12, headTilt: 6 }),
      kf(1, { squash: 0, headTilt: 4, mouth: "cat" })
    ),
    hold: { headTilt: 4, mouth: "cat", earPerk: 0.9, tailAngle: 45, blush: 0.3 },
    loop: TAIL_SWAY,
    // 離開：歪頭回正時輕微過衝（follow-through），腮紅退掉。
    exit: phase(
      220,
      kf(0, {}),
      kf(0.55, { headTilt: -2, blush: 0.15, earPerk: 0.75 }),
      kf(1, { headTilt: 0, blush: 0, mouth: "soft" })
    ),
  })
);

add(
  E("poked-rapid", "被連戳", {
    enter: phase(
      700,
      kf(0, {}),
      kf(0.15, { squash: 0.28, headTilt: -5 }),
      kf(0.3, { squash: 0.1, headTilt: 5 }),
      kf(0.45, { squash: 0.28, headTilt: -5 }),
      kf(0.6, { squash: 0.1, headTilt: 5 }),
      kf(1, { squash: 0, headTilt: 0, browL: -0.6, browR: -0.6, mouth: "pout", blush: 0.5 })
    ),
    hold: {
      browL: -0.6,
      browR: -0.6,
      mouth: "pout",
      blush: 0.5,
      earLTilt: -12,
      earRTilt: 12,
      earPerk: 0.6,
      tailAngle: 58,
      sweat: 0.3,
    },
    loop: phase(
      1800,
      kf(0, { tailSway: -0.7 }),
      kf(0.5, { tailSway: 0.7 }),
      kf(1, { tailSway: -0.7 })
    ),
    // 離開：嘟嘴鬆開＋最後甩一下尾巴（抗議收工，不是原諒）。
    exit: phase(
      260,
      kf(0, {}),
      kf(0.4, { tailSway: -0.9, browL: -0.35, browR: -0.35, sweat: 0.15 }),
      kf(0.75, { tailSway: 0.4, mouth: "flat", blush: 0.25 }),
      kf(1, { tailSway: 0, browL: 0, browR: 0, mouth: "soft", blush: 0, sweat: 0, earLTilt: 0, earRTilt: 0 })
    ),
  })
);

add(
  E("lifted", "被拖起", {
    enter: phase(
      400,
      kf(0, {}),
      kf(0.4, { squash: -0.3, earPerk: 1, eyeOpen: 1, pupilScale: 0.8, mouth: "open" }),
      kf(1, { squash: -0.25 })
    ),
    hold: {
      squash: -0.25,
      earPerk: 1,
      pupilScale: 0.8,
      pupilY: 1.5,
      mouth: "open",
      tailAngle: 62,
      tailCurl: 0.8,
      legPhase: 0.6,
      hairSway: 0.7,
      headpieceTilt: 6,
      armPose: "down",
    },
    loop: phase(
      900,
      kf(0, { legPhase: 0.6 }),
      kf(0.5, { legPhase: -0.6 }),
      kf(1, { legPhase: 0.6 })
    ),
    // 離開懸空：停止踢腿、拉長的身體回彈、頭髮與頭飾晚一步歸位。
    exit: phase(
      240,
      kf(0, {}),
      kf(0.45, { legPhase: -0.2, squash: 0.06, hairSway: 0.4, headpieceTilt: 3 }),
      kf(1, { legPhase: 0, squash: 0, hairSway: 0, headpieceTilt: 0, mouth: "soft", pupilY: 0 })
    ),
  })
);

add(
  E("wobbly-landing", "落地站不穩", {
    enter: phase(
      900,
      kf(0, { squash: -0.2, bodyBob: -6 }),
      kf(0.2, { squash: 0.4, bodyBob: 0, particles: "dust", particlePhase: 0 }),
      kf(0.3, { squash: 0.4, particlePhase: 0.2 }), // hit-stop
      kf(0.55, { squash: -0.1, bodyLean: -8, particlePhase: 0.5, armPose: "block", armPhase: 0.6 }),
      kf(0.75, { bodyLean: 6, particlePhase: 0.8 }),
      kf(1, { bodyLean: 0, squash: 0, particlePhase: 1, armPose: "front" })
    ),
    hold: { headpieceTilt: -8, sweat: 0.4, mouth: "flat" },
    exit: phase(600, kf(0, { headpieceTilt: -8 }), kf(1, { headpieceTilt: 0, sweat: 0 })),
  })
);

add(
  E("pretend-not-hear", "假裝沒聽見", {
    hold: {
      headTurn: -0.5,
      eyeLid: 0.4,
      pupilX: -2,
      mouth: "soft",
      earPerk: 0.25,
      earRTilt: 14, // 耳朵誠實地朝向聲源
      tailSway: 0.3,
    },
    loop: phase(
      3600,
      kf(0, { earRTilt: 14 }),
      kf(0.5, { earRTilt: 8 }),
      kf(1, { earRTilt: 14 })
    ),
  })
);

add(
  E("sneak-closer", "悄悄靠近", {
    hold: {
      pose: "crouch",
      bodyLean: 6,
      headNod: 0.3,
      eyeLid: 0.2,
      pupilScale: 1.2,
      earPerk: 1,
      mouth: "cat",
      tailAngle: 15,
      armPose: "front",
    },
    loop: phase(
      1400,
      kf(0, { bodyBob: 0, legPhase: -0.5 }),
      kf(0.5, { bodyBob: -2, legPhase: 0.5 }),
      kf(1, { bodyBob: 0, legPhase: -0.5 })
    ),
  })
);

add(
  E("block-cursor", "伸手擋游標", {
    enter: phase(
      500,
      kf(0, {}),
      kf(0.6, { armPose: "block", armPhase: 1, headTurn: 0.5, browR: 0.6, mouth: "pout" }),
      kf(1, { armPose: "block", armPhase: 1 })
    ),
    hold: {
      armPose: "block",
      armPhase: 1,
      headTurn: 0.5,
      browL: -0.4,
      browR: 0.6,
      mouth: "pout",
      earRTilt: 8,
      tailAngle: 50,
    },
    loop: TAIL_SWAY,
  })
);

add(
  E("hold-ball", "抱球", {
    hold: {
      armPose: "hug",
      mouth: "cat",
      blush: 0.4,
      earPerk: 0.85,
      tailAngle: 50,
      pupilY: 1.2,
    },
    loop: phase(
      2600,
      kf(0, { bodyBob: 0, tailSway: -0.5 }),
      kf(0.5, { bodyBob: -1.5, tailSway: 0.5 }),
      kf(1, { bodyBob: 0, tailSway: -0.5 })
    ),
  })
);

add(
  E("keep-ball", "不還球", {
    hold: {
      armPose: "hug",
      headTurn: -0.7,
      bodyLean: -6,
      mouth: "pout",
      eyeLid: 0.25,
      pupilX: 2.4, // 偷瞄你
      earPerk: 0.7,
      tailAngle: 58,
      blush: 0.3,
    },
    loop: phase(
      2000,
      kf(0, { pupilX: 2.4 }),
      kf(0.6, { pupilX: 1.2 }),
      kf(1, { pupilX: 2.4 })
    ),
  })
);

add(
  E("pounce-miss", "撲空", {
    enter: phase(
      1100,
      kf(0, { pose: "crouch", squash: 0.3, earPerk: 1, pupilScale: 1.3 }), // anticipation
      kf(0.3, { pose: "stand", squash: -0.35, bodyBob: -8, bodyLean: 10, armPose: "reach", armPhase: 1 }),
      kf(0.5, { squash: 0.35, bodyBob: 0, bodyLean: 4, particles: "dust", particlePhase: 0.2 }),
      kf(0.62, { squash: 0.35 }), // hit-stop
      kf(1, { squash: 0, bodyLean: 0, armPose: "front", particlePhase: 1, pupilScale: 1, eyeOpen: 1 })
    ),
    hold: { mouth: "flat", browL: -0.3, browR: -0.3, sweat: 0.5, earLTilt: -8, earRTilt: 8, earPerk: 0.4 },
    exit: phase(500, kf(0, { sweat: 0.5 }), kf(1, { sweat: 0 })),
  })
);

add(
  E("slip-play-cool", "滑倒裝沒事", {
    enter: phase(
      1400,
      kf(0, { bodyLean: -14, squash: 0.3, eyeOpen: 1, pupilScale: 0.8, hairSway: -1, particles: "dust", particlePhase: 0 }),
      kf(0.18, { bodyLean: -14, squash: 0.3 }), // hit-stop
      kf(0.45, { bodyLean: 0, squash: 0, particlePhase: 0.6 }),
      kf(0.7, { armPose: "raise", armPhase: 0.4, headTilt: 5, eyeLid: 0.3, hairSway: 0.4 }), // 整理袖口
      kf(1, { armPose: "front", headTilt: 0, mouth: "smirk", eyeLid: 0.3, particlePhase: 1 })
    ),
    hold: { mouth: "smirk", eyeLid: 0.3, sweat: 0.5, blush: 0.3, tailSway: 0.4, earPerk: 0.6 },
    exit: phase(600, kf(0, { sweat: 0.5 }), kf(1, { sweat: 0 })),
  })
);

add(
  E("praised", "被稱讚", {
    enter: phase(
      700,
      kf(0, {}),
      kf(0.4, { eyeLid: 0.45, headNod: -0.6, tailAngle: 62, earPerk: 0.9, mouth: "smirk", fang: 1 }),
      kf(1, { eyeLid: 0.45, headNod: -0.5 })
    ),
    hold: {
      eyeLid: 0.45,
      headNod: -0.5,
      mouth: "smirk",
      fang: 1,
      blush: 0.7,
      tailAngle: 62,
      earPerk: 0.9,
      particles: "heart",
    },
    loop: phase(
      2200,
      kf(0, { tailSway: -0.6, particlePhase: 0 }),
      kf(0.5, { tailSway: 0.6, particlePhase: 0.5 }),
      kf(1, { tailSway: -0.6, particlePhase: 1 })
    ),
  })
);

add(
  E("caught-slacking", "偷懶被抓", {
    enter: phase(
      800,
      kf(0, { pose: "sit", eyeLid: 0.5 }),
      kf(0.25, { pose: "sit", eyeOpen: 1, eyeLid: 0, pupilScale: 0.75, earPerk: 1, squash: 0.15 }),
      kf(0.45, { pose: "sit", squash: 0.15 }), // 定格
      kf(0.8, { pose: "stand", pupilX: -2.5, headTurn: -0.4, armPose: "raise", armPhase: 0.4 }),
      kf(1, { pose: "stand", armPose: "front", pupilX: -2 })
    ),
    hold: {
      pupilX: -2,
      headTurn: -0.4,
      sweat: 0.8,
      mouth: "flat",
      earLTilt: -12,
      earRTilt: 12,
      earPerk: 0.9,
      tailAngle: 60,
    },
    exit: phase(600, kf(0, { sweat: 0.8 }), kf(1, { sweat: 0 })),
  })
);

add(
  E("await-player", "等玩家", {
    ambientOverlay: true,
    autoBlink: true,
    hold: { pose: "sit", earPerk: 0.6, mouth: "soft", tailAngle: 30 },
    loop: phase(
      5200,
      kf(0, { pupilX: 0, tailSway: -0.3, bodyBob: 0 }),
      kf(0.3, { pupilX: 1.8, tailSway: 0.1 }),
      kf(0.6, { pupilX: -1.5, tailSway: 0.4, bodyBob: -1.2 }),
      kf(1, { pupilX: 0, tailSway: -0.3, bodyBob: 0 })
    ),
  })
);

add(
  E("player-back", "玩家回來", {
    enter: phase(
      900,
      kf(0, { pose: "sit" }),
      kf(0.25, { pose: "sit", earPerk: 1, pupilScale: 1.25 }),
      kf(0.6, { pose: "stand", bodyBob: -3, tailAngle: 58, mouth: "cat" }),
      kf(1, { pose: "stand", bodyBob: 0, tailAngle: 55 })
    ),
    hold: { earPerk: 0.95, mouth: "cat", tailAngle: 55, blush: 0.3 },
    loop: TAIL_SWAY,
  })
);

add(
  E("play-chase", "追玩具", {
    hold: {
      bodyLean: 6,
      headNod: 0.2,
      pupilScale: 1.25,
      earPerk: 1,
      mouth: "cat",
      tailAngle: 42,
      armPose: "down",
    },
    loop: phase(
      900,
      kf(0, { tailSway: -0.6 }),
      kf(0.5, { tailSway: 0.6 }),
      kf(1, { tailSway: -0.6 })
    ),
  })
);

add(
  E("play-carry", "叼著玩具", {
    hold: {
      mouth: "cat",
      blush: 0.3,
      earPerk: 0.85,
      armPose: "hug",
      tailAngle: 52,
    },
    loop: TAIL_SWAY,
  })
);

// ---------------------------------------------------------------------------
// AI 與工作（§7.4）
// ---------------------------------------------------------------------------

add(
  E("listening", "聆聽", {
    hold: {
      earPerk: 1,
      earL: 0.9, // 感知耳亮
      headTilt: 6,
      pupilScale: 1.15,
      mouth: "soft",
      tailAngle: 40,
    },
    loop: phase(
      2000,
      kf(0, { earLTilt: 0 }),
      kf(0.5, { earLTilt: -5 }),
      kf(1, { earLTilt: 0 })
    ),
  })
);

add(
  E("thinking", "思考", {
    hold: {
      pupilY: -1.8,
      pupilX: 1.4,
      eyeLid: 0.2,
      mouth: "flat",
      overlay: "cloud",
      earPerk: 0.6,
      armPose: "front",
      coreGlow: 0.7,
      tailCurl: 0.7,
    },
    loop: phase(
      2400,
      kf(0, { overlayPhase: 0, corePulse: 0 }),
      kf(1, { overlayPhase: 1, corePulse: 1 })
    ),
  })
);

add(
  E("routing", "找資料", {
    hold: {
      pupilScale: 1.1,
      eyeLid: 0.15,
      mouth: "soft",
      armPose: "pocket",
      earPerk: 0.8,
      coreGlow: 0.7,
      headNod: 0.3,
      tailTip: 0.8, // 工具尾尖亮
    },
    loop: phase(
      1600,
      kf(0, { pupilX: -1.8, corePulse: 0 }),
      kf(0.5, { pupilX: 1.8, corePulse: 0.5 }),
      kf(1, { pupilX: -1.8, corePulse: 1 })
    ),
  })
);

add(
  E("working", "努力工作", {
    hold: {
      headNod: 0.4,
      eyeLid: 0.18,
      mouth: "flat",
      armPose: "pocket",
      earR: 0.9, // 行動耳亮
      earPerk: 0.75,
      coreGlow: 1,
      headpieceGlow: 0.8,
      tailAngle: 35,
    },
    loop: phase(
      1500,
      kf(0, { corePulse: 0, bodyBob: 0 }),
      kf(0.5, { corePulse: 0.5, bodyBob: -0.8 }),
      kf(1, { corePulse: 1, bodyBob: 0 })
    ),
  })
);

add(
  E("waiting", "等待結果", {
    hold: {
      eyeLid: 0.25,
      mouth: "soft",
      earPerk: 0.5,
      skirtGlow: 0.7,
      skirtTone: "amber",
      overlay: "dots",
      armPose: "front",
      coreGlow: 0.6,
    },
    loop: phase(
      2100,
      kf(0, { overlayPhase: 0, corePulse: 0 }),
      kf(1, { overlayPhase: 1, corePulse: 1 })
    ),
  })
);

add(
  E("wait-codex", "等 Codex", {
    hold: {
      eyeLid: 0.2,
      mouth: "soft",
      earPerk: 0.6,
      overlay: "dots",
      headpieceGlow: 0.9,
      armPose: "front",
      coreGlow: 0.7,
      headTilt: 4,
    },
    loop: phase(
      2100,
      kf(0, { overlayPhase: 0, corePulse: 0 }),
      kf(1, { overlayPhase: 1, corePulse: 1 })
    ),
    // 離開等待：「…」收掉、頭飾光回到連線基準、歪頭回正。
    exit: phase(
      200,
      kf(0, {}),
      kf(0.5, { overlayPhase: 1, headpieceGlow: 0.6, headTilt: 2 }),
      kf(1, { overlay: "none", headpieceGlow: 0.35, headTilt: 0, eyeLid: 0, corePulse: 0 })
    ),
  })
);

add(
  E("wait-claude", "等 Claude", {
    hold: {
      eyeLid: 0.2,
      mouth: "soft",
      earPerk: 0.6,
      overlay: "dots",
      headpieceGlow: 0.9,
      armPose: "front",
      coreGlow: 0.7,
      headTilt: -4,
    },
    loop: phase(
      2100,
      kf(0, { overlayPhase: 0, corePulse: 0 }),
      kf(1, { overlayPhase: 1, corePulse: 1 })
    ),
  })
);

add(
  E("ask", "需要確認", {
    // 真相狀態：「需要你確認」代表 runtime 真的在等 consent／輸入
    // （waiting-consent / waiting-input）。AI 不得經 presentation 點播它來
    // 假裝需要授權——想表達疑惑請用非真相的 `question`。
    truthState: true,
    enter: phase(
      600,
      kf(0, {}),
      kf(0.6, { armPose: "raise", armPhase: 0.85, earPerk: 1, mouth: "open", browL: 0.6, browR: 0.6 }),
      kf(1, { armPose: "raise", armPhase: 0.8 })
    ),
    hold: {
      armPose: "raise",
      armPhase: 0.8,
      earPerk: 1,
      mouth: "open",
      browL: 0.6,
      browR: 0.6,
      overlay: "question",
      headpieceGlow: 0.8,
      tailAngle: 50,
    },
    loop: phase(1600, kf(0, { overlayPhase: 0 }), kf(1, { overlayPhase: 1 })),
  })
);

add(
  E("not-found", "找不到", {
    hold: {
      headTilt: -8,
      browL: -0.5,
      browR: -0.5,
      mouth: "pout",
      earLTilt: -12,
      earRTilt: 12,
      earPerk: 0.35,
      overlay: "question",
      sweat: 0.4,
      armPose: "raise",
      armPhase: 0.3,
      armSide: -1,
    },
    loop: phase(1900, kf(0, { overlayPhase: 0 }), kf(1, { overlayPhase: 1 })),
  })
);

// ---- 真相狀態（truthState：只能由 runtime 事件驅動） ----

add(
  E("success-claimed", "聲稱完成", {
    truthState: true,
    // 誠實：只點頭；沒有綠勾、沒有慶祝粒子、尾巴不高豎。
    enter: phase(
      900,
      kf(0, {}),
      kf(0.35, { headNod: 0.5, mouth: "smile" }),
      kf(0.65, { headNod: 0.1 }),
      kf(1, { headNod: 0.35, mouth: "smile" })
    ),
    hold: { headNod: 0.2, mouth: "smile", earPerk: 0.6, coreGlow: 0.6 },
  })
);

add(
  E("success-verified", "驗證成功", {
    truthState: true,
    enter: phase(
      1100,
      kf(0, {}),
      kf(0.2, { pose: "crouch", squash: 0.18 }), // anticipation
      kf(0.5, { pose: "stand", bodyBob: -5, squash: -0.2, tailAngle: 62, mouth: "smile", fang: 1, overlay: "check" }),
      kf(0.7, { bodyBob: 0, squash: 0.08, particles: "sparkle", particlePhase: 0.2 }),
      kf(1, { squash: 0, overlay: "check", particlePhase: 0.6 })
    ),
    hold: {
      overlay: "check",
      mouth: "smile",
      fang: 1,
      blush: 0.5,
      earPerk: 0.9,
      tailAngle: 60,
      coreGlow: 0.9,
      particles: "sparkle",
    },
    loop: phase(
      2400,
      kf(0, { particlePhase: 0, tailSway: -0.5 }),
      kf(0.5, { particlePhase: 0.5, tailSway: 0.5 }),
      kf(1, { particlePhase: 1, tailSway: -0.5 })
    ),
    // 離開：綠勾與慶祝粒子收掉、尾巴放下（回到中性，不留下慶祝殘影）。
    exit: phase(
      260,
      kf(0, {}),
      kf(0.45, { tailAngle: 50, particlePhase: 1 }),
      kf(1, { overlay: "none", particles: "none", tailAngle: 30, fang: 0, blush: 0.1, mouth: "soft" })
    ),
  })
);

add(
  E("blocked", "權限不足", {
    truthState: true,
    hold: {
      shield: 1,
      mouth: "flat",
      browL: -0.4,
      browR: -0.4,
      earPerk: 0.5,
      earLTilt: -6,
      tailAngle: 20,
      skirtGlow: 0.7,
      skirtTone: "red",
      armPose: "front",
    },
    loop: phase(2600, kf(0, { corePulse: 0 }), kf(1, { corePulse: 1 })),
  })
);

add(
  E("unknown", "結果未知", {
    truthState: true,
    hold: {
      browL: -0.4,
      browR: -0.4,
      mouth: "flat",
      earLTilt: -10,
      earRTilt: 10,
      earPerk: 0.4,
      overlay: "question",
      sweat: 0.5,
      skirtGlow: 0.8,
      skirtTone: "violet",
      armPose: "front",
    },
    loop: phase(2000, kf(0, { overlayPhase: 0 }), kf(1, { overlayPhase: 1 })),
  })
);

add(
  E("failed", "工作失敗", {
    truthState: true,
    enter: phase(
      1000,
      kf(0, {}),
      kf(0.3, { eyeOpen: 1, pupilScale: 0.8, earPerk: 0.9 }), // 愣住
      kf(0.5, { eyeOpen: 1, pupilScale: 0.8 }), // 定格
      kf(1, { headNod: 0.4, eyeLid: 0.2, mouth: "flat", overlay: "cross" })
    ),
    // 不崩潰、不責怪：認真檢查現場的樣子。
    hold: {
      headNod: 0.4,
      eyeLid: 0.2,
      mouth: "flat",
      overlay: "cross",
      earPerk: 0.55,
      tailAngle: 12,
      coreGlow: 0.45,
      armPose: "pocket",
    },
    // 離開：叉號收掉、把頭抬回來、耳朵重新立起（準備提出下一步）。
    exit: phase(
      240,
      kf(0, {}),
      kf(0.5, { headNod: 0.1, earPerk: 0.7, overlay: "cross" }),
      kf(1, { headNod: 0, eyeLid: 0, overlay: "none", earPerk: 0.6, tailAngle: 24 })
    ),
  })
);

add(
  E("emergency", "緊急停止", {
    truthState: true,
    hold: {
      dim: 1,
      overlay: "stop",
      mouth: "flat",
      eyeOpen: 0.9,
      earPerk: 0.15,
      tailAngle: 5,
      coreGlow: 0.05,
      armPose: "down",
      headpieceGlow: 0,
    },
    // 凍結：無 loop（安全狀態不做任何表演）。
  })
);

add(
  E("offline", "離線", {
    truthState: true,
    hold: {
      dim: 0.7,
      eyeOpen: 0.15,
      eyeLid: 0.5,
      mouth: "flat",
      earPerk: 0.1,
      tailAngle: 3,
      coreGlow: 0,
      headpieceGlow: 0,
      armPose: "down",
    },
  })
);

add(
  E("paused", "已暫停", {
    truthState: true,
    hold: {
      pose: "sit",
      eyeLid: 0.4,
      mouth: "soft",
      earPerk: 0.3,
      dim: 0.25,
      tailWrap: 1,
      armPose: "front",
      coreGlow: 0.2,
    },
  })
);

add(
  E("quiet", "安靜陪伴", {
    autoBlink: true,
    hold: {
      pose: "sit",
      eyeLid: 0.35,
      mouth: "soft",
      earPerk: 0.35,
      tailWrap: 1,
      coreGlow: 0.25,
      armPose: "front",
    },
    loop: phase(4600, kf(0, { bodyBob: 0 }), kf(0.5, { bodyBob: -1 }), kf(1, { bodyBob: 0 })),
  })
);

// ---------------------------------------------------------------------------
// v0.5 補充表情（不屬於 OFFICIAL_36，但由真實事件/落地判定驅動）。
// 全部非 truthState：沒有綠勾、沒有慶祝粒子，也不冒充「完成」。
// ---------------------------------------------------------------------------

add(
  E("land-light", "輕巧落地", {
    // 低速、低落差的放下：小幅吸收後直接站好（沒有踉蹌、不裝沒事）。
    enter: phase(
      420,
      kf(0, { squash: -0.12, bodyBob: -3 }),
      kf(0.25, { pose: "crouch", squash: 0.18, bodyBob: 0 }),
      kf(0.6, { pose: "stand", squash: -0.05, mouth: "cat" }),
      kf(1, { squash: 0, bodyBob: 0, mouth: "cat", earPerk: 0.8 })
    ),
    hold: { mouth: "cat", earPerk: 0.8, tailAngle: 40 },
    loop: TAIL_SWAY,
    exit: phase(
      160,
      kf(0, {}),
      kf(0.5, { tailSway: 0.25, earPerk: 0.62 }),
      kf(1, { tailSway: 0, mouth: "soft", earPerk: 0.55 })
    ),
  })
);

add(
  E("device-hello", "裝置上線", {
    // 右耳（行動側）亮起＋看過去。不代表裝置「可用」，只代表剛連上。
    enter: phase(
      600,
      kf(0, {}),
      kf(0.3, { earPerk: 1, earRTilt: 8, earR: 0.9 }), // 先耳
      kf(0.6, { pupilX: 2, pupilScale: 1.15 }), // 再眼
      kf(1, { headTurn: 0.35, earR: 0.85, pupilX: 1.4 }) // 最後頭
    ),
    hold: { earR: 0.85, earPerk: 0.95, headTurn: 0.35, pupilX: 1.4, mouth: "cat", tailAngle: 48 },
    loop: phase(
      2200,
      kf(0, { earR: 0.7, tailSway: -0.35 }),
      kf(0.5, { earR: 1, tailSway: 0.35 }),
      kf(1, { earR: 0.7, tailSway: -0.35 })
    ),
    exit: phase(
      200,
      kf(0, {}),
      kf(0.5, { earR: 0.5, headTurn: 0.15 }),
      kf(1, { earR: 0, headTurn: 0, pupilX: 0, mouth: "soft", earPerk: 0.55 })
    ),
  })
);

add(
  E("device-lost", "裝置離線", {
    // 耳朵下垂：連線沒了就是沒了，不演成「還在」。
    enter: phase(
      520,
      kf(0, {}),
      kf(0.35, { earPerk: 0.15, earLTilt: -14, earRTilt: 14, earR: 0 }),
      kf(1, { earPerk: 0.12, mouth: "flat", headNod: 0.25, tailAngle: 10 })
    ),
    hold: {
      earPerk: 0.12,
      earLTilt: -14,
      earRTilt: 14,
      earR: 0,
      mouth: "flat",
      headNod: 0.25,
      tailAngle: 10,
      skirtGlow: 0.4,
      skirtTone: "amber",
    },
    loop: phase(3000, kf(0, { tailSway: -0.15 }), kf(0.5, { tailSway: 0.1 }), kf(1, { tailSway: -0.15 })),
    exit: phase(
      200,
      kf(0, {}),
      kf(0.5, { earPerk: 0.35, headNod: 0.1 }),
      kf(1, { earPerk: 0.5, headNod: 0, earLTilt: 0, earRTilt: 0, skirtGlow: 0, mouth: "soft" })
    ),
  })
);

add(
  E("operate-tool", "操作工具", {
    // 尾尖紫光＝正在動別的東西（不是「已完成」）。
    enter: phase(
      500,
      kf(0, {}),
      kf(0.4, { armPose: "reach", armPhase: 0.7, tailTip: 0.5, earR: 0.6 }),
      kf(1, { armPose: "reach", armPhase: 0.6, tailTip: 0.9 })
    ),
    hold: {
      armPose: "reach",
      armPhase: 0.6,
      tailTip: 0.9,
      tailAngle: 38,
      earR: 0.7,
      earPerk: 0.8,
      eyeLid: 0.15,
      mouth: "flat",
      coreGlow: 0.7,
    },
    loop: phase(
      1400,
      kf(0, { tailTip: 0.55, corePulse: 0 }),
      kf(0.5, { tailTip: 1, corePulse: 0.6 }),
      kf(1, { tailTip: 0.55, corePulse: 1 })
    ),
    exit: phase(
      200,
      kf(0, {}),
      kf(0.5, { tailTip: 0.4, armPhase: 0.3 }),
      kf(1, { tailTip: 0, armPose: "front", armPhase: 0, earR: 0 })
    ),
  })
);

add(
  E("ack-nod", "收到（短點頭）", {
    // acknowledged ≠ completed：只點一下頭表示收到，沒有勾、沒有粒子。
    enter: phase(
      420,
      kf(0, {}),
      kf(0.35, { headNod: 0.45, earPerk: 0.85 }),
      kf(0.7, { headNod: -0.08 }), // overshoot
      kf(1, { headNod: 0.05, mouth: "soft" })
    ),
    hold: { headNod: 0.05, earPerk: 0.7, mouth: "soft", tailAngle: 34 },
    loop: TAIL_SWAY,
    exit: phase(
      160,
      kf(0, {}),
      kf(0.5, { headNod: -0.04 }),
      kf(1, { headNod: 0, earPerk: 0.5 })
    ),
  })
);

// ---------------------------------------------------------------------------
// 相容別名：machine.ts / presentation 白名單使用的動畫名 → 表情 id。
// ---------------------------------------------------------------------------

export const EXPRESSION_ALIASES: Record<string, string> = {
  act: "working",
  move: "look-around",
  clicked: "poked",
  dragged: "lifted",
  lie: "lie-flat",
  // 舊 success 動畫由 renderer 依 frameSlice 區分 claimed/verified，
  // 此處預設為誠實的 claimed。
  success: "success-claimed",
};

/** 取得表情（含別名解析）；不存在回傳 null（呼叫端決定 fallback）。 */
export function resolveExpression(name: string): Expression | null {
  const id = EXPRESSION_ALIASES[name] ?? name;
  return EXPRESSIONS[id] ?? null;
}

/** 安全 fallback 鏈（沿用 sprite renderer 的原則：絕不 fallback 到成功）。 */
export const RIG_FALLBACKS: Record<string, string[]> = {
  emergency: ["paused", "offline", "idle"],
  offline: ["paused", "idle"],
  blocked: ["paused", "idle"],
  unknown: ["paused", "idle"],
  failed: ["blocked", "paused", "idle"],
  "success-claimed": ["idle"],
  "success-verified": ["idle"],
  default: ["idle"],
};

/** 36 個正式表情 id（驗收清單 §7.5，順序照 spec）。 */
export const OFFICIAL_36: string[] = [
  "question",
  "peek",
  "curious",
  "lean-in",
  "deadpan",
  "spaced-out",
  "yawn",
  "lie-flat",
  "stretch",
  "startled-awake",
  "pretend-not-hear",
  "sneak-closer",
  "poked",
  "poked-rapid",
  "lifted",
  "wobbly-landing",
  "hold-ball",
  "keep-ball",
  "pounce-miss",
  "slip-play-cool",
  "praised",
  "caught-slacking",
  "await-player",
  "player-back",
  "thinking",
  "routing",
  "working",
  "wait-codex",
  "wait-claude",
  "ask",
  "blocked",
  "not-found",
  "unknown",
  "success-claimed",
  "success-verified",
  "failed",
];
