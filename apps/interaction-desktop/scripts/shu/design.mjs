// 小樞（Shū）v2 — 貓系數位小精靈，原創參數化 rig。
//
// 設計語言（spec §4）：
//   - 3.2 頭身 Q 版：小巧、柔軟但非嬰兒體態；圓潤主輪廓＋耳朵/髮束/尾巴的少量尖角。
//   - 眼睛明亮、眼尾微上揚、略帶狡黠；眉毛與眼皮能演「發現了」「真的假的」「讓我看看」。
//   - 貓耳是注意力與狀態顯示器（earPerk），不只是裝飾；
//     沿用既有語意色：左耳=感知(冷藍)、右耳=行動(暖橙)、尾尖=工具(紫)、
//     胸口六角核心=runtime(青)、小盾=policy governor。
//   - 尾巴有重量與慣性（tailAngle/tailCurl 由動畫時間軸給出延遲）。
//   - 視覺語意永不取代 UI 的標準安全文字。
//
// 每一幀都是 (params, variant) 的純函式：確定性、可重現、腳底錨定 (64,120)。

export const VARIANTS = {
  // 靈巧型（預設）：聰明與機靈最明顯 — 清晰的眼神、挺立的耳朵。
  agile: {
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
    lidBias: 0, // 常態眼皮下垂量（慵懶型 > 0）
    perkBias: 0.15, // 常態耳朵挺立度加成
  },
  // 慵懶型：半睜眼、耳朵放鬆、色調更柔。
  lazy: {
    body: "#37404f",
    bodyEdge: "#2b3340",
    belly: "#46536a",
    earInner: "#242c39",
    eyeWhite: "#e6eef6",
    pupil: "#202834",
    iris: "#8fd8c6",
    coolBlue: "#6fa9e8",
    warmOrange: "#eaa96e",
    toolViolet: "#b9a4e6",
    coreTeal: "#8fd8c6",
    blush: "none",
    eyeScale: 0.96,
    earSize: 0.94,
    lidBias: 0.3,
    perkBias: -0.15,
  },
  // 活潑型：眼睛與耳朵更大、腮紅、反應幅度大（不增加任何權限）。
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
    eyeScale: 1.15,
    earSize: 1.1,
    lidBias: 0,
    perkBias: 0.25,
  },
};

/** 預設參數：端坐、放鬆、微挺耳。 */
export function defaults() {
  return {
    pose: "sit", // sit | lie（趴）
    bodyBob: 0, // px 垂直起伏（負=上）
    bodyLean: 0, // deg 全身傾斜
    headTilt: 0, // deg 歪頭
    headTurn: 0, // -1..1 頭水平轉向（臉部特徵平移）
    eyeOpen: 1, // 0 閉 … 1 全開
    eyeLid: 0, // 0..1 上眼皮下垂（半睜=0.4；與 eyeOpen 疊加）
    pupilX: 0, // -3..3 px 視線
    pupilY: 0,
    browL: 0, // -1(憂)…0…1(挑眉) 左眉
    browR: 0, // 右眉（不對稱=「真的假的」）
    mouth: "soft", // soft | smile | smirk | cat | open | flat | none
    earPerk: 0.4, // 0(放鬆貼後)…1(全立) 注意力顯示器
    earL: 0, // 0..1 冷藍內耳光（感知）
    earR: 0, // 0..1 暖橙內耳光（行動）
    earLTilt: 0, // deg 個別耳偏（不對稱=困惑）
    earRTilt: 0,
    coreGlow: 0.35,
    coreSpin: 0,
    tailAngle: 20, // deg 尾巴抬起角（0=下垂）
    tailCurl: 0.3, // 0..1 尾梢捲曲
    tailWrap: 0, // 0..1 尾巴繞到身前（安靜/睡）
    tailTip: 0, // 0..1 尾尖紫光（工具）
    hugTail: 0, // 0..1 前爪抱尾巴（慵懶待機）
    legSwing: 0, // -1..1 坐姿晃腳相位
    stretchArc: 0, // 0..1 伸展（雙臂上舉＋拱背）
    armRaise: 0, // 0..1 雙臂舉起（ask）
    shield: 0,
    overlay: "none", // none|question|cloud|stop|zzz|check|cross|drop|spark
    overlayPhase: 0,
    dim: 0,
    squash: 0,
  };
}

