// 小樞 v3「女僕正式版」— 分層 canvas 純繪製。
//
// (ctx, params, palette) → 一幀。無狀態、確定性；所有動態都來自 params。
// 邏輯座標 128×128、腳底錨定 (64,124)。呼叫端負責 scale/DPR transform。
//
// 圖層順序（背→前）：
//   尾巴 → 後髮 → 腿/靴 → 裙後層 → 燈籠褲 → 裙前層(裙擺細光) → 軀幹洋裝
//   → 圍裙 → 手臂/泡泡袖 → 領口/蝴蝶結+核心 → 頭(臉/嘴/眼/瀏海)
//   → 貓耳 → 頭飾 → 不對稱髮束 → 盾/浮標/粒子
//
// 角色設計（spec §4）：約 2.5–2.6 頭身、大頭低重心、圓潤輪廓；
// 女性化但無成熟成人特徵；女僕工作服非性感裝（不透膚、蓬裙+燈籠褲）。

import {
  clamp,
  clamp01,
  mixColor,
  POSE_HEAD_Y,
  RigArmPose,
  RigPalette,
  RigParams,
} from "./params";

type Ctx = CanvasRenderingContext2D;

const TAU = Math.PI * 2;

export interface Layout {
  /** 頭中心。 */
  hx: number;
  hy: number;
  /** 頭半徑。 */
  hrx: number;
  hry: number;
  /** 腰（裙起點）y。 */
  waistY: number;
  /** 裙擺 y。 */
  hemY: number;
  /** 地面 y。 */
  groundY: number;
  /** 肩 y。 */
  shoulderY: number;
  lie: boolean;
}

function lieLayout(): Layout {
  return {
    hx: 52,
    hy: POSE_HEAD_Y.lie,
    hrx: 21,
    hry: 19,
    waistY: 104,
    hemY: 118,
    groundY: 124,
    shoulderY: 100,
    lie: true,
  };
}

function uprightLayout(pose: RigParams["pose"], pal: RigPalette): Layout {
  const sit = pose === "sit";
  const crouch = pose === "crouch";
  // stand：頭中心 46；sit 整體下移 10；crouch 下移 6（高度表由 params.ts 共用，
  // blendPose 的姿勢混合也吃同一份，避免兩邊各寫一套）。
  const drop = POSE_HEAD_Y[pose] - POSE_HEAD_Y.stand;
  return {
    hx: 64,
    hy: POSE_HEAD_Y.stand + drop,
    hrx: 21.5 * (1 + (pal.eyeScale - 1) * 0.1),
    hry: 19.5,
    waistY: 84 + drop,
    hemY: (sit ? 108 : 106) + (crouch ? 4 : 0),
    groundY: 124,
    shoulderY: 66 + drop,
    lie: false,
  };
}

/** 直立（stand/sit/crouch）身體的水平錨點。 */
export const UPRIGHT_ANCHOR_X = 64;
/** 趴姿身體的水平錨點。 */
export const LIE_ANCHOR_X = 52;

/** 這個姿勢的軀幹水平錨點（＝身體繪製函式實際用的中心 x）。 */
export function bodyAnchorX(pose: RigParams["pose"]): number {
  return pose === "lie" ? LIE_ANCHOR_X : UPRIGHT_ANCHOR_X;
}

/**
 * 姿勢過場中**整個角色**的水平位移（px）。
 *
 * lie 的身體畫在 x=52 附近、直立的畫在 64；以前 `layoutFor` 直接把頭中心 `hx`
 * 在兩者之間插值，但只有頭／貓耳／頭飾讀 `hx`，兩個身體繪製函式都是各自的
 * 固定座標——於是 lie↔直立的每一次過場中，頭會相對軀幹橫向漂移最多 6.24px、
 * 持續 230～280ms（對抗審查 rig-renderer-045）。現在 `hx` 一律等於目標姿勢的
 * 錨點（頭與軀幹永遠對齊），錨點差改成整個角色的水平位移，由 drawRig 一次
 * 帶過：她是「站起來時整個人挪過去」，不是「頭自己飄」。
 */
export function poseShiftX(p: RigParams): number {
  const blend = clamp01(p.poseBlend);
  if (p.poseFrom === p.pose || blend >= 0.999) return 0;
  return (bodyAnchorX(p.poseFrom) - bodyAnchorX(p.pose)) * (1 - blend);
}

/**
 * 這一幀的「坐姿程度」0..1：`pose` 字串在 poseBlend 通過 0.5 那一幀才翻面，
 * 但裙擺半寬與腿型不能跟著硬切（stand↔sit 一次跳 4px／側、腿部輪廓整組替換，
 * 對抗審查 rig-renderer-047）。這裡把它變成連續量。
 */
export function sitAmount(p: RigParams): number {
  const to = p.pose === "sit" ? 1 : 0;
  if (p.poseFrom === p.pose) return to;
  const from = p.poseFrom === "sit" ? 1 : 0;
  return from + (to - from) * clamp01(p.poseBlend);
}

/** 裙擺半寬（純坐姿 27／其餘 23；過場中連續）。 */
export function skirtFlare(p: RigParams): number {
  return 23 + 4 * sitAmount(p);
}

/** 兩個 layout 的線性混合（`lie` 旗標與水平錨點跟著目標姿勢，不混合）。 */
function mixLayout(from: Layout, to: Layout, k: number): Layout {
  const m = (a: number, b: number) => a + (b - a) * k;
  return {
    hx: to.hx,
    hy: m(from.hy, to.hy),
    hrx: m(from.hrx, to.hrx),
    hry: m(from.hry, to.hry),
    waistY: m(from.waistY, to.waistY),
    hemY: m(from.hemY, to.hemY),
    groundY: m(from.groundY, to.groundY),
    shoulderY: m(from.shoulderY, to.shoulderY),
    lie: to.lie,
  };
}

function poseLayout(pose: RigParams["pose"], pal: RigPalette): Layout {
  return pose === "lie" ? lieLayout() : uprightLayout(pose, pal);
}

/**
 * 姿勢 → 版面。`poseBlend < 1` 時（任何姿勢切換的過場中）頭中心高度與身體高度
 * 在 `poseFrom` 與 `pose` 兩個版面之間線性插值，避免字串通道中點硬切造成的
 * 單幀瞬移。水平錨點 `hx` **不**插值——它永遠是目標姿勢的軀幹錨點，過場的水平
 * 差由 `poseShiftX()` 移動整個角色（rig-renderer-045）。匯出供測試量測
 * 「連續兩幀頭部位移」。
 *
 * 舊版把「另一端」硬寫成 lie／stand，所以 crouch↔lie（startled-awake 的
 * enter）之類的過場混到錯的版面、切換點還會多跳幾 px；現在來源姿勢由
 * `poseFrom` 明寫（對抗審查 rig-renderer-056）。`poseFrom` 未被寫入時等於
 * `pose`，代表沒有過場。
 */
export function layoutFor(p: RigParams, pal: RigPalette): Layout {
  const blend = clamp01(p.poseBlend);
  const cur = poseLayout(p.pose, pal);
  if (blend >= 0.999 || p.poseFrom === p.pose) return cur;
  return mixLayout(poseLayout(p.poseFrom, pal), cur, blend);
}

// ---------------------------------------------------------------------------
// 幾何形變（stand↔sit 的腿部）：兩個形狀都表示成「四段三次貝茲的封閉輪廓」，
// 端點與原本的畫法完全相同（直線邊的控制點落在邊上；橢圓用標準 kappa 弧），
// 中間依 poseBlend 逐點混合——不是字串硬切，也不是疊兩層半透明鬼影。
// ---------------------------------------------------------------------------

interface Pt {
  x: number;
  y: number;
}

/** 一段三次貝茲（起點是上一段的終點）。 */
interface Seg {
  c1: Pt;
  c2: Pt;
  to: Pt;
}

/** 四段封閉輪廓。 */
interface Loop4 {
  start: Pt;
  segs: [Seg, Seg, Seg, Seg];
}

const KAPPA = 0.5522847498307936;

const pt = (x: number, y: number): Pt => ({ x, y });
const mixPt = (a: Pt, b: Pt, k: number): Pt => pt(a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k);

