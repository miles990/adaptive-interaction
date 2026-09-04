//! `docs/aip/character-session.md` 的 Session 驗收：join／leave、snapshot、事件套用、sequence、
//! 去重、亂序、delta replay、snapshot fallback、deadline、presence、撤銷、跨 session、renderer
//! 不可用、能力降級、身分不符、scope、emergency、restore、hash、有界集合、rate limit。

use chrono::{Duration, TimeZone, Utc};
use interaction_aip::{
    CapabilityAnnouncement, Envelope, ErrorCode, IntentSupport, MemberRole, MessageType, Outcome,
    Party, SyncClass, Timestamp,
};
use interaction_character::TruthState;
use interaction_session::ports::{MemoryStore, SessionStore};
use interaction_session::{
    accept_state, apply_patch, state_hash, Activity, CharacterSession, MoodKind, Output, Presence,
    RuntimeFact, SessionConfig, SessionError, Snapshot, StateDecision, EVENT_DISMISS, EVENT_TOUCH,
};
use serde_json::{json, Value};

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

fn session() -> CharacterSession {
    CharacterSession::new(config(), 1, t0())
}

fn announcement(role: MemberRole, intents: &[&str], inputs: &[&str]) -> CapabilityAnnouncement {
    CapabilityAnnouncement {
        spec_versions: vec!["aip/1.0".to_string()],
        role: Some(role),
        profiles: vec!["character-session".to_string()],
        sync_classes: vec![SyncClass::Semantic],
        intents: intents.iter().map(|s| s.to_string()).collect(),
        inputs: inputs.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn device_announcement() -> CapabilityAnnouncement {
    announcement(
        MemberRole::RemoteRenderer,
        &["react-happily-to-touch", "celebrate", "settle", "idle"],
        &[EVENT_TOUCH, EVENT_DISMISS],
    )
}

fn touch(id: &str, source: &Party, now: Timestamp) -> Envelope {
    Envelope::new(MessageType::Event, EVENT_TOUCH, source.clone(), id, now)
        .with_session(SESSION)
        .with_expiry(now + Duration::seconds(5))
        .with_payload(json!({"kind": "tap", "intensity": 0.4}))
}

/// 每一則要送上線的 envelope 都必須 `validate()` 通過。
fn assert_outputs_valid(outputs: &[Output]) {
    for output in outputs {
        match output {
            Output::Send { envelope, .. } | Output::Broadcast { envelope, .. } => envelope
                .validate()
                .unwrap_or_else(|e| panic!("output envelope must validate: {e}")),
            Output::Persist(snapshot) => {
                assert_eq!(state_hash(&snapshot.state), snapshot.hash);
            }
            Output::Audit { detail, .. } => {
                let text = serde_json::to_string(detail).expect("audit detail serializes");
                assert!(!text.contains('/'), "稽核不得含路徑：{text}");
            }
            Output::RendererIntent { .. } => {}
        }
    }
}

fn sequences(outputs: &[Output]) -> Vec<u64> {
    outputs
        .iter()
        .filter_map(|o| match o {
            Output::Send { envelope, .. } | Output::Broadcast { envelope, .. } => envelope.sequence,
            _ => None,
        })
        .collect()
}

fn sends(outputs: &[Output]) -> Vec<(&Party, &Envelope)> {
    outputs
        .iter()
        .filter_map(|o| match o {
            Output::Send { to, envelope } => Some((to, envelope)),
            _ => None,
        })
        .collect()
}

fn broadcasts(outputs: &[Output]) -> Vec<&Envelope> {
    outputs
        .iter()
        .filter_map(|o| match o {
            Output::Broadcast { envelope, .. } => Some(envelope),
            _ => None,
        })
        .collect()
}

fn renderer_intents(outputs: &[Output]) -> Vec<&interaction_session::BehaviorIntent> {
    outputs
        .iter()
        .filter_map(|o| match o {
            Output::RendererIntent { intent, .. } => Some(intent),
            _ => None,
        })
        .collect()
}

fn join(
    session: &mut CharacterSession,
    party: &Party,
    ann: &CapabilityAnnouncement,
    now: Timestamp,
) {
    let outcome = session
        .join(party.clone(), ann, now)
        .expect("join should succeed");
    outcome
        .capability_envelope
        .validate()
        .expect("capability envelope validates");
    outcome
        .snapshot_envelope
        .validate()
        .expect("snapshot envelope validates");
    assert_outputs_valid(&outcome.outputs);
}

// ------------------------------------------------------------ join／leave

#[test]
fn initial_snapshot_is_empty_and_hashed() {
    let session = session();
    let snapshot = session.snapshot();
    assert_eq!(snapshot.session_id, SESSION);
    assert_eq!(snapshot.epoch, 1);
    assert_eq!(snapshot.revision, 1, "revision 從 1 起");
    assert_eq!(snapshot.sequence, 0, "還沒送過任何訊息");
    assert_eq!(snapshot.hash, state_hash(&snapshot.state));
    assert_eq!(snapshot.state["activity"], json!("idle"));
    assert_eq!(snapshot.state["truth"], json!({"state": "none"}));
    assert_eq!(snapshot.state["members"], json!([]));
    assert_eq!(session.state().mood().kind, MoodKind::Neutral);
    assert!(session.event_log().is_empty());
}

#[test]
fn join_negotiates_broadcasts_and_snapshots() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    let outcome = session
        .join(phone.clone(), &device_announcement(), t0())
        .expect("join");
    assert_eq!(outcome.negotiated.role, MemberRole::RemoteRenderer);
    assert_eq!(
        outcome.negotiated.intents["react-happily-to-touch"],
        IntentSupport::Exact
    );
    assert_eq!(outcome.negotiated.inputs.len(), 2);
    assert_eq!(session.revision(), 2, "成員變動 = 一次 applied");
    // patch 廣播不送給剛拿到 snapshot 的人。
    let patch = broadcasts(&outcome.outputs);
    assert_eq!(patch.len(), 1);
    assert_eq!(patch[0].payload["kind"], json!("patch"));
    assert!(matches!(
        &outcome.outputs[0],
        Output::Broadcast { except: Some(p), .. } if p == &phone
    ));
    // snapshot 的 sequence 在 patch 之後。
    assert_eq!(patch[0].sequence, Some(1));
    assert_eq!(outcome.snapshot_envelope.sequence, Some(2));
    assert_eq!(outcome.snapshot_envelope.payload["revision"], json!(2));
    assert_eq!(
        outcome.snapshot_envelope.payload["hash"],
        json!(session.snapshot().hash)
    );
    assert_eq!(session.state().members().len(), 1);
    assert_eq!(session.state().members()[0].presence, Presence::Online);
    assert_outputs_valid(&outcome.outputs);
}

#[test]
fn rejoin_renegotiates_instead_of_duplicating_the_member() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let outcome = session
        .join(
            phone.clone(),
            &announcement(MemberRole::RemoteRenderer, &["idle"], &[EVENT_TOUCH]),
            at(1_000),
        )
        .expect("rejoin");
    assert_eq!(
        session.state().members().len(),
        1,
        "重複 join 不會多一個成員"
    );
    assert_eq!(
        outcome.negotiated.intents["react-happily-to-touch"],
        IntentSupport::Unsupported,
        "重新協商會收回先前宣告的能力"
    );
    assert_eq!(outcome.negotiated.inputs, vec![EVENT_TOUCH.to_string()]);
}

