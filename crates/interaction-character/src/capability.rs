//! §3 能力（Capability）與協商：canonical id、namespaced custom、§3.4 確定性解析演算法。

use crate::intent::CharacterIntent;
use crate::manifest::{CapabilityDecl, Fallbacks, ReducedMotionBehavior};
use crate::wire::{Hello, Negotiate, Negotiated};
use crate::{parse_protocol_version, PROTOCOL_MAJOR, PROTOCOL_VERSION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// §3.1 canonical capability ids（26）。`system.text` 由 Runtime 提供、永遠可用。
pub const CANONICAL_CAPABILITIES: [&str; 26] = [
    "visual.presence",
    "visual.pose",
    "visual.expression",
    "visual.gaze",
    "visual.locomotion",
    "visual.overlay",
    "visual.particles",
    "visual.prop",
    "visual.textBubble",
    "audio.speech",
    "audio.effect",
    "haptic.cue",
    "light.cue",
    "input.click",
    "input.hover",
    "input.drag",
    "input.drop",
    "input.pointerProximity",
    "input.text",
    "input.fileDrop",
    "multiCharacter",
    "scene",
    "rollCall",
    "gameplay.toys",
    "gameplay.autonomy",
    "system.text",
];

/// Runtime 的最後退路能力 id。
pub const SYSTEM_TEXT: &str = "system.text";

/// §2.1 已知 canonical 前綴：有此前綴但未收錄的 id 視為 custom 並標 `unknown: true`。
pub const KNOWN_PREFIXES: [&str; 5] = ["visual.", "audio.", "haptic.", "light.", "input."];

/// §5 canonical semantic channels（12）。
pub const CANONICAL_CHANNELS: [&str; 12] = [
    "transform",
    "locomotion",
    "pose",
    "expression",
    "gaze",
    "speech",
    "bubble",
    "audio",
    "prop",
    "overlay",
    "particle",
    "scene",
];

/// 能力 id newtype（序列化為字串）。
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct CapabilityId(pub String);

/// 能力 id 分類（§2.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityClass {
    /// §3.1 收錄的 canonical id。
    Canonical,
    /// namespaced custom（≥ 3 段，例如 `com.example.character.wings`）。
    Custom,
    /// 已知 canonical 前綴但未收錄：視為 custom 並標 `unknown: true`。
    UnknownCanonical,
    /// 不合法（既非 canonical、亦非 namespaced）。
    Invalid,
}

impl CapabilityId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_canonical(&self) -> bool {
        is_canonical(&self.0)
    }

    pub fn is_namespaced_custom(&self) -> bool {
        is_namespaced_custom(&self.0)
    }

    pub fn classify(&self) -> CapabilityClass {
        classify_capability(&self.0)
    }
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for CapabilityId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// 是否為 §3.1 canonical id。
pub fn is_canonical(id: &str) -> bool {
    CANONICAL_CAPABILITIES.contains(&id)
}

