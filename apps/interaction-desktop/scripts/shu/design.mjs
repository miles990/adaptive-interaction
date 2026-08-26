// 小樞（Shū）— original parametric character rig.
//
// One SVG rig, three appearance variants (standard / lively / minimal) that
// share the same skeleton and animation parameter space. Every frame is a
// pure function of (params, variant): deterministic, reproducible, anchored.
//
// Body-language semantics (per product spec):
//   left ear  = perception (cool blue glows when observing)
//   right ear = actuation  (warm orange glows when acting)
//   tail tip  = tool operations (violet)
//   chest hex core = runtime (teal; spins when thinking)
//   mini shield = policy governor (appears when blocked)
// Visual semantics never replace the standard safety text in the UI.

export const VARIANTS = {
  standard: {
    body: "#2e3a4e",
    bodyEdge: "#232c3b",
    belly: "#3a4a63",
    earInner: "#1d2634",
    eyeWhite: "#e8f4ff",
    pupil: "#1b2430",
    iris: "#57e6c4",
    coolBlue: "#4aa3ff",
    warmOrange: "#ff9d4a",
    toolViolet: "#b48bff",
    coreTeal: "#57e6c4",
    blush: "none",
    eyeScale: 1.0,
    earSize: 1.0,
    outline: 0,
  },
  lively: {
    body: "#33415c",
    bodyEdge: "#26314a",
    belly: "#465a7d",
    earInner: "#202a3d",
    eyeWhite: "#f2fbff",
    pupil: "#141d29",
    iris: "#63f0d0",
    coolBlue: "#5cb2ff",
    warmOrange: "#ffb066",
    toolViolet: "#c49dff",
    coreTeal: "#63f0d0",
    blush: "#ff9d8a",
    eyeScale: 1.18,
    earSize: 1.12,
    outline: 0,
  },
  minimal: {
    body: "#3c4454",
    bodyEdge: "#3c4454",
    belly: "#3c4454",
    earInner: "#2b323f",
    eyeWhite: "#eef2f6",
    pupil: "#20262f",
    iris: "#8fd8c6",
    coolBlue: "#7fb8f0",
    warmOrange: "#f0b585",
    toolViolet: "#c0a8e8",
    coreTeal: "#8fd8c6",
    blush: "none",
    eyeScale: 0.92,
    earSize: 0.9,
    outline: 0,
  },
};

/** Default frame parameters: sitting at rest, eyes open, everything calm. */
export function defaults() {
  return {
    bodyBob: 0, // px vertical bob (negative = up)
    bodyLean: 0, // deg lean of whole figure
    headTilt: 0, // deg head tilt
    eyeOpen: 1, // 0 closed … 1 open
    pupilX: 0, // -3 … 3 px gaze offset
    pupilY: 0,
    browSad: 0, // 0..1 concerned brows
    mouth: "soft", // soft | smile | open | flat | none
    earL: 0, // 0..1 cool-blue glow (perception)
    earR: 0, // 0..1 warm-orange glow (actuation)
    earLTilt: 0, // deg
    earRTilt: 0,
    coreGlow: 0.35, // 0..1 core brightness
    coreSpin: 0, // deg hex rotation
    tailAngle: 18, // deg from resting droop
    tailTip: 0, // 0..1 violet tip glow
    tailWrap: 0, // 0..1 tail wraps around body (quiet)
    shield: 0, // 0..1 policy shield opacity
    overlay: "none", // none|question|cloud|stop|zzz|check|drop|spark
    overlayPhase: 0, // 0..1 for overlay micro-animation
    armRaise: 0, // 0..1 both arms raise (ask)
    dim: 0, // 0..1 whole-figure desaturation (offline/paused)
    squash: 0, // 0..1 clicked squash
  };
}

const clamp = (v, a, b) => Math.max(a, Math.min(b, v));

function mix(hex, target, t) {
  // Linear blend of two #rrggbb colors.
  const h = (x) => [parseInt(x.slice(1, 3), 16), parseInt(x.slice(3, 5), 16), parseInt(x.slice(5, 7), 16)];
  const [r1, g1, b1] = h(hex);
  const [r2, g2, b2] = h(target);
  const c = (a, b2_) => Math.round(a + (b2_ - a) * clamp(t, 0, 1));
  return `#${[c(r1, r2), c(g1, g2), c(b1, b2)].map((x) => x.toString(16).padStart(2, "0")).join("")}`;
}

