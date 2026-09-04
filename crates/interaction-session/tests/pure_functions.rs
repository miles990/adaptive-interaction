//! 純函式層：RFC 7396 patch、state hash、接收端 revision 規則、Director 表格、CPP 投影 golden。
//!
//! 對應 `docs/aip/character-session.md` §3／§5／§6 與 AIP §6。

use chrono::{Duration, TimeZone, Utc};
use interaction_aip::{Envelope, MessageType, Party, Timestamp};
use interaction_character::CharacterIntent;
use interaction_session::director::{self, InteractionEvent};
use interaction_session::{
    accept_state, accept_state_with_epoch, apply_patch, behavior_to_cpp, merge_diff, state_hash,
    Activity, Attention, BehaviorIntent, IgnoreReason, IntentOrigin, MoodKind, RuntimeFact,
    SemanticState, SessionConfig, StateDecision,
};
use serde_json::{json, Value};

fn t0() -> Timestamp {
    Utc.with_ymd_and_hms(2026, 9, 4, 12, 30, 0)
        .single()
        .expect("fixed timestamp")
}

fn cfg() -> SessionConfig {
    SessionConfig::default()
}

fn base_state() -> SemanticState {
    SemanticState::new("ref-shape")
}

// ---------------------------------------------------------------- RFC 7396

#[test]
fn apply_patch_follows_rfc_7396() {
    let base = json!({"a": "b", "c": {"d": "e", "f": "g"}});
    let patched = apply_patch(&base, &json!({"a": "z", "c": {"f": null}}));
    assert_eq!(patched, json!({"a": "z", "c": {"d": "e"}}));

    // 非物件的 patch 直接取代。
    assert_eq!(
        apply_patch(&json!({"a": 1}), &json!("scalar")),
        json!("scalar")
    );
    // patch 是物件、base 不是物件 → base 視為空物件。
    assert_eq!(apply_patch(&json!("x"), &json!({"a": 1})), json!({"a": 1}));
    // 陣列整體取代，不做逐項合併。
    assert_eq!(
        apply_patch(&json!({"m": [1, 2, 3]}), &json!({"m": [4]})),
        json!({"m": [4]})
    );
    // 刪除不存在的鍵是 no-op。
    assert_eq!(
        apply_patch(&json!({"a": 1}), &json!({"b": null})),
        json!({"a": 1})
    );
}

#[test]
fn merge_diff_round_trips_through_apply_patch() {
    let cases: Vec<(Value, Value)> = vec![
        (json!({"a": 1}), json!({"a": 2})),
        (json!({"a": 1, "b": 2}), json!({"a": 1})),
        (
            json!({"truth": {"state": "working", "correlationId": "c1"}}),
            json!({"truth": {"state": "none"}}),
        ),
        (
            json!({"members": [{"id": "a"}]}),
            json!({"members": [{"id": "a"}, {"id": "b"}]}),
        ),
        (json!({}), json!({"x": {"y": [1, 2]}})),
    ];
    for (old, new) in cases {
        let patch = merge_diff(&old, &new);
        assert_eq!(
            apply_patch(&old, &patch),
            new,
            "merge_diff must be exactly invertible by apply_patch"
        );
    }
    // 相同 → 空 patch。
    assert_eq!(merge_diff(&json!({"a": 1}), &json!({"a": 1})), json!({}));
}

#[test]
fn state_hash_is_canonical_and_order_independent() {
    let a = json!({"b": 1, "a": {"y": 2, "x": 3}});
    let b = json!({"a": {"x": 3, "y": 2}, "b": 1});
    assert_eq!(state_hash(&a), state_hash(&b));
    assert_ne!(state_hash(&a), state_hash(&json!({"b": 2, "a": {}})));
    assert_eq!(state_hash(&a).len(), 64, "sha-256 hex");
}

// -------------------------------------------------- 接收端 revision 規則

fn state_envelope(kind: &str, revision: u64, base_revision: Option<u64>, extra: Value) -> Envelope {
    let mut payload = json!({"kind": kind, "revision": revision});
    if let (Value::Object(p), Value::Object(e)) = (&mut payload, &extra) {
        for (k, v) in e {
            p.insert(k.clone(), v.clone());
        }
    }
    let mut env = Envelope::new(
        MessageType::State,
        if kind == "patch" {
            "character.session.patch"
        } else {
            "character.session.snapshot"
        },
        Party::runtime(),
        "m1",
        t0(),
    )
    .with_session("session.home")
    .with_sequence(9)
    .with_payload(payload);
    if let Some(base) = base_revision {
        env = env.with_base_revision(base);
    }
    env
}