/** 直線邊的四邊形（順時針：右上→右下→左下→左上）。 */
function quadLoop(tr: Pt, br: Pt, bl: Pt, tl: Pt): Loop4 {
  const edge = (from: Pt, to: Pt): Seg => ({
    c1: mixPt(from, to, 1 / 3),
    c2: mixPt(from, to, 2 / 3),
    to,
  });
  return { start: tr, segs: [edge(tr, br), edge(br, bl), edge(bl, tl), edge(tl, tr)] };
}

/** 橢圓（順時針：右→下→左→上），與 ctx.ellipse 同形。 */
function ellipseLoop(cx: number, cy: number, rx: number, ry: number): Loop4 {
  const ox = rx * KAPPA;
  const oy = ry * KAPPA;
  return {
    start: pt(cx + rx, cy),
    segs: [
      { c1: pt(cx + rx, cy + oy), c2: pt(cx + ox, cy + ry), to: pt(cx, cy + ry) },
      { c1: pt(cx - ox, cy + ry), c2: pt(cx - rx, cy + oy), to: pt(cx - rx, cy) },
      { c1: pt(cx - rx, cy - oy), c2: pt(cx - ox, cy - ry), to: pt(cx, cy - ry) },
      { c1: pt(cx + ox, cy - ry), c2: pt(cx + rx, cy - oy), to: pt(cx + rx, cy) },
    ],
  };
}

function mixLoop(a: Loop4, b: Loop4, k: number): Loop4 {
  const seg = (i: number): Seg => ({
    c1: mixPt(a.segs[i].c1, b.segs[i].c1, k),
    c2: mixPt(a.segs[i].c2, b.segs[i].c2, k),
    to: mixPt(a.segs[i].to, b.segs[i].to, k),
  });
  return { start: mixPt(a.start, b.start, k), segs: [seg(0), seg(1), seg(2), seg(3)] };
}

function loopPath(ctx: Ctx, loop: Loop4) {
  ctx.beginPath();
  ctx.moveTo(loop.start.x, loop.start.y);
  for (const sg of loop.segs) ctx.bezierCurveTo(sg.c1.x, sg.c1.y, sg.c2.x, sg.c2.y, sg.to.x, sg.to.y);
  ctx.closePath();
}

/** 圓角路徑工具。 */
function ellipse(ctx: Ctx, cx: number, cy: number, rx: number, ry: number, rot = 0) {
  ctx.beginPath();
  ctx.ellipse(cx, cy, Math.max(0.1, rx), Math.max(0.1, ry), rot, 0, TAU);
}

function fillStroke(ctx: Ctx, fill: string, stroke?: string, width = 1) {
  ctx.fillStyle = fill;
  ctx.fill();
  if (stroke) {
    ctx.strokeStyle = stroke;
    ctx.lineWidth = width;
    ctx.stroke();
  }
}

/** 主繪製入口。 */
export function drawRig(ctx: Ctx, p: RigParams, pal: RigPalette): void {
  const L = layoutFor(p, pal);
  const dim = clamp01(p.dim);
  // 降暗：主要色一次性混灰。
  const hair = mixColor(pal.hair, "#565360", dim * 0.55);
  const hairEdge = mixColor(pal.hairEdge, "#4a4854", dim * 0.5);
  const skin = mixColor(pal.skin, "#c9c2bd", dim * 0.5);
  const dress = mixColor(pal.dress, "#59556a", dim * 0.55);
  const dressEdge = mixColor(pal.dressEdge, "#47445a", dim * 0.5);
  const cream = mixColor(pal.cream, "#cfcac0", dim * 0.45);
  const creamEdge = mixColor(pal.creamEdge, "#a8a296", dim * 0.4);
  const boot = mixColor(pal.boot, "#4c4858", dim * 0.5);

  ctx.save();
  // 姿勢過場的水平帶過（lie↔直立的錨點差；頭與軀幹一起走）。
  const shiftX = poseShiftX(p);
  if (shiftX !== 0) ctx.translate(shiftX, 0);
  // squash & stretch + 全身傾斜（腳底為軸）。
  const sy = 1 - p.squash * 0.16;
  const sx = 1 + p.squash * 0.12;
  ctx.translate(64, L.groundY);
  ctx.rotate((p.bodyLean * Math.PI) / 180);
  ctx.scale(sx, sy);
  ctx.translate(-64, -L.groundY);
  ctx.translate(0, p.bodyBob);

  // ---------------- 尾巴（後層） ----------------
  const tailBehind = p.tailWrap < 0.5;
  if (tailBehind) drawTail(ctx, p, pal, L, hair, hairEdge, dim);

  if (L.lie) {
    drawLieBody(ctx, p, pal, L, { hair, hairEdge, skin, dress, dressEdge, cream, creamEdge, boot });
  } else {
    drawUprightBody(ctx, p, pal, L, {
      hair,
      hairEdge,
      skin,
      dress,
      dressEdge,
      cream,
      creamEdge,
      boot,
    });
  }

  // ---------------- 頭 ----------------
  drawHead(ctx, p, pal, L, { hair, hairEdge, skin, cream, creamEdge, dim });

  // 尾巴繞前（安靜/抱尾）畫在最上身體層。
  if (!tailBehind) drawTail(ctx, p, pal, L, hair, hairEdge, dim);

  // policy 小盾。
  if (p.shield > 0.02) drawShield(ctx, L, clamp01(p.shield));

  ctx.restore();

  // 浮標與粒子不受身體變形影響。
  drawOverlay(ctx, p, pal);
  drawParticles(ctx, p, pal, L);
}

interface Cols {
  hair: string;
  hairEdge: string;
  skin: string;
  dress: string;
  dressEdge: string;
  cream: string;
  creamEdge: string;
  boot: string;
}

// ---------------------------------------------------------------------------
// 站/坐/蹲身體
// ---------------------------------------------------------------------------