/** Compose one 128×128 SVG frame. Anchor: feet at (64,120). */
export function svgFrame(p, variantName = "standard") {
  const v = VARIANTS[variantName];
  const P = { ...defaults(), ...p };
  const dim = clamp(P.dim, 0, 1);
  const body = mix(v.body, "#555b63", dim * 0.6);
  const belly = mix(v.belly, "#5e646c", dim * 0.6);
  const eyeOpen = clamp(P.eyeOpen, 0.02, 1);
  const squashY = 1 - P.squash * 0.08;
  const squashX = 1 + P.squash * 0.06;

  const earGlowL = mix(v.earInner, v.coolBlue, P.earL);
  const earGlowR = mix(v.earInner, v.warmOrange, P.earR);
  const core = mix("#28313f", v.coreTeal, clamp(P.coreGlow, 0, 1) * (1 - dim * 0.5));
  const tailTip = mix("#3a4353", v.toolViolet, P.tailTip);

  const es = v.eyeScale;
  const earS = v.earSize;

  // Tail: rest droop → raised curve; wrap pulls it around the front.
  const tailA = P.tailAngle;
  const wrap = clamp(P.tailWrap, 0, 1);
  const tailPath = wrap > 0.5
    ? // wrapped around the body front (quiet/sleep)
      `M 82 106 C 96 112, 92 122, 64 121 C 48 120.5, 40 118, 38 113`
    : `M 83 103 C ${94 + tailA * 0.35} ${100 - tailA * 0.8}, ${96 + tailA * 0.3} ${86 - tailA}, ${88 + tailA * 0.22} ${78 - tailA * 1.15}`;
  const tailTipPos = wrap > 0.5
    ? { x: 38, y: 113 }
    : { x: 88 + tailA * 0.22, y: 78 - tailA * 1.15 };

  // Mouth variants.
  const mouths = {
    soft: `<path d="M 59 74 Q 64 77 69 74" stroke="${v.pupil}" stroke-width="1.6" fill="none" stroke-linecap="round"/>`,
    smile: `<path d="M 57 73 Q 64 79 71 73" stroke="${v.pupil}" stroke-width="1.8" fill="none" stroke-linecap="round"/>`,
    open: `<ellipse cx="64" cy="75.5" rx="3.4" ry="2.6" fill="${v.pupil}"/>`,
    flat: `<path d="M 59 75 L 69 75" stroke="${v.pupil}" stroke-width="1.6" fill="none" stroke-linecap="round"/>`,
    none: "",
  };

  // Brows (concern) — drawn only when browSad > 0.
  const brows = P.browSad > 0.02
    ? `<path d="M 48 ${52 + P.browSad * 2} Q 54 ${50 + P.browSad * 4} 59 ${53 + P.browSad * 3}" stroke="${v.pupil}" stroke-width="1.6" fill="none" stroke-linecap="round"/>
       <path d="M 80 ${52 + P.browSad * 2} Q 74 ${50 + P.browSad * 4} 69 ${53 + P.browSad * 3}" stroke="${v.pupil}" stroke-width="1.6" fill="none" stroke-linecap="round"/>`
    : "";

  // Eyes with lids (eyeOpen scales the visible ellipse).
  const eye = (cx) => `
    <g>
      <ellipse cx="${cx}" cy="61" rx="${6.4 * es}" ry="${8.2 * es * eyeOpen}" fill="${v.eyeWhite}"/>
      ${eyeOpen > 0.25
        ? `<circle cx="${cx + clamp(P.pupilX, -3, 3)}" cy="${61 + clamp(P.pupilY, -3, 3)}" r="${3.1 * es}" fill="${v.pupil}"/>
           <circle cx="${cx + clamp(P.pupilX, -3, 3) + 1.1}" cy="${60 + clamp(P.pupilY, -3, 3) - 1.1}" r="${1.0 * es}" fill="${v.iris}" opacity="0.9"/>`
        : ""}
    </g>`;

  // Overlays (status glyphs floating near the head).
  const ph = P.overlayPhase;
  const overlays = {
    none: "",
    question: `<g transform="translate(96 ${34 - ph * 3})">
        <circle r="9.5" fill="#ffcf5c" opacity="0.95"/>
        <text x="0" y="4.6" text-anchor="middle" font-family="Arial, sans-serif" font-size="13" font-weight="bold" fill="#4a3300">?</text>
      </g>`,
    cloud: `<g transform="translate(97 ${32 - ph * 2})" opacity="0.95">
        <ellipse cx="0" cy="0" rx="11" ry="7" fill="#cfe3ff"/>
        <ellipse cx="-7" cy="2" rx="6" ry="5" fill="#cfe3ff"/>
        <ellipse cx="7" cy="2" rx="6" ry="5" fill="#cfe3ff"/>
        <circle cx="${-4 + ph * 8}" cy="0" r="1.4" fill="#4aa3ff"/>
      </g>`,
    stop: `<g transform="translate(64 30)">
        <polygon points="-8,-19 8,-19 19,-8 19,8 8,19 -8,19 -19,8 -19,-8" transform="scale(0.62)" fill="#e5484d"/>
        <rect x="-6.5" y="-1.6" width="13" height="3.2" rx="1.2" fill="#ffffff"/>
      </g>`,
    zzz: `<g fill="#9db2c8" font-family="Arial, sans-serif" font-weight="bold">
        <text x="92" y="${40 - ph * 4}" font-size="11" opacity="${0.9 - ph * 0.3}">z</text>
        <text x="100" y="${32 - ph * 6}" font-size="8" opacity="${0.7 - ph * 0.3}">z</text>
      </g>`,
    check: `<g transform="translate(96 34)">
        <circle r="9.5" fill="#46a758"/>
        <path d="M -4.5 0 L -1.5 3.5 L 5 -3.5" stroke="#ffffff" stroke-width="2.4" fill="none" stroke-linecap="round" stroke-linejoin="round"/>
      </g>`,
    drop: `<g transform="translate(97 ${34 + ph * 2})">
        <path d="M 0 -6 C 4 0, 5 3, 0 6 C -5 3, -4 0, 0 -6 Z" fill="#7fb8f0" opacity="0.9"/>
      </g>`,
    spark: `<g transform="translate(96 33)" opacity="${0.85 - ph * 0.4}">
        <path d="M 0 -8 L 2 -2 L 8 0 L 2 2 L 0 8 L -2 2 L -8 0 L -2 -2 Z" fill="${v.coreTeal}"/>
      </g>`,
  };

  // Policy shield (blocked): small hex-shield near the core.
  const shield = P.shield > 0.02
    ? `<g transform="translate(85 88)" opacity="${clamp(P.shield, 0, 1)}">
        <path d="M 0 -9 L 8 -5 L 8 3 C 8 8, 4 11, 0 12 C -4 11, -8 8, -8 3 L -8 -5 Z"
              fill="#39435a" stroke="#8fb6ff" stroke-width="1.6"/>
        <path d="M -3 0 L -1 2.5 L 4 -3" stroke="#8fb6ff" stroke-width="1.8" fill="none" stroke-linecap="round"/>
      </g>`
    : "";

  const blush = v.blush !== "none" && P.mouth === "smile"
    ? `<ellipse cx="47" cy="70" rx="4" ry="2.2" fill="${v.blush}" opacity="0.5"/>
       <ellipse cx="81" cy="70" rx="4" ry="2.2" fill="${v.blush}" opacity="0.5"/>`
    : "";

  // Hex core path (r=7) with spin.
  const hex = Array.from({ length: 6 }, (_, i) => {
    const a = ((60 * i - 90 + P.coreSpin) * Math.PI) / 180;
    return `${(7 * Math.cos(a)).toFixed(2)},${(7 * Math.sin(a)).toFixed(2)}`;
  }).join(" ");

  return `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <g transform="translate(64 120) rotate(${P.bodyLean}) scale(${squashX} ${squashY}) translate(-64 -120)">
    <g transform="translate(0 ${P.bodyBob})">
      <!-- tail (behind body) -->
      <path d="${tailPath}" stroke="${mix(v.bodyEdge, "#555b63", dim * 0.5)}" stroke-width="5.5" fill="none" stroke-linecap="round"/>
      <circle cx="${tailTipPos.x}" cy="${tailTipPos.y}" r="4.2" fill="${tailTip}"/>
      <!-- legs -->
      <ellipse cx="52" cy="117" rx="7" ry="5" fill="${body}"/>
      <ellipse cx="76" cy="117" rx="7" ry="5" fill="${body}"/>
      <!-- body -->
      <ellipse cx="64" cy="99" rx="23" ry="20" fill="${body}"/>
      <ellipse cx="64" cy="103" rx="15" ry="12.5" fill="${belly}"/>
      <!-- arms -->
      <ellipse cx="${42 - P.armRaise * 2}" cy="${99 - P.armRaise * 14}" rx="5.5" ry="7.5" fill="${body}" transform="rotate(${-P.armRaise * 40} 42 99)"/>
      <ellipse cx="${86 + P.armRaise * 2}" cy="${99 - P.armRaise * 14}" rx="5.5" ry="7.5" fill="${body}" transform="rotate(${P.armRaise * 40} 86 99)"/>
      <!-- chest core -->
      <polygon points="${hex}" transform="translate(64 100)" fill="${core}" stroke="${mix("#1c232e", v.coreTeal, P.coreGlow * 0.5)}" stroke-width="1.6"/>
      ${shield}
      <!-- head -->
      <g transform="rotate(${P.headTilt} 64 62)">
        <!-- ears -->
        <g transform="rotate(${-8 + P.earLTilt} 46 40)">
          <path d="M 38 46 L 46 ${22 + (1 - earS) * 8} L 56 42 Z" fill="${body}"/>
          <path d="M 41 44 L 46.2 ${26 + (1 - earS) * 6} L 53.5 41.5 Z" fill="${earGlowL}"/>
        </g>
        <g transform="rotate(${8 + P.earRTilt} 82 40)">
          <path d="M 72 42 L 82 ${22 + (1 - earS) * 8} L 90 46 Z" fill="${body}"/>
          <path d="M 74.5 41.5 L 81.8 ${26 + (1 - earS) * 6} L 87 44 Z" fill="${earGlowR}"/>
        </g>
        <!-- head shape -->
        <ellipse cx="64" cy="62" rx="30" ry="27" fill="${body}"/>
        <ellipse cx="64" cy="66" rx="24" ry="19" fill="${mix(belly, body, 0.45)}"/>
        ${brows}
        ${eye(53)}
        ${eye(75)}
        ${blush}
        ${mouths[P.mouth] ?? mouths.soft}
      </g>
    </g>
  </g>
  ${overlays[P.overlay] ?? ""}
</svg>`;
}