#[test]
fn accept_state_applies_only_contiguous_patches() {
    let patch = state_envelope("patch", 11, Some(10), json!({}));
    assert_eq!(
        accept_state(10, &patch),
        StateDecision::Apply { revision: 11 }
    );
    // baseRevision 不等於本地 → 不得套用，改 resume。
    assert_eq!(accept_state(9, &patch), StateDecision::Resume);
    // revision 回頭 → 忽略（rollback 防護）。
    let old = state_envelope("patch", 5, Some(4), json!({}));
    assert_eq!(
        accept_state(10, &old),
        StateDecision::Ignore {
            reason: IgnoreReason::Rollback
        }
    );
    let same = state_envelope("patch", 10, Some(9), json!({}));
    assert_eq!(
        accept_state(10, &same),
        StateDecision::Ignore {
            reason: IgnoreReason::AlreadyApplied
        }
    );
    // patch 少了 baseRevision → 無效。
    assert_eq!(
        accept_state(10, &state_envelope("patch", 11, None, json!({}))),
        StateDecision::Invalid
    );
}

#[test]
fn accept_state_handles_snapshots_and_session_reset() {
    let snap = state_envelope("snapshot", 12, None, json!({}));
    assert_eq!(
        accept_state(10, &snap),
        StateDecision::Apply { revision: 12 }
    );
    assert_eq!(
        accept_state(30, &snap),
        StateDecision::Ignore {
            reason: IgnoreReason::Rollback
        }
    );
    // session-reset：epoch 更大時即使 revision 較小也要接受。
    let reset = state_envelope(
        "snapshot",
        1,
        None,
        json!({"reason": "session-reset", "sessionEpoch": 7}),
    );
    assert_eq!(
        accept_state_with_epoch(30, 6, &reset),
        StateDecision::Reset { revision: 1 }
    );
    // epoch 相同 → 不是新 session，照 rollback 防護處理。
    assert_eq!(
        accept_state_with_epoch(30, 7, &reset),
        StateDecision::Ignore {
            reason: IgnoreReason::Rollback
        }
    );
    // §7 第 4 步寫的是「epoch **不同**」：host 重灌／被重新配對到另一台桌面之後，
    // 新 host 的 epoch 可能比本地記得的**小**（全新 session 的 epoch 就是 1），
    // 這一份仍然是權威快照，不得被當成 rollback 丟掉（否則手機永遠停在舊狀態）。
    let reinstalled = state_envelope(
        "snapshot",
        1,
        None,
        json!({"reason": "session-reset", "sessionEpoch": 1}),
    );
    assert_eq!(
        accept_state_with_epoch(30, 5, &reinstalled),
        StateDecision::Reset { revision: 1 }
    );
    // 沒有 `reason: session-reset` 的 snapshot 不享有這個例外（任何人都能宣稱 epoch 不同）。
    let plain = state_envelope("snapshot", 1, None, json!({"sessionEpoch": 1}));
    assert_eq!(
        accept_state_with_epoch(30, 5, &plain),
        StateDecision::Ignore {
            reason: IgnoreReason::Rollback
        }
    );
    // 不是 state 訊息 → 無效。
    let ev = Envelope::new(
        MessageType::Event,
        "character.interaction.touch",
        Party::device("d"),
        "m2",
        t0(),
    );
    assert_eq!(accept_state(1, &ev), StateDecision::Invalid);
}

// ------------------------------------------------------------- Director

fn touch(kind: &str, intensity: Option<f64>) -> InteractionEvent {
    InteractionEvent {
        name: "character.interaction.touch".into(),
        kind: kind.into(),
        intensity,
        source: Party::device("iphone-1"),
        correlation_id: "flow_1".into(),
        at: t0(),
    }
}