#[test]
fn leave_is_idempotent_and_revokes_membership() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let outputs = session.leave(&phone, at(1_000));
    assert_eq!(broadcasts(&outputs).len(), 1);
    assert!(session.state().members().is_empty());
    assert_outputs_valid(&outputs);
    // 冪等：再 leave 一次沒有輸出。
    assert!(session.leave(&phone, at(1_100)).is_empty());
    // 撤銷後再送事件 → not-a-member。
    let submission = session.submit(touch("m1", &phone, at(1_200)), &phone, at(1_200));
    assert_eq!(submission.outcome, Outcome::Rejected);
    assert_eq!(submission.error, Some(ErrorCode::NotAMember));
    assert!(submission.reply, "被拒絕的訊息一定要回");
}

#[test]
fn member_cap_is_enforced() {
    let mut session = CharacterSession::new(
        SessionConfig {
            max_members: 2,
            ..config()
        },
        1,
        t0(),
    );
    join(
        &mut session,
        &Party::device("a"),
        &device_announcement(),
        t0(),
    );
    join(
        &mut session,
        &Party::device("b"),
        &device_announcement(),
        t0(),
    );
    let err = session
        .join(Party::device("c"), &device_announcement(), t0())
        .expect_err("third member must be refused");
    assert_eq!(err, SessionError::MembersFull);
    assert_eq!(session.state().members().len(), 2);
}

// -------------------------------------------------------- 事件套用與輸出

#[test]
fn touch_applies_state_intent_and_a_single_result() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let before = session.revision();
    let submission = session.submit(touch("m1", &phone, at(100)), &phone, at(100));

    assert_eq!(submission.outcome, Outcome::Applied);
    assert_eq!(submission.error, None);
    assert!(submission.reply);
    submission.result.validate().expect("result validates");
    assert_eq!(submission.result.message_type, MessageType::Result);
    assert_eq!(submission.result.causation_id.as_deref(), Some("m1"));
    assert_eq!(submission.result.payload["status"], json!("applied"));

    assert_eq!(session.revision(), before + 1);
    assert_eq!(session.state().mood().kind, MoodKind::Happy);
    assert_eq!(session.state().mood().intensity, 0.4);
    assert_eq!(session.state().activity(), Activity::Reacting);

    let patches = broadcasts(&submission.outputs);
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].payload["kind"], json!("patch"));
    assert_eq!(patches[0].base_revision, Some(before));
    assert_eq!(patches[0].payload["revision"], json!(before + 1));
    assert_eq!(patches[0].payload["hash"], json!(session.snapshot().hash));

    let sent = sends(&submission.outputs);
    assert_eq!(sent.len(), 1, "協商過的 remote renderer 收到 command");
    assert_eq!(sent[0].0, &phone);
    assert_eq!(sent[0].1.message_type, MessageType::Command);
    assert_eq!(sent[0].1.name, "character.behavior.request");
    assert_eq!(sent[0].1.payload["intent"], json!("react-happily-to-touch"));
    assert_eq!(sent[0].1.correlation_id.as_deref(), Some("m1"));
    assert_eq!(
        sent[0].1.expires_at,
        Some(at(100) + Duration::milliseconds(session.config().intent_ttl_ms))
    );

    let intents = renderer_intents(&submission.outputs);
    assert_eq!(intents.len(), 1, "host 端 renderer 一定拿得到 intent");
    assert_eq!(intents[0].intent, "react-happily-to-touch");
    assert_outputs_valid(&submission.outputs);
}

#[test]
fn broadcast_patch_reproduces_the_authoritative_state() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let mut local = session.snapshot().state;
    let mut local_revision = session.revision();

    for (index, now) in [at(100), at(4_000), at(8_000)].into_iter().enumerate() {
        let submission = session.submit(touch(&format!("m{index}"), &phone, now), &phone, now);
        assert_eq!(submission.outcome, Outcome::Applied);
        for envelope in broadcasts(&submission.outputs) {
            assert_eq!(
                accept_state(local_revision, envelope),
                StateDecision::Apply {
                    revision: local_revision + 1
                }
            );
            local = apply_patch(&local, &envelope.payload["patch"]);
            local_revision += 1;
            assert_eq!(
                state_hash(&local),
                envelope.payload["hash"].as_str().unwrap_or_default(),
                "套用 patch 後本地 hash 必須等於 host 的 hash"
            );
        }
    }
    assert_eq!(local, session.snapshot().state);
    assert_eq!(local_revision, session.revision());
}

