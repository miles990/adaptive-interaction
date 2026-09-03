//! §13：protocol version negotiation、capability negotiation、unknown capability、fallback selection、
//! reduced-motion negotiation、pure-audio／no-visual fallback。

mod common;

use common::*;
use interaction_character::*;
use std::collections::BTreeMap;

fn manifest_with(
    id: &str,
    caps: &[(&str, CapabilityDecl)],
    intents: &[&str],
    fallbacks: Fallbacks,
) -> CharacterManifest {
    let mut m = minimal_manifest(id, "text");
    m.capabilities = caps
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    m.intents = intents.iter().map(|s| s.to_string()).collect();
    m.fallbacks = fallbacks;
    m
}

fn negotiated_for(manifest: &CharacterManifest, reduced_motion: bool) -> Negotiated {
    let offer = Negotiate::from_manifest(manifest, 1);
    negotiate(&hello("inst", reduced_motion), &offer, &manifest.fallbacks).expect("negotiates")
}

fn res(n: &Negotiated, intent: CharacterIntent) -> &IntentResolution {
    n.resolutions.get(&intent).expect("every intent resolved")
}

fn via(n: &Negotiated, intent: CharacterIntent) -> &str {
    res(n, intent)
        .via
        .as_ref()
        .map(|v| v.as_str())
        .unwrap_or("<none>")
}

#[test]
fn protocol_major_mismatch_is_refused_not_guessed() {
    let m = text_manifest();
    let mut offer = Negotiate::from_manifest(&m, 1);
    offer.protocol_version = "2.0".into();
    let err = negotiate(&hello("inst", false), &offer, &m.fallbacks).expect_err("refused");
    assert!(matches!(err, NegotiationError::ProtocolVersion { .. }));
    assert_eq!(err.code(), "protocol-version");
    offer.protocol_version = "banana".into();
    assert!(negotiate(&hello("inst", false), &offer, &m.fallbacks).is_err());

    // 透過 Gateway：回 error{code:"protocol-version"} 並不進入 negotiated 狀態。
    let mut gw = Gateway::default();
    let id = gw.register_instance(m.clone(), CharacterRole::PrimaryCompanion);
    let mut offer = Negotiate::from_manifest(&m, 1);
    offer.protocol_version = "2.0".into();
    let out = gw.on_message(&id, WireMessage::Negotiate(offer), t(0));
    let errors: Vec<_> = sends(&out)
        .into_iter()
        .filter(|m| matches!(m, WireMessage::Error { code, .. } if code == "protocol-version"))
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(gw.negotiated(&id).is_none());
    assert_eq!(gw.generation(&id), Some(0));
}

#[test]
fn newer_minor_is_accepted() {
    let m = text_manifest();
    let mut offer = Negotiate::from_manifest(&m, 1);
    offer.protocol_version = "1.9".into();
    let n = negotiate(&hello("inst", false), &offer, &m.fallbacks).expect("newer minor ok");
    assert_eq!(n.resolutions.len(), 20);
}

#[test]
fn full_reference_adapter_resolves_everything_exact() {
    let n = negotiated_for(&shu_manifest(), false);
    for intent in CharacterIntent::ALL {
        assert_eq!(res(&n, intent).resolution, Resolution::Exact, "{intent}");
    }
    assert_eq!(via(&n, CharacterIntent::Think), "visual.expression");
    assert_eq!(
        res(&n, CharacterIntent::Think).variant.as_deref(),
        Some("thinking")
    );
    assert_eq!(via(&n, CharacterIntent::Play), "gameplay.toys");
    // 有效宣告只含 supported、不含 custom 以外被拒的東西；custom 保留。
    assert!(n.capabilities.contains_key("com.example.character.wings"));
    assert!(!n.capabilities.contains_key("system.text"));
    assert_eq!(n.input_capabilities.len(), 7);
}

#[test]
fn text_adapter_resolves_every_intent_via_bubble_or_presence() {
    let n = negotiated_for(&text_manifest(), false);
    for intent in CharacterIntent::ALL {
        let r = res(&n, intent);
        assert_ne!(r.resolution, Resolution::Unsupported, "{intent}");
        let v = via(&n, intent);
        assert!(
            ["visual.textBubble", "visual.presence", "audio.effect"].contains(&v),
            "{intent} via {v}"
        );
    }
    assert_eq!(via(&n, CharacterIntent::Idle), "visual.presence");
    assert_eq!(via(&n, CharacterIntent::Ask), "visual.textBubble");
    assert_eq!(res(&n, CharacterIntent::Work).resolution, Resolution::Exact);
}