const clamp = (v, a, b) => Math.max(a, Math.min(b, v));

function mix(hex, target, t) {
  const h = (x) => [
    parseInt(x.slice(1, 3), 16),
    parseInt(x.slice(3, 5), 16),
    parseInt(x.slice(5, 7), 16),
  ];
  const [r1, g1, b1] = h(hex);
  const [r2, g2, b2] = h(target);
  const c = (a, b2_) => Math.round(a + (b2_ - a) * clamp(t, 0, 1));
  return `#${[c(r1, r2), c(g1, g2), c(b1, b2)].map((x) => x.toString(16).padStart(2, "0")).join("")}`;
}

/**
 * 幾何錨點（給 Behavior Runtime 的程序化微動作：視線點、耳根）。
 * 只在 sit 姿勢有意義；回傳座標已含 bodyBob。
 */
export function anchorsFor(p) {
  const P = { ...defaults(), ...p };
  const bob = P.bodyBob;
  const turn = P.headTurn * 4;
  return {
    eyeL: [56.5 + turn, 41 + bob],
    eyeR: [71.5 + turn, 41 + bob],
    pupilR: 3.4,
    earL: [53.5, 27.5 + bob],
    earR: [74.5, 27.5 + bob],
  };
}

/** 組合一張 128×128 SVG。錨點：腳底 (64,120)。 */
export function svgFrame(p, variantName = "agile") {
  const v = VARIANTS[variantName];
  const P = { ...defaults(), ...p };
  const dim = clamp(P.dim, 0, 1);
  const body = mix(v.body, "#555b63", dim * 0.6);
  const edge = mix(v.bodyEdge, "#555b63", dim * 0.5);
  const belly = mix(v.belly, "#5e646c", dim * 0.6);
  const squashY = 1 - P.squash * 0.08;
  const squashX = 1 + P.squash * 0.06;
  const lie = P.pose === "lie";

  const perk = clamp(P.earPerk + v.perkBias, 0, 1);
  const lid = clamp(P.eyeLid + v.lidBias, 0, 0.85);
  const eyeOpen = clamp(P.eyeOpen, 0.02, 1) * (1 - lid * 0.75);
  const es = v.eyeScale;
  const earS = v.earSize;
  const turn = P.headTurn * 4; // 臉部特徵水平平移

  const earGlowL = mix(v.earInner, v.coolBlue, P.earL);
  const earGlowR = mix(v.earInner, v.warmOrange, P.earR);
  const core = mix("#28313f", v.coreTeal, clamp(P.coreGlow, 0, 1) * (1 - dim * 0.5));
  const tailTipCol = mix("#3a4353", v.toolViolet, P.tailTip);

  // ---------------- 頭（3 頭身級：頭高 ~32px，底緣與軀幹相接） ----------------
  const headCy = lie ? 78 : 40;
  const headCx = 64;
  const headRx = 18.5;
  const headRy = 16;

  // 貓耳：貼附頭頂邊緣的三角，perk 控制立起（放鬆時外倒）。
  const earOut = (1 - perk) * 16; // deg 外倒
  const earLift = perk * 3.5; // px 立起提升
  const ear = (side, tilt, glow) => {
    const s = side; // -1 左 / +1 右
    const bx = headCx + s * 10.5 * earS + turn * 0.5; // 耳根中心 x
    const by = headCy - headRy + 3.5; // 耳根埋進頭頂邊緣
    const rot = s * (8 + earOut) + tilt;
    return `<g transform="rotate(${rot} ${bx} ${by})">
      <path d="M ${bx - 7 * earS} ${by + 3} L ${bx - s * 1.5} ${by - 13.5 * earS - earLift} L ${bx + 7 * earS} ${by + 3} Z"
            fill="${body}" stroke="${edge}" stroke-width="1"/>
      <path d="M ${bx - 4 * earS} ${by + 2} L ${bx - s * 1} ${by - 9 * earS - earLift} L ${bx + 4 * earS} ${by + 2} Z"
            fill="${glow}"/>
    </g>`;
  };

  // 眼睛：眼尾微上揚（外眼角高 → 旋轉橢圓），豎瞳孔帶亮點；eyeLid 畫上眼皮。
  const eye = (cx, side) => {
    const rot = side * 8; // 外眼角上揚
    const rx = 4.8 * es;
    const ry = 6.4 * es * eyeOpen;
    const px = clamp(P.pupilX, -3, 3);
    const py = clamp(P.pupilY, -3, 3);
    const lidRect =
      lid > 0.03
        ? `<rect x="${cx - rx - 1}" y="${37 - ry - 1}" width="${rx * 2 + 2}" height="${ry * 2 * lid}" fill="${body}"/>`
        : "";
    return `<g transform="rotate(${rot} ${cx} 37) translate(0 ${headCy - 36})">
      <ellipse cx="${cx}" cy="37" rx="${rx}" ry="${ry}" fill="${v.eyeWhite}"/>
      ${
        eyeOpen > 0.22
          ? `<ellipse cx="${cx + px}" cy="${37 + py}" rx="${2.5 * es}" ry="${3.4 * es * Math.min(1, eyeOpen + 0.2)}" fill="${v.pupil}"/>
             <circle cx="${cx + px + 1.0}" cy="${36 + py - 1.2}" r="${1.0 * es}" fill="${v.iris}" opacity="0.95"/>
             <circle cx="${cx + px - 0.9}" cy="${37.8 + py}" r="${0.5 * es}" fill="${v.eyeWhite}" opacity="0.8"/>`
          : `<path d="M ${cx - rx} 37 Q ${cx} ${37 + 2.5} ${cx + rx} 37" stroke="${v.pupil}" stroke-width="1.5" fill="none" stroke-linecap="round"/>`
      }
      ${lidRect}
    </g>`;
  };

  // 眉毛：browL/browR -1..1（負=憂、正=挑）；不對稱演「真的假的」。
  const brow = (cx, side, val) => {
    if (Math.abs(val) < 0.03) return "";
    const lift = -val * 3.2; // 正值上挑
    const inner = val < 0 ? -val * 2.4 : 0; // 憂：內端下壓
    const x1 = cx - side * 5.2;
    const x2 = cx + side * 4.6;
    return `<path d="M ${x1} ${29.5 + lift + inner} Q ${cx} ${28.2 + lift} ${x2} ${29.8 + lift}"
      stroke="${v.pupil}" stroke-width="1.7" fill="none" stroke-linecap="round"
      transform="translate(${turn} ${headCy - 36})"/>`;
  };

  // 嘴形（smirk=狡黠單邊、cat=ω）。
  const my = headCy + 9;
  const mouths = {
    soft: `<path d="M 61 ${my} Q 64 ${my + 2} 67 ${my}" stroke="${v.pupil}" stroke-width="1.5" fill="none" stroke-linecap="round"/>`,
    smile: `<path d="M 59 ${my - 0.5} Q 64 ${my + 3.5} 69 ${my - 0.5}" stroke="${v.pupil}" stroke-width="1.7" fill="none" stroke-linecap="round"/>`,
    smirk: `<path d="M 60 ${my + 0.5} Q 65 ${my + 2.5} 69.5 ${my - 1.5}" stroke="${v.pupil}" stroke-width="1.7" fill="none" stroke-linecap="round"/>`,
    cat: `<path d="M 60 ${my} Q 62 ${my + 2.2} 64 ${my} Q 66 ${my + 2.2} 68 ${my}" stroke="${v.pupil}" stroke-width="1.5" fill="none" stroke-linecap="round"/>`,
    open: `<ellipse cx="64" cy="${my + 1}" rx="2.8" ry="2.2" fill="${v.pupil}"/>`,
    flat: `<path d="M 60.5 ${my + 0.5} L 67.5 ${my + 0.5}" stroke="${v.pupil}" stroke-width="1.5" fill="none" stroke-linecap="round"/>`,
    none: "",
  };
  const mouthSvg = (mouths[P.mouth] ?? mouths.soft).replace(
    "<path",
    `<path transform="translate(${turn} 0)"`
  );

  const blush =
    v.blush !== "none" && (P.mouth === "smile" || P.mouth === "cat")
      ? `<ellipse cx="${52 + turn}" cy="${headCy + 7}" rx="3.4" ry="1.9" fill="${v.blush}" opacity="0.5"/>
         <ellipse cx="${76 + turn}" cy="${headCy + 7}" rx="3.4" ry="1.9" fill="${v.blush}" opacity="0.5"/>`
      : "";

  // 頰毛（少量尖角）＋頂上髮束。
  const cheekFluff = `
    <path d="M ${headCx - 19} ${headCy + 4} l -4.5 1.8 l 4.2 2.2" fill="${body}"/>
    <path d="M ${headCx + 19} ${headCy + 4} l 4.5 1.8 l -4.2 2.2" fill="${body}"/>`;
  const tuft = `<path d="M ${headCx - 3 + turn * 0.5} ${headCy - 16.5} q 2.5 -4.5 6.5 -3.5 q -2.5 1.8 -2.8 4.2" fill="${body}"/>`;

  const headGroup = `
    <g transform="rotate(${P.headTilt} ${headCx} ${headCy})">
      ${ear(-1, P.earLTilt, earGlowL)}
      ${ear(1, P.earRTilt, earGlowR)}
      <ellipse cx="${headCx}" cy="${headCy}" rx="${headRx}" ry="${headRy}" fill="${body}" stroke="${edge}" stroke-width="1"/>
      ${tuft}
      ${cheekFluff}
      <ellipse cx="${headCx + turn * 0.6}" cy="${headCy + 5.5}" rx="14.5" ry="10.5" fill="${mix(belly, body, 0.45)}"/>
      ${brow(56.5 + turn, -1, P.browL)}
      ${brow(71.5 + turn, 1, P.browR)}
      ${eye(56.5 + turn, -1)}
      ${eye(71.5 + turn, 1)}
      ${blush}
      ${mouthSvg}
    </g>`;

  // ---------------- 身體 ----------------
  // 尾巴：三次曲線，tailAngle 抬起、tailCurl 捲梢、tailWrap 繞前。
  const tailA = P.tailAngle;
  const curl = clamp(P.tailCurl, 0, 1);
  const wrap = clamp(P.tailWrap, 0, 1);
  let tailPath, tailTipPos;
  if (wrap > 0.5) {
    tailPath = lie
      ? `M 86 112 C 100 114, 96 120, 66 119 C 50 118.5, 42 116, 40 112`
      : `M 78 108 C 94 112, 90 121, 62 120 C 46 119.5, 38 117, 36 112`;
    tailTipPos = lie ? { x: 40, y: 112 } : { x: 36, y: 112 };
  } else if (lie) {
    tailPath = `M 88 110 C ${98 + tailA * 0.2} ${106 - tailA * 0.5}, ${104 + tailA * 0.15} ${98 - tailA * 0.7}, ${100 + tailA * 0.1 - curl * 8} ${90 - tailA * 0.8 - curl * 4}`;
    tailTipPos = { x: 100 + tailA * 0.1 - curl * 8, y: 90 - tailA * 0.8 - curl * 4 };
  } else {
    // 坐姿：尾巴從臀後貼地掃出、末端依 curl 收小勾（貓的重量感，不誇張）。
    const ex = 88 + tailA * 0.2 - curl * 8;
    const ey = 94 - tailA * 0.85 - curl * 3;
    tailPath = `M 74 106 C ${86 + tailA * 0.2} ${106 - tailA * 0.35}, ${93 + tailA * 0.2} ${100 - tailA * 0.7}, ${ex} ${ey}`;
    tailTipPos = { x: ex, y: ey };
  }

  // 坐姿身體：西洋梨形軀幹＋前爪＋盤起的後腿；趴姿：低扁身體。
  const swing = clamp(P.legSwing, -1, 1);
  const stretch = clamp(P.stretchArc, 0, 1);
  const armLift = P.armRaise * 16 + stretch * 22;
  const armRot = P.armRaise * 42 + stretch * 70;

  let bodySvg;
  if (lie) {
    bodySvg = `
      <!-- 趴姿 -->
      <ellipse cx="64" cy="106" rx="26" ry="13" fill="${body}" stroke="${edge}" stroke-width="1"/>
      <ellipse cx="64" cy="109" rx="18" ry="8" fill="${belly}"/>
      <ellipse cx="46" cy="114" rx="6.5" ry="3.6" fill="${body}"/>
      <ellipse cx="82" cy="114" rx="6.5" ry="3.6" fill="${body}"/>
      <polygon points="${hexPts(P.coreSpin)}" transform="translate(64 104) scale(0.8)" fill="${core}" stroke="${mix("#1c232e", v.coreTeal, P.coreGlow * 0.5)}" stroke-width="1.4"/>`;
  } else {
    const hipY = 104;
    // 晃腳：兩腳前伸、交替擺動（腳掌橢圓上下小幅相位差）。
    const footL = `<ellipse cx="55" cy="${114 - swing * 2.5}" rx="5.8" ry="4" fill="${body}" stroke="${edge}" stroke-width="0.8"/>`;
    const footR = `<ellipse cx="73" cy="${114 + swing * 2.5}" rx="5.8" ry="4" fill="${body}" stroke="${edge}" stroke-width="0.8"/>`;
    // 抱尾巴：前爪收到胸前（畫在尾巴之上）。
    const hug = clamp(P.hugTail, 0, 1);
    const pawL = `<ellipse cx="${53 - armLift * 0.1 + hug * 4}" cy="${90 - armLift + hug * 2}" rx="4.4" ry="6" fill="${body}" stroke="${edge}" stroke-width="0.8" transform="rotate(${-armRot - hug * 30} 53 90)"/>`;
    const pawR = `<ellipse cx="${75 + armLift * 0.1 - hug * 4}" cy="${90 - armLift + hug * 2}" rx="4.4" ry="6" fill="${body}" stroke="${edge}" stroke-width="0.8" transform="rotate(${armRot + hug * 30} 75 90)"/>`;
    const hugTailFront =
      hug > 0.4
        ? `<path d="M 74 104 C 84 108, 78 114, 62 112 C 54 111, 52 108, 54 105" stroke="${edge}" stroke-width="6" fill="none" stroke-linecap="round"/>
           <circle cx="54" cy="105" r="3.8" fill="${tailTipCol}"/>`
        : "";
    bodySvg = `
      <!-- 坐姿：梨形軀幹，上緣與頭底相接（拱背由 stretchArc 拉高） -->
      <ellipse cx="64" cy="${hipY}" rx="18" ry="13" fill="${body}" stroke="${edge}" stroke-width="1"/>
      <ellipse cx="64" cy="${74 - stretch * 5}" rx="14.5" ry="${21 + stretch * 4}" fill="${body}" stroke="${edge}" stroke-width="1"/>
      <ellipse cx="64" cy="${90 - stretch * 3}" rx="10.5" ry="13.5" fill="${belly}"/>
      ${footL}${footR}
      <polygon points="${hexPts(P.coreSpin)}" transform="translate(64 ${88 - stretch * 3})" fill="${core}" stroke="${mix("#1c232e", v.coreTeal, P.coreGlow * 0.5)}" stroke-width="1.6"/>
      ${hugTailFront}
      ${pawL}${pawR}`;
  }

  // 小盾（policy governor）。
  const shield =
    P.shield > 0.02
      ? `<g transform="translate(84 ${lie ? 96 : 90})" opacity="${clamp(P.shield, 0, 1)}">
        <path d="M 0 -9 L 8 -5 L 8 3 C 8 8, 4 11, 0 12 C -4 11, -8 8, -8 3 L -8 -5 Z"
              fill="#39435a" stroke="#8fb6ff" stroke-width="1.6"/>
        <path d="M -3 0 L -1 2.5 L 4 -3" stroke="#8fb6ff" stroke-width="1.8" fill="none" stroke-linecap="round"/>
      </g>`
      : "";

  // 狀態浮標。
  const ph = P.overlayPhase;
  const overlays = {
    none: "",
    question: `<g transform="translate(98 ${28 - ph * 3})">
        <circle r="9" fill="#ffcf5c" opacity="0.95"/>
        <text x="0" y="4.4" text-anchor="middle" font-family="Arial, sans-serif" font-size="12.5" font-weight="bold" fill="#4a3300">?</text>
      </g>`,
    cloud: `<g transform="translate(99 ${27 - ph * 2})" opacity="0.95">
        <ellipse cx="0" cy="0" rx="10.5" ry="6.6" fill="#cfe3ff"/>
        <ellipse cx="-6.6" cy="2" rx="5.6" ry="4.6" fill="#cfe3ff"/>
        <ellipse cx="6.6" cy="2" rx="5.6" ry="4.6" fill="#cfe3ff"/>
        <circle cx="${-4 + ph * 8}" cy="0" r="1.3" fill="#4aa3ff"/>
      </g>`,
    stop: `<g transform="translate(64 22)">
        <polygon points="-8,-19 8,-19 19,-8 19,8 8,19 -8,19 -19,8 -19,-8" transform="scale(0.6)" fill="#e5484d"/>
        <rect x="-6.2" y="-1.5" width="12.4" height="3" rx="1.2" fill="#ffffff"/>
      </g>`,
    zzz: `<g fill="#9db2c8" font-family="Arial, sans-serif" font-weight="bold">
        <text x="94" y="${34 - ph * 4}" font-size="11" opacity="${0.9 - ph * 0.3}">z</text>
        <text x="102" y="${26 - ph * 6}" font-size="8" opacity="${0.7 - ph * 0.3}">z</text>
      </g>`,
    check: `<g transform="translate(98 28)">
        <circle r="9" fill="#46a758"/>
        <path d="M -4.2 0 L -1.4 3.3 L 4.7 -3.3" stroke="#ffffff" stroke-width="2.3" fill="none" stroke-linecap="round" stroke-linejoin="round"/>
      </g>`,
    cross: `<g transform="translate(98 28)">
        <circle r="9" fill="#e5484d"/>
        <path d="M -3.6 -3.6 L 3.6 3.6 M 3.6 -3.6 L -3.6 3.6" stroke="#ffffff" stroke-width="2.3" stroke-linecap="round"/>
      </g>`,
    drop: `<g transform="translate(99 ${28 + ph * 2})">
        <path d="M 0 -6 C 4 0, 5 3, 0 6 C -5 3, -4 0, 0 -6 Z" fill="#7fb8f0" opacity="0.9"/>
      </g>`,
    spark: `<g transform="translate(98 27)" opacity="${0.85 - ph * 0.4}">
        <path d="M 0 -8 L 2 -2 L 8 0 L 2 2 L 0 8 L -2 2 L -8 0 L -2 -2 Z" fill="${v.coreTeal}"/>
      </g>`,
  };

  return `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <g transform="translate(64 120) rotate(${P.bodyLean}) scale(${squashX} ${squashY}) translate(-64 -120)">
    <g transform="translate(0 ${P.bodyBob})">
      <path d="${tailPath}" stroke="${edge}" stroke-width="5.2" fill="none" stroke-linecap="round"/>
      <circle cx="${tailTipPos.x}" cy="${tailTipPos.y}" r="3.9" fill="${tailTipCol}"/>
      ${bodySvg}
      ${shield}
      ${headGroup}
    </g>
  </g>
  ${overlays[P.overlay] ?? ""}
</svg>`;
}

function hexPts(spin) {
  return Array.from({ length: 6 }, (_, i) => {
    const a = ((60 * i - 90 + spin) * Math.PI) / 180;
    return `${(6.4 * Math.cos(a)).toFixed(2)},${(6.4 * Math.sin(a)).toFixed(2)}`;
  }).join(" ");
}