#[test]
fn sequence_is_monotonic_without_holes() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    let desk = Party::renderer("desktop");
    let mut seen: Vec<u64> = Vec::new();

    let joined = session
        .join(phone.clone(), &device_announcement(), t0())
        .expect("join");
    seen.extend(sequences(&joined.outputs));
    seen.extend(joined.snapshot_envelope.sequence);
    let joined = session
        .join(desk.clone(), &device_announcement(), at(10))
        .expect("join");
    seen.extend(sequences(&joined.outputs));
    seen.extend(joined.snapshot_envelope.sequence);

    for i in 0..3 {
        let now = at(100 + i * 4_000);
        let submission = session.submit(touch(&format!("m{i}"), &phone, now), &phone, now);
        seen.extend(sequences(&submission.outputs));
    }
    seen.extend(sequences(&session.tick(at(20_000))));
    let snapshot = session.snapshot_envelope(&phone, at(21_000));
    seen.extend(snapshot.sequence);

    assert!(!seen.is_empty());
    assert_eq!(
        seen,
        (1..=seen.len() as u64).collect::<Vec<u64>>(),
        "host 送出的 sequence 必須單調且無洞：{seen:?}"
    );
    assert_eq!(session.sequence(), *seen.last().unwrap_or(&0));
}

// -------------------------------------------------------- 去重／亂序／deadline

#[test]
fn duplicate_message_id_is_accepted_once_and_never_reapplied() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let envelope = touch("m1", &phone, at(100));
    let first = session.submit(envelope.clone(), &phone, at(100));
    assert_eq!(first.outcome, Outcome::Applied);
    let revision = session.revision();

    let second = session.submit(envelope, &phone, at(200));
    assert_eq!(second.outcome, Outcome::Accepted);
    assert_eq!(second.result.payload["duplicate"], json!(true));
    assert_eq!(session.revision(), revision, "重複訊息不得重套用");
    assert!(broadcasts(&second.outputs).is_empty());
    assert_eq!(session.diagnostics().counters.get("duplicates"), Some(&1));
}

#[test]
fn out_of_order_delivery_cannot_replay_an_old_touch() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    // 較新的先到、較舊的後到：兩則都是新的 messageId，都會套用（host 以自己的時鐘為準）。
    let newer = touch("m2", &phone, at(2_000));
    let older = touch("m1", &phone, at(100));
    assert_eq!(
        session.submit(newer.clone(), &phone, at(2_000)).outcome,
        Outcome::Applied
    );
    assert_eq!(
        session.submit(older.clone(), &phone, at(2_010)).outcome,
        Outcome::Applied
    );
    let revision = session.revision();
    // 重放：同一個 messageId 再送一次 → duplicate，不重套用。
    assert_eq!(
        session.submit(older, &phone, at(2_020)).outcome,
        Outcome::Accepted
    );
    assert_eq!(session.revision(), revision, "重複的 messageId 不改狀態");
    let interaction = session.state().last_interaction().cloned();
    // 真正的舊事件（deadline 已過）→ expired，不執行。
    let stale = touch("m3", &phone, at(100));
    let submission = session.submit(stale, &phone, at(60_000));
    assert_eq!(submission.outcome, Outcome::Expired);
    assert_eq!(submission.error, Some(ErrorCode::Expired));
    // 過期的訊息不套用任何語意；它仍然是存活證明，所以 `lastSeenAt`（成員投影）
    // 會前進——這是刻意的，否則手機下一個 tick 就被誤判成離線。
    assert_eq!(session.state().last_interaction().cloned(), interaction);
    assert_eq!(session.state().activity(), Activity::Reacting);
    assert_eq!(session.diagnostics().counters.get("expired"), Some(&1));
}

#[test]
fn rate_limit_is_a_token_bucket_on_injected_time() {
    let mut session = CharacterSession::new(
        SessionConfig {
            rate_limit_per_sec: 2,
            ..config()
        },
        1,
        t0(),
    );
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    assert_eq!(
        session
            .submit(touch("m1", &phone, at(10)), &phone, at(10))
            .outcome,
        Outcome::Applied
    );
    assert_eq!(
        session
            .submit(touch("m2", &phone, at(11)), &phone, at(11))
            .outcome,
        Outcome::Applied
    );
    let limited = session.submit(touch("m3", &phone, at(12)), &phone, at(12));
    assert_eq!(limited.error, Some(ErrorCode::RateLimited));
    assert!(
        limited.result.payload["retryable"] == json!(true),
        "rate-limited 可以用同一個 messageId 重送"
    );
    // 一秒後補回 token。
    assert_eq!(
        session
            .submit(touch("m4", &phone, at(1_100)), &phone, at(1_100))
            .outcome,
        Outcome::Applied
    );
}

// ------------------------------------------------------------------ 安全

