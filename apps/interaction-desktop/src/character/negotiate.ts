// CPP §3.3／§3.4：能力協商（deterministic）。
//
// 對 20 個 canonical intent 逐一解析：exact → 換 intent（substituted）→ 換能力鏈
// （substituted）→ reduced motion 調整 → 安全 intent 回 system.text／非安全回 unsupported。
// 協定 major 不同一律拒絕（typed error），不猜。custom channel 只分類，不影響安全。

import {
  CapabilityDecl,
  CHARACTER_INTENTS,
  CharacterIntent,
  CUSTOM_CAPABILITY_ID_RE,
  FallbackDecl,
  Hello,
  INTENT_CAPABILITIES,
  IntentResolution,
  isSafetyIntent,
  isSemanticChannel,
  Negotiate,
  Negotiated,
  parseProtocolVersion,
  Resolution,
} from "./protocol";

/** 握手拒絕：協定 major 不同或版本字串非法。 */
export class ProtocolVersionError extends Error {
  readonly code = "protocol-version" as const;
  constructor(
    message: string,
    readonly offered: string,
    readonly expected: string
  ) {
    super(message);
    this.name = "ProtocolVersionError";
  }
}

interface CapTry {
  ok: boolean;
  reduced: boolean;
  decl?: CapabilityDecl;
}

function tryCapability(caps: Record<string, CapabilityDecl>, capId: string, reducedMotion: boolean): CapTry {
  const decl = caps[capId];
  if (!decl || !decl.supported) return { ok: false, reduced: false };
  if (reducedMotion) {
    const b = decl.reducedMotionBehavior;
    if (b === "disabled") return { ok: false, reduced: false };
    return { ok: true, reduced: b === "static" || b === "reduced", decl };
  }
  return { ok: true, reduced: false, decl };
}

function firstCapability(
  caps: Record<string, CapabilityDecl>,
  candidates: readonly string[],
  reducedMotion: boolean
): { capId: string; reduced: boolean; decl: CapabilityDecl } | null {
  for (const capId of candidates) {
    const r = tryCapability(caps, capId, reducedMotion);
    if (r.ok && r.decl) return { capId, reduced: r.reduced, decl: r.decl };
  }
  return null;
}

function finalResolution(base: Resolution, reduced: boolean): Resolution {
  return reduced ? "reduced" : base;
}

/** 單一 intent 的解析（§3.4 步驟 1–5）。 */
export function resolveIntent(
  intent: CharacterIntent,
  offer: Pick<Negotiate, "capabilities" | "intents">,
  fallbacks: FallbackDecl,
  reducedMotion: boolean
): IntentResolution {
  const caps = offer.capabilities;
  const candidates = INTENT_CAPABILITIES[intent];

  // 1. 原生 intent ＋ 對應能力 supported → exact
  if (offer.intents.includes(intent)) {
    const hit = firstCapability(caps, candidates, reducedMotion);
    if (hit) {
      const res: IntentResolution = { resolution: finalResolution("exact", hit.reduced), via: hit.capId };
      if (hit.decl.variants?.includes(intent)) res.variant = intent;
      return res;
    }
  }

  // 2. fallbacks.intents（只換一次）
  //    安全守衛：安全 intent 只能換成另一個安全 intent（failed → blocked 合法，
  //    request-consent → greet 不合法）。呈現層沒有權限主權，不得把安全語意演成
  //    打招呼／玩耍；被擋下的替換落到步驟 3／5（最差 system.text）。
  const alt = fallbacks.intents?.[intent];
  if (alt && alt !== intent && offer.intents.includes(alt) && (!isSafetyIntent(intent) || isSafetyIntent(alt))) {
    const hit = firstCapability(caps, INTENT_CAPABILITIES[alt], reducedMotion);
    if (hit) {
      const res: IntentResolution = {
        resolution: finalResolution("substituted", hit.reduced),
        via: hit.capId,
        viaIntent: alt,
      };
      if (hit.decl.variants?.includes(alt)) res.variant = alt;
      return res;
    }
  }

  // 3. fallbacks.capabilities[primary] 鏈（disabled 的能力略過＝步驟 4）
  const primary = candidates[0];
  const chain = fallbacks.capabilities?.[primary] ?? [];
  const hit = firstCapability(caps, chain, reducedMotion);
  if (hit) {
    return { resolution: finalResolution("substituted", hit.reduced), via: hit.capId };
  }

  // 5. 什麼都沒有
  if (isSafetyIntent(intent)) return { resolution: "substituted", via: "system.text" };
  return { resolution: "unsupported" };
}