/// `^[a-z][a-z0-9]*(\.[a-z][a-z0-9]*){2,}$`：至少三段、每段小寫字母開頭。
pub fn is_namespaced_custom(id: &str) -> bool {
    let segments: Vec<&str> = id.split('.').collect();
    if segments.len() < 3 {
        return false;
    }
    segments.iter().all(|seg| {
        let mut chars = seg.chars();
        match chars.next() {
            Some(first) if first.is_ascii_lowercase() => {}
            _ => return false,
        }
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

/// §2.1 分類規則：canonical → namespaced custom → 已知前綴（unknown canonical）→ invalid。
pub fn classify_capability(id: &str) -> CapabilityClass {
    if is_canonical(id) {
        return CapabilityClass::Canonical;
    }
    if is_namespaced_custom(id) {
        return CapabilityClass::Custom;
    }
    if KNOWN_PREFIXES.iter().any(|p| id.starts_with(p)) && is_plain_identifier_path(id) {
        return CapabilityClass::UnknownCanonical;
    }
    CapabilityClass::Invalid
}

/// 已知前綴下的 id 仍須是 `a.b` 這種 ASCII 識別字路徑（每段字母開頭、可含數字，允許 camelCase）。
fn is_plain_identifier_path(id: &str) -> bool {
    id.split('.').all(|seg| {
        let mut chars = seg.chars();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() => {}
            _ => return false,
        }
        chars.all(|c| c.is_ascii_alphanumeric())
    })
}

/// 是否為 §5 canonical channel。
pub fn is_canonical_channel(channel: &str) -> bool {
    CANONICAL_CHANNELS.contains(&channel)
}

/// 每個 intent 可用來表達它的能力，依偏好排序；第一個是「主要能力」（§3.4 步驟 3 的鏈起點）。
pub fn intent_capabilities(intent: CharacterIntent) -> &'static [&'static str] {
    use CharacterIntent::*;
    match intent {
        Idle => &[
            "visual.presence",
            "visual.pose",
            "visual.expression",
            "visual.textBubble",
            "light.cue",
        ],
        Notice | Acknowledge => &[
            "visual.expression",
            "visual.pose",
            "visual.textBubble",
            "audio.effect",
            "light.cue",
            "haptic.cue",
        ],
        // 純聲音／燈光／觸覺角色也要能誠實表達「在想／在等」：視覺優先，沒有視覺才落到 audio／light／haptic。
        Think | Wait => &[
            "visual.expression",
            "visual.pose",
            "visual.textBubble",
            "audio.speech",
            "audio.effect",
            "light.cue",
            "haptic.cue",
        ],
        Work => &[
            "visual.pose",
            "visual.expression",
            "visual.textBubble",
            "audio.speech",
            "audio.effect",
            "light.cue",
            "haptic.cue",
        ],
        Ask => &[
            "visual.textBubble",
            "visual.expression",
            "audio.speech",
            "light.cue",
        ],
        RequestConsent => &[
            "visual.textBubble",
            "visual.expression",
            "audio.speech",
            "light.cue",
            "haptic.cue",
        ],
        Blocked | Failed => &[
            "visual.expression",
            "visual.textBubble",
            "visual.pose",
            "audio.effect",
            "light.cue",
            "haptic.cue",
        ],
        Unknown | Cancelled => &[
            "visual.expression",
            "visual.textBubble",
            "visual.pose",
            "audio.speech",
            "audio.effect",
            "light.cue",
            "haptic.cue",
        ],
        ClaimCompleted => &[
            "visual.expression",
            "visual.textBubble",
            "visual.pose",
            "audio.effect",
            "light.cue",
        ],
        VerifiedSuccess => &[
            "visual.expression",
            "visual.particles",
            "visual.textBubble",
            "audio.effect",
            "light.cue",
        ],
        Offline => &[
            "visual.expression",
            "visual.textBubble",
            "visual.presence",
            "light.cue",
            "audio.effect",
        ],
        Emergency => &[
            "visual.expression",
            "visual.overlay",
            "visual.textBubble",
            "audio.effect",
            "light.cue",
            "haptic.cue",
        ],
        Greet => &[
            "visual.expression",
            "visual.pose",
            "visual.textBubble",
            "audio.speech",
            "audio.effect",
        ],
        Play => &[
            "gameplay.toys",
            "visual.locomotion",
            "visual.pose",
            "visual.expression",
        ],
        Rest | Sleep => &[
            "visual.pose",
            "visual.expression",
            "visual.presence",
            "visual.textBubble",
        ],
    }
}

/// 能力對應的 semantic channels（§5 mixer 的 owner 粒度）。custom 能力不佔 canonical channel。
pub fn capability_channels(capability: &str) -> &'static [&'static str] {
    match capability {
        "visual.presence" => &["transform"],
        "visual.pose" => &["pose"],
        "visual.expression" => &["expression"],
        "visual.gaze" => &["gaze"],
        "visual.locomotion" => &["locomotion", "transform"],
        "visual.overlay" => &["overlay"],
        "visual.particles" => &["particle"],
        "visual.prop" => &["prop"],
        "visual.textBubble" => &["bubble"],
        "audio.speech" => &["speech", "audio"],
        "audio.effect" => &["audio"],
        "scene" | "rollCall" => &["scene"],
        "gameplay.toys" => &["prop", "particle"],
        "gameplay.autonomy" => &["locomotion", "pose"],
        _ => &[],
    }
}

/// 舊 sprite pack 動畫名 ↔ intent 的別名（協商時挑 `variant` 用）。
pub fn intent_variant_aliases(intent: CharacterIntent) -> &'static [&'static str] {
    use CharacterIntent::*;
    match intent {
        Idle => &["idle"],
        Notice => &["notice"],
        Acknowledge => &["acknowledge", "notice"],
        Think => &["think", "thinking"],
        Work => &["work", "act", "routing"],
        Wait => &["wait", "waiting"],
        Ask => &["ask"],
        RequestConsent => &["request-consent", "consent", "ask"],
        Blocked => &["blocked"],
        Unknown => &["unknown"],
        ClaimCompleted => &["claim-completed", "claimed"],
        VerifiedSuccess => &["verified-success", "success"],
        Failed => &["failed"],
        Cancelled => &["cancelled"],
        Offline => &["offline"],
        Emergency => &["emergency"],
        Greet => &["greet"],
        Play => &["play"],
        Rest => &["rest", "quiet"],
        Sleep => &["sleep", "paused"],
    }
}