#[test]
fn identity_mismatch_is_rejected_and_audited() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let forged = touch("m1", &Party::device("iphone-2"), at(100));
    let submission = session.submit(forged, &phone, at(100));
    assert_eq!(submission.error, Some(ErrorCode::IdentityMismatch));
    assert!(submission.outputs.iter().any(|o| matches!(
        o,
        Output::Audit { kind, .. } if kind == "aip.identity-mismatch"
    )));
    assert_eq!(
        session.diagnostics().counters.get("identity_mismatch"),
        Some(&1)
    );
    // 宣稱自己是 runtime 也一樣。
    let impersonation = Envelope::new(
        MessageType::Event,
        EVENT_TOUCH,
        Party::runtime(),
        "m2",
        at(200),
    )
    .with_session(SESSION)
    .with_expiry(at(5_200))
    .with_payload(json!({"kind": "tap"}));
    let submission = session.submit(impersonation, &Party::runtime(), at(200));
    assert_eq!(submission.error, Some(ErrorCode::IdentityMismatch));
}

#[test]
fn cross_session_injection_is_not_a_member() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let other = Envelope::new(
        MessageType::Event,
        EVENT_TOUCH,
        phone.clone(),
        "m1",
        at(100),
    )
    .with_session("session.other")
    .with_expiry(at(5_100))
    .with_payload(json!({"kind": "tap"}));
    let submission = session.submit(other, &phone, at(100));
    assert_eq!(submission.error, Some(ErrorCode::NotAMember));
}

#[test]
fn devices_may_not_produce_runtime_truth_or_verified() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());

    let forged_truth = Envelope::new(
        MessageType::Event,
        "task.verified",
        phone.clone(),
        "m1",
        at(100),
    )
    .with_session(SESSION)
    .with_payload(json!({"correlationId": "c1"}));
    let submission = session.submit(forged_truth, &phone, at(100));
    assert_eq!(submission.error, Some(ErrorCode::ScopeDenied));
    assert_eq!(session.state().truth().state, TruthState::None);

    let forged_verified = Envelope::new(
        MessageType::Result,
        "character.behavior.request",
        phone.clone(),
        "m2",
        at(200),
    )
    .with_session(SESSION)
    .with_causation("cmd-1")
    .with_payload(json!({"status": "verified"}));
    let submission = session.submit(forged_verified, &phone, at(200));
    assert_eq!(submission.error, Some(ErrorCode::ScopeDenied));

    // 成員也不能自己送 state／command。
    let forged_state = Envelope::new(
        MessageType::State,
        "character.session.patch",
        phone.clone(),
        "m3",
        at(300),
    )
    .with_session(SESSION)
    .with_sequence(99)
    .with_base_revision(1)
    .with_payload(json!({"kind": "patch", "revision": 99, "patch": {}, "hash": "x"}));
    assert_eq!(
        session.submit(forged_state, &phone, at(300)).error,
        Some(ErrorCode::ScopeDenied)
    );
}

#[test]
fn undeclared_inputs_are_scope_denied() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    // 只宣告 touch，沒有宣告 dismiss。
    join(
        &mut session,
        &phone,
        &announcement(MemberRole::InputDevice, &[], &[EVENT_TOUCH]),
        t0(),
    );
    let dismiss = Envelope::new(
        MessageType::Event,
        EVENT_DISMISS,
        phone.clone(),
        "m1",
        at(100),
    )
    .with_session(SESSION)
    .with_expiry(at(5_100))
    .with_payload(json!({}));
    assert_eq!(
        session.submit(dismiss, &phone, at(100)).error,
        Some(ErrorCode::ScopeDenied)
    );
}

#[test]
fn unknown_touch_kind_is_schema_invalid_and_never_echoed() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let weird = Envelope::new(
        MessageType::Event,
        EVENT_TOUCH,
        phone.clone(),
        "m1",
        at(100),
    )
    .with_session(SESSION)
    .with_expiry(at(5_100))
    .with_payload(json!({"kind": "headbutt-do-not-echo"}));
    let submission = session.submit(weird, &phone, at(100));
    assert_eq!(submission.error, Some(ErrorCode::SchemaInvalid));
    let audit = serde_json::to_string(&submission.outputs).unwrap_or_default();
    assert!(
        !audit.contains("headbutt-do-not-echo"),
        "稽核不得回顯輸入內容"
    );
}

#[test]
fn emergency_freezes_the_character_and_refuses_interaction() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    // 先製造一個 pending intent。
    session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    assert_eq!(session.pending_intents().len(), 1);

    let outputs = session.submit_runtime(RuntimeFact::Emergency { engaged: true }, None, at(200));
    assert_eq!(session.state().truth().state, TruthState::Emergency);
    assert_eq!(session.state().activity(), Activity::Frozen);
    assert!(
        session.pending_intents().is_empty(),
        "emergency 取消所有 pending intent"
    );
    assert!(outputs.iter().any(|o| matches!(
        o,
        Output::Audit { kind, .. } if kind == "character.session.emergency"
    )));
    assert_outputs_valid(&outputs);

    let blocked = session.submit(touch("m2", &phone, at(300)), &phone, at(300));
    assert_eq!(blocked.outcome, Outcome::Rejected);
    assert_eq!(blocked.error, Some(ErrorCode::ScopeDenied));
    assert_eq!(session.state().activity(), Activity::Frozen);

    // 解除後恢復。
    session.submit_runtime(RuntimeFact::Emergency { engaged: false }, None, at(400));
    assert_eq!(session.state().truth().state, TruthState::None);
    assert_eq!(session.state().activity(), Activity::Idle);
    assert_eq!(
        session
            .submit(touch("m3", &phone, at(500)), &phone, at(500))
            .outcome,
        Outcome::Applied
    );
}

// --------------------------------------------------------- renderer 能力

#[test]
fn renderer_unavailable_still_produces_a_host_renderer_intent() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    // 純輸入裝置：不宣告任何 intent。
    join(
        &mut session,
        &phone,
        &announcement(MemberRole::InputDevice, &[], &[EVENT_TOUCH]),
        t0(),
    );
    let submission = session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    assert_eq!(submission.outcome, Outcome::Applied);
    assert!(
        sends(&submission.outputs).is_empty(),
        "沒有 renderer 成員就不送 command"
    );
    assert_eq!(renderer_intents(&submission.outputs).len(), 1);
}