#[test]
fn director_touch_table() {
    let state = base_state();
    let table = [
        ("tap", MoodKind::Happy),
        ("longpress", MoodKind::Playful),
        ("pat", MoodKind::Playful),
        ("stroke", MoodKind::Playful),
    ];
    for (kind, mood) in table {
        let (patch, intents) =
            director::react(&state, &touch(kind, Some(0.4)), &cfg(), t0()).expect("handled");
        assert_eq!(patch["mood"]["kind"], json!(mood.as_str()), "kind={kind}");
        assert_eq!(patch["mood"]["intensity"], json!(0.4));
        assert_eq!(patch["activity"], json!("reacting"));
        assert_eq!(patch["attention"]["kind"], json!("member"));
        assert_eq!(patch["attention"]["id"], json!("device:iphone-1"));
        assert_eq!(patch["lastInteraction"]["kind"], json!(kind));
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].intent, "react-happily-to-touch");
        assert_eq!(intents[0].origin, IntentOrigin::Interaction);
        assert!(intents[0].interruptible);
        assert_eq!(intents[0].correlation_id, "flow_1");
        assert_eq!(
            intents[0].expires_at,
            t0() + Duration::milliseconds(cfg().intent_ttl_ms)
        );
    }
}

#[test]
fn director_clamps_intensity_and_defaults_to_half() {
    let (patch, intents) =
        director::react(&base_state(), &touch("tap", None), &cfg(), t0()).expect("handled");
    assert_eq!(patch["mood"]["intensity"], json!(0.5));
    assert_eq!(intents[0].intensity, 0.5);
    let (high, _) =
        director::react(&base_state(), &touch("tap", Some(9.0)), &cfg(), t0()).expect("handled");
    assert_eq!(high["mood"]["intensity"], json!(1.0));
    let (low, _) =
        director::react(&base_state(), &touch("tap", Some(-3.0)), &cfg(), t0()).expect("handled");
    assert_eq!(low["mood"]["intensity"], json!(0.0));
    let (nan, _) = director::react(&base_state(), &touch("tap", Some(f64::NAN)), &cfg(), t0())
        .expect("handled");
    assert_eq!(nan["mood"]["intensity"], json!(0.5), "NaN 退回預設值");
}

#[test]
fn director_dismiss_settles_and_unknown_names_are_not_handled() {
    let mut ev = touch("tap", None);
    ev.name = "character.interaction.dismiss".into();
    let (patch, intents) = director::react(&base_state(), &ev, &cfg(), t0()).expect("handled");
    assert_eq!(patch["activity"], json!("resting"));
    assert_eq!(patch["attention"]["kind"], json!("none"));
    assert_eq!(intents[0].intent, "settle");

    let mut unknown = touch("tap", None);
    unknown.name = "character.interaction.poke".into();
    assert!(director::react(&base_state(), &unknown, &cfg(), t0()).is_none());
}

#[test]
fn director_on_fact_table() {
    let state = base_state();
    // task.state：真相轉錄＋activity 對照。
    let cases = [
        (
            interaction_character::TruthState::Working,
            "working",
            Some("working"),
        ),
        (
            interaction_character::TruthState::WaitingInput,
            "waiting-input",
            Some("waiting"),
        ),
        (
            interaction_character::TruthState::WaitingConsent,
            "waiting-consent",
            Some("waiting"),
        ),
        (
            interaction_character::TruthState::None,
            "none",
            Some("idle"),
        ),
    ];
    for (truth, wire, activity) in cases {
        let fact = RuntimeFact::TaskState {
            truth,
            correlation_id: Some("c1".into()),
        };
        let (patch, intents) =
            director::on_fact(&state, &fact, None, &cfg(), t0()).expect("handled");
        assert_eq!(patch["truth"]["state"], json!(wire));
        assert_eq!(patch["truth"]["correlationId"], json!("c1"));
        if let Some(a) = activity {
            assert_eq!(patch["activity"], json!(a), "truth={wire}");
        }
        assert!(intents.is_empty(), "task.state 不產生 Behavior Intent");
    }
    // failed → mood down。
    let (patch, _) = director::on_fact(
        &state,
        &RuntimeFact::TaskState {
            truth: interaction_character::TruthState::Failed,
            correlation_id: None,
        },
        None,
        &cfg(),
        t0(),
    )
    .expect("handled");
    assert_eq!(patch["mood"]["kind"], json!("down"));
    assert_eq!(
        patch["truth"]["correlationId"],
        Value::Null,
        "沒有 correlation 時以 null 刪除該鍵"
    );
}

