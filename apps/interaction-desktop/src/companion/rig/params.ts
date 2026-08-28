// 小樞 v3「女僕正式版」執行期參數化 rig — 參數模型。
//
// 邏輯畫布 128×128、腳底錨定 (64,124)。所有通道都是 (params) 純資料：
// Interaction Director / 表情系統 / 微動作疊加層只操作這份參數，
// 由 draw.ts 純函式繪出。數值一律有界（clampParams），插值可預測
// （lerpParams）——不存在「壞參數畫壞畫面」的路徑。
//
// 服裝參與功能呈現（spec §4.2）：
//   左耳=感知(冷藍)、右耳=行動(暖橙)、胸前核心=Runtime/AI 工作、
//   頭飾=網路/Agent 連線、裙擺細光=waiting/unknown/blocked 輔助、
//   尾巴=指向/情緒、圍裙口袋=道具。

export type RigPose = "stand" | "sit" | "lie" | "crouch";
export type RigMouth =
  | "soft"
  | "smile"
  | "smirk"
  | "cat"
  | "open"
  | "flat"
  | "pout"
  | "none";
export type RigArmPose =
  | "down" // 自然下垂
  | "front" // 雙手交疊身前（女僕待機）
  | "raise" // 雙手上舉（ask/歡呼）
  | "hug" // 抱（尾巴/物件）
  | "pocket" // 手插圍裙口袋
  | "reach" // 單手前伸（指/戳）
  | "stretch" // 伸懶腰
  | "block"; // 伸手擋（游標）
export type RigOverlay =
  | "none"
  | "question"
  | "cloud"
  | "stop"
  | "zzz"
  | "check"
  | "cross"
  | "drop"
  | "spark"
  | "heart"
  | "dots"; // 「…」思考/等待
export type RigSkirtTone = "none" | "amber" | "violet" | "red";
export type RigParticles = "none" | "dust" | "sparkle" | "zzz" | "heart";

export interface RigParams {
  pose: RigPose;
  /**
   * 0..1 目前 pose 的權重（1＝完全就位）。
   *
   * 姿勢是字串通道，插值時會在中點硬切；lie↔stand/sit 的頭部與身體高度
   * 差很大（~46px），硬切會單幀瞬移。lerpParams 在 lie 相關的切換期間把
   * 這個通道從 0.5 附近連續帶過，draw.ts 的 layout 依它在「另一個姿勢」
   * 與「目前姿勢」之間線性插值頭中心與身體高度。
   */
  poseBlend: number;
  /** px 垂直起伏（負=上）。呼吸/跳躍前壓。 */
  bodyBob: number;
  /** deg 全身傾斜。 */
  bodyLean: number;
  /** -0.5..0.5 squash(+壓扁)/stretch(-拉長)。 */
  squash: number;

  /** deg 歪頭。 */
  headTilt: number;
  /** -1..1 頭水平轉向（臉部特徵平移）。 */
  headTurn: number;
  /** -1..1 點頭俯仰（+低頭 -抬下巴）。 */
  headNod: number;

  /** 0 閉 … 1 全開。 */
  eyeOpen: number;
  /** 0..1 上眼皮下垂（半瞇=0.45）。 */
  eyeLid: number;
  /** -3..3 px 視線。 */
  pupilX: number;
  pupilY: number;
  /** 0.7..1.4 瞳孔縮放（好奇放大）。 */
  pupilScale: number;
  /** -1(憂/怒)…0…1(挑眉)。 */
  browL: number;
  browR: number;

  mouth: RigMouth;
  /** 0..1 小虎牙（得意時露出）。 */
  fang: number;
  /** 0..1 腮紅。 */
  blush: number;
  /** 0..1 冷汗（尷尬/裝沒事）。 */
  sweat: number;

  /** 0(放鬆貼後)…1(全立)。 */
  earPerk: number;
  /** 0..1 左耳冷藍感知光。 */
  earL: number;
  /** 0..1 右耳暖橙行動光。 */
  earR: number;
  /** deg 個別耳偏（不對稱=困惑）。 */
  earLTilt: number;
  earRTilt: number;

  /** -1..1 髮束/髮尾擺動（secondary motion）。 */
  hairSway: number;
  /** deg 頭飾歪斜（奔跑後歪掉→扶正）。 */
  headpieceTilt: number;
  /** 0..1 頭飾光（網路/Agent 連線）。 */
  headpieceGlow: number;

  /** 0..1 胸前核心亮度（Runtime/AI 工作）。 */
  coreGlow: number;
  /** 0..1 核心呼吸相位。 */
  corePulse: number;