#[test]
fn capability_degradation_skips_renderers_that_did_not_declare_the_intent() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    let weak = Party::renderer("text-only");
    join(
        &mut session,
        &phone,
        &announcement(MemberRole::InputDevice, &[], &[EVENT_TOUCH]),
        t0(),
    );
    let outcome = session
        .join(
            weak.clone(),
            &announcement(MemberRole::RemoteRenderer, &["idle"], &[]),
            at(10),
        )
        .expect("join");
    assert_eq!(
        outcome.negotiated.intents["react-happily-to-touch"],
        IntentSupport::Unsupported
    );
    assert_eq!(outcome.negotiated.intents["idle"], IntentSupport::Exact);

    let submission = session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    assert!(
        sends(&submission.outputs).is_empty(),
        "未宣告該 intent 的 renderer 不會收到 command"
    );
    assert_eq!(renderer_intents(&submission.outputs).len(), 1);
}

#[test]
fn offline_renderers_do_not_receive_intents() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    let desktop = Party::human_surface("desktop");
    join(&mut session, &phone, &device_announcement(), t0());
    join(
        &mut session,
        &desktop,
        &announcement(
            MemberRole::HostRenderer,
            &["react-happily-to-touch", "celebrate", "settle", "idle"],
            &[EVENT_TOUCH, EVENT_DISMISS],
        ),
        t0(),
    );
    session.presence(&phone, Presence::Offline, at(50));
    // 互動由**桌面**送出：手機還是離線的（它自己送訊息就會證明自己還在，
    // 那是另一條規則），所以它不得收到 Behavior Intent。
    let submission = session.submit(touch("m1", &desktop, at(100)), &desktop, at(100));
    assert_eq!(submission.outcome, Outcome::Applied);
    assert!(
        sends(&submission.outputs).is_empty(),
        "character.behavior.* 是 drop-if-offline，不排隊"
    );
    assert_eq!(
        session
            .members()
            .iter()
            .find(|m| m.party == phone)
            .map(|m| m.presence),
        Some(Presence::Offline),
        "別人送的訊息不會替離線的裝置作證"
    );
}

// --------------------------------------------------- 真相事實與 CPP 投影

#[test]
fn task_verified_celebrates_without_a_cpp_projection() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let outputs = session.submit_runtime(
        RuntimeFact::TaskVerified {
            correlation_id: "flow_9".into(),
        },
        None,
        at(100),
    );
    assert_eq!(session.state().truth().state, TruthState::Verified);
    assert_eq!(session.state().mood().kind, MoodKind::Proud);
    assert_eq!(session.state().activity(), Activity::Celebrating);
    let sent = sends(&outputs);
    assert_eq!(
        sent.len(),
        1,
        "iPhone 沒有既有真相投影，celebrate 直接送給它"
    );
    assert_eq!(sent[0].1.payload["intent"], json!("celebrate"));
    let projected = outputs.iter().find_map(|o| match o {
        Output::RendererIntent { cpp, .. } => Some(cpp),
        _ => None,
    });
    assert_eq!(
        projected,
        Some(&None),
        "桌面已有 verified-success 投影，celebrate 不得雙播"
    );
    assert_outputs_valid(&outputs);
}

#[test]
fn task_state_only_transcribes_truth() {
    let mut session = session();
    let outputs = session.submit_runtime(
        RuntimeFact::TaskState {
            truth: TruthState::Working,
            correlation_id: Some("flow_1".into()),
        },
        None,
        at(100),
    );
    assert_eq!(session.state().truth().state, TruthState::Working);
    assert_eq!(session.state().activity(), Activity::Working);
    assert!(
        renderer_intents(&outputs).is_empty(),
        "task.state 不產生演出"
    );
    session.submit_runtime(
        RuntimeFact::TaskState {
            truth: TruthState::Failed,
            correlation_id: Some("flow_1".into()),
        },
        None,
        at(200),
    );
    assert_eq!(session.state().mood().kind, MoodKind::Down);
    assert_eq!(session.state().truth().state, TruthState::Failed);
}

#[test]
fn reduced_motion_is_shared_state() {
    let mut session = session();
    session.submit_runtime(RuntimeFact::ReducedMotion(true), None, at(100));
    assert!(session.state().reduced_motion());
    assert_eq!(session.snapshot().state["reducedMotion"], json!(true));
}

// ------------------------------------------------------------ tick／presence

#[test]
fn reacting_returns_to_idle_after_the_reaction_window() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    assert_eq!(session.state().activity(), Activity::Reacting);
    // 還沒到 reaction_ms。
    assert!(broadcasts(&session.tick(at(1_000))).is_empty());
    assert_eq!(session.state().activity(), Activity::Reacting);
    let outputs = session.tick(at(100 + session.config().reaction_ms));
    assert_eq!(session.state().activity(), Activity::Idle);
    assert_eq!(broadcasts(&outputs).len(), 1);
    assert_outputs_valid(&outputs);
}

#[test]
fn presence_times_out_and_expired_intents_are_dropped() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    assert_eq!(session.pending_intents().len(), 1);

    let timeout = session.config().presence_timeout_ms;
    let outputs = session.tick(at(timeout + 1_000));
    assert_eq!(session.state().members()[0].presence, Presence::Offline);
    assert!(session.pending_intents().is_empty(), "過期 intent 被清掉");
    assert!(outputs.iter().any(|o| matches!(
        o,
        Output::Audit { kind, .. } if kind == "character.session.intent-expired"
    )));
    assert_outputs_valid(&outputs);
}