#[test]
fn director_verified_celebrates_and_emergency_freezes() {
    let (patch, intents) = director::on_fact(
        &base_state(),
        &RuntimeFact::TaskVerified {
            correlation_id: "c9".into(),
        },
        None,
        &cfg(),
        t0(),
    )
    .expect("handled");
    assert_eq!(patch["truth"]["state"], json!("verified"));
    assert_eq!(patch["mood"]["kind"], json!("proud"));
    assert_eq!(patch["activity"], json!("celebrating"));
    assert_eq!(patch["attention"]["correlationId"], json!("c9"));
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].intent, "celebrate");
    assert_eq!(intents[0].origin, IntentOrigin::Truth);

    let (engaged, engaged_intents) = director::on_fact(
        &base_state(),
        &RuntimeFact::Emergency { engaged: true },
        None,
        &cfg(),
        t0(),
    )
    .expect("handled");
    assert_eq!(engaged["truth"]["state"], json!("emergency"));
    assert_eq!(engaged["activity"], json!("frozen"));
    assert!(engaged_intents.is_empty(), "emergency 不得產生演出 intent");

    let (released, _) = director::on_fact(
        &base_state(),
        &RuntimeFact::Emergency { engaged: false },
        None,
        &cfg(),
        t0(),
    )
    .expect("handled");
    assert_eq!(released["truth"]["state"], json!("none"));
    assert_eq!(released["activity"], json!("idle"));

    let (rm, _) = director::on_fact(
        &base_state(),
        &RuntimeFact::ReducedMotion(true),
        None,
        &cfg(),
        t0(),
    )
    .expect("handled");
    assert_eq!(rm["reducedMotion"], json!(true));
}

#[test]
fn director_patches_are_applicable_to_the_serialized_state() {
    let state = base_state();
    let before = serde_json::to_value(&state).expect("serialize");
    let (patch, _) =
        director::react(&state, &touch("pat", Some(0.7)), &cfg(), t0()).expect("handled");
    let after: SemanticState =
        serde_json::from_value(apply_patch(&before, &patch)).expect("patched state stays valid");
    assert_eq!(after.mood().kind, MoodKind::Playful);
    assert_eq!(after.activity(), Activity::Reacting);
    assert!(matches!(after.attention(), Attention::Member { .. }));
    assert_eq!(
        after
            .last_interaction()
            .map(|i| i.kind.clone())
            .unwrap_or_default(),
        "pat"
    );
}

// ------------------------------------------------------------- CPP 投影

fn intent(name: &str, origin: IntentOrigin, intensity: f64) -> BehaviorIntent {
    BehaviorIntent {
        intent: name.into(),
        intensity,
        interruptible: true,
        origin,
        hints: serde_json::Map::new(),
        correlation_id: "flow_1".into(),
        expires_at: t0() + Duration::seconds(10),
    }
}

#[test]
fn cpp_projection_golden() {
    let play = behavior_to_cpp(&intent(
        "react-happily-to-touch",
        IntentOrigin::Interaction,
        0.45,
    ))
    .expect("projected");
    assert_eq!(play.intent, CharacterIntent::Play);
    assert_eq!(play.variant, "react-happily-to-touch");
    assert_eq!(play.parameters, json!({"intensity": 0.45}));
    assert_eq!(play.priority, 40);

    let settle =
        behavior_to_cpp(&intent("settle", IntentOrigin::Interaction, 0.3)).expect("projected");
    assert_eq!(settle.intent, CharacterIntent::Rest);
    assert_eq!(settle.variant, "settle");

    let idle = behavior_to_cpp(&intent("idle", IntentOrigin::Ambient, 0.0)).expect("projected");
    assert_eq!(idle.intent, CharacterIntent::Idle);
    assert_eq!(idle.variant, "idle");

    // celebrate 不投影：桌面已由既有 Runtime 真相投影送 verified-success，不雙播。
    assert!(behavior_to_cpp(&intent("celebrate", IntentOrigin::Truth, 1.0)).is_none());
    assert!(behavior_to_cpp(&intent("celebrate", IntentOrigin::Ambient, 1.0)).is_none());
    // 未知 intent 不猜。
    assert!(behavior_to_cpp(&intent("fly", IntentOrigin::Ambient, 1.0)).is_none());
}

#[test]
fn cpp_projection_never_drops_below_the_priority_floor() {
    for name in ["react-happily-to-touch", "settle", "idle"] {
        let p = behavior_to_cpp(&intent(name, IntentOrigin::Interaction, 0.5)).expect("projected");
        assert!(
            p.priority >= p.intent.priority_floor(),
            "{name} 的 priority 不得低於 floor"
        );
        assert!(!p.intent.is_safety(), "投影出來的 intent 不得是安全 intent");
    }
}