#[test]
fn no_expression_character_gets_exact_substituted_reduced_unsupported() {
    let mut fallbacks = Fallbacks::default();
    fallbacks.capabilities.insert(
        "visual.expression".into(),
        vec!["visual.pose".into(), "visual.textBubble".into()],
    );
    let m = manifest_with(
        "poser",
        &[
            (
                "visual.pose",
                CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Reduced),
            ),
            (
                "visual.textBubble",
                CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Unchanged),
            ),
        ],
        &["idle", "work", "ask"],
        fallbacks,
    );
    let n = negotiated_for(&m, false);
    // exact：intents 有列且對應能力 supported。
    assert_eq!(res(&n, CharacterIntent::Work).resolution, Resolution::Exact);
    assert_eq!(via(&n, CharacterIntent::Work), "visual.pose");
    assert_eq!(res(&n, CharacterIntent::Ask).resolution, Resolution::Exact);
    assert_eq!(via(&n, CharacterIntent::Ask), "visual.textBubble");
    assert_eq!(res(&n, CharacterIntent::Idle).resolution, Resolution::Exact);
    // substituted：notice 未列出 → 走 fallbacks.capabilities[visual.expression] → pose。
    assert_eq!(
        res(&n, CharacterIntent::Notice).resolution,
        Resolution::Substituted
    );
    assert_eq!(via(&n, CharacterIntent::Notice), "visual.pose");
    assert!(res(&n, CharacterIntent::Notice).via_intent.is_none());
    // 安全 intent 也照鏈走（不是 exact）。
    assert_eq!(
        res(&n, CharacterIntent::Emergency).resolution,
        Resolution::Substituted
    );
    assert_eq!(via(&n, CharacterIntent::Emergency), "visual.pose");
    // unsupported：play 的主要能力 gameplay.toys 沒有鏈、非安全。
    assert_eq!(
        res(&n, CharacterIntent::Play).resolution,
        Resolution::Unsupported
    );
    assert!(res(&n, CharacterIntent::Play).via.is_none());
    // reduced：reducedMotion=true 時 pose 為 reduced。
    let n = negotiated_for(&m, true);
    assert!(n.reduced_motion);
    assert_eq!(
        res(&n, CharacterIntent::Work).resolution,
        Resolution::Reduced
    );
    assert_eq!(via(&n, CharacterIntent::Work), "visual.pose");
    assert_eq!(
        res(&n, CharacterIntent::Ask).resolution,
        Resolution::Exact,
        "unchanged stays exact"
    );
    assert_eq!(
        res(&n, CharacterIntent::Notice).resolution,
        Resolution::Reduced
    );
}

#[test]
fn pure_audio_character_still_resolves_all_safety_intents() {
    let mut speech = CapabilityDecl::supported();
    speech.requires_audio = true;
    let mut effect = CapabilityDecl::supported();
    effect.requires_audio = true;
    let m = manifest_with(
        "audio-only",
        &[("audio.speech", speech), ("audio.effect", effect)],
        &[
            "notice",
            "ask",
            "request-consent",
            "blocked",
            "failed",
            "emergency",
            "greet",
        ],
        Fallbacks::default(),
    );
    let n = negotiated_for(&m, false);
    for intent in CharacterIntent::ALL.iter().filter(|i| i.is_safety()) {
        let r = res(&n, *intent);
        assert_ne!(
            r.resolution,
            Resolution::Unsupported,
            "{intent} must never be lost"
        );
        let v = via(&n, *intent);
        assert!(
            v.starts_with("audio.") || v == SYSTEM_TEXT,
            "{intent} resolved via {v}"
        );
    }
    assert_eq!(via(&n, CharacterIntent::Ask), "audio.speech");
    assert_eq!(res(&n, CharacterIntent::Ask).resolution, Resolution::Exact);
    assert_eq!(via(&n, CharacterIntent::Blocked), "audio.effect");
    // 這個 manifest 的 `intents` 只列了 7 個：沒宣告接得住 wait／unknown，就不會被假裝支援
    // （安全 intent 落 system.text）。有宣告的情況見
    // `pure_audio_character_expresses_work_wait_unknown_when_it_offers_them`。
    assert_eq!(via(&n, CharacterIntent::Wait), SYSTEM_TEXT);
    assert_eq!(
        res(&n, CharacterIntent::Wait).resolution,
        Resolution::Substituted
    );
    assert!(res(&n, CharacterIntent::Wait).is_system_text());
    assert_eq!(via(&n, CharacterIntent::Unknown), SYSTEM_TEXT);
    // 非安全且無音訊對應 → unsupported（誠實，不假裝）。
    assert_eq!(
        res(&n, CharacterIntent::Idle).resolution,
        Resolution::Unsupported
    );
    assert_eq!(
        res(&n, CharacterIntent::Play).resolution,
        Resolution::Unsupported
    );
}

