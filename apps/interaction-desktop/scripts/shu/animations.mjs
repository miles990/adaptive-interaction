// Animation parameter timelines for the Shū rig.
// Each animation = array of param objects (one per frame) + fps + loop flag.
// Loops are authored so the last frame flows back into the first.

const sin = (i, n, amp) => Math.sin((i / n) * Math.PI * 2) * amp;

function frames(n, fn) {
  return Array.from({ length: n }, (_, i) => fn(i, n));
}

export const ANIMATIONS = {
  // -- ambient ---------------------------------------------------------------
  idle: {
    fps: 3,
    loop: true,
    frames: frames(6, (i, n) => ({
      bodyBob: sin(i, n, 1.2),
      tailAngle: 18 + sin(i, n, 4),
      coreGlow: 0.35 + sin(i, n, 0.06),
      earLTilt: i === 4 ? -4 : 0,
    })),
  },
  blink: {
    fps: 12,
    loop: false,
    frames: [{ eyeOpen: 1 }, { eyeOpen: 0.12 }, { eyeOpen: 0.55 }, { eyeOpen: 1 }],
  },
  move: {
    fps: 7,
    loop: true,
    frames: frames(4, (i, n) => ({
      bodyBob: -Math.abs(sin(i, n, 2.4)),
      bodyLean: sin(i, n, 2.5),
      tailAngle: 22 + sin(i, n, 6),
    })),
  },
  // -- perceive / think / act ------------------------------------------------
  notice: {
    fps: 9,
    loop: false,
    frames: [
      { earL: 0.4, headTilt: -2 },
      { earL: 0.8, headTilt: -4, pupilX: -2 },
      { earL: 1, headTilt: -5, pupilX: -2.6, earLTilt: -6 },
      { earL: 0.9, headTilt: -4, pupilX: -2.4, earLTilt: -4 },
    ],
  },
  thinking: {
    fps: 5,
    loop: true,
    frames: frames(6, (i, n) => ({
      pupilY: -2.4,
      pupilX: 1,
      coreGlow: 0.55 + sin(i, n, 0.15),
      coreSpin: (360 / n) * i,
      tailAngle: 14,
    })),
  },
  routing: {
    fps: 8,
    loop: true,
    frames: frames(6, (i, n) => ({
      coreSpin: (360 / n) * i,
      coreGlow: 0.7,
      earL: i % 2 === 0 ? 0.8 : 0.2,
      earR: i % 2 === 1 ? 0.8 : 0.2,
      tailTip: 0.5 + sin(i, n, 0.4),
      tailAngle: 24,
    })),
  },
  ask: {
    fps: 6,
    loop: false,
    frames: [
      { armRaise: 0.3, headTilt: 3, mouth: "open" },
      { armRaise: 0.7, headTilt: 5, mouth: "open", eyeOpen: 1 },
      { armRaise: 1, headTilt: 6, mouth: "open", bodyBob: -1.5 },
    ],
  },
  act: {
    fps: 8,
    loop: true,
    frames: frames(4, (i, n) => ({
      earR: 0.85 + sin(i, n, 0.15),
      tailTip: 0.9,
      tailAngle: 30 + sin(i, n, 5),
      mouth: "soft",
      coreGlow: 0.5,
    })),
  },
  waiting: {
    fps: 4,
    loop: true,
    frames: frames(6, (i, n) => ({
      overlay: "cloud",
      overlayPhase: i / n,
      pupilY: -2,
      pupilX: 2,
      coreGlow: 0.45 + sin(i, n, 0.08),
      tailAngle: 12,
    })),
  },
  // -- outcomes (honesty-critical) -------------------------------------------
  success: {
    fps: 6,
    loop: false,
    frames: [
      { mouth: "smile", bodyBob: 0 },
      { mouth: "smile", bodyBob: -2.5, tailAngle: 30 },
      { overlay: "check", mouth: "smile", bodyBob: -1.5, tailAngle: 34, headTilt: -3 },
      { overlay: "check", mouth: "smile", bodyBob: 0, tailAngle: 30 },
    ],
  },
  blocked: {
    fps: 6,
    loop: false,
    frames: [
      { shield: 0.35, mouth: "flat", tailAngle: 10 },
      { shield: 0.7, mouth: "flat", browSad: 0.3, tailAngle: 7, headTilt: 2 },
      { shield: 1, mouth: "flat", browSad: 0.55, tailAngle: 5, earL: 0, earR: 0 },
    ],
  },
  unknown: {
    fps: 4,
    loop: true,
    frames: frames(6, (i, n) => ({
      overlay: "question",
      overlayPhase: (sin(i, n, 0.5) + 0.5) / 1,
      browSad: 0.35,
      mouth: "flat",
      pupilX: sin(i, n, 2.6),
      headTilt: sin(i, n, 3),
      tailAngle: 12,
    })),
  },
  // -- low-activity ----------------------------------------------------------
  quiet: {
    fps: 2,
    loop: true,
    frames: frames(4, (i, n) => ({
      tailWrap: 1,
      eyeOpen: 0.1,
      overlay: "zzz",
      overlayPhase: i / n,
      coreGlow: 0.15,
      mouth: "none",
      bodyBob: sin(i, n, 0.8),
    })),
  },
  paused: {
    fps: 1,
    loop: true,
    frames: [
      { dim: 0.35, eyeOpen: 0.6, mouth: "flat", coreGlow: 0.2, tailAngle: 8 },
      { dim: 0.35, eyeOpen: 0.55, mouth: "flat", coreGlow: 0.16, tailAngle: 8 },
    ],
  },
  emergency: {
    // Fixed safe pose: no ordinary animation, no gaze tracking, core dark.
    fps: 1,
    loop: true,
    frames: [
      {
        overlay: "stop",
        eyeOpen: 0.6,
        mouth: "flat",
        earL: 0,
        earR: 0,
        coreGlow: 0.08,
        tailAngle: 2,
        dim: 0.45,
      },
      {
        overlay: "stop",
        eyeOpen: 0.55,
        mouth: "flat",
        earL: 0,
        earR: 0,
        coreGlow: 0.08,
        tailAngle: 2,
        dim: 0.45,
      },
    ],
  },
  offline: {
    fps: 1,
    loop: true,
    frames: [
      { dim: 0.65, eyeOpen: 0.05, mouth: "none", coreGlow: 0.02, tailAngle: 4 },
      { dim: 0.65, eyeOpen: 0.05, mouth: "none", coreGlow: 0.05, tailAngle: 4 },
    ],
  },
  // -- direct interaction ----------------------------------------------------
  clicked: {
    fps: 12,
    loop: false,
    frames: [
      { squash: 1, eyeOpen: 0.8 },
      { squash: 0.4, mouth: "smile" },
      { squash: 0, mouth: "smile", bodyBob: -1.5 },
      { squash: 0, mouth: "soft" },
    ],
  },
  dragged: {
    fps: 6,
    loop: true,
    frames: [
      { bodyLean: -4, eyeOpen: 1, mouth: "open", tailAngle: 34, earLTilt: -8, earRTilt: 8 },
      { bodyLean: 4, eyeOpen: 1, mouth: "open", tailAngle: 30, earLTilt: -6, earRTilt: 6 },
    ],
  },
};
