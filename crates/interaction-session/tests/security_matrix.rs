//! `docs/aip/character-session.md` §8 必測清單裡尚未被 `session.rs` 涵蓋的項目：
//! 偽造 source、未配對裝置、oversized、unknown type、版本不符、renderer capability spoofing、
//! dismiss 路徑、以及「安全管線的順序真的是固定的」。

use chrono::{Duration, TimeZone, Utc};
use interaction_aip::{
    limits, CapabilityAnnouncement, Envelope, ErrorCode, IntentSupport, MemberRole, MessageType,
    Outcome, Party, SyncClass, Timestamp,
};
use interaction_session::{
    Activity, CharacterSession, Output, SessionConfig, EVENT_DISMISS, EVENT_TOUCH,
};
use serde_json::json;

const SESSION: &str = "session.home";

fn t0() -> Timestamp {
    Utc.with_ymd_and_hms(2026, 9, 4, 12, 30, 0)
        .single()
        .expect("fixed timestamp")
}

fn at(ms: i64) -> Timestamp {
    t0() + Duration::milliseconds(ms)
}

fn config() -> SessionConfig {
    SessionConfig {
        session_id: SESSION.to_string(),
        character_id: "ref-shape".to_string(),
        ..SessionConfig::default()
    }
}

fn announcement(intents: &[&str], inputs: &[&str]) -> CapabilityAnnouncement {
    CapabilityAnnouncement {
        spec_versions: vec!["aip/1.0".to_string()],
        role: Some(MemberRole::RemoteRenderer),
        profiles: vec!["character-session".to_string()],
        sync_classes: vec![SyncClass::Semantic],
        intents: intents.iter().map(|s| s.to_string()).collect(),
        inputs: inputs.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn joined_session(party: &Party) -> CharacterSession {
    let mut session = CharacterSession::new(config(), 1, t0());
    session
        .join(
            party.clone(),
            &announcement(
                &["react-happily-to-touch", "celebrate", "settle", "idle"],
                &[EVENT_TOUCH, EVENT_DISMISS],
            ),
            t0(),
        )
        .expect("join");
    session
}

fn touch(id: &str, source: &Party, now: Timestamp) -> Envelope {
    Envelope::new(MessageType::Event, EVENT_TOUCH, source.clone(), id, now)
        .with_session(SESSION)
        .with_expiry(now + Duration::seconds(5))
        .with_payload(json!({"kind": "tap"}))
}

#[test]
fn an_unpaired_device_is_never_a_member() {
    let mut session = CharacterSession::new(config(), 1, t0());
    let stranger = Party::device("unpaired");
    let submission = session.submit(touch("m1", &stranger, at(10)), &stranger, at(10));
    assert_eq!(submission.error, Some(ErrorCode::NotAMember));
    assert_eq!(session.revision(), 1, "被拒的訊息不得改變權威狀態");
    assert!(session.state().members().is_empty());
}

#[test]
fn oversized_payloads_are_rejected_before_anything_is_applied() {
    let member = Party::device("iphone-1");
    let mut session = joined_session(&member);
    let revision = session.revision();
    let big = touch("m1", &member, at(10))
        .with_payload(json!({"kind": "tap", "blob": "x".repeat(limits::MAX_PAYLOAD_BYTES)}));
    let submission = session.submit(big, &member, at(10));
    assert_eq!(submission.error, Some(ErrorCode::PayloadTooLarge));
    assert_eq!(session.revision(), revision);

    let deep = touch("m2", &member, at(20))
        .with_payload(json!({"kind":"tap","a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":1}}}}}}}}}));
    assert_eq!(
        session.submit(deep, &member, at(20)).error,
        Some(ErrorCode::SchemaInvalid)
    );

    let long_string = touch("m3", &member, at(30))
        .with_payload(json!({"kind": "tap", "note": "n".repeat(limits::MAX_STRING_CHARS + 1)}));
    assert_eq!(
        session.submit(long_string, &member, at(30)).error,
        Some(ErrorCode::SchemaInvalid)
    );
    assert_eq!(session.revision(), revision);
}

#[test]
fn unknown_message_types_and_versions_are_not_executed() {
    let member = Party::device("iphone-1");
    let mut session = joined_session(&member);

    let raw = json!({
        "specVersion": "aip/1.0", "messageId": "m1", "messageType": "teleport",
        "name": "character.interaction.touch", "source": {"kind": "device", "id": "iphone-1"},
        "sessionId": SESSION, "occurredAt": "2026-09-04T12:30:00Z", "payload": {"kind": "tap"}
    });
    let bytes = serde_json::to_vec(&raw).expect("serialize");
    let envelope = Envelope::parse(&bytes).expect("parses as an envelope");
    let submission = session.submit(envelope, &member, at(10));
    assert_eq!(submission.error, Some(ErrorCode::UnsupportedMessageType));

    let mut future = touch("m2", &member, at(20));
    future.spec_version = "aip/2.0".into();
    assert_eq!(
        session.submit(future, &member, at(20)).error,
        Some(ErrorCode::UnsupportedVersion)
    );
    assert_eq!(session.revision(), 2, "都沒有被套用");
}