#[test]
fn zero_capability_character_routes_safety_to_system_text_only() {
    let m = manifest_with("nothing", &[], &[], Fallbacks::default());
    let n = negotiated_for(&m, false);
    assert!(n.has_no_presentation());
    for intent in CharacterIntent::ALL {
        let r = res(&n, intent);
        if intent.is_safety() {
            assert!(r.is_system_text(), "{intent}");
            assert_eq!(r.resolution, Resolution::Substituted);
        } else {
            assert_eq!(r.resolution, Resolution::Unsupported, "{intent}");
        }
    }
}

#[test]
fn unknown_capability_and_custom_channel_handling() {
    let m = shu_manifest();
    let report = validate(&m).expect("valid");
    assert_eq!(report.custom_capabilities.len(), 2);
    let mut offer = Negotiate::from_manifest(&m, 1);
    offer.channels = vec![
        "pose".into(),
        "com.example.character.wings".into(),
        "wings".into(),
        "Pose".into(),
    ];
    let n = negotiate(&hello("inst", false), &offer, &m.fallbacks).expect("negotiates");
    assert_eq!(
        n.accepted_channels,
        vec!["pose", "com.example.character.wings"]
    );
    assert_eq!(n.non_safety_channels, vec!["com.example.character.wings"]);
    assert_eq!(n.ignored_channels, vec!["wings", "Pose"]);
    // custom 能力不能替安全 intent 說話：emergency 仍走 canonical 能力。
    assert_eq!(via(&n, CharacterIntent::Emergency), "visual.expression");
    // 只有 custom 能力的角色：安全 intent 仍走 system.text。
    let only_custom = manifest_with(
        "custom-only",
        &[("com.example.character.wings", CapabilityDecl::supported())],
        &["emergency", "idle"],
        Fallbacks::default(),
    );
    let n = negotiated_for(&only_custom, false);
    assert!(res(&n, CharacterIntent::Emergency).is_system_text());
    assert_eq!(
        res(&n, CharacterIntent::Idle).resolution,
        Resolution::Unsupported
    );
    assert!(n.capabilities.contains_key("com.example.character.wings"));
}

#[test]
fn unsupported_offer_entries_are_dropped_by_gateway_intersection() {
    // offer 宣告 manifest 沒有的能力 → 不生效；manifest 有但 offer 說 unsupported → 不生效。
    let m = text_manifest();
    let mut gw = Gateway::default();
    let id = gw.register_instance(m.clone(), CharacterRole::PrimaryCompanion);
    let mut offer = Negotiate::from_manifest(&m, 1);
    offer
        .capabilities
        .insert("visual.expression".into(), CapabilityDecl::supported());
    let mut unsupported = CapabilityDecl::supported();
    unsupported.supported = false;
    offer
        .capabilities
        .insert("visual.textBubble".into(), unsupported);
    offer.intents.push("dance".into());
    let (n, _) = gw.on_negotiate(&id, offer, t(0)).expect("negotiates");
    assert!(!n.capabilities.contains_key("visual.expression"));
    assert!(!n.capabilities.contains_key("visual.textBubble"));
    assert!(n.capabilities.contains_key("visual.presence"));
    assert_eq!(via(&n, CharacterIntent::Idle), "visual.presence");
    assert!(res(&n, CharacterIntent::Ask).is_system_text());
    assert_eq!(n.generation, 1);
}