#[test]
fn heartbeats_do_not_flood_revisions() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let start = session.revision();
    for i in 0..20 {
        let beat = Envelope::new(
            MessageType::Heartbeat,
            "character.session.heartbeat",
            phone.clone(),
            format!("hb{i}"),
            at(100 + i * 100),
        )
        .with_session(SESSION);
        let submission = session.submit(beat, &phone, at(100 + i * 100));
        assert_eq!(submission.outcome, Outcome::Applied);
        assert!(!submission.reply, "heartbeat 不回 result");
    }
    assert!(
        session.revision() - start <= 2,
        "heartbeat 不得把 revision 打成無界成長（實際 {}）",
        session.revision() - start
    );
    assert_eq!(session.state().members()[0].presence, Presence::Online);
}

/// 存活證明不只是 heartbeat：只送 touch、從不送 heartbeat 的裝置跨過 presence timeout
/// 之後仍必須是 online。否則它會被標成離線，再被 host 的 stale 清除踢出成員，
/// 之後每一則 touch 都變成 `not-a-member`。
#[test]
fn events_alone_keep_a_member_online_across_the_presence_timeout() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let timeout = session.config().presence_timeout_ms;

    let mut now = 0i64;
    while now < timeout * 2 {
        now += 5_000;
        let submission =
            session.submit(touch(&format!("m-{now}"), &phone, at(now)), &phone, at(now));
        assert_eq!(
            submission.outcome,
            Outcome::Applied,
            "只送 event 的成員必須一直是成員（{now} ms）"
        );
        session.tick(at(now));
        assert_eq!(
            session.state().members().len(),
            1,
            "成員不得在 {now} ms 被清掉"
        );
        assert_eq!(
            session.state().members()[0].presence,
            Presence::Online,
            "已驗證的 inbound 訊息就是存活證明（{now} ms）"
        );
    }
}

/// 被拒絕／過期的訊息一樣是存活證明：對方還在，只是這一則不合法。
#[test]
fn a_rejected_message_still_proves_the_member_is_alive() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let timeout = session.config().presence_timeout_ms;
    let late = timeout - 1_000;

    // 過期的 touch：不套用，但它證明手機還連著。
    let expired = Envelope::new(
        MessageType::Event,
        EVENT_TOUCH,
        phone.clone(),
        "m-late",
        at(late),
    )
    .with_session(SESSION)
    .with_expiry(at(late - 1))
    .with_payload(json!({"kind": "tap"}));
    let submission = session.submit(expired, &phone, at(late));
    assert_eq!(submission.outcome, Outcome::Expired);

    session.tick(at(late + 2_000));
    assert_eq!(
        session.state().members()[0].presence,
        Presence::Online,
        "被拒絕的訊息也證明成員還在"
    );
}

/// Offline 的成員送 event 之後轉回 Online，而且只產生**一次** presence patch
/// （互動與 presence 合併在同一個 revision 裡，不會一則訊息兩次廣播）。
#[test]
fn an_offline_member_returns_online_with_a_single_presence_patch() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let timeout = session.config().presence_timeout_ms;
    session.tick(at(timeout + 1_000));
    assert_eq!(session.state().members()[0].presence, Presence::Offline);

    let before = session.revision();
    let now = at(timeout + 2_000);
    let submission = session.submit(touch("m-back", &phone, now), &phone, now);
    assert_eq!(submission.outcome, Outcome::Applied);
    assert_eq!(session.state().members()[0].presence, Presence::Online);
    assert_eq!(
        session.revision(),
        before + 1,
        "presence 與互動只推進一個 revision"
    );
    let patches = broadcasts(&submission.outputs);
    assert_eq!(patches.len(), 1, "只送一則 state patch：{:?}", patches);
    assert_eq!(
        patches[0].payload["patch"]["members"][0]["presence"],
        json!("online")
    );
    assert_outputs_valid(&submission.outputs);
}

// --------------------------------------------------- 成員回報結清 intent

/// 成員回報的 `result` 是 host 待決 intent 的唯一結清來源之一：回了 `observed`
/// 之後不得再等 TTL，也不得再稽核 `intent-expired`。observed 仍然 ≠ verified。
#[test]
fn a_member_result_settles_the_pending_intent_before_its_ttl() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let submission = session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    let command_id = sends(&submission.outputs)
        .into_iter()
        .find(|(_, envelope)| envelope.message_type == MessageType::Command)
        .map(|(_, envelope)| envelope.message_id.clone())
        .expect("host 送出了 Behavior Intent");
    assert_eq!(session.pending_intents().len(), 1);

    let report = Envelope::new(
        MessageType::Result,
        interaction_session::NAME_BEHAVIOR_REQUEST,
        phone.clone(),
        "r1",
        at(200),
    )
    .with_session(SESSION)
    .with_causation(command_id.clone())
    .with_payload(json!({"status": "observed"}));
    let reported = session.submit(report, &phone, at(200));
    assert_eq!(reported.outcome, Outcome::Accepted);
    assert!(
        session.pending_intents().is_empty(),
        "回報 observed 之後不該再掛著等 TTL"
    );

    let outputs = session.tick(at(60_000));
    assert!(
        !outputs.iter().any(|o| matches!(
            o,
            Output::Audit { kind, .. } if kind == "character.session.intent-expired"
        )),
        "已結清的 intent 不得再稽核成過期"
    );
    let counters = session.diagnostics().counters;
    assert_eq!(counters.get("intents.observed"), Some(&1));
    assert_eq!(counters.get("intents.expired"), None);
    // 誠實階梯：observed 只是「對方說它演了」，不是 verified。
    assert_eq!(session.state().truth().state, TruthState::None);
}

