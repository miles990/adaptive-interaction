// 開發預覽入口（僅供 scripts/shu/preview-rig.mjs 打包使用，不進 App bundle）。
// 在頁面上畫出代表性參數組合的網格，供人工目視驗收角色設計。

import { drawRig } from "./draw";
import { clampParams, DEFAULT_PARAMS, RIG_PALETTES, RigParams } from "./params";
import { EXPRESSIONS, OFFICIAL_36 } from "./expressions";
import { drawExpressionPreview } from "./renderer";
import { StageRenderer } from "./stage";

const CASES: { label: string; params: Partial<RigParams>; palette?: string }[] = [
  { label: "站立待機", params: {} },
  {
    label: "微笑+虎牙+腮紅（得意）",
    params: {
      mouth: "smirk",
      fang: 1,
      blush: 0.7,
      eyeLid: 0.35,
      headNod: -0.5,
      tailAngle: 55,
      earPerk: 0.8,
      browL: 0.4,
      browR: 0.4,
    },
  },
  {
    label: "好奇（歪頭+瞳孔放大）",
    params: {
      headTilt: 14,
      pupilScale: 1.35,
      pupilX: 1.5,
      earPerk: 0.95,
      earLTilt: -6,
      mouth: "cat",
      tailAngle: 45,
      tailCurl: 0.7,
      armPose: "front",
    },
  },
  {
    label: "打瞌睡（坐）",
    params: {
      pose: "sit",
      eyeOpen: 0.1,
      eyeLid: 0.6,
      headTilt: -8,
      headNod: 0.8,
      earPerk: 0.1,
      mouth: "soft",
      overlay: "zzz",
      overlayPhase: 0.4,
      tailWrap: 1,
      armPose: "front",
    },
  },
  {
    label: "坐著晃腳",
    params: {
      pose: "sit",
      legPhase: 0.8,
      mouth: "smile",
      earPerk: 0.5,
      tailAngle: 35,
      tailSway: 0.6,
      armPose: "down",
    },
  },
  { label: "趴下", params: { pose: "lie", eyeLid: 0.3, mouth: "soft", tailAngle: 40 } },
  {
    label: "伸懶腰",
    params: {
      armPose: "stretch",
      armPhase: 1,
      squash: -0.35,
      eyeOpen: 0.15,
      mouth: "open",
      earPerk: 0.3,
      tailAngle: 50,
      headNod: -1,
    },
  },
  {
    label: "伸手擋游標",
    params: {
      armPose: "block",
      armPhase: 1,
      armSide: 1,
      mouth: "pout",
      browL: -0.5,
      browR: 0.6,
      headTurn: 0.5,
      earRTilt: 8,
    },
  },
  {
    label: "請求確認（ask）",
    params: {
      armPose: "raise",
      armPhase: 0.8,
      mouth: "open",
      browL: 0.6,
      browR: 0.6,
      overlay: "question",
      overlayPhase: 0.5,
      earPerk: 0.9,
      headpieceGlow: 0.8,
    },
  },
  {
    label: "工作中（核心亮）",
    params: {
      coreGlow: 1,
      corePulse: 0.5,
      earR: 1,
      armPose: "pocket",
      mouth: "flat",
      pupilY: 1.5,
      eyeLid: 0.2,
      headpieceGlow: 1,
    },
  },
  {
    label: "結果未知（裙光紫）",
    params: {
      skirtGlow: 1,
      skirtTone: "violet",
      overlay: "question",
      mouth: "flat",
      browL: -0.4,
      browR: -0.4,
      earPerk: 0.3,
      earLTilt: -10,
      earRTilt: 10,
      sweat: 0.6,
      armPose: "front",
    },
  },
  {
    label: "緊急停止（dim+stop）",
    params: {
      dim: 1,
      overlay: "stop",
      mouth: "flat",
      eyeOpen: 0.9,
      earPerk: 0.15,
      tailAngle: 5,
      armPose: "down",
      coreGlow: 0.05,
    },
  },
  { label: "暮色配色", params: { mouth: "smile", blush: 0.4 }, palette: "maid-dusk" },
  {
    label: "櫻花配色",
    params: { mouth: "cat", blush: 0.6, earPerk: 0.9, tailAngle: 50 },
    palette: "maid-sakura",
  },
  {
    label: "被抓包（定格+冷汗）",
    params: {
      sweat: 1,
      eyeOpen: 1,
      pupilScale: 0.75,
      pupilX: -2.5,
      mouth: "flat",
      earPerk: 1,
      earLTilt: -14,
      earRTilt: 14,
      tailAngle: 62,
      hairSway: 0.8,
      armPose: "front",
    },
  },
  {
    label: "已驗證成功（綠勾）",
    params: {
      overlay: "check",
      mouth: "smile",
      fang: 0.8,
      blush: 0.5,
      earPerk: 0.85,
      tailAngle: 55,
      coreGlow: 0.9,
      particles: "sparkle",
      particlePhase: 0.3,
      armPose: "raise",
      armPhase: 0.5,
    },
  },
];