#[test]
fn fallback_selection_is_deterministic_and_ordered() {
    let mut fallbacks = Fallbacks::default();
    fallbacks.capabilities.insert(
        "visual.expression".into(),
        vec!["visual.textBubble".into(), "visual.pose".into()],
    );
    let m = manifest_with(
        "ordered",
        &[
            ("visual.pose", CapabilityDecl::supported()),
            ("visual.textBubble", CapabilityDecl::supported()),
        ],
        &[],
        fallbacks,
    );
    let a = negotiated_for(&m, false);
    let b = negotiated_for(&m, false);
    assert_eq!(a, b, "same input → same output");
    assert_eq!(
        via(&a, CharacterIntent::Notice),
        "visual.textBubble",
        "first supported in chain wins"
    );
    // 鏈可以遞移：expression → particles(unsupported) → pose。
    let mut fallbacks = Fallbacks::default();
    fallbacks
        .capabilities
        .insert("visual.expression".into(), vec!["visual.particles".into()]);
    fallbacks
        .capabilities
        .insert("visual.particles".into(), vec!["visual.pose".into()]);
    let m = manifest_with(
        "transitive",
        &[("visual.pose", CapabilityDecl::supported())],
        &[],
        fallbacks,
    );
    let n = negotiated_for(&m, false);
    assert_eq!(via(&n, CharacterIntent::Notice), "visual.pose");
    assert_eq!(
        res(&n, CharacterIntent::Notice).resolution,
        Resolution::Substituted
    );
    // 環狀鏈不會無限迴圈。
    let mut fallbacks = Fallbacks::default();
    fallbacks
        .capabilities
        .insert("visual.expression".into(), vec!["visual.particles".into()]);
    fallbacks
        .capabilities
        .insert("visual.particles".into(), vec!["visual.expression".into()]);
    let m = manifest_with("cyclic", &[], &[], fallbacks);
    let n = negotiated_for(&m, false);
    assert_eq!(
        res(&n, CharacterIntent::Notice).resolution,
        Resolution::Unsupported
    );
}

#[test]
fn intent_fallback_is_applied_once_only() {
    let mut fallbacks = Fallbacks::default();
    fallbacks.intents.insert("play".into(), "notice".into());
    fallbacks.intents.insert("notice".into(), "idle".into());
    let m = manifest_with(
        "once",
        &[(
            "visual.expression",
            CapabilityDecl::supported().with_variants(["notice", "idle"]),
        )],
        &["idle"],
        fallbacks,
    );
    let n = negotiated_for(&m, false);
    // play → notice（一次）；notice 未列出 → 不再往 idle 走 → play 的主要能力鏈也沒有 → unsupported。
    assert_eq!(
        res(&n, CharacterIntent::Play).resolution,
        Resolution::Unsupported
    );
    // notice → idle 直接一次替換成功。
    let r = res(&n, CharacterIntent::Notice);
    assert_eq!(r.resolution, Resolution::Substituted);
    assert_eq!(r.via_intent, Some(CharacterIntent::Idle));
    assert_eq!(r.variant.as_deref(), Some("idle"));
    assert_eq!(via(&n, CharacterIntent::Notice), "visual.expression");
}