function drawUprightBody(
  ctx: Ctx,
  p: RigParams,
  pal: RigPalette,
  L: Layout,
  c: Cols
) {
  const crouch = p.pose === "crouch";
  const legLift = p.legPhase; // -1..1
  // 坐姿程度（0..1）：字串 `pose` 只在 poseBlend 通過 0.5 那一幀翻面，裙擺與腿型
  // 要跟著 poseBlend 連續走（rig-renderer-047）。
  const sitK = sitAmount(p);
  const mix = (a: number, b: number) => a + (b - a) * sitK;
  // 直立身體一律以 L.hx 為中心（layoutFor 保證它等於 bodyAnchorX(pose)）。
  const bx = L.hx;

  // ---- 腿與靴（短短的，圓頭軟底） ----
  const legY = L.hemY - 2;
  const footY = L.groundY - 3.2;
  for (const side of [-1, 1] as const) {
    // 站姿：小腿梯形＋踏步相位；坐姿：雙腿前伸的橢圓。兩者的座標都照舊，
    // 只是中間依 sitK 逐點混合。
    const step = crouch ? 0 : side * legLift * 3;
    const sway = side * legLift * 2.4;
    const lxStand = bx + side * 7;
    const lxSit = bx + side * 8.5;
    const standCalf = quadLoop(
      pt(lxStand + 3.4, legY),
      pt(lxStand + 3.2, footY - 2 - step),
      pt(lxStand - 3.2, footY - 2 - step),
      pt(lxStand - 3.4, legY)
    );
    const sitCalf = ellipseLoop(lxSit, legY + 6 - Math.abs(sway) * 0.4, 4.6, 7);
    loopPath(
      ctx,
      sitK <= 0 ? standCalf : sitK >= 1 ? sitCalf : mixLoop(standCalf, sitCalf, sitK)
    );
    fillStroke(ctx, c.skin, pal.skinEdge, 0.8);
    // 圓頭短靴（兩個姿勢都是橢圓：直接混參數）。
    const bootX = mix(lxStand + side * 0.8, lxSit + side * 1.2);
    const bootY = mix(footY - step, footY - sway);
    ellipse(ctx, bootX, bootY, mix(6, 6.2), mix(4.2, 4.4), 0);
    fillStroke(ctx, c.boot, c.dressEdge, 0.9);
    // 軟底
    ellipse(ctx, bootX, mix(footY - step + 2.9, footY - sway + 3), mix(5.2, 5.4), mix(1.5, 1.6), 0);
    fillStroke(ctx, c.cream);
  }

  // ---- 蓬蓬裙（多層）＋燈籠褲影 ----
  const hemY = L.hemY;
  const waistY = L.waistY;
  const flare = skirtFlare(p); // 裙擺半寬（過場中連續，不是 27/23 硬切）
  // 燈籠褲（不透明安全短褲）：裙下小小奶白蓬。
  for (const side of [-1, 1] as const) {
    ellipse(ctx, bx + side * 9, hemY - 1, 8, 5.5, 0);
    fillStroke(ctx, mixColor(c.cream, "#b9ad97", 0.25), c.creamEdge, 0.8);
  }
  // 裙後層（奶白襯裙）。
  skirtPath(ctx, bx, waistY + 3, hemY + 2.5, flare + 2.5, 4);
  fillStroke(ctx, c.cream, c.creamEdge, 0.9);
  // 裙主層（深灰紫）。
  skirtPath(ctx, bx, waistY, hemY, flare, 4);
  fillStroke(ctx, c.dress, c.dressEdge, 1.1);
  // 裙擺細光（waiting/unknown/blocked 輔助狀態）。
  if (p.skirtGlow > 0.03 && p.skirtTone !== "none") {
    const tone =
      p.skirtTone === "amber" ? "#ffcf5c" : p.skirtTone === "violet" ? pal.toolViolet : "#e5484d";
    ctx.save();
    ctx.globalAlpha = 0.35 + clamp01(p.skirtGlow) * 0.5;
    ctx.strokeStyle = tone;
    ctx.lineWidth = 1.6;
    skirtHemPath(ctx, bx, hemY, flare, 4);
    ctx.stroke();
    ctx.restore();
  }

  // ---- 軀幹（洋裝上身，小小的） ----
  ctx.beginPath();
  ctx.moveTo(bx - 11.5, L.shoulderY);
  ctx.bezierCurveTo(bx - 12.5, waistY - 6, bx - 10, waistY, bx, waistY);
  ctx.bezierCurveTo(bx + 10, waistY, bx + 12.5, waistY - 6, bx + 11.5, L.shoulderY);
  ctx.closePath();
  fillStroke(ctx, c.dress, c.dressEdge, 1);

  // ---- 圍裙（工具圍裙＋口袋）：窄於裙，讓深灰紫洋裝在兩側可見 ----
  ctx.beginPath();
  ctx.moveTo(bx - 6.2, L.shoulderY + 7);
  ctx.bezierCurveTo(bx - 8.5, waistY + 2, bx - flare * 0.5, hemY - 10, bx - flare * 0.42, hemY - 5.5);
  ctx.quadraticCurveTo(bx, hemY - 1.5, bx + flare * 0.42, hemY - 5.5);
  ctx.bezierCurveTo(bx + flare * 0.5, hemY - 10, bx + 8.5, waistY + 2, bx + 6.2, L.shoulderY + 7);
  ctx.closePath();
  fillStroke(ctx, c.cream, c.creamEdge, 0.9);
  // 兩側口袋。
  for (const side of [-1, 1] as const) {
    const px = bx + side * 8.5;
    const py = waistY + 7;
    ctx.beginPath();
    ctx.moveTo(px - 4.4, py);
    ctx.quadraticCurveTo(px, py + 6.6, px + 4.4, py);
    ctx.quadraticCurveTo(px, py + 2.2, px - 4.4, py);
    ctx.closePath();
    fillStroke(ctx, mixColor(c.cream, "#b9ad97", 0.18), c.creamEdge, 0.8);
  }

  // ---- 手臂＋泡泡袖 ----
  drawArms(ctx, p, pal, L, c);

  // ---- 領口＋蝴蝶結＋核心 ----
  ellipse(ctx, bx, L.shoulderY + 0.5, 6.4, 3.1, 0);
  fillStroke(ctx, c.cream, c.creamEdge, 0.8);
  drawBowCore(ctx, p, pal, bx, L.shoulderY + 6.2, c);
}

/** 蓬裙輪廓（扇形＋波浪襬）。 */
function skirtPath(ctx: Ctx, cx: number, topY: number, hemY: number, flare: number, scallops: number) {
  ctx.beginPath();
  ctx.moveTo(cx - 10, topY);
  ctx.bezierCurveTo(cx - flare * 0.95, topY + 6, cx - flare, hemY - 7, cx - flare, hemY - 2);
  scallopAcross(ctx, cx, -flare, flare, hemY, scallops);
  ctx.bezierCurveTo(cx + flare, hemY - 7, cx + flare * 0.95, topY + 6, cx + 10, topY);
  ctx.closePath();
}

/** 只有裙襬弧線（細光用）。 */
function skirtHemPath(ctx: Ctx, cx: number, hemY: number, flare: number, scallops: number) {
  ctx.beginPath();
  ctx.moveTo(cx - flare, hemY - 2);
  scallopAcross(ctx, cx, -flare, flare, hemY, scallops);
}

function scallopAcross(ctx: Ctx, cx: number, fromX: number, toX: number, hemY: number, n: number) {
  const w = (toX - fromX) / n;
  for (let i = 0; i < n; i++) {
    const x0 = cx + fromX + w * i;
    ctx.quadraticCurveTo(x0 + w / 2, hemY + 3.4, x0 + w, hemY - 2);
  }
}

/** 泡泡袖＋手套手臂（各姿勢）。 */
// ---------------------------------------------------------------------------
// 手臂：每個姿勢先算成幾何（袖口泡泡／手臂路徑／手），再依 armBlend 在
// `armFrom` 與 `armPose` 兩套幾何之間線性混合，最後才畫。armPose 是字串通道，
// 切換期間 lerpParams／blendArm 把 armBlend 從 0.5 附近連續帶過，手位不再單幀瞬移
// （對抗審查 rig-renderer-016）。armBlend=1 時的座標與原本逐 case 繪製完全相同。
// ---------------------------------------------------------------------------

interface ArmPuff {
  x: number;
  y: number;
  r: number;
}

/** 手臂路徑：二次曲線（直線＝控制點放中點）。 */
interface ArmPath {
  x0: number;
  y0: number;
  cx: number;
  cy: number;
  x1: number;
  y1: number;
  width: number;
}

interface ArmHand {
  x: number;
  y: number;
  r: number;
}

interface ArmSide {
  puff: ArmPuff;
  path: ArmPath | null;
  hand: ArmHand | null;
}

/** 兩側（-1 左、+1 右）的手臂幾何。 */
type ArmGeometry = [ArmSide, ArmSide];

const line = (x0: number, y0: number, x1: number, y1: number, width: number): ArmPath => ({
  x0,
  y0,
  cx: (x0 + x1) / 2,
  cy: (y0 + y1) / 2,
  x1,
  y1,
  width,
});

