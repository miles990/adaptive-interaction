// 小樞 v2 動畫時間軸。
// 每個動畫 = 每幀參數陣列 + fps + loop。迴圈動畫的末幀流回首幀。
//
// 表演原則（spec §4.3／§5.2 反應鏈）：
//   - 察覺：耳朵先立（earPerk）→ 眼睛亮（eyeOpen/瞳孔）→ 頭稍後才轉（headTilt/Turn）。
//   - 工作：目光集中、動作乾淨、減少玩鬧。
//   - 完成：短促得意（smirk＋小點頭），不過度慶祝；綠勾只出現在 verified 幀（2-3）。
//   - 失敗：先愣住（瞪大、耳落）再認真檢查（前傾、視線向下）——與 blocked（政策盾）不同。
//   - 未知：歪頭＋耳不對稱，永不播成功。
//   - Emergency：固定安全姿態，停止一切俏皮表現。
//   - 慵懶只出現在無任務、無風險的待機（quiet/lie/tailhug/legswing）。

const sin = (i, n, amp) => Math.sin((i / n) * Math.PI * 2) * amp;

function frames(n, fn) {
  return Array.from({ length: n }, (_, i) => fn(i, n));
}

export const ANIMATIONS = {
  // ---------------- ambient ----------------
  idle: {
    fps: 3,
    loop: true,
    frames: frames(6, (i, n) => ({
      bodyBob: sin(i, n, 1.1),
      tailAngle: 20 + sin(i, n, 5),
      tailCurl: 0.3 + sin(i, n, 0.12),
      coreGlow: 0.35 + sin(i, n, 0.06),
      earPerk: 0.4 + sin(i, n, 0.05),
      earLTilt: i === 4 ? -5 : 0,
      mouth: "cat",
    })),
  },
  blink: {
    fps: 12,
    loop: false,
    frames: [
      { eyeOpen: 1, mouth: "cat" },
      { eyeOpen: 0.12, mouth: "cat" },
      { eyeOpen: 0.55, mouth: "cat" },
      { eyeOpen: 1, mouth: "cat" },
    ],
  },
  // 伸展（ambient／可點播）。
  stretch: {
    fps: 6,
    loop: false,
    frames: [
      { stretchArc: 0.3, eyeOpen: 0.6, mouth: "cat", tailAngle: 26 },
      { stretchArc: 0.85, eyeOpen: 0.25, mouth: "open", tailAngle: 34, bodyBob: -2 },
      { stretchArc: 1, eyeOpen: 0.15, mouth: "open", tailAngle: 38, bodyBob: -3, earPerk: 0.2 },
      { stretchArc: 0.5, eyeOpen: 0.7, mouth: "cat", tailAngle: 28 },
    ],
  },
  // 坐著晃腳（無任務時的放鬆）。
  legswing: {
    fps: 4,
    loop: true,
    frames: frames(6, (i, n) => ({
      legSwing: sin(i, n, 1),
      bodyBob: sin(i, n, 0.6),
      tailAngle: 22 + sin(i, n, 6),
      mouth: "cat",
      eyeLid: 0.15,
      earPerk: 0.35,
    })),
  },
  // 抱尾巴（慵懶待機）。
  tailhug: {
    fps: 3,
    loop: true,
    frames: frames(4, (i, n) => ({
      hugTail: 1,
      tailWrap: 0,
      eyeLid: 0.35 + sin(i, n, 0.08),
      bodyBob: sin(i, n, 0.8),
      earPerk: 0.25,
      mouth: "soft",
      headTilt: 3,
    })),
  },
  // 趴著半睜眼觀察（慵懶但仍在看）。
  lie: {
    fps: 3,
    loop: true,
    frames: frames(4, (i, n) => ({
      pose: "lie",
      eyeLid: 0.4,
      bodyBob: sin(i, n, 0.7),
      tailAngle: 14 + sin(i, n, 5),
      earPerk: 0.3 + (i === 2 ? 0.2 : 0),
      mouth: "soft",
    })),
  },

  // ---------------- perceive / think / act ----------------
  // 察覺：耳先立 → 眼亮 → 頭稍後轉（反應鏈的視覺註冊）。
  notice: {
    fps: 9,
    loop: false,
    frames: [
      { earPerk: 0.9, earL: 0.3, mouth: "cat" }, // 1. 耳朵先動
      { earPerk: 1, earL: 0.7, eyeOpen: 1, pupilX: -2.2, mouth: "cat" }, // 2. 眼睛亮起看過去
      { earPerk: 1, earL: 1, pupilX: -2.6, headTilt: -4, headTurn: -0.5, browR: 0.5 }, // 3. 頭才跟上
      { earPerk: 0.95, earL: 0.85, pupilX: -2.2, headTilt: -3, headTurn: -0.4, browR: 0.3 },
    ],
  },
  // 好奇「讓我看看」：前傾＋歪頭＋挑眉。
  curious: {
    fps: 7,
    loop: false,
    frames: [
      { earPerk: 0.9, eyeOpen: 1, mouth: "cat" },
      { earPerk: 1, headTilt: 6, browL: 0.6, pupilX: 1.8, mouth: "smirk", tailAngle: 30 },
      { earPerk: 1, headTilt: 9, browL: 0.8, pupilX: 2.4, bodyLean: 3, mouth: "smirk", tailAngle: 34, tailCurl: 0.6 },
      { earPerk: 1, headTilt: 8, browL: 0.7, pupilX: 2.2, bodyLean: 2.5, mouth: "smirk", tailAngle: 32 },
      { earPerk: 0.9, headTilt: 5, browL: 0.4, pupilX: 1.6, mouth: "cat", tailAngle: 26 },
    ],
  },
  // 聆聽：雙耳全立＋冷藍、視線專注、尾梢輕捲。
  listening: {
    fps: 6,
    loop: true,
    frames: frames(4, (i, n) => ({
      earPerk: 1,
      earL: 0.8 + sin(i, n, 0.2),
      eyeOpen: 1,
      mouth: "soft",
      tailAngle: 24,
      tailCurl: 0.5 + sin(i, n, 0.1),
      coreGlow: 0.45,
    })),
  },
  thinking: {
    fps: 5,
    loop: true,
    frames: frames(6, (i, n) => ({
      pupilY: -2.4,
      pupilX: 1.2,
      eyeLid: 0.1,
      coreGlow: 0.55 + sin(i, n, 0.15),
      coreSpin: (360 / n) * i,
      tailAngle: 16,
      earPerk: 0.6,
      mouth: "flat",
    })),
  },
  routing: {
    fps: 8,
    loop: true,
    frames: frames(4, (i, n) => ({
      pupilX: sin(i, n, 2.6),
      earPerk: 0.9,
      earL: i < 2 ? 0.8 : 0.2,
      earR: i < 2 ? 0.2 : 0.8,
      coreGlow: 0.5,
      mouth: "soft",
      tailAngle: 24 + sin(i, n, 3),
    })),
  },
  // 請求確認（requesting-consent）：舉手＋亮眼；不裝可憐。
  ask: {
    fps: 6,
    loop: true,
    frames: frames(3, (i) => ({
      armRaise: 0.85,
      earPerk: 1,
      eyeOpen: 1,
      mouth: i === 1 ? "open" : "smile",
      overlay: "question",
      overlayPhase: i / 3,
      tailAngle: 28,
    })),
  },
  // 工作中：目光集中、身體前傾、右耳（行動）暖光、尾巴俐落。
  act: {
    fps: 7,
    loop: true,
    frames: frames(4, (i, n) => ({
      earR: 0.8 + sin(i, n, 0.2),
      earPerk: 0.9,
      eyeLid: 0.12,
      pupilY: 1.2,
      bodyLean: 2,
      coreGlow: 0.6 + sin(i, n, 0.1),
      mouth: "flat",
      tailAngle: 26 + sin(i, n, 3),
      tailTip: 0.5,
    })),
  },
  waiting: {
    fps: 4,
    loop: true,
    frames: frames(4, (i, n) => ({
      overlay: "cloud",
      overlayPhase: i / 4,
      earPerk: 0.55,
      eyeLid: 0.1,
      mouth: "soft",
      tailAngle: 18 + sin(i, n, 3),
      bodyBob: sin(i, n, 0.6),
    })),
  },

  // ---------------- outcomes ----------------
  // 成功：幀 0-1＝短促得意的小點頭（completed 未驗證只播這兩幀）；
  // 幀 2-3 才有綠勾（verified 的誠實視覺）。此契約不可改。
  success: {
    fps: 6,
    loop: false,
    frames: [
      { headTilt: 0, bodyBob: -1.5, mouth: "smirk", earPerk: 0.95, tailAngle: 30, tailCurl: 0.6 },
      { headTilt: 2, bodyBob: 0.5, mouth: "smirk", earPerk: 0.9, tailAngle: 26 },
      { mouth: "smile", overlay: "check", earPerk: 0.95, tailAngle: 30, coreGlow: 0.6 },
      { mouth: "smile", overlay: "check", earPerk: 0.9, tailAngle: 28, coreGlow: 0.55 },
    ],
  },
  blocked: {
    fps: 4,
    loop: false,
    frames: [
      { shield: 0.8, mouth: "flat", earPerk: 0.3, earLTilt: -8, earRTilt: 8, tailAngle: 10 },
      { shield: 1, mouth: "flat", earPerk: 0.25, earLTilt: -10, earRTilt: 10, tailAngle: 8, browL: -0.4, browR: -0.4 },
      { shield: 1, mouth: "flat", earPerk: 0.3, earLTilt: -8, earRTilt: 8, tailAngle: 10, browL: -0.3, browR: -0.3 },
    ],
  },
  unknown: {
    fps: 5,
    loop: false,
    frames: [
      { headTilt: 5, earLTilt: -10, earRTilt: 4, overlay: "question", overlayPhase: 0, mouth: "soft" },
      { headTilt: 8, earLTilt: -14, earRTilt: 8, overlay: "question", overlayPhase: 0.4, mouth: "soft", browR: 0.5 },
      { headTilt: 7, earLTilt: -12, earRTilt: 6, overlay: "question", overlayPhase: 0.8, mouth: "soft", browR: 0.4 },
      { headTilt: 6, earLTilt: -11, earRTilt: 5, overlay: "question", overlayPhase: 0.5, mouth: "soft" },
    ],
  },
  // 失敗（專屬美術）：先愣住（瞪大、耳落）→ 認真檢查（前傾、視線向下）。
  failed: {
    fps: 4,
    loop: false,
    frames: [
      { eyeOpen: 1, earPerk: 0.15, earLTilt: -14, earRTilt: 14, mouth: "open", overlay: "cross", tailAngle: 6 },
      { eyeOpen: 1, earPerk: 0.15, earLTilt: -14, earRTilt: 14, mouth: "flat", overlay: "cross", tailAngle: 6 },
      { bodyLean: 3, pupilY: 2.4, eyeLid: 0.15, earPerk: 0.5, mouth: "flat", browL: -0.5, browR: -0.5, overlay: "cross", tailAngle: 12 },
      { bodyLean: 3.5, pupilY: 2.6, eyeLid: 0.15, earPerk: 0.55, mouth: "flat", browL: -0.5, browR: -0.5, overlay: "cross", tailAngle: 12, coreGlow: 0.5 },
    ],
  },

  // ---------------- low-activity ----------------
  quiet: {
    fps: 2,
    loop: true,
    frames: frames(4, (i, n) => ({
      tailWrap: 1,
      hugTail: 0,
      eyeLid: 0.55,
      eyeOpen: 0.7,
      earPerk: 0.15,
      overlay: "zzz",
      overlayPhase: i / 4,
      bodyBob: sin(i, n, 0.8),
      mouth: "soft",
    })),
  },
  paused: {
    fps: 2,
    loop: true,
    frames: [
      { dim: 0.25, mouth: "flat", earPerk: 0.3, tailAngle: 12, eyeLid: 0.2 },
      { dim: 0.25, mouth: "flat", earPerk: 0.3, tailAngle: 12, eyeLid: 0.2, bodyBob: 0.6 },
    ],
  },
  // 緊急停止：固定安全姿態——耳落、核心暗、停止標誌。無任何俏皮成分。
  emergency: {
    fps: 1,
    loop: true,
    frames: [
      {
        dim: 0.55,
        overlay: "stop",
        earPerk: 0.05,
        earL: 0,
        earR: 0,
        coreGlow: 0.08,
        mouth: "flat",
        eyeLid: 0.25,
        tailAngle: 4,
        tailCurl: 0,
      },
    ],
  },
  offline: {
    fps: 1,
    loop: true,
    frames: [
      { dim: 0.7, eyeOpen: 0.08, mouth: "none", earPerk: 0.1, coreGlow: 0.05, tailAngle: 4 },
    ],
  },

  // ---------------- direct interaction ----------------
  clicked: {
    fps: 10,
    loop: false,
    frames: [
      { squash: 0.9, eyeOpen: 0.5, mouth: "open", earPerk: 0.8 },
      { squash: 0.35, eyeOpen: 1, mouth: "cat", earPerk: 1 },
      { squash: 0, mouth: "cat", earPerk: 0.9, tailAngle: 28 },
    ],
  },
  dragged: {
    fps: 8,
    loop: true,
    frames: [
      { bodyLean: -4, eyeOpen: 1, mouth: "open", earPerk: 1, tailAngle: 34 },
      { bodyLean: 4, eyeOpen: 1, mouth: "open", earPerk: 1, tailAngle: 30 },
    ],
  },
};