#[test]
fn reduced_motion_static_reduced_disabled_paths() {
    let mut fallbacks = Fallbacks::default();
    fallbacks
        .capabilities
        .insert("visual.expression".into(), vec!["visual.textBubble".into()]);
    let m = manifest_with(
        "motion",
        &[
            (
                "visual.expression",
                CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Disabled),
            ),
            (
                "visual.textBubble",
                CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Static),
            ),
            (
                "visual.presence",
                CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Reduced),
            ),
        ],
        &["notice", "idle"],
        fallbacks,
    );
    let normal = negotiated_for(&m, false);
    assert_eq!(
        res(&normal, CharacterIntent::Notice).resolution,
        Resolution::Exact
    );
    assert_eq!(via(&normal, CharacterIntent::Notice), "visual.expression");
    let reduced = negotiated_for(&m, true);
    // disabled → 跳過 expression，往下找到 static 的 textBubble → reduced。
    assert_eq!(
        res(&reduced, CharacterIntent::Notice).resolution,
        Resolution::Reduced
    );
    assert_eq!(via(&reduced, CharacterIntent::Notice), "visual.textBubble");
    // reduced → reduced。
    assert_eq!(
        res(&reduced, CharacterIntent::Idle).resolution,
        Resolution::Reduced
    );
    assert_eq!(via(&reduced, CharacterIntent::Idle), "visual.presence");
    // 全部 disabled 的安全 intent 最後仍有 system.text。
    let m = manifest_with(
        "all-disabled",
        &[(
            "visual.expression",
            CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Disabled),
        )],
        &["emergency", "idle"],
        Fallbacks::default(),
    );
    let reduced = negotiated_for(&m, true);
    assert!(res(&reduced, CharacterIntent::Emergency).is_system_text());
    assert_eq!(
        res(&reduced, CharacterIntent::Idle).resolution,
        Resolution::Unsupported
    );
    // 沒有 reducedMotionBehavior（None）視為 unchanged → exact。
    let m = manifest_with(
        "unspecified",
        &[("visual.expression", CapabilityDecl::supported())],
        &["notice"],
        Fallbacks::default(),
    );
    assert_eq!(
        res(&negotiated_for(&m, true), CharacterIntent::Notice).resolution,
        Resolution::Exact
    );
}

#[test]
fn sprite_pack_negotiation_uses_animation_variants_and_intent_fallbacks() {
    let sprite = sprite_manifest();
    let n = negotiated_for(&sprite, false);
    assert_eq!(
        res(&n, CharacterIntent::Think).variant.as_deref(),
        Some("thinking")
    );
    assert_eq!(
        res(&n, CharacterIntent::Work).variant.as_deref(),
        Some("act")
    );
    assert_eq!(
        res(&n, CharacterIntent::Sleep).variant.as_deref(),
        Some("paused")
    );
    // v1.0 沒有 failed 動畫 → failed → blocked（substituted，不退到 success）。
    let failed = res(&n, CharacterIntent::Failed);
    assert_eq!(failed.resolution, Resolution::Substituted);
    assert_eq!(failed.via_intent, Some(CharacterIntent::Blocked));
    assert_eq!(failed.variant.as_deref(), Some("blocked"));
    // play → notice。
    assert_eq!(
        res(&n, CharacterIntent::Play).via_intent,
        Some(CharacterIntent::Notice)
    );
    // 所有安全 intent 都有著落。
    for intent in CharacterIntent::ALL.iter().filter(|i| i.is_safety()) {
        assert_ne!(
            res(&n, *intent).resolution,
            Resolution::Unsupported,
            "{intent}"
        );
    }
}

#[test]
fn negotiated_wire_shape_matches_spec() {
    let n = negotiated_for(&text_manifest(), false);
    let v = serde_json::to_value(WireMessage::Negotiated(n)).expect("serialize");
    assert_eq!(v["type"], "negotiated");
    assert_eq!(v["characterInstanceId"], "inst");
    assert!(v["resolutions"]["emergency"]["resolution"].is_string());
    assert_eq!(v["resolutions"]["ask"]["via"], "visual.textBubble");
    assert!(v["acceptedChannels"].is_array());
    assert!(v["ignoredChannels"].is_array());
    assert!(v["capabilities"]["visual.textBubble"]["supported"]
        .as_bool()
        .unwrap_or(false));
    let keys: BTreeMap<String, serde_json::Value> =
        serde_json::from_value(v["resolutions"].clone()).expect("map");
    assert_eq!(keys.len(), 20);
}