/** 單一姿勢的手臂幾何（座標與原本逐 case 的繪製一致）。 */
function armGeometry(pose: RigArmPose, p: RigParams, L: Layout): ArmGeometry {
  const ph = clamp01(p.armPhase);
  const shY = L.shoulderY + 2.5;
  const sides: ArmSide[] = [];
  for (const side of [-1, 1] as const) {
    switch (pose) {
      case "raise": {
        // 雙手上舉（ask/歡呼/伸展前段）。
        const lift = 10 + ph * 10;
        sides.push({
          puff: { x: 64 + side * 13, y: shY - 1, r: 5.6 },
          path: line(64 + side * 14, shY - 2, 64 + side * (16 + ph * 3), shY - lift, 4.6),
          hand: { x: 64 + side * (16 + ph * 3), y: shY - lift - 1.5, r: 3.1 },
        });
        break;
      }
      case "stretch": {
        // 伸懶腰：雙臂高舉過頭、微外八。
        sides.push({
          puff: { x: 64 + side * 12, y: shY - 2, r: 5.4 },
          path: line(64 + side * 12.5, shY - 3, 64 + side * 9, shY - 24 - ph * 4, 4.6),
          hand: { x: 64 + side * 8.5, y: shY - 26 - ph * 4, r: 3 },
        });
        break;
      }
      case "reach":
      case "block": {
        // 單手前伸/擋：另一手自然下垂。
        const front = p.armSide >= 0 ? 1 : -1;
        if (side !== front) {
          sides.push({
            puff: { x: 64 + side * 13, y: shY + 1, r: 5.4 },
            path: null,
            hand: { x: 64 + side * 14.5, y: shY + 9.5, r: 3.1 },
          });
        } else {
          const reach = 8 + ph * (pose === "block" ? 14 : 10);
          const lift = pose === "block" ? 2 : 5 - ph * 3;
          sides.push({
            puff: { x: 64 + side * 13, y: shY, r: 5.6 },
            path: line(64 + side * 14, shY + 1, 64 + side * (13 + reach), shY + lift, 4.6),
            hand: { x: 64 + side * (14.5 + reach), y: shY + lift, r: 3.4 },
          });
        }
        break;
      }
      case "pocket": {
        // 手插圍裙口袋：只見袖子。
        sides.push({
          puff: { x: 64 + side * 12.5, y: shY + 1, r: 5.4 },
          path: line(64 + side * 13, shY + 3, 64 + side * 10.5, L.waistY + 8, 4.4),
          hand: null,
        });
        break;
      }
      case "hug": {
        // 環抱身前（抱尾巴/物件）。
        sides.push({
          puff: { x: 64 + side * 12.5, y: shY + 1.5, r: 5.4 },
          path: {
            x0: 64 + side * 13,
            y0: shY + 3.5,
            cx: 64 + side * 12,
            cy: L.waistY + 5,
            x1: 64 + side * 3.5,
            y1: L.waistY + 6.5,
            width: 4.4,
          },
          hand: { x: 64 + side * 3.2, y: L.waistY + 6.5, r: 2.9 },
        });
        break;
      }
      case "down": {
        sides.push({
          puff: { x: 64 + side * 13, y: shY + 1, r: 5.5 },
          path: line(64 + side * 13.5, shY + 3, 64 + side * 14.5, shY + 12, 4.5),
          hand: { x: 64 + side * 14.5, y: shY + 13.5, r: 3.1 },
        });
        break;
      }
      case "front":
      default: {
        // 女僕待機：雙手交疊身前（兩隻手在身體中線交疊）。
        sides.push({
          puff: { x: 64 + side * 12.5, y: shY + 1, r: 5.5 },
          path: {
            x0: 64 + side * 13,
            y0: shY + 3,
            cx: 64 + side * 10,
            cy: L.waistY - 1,
            x1: 64 + side * 2.6,
            y1: L.waistY + 1.5,
            width: 4.5,
          },
          hand: side < 0 ? { x: 62.4, y: L.waistY + 2, r: 2.9 } : { x: 65.6, y: L.waistY + 2.4, r: 2.9 },
        });
        break;
      }
    }
  }
  return [sides[0], sides[1]];
}

const mixN = (a: number, b: number, k: number) => a + (b - a) * k;

function mixPath(a: ArmPath | null, b: ArmPath | null, k: number): ArmPath | null {
  if (!a && !b) return null;
  if (!a) return { ...b!, width: b!.width * k };
  if (!b) return { ...a, width: a.width * (1 - k) };
  return {
    x0: mixN(a.x0, b.x0, k),
    y0: mixN(a.y0, b.y0, k),
    cx: mixN(a.cx, b.cx, k),
    cy: mixN(a.cy, b.cy, k),
    x1: mixN(a.x1, b.x1, k),
    y1: mixN(a.y1, b.y1, k),
    width: mixN(a.width, b.width, k),
  };
}

function mixHand(a: ArmHand | null, b: ArmHand | null, k: number): ArmHand | null {
  if (!a && !b) return null;
  // 一邊沒有手（pocket）：從對方的位置淡入／淡出（半徑歸零），不憑空冒出。
  const aa = a ?? { x: b!.x, y: b!.y, r: 0 };
  const bb = b ?? { x: a!.x, y: a!.y, r: 0 };
  return { x: mixN(aa.x, bb.x, k), y: mixN(aa.y, bb.y, k), r: mixN(aa.r, bb.r, k) };
}

/** `k`＝目標姿勢（b）的權重。 */
function mixArmGeometry(a: ArmGeometry, b: ArmGeometry, k: number): ArmGeometry {
  const mixSide = (sa: ArmSide, sb: ArmSide): ArmSide => ({
    puff: { x: mixN(sa.puff.x, sb.puff.x, k), y: mixN(sa.puff.y, sb.puff.y, k), r: mixN(sa.puff.r, sb.puff.r, k) },
    path: mixPath(sa.path, sb.path, k),
    hand: mixHand(sa.hand, sb.hand, k),
  });
  return [mixSide(a[0], b[0]), mixSide(a[1], b[1])];
}

/** 此刻實際要畫的手臂幾何（含 armFrom↔armPose 的混合）。 */
function resolvedArmGeometry(p: RigParams, L: Layout): ArmGeometry {
  const current = armGeometry(p.armPose, p, L);
  const blend = clamp01(p.armBlend);
  if (blend >= 0.999 || p.armFrom === p.armPose) return current;
  return mixArmGeometry(armGeometry(p.armFrom, p, L), current, blend);
}

/**
 * 測試／量測用：此刻兩隻手的中心（邏輯座標；沒有手的姿勢回半徑 0 的點）。
 * 連續兩幀的手位位移就是「手臂有沒有瞬移」的直接量測。
 */
export function armHandPoints(p: RigParams, pal: RigPalette): { x: number; y: number; r: number }[] {
  const L = layoutFor(p, pal);
  const g = resolvedArmGeometry(p, L);
  return g.map((side) => side.hand ?? { x: side.puff.x, y: side.puff.y, r: 0 });
}

function drawArms(ctx: Ctx, p: RigParams, pal: RigPalette, L: Layout, c: Cols) {
  const glove = c.cream;
  const puff = (x: number, y: number, r: number) => {
    ellipse(ctx, x, y, r, r * 0.92, 0);
    fillStroke(ctx, c.cream, c.creamEdge, 0.9);
  };
  const hand = (x: number, y: number, r: number) => {
    if (r <= 0.05) return;
    ellipse(ctx, x, y, r, r, 0);
    fillStroke(ctx, glove, c.creamEdge, 0.8);
    // 肉球細節（低飽和粉紫）——手心小點。
    ctx.fillStyle = pal.pinkLilac;
    ctx.globalAlpha = 0.5;
    ellipse(ctx, x, y + 0.8, r * 0.36, r * 0.3, 0);
    ctx.fill();
    ctx.globalAlpha = 1;
  };
  const geometry = resolvedArmGeometry(p, L);
  // 先畫袖口與手臂，再畫手（交疊身前時兩隻手永遠在手臂之上；與原本繪製順序一致）。
  for (const side of geometry) {
    puff(side.puff.x, side.puff.y, side.puff.r);
    const path = side.path;
    if (path && path.width > 0.05) {
      ctx.beginPath();
      ctx.moveTo(path.x0, path.y0);
      ctx.quadraticCurveTo(path.cx, path.cy, path.x1, path.y1);
      ctx.strokeStyle = c.cream;
      ctx.lineWidth = path.width;
      ctx.lineCap = "round";
      ctx.stroke();
    }
  }
  for (const side of geometry) {
    if (side.hand) hand(side.hand.x, side.hand.y, side.hand.r);
  }
}