/// 解析結果等級。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Resolution {
    Exact,
    Substituted,
    Reduced,
    Unsupported,
    Failed,
}

/// 單一 intent 的協商結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntentResolution {
    pub resolution: Resolution,
    /// 實際使用的能力（`system.text` 代表 Runtime 文字退路）；`unsupported` 時為 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<CapabilityId>,
    /// 經 `fallbacks.intents` 換成的實際 intent（§3.4 步驟 2）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via_intent: Option<CharacterIntent>,
    /// 該能力 `variants` 內對應此 intent 的 variant（例如 sprite 動畫名）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl IntentResolution {
    pub fn unsupported() -> Self {
        IntentResolution {
            resolution: Resolution::Unsupported,
            via: None,
            via_intent: None,
            variant: None,
        }
    }

    pub fn system_text() -> Self {
        IntentResolution {
            resolution: Resolution::Substituted,
            via: Some(CapabilityId::new(SYSTEM_TEXT)),
            via_intent: None,
            variant: None,
        }
    }

    pub fn is_system_text(&self) -> bool {
        self.via.as_ref().map(|v| v.as_str()) == Some(SYSTEM_TEXT)
    }
}

/// 協商完成後的有效能力集合。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct NegotiatedCapabilities {
    pub resolutions: BTreeMap<CharacterIntent, IntentResolution>,
    pub capabilities: BTreeMap<String, CapabilityDecl>,
    pub input_capabilities: BTreeMap<String, CapabilityDecl>,
    pub accepted_channels: Vec<String>,
    /// `acceptedChannels` 中的 namespaced custom channel：不能影響 priority、truthState 或搶占。
    pub non_safety_channels: Vec<String>,
    pub ignored_channels: Vec<String>,
}

/// 協商錯誤（Gateway 回 `error{code}` 並拒絕）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(tag = "code", rename_all = "kebab-case")]
pub enum NegotiationError {
    #[error(
        "protocol version {offered} is not compatible with {PROTOCOL_VERSION} (major mismatch)"
    )]
    ProtocolVersion { offered: String },
    #[error("negotiate.characterId {offered} does not match registered manifest {expected}")]
    CharacterMismatch { expected: String, offered: String },
    #[error("unknown character instance")]
    UnknownInstance,
}

impl NegotiationError {
    /// wire `error{code}` 用的 code。
    pub fn code(&self) -> &'static str {
        match self {
            NegotiationError::ProtocolVersion { .. } => "protocol-version",
            NegotiationError::CharacterMismatch { .. } => "character-mismatch",
            NegotiationError::UnknownInstance => "unknown-instance",
        }
    }
}

enum CapabilityTry {
    Unsupported,
    Disabled,
    Ok(Resolution),
}

fn try_capability(
    capabilities: &BTreeMap<String, CapabilityDecl>,
    id: &str,
    reduced_motion: bool,
) -> CapabilityTry {
    let Some(decl) = capabilities.get(id).filter(|d| d.supported) else {
        return CapabilityTry::Unsupported;
    };
    if !reduced_motion {
        return CapabilityTry::Ok(Resolution::Exact);
    }
    match decl
        .reduced_motion_behavior
        .unwrap_or(ReducedMotionBehavior::Unchanged)
    {
        ReducedMotionBehavior::Disabled => CapabilityTry::Disabled,
        ReducedMotionBehavior::Static | ReducedMotionBehavior::Reduced => {
            CapabilityTry::Ok(Resolution::Reduced)
        }
        ReducedMotionBehavior::Unchanged => CapabilityTry::Ok(Resolution::Exact),
    }
}

fn pick_variant(decl: Option<&CapabilityDecl>, intent: CharacterIntent) -> Option<String> {
    let decl = decl?;
    intent_variant_aliases(intent)
        .iter()
        .find(|alias| decl.variants.iter().any(|v| v == *alias))
        .map(|s| s.to_string())
}

fn resolve_native(
    intent: CharacterIntent,
    capabilities: &BTreeMap<String, CapabilityDecl>,
    reduced_motion: bool,
) -> Option<(String, Resolution)> {
    for cap in intent_capabilities(intent) {
        if let CapabilityTry::Ok(r) = try_capability(capabilities, cap, reduced_motion) {
            return Some((cap.to_string(), r));
        }
    }
    None
}