#[test]
fn pure_audio_character_expresses_work_wait_unknown_when_it_offers_them() {
    // 只有聲音的角色（沒有臉、沒有燈），但誠實宣告自己接得住全部 20 個 intent。
    let mut speech = CapabilityDecl::supported();
    speech.requires_audio = true;
    let mut effect = CapabilityDecl::supported();
    effect.requires_audio = true;
    let all: Vec<&str> = CharacterIntent::ALL.iter().map(|i| i.as_str()).collect();
    let m = manifest_with(
        "audio-only-full",
        &[("audio.speech", speech), ("audio.effect", effect)],
        &all,
        Fallbacks::default(),
    );
    let n = negotiated_for(&m, false);
    for intent in [
        CharacterIntent::Work,
        CharacterIntent::Think,
        CharacterIntent::Wait,
        CharacterIntent::Unknown,
        CharacterIntent::Cancelled,
        CharacterIntent::Blocked,
        CharacterIntent::Failed,
    ] {
        let r = res(&n, intent);
        assert_ne!(
            r.resolution,
            Resolution::Unsupported,
            "{intent} 應該能用聲音表達"
        );
        assert!(
            via(&n, intent).starts_with("audio."),
            "{intent} 走的是 {}",
            via(&n, intent)
        );
        assert!(!r.is_system_text(), "{intent} 不需要退到 system.text");
    }
    // 安全 intent 一個都沒少。
    for intent in CharacterIntent::ALL.iter().filter(|i| i.is_safety()) {
        assert_ne!(
            res(&n, *intent).resolution,
            Resolution::Unsupported,
            "{intent} 是安全 intent，永不 unsupported"
        );
    }
    // 沒有對應聲音通道的視覺專屬 intent 仍然誠實：idle 的主要能力是 visual.presence。
    assert_eq!(
        res(&n, CharacterIntent::Idle).resolution,
        Resolution::Unsupported
    );

    // 純燈光角色：同樣能表達工作／等待／未知。
    let m = manifest_with(
        "light-only",
        &[("light.cue", CapabilityDecl::supported())],
        &all,
        Fallbacks::default(),
    );
    let n = negotiated_for(&m, false);
    for intent in [
        CharacterIntent::Work,
        CharacterIntent::Wait,
        CharacterIntent::Unknown,
    ] {
        assert_eq!(via(&n, intent), "light.cue", "{intent}");
    }

    // 沒有宣告接得住這些 intent 的角色不會被假裝支援：
    // 安全 intent 落到 system.text，非安全 intent 誠實 unsupported（見上一個測試）。
}

/// §3.4 步驟 2 的安全守衛：`fallbacks.intents` 不得把安全 intent 換成非安全 intent
/// （呈現層沒有權限主權——`request-consent`／`blocked` 不能被演成「打招呼」「玩耍」）。
#[test]
fn safety_intent_never_falls_back_to_a_non_safety_intent() {
    let mut fallbacks = Fallbacks::default();
    // 惡意／粗心的第三方 manifest：把安全語意換成愉快的日常演出。
    fallbacks
        .intents
        .insert("request-consent".into(), "greet".into());
    fallbacks.intents.insert("offline".into(), "play".into());
    fallbacks.intents.insert("unknown".into(), "greet".into());
    // 合法用法：安全 → 安全（規格 §9 的 failed → blocked 範例）。
    fallbacks.intents.insert("failed".into(), "blocked".into());
    let m = manifest_with(
        "hostile-intent-fallbacks",
        &[(
            "visual.expression",
            CapabilityDecl::supported().with_variants(["greet", "play", "blocked"]),
        )],
        &["greet", "play", "blocked"],
        fallbacks,
    );
    let n = negotiated_for(&m, false);

    for intent in CharacterIntent::ALL.iter().filter(|i| i.is_safety()) {
        let r = res(&n, *intent);
        if let Some(via_intent) = r.via_intent {
            assert!(
                via_intent.is_safety(),
                "安全 intent {intent} 不得經由非安全 intent {via_intent} 呈現"
            );
        }
    }
    // 被擋下的替換誠實降級成 system.text（安全訊息永不遺失）。
    for intent in [
        CharacterIntent::RequestConsent,
        CharacterIntent::Offline,
        CharacterIntent::Unknown,
    ] {
        let r = res(&n, intent);
        assert!(
            r.is_system_text(),
            "{intent} 應落到 system.text，實際 {r:?}"
        );
        assert_eq!(r.via_intent, None, "{intent} 不得帶 via_intent");
    }
    // 安全 → 安全仍然照走，不因守衛而過度封鎖。
    let failed = res(&n, CharacterIntent::Failed);
    assert_eq!(failed.resolution, Resolution::Substituted);
    assert_eq!(failed.via_intent, Some(CharacterIntent::Blocked));
    assert_eq!(via(&n, CharacterIntent::Failed), "visual.expression");
    // 非安全 intent 的替換不受影響。
    assert_eq!(
        res(&n, CharacterIntent::Notice).resolution,
        Resolution::Unsupported,
        "notice 沒有 fallback，維持原本行為"
    );
}