/** 胸前蝴蝶結＋呼吸發光核心。 */
function drawBowCore(
  ctx: Ctx,
  p: RigParams,
  pal: RigPalette,
  cx: number,
  cy: number,
  c: Cols
) {
  const pulse = 0.85 + Math.sin(clamp01(p.corePulse) * TAU) * 0.15;
  const glow = clamp01(p.coreGlow) * pulse;
  // 蝴蝶結翼（低飽和粉紫，清楚可見）。
  for (const side of [-1, 1] as const) {
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.quadraticCurveTo(cx + side * 8.5, cy - 5.5, cx + side * 9.5, cy - 0.5);
    ctx.quadraticCurveTo(cx + side * 8.5, cy + 4.5, cx, cy + 0.8);
    ctx.closePath();
    fillStroke(ctx, mixColor(pal.pinkLilac, c.dress, 0.28), c.dressEdge, 0.9);
  }
  // 緞帶尾。
  for (const side of [-1, 1] as const) {
    ctx.beginPath();
    ctx.moveTo(cx + side * 1.2, cy + 1.5);
    ctx.quadraticCurveTo(cx + side * 4.5, cy + 6.5, cx + side * 2.8, cy + 9);
    ctx.quadraticCurveTo(cx + side * 1.2, cy + 7, cx, cy + 3);
    ctx.closePath();
    fillStroke(ctx, mixColor(pal.pinkLilac, c.dress, 0.4), c.dressEdge, 0.7);
  }
  // 中心結晶核心（Runtime/AI 狀態）——熄滅時仍是可辨識的藍灰結晶。
  const core = mixColor("#3d4c58", pal.coreTeal, glow);
  ctx.save();
  if (glow > 0.4) {
    ctx.shadowColor = pal.coreTeal;
    ctx.shadowBlur = 5 * glow;
  }
  ctx.beginPath();
  const r = 3.4;
  for (let i = 0; i < 6; i++) {
    const a = (i / 6) * TAU - Math.PI / 2;
    const x = cx + r * Math.cos(a);
    const y = cy + r * Math.sin(a);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.closePath();
  fillStroke(ctx, core, mixColor("#1c232e", pal.coreTeal, glow * 0.5), 1.1);
  ctx.restore();
}

// ---------------------------------------------------------------------------
// 趴姿身體
// ---------------------------------------------------------------------------

function drawLieBody(ctx: Ctx, p: RigParams, pal: RigPalette, L: Layout, c: Cols) {
  // 身體橫向：裙成蓬鬆丘、靴子在後。座標以趴姿錨點（LIE_ANCHOR_X＝L.hx）為準；
  // 過場中的水平差由 drawRig 的 poseShiftX 帶過，頭與軀幹不會脫節。
  // 靴子（後方露出小小圓頭）。
  for (const [bx, by] of [
    [98, L.hemY - 2],
    [104, L.hemY + 1],
  ] as const) {
    ellipse(ctx, bx, by, 5.2, 3.6, 0);
    fillStroke(ctx, c.boot, c.dressEdge, 0.8);
  }
  // 裙丘（主體）。
  ctx.beginPath();
  ctx.moveTo(58, L.hemY + 4);
  ctx.bezierCurveTo(66, L.waistY - 10, 92, L.waistY - 8, 100, L.hemY - 1);
  ctx.quadraticCurveTo(88 , L.hemY + 6, 58, L.hemY + 4);
  ctx.closePath();
  fillStroke(ctx, c.dress, c.dressEdge, 1);
  // 圍裙蓋在裙丘前緣。
  ctx.beginPath();
  ctx.moveTo(60, L.hemY + 3.4);
  ctx.bezierCurveTo(68, L.waistY - 5, 84, L.waistY - 4, 90, L.hemY + 1);
  ctx.quadraticCurveTo(78, L.hemY + 5.5, 60, L.hemY + 3.4);
  ctx.closePath();
  fillStroke(ctx, c.cream, c.creamEdge, 0.8);
  // 前爪（手套）交疊在下巴前。
  for (const [hx2, hy2] of [
    [40, L.hemY + 1.5],
    [46, L.hemY + 2.8],
  ] as const) {
    ellipse(ctx, hx2, hy2, 3.4, 2.7, 0);
    fillStroke(ctx, c.cream, c.creamEdge, 0.8);
  }
  // 泡泡袖。
  ellipse(ctx, 52, L.hemY - 3, 5, 4.4, 0);
  fillStroke(ctx, c.cream, c.creamEdge, 0.9);
  // 核心在側身可見。
  drawBowCore(ctx, p, pal, 58, L.waistY + 1, c);
}

// ---------------------------------------------------------------------------
// 尾巴
// ---------------------------------------------------------------------------

function drawTail(
  ctx: Ctx,
  p: RigParams,
  pal: RigPalette,
  L: Layout,
  hair: string,
  hairEdge: string,
  dim: number
) {
  const wrap = clamp01(p.tailWrap);
  const curl = clamp01(p.tailCurl);
  const angle = clamp(p.tailAngle, -10, 70);
  const sway = clamp(p.tailSway, -1, 1) * 6;
  const tip = mixColor("#4d4560", pal.toolViolet, clamp01(p.tailTip) * (1 - dim * 0.5));

  ctx.save();
  ctx.strokeStyle = hair;
  ctx.lineWidth = 5.6;
  ctx.lineCap = "round";
  let tipX: number;
  let tipY: number;
  if (wrap > 0.5) {
    // 繞到身前腳邊（細一點，貼著裙襬）。
    const baseY = L.lie ? L.hemY + 2 : L.hemY + 6;
    ctx.lineWidth = 4.4;
    ctx.beginPath();
    ctx.moveTo(78, baseY - 4);
    ctx.bezierCurveTo(90, baseY, 84, baseY + 7, 60, baseY + 5.5);
    ctx.quadraticCurveTo(50, baseY + 4.5, 47, baseY + 1);
    ctx.stroke();
    tipX = 47;
    tipY = baseY + 1;
  } else if (L.lie) {
    const ex = 106 + sway - curl * 8;
    const ey = 96 - angle * 0.6 - curl * 5;
    ctx.beginPath();
    ctx.moveTo(96, L.hemY - 4);
    ctx.bezierCurveTo(106, L.hemY - 8 - angle * 0.3, 112 + sway, 104 - angle * 0.5, ex, ey);
    ctx.stroke();
    tipX = ex;
    tipY = ey;
  } else {
    // 站/坐：從裙後右側伸出，長而柔軟。
    const baseX = 78;
    const baseY = L.hemY - 6;
    const ex = 92 + angle * 0.25 + sway - curl * 9;
    const ey = baseY - angle * 0.75 - curl * 4 + Math.abs(sway) * 0.2;
    ctx.beginPath();
    ctx.moveTo(baseX, baseY);
    ctx.bezierCurveTo(
      baseX + 12,
      baseY + 4 - angle * 0.15,
      ex + 6 + curl * 6,
      ey + 12,
      ex,
      ey
    );
    ctx.stroke();
    tipX = ex;
    tipY = ey;
  }
  // 邊線。
  ctx.strokeStyle = hairEdge;
  ctx.lineWidth = 0.9;
  // 尾尖光。
  ctx.beginPath();
  ctx.arc(tipX, tipY, 4, 0, TAU);
  ctx.fillStyle = tip;
  ctx.fill();
  ctx.stroke();
  ctx.restore();
}

// ---------------------------------------------------------------------------
// 頭（臉、髮、耳、頭飾）
// ---------------------------------------------------------------------------

function drawHead(
  ctx: Ctx,
  p: RigParams,
  pal: RigPalette,
  L: Layout,
  c: { hair: string; hairEdge: string; skin: string; cream: string; creamEdge: string; dim: number }
) {
  const turn = p.headTurn * 5;
  const nod = p.headNod * 3;
  ctx.save();
  ctx.translate(L.hx, L.hy);
  ctx.rotate((p.headTilt * Math.PI) / 180);
  ctx.translate(-L.hx, -L.hy);
  ctx.translate(0, nod);

  const hx = L.hx;
  const hy = L.hy;
  const hrx = L.hrx;
  const hry = L.hry;

  // ---- 貓耳（畫在頭髮之下基部、之上尖端 → 先畫，頭蓋住基部） ----
  drawEars(ctx, p, pal, L, c, turn);

  // ---- 後髮（bob 輪廓比頭大一圈） ----
  ellipse(ctx, hx + turn * 0.3, hy + 1.5, hrx + 2.4, hry + 2.8, 0);
  fillStroke(ctx, c.hair, c.hairEdge, 1);
  // 後髮下緣內收（脖子兩側髮尾微尖）。
  for (const side of [-1, 1] as const) {
    ctx.beginPath();
    const bx = hx + side * (hrx - 2) + turn * 0.3;
    ctx.moveTo(bx, hy + hry - 4);
    ctx.quadraticCurveTo(bx + side * 3, hy + hry + 3.5 + p.hairSway * side * 1.2, bx - side * 2.5, hy + hry + 1.5);
    ctx.closePath();
    ctx.fillStyle = c.hair;
    ctx.fill();
  }

  // ---- 臉 ----
  ellipse(ctx, hx + turn * 0.15, hy + 2.2, hrx - 3.2, hry - 3.4, 0);
  fillStroke(ctx, c.skin, pal.skinEdge, 0.8);

  const faceX = hx + turn;
  const eyeY = hy + 3.2;

  // 腮紅。
  if (p.blush > 0.03) {
    ctx.save();
    ctx.globalAlpha = clamp01(p.blush) * 0.55;
    ctx.fillStyle = pal.pinkLilac;
    ellipse(ctx, faceX - 12.5, eyeY + 7.2, 3.8, 2, 0);
    ctx.fill();
    ellipse(ctx, faceX + 12.5, eyeY + 7.2, 3.8, 2, 0);
    ctx.fill();
    ctx.restore();
  }

  // 嘴（+小虎牙）。
  drawMouth(ctx, p, pal, faceX, eyeY + 9.2);

  // 眼睛＋眉。
  for (const side of [-1, 1] as const) {
    drawEye(ctx, p, pal, faceX + side * 8.6, eyeY, side, c.skin);
  }

  // 冷汗。
  if (p.sweat > 0.05) {
    ctx.save();
    ctx.globalAlpha = clamp01(p.sweat);
    ctx.fillStyle = "#9fd0f0";
    ctx.beginPath();
    const sx2 = faceX + 15;
    const sy2 = eyeY - 6 + clamp01(p.sweat) * 3;
    ctx.moveTo(sx2, sy2 - 3);
    ctx.quadraticCurveTo(sx2 + 2.6, sy2 + 1.5, sx2, sy2 + 3.2);
    ctx.quadraticCurveTo(sx2 - 2.6, sy2 + 1.5, sx2, sy2 - 3);
    ctx.fill();
    ctx.restore();
  }

  // ---- 瀏海（弧形波浪，蓋額頭）＋不對稱髮束 ----
  ctx.beginPath();
  ctx.moveTo(hx - hrx - 0.5 + turn * 0.4, hy - 1);
  ctx.quadraticCurveTo(hx - hrx + 2, hy - hry - 1, hx - 6 + turn * 0.5, hy - hry - 2.2);
  ctx.quadraticCurveTo(hx + hrx - 4, hy - hry - 1.5, hx + hrx + 0.5 + turn * 0.4, hy - 1);
  // 波浪內緣（3 束瀏海）。
  const fringeY = hy - 2.5;
  ctx.quadraticCurveTo(hx + hrx - 3.5, fringeY + 4.5, hx + 7.5 + turn, fringeY + 1.2);
  ctx.quadraticCurveTo(hx + 4 + turn, fringeY + 5.8, hx + 0.5 + turn, fringeY + 1.6);
  ctx.quadraticCurveTo(hx - 3.5 + turn, fringeY + 6.2, hx - 7 + turn, fringeY + 1.4);
  ctx.quadraticCurveTo(hx - hrx + 4, fringeY + 5, hx - hrx - 0.5 + turn * 0.4, hy - 1);
  ctx.closePath();
  fillStroke(ctx, c.hair, c.hairEdge, 0.9);
  // 髮光。
  ctx.save();
  ctx.strokeStyle = pal.hairShine;
  ctx.globalAlpha = 0.8 - c.dim * 0.4;
  ctx.lineWidth = 1.4;
  ctx.beginPath();
  ctx.moveTo(hx - 8, hy - hry + 3.2);
  ctx.quadraticCurveTo(hx, hy - hry + 1.4, hx + 8, hy - hry + 3.6);
  ctx.stroke();
  ctx.restore();

  // 不對稱長髮束（右側，垂到下巴側；隨 hairSway 擺）。
  const sway = p.hairSway * 2.4;
  ctx.beginPath();
  const lockX = hx + hrx - 1.5 + turn * 0.4;
  ctx.moveTo(lockX - 2.5, hy - 2);
  ctx.quadraticCurveTo(lockX + 3, hy + 6, lockX + 1.5 + sway, hy + hry + 5.5);
  ctx.quadraticCurveTo(lockX - 2 + sway * 0.6, hy + hry + 1, lockX - 4.5, hy + 4);
  ctx.closePath();
  fillStroke(ctx, c.hair, c.hairEdge, 0.8);
  // 左側短髮束（不對稱感）。
  ctx.beginPath();
  const lockL = hx - hrx + 1.5 + turn * 0.4;
  ctx.moveTo(lockL + 2.2, hy - 1);
  ctx.quadraticCurveTo(lockL - 2.5, hy + 4.5, lockL - 0.5 - sway * 0.5, hy + hry - 2);
  ctx.quadraticCurveTo(lockL + 2.4, hy + hry - 5, lockL + 4.2, hy + 3);
  ctx.closePath();
  fillStroke(ctx, c.hair, c.hairEdge, 0.8);
  // 呆毛（ahoge）：頭頂正中偏左的小彈簧捲，隨 sway 微彈。
  ctx.beginPath();
  ctx.moveTo(hx - 1.5, hy - hry - 2.4);
  ctx.quadraticCurveTo(hx - 0.5 + sway, hy - hry - 9.5, hx + 4.5 + sway * 1.4, hy - hry - 8);
  ctx.quadraticCurveTo(hx + 2 + sway, hy - hry - 5, hx + 0.8, hy - hry - 1.6);
  ctx.closePath();
  fillStroke(ctx, c.hair, c.hairEdge, 0.8);

  // ---- 頭飾（分體小頭飾，中央讓位給貓耳） ----
  drawHeadpiece(ctx, p, pal, L, c, turn);

  ctx.restore();
}

/** 貓耳：立於頭頂偏外側，耳內有能力訊號光。 */
function drawEars(
  ctx: Ctx,
  p: RigParams,
  pal: RigPalette,
  L: Layout,
  c: { hair: string; hairEdge: string },
  turn: number
) {
  const perk = clamp01(p.earPerk + pal.perkBias);
  const earS = pal.earSize;
  const out = (1 - perk) * 16; // 放鬆外倒角
  const lift = perk * 3.5;
  for (const side of [-1, 1] as const) {
    const tiltDeg = side === -1 ? p.earLTilt : p.earRTilt;
    const glowT = side === -1 ? clamp01(p.earL) : clamp01(p.earR);
    const glowCol = side === -1 ? pal.coolBlue : pal.warmOrange;
    const bx = L.hx + side * 12 * earS + turn * 0.5;
    const by = L.hy - L.hry + 0.5;
    ctx.save();
    ctx.translate(bx, by);
    ctx.rotate(((side * (7 + out) + tiltDeg) * Math.PI) / 180);
    // 外耳（小巧的貓耳三角，圓滑邊）。
    ctx.beginPath();
    ctx.moveTo(-6 * earS, 3.4);
    ctx.quadraticCurveTo(-3.4 * earS, -6.5 * earS - lift, side * 1.2, -11 * earS - lift);
    ctx.quadraticCurveTo(3.6 * earS + side * 0.6, -5 * earS - lift * 0.6, 6 * earS, 3.4);
    ctx.closePath();
    fillStroke(ctx, c.hair, c.hairEdge, 1);
    // 內耳（能力訊號光；無訊號時是可見的淡紫底）。
    const inner = mixColor("#584a5e", glowCol, glowT);
    ctx.save();
    if (glowT > 0.35) {
      ctx.shadowColor = glowCol;
      ctx.shadowBlur = 4.5 * glowT;
    }
    ctx.beginPath();
    ctx.moveTo(-3.2 * earS, 2.2);
    ctx.quadraticCurveTo(-1.8 * earS, -4.6 * earS - lift * 0.7, side * 0.8, -7.6 * earS - lift * 0.8);
    ctx.quadraticCurveTo(2 * earS, -3.5 * earS - lift * 0.5, 3.2 * earS, 2.2);
    ctx.closePath();
    ctx.fillStyle = inner;
    ctx.fill();
    ctx.restore();
    ctx.restore();
  }
}

/** 分體女僕頭飾：兩片白色荷葉邊，中央讓位給貓耳；可歪斜、可發光。 */
function drawHeadpiece(
  ctx: Ctx,
  p: RigParams,
  pal: RigPalette,
  L: Layout,
  c: { cream: string; creamEdge: string },
  turn: number
) {
  const glow = clamp01(p.headpieceGlow);
  ctx.save();
  ctx.translate(L.hx, L.hy - L.hry + 2);
  ctx.rotate((p.headpieceTilt * Math.PI) / 180);
  for (const side of [-1, 1] as const) {
    // 分體頭飾：貼在瀏海坡面、貓耳內側，中央留給呆毛。
    const bx = side * 6.6 + turn * 0.3;
    const by = side === 1 ? 0.6 : 1.0;
    ctx.save();
    ctx.translate(bx, by);
    ctx.scale(0.82, 0.82);
    ctx.rotate(side * 0.42);
    // 荷葉邊三瓣。
    ctx.beginPath();
    ctx.moveTo(-5, 1.8);
    for (const [qx, qy, ex, ey] of [
      [-4.4, -3.4, -1.8, -2.6],
      [0, -4.6, 1.8, -2.6],
      [4.4, -3.4, 5, 1.8],
    ] as const) {
      ctx.quadraticCurveTo(qx, qy, ex, ey);
    }
    ctx.quadraticCurveTo(0, 3.4, -5, 1.8);
    ctx.closePath();
    fillStroke(ctx, c.cream, c.creamEdge, 0.8);
    // 連線光（網路/Agent 狀態）。
    if (glow > 0.05) {
      ctx.globalAlpha = 0.4 + glow * 0.5;
      ctx.strokeStyle = pal.coolBlue;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(-3.6, 1);
      ctx.quadraticCurveTo(0, -1.4, 3.6, 1);
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
    ctx.restore();
  }
  ctx.restore();
}

/** 眼睛：大而機靈、眼尾微揚、紫灰虹膜。 */
function drawEye(
  ctx: Ctx,
  p: RigParams,
  pal: RigPalette,
  cx: number,
  cy: number,
  side: -1 | 1,
  skin: string
) {
  const es = pal.eyeScale;
  const lid = clamp(p.eyeLid + pal.lidBias, 0, 0.85);
  const open = clamp(p.eyeOpen, 0, 1) * (1 - lid * 0.72);
  const rx = 5.6 * es;
  const ry = 7 * es * Math.max(open, 0.06);
  const rot = (side * -7 * Math.PI) / 180; // 外眼角上揚

  ctx.save();
  ctx.translate(cx, cy);
  ctx.rotate(rot);

  if (open <= 0.14) {
    // 閉眼：上揚弧線＋睫毛尖。
    ctx.strokeStyle = pal.pupil;
    ctx.lineWidth = 1.6;
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.moveTo(-rx, 0.5);
    ctx.quadraticCurveTo(0, 0.5 + 2.6 * (p.eyeOpen < 0.5 ? 1 : -1), rx, 0);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(side * rx, 0);
    ctx.lineTo(side * (rx + 1.8), -1.2);
    ctx.stroke();
    ctx.restore();
    return;
  }

  // 眼白。
  ellipse(ctx, 0, 0, rx, ry, 0);
  ctx.fillStyle = pal.eyeWhite;
  ctx.fill();
  // 虹膜（紫灰、上深下淺）＋瞳孔＋雙高光。
  const px = clamp(p.pupilX, -3, 3) * 0.9;
  const py = clamp(p.pupilY, -3, 3) * 0.8;
  const ps = clamp(p.pupilScale, 0.7, 1.4);
  const irx = 3.5 * es * ps;
  const iry = Math.min(4.9 * es * ps, ry * 0.94);
  ellipse(ctx, px, py, irx, iry, 0);
  ctx.fillStyle = pal.iris;
  ctx.fill();
  ellipse(ctx, px, py + iry * 0.34, irx * 0.84, iry * 0.6, 0);
  ctx.fillStyle = pal.irisDeep;
  ctx.fill();
  ellipse(ctx, px, py + iry * 0.05, irx * 0.5, iry * 0.52, 0);
  ctx.fillStyle = pal.pupil;
  ctx.fill();
  ctx.fillStyle = "#ffffff";
  ctx.globalAlpha = 0.95;
  ellipse(ctx, px - irx * 0.35, py - iry * 0.42, 1.15 * es, 1.15 * es, 0);
  ctx.fill();
  ctx.globalAlpha = 0.65;
  ellipse(ctx, px + irx * 0.42, py + iry * 0.28, 0.6 * es, 0.6 * es, 0);
  ctx.fill();
  ctx.globalAlpha = 1;

  // 上眼皮（半瞇）。
  if (lid > 0.04) {
    ctx.fillStyle = skin;
    ctx.beginPath();
    ctx.rect(-rx - 1, -ry - 1.5, rx * 2 + 2, ry * 2 * lid * 0.92);
    ctx.fill();
    ctx.strokeStyle = pal.pupil;
    ctx.lineWidth = 1.1;
    ctx.beginPath();
    ctx.moveTo(-rx + 0.4, -ry - 1.5 + ry * 2 * lid * 0.92);
    ctx.lineTo(rx - 0.4, -ry - 1.5 + ry * 2 * lid * 0.92);
    ctx.stroke();
  }
  // 上睫毛線（大眼輪廓）＋外角小睫毛。
  ctx.strokeStyle = pal.pupil;
  ctx.lineWidth = 1.5;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.ellipse(0, 0, rx, ry, 0, Math.PI * 1.15, Math.PI * 1.85);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(side * (rx * 0.92), -ry * 0.55);
  ctx.lineTo(side * (rx + 1.9), -ry * 0.75);
  ctx.stroke();
  ctx.restore();

  // 眉（不隨眼旋轉）。
  const bv = side === -1 ? p.browL : p.browR;
  if (Math.abs(bv) > 0.03) {
    const lift = -bv * 3.4;
    const innerDrop = bv < 0 ? -bv * 2.6 : 0;
    ctx.strokeStyle = mixColor(pal.hair, pal.pupil, 0.4);
    ctx.lineWidth = 1.6;
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.moveTo(cx - side * 5.4, cy - 8.6 + lift + innerDrop);
    ctx.quadraticCurveTo(cx, cy - 10 + lift, cx + side * 4.8, cy - 8.4 + lift);
    ctx.stroke();
  }
}

/** 嘴＋小虎牙。 */
function drawMouth(ctx: Ctx, p: RigParams, pal: RigPalette, mx: number, my: number) {
  ctx.strokeStyle = pal.pupil;
  ctx.lineCap = "round";
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  switch (p.mouth) {
    case "smile":
      ctx.moveTo(mx - 4.6, my - 0.6);
      ctx.quadraticCurveTo(mx, my + 3, mx + 4.6, my - 0.6);
      break;
    case "smirk":
      ctx.moveTo(mx - 4, my + 0.4);
      ctx.quadraticCurveTo(mx + 1, my + 2.4, mx + 5, my - 1.6);
      break;
    case "cat":
      ctx.moveTo(mx - 4, my);
      ctx.quadraticCurveTo(mx - 2, my + 2.2, mx, my);
      ctx.quadraticCurveTo(mx + 2, my + 2.2, mx + 4, my);
      break;
    case "open":
      ctx.stroke();
      ellipse(ctx, mx, my + 0.8, 2.6, 2.2, 0);
      ctx.fillStyle = pal.pupil;
      ctx.fill();
      break;
    case "flat":
      ctx.moveTo(mx - 3.4, my + 0.4);
      ctx.lineTo(mx + 3.4, my + 0.4);
      break;
    case "pout":
      ctx.moveTo(mx - 3.4, my + 1.2);
      ctx.quadraticCurveTo(mx, my - 1, mx + 3.4, my + 1.2);
      break;
    case "none":
      break;
    case "soft":
    default:
      ctx.moveTo(mx - 3, my);
      ctx.quadraticCurveTo(mx, my + 1.8, mx + 3, my);
      break;
  }
  if (p.mouth !== "open" && p.mouth !== "none") ctx.stroke();
  // 小虎牙（得意時）：嘴角右上的小三角。
  if (p.fang > 0.25 && p.mouth !== "none") {
    ctx.fillStyle = "#ffffff";
    ctx.strokeStyle = pal.pupil;
    ctx.lineWidth = 0.6;
    ctx.beginPath();
    const fx = mx + 2.6;
    const fy = my + (p.mouth === "smirk" ? 0.4 : 0.8);
    ctx.moveTo(fx - 1.2, fy);
    ctx.lineTo(fx + 1.2, fy);
    ctx.lineTo(fx, fy + 2.3 * clamp01(p.fang));
    ctx.closePath();
    ctx.fill();
    ctx.stroke();
  }
}

// ---------------------------------------------------------------------------
// 盾/浮標/粒子
// ---------------------------------------------------------------------------

function drawShield(ctx: Ctx, L: Layout, opacity: number) {
  ctx.save();
  ctx.globalAlpha = opacity;
  ctx.translate(88, L.lie ? 100 : 92);
  ctx.beginPath();
  ctx.moveTo(0, -9);
  ctx.lineTo(8, -5);
  ctx.lineTo(8, 3);
  ctx.bezierCurveTo(8, 8, 4, 11, 0, 12);
  ctx.bezierCurveTo(-4, 11, -8, 8, -8, 3);
  ctx.lineTo(-8, -5);
  ctx.closePath();
  ctx.fillStyle = "#39435a";
  ctx.fill();
  ctx.strokeStyle = "#8fb6ff";
  ctx.lineWidth = 1.6;
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(-3, 0);
  ctx.lineTo(-1, 2.5);
  ctx.lineTo(4, -3);
  ctx.stroke();
  ctx.restore();
}

function drawOverlay(ctx: Ctx, p: RigParams, pal: RigPalette) {
  const ph = clamp01(p.overlayPhase);
  ctx.save();
  switch (p.overlay) {
    case "question": {
      ctx.translate(100, 26 - ph * 3);
      ctx.beginPath();
      ctx.arc(0, 0, 9, 0, TAU);
      ctx.fillStyle = "#ffcf5c";
      ctx.fill();
      ctx.fillStyle = "#4a3300";
      ctx.font = "bold 12.5px Arial, sans-serif";
      ctx.textAlign = "center";
      ctx.fillText("?", 0, 4.4);
      break;
    }
    case "cloud": {
      ctx.translate(101, 25 - ph * 2);
      ctx.fillStyle = "#cfe3ff";
      for (const [ex, ey, rx2, ry2] of [
        [0, 0, 10.5, 6.6],
        [-6.6, 2, 5.6, 4.6],
        [6.6, 2, 5.6, 4.6],
      ] as const) {
        ellipse(ctx, ex, ey, rx2, ry2, 0);
        ctx.fill();
      }
      ctx.fillStyle = pal.coolBlue;
      ctx.beginPath();
      ctx.arc(-4 + ph * 8, 0, 1.3, 0, TAU);
      ctx.fill();
      break;
    }
    case "stop": {
      ctx.translate(64, 13);
      ctx.beginPath();
      const r = 11.5;
      for (let i = 0; i < 8; i++) {
        const a = (i / 8) * TAU + Math.PI / 8;
        const x = r * Math.cos(a);
        const y = r * Math.sin(a);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.closePath();
      ctx.fillStyle = "#e5484d";
      ctx.fill();
      ctx.fillStyle = "#ffffff";
      ctx.fillRect(-6.2, -1.5, 12.4, 3);
      break;
    }
    case "zzz": {
      ctx.fillStyle = "#9db2c8";
      ctx.font = "bold 11px Arial, sans-serif";
      ctx.globalAlpha = 0.9 - ph * 0.3;
      ctx.fillText("z", 96, 34 - ph * 4);
      ctx.font = "bold 8px Arial, sans-serif";
      ctx.globalAlpha = 0.7 - ph * 0.3;
      ctx.fillText("z", 104, 26 - ph * 6);
      break;
    }
    case "check": {
      ctx.translate(100, 26);
      ctx.beginPath();
      ctx.arc(0, 0, 9, 0, TAU);
      ctx.fillStyle = "#46a758";
      ctx.fill();
      ctx.strokeStyle = "#ffffff";
      ctx.lineWidth = 2.3;
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      ctx.beginPath();
      ctx.moveTo(-4.2, 0);
      ctx.lineTo(-1.4, 3.3);
      ctx.lineTo(4.7, -3.3);
      ctx.stroke();
      break;
    }
    case "cross": {
      ctx.translate(100, 26);
      ctx.beginPath();
      ctx.arc(0, 0, 9, 0, TAU);
      ctx.fillStyle = "#e5484d";
      ctx.fill();
      ctx.strokeStyle = "#ffffff";
      ctx.lineWidth = 2.3;
      ctx.lineCap = "round";
      ctx.beginPath();
      ctx.moveTo(-3.6, -3.6);
      ctx.lineTo(3.6, 3.6);
      ctx.moveTo(3.6, -3.6);
      ctx.lineTo(-3.6, 3.6);
      ctx.stroke();
      break;
    }
    case "drop": {
      ctx.translate(101, 26 + ph * 2);
      ctx.fillStyle = "#7fb8f0";
      ctx.globalAlpha = 0.9;
      ctx.beginPath();
      ctx.moveTo(0, -6);
      ctx.bezierCurveTo(4, 0, 5, 3, 0, 6);
      ctx.bezierCurveTo(-5, 3, -4, 0, 0, -6);
      ctx.fill();
      break;
    }
    case "spark": {
      ctx.translate(100, 25);
      ctx.globalAlpha = 0.85 - ph * 0.4;
      ctx.fillStyle = pal.coreTeal;
      star(ctx, 0, 0, 8, 2);
      ctx.fill();
      break;
    }
    case "heart": {
      ctx.translate(100, 25 - ph * 4);
      ctx.globalAlpha = 0.95 - ph * 0.45;
      ctx.fillStyle = pal.pinkLilac;
      heart(ctx, 0, 0, 6.5);
      ctx.fill();
      break;
    }
    case "dots": {
      ctx.translate(98, 26);
      ctx.fillStyle = "#cfd6e4";
      for (let i = 0; i < 3; i++) {
        const on = ph * 3 >= i;
        ctx.globalAlpha = on ? 0.95 : 0.35;
        ctx.beginPath();
        ctx.arc(-7 + i * 7, 0, 2.2, 0, TAU);
        ctx.fill();
      }
      break;
    }
    case "none":
    default:
      break;
  }
  ctx.restore();
}

function drawParticles(ctx: Ctx, p: RigParams, pal: RigPalette, L: Layout) {
  const ph = clamp01(p.particlePhase);
  if (p.particles === "none") return;
  ctx.save();
  switch (p.particles) {
    case "dust": {
      // 腳邊小灰塵弧（落地/急停）。
      ctx.strokeStyle = "#c9c2b8";
      ctx.lineWidth = 1.4;
      ctx.globalAlpha = Math.max(0, 0.8 - ph);
      for (const side of [-1, 1] as const) {
        ctx.beginPath();
        ctx.arc(64 + side * (16 + ph * 10), L.groundY - 4, 3 + ph * 3, Math.PI * 1.1, Math.PI * 1.9);
        ctx.stroke();
      }
      break;
    }
    case "sparkle": {
      ctx.fillStyle = pal.coreTeal;
      for (const [sx, sy, r, off] of [
        [40, 40, 2.6, 0],
        [92, 34, 2, 0.33],
        [86, 62, 1.6, 0.66],
      ] as const) {
        const a = Math.max(0, Math.sin((ph + off) * Math.PI * 2));
        ctx.globalAlpha = a * 0.9;
        star(ctx, sx, sy, r * (0.7 + a * 0.5), 2);
        ctx.fill();
      }
      break;
    }
    case "zzz": {
      ctx.fillStyle = "#9db2c8";
      ctx.font = "bold 10px Arial, sans-serif";
      ctx.globalAlpha = 0.85 - ph * 0.4;
      ctx.fillText("z", 92 + ph * 3, 40 - ph * 8);
      ctx.font = "bold 7px Arial, sans-serif";
      ctx.globalAlpha = 0.6 - ph * 0.3;
      ctx.fillText("z", 99 + ph * 2, 32 - ph * 10);
      break;
    }
    case "heart": {
      ctx.fillStyle = pal.pinkLilac;
      ctx.globalAlpha = Math.max(0, 0.9 - ph * 0.7);
      heart(ctx, 96, 40 - ph * 10, 4.5);
      ctx.fill();
      break;
    }
  }
  ctx.restore();
}

function star(ctx: Ctx, cx: number, cy: number, r: number, ratio: number) {
  ctx.beginPath();
  for (let i = 0; i < 8; i++) {
    const rr = i % 2 === 0 ? r : r / ratio / 2;
    const a = (i / 8) * TAU - Math.PI / 2;
    const x = cx + rr * Math.cos(a);
    const y = cy + rr * Math.sin(a);
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.closePath();
}

function heart(ctx: Ctx, cx: number, cy: number, s: number) {
  ctx.beginPath();
  ctx.moveTo(cx, cy + s * 0.8);
  ctx.bezierCurveTo(cx - s * 1.4, cy - s * 0.3, cx - s * 0.6, cy - s * 1.1, cx, cy - s * 0.35);
  ctx.bezierCurveTo(cx + s * 0.6, cy - s * 1.1, cx + s * 1.4, cy - s * 0.3, cx, cy + s * 0.8);
  ctx.closePath();
}