/// 沒有人回報的 intent 照舊在 TTL 到期時被稽核（結清只對真的有回覆的那些生效）。
#[test]
fn an_unanswered_intent_still_expires_and_is_audited() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    assert_eq!(session.pending_intents().len(), 1);

    let outputs = session.tick(at(60_000));
    assert!(outputs.iter().any(|o| matches!(
        o,
        Output::Audit { kind, .. } if kind == "character.session.intent-expired"
    )));
    assert_eq!(
        session.diagnostics().counters.get("intents.expired"),
        Some(&1)
    );
}

/// 已終態的 intent 再收到一則 result：忽略，不重複計數（重播不得灌計數器）。
#[test]
fn repeating_a_terminal_result_never_counts_twice() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let submission = session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    let command_id = sends(&submission.outputs)
        .into_iter()
        .find(|(_, envelope)| envelope.message_type == MessageType::Command)
        .map(|(_, envelope)| envelope.message_id.clone())
        .expect("behavior command");

    for (index, at_ms) in [200i64, 300].into_iter().enumerate() {
        let report = Envelope::new(
            MessageType::Result,
            interaction_session::NAME_BEHAVIOR_REQUEST,
            phone.clone(),
            format!("r{index}"),
            at(at_ms),
        )
        .with_session(SESSION)
        .with_causation(command_id.clone())
        .with_payload(json!({"status": "rejected", "code": "unsupported-capability"}));
        session.submit(report, &phone, at(at_ms));
    }
    assert_eq!(
        session.diagnostics().counters.get("intents.rejected"),
        Some(&1),
        "同一個 intent 的重複回報只計一次"
    );
}

/// `accepted`／`acknowledged` 不是終態：intent 仍然掛著（誠實階梯：acknowledged ≠ completed）。
#[test]
fn an_acknowledged_result_does_not_settle_the_intent() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let submission = session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    let command_id = sends(&submission.outputs)
        .into_iter()
        .find(|(_, envelope)| envelope.message_type == MessageType::Command)
        .map(|(_, envelope)| envelope.message_id.clone())
        .expect("behavior command");

    let report = Envelope::new(
        MessageType::Result,
        interaction_session::NAME_BEHAVIOR_REQUEST,
        phone.clone(),
        "r-ack",
        at(200),
    )
    .with_session(SESSION)
    .with_causation(command_id)
    .with_payload(json!({"status": "acknowledged"}));
    session.submit(report, &phone, at(200));
    assert_eq!(
        session.pending_intents().len(),
        1,
        "acknowledged 只是收到了，不是演過了"
    );
}

// ------------------------------------------------------- resume／snapshot

#[test]
fn resume_replays_the_delta_log() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let mut local = session.snapshot().state;
    let mut local_revision = session.revision();

    for i in 0..3 {
        let now = at(100 + i * 4_000);
        session.submit(touch(&format!("m{i}"), &phone, now), &phone, now);
    }
    let resumed = session.resume(&phone, local_revision, 2, session.epoch(), at(20_000));
    let interaction_session::Resume::Patches { envelopes } = resumed else {
        panic!("日誌內有這些 revision，必須回 patches");
    };
    assert_eq!(envelopes.len() as u64, session.revision() - local_revision);
    for envelope in &envelopes {
        envelope.validate().expect("replayed patch validates");
        assert_eq!(
            accept_state(local_revision, envelope),
            StateDecision::Apply {
                revision: local_revision + 1
            }
        );
        local = apply_patch(&local, &envelope.payload["patch"]);
        local_revision += 1;
    }
    assert_eq!(local, session.snapshot().state);
    assert_eq!(state_hash(&local), session.snapshot().hash);
    assert_eq!(session.diagnostics().counters.get("resumes"), Some(&1));
}

#[test]
fn resume_falls_back_to_a_snapshot_when_the_log_ring_wrapped() {
    let mut session = CharacterSession::new(
        SessionConfig {
            event_log_cap: 2,
            ..config()
        },
        1,
        t0(),
    );
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let stale_revision = session.revision();
    for i in 0..4 {
        let now = at(100 + i * 4_000);
        session.submit(touch(&format!("m{i}"), &phone, now), &phone, now);
    }
    assert_eq!(session.event_log().len(), 2, "日誌是有界環");
    let resumed = session.resume(&phone, stale_revision, 2, session.epoch(), at(30_000));
    let interaction_session::Resume::Snapshot { envelope } = resumed else {
        panic!("日誌涵蓋不到就要 snapshot fallback（這不是錯誤）");
    };
    envelope.validate().expect("snapshot validates");
    assert_eq!(envelope.payload["kind"], json!("snapshot"));
    assert_eq!(envelope.payload["revision"], json!(session.revision()));
    assert_eq!(envelope.payload["hash"], json!(session.snapshot().hash));
}

#[test]
fn resume_with_a_different_epoch_returns_a_session_reset() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let resumed = session.resume(&phone, 1, 0, session.epoch() + 5, at(1_000));
    let interaction_session::Resume::EpochMismatch { envelope } = resumed else {
        panic!("epoch 不同必須回 session-reset snapshot");
    };
    envelope.validate().expect("reset snapshot validates");
    assert_eq!(envelope.payload["reason"], json!("session-reset"));
    assert_eq!(envelope.payload["sessionEpoch"], json!(session.epoch()));
}

#[test]
fn resume_at_the_current_revision_needs_no_patches() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let resumed = session.resume(&phone, session.revision(), session.sequence(), 1, at(1_000));
    assert_eq!(
        resumed,
        interaction_session::Resume::Patches {
            envelopes: Vec::new()
        }
    );
}

// ------------------------------------------------------------------ restore