#[test]
fn renderer_capability_spoofing_only_earns_unsupported() {
    let honest = Party::device("iphone-1");
    let mut session = joined_session(&honest);
    let liar = Party::renderer("liar");
    let outcome = session
        .join(
            liar.clone(),
            &announcement(
                &["mind-control", "react-happily-to-touch"],
                &["task.verified"],
            ),
            at(10),
        )
        .expect("join");
    // host 只協商自己提供的 intent；對方發明的 intent 根本不會出現。
    assert!(!outcome.negotiated.intents.contains_key("mind-control"));
    assert_eq!(
        outcome.negotiated.intents["react-happily-to-touch"],
        IntentSupport::Exact
    );
    // host 不接受的 input 進 unsupported，不進 inputs。
    assert!(outcome.negotiated.inputs.is_empty());
    assert_eq!(
        outcome.negotiated.unsupported_inputs,
        vec!["task.verified".to_string()]
    );
    // 而且不影響誠實的成員。
    let submission = session.submit(touch("m1", &honest, at(20)), &honest, at(20));
    assert_eq!(submission.outcome, Outcome::Applied);
    // 說謊者仍然不能送 task.verified。
    let forged = Envelope::new(
        MessageType::Event,
        "task.verified",
        liar.clone(),
        "m2",
        at(30),
    )
    .with_session(SESSION)
    .with_payload(json!({"correlationId": "c1"}));
    assert_eq!(
        session.submit(forged, &liar, at(30)).error,
        Some(ErrorCode::ScopeDenied)
    );
}

#[test]
fn dismiss_settles_the_character() {
    let member = Party::device("iphone-1");
    let mut session = joined_session(&member);
    session.submit(touch("m1", &member, at(10)), &member, at(10));
    assert_eq!(session.state().activity(), Activity::Reacting);

    let dismiss = Envelope::new(
        MessageType::Event,
        EVENT_DISMISS,
        member.clone(),
        "m2",
        at(20),
    )
    .with_session(SESSION)
    .with_expiry(at(5_020))
    .with_payload(json!({}));
    let submission = session.submit(dismiss, &member, at(20));
    assert_eq!(submission.outcome, Outcome::Applied);
    assert_eq!(session.state().activity(), Activity::Resting);
    assert_eq!(
        session
            .state()
            .last_interaction()
            .map(|i| i.name.clone())
            .unwrap_or_default(),
        EVENT_DISMISS
    );
    let sent: Vec<&Envelope> = submission
        .outputs
        .iter()
        .filter_map(|o| match o {
            Output::Send { envelope, .. } => Some(envelope),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].payload["intent"], json!("settle"));
}

#[test]
fn the_pipeline_order_is_fixed_identity_before_membership_before_scope() {
    let member = Party::device("iphone-1");
    let mut session = joined_session(&member);
    // 同一則訊息同時違反身分、membership 與 scope：先回報身分不符（管線最前面的那一關）。
    let hostile = Envelope::new(
        MessageType::Event,
        "task.verified",
        Party::device("someone-else"),
        "m1",
        at(10),
    )
    .with_session("session.other")
    .with_payload(json!({}));
    assert_eq!(
        session.submit(hostile, &member, at(10)).error,
        Some(ErrorCode::IdentityMismatch)
    );

    // 身分對了但跨 session＋runtime-only name：先回報 not-a-member（sessionId 那一關在 scope 之前）。
    let cross = Envelope::new(
        MessageType::Event,
        "task.verified",
        member.clone(),
        "m2",
        at(20),
    )
    .with_session("session.other")
    .with_payload(json!({}));
    assert_eq!(
        session.submit(cross, &member, at(20)).error,
        Some(ErrorCode::NotAMember)
    );

    // schema 壞掉時最先回報 schema-invalid，連身分都還沒比對。
    let mut broken = touch("m3", &Party::device("someone-else"), at(30));
    broken.name = "NotAValidName".into();
    assert_eq!(
        session.submit(broken, &member, at(30)).error,
        Some(ErrorCode::SchemaInvalid)
    );
}

#[test]
fn every_result_envelope_validates_and_never_claims_verified() {
    let member = Party::device("iphone-1");
    let mut session = joined_session(&member);
    let cases: Vec<Envelope> = vec![
        touch("ok", &member, at(10)),
        touch("expired", &member, at(10)).with_expiry(at(5)),
        touch("forged", &Party::device("other"), at(20)),
        touch("dup", &member, at(30)),
        touch("dup", &member, at(31)),
        Envelope::new(
            MessageType::Event,
            EVENT_TOUCH,
            member.clone(),
            "bad-kind",
            at(40),
        )
        .with_session(SESSION)
        .with_expiry(at(5_040))
        .with_payload(json!({"kind": "headbutt"})),
    ];
    for envelope in cases {
        let submission = session.submit(envelope, &member, at(50));
        submission
            .result
            .validate()
            .expect("every result envelope must validate");
        assert_ne!(
            submission.result.payload["status"],
            json!("verified"),
            "Session 不得對外部訊息宣稱 verified"
        );
        assert_eq!(submission.result.source, Party::runtime());
    }
}