/**
 * 呈現時該用哪個 intent：安全 intent 只接受同樣是安全 intent 的 `viaIntent`，
 * 其餘一律回 envelope 的原始 intent。adapter 用它挑固定文案／動畫，確保呈現層
 * 不會（因為惡意 manifest 或自帶 fallback）把安全語意換成日常演出。
 */
export function presentedIntent(intent: CharacterIntent, viaIntent?: CharacterIntent): CharacterIntent {
  if (!viaIntent || viaIntent === intent) return intent;
  if (isSafetyIntent(intent) && !isSafetyIntent(viaIntent)) return intent;
  return viaIntent;
}

/** 分類 channel：語意 channel 或 namespaced custom 接受（custom 標 nonSafety）；其餘忽略。 */
export function classifyChannels(channels: readonly string[]): {
  accepted: string[];
  ignored: string[];
  nonSafety: string[];
} {
  const accepted: string[] = [];
  const ignored: string[] = [];
  const nonSafety: string[] = [];
  for (const ch of channels) {
    if (typeof ch !== "string" || accepted.includes(ch) || ignored.includes(ch)) continue;
    if (isSemanticChannel(ch)) accepted.push(ch);
    else if (CUSTOM_CAPABILITY_ID_RE.test(ch)) {
      accepted.push(ch);
      nonSafety.push(ch);
    } else ignored.push(ch);
  }
  return { accepted, ignored, nonSafety };
}

/**
 * §3.3 步驟 3：由 hello 與 adapter 的 negotiate 算出 NegotiatedCapabilities。
 * major 不同 → 擲 ProtocolVersionError（Gateway 轉成 error{code:"protocol-version"}）。
 */
export function negotiate(
  hello: Hello,
  offer: Negotiate,
  fallbacks: FallbackDecl = offer.fallbacks ?? {},
  generation: number = offer.generation
): Negotiated {
  const expected = parseProtocolVersion(hello.protocolVersion);
  const offered = parseProtocolVersion(offer.protocolVersion);
  if (!expected) {
    throw new ProtocolVersionError("hello.protocolVersion is not major.minor", String(offer.protocolVersion), String(hello.protocolVersion));
  }
  if (!offered || offered.major !== expected.major) {
    throw new ProtocolVersionError(
      `protocol major mismatch (adapter offered ${String(offer.protocolVersion).slice(0, 16)}, runtime speaks ${hello.protocolVersion})`,
      String(offer.protocolVersion),
      hello.protocolVersion
    );
  }

  const reducedMotion = hello.reducedMotion === true;
  const allCaps: Record<string, CapabilityDecl> = { ...offer.capabilities };
  const resolutions = {} as Record<CharacterIntent, IntentResolution>;
  for (const intent of CHARACTER_INTENTS) {
    resolutions[intent] = resolveIntent(intent, { capabilities: allCaps, intents: offer.intents }, fallbacks, reducedMotion);
  }

  const ch = classifyChannels(offer.channels ?? []);

  // 最終有效宣告：只保留 supported 者；reduced motion 時調整 qualityLevel／停用 disabled 者。
  const effective: Record<string, CapabilityDecl> = {};
  const merge = (src: Record<string, CapabilityDecl>) => {
    for (const [id, decl] of Object.entries(src)) {
      if (!decl?.supported) continue;
      const copy: CapabilityDecl = { ...decl };
      if (reducedMotion) {
        if (decl.reducedMotionBehavior === "disabled") continue;
        if (decl.reducedMotionBehavior === "static") copy.qualityLevel = "minimal";
        else if (decl.reducedMotionBehavior === "reduced") copy.qualityLevel = "reduced";
      }
      effective[id] = copy;
    }
  };
  merge(offer.capabilities ?? {});
  merge(offer.inputCapabilities ?? {});
  effective["system.text"] = { supported: true, qualityLevel: "minimal", interruptible: false, resumable: false };

  return {
    type: "negotiated",
    characterInstanceId: hello.characterInstanceId,
    generation,
    reducedMotion,
    resolutions,
    acceptedChannels: ch.accepted,
    ignoredChannels: ch.ignored,
    nonSafetyChannels: ch.nonSafety,
    capabilities: effective,
  };
}