#[test]
fn restore_continues_the_revision_and_survives_a_round_trip() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    let snapshot = session.snapshot();
    let store = MemoryStore::default();
    store.save(&snapshot).expect("save");
    let loaded = store
        .load(SESSION)
        .expect("load")
        .expect("snapshot is present");

    let restored = CharacterSession::restore(config(), &loaded, at(10_000)).expect("restore");
    assert_eq!(restored.revision(), session.revision(), "revision 不歸零");
    assert_eq!(restored.epoch(), session.epoch());
    assert_eq!(restored.sequence(), session.sequence());
    assert_eq!(restored.snapshot().hash, snapshot.hash);
    assert_eq!(restored.state(), session.state());
    assert_eq!(restored.state().members().len(), 1);
}

#[test]
fn restored_members_must_renegotiate_before_sending_events() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let snapshot = session.snapshot();
    let mut restored = CharacterSession::restore(config(), &snapshot, at(10_000)).expect("restore");
    let denied = restored.submit(touch("m1", &phone, at(10_100)), &phone, at(10_100));
    assert_eq!(
        denied.error,
        Some(ErrorCode::ScopeDenied),
        "還原出來的成員沒有協商結果，必須重送 capability"
    );

    // 重送 capability（§7 重連流程第 2 步）就恢復。
    let capability = Envelope::new(
        MessageType::Capability,
        "character.session.capability",
        phone.clone(),
        "cap1",
        at(10_200),
    )
    .with_session(SESSION)
    .with_payload(serde_json::to_value(device_announcement()).expect("serialize"));
    let submission = restored.submit(capability, &phone, at(10_200));
    assert_eq!(submission.outcome, Outcome::Applied);
    assert_eq!(sends(&submission.outputs).len(), 2, "capability + snapshot");
    assert_outputs_valid(&submission.outputs);
    assert_eq!(
        restored
            .submit(touch("m2", &phone, at(10_300)), &phone, at(10_300))
            .outcome,
        Outcome::Applied
    );
}

#[test]
fn restore_rejects_tampered_snapshots() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    let good = session.snapshot();

    let mut tampered = good.clone();
    if let Value::Object(map) = &mut tampered.state {
        map.insert("activity".into(), json!("celebrating"));
    }
    assert_eq!(
        CharacterSession::restore(config(), &tampered, at(1_000)).unwrap_err(),
        SessionError::HashMismatch
    );

    let mut wrong_session = good.clone();
    wrong_session.session_id = "session.other".into();
    assert_eq!(
        CharacterSession::restore(config(), &wrong_session, at(1_000)).unwrap_err(),
        SessionError::SessionMismatch
    );

    let broken_state = json!({"characterId": "x"});
    let broken = Snapshot {
        session_id: SESSION.into(),
        epoch: 1,
        revision: 4,
        sequence: 3,
        hash: state_hash(&broken_state),
        state: broken_state,
        at: t0(),
    };
    assert_eq!(
        CharacterSession::restore(config(), &broken, at(1_000)).unwrap_err(),
        SessionError::InvalidState
    );
}

// ------------------------------------------------------------------ cancel

#[test]
fn cancel_is_idempotent() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    assert_eq!(session.pending_intents().len(), 1);

    let cancel = Envelope::new(
        MessageType::Cancel,
        "character.behavior.cancel",
        phone.clone(),
        "c1",
        at(200),
    )
    .with_session(SESSION)
    .with_correlation("m1")
    .with_causation("m1");
    let first = session.cancel(cancel.clone(), &phone, at(200));
    assert_eq!(first.outcome, Outcome::CancelConfirmed);
    assert!(session.pending_intents().is_empty());
    assert_outputs_valid(&first.outputs);

    let mut again = cancel.clone();
    again.message_id = "c2".into();
    let second = session.cancel(again, &phone, at(300));
    assert_eq!(second.outcome, Outcome::CancelConfirmed);
    assert_eq!(second.result.payload["alreadyTerminal"], json!(true));

    // 同一個 messageId 的 cancel 是重複訊息，不重執行。
    let repeat = session.cancel(cancel, &phone, at(400));
    assert_eq!(repeat.result.payload["duplicate"], json!(true));
}

// ------------------------------------------------------------ diagnostics

#[test]
fn diagnostics_counts_without_leaking() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &device_announcement(), t0());
    session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    session.submit(touch("m1", &phone, at(110)), &phone, at(110));
    session.submit(
        touch("m2", &Party::device("iphone-2"), at(120)),
        &phone,
        at(120),
    );
    let diagnostics = session.diagnostics();
    assert_eq!(diagnostics.session_id, SESSION);
    assert_eq!(diagnostics.event_log_cap, session.config().event_log_cap);
    assert_eq!(diagnostics.counters.get("applied"), Some(&1));
    assert_eq!(diagnostics.counters.get("duplicates"), Some(&1));
    assert_eq!(
        diagnostics.counters.get("rejected.identity-mismatch"),
        Some(&1)
    );
    assert!(diagnostics.counters.keys().all(|k| !k.contains("iphone")));
    let text = serde_json::to_string(&diagnostics.members).expect("members serialize");
    assert!(!text.contains("negotiated"), "diagnostics 不外洩協商細節");
}

#[test]
fn persist_is_suggested_on_a_bounded_cadence() {
    let mut session = CharacterSession::new(
        SessionConfig {
            persist_every_revisions: 2,
            ..config()
        },
        1,
        t0(),
    );
    let phone = Party::device("iphone-1");
    let outcome = session
        .join(phone.clone(), &device_announcement(), t0())
        .expect("join");
    let touched = session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    let persisted = outcome
        .outputs
        .iter()
        .chain(touched.outputs.iter())
        .filter(|o| matches!(o, Output::Persist(_)))
        .count();
    assert!(persisted > 0, "revision 累積到門檻要建議持久化");
}