  armPose: RigArmPose;
  /** 0..1 手臂動作進行度（reach 伸多遠、raise 舉多高）。 */
  armPhase: number;
  /** -1..1 慣用側（reach/block 用哪隻手；負=左）。 */
  armSide: number;

  /** deg 尾巴抬起角（0=下垂、60=高豎）。 */
  tailAngle: number;
  /** 0..1 尾梢捲曲。 */
  tailCurl: number;
  /** 0..1 尾巴繞到身前。 */
  tailWrap: number;
  /** 0..1 尾尖紫光（工具）。 */
  tailTip: number;
  /** -1..1 尾巴左右擺相位。 */
  tailSway: number;

  /** -1..1 腳步/晃腳相位。 */
  legPhase: number;

  /** 0..1 裙擺細光強度。 */
  skirtGlow: number;
  skirtTone: RigSkirtTone;

  overlay: RigOverlay;
  /** 0..1 浮標動畫相位。 */
  overlayPhase: number;

  particles: RigParticles;
  /** 0..1 粒子相位。 */
  particlePhase: number;

  /** 0..1 全體降暗（offline/paused）。 */
  dim: number;
  /** 0..1 policy 小盾。 */
  shield: number;
}

export const DEFAULT_PARAMS: RigParams = {
  pose: "stand",
  poseBlend: 1,
  bodyBob: 0,
  bodyLean: 0,
  squash: 0,
  headTilt: 0,
  headTurn: 0,
  headNod: 0,
  eyeOpen: 1,
  eyeLid: 0,
  pupilX: 0,
  pupilY: 0,
  pupilScale: 1,
  browL: 0,
  browR: 0,
  mouth: "soft",
  fang: 0,
  blush: 0,
  sweat: 0,
  earPerk: 0.45,
  earL: 0,
  earR: 0,
  earLTilt: 0,
  earRTilt: 0,
  hairSway: 0,
  headpieceTilt: 0,
  headpieceGlow: 0.25,
  coreGlow: 0.35,
  corePulse: 0,
  armPose: "front",
  armPhase: 0,
  armSide: 1,
  tailAngle: 24,
  tailCurl: 0.35,
  tailWrap: 0,
  tailTip: 0,
  tailSway: 0,
  legPhase: 0,
  skirtGlow: 0,
  skirtTone: "none",
  overlay: "none",
  overlayPhase: 0,
  particles: "none",
  particlePhase: 0,
  dim: 0,
  shield: 0,
};

/** 每個數值參數的硬界線（clamp 專用；字串參數走白名單）。 */
const NUM_BOUNDS: Record<string, [number, number]> = {
  poseBlend: [0, 1],
  bodyBob: [-14, 10],
  bodyLean: [-18, 18],
  squash: [-0.5, 0.5],
  headTilt: [-22, 22],
  headTurn: [-1, 1],
  headNod: [-1, 1],
  eyeOpen: [0, 1],
  eyeLid: [0, 0.85],
  pupilX: [-3, 3],
  pupilY: [-3, 3],
  pupilScale: [0.7, 1.4],
  browL: [-1, 1],
  browR: [-1, 1],
  fang: [0, 1],
  blush: [0, 1],
  sweat: [0, 1],
  earPerk: [0, 1],
  earL: [0, 1],
  earR: [0, 1],
  earLTilt: [-30, 30],
  earRTilt: [-30, 30],
  hairSway: [-1, 1],
  headpieceTilt: [-25, 25],
  headpieceGlow: [0, 1],
  coreGlow: [0, 1],
  corePulse: [0, 1],
  armPhase: [0, 1],
  armSide: [-1, 1],
  tailAngle: [-10, 70],
  tailCurl: [0, 1],
  tailWrap: [0, 1],
  tailTip: [0, 1],
  tailSway: [-1, 1],
  legPhase: [-1, 1],
  skirtGlow: [0, 1],
  overlayPhase: [0, 1],
  particlePhase: [0, 1],
  dim: [0, 1],
  shield: [0, 1],
};

const POSES: RigPose[] = ["stand", "sit", "lie", "crouch"];
const MOUTHS: RigMouth[] = ["soft", "smile", "smirk", "cat", "open", "flat", "pout", "none"];
const ARM_POSES: RigArmPose[] = [
  "down",
  "front",
  "raise",
  "hug",
  "pocket",
  "reach",
  "stretch",
  "block",
];
const OVERLAYS: RigOverlay[] = [
  "none",
  "question",
  "cloud",
  "stop",
  "zzz",
  "check",
  "cross",
  "drop",
  "spark",
  "heart",
  "dots",
];
const SKIRT_TONES: RigSkirtTone[] = ["none", "amber", "violet", "red"];
const PARTICLES: RigParticles[] = ["none", "dust", "sparkle", "zzz", "heart"];