function cell(
  parent: HTMLElement,
  label: string,
  params: Partial<RigParams>,
  paletteName: string,
  dark: boolean
) {
  const wrap = document.createElement("div");
  wrap.style.cssText = `display:inline-block;margin:4px;text-align:center;background:${dark ? "#20242c" : "#f2f2f0"};padding:6px;border-radius:8px;`;
  const canvas = document.createElement("canvas");
  const scale = 2;
  canvas.width = 128 * scale;
  canvas.height = 128 * scale;
  canvas.style.width = "192px";
  canvas.style.height = "192px";
  const ctx = canvas.getContext("2d")!;
  ctx.scale(scale, scale);
  drawRig(ctx, clampParams({ ...DEFAULT_PARAMS, ...params }), RIG_PALETTES[paletteName]);
  const cap = document.createElement("div");
  cap.textContent = label;
  cap.style.cssText = `font:11px sans-serif;color:${dark ? "#cfd6e4" : "#333"};margin-top:2px;`;
  wrap.appendChild(canvas);
  wrap.appendChild(cap);
  parent.appendChild(wrap);
}

const root = document.getElementById("root")!;
for (const dark of [false, true]) {
  const section = document.createElement("div");
  section.style.cssText = `background:${dark ? "#14171d" : "#ffffff"};padding:8px;`;
  for (const c of CASES) {
    cell(section, c.label, c.params, c.palette ?? "maid-classic", dark);
  }
  root.appendChild(section);
}

// 36 正式表情（hold 姿勢）驗收網格。
const exprSection = document.createElement("div");
exprSection.style.cssText = "background:#ffffff;padding:8px;";
const title = document.createElement("h3");
title.textContent = "OFFICIAL 36 EXPRESSIONS (hold pose)";
title.style.cssText = "font:14px sans-serif;margin:4px;";
exprSection.appendChild(title);
for (const id of OFFICIAL_36) {
  const wrap = document.createElement("div");
  wrap.style.cssText =
    "display:inline-block;margin:3px;text-align:center;background:#f4f4f2;padding:5px;border-radius:8px;";
  const canvas = document.createElement("canvas");
  canvas.width = 288;
  canvas.height = 288;
  canvas.style.width = "144px";
  canvas.style.height = "144px";
  const ctx = canvas.getContext("2d")!;
  ctx.scale(2, 2);
  drawExpressionPreview(ctx, id, "maid-classic", 144);
  const cap = document.createElement("div");
  cap.textContent = `${EXPRESSIONS[id]?.label ?? id}`;
  cap.style.cssText = "font:10px sans-serif;color:#333;margin-top:2px;";
  wrap.appendChild(canvas);
  wrap.appendChild(cap);
  exprSection.appendChild(wrap);
}
root.appendChild(exprSection);

// 遊玩場快照：玩具＋使魔＋場景＋角色走動（模擬 6 秒後的畫面）。
const stageSection = document.createElement("div");
stageSection.style.cssText = "background:#20242c;padding:10px;";
const stageTitle = document.createElement("h3");
stageTitle.textContent = "PLAYFIELD STAGE (simulated 6s)";
stageTitle.style.cssText = "font:14px sans-serif;margin:4px;color:#dfe6f2;";
stageSection.appendChild(stageTitle);
for (const scene of ["desk", "night"] as const) {
  const canvas = document.createElement("canvas");
  canvas.style.width = "520px";
  canvas.style.height = "283px";
  canvas.style.display = "block";
  canvas.style.margin = "6px";
  canvas.style.background = scene === "night" ? "#141821" : "#f4f2ee";
  stageSection.appendChild(canvas);
  let simNow = 0;
  const stage = new StageRenderer(canvas, "maid-classic", 1.1, {
    now: () => simNow,
    rng: (() => {
      let t = 42 >>> 0;
      return () => {
        t += 0x6d2b79f5;
        let r = Math.imul(t ^ (t >>> 15), 1 | t);
        r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
        return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
      };
    })(),
    autoStart: false,
  });
  stage.setScene(scene);
  stage.setCharName("小樞");
  stage.setFamiliars([
    { id: "a", name: "小雪", palette: "maid-sakura" },
    { id: "b", name: "小炭", palette: "maid-dusk" },
  ]);
  stage.setMachineFlags({ ambient: true, frozen: false, quiet: false, playPerforming: false });
  stage.setAnimation("idle");
  stage.spawnToy("yarn");
  stage.spawnToy("plane");
  stage.spawnToy("wand");
  // 模擬 6 秒（16ms 步進）。
  for (let i = 0; i < 375; i++) {
    simNow += 16;
    stage.renderFrame(simNow);
  }
  const cap = document.createElement("div");
  cap.textContent = `scene=${scene}・rollCall: ${stage
    .rollCallNow(null)
    .map((r) => `${r.name}${r.activity}`)
    .join("、")}`;
  cap.style.cssText = "font:11px sans-serif;color:#cfd6e4;margin:0 6px 8px;";
  stageSection.appendChild(cap);
  stage.destroy();
}
root.appendChild(stageSection);
document.title = "rig-preview-ready";