/// 步驟 3：沿 `fallbacks.capabilities[primary]` 鏈找第一個 supported 的能力（有界、去重）。
fn resolve_chain(
    primary: &str,
    capabilities: &BTreeMap<String, CapabilityDecl>,
    fallbacks: &Fallbacks,
    reduced_motion: bool,
) -> Option<(String, Resolution)> {
    const MAX_VISITS: usize = 16;
    let mut visited: BTreeSet<String> = BTreeSet::new();
    visited.insert(primary.to_string());
    let mut queue: std::collections::VecDeque<String> = fallbacks
        .capabilities
        .get(primary)
        .map(|chain| chain.iter().cloned().collect())
        .unwrap_or_default();
    let mut visits = 0;
    while let Some(cap) = queue.pop_front() {
        if visits >= MAX_VISITS || !visited.insert(cap.clone()) {
            continue;
        }
        visits += 1;
        match try_capability(capabilities, &cap, reduced_motion) {
            CapabilityTry::Ok(r) => return Some((cap, r)),
            CapabilityTry::Unsupported | CapabilityTry::Disabled => {
                if let Some(next) = fallbacks.capabilities.get(&cap) {
                    queue.extend(next.iter().cloned());
                }
            }
        }
    }
    None
}

/// §3.4：解析單一 intent（確定性）。
pub fn resolve_intent(
    intent: CharacterIntent,
    offered_intents: &BTreeSet<String>,
    capabilities: &BTreeMap<String, CapabilityDecl>,
    fallbacks: &Fallbacks,
    reduced_motion: bool,
) -> IntentResolution {
    // 1. 原生支援。
    if offered_intents.contains(intent.as_str()) {
        if let Some((cap, resolution)) = resolve_native(intent, capabilities, reduced_motion) {
            let variant = pick_variant(capabilities.get(&cap), intent);
            return IntentResolution {
                resolution,
                via: Some(CapabilityId::new(cap)),
                via_intent: None,
                variant,
            };
        }
    }
    // 2. fallbacks.intents（只換一次）。
    if let Some(alt) = fallbacks.intents.get(intent.as_str()) {
        if let Some(alt_intent) = CharacterIntent::parse(alt) {
            if alt_intent != intent && offered_intents.contains(alt) {
                if let Some((cap, resolution)) =
                    resolve_native(alt_intent, capabilities, reduced_motion)
                {
                    let variant = pick_variant(capabilities.get(&cap), alt_intent);
                    return IntentResolution {
                        resolution: match resolution {
                            Resolution::Reduced => Resolution::Reduced,
                            _ => Resolution::Substituted,
                        },
                        via: Some(CapabilityId::new(cap)),
                        via_intent: Some(alt_intent),
                        variant,
                    };
                }
            }
        }
    }
    // 3. fallbacks.capabilities 鏈（自主要能力起）。
    if let Some(primary) = intent_capabilities(intent).first() {
        if let Some((cap, resolution)) =
            resolve_chain(primary, capabilities, fallbacks, reduced_motion)
        {
            let variant = pick_variant(capabilities.get(&cap), intent);
            return IntentResolution {
                resolution: match resolution {
                    Resolution::Reduced => Resolution::Reduced,
                    _ => Resolution::Substituted,
                },
                via: Some(CapabilityId::new(cap)),
                via_intent: None,
                variant,
            };
        }
    }
    // 5. 什麼都沒有：安全 intent 走 system.text，其餘 unsupported。
    if intent.is_safety() {
        IntentResolution::system_text()
    } else {
        IntentResolution::unsupported()
    }
}

/// 把 offer 的 channels 分成 accepted／nonSafety／ignored（去重、保序）。
pub fn classify_channels(channels: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut accepted = Vec::new();
    let mut non_safety = Vec::new();
    let mut ignored = Vec::new();
    let mut seen = BTreeSet::new();
    for channel in channels {
        if !seen.insert(channel.clone()) {
            continue;
        }
        if is_canonical_channel(channel) {
            accepted.push(channel.clone());
        } else if is_namespaced_custom(channel) {
            accepted.push(channel.clone());
            non_safety.push(channel.clone());
        } else {
            ignored.push(channel.clone());
        }
    }
    (accepted, non_safety, ignored)
}