/**
 * 任意輸入 → 有界合法參數。未知字串值回退預設；非數值/NaN 回退預設。
 *
 * 數值通道**只接受 `typeof === "number"` 且有限**的值：`Number()` 強制轉型
 * 會把 `null`/`""`/`[]` 變 0、`true` 變 1、`"3"` 變 3——那是把壞資料悄悄
 * 當成合法參數，不是誠實的回退。
 */
export function clampParams(p: Partial<RigParams>): RigParams {
  const out: RigParams = { ...DEFAULT_PARAMS };
  for (const [key, value] of Object.entries(p)) {
    if (!(key in DEFAULT_PARAMS)) continue;
    const k = key as keyof RigParams;
    if (typeof DEFAULT_PARAMS[k] === "number") {
      if (typeof value !== "number" || !Number.isFinite(value)) continue;
      const [lo, hi] = NUM_BOUNDS[k] ?? [-1e6, 1e6];
      (out as unknown as Record<string, unknown>)[k] = Math.max(lo, Math.min(hi, value));
    } else {
      const allowed: readonly string[] =
        k === "pose"
          ? POSES
          : k === "mouth"
            ? MOUTHS
            : k === "armPose"
              ? ARM_POSES
              : k === "overlay"
                ? OVERLAYS
                : k === "skirtTone"
                  ? SKIRT_TONES
                  : k === "particles"
                    ? PARTICLES
                    : [];
      if (typeof value === "string" && allowed.includes(value)) {
        (out as unknown as Record<string, unknown>)[k] = value;
      }
    }
  }
  return out;
}

/** 允許的外推範圍：ease 的回彈（easeOutBackLite 過衝 ~5%）要真的畫得出來。 */
export const LERP_T_MIN = -0.2;
export const LERP_T_MAX = 1.2;

/**
 * 參數插值：數值 lerp、字串在 t>=0.5 切換。輸出必然合法（clampParams 收尾）。
 *
 * `t` 允許落在 [-0.2, 1.2]：時間軸用 easeOutBackLite 做回彈，若在這裡把 t
 * 夾回 [0,1]，過衝就被吃掉、過場完全沒有回彈。外推後仍由 clampParams 保證
 * 每個通道在硬界線內。
 */
export function lerpParams(a: RigParams, b: RigParams, t: number): RigParams {
  const tt = Math.max(LERP_T_MIN, Math.min(LERP_T_MAX, Number.isFinite(t) ? t : 0));
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(DEFAULT_PARAMS) as (keyof RigParams)[]) {
    const av = a[key];
    const bv = b[key];
    if (typeof av === "number" && typeof bv === "number") {
      out[key] = av + (bv - av) * tt;
    } else {
      out[key] = tt >= 0.5 ? bv : av;
    }
  }
  // 姿勢硬切的補償：lie ↔ stand/sit 的 layout 差 ~46px，中點硬切會讓頭部
  // 單幀瞬移。這裡把「目前姿勢的權重」連續帶過切換點（t=0→1、t=0.5 兩側
  // 都是 0.5），draw.ts 的 layout 依它插值頭中心與身體高度。
  if (a.pose !== b.pose && (a.pose === "lie" || b.pose === "lie")) {
    const k = Math.max(0, Math.min(1, tt));
    out.poseBlend = k < 0.5 ? 1 - k : k;
  }
  return clampParams(out as Partial<RigParams>);
}

/**
 * 姿勢過場：把 `pose` 的切換點與 `poseBlend` 綁在**同一個進度**上。
 *
 * 只處理有 `lie` 參與的切換（stand↔sit 只差 10px，硬切看不出來）。呼叫端
 * 給的 `k` 必須是線性進度：跟著回彈 ease 走的話，第一幀就會前進三成多，
 * 頭部照樣跳 ~16px。
 */
export function blendPose(
  from: RigParams,
  to: RigParams,
  params: RigParams,
  k: number
): RigParams {
  if (from.pose === to.pose) return params;
  if (from.pose !== "lie" && to.pose !== "lie") return params;
  const kk = Math.max(0, Math.min(1, Number.isFinite(k) ? k : 1));
  return clampParams({
    ...params,
    pose: kk >= 0.5 ? to.pose : from.pose,
    poseBlend: kk < 0.5 ? 1 - kk : kk,
  });
}

// ---------------------------------------------------------------------------
// 調色盤（女僕正式版）：奶白＋深灰紫主色、冷藍/暖橙能力訊號、
// 低飽和粉紫細節。變體只改配色與少量比例，不改任何權限或行為。
// ---------------------------------------------------------------------------

export interface RigPalette {
  /** 髮色（柔黑/深灰紫）。 */
  hair: string;
  hairEdge: string;
  hairShine: string;
  /** 皮膚。 */
  skin: string;
  skinEdge: string;
  /** 洋裝主體（深灰紫）。 */
  dress: string;
  dressEdge: string;
  /** 奶白（圍裙/泡泡袖/頭飾）。 */
  cream: string;
  creamEdge: string;
  creamShade: string;
  /** 眼睛。 */
  eyeWhite: string;
  iris: string;
  irisDeep: string;
  pupil: string;
  /** 能力訊號色。 */
  coolBlue: string;
  warmOrange: string;
  toolViolet: string;
  coreTeal: string;
  /** 低飽和粉紫（腮紅/肉球/蝴蝶結陰影）。 */
  pinkLilac: string;
  /** 靴子。 */
  boot: string;
  /** 比例微調。 */
  eyeScale: number;
  earSize: number;
  lidBias: number;
  perkBias: number;
}

export const RIG_PALETTES: Record<string, RigPalette> = {
  // 正式預設：奶白×深灰紫，紫灰眼。
  "maid-classic": {
    hair: "#38323f",
    hairEdge: "#2a2530",
    hairShine: "#57506b",
    skin: "#f6e7dc",
    skinEdge: "#d9bfae",
    dress: "#453d58",
    dressEdge: "#332d42",
    cream: "#f4ecdd",
    creamEdge: "#c9bda6",
    creamShade: "#e3d8c4",
    eyeWhite: "#fbf7ff",
    iris: "#a794c9",
    irisDeep: "#6d5b93",
    pupil: "#2c2438",
    coolBlue: "#4aa3ff",
    warmOrange: "#ff9d4a",
    toolViolet: "#b48bff",
    coreTeal: "#57e6c4",
    pinkLilac: "#d8a7c4",
    boot: "#3a3348",
    eyeScale: 1,
    earSize: 1,
    lidBias: 0,
    perkBias: 0.1,
  },
  // 暮色：更深的紫、暖一點的奶白（慵懶氛圍）。
  "maid-dusk": {
    hair: "#332f42",
    hairEdge: "#262336",
    hairShine: "#4d4868",
    skin: "#f3e4d9",
    skinEdge: "#d6bcab",
    dress: "#3b3550",
    dressEdge: "#2b263c",
    cream: "#efe4d0",
    creamEdge: "#c3b59c",
    creamShade: "#ddd0b8",
    eyeWhite: "#f6f2fa",
    iris: "#9b8bbd",
    irisDeep: "#615183",
    pupil: "#282136",
    coolBlue: "#6fa9e8",
    warmOrange: "#eaa96e",
    toolViolet: "#b9a4e6",
    coreTeal: "#8fd8c6",
    pinkLilac: "#cf9fbb",
    boot: "#332d42",
    eyeScale: 0.97,
    earSize: 0.96,
    lidBias: 0.22,
    perkBias: -0.1,
  },
  // 櫻花：低飽和粉紫比例更高、眼睛更大（活潑氛圍）。
  "maid-sakura": {
    hair: "#3d3547",
    hairEdge: "#2e2837",
    hairShine: "#5d5378",
    skin: "#f8e9de",
    skinEdge: "#dcc2b1",
    dress: "#4c4060",
    dressEdge: "#382f49",
    cream: "#f7efe2",
    creamEdge: "#cec0a9",
    creamShade: "#e8dcc8",
    eyeWhite: "#fdfaff",
    iris: "#b7a0d6",
    irisDeep: "#7a659f",
    pupil: "#2e2540",
    coolBlue: "#5cb2ff",
    warmOrange: "#ffb066",
    toolViolet: "#c49dff",
    coreTeal: "#63f0d0",
    pinkLilac: "#e2aecb",
    boot: "#413853",
    eyeScale: 1.12,
    earSize: 1.06,
    lidBias: 0,
    perkBias: 0.2,
  },
};

export const clamp01 = (v: number): number => Math.max(0, Math.min(1, v));
export const clamp = (v: number, a: number, b: number): number => Math.max(a, Math.min(b, v));

/** 顏色混合（#rrggbb）。 */
export function mixColor(hex: string, target: string, t: number): string {
  const h = (x: string) => [
    parseInt(x.slice(1, 3), 16),
    parseInt(x.slice(3, 5), 16),
    parseInt(x.slice(5, 7), 16),
  ];
  const [r1, g1, b1] = h(hex);
  const [r2, g2, b2] = h(target);
  const c = (a: number, b_: number) => Math.round(a + (b_ - a) * clamp01(t));
  return `#${[c(r1, r2), c(g1, g2), c(b1, b2)]
    .map((x) => x.toString(16).padStart(2, "0"))
    .join("")}`;
}