/// §3.3／§3.4：對 20 個 intent 全部解析。major 不同 → `Err(ProtocolVersion)`（不猜）。
///
/// `offer.capabilities` 視為最終有效宣告（Gateway 在呼叫前已與 manifest 取交集）；
/// `manifest_fallbacks` 來自已驗證的 manifest。
pub fn negotiate(
    hello: &Hello,
    offer: &Negotiate,
    manifest_fallbacks: &Fallbacks,
) -> Result<Negotiated, NegotiationError> {
    match parse_protocol_version(&offer.protocol_version) {
        Some((major, _)) if major == PROTOCOL_MAJOR => {}
        _ => {
            return Err(NegotiationError::ProtocolVersion {
                offered: crate::truncate_for_echo(&offer.protocol_version),
            })
        }
    }
    let offered_intents: BTreeSet<String> = offer.intents.iter().cloned().collect();
    let supported: BTreeMap<String, CapabilityDecl> = offer
        .capabilities
        .iter()
        .filter(|(_, d)| d.supported)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut resolutions = BTreeMap::new();
    for intent in CharacterIntent::ALL {
        resolutions.insert(
            intent,
            resolve_intent(
                intent,
                &offered_intents,
                &supported,
                manifest_fallbacks,
                hello.reduced_motion,
            ),
        );
    }
    let (accepted_channels, non_safety_channels, ignored_channels) =
        classify_channels(&offer.channels);
    let input_capabilities: BTreeMap<String, CapabilityDecl> = offer
        .input_capabilities
        .iter()
        .filter(|(_, d)| d.supported)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Ok(Negotiated {
        character_instance_id: hello.character_instance_id.clone(),
        generation: offer.generation,
        reduced_motion: hello.reduced_motion,
        resolutions,
        accepted_channels,
        non_safety_channels,
        ignored_channels,
        capabilities: supported,
        input_capabilities,
    })
}

impl Negotiated {
    /// 有效能力集合（不含 wire 標頭）。
    pub fn capabilities(&self) -> NegotiatedCapabilities {
        NegotiatedCapabilities {
            resolutions: self.resolutions.clone(),
            capabilities: self.capabilities.clone(),
            input_capabilities: self.input_capabilities.clone(),
            accepted_channels: self.accepted_channels.clone(),
            non_safety_channels: self.non_safety_channels.clone(),
            ignored_channels: self.ignored_channels.clone(),
        }
    }

    /// 是否完全沒有任何呈現能力（安全訊息全部走 `system.text`）。
    pub fn has_no_presentation(&self) -> bool {
        self.capabilities.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_list_has_26_unique_ids() {
        let set: BTreeSet<&str> = CANONICAL_CAPABILITIES.iter().copied().collect();
        assert_eq!(set.len(), 26);
        assert!(is_canonical("visual.textBubble"));
        assert!(is_canonical("system.text"));
        assert!(!is_canonical("visual.wings"));
    }

    #[test]
    fn namespaced_custom_regex() {
        assert!(is_namespaced_custom("com.example.character.wings"));
        assert!(is_namespaced_custom("a.b.c"));
        assert!(!is_namespaced_custom("a.b"));
        assert!(!is_namespaced_custom("Com.example.x"));
        assert!(!is_namespaced_custom("com.1x.y"));
        assert!(!is_namespaced_custom("com..y.z"));
        assert!(!is_namespaced_custom("com.exa-mple.y"));
    }

    #[test]
    fn classification_rules() {
        assert_eq!(
            classify_capability("visual.pose"),
            CapabilityClass::Canonical
        );
        assert_eq!(
            classify_capability("com.example.character.wings"),
            CapabilityClass::Custom
        );
        assert_eq!(
            classify_capability("visual.wings"),
            CapabilityClass::UnknownCanonical
        );
        assert_eq!(
            classify_capability("input.eyeTracking"),
            CapabilityClass::UnknownCanonical
        );
        assert_eq!(classify_capability("wings"), CapabilityClass::Invalid);
        assert_eq!(classify_capability("visual."), CapabilityClass::Invalid);
        assert_eq!(classify_capability("visual.../x"), CapabilityClass::Invalid);
    }

    #[test]
    fn every_intent_has_a_primary_capability_and_channels_are_canonical() {
        for intent in CharacterIntent::ALL {
            let caps = intent_capabilities(intent);
            assert!(!caps.is_empty(), "{intent}");
            for cap in caps {
                assert!(is_canonical(cap), "{cap}");
                for ch in capability_channels(cap) {
                    assert!(is_canonical_channel(ch), "{ch}");
                }
            }
        }
    }

    #[test]
    fn channel_classification() {
        let (accepted, non_safety, ignored) = classify_channels(&[
            "pose".into(),
            "com.example.character.wings".into(),
            "wings".into(),
            "pose".into(),
        ]);
        assert_eq!(accepted, vec!["pose", "com.example.character.wings"]);
        assert_eq!(non_safety, vec!["com.example.character.wings"]);
        assert_eq!(ignored, vec!["wings"]);
    }
}
