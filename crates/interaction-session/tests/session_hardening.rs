//! 對抗審查（v0.6.0 Foundation）confirmed findings 的回歸測試。
//!
//! 每一條都對應 `docs/reviews/adversarial/6683403-20260904T161327Z.json` 裡的一則
//! confirmed finding，並且在修復前會紅：
//!
//! | finding | 測試 |
//! |---|---|
//! | identity-binding-004 | `capability_reannounce_does_not_flood_revisions` |
//! | capability-consent-049／identity-binding-005 | `capability_reannounce_does_not_refill_the_rate_bucket` |
//! | capability-consent-050／session-integrity-057 | `a_non_target_member_cannot_settle_or_count_an_intent` |
//! | capability-consent-050／evidence-honesty-010 | `a_target_that_already_reported_cannot_count_twice` |
//! | capability-consent-053／reconnect-recovery-043 | `an_intent_with_no_target_is_dropped_not_expired` |
//! | capability-consent-054 | `emergency_cancels_the_intents_already_sent_to_renderers` |
//! | capability-consent-055 | `member_messages_may_not_carry_a_consent_grant` |
//! | identity-binding-007 | `a_device_may_not_claim_the_host_renderer_role` |
//! | reconnect-recovery-042 | `a_self_reported_deadline_cannot_outlive_the_interaction_ttl` |
//! | session-integrity-056 | `task_truth_cannot_clear_an_emergency` |
//! | session-integrity-058 | `restore_never_moves_the_revision_backwards`／`a_member_cannot_rebuild_a_live_session_by_claiming_to_be_ahead` |
//! | reconnect-recovery-044（session 這一半） | `reconnecting_is_a_transport_fact_the_session_projects_faithfully` |
//! | session-integrity-060 | `a_capability_reply_never_exceeds_the_payload_limit` |
//! | session-integrity-061 | `restore_rejects_a_poisoned_snapshot_even_with_a_matching_hash` |
//! | session-integrity-062 | `a_rejected_message_does_not_consume_its_dedupe_slot` |

use chrono::{Duration, TimeZone, Utc};
use interaction_aip::{
    CapabilityAnnouncement, Envelope, ErrorCode, MemberRole, MessageType, Outcome, Party,
    SyncClass, Timestamp,
};
use interaction_character::TruthState;
use interaction_session::{
    accept_state_with_epoch, state_hash, Activity, CharacterSession, IgnoreReason, Output,
    Presence, Resume, RuntimeFact, SessionConfig, SessionError, StateDecision, EVENT_DISMISS,
    EVENT_TOUCH, REASON_SESSION_RESET,
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

fn renderer_announcement() -> CapabilityAnnouncement {
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

fn capability_frame(
    id: &str,
    source: &Party,
    ann: &CapabilityAnnouncement,
    now: Timestamp,
) -> Envelope {
    Envelope::new(
        MessageType::Capability,
        "character.session.capability",
        source.clone(),
        id,
        now,
    )
    .with_session(SESSION)
    .with_payload(serde_json::to_value(ann).expect("announcement serializes"))
}

fn report(id: &str, source: &Party, status: &str, now: Timestamp) -> Envelope {
    Envelope::new(
        MessageType::Result,
        interaction_session::NAME_BEHAVIOR_REQUEST,
        source.clone(),
        id,
        now,
    )
    .with_session(SESSION)
    .with_payload(json!({"status": status}))
}

fn join(
    session: &mut CharacterSession,
    party: &Party,
    ann: &CapabilityAnnouncement,
    now: Timestamp,
) {
    session
        .join(party.clone(), ann, now)
        .expect("join should succeed");
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

fn audits<'a>(outputs: &'a [Output], kind: &str) -> Vec<&'a Value> {
    outputs
        .iter()
        .filter_map(|o| match o {
            Output::Audit { kind: k, detail } if k == kind => Some(detail),
            _ => None,
        })
        .collect()
}

fn counter(session: &CharacterSession, key: &str) -> Option<u64> {
    session.diagnostics().counters.get(key).copied()
}

// ------------------------------------------------------- identity-binding-004

/// 重新協商是存活證明，不是狀態變更：連續的 capability 不得每則都推進一個 revision
/// （`docs/aip/character-session.md` §12.7 投影格線）。
#[test]
fn capability_reannounce_does_not_flood_revisions() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());
    let revision = session.revision();

    // presenceTimeout 預設 45 s → 投影格線 15 s。以下 10 則全部落在格線內。
    for index in 0..10 {
        let now = at(100 * (index + 1));
        let submission = session.submit(
            capability_frame(
                &format!("cap{index}"),
                &phone,
                &renderer_announcement(),
                now,
            ),
            &phone,
            now,
        );
        assert_eq!(submission.outcome, Outcome::Applied);
    }
    assert_eq!(
        session.revision(),
        revision,
        "格線內的重新協商不得產生新的 revision／廣播"
    );
}

// ------------------------ capability-consent-049／identity-binding-005

/// 重新協商不得把 token bucket 灌滿：否則成員每插一則 capability 就換回一整桶額度。
#[test]
fn capability_reannounce_does_not_refill_the_rate_bucket() {
    let mut session = CharacterSession::new(
        SessionConfig {
            rate_limit_per_sec: 4,
            ..config()
        },
        1,
        t0(),
    );
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());

    // 同一個 timestamp（零經過時間）：3 則 touch ＋ 1 則 capability 用掉整桶 4 個 token。
    for index in 0..3 {
        assert_eq!(
            session
                .submit(touch(&format!("m{index}"), &phone, at(10)), &phone, at(10))
                .outcome,
            Outcome::Applied
        );
    }
    assert_eq!(
        session
            .submit(
                capability_frame("cap", &phone, &renderer_announcement(), at(10)),
                &phone,
                at(10)
            )
            .outcome,
        Outcome::Applied
    );

    let limited = session.submit(touch("m9", &phone, at(10)), &phone, at(10));
    assert_eq!(
        limited.error,
        Some(ErrorCode::RateLimited),
        "capability 不得重置速率上限"
    );
}

// ------------- capability-consent-050／evidence-honesty-010／session-integrity-057

/// 沒有收到過 command 的成員不得結清 intent，也不得推高 `intents.observed`。
#[test]
fn a_non_target_member_cannot_settle_or_count_an_intent() {
    let mut session = session();
    let pad = Party::device("input-pad");
    let renderer = Party::device("iphone-1");
    // pad 只是輸入裝置（不宣告任何 intent），renderer 才是真正的派送目標。
    join(
        &mut session,
        &pad,
        &announcement(MemberRole::InputDevice, &[], &[EVENT_TOUCH]),
        t0(),
    );
    join(&mut session, &renderer, &renderer_announcement(), t0());

    let submission = session.submit(touch("m1", &pad, at(100)), &pad, at(100));
    assert_eq!(submission.outcome, Outcome::Applied);
    let targets = sends(&submission.outputs)
        .into_iter()
        .filter(|(_, e)| e.message_type == MessageType::Command)
        .count();
    assert_eq!(targets, 1, "只有 renderer 收得到 command");
    assert_eq!(session.pending_intents().len(), 1);

    // pad 用自己那則 touch 的 correlationId 冒領 intent（causationId 是隨便編的）。
    let forged = report("r-forged", &pad, "observed", at(200))
        .with_causation("not-a-real-command-id")
        .with_correlation("m1");
    session.submit(forged, &pad, at(200));

    assert_eq!(
        session.pending_intents().len(),
        1,
        "非目標的回報不得結清 intent"
    );
    assert_eq!(
        counter(&session, "intents.observed"),
        None,
        "非目標不得推高 intents.observed"
    );

    // 真正的目標從未回報 → TTL 到期仍然要誠實地稽核成過期。
    let outputs = session.tick(at(60_000));
    assert_eq!(
        audits(&outputs, "character.session.intent-expired").len(),
        1
    );
}

/// 已經回報過終態的目標再送一次（不同 messageId）不得重複計數。
#[test]
fn a_target_that_already_reported_cannot_count_twice() {
    let mut session = session();
    let a = Party::device("iphone-a");
    let b = Party::device("iphone-b");
    join(&mut session, &a, &renderer_announcement(), t0());
    join(&mut session, &b, &renderer_announcement(), t0());

    let submission = session.submit(touch("m1", &a, at(100)), &a, at(100));
    let command_to_a = sends(&submission.outputs)
        .into_iter()
        .find(|(to, e)| *to == &a && e.message_type == MessageType::Command)
        .map(|(_, e)| e.message_id.clone())
        .expect("A 收到 command");
    assert_eq!(session.pending_intents().len(), 1);

    for index in 0..3 {
        let now = at(200 + index * 10);
        let envelope =
            report(&format!("r{index}"), &a, "observed", now).with_causation(command_to_a.clone());
        session.submit(envelope, &a, now);
    }

    assert_eq!(
        counter(&session, "intents.observed"),
        Some(1),
        "同一個目標的重播回報只能計一次"
    );
    assert_eq!(
        session.pending_intents().len(),
        1,
        "B 還沒回報，intent 不得被 A 一個人結清"
    );
}

// ------------------ capability-consent-053／reconnect-recovery-043

/// 沒有任何目標的 intent 是 drop-if-offline，不是「沒有人回覆」：不得記成 intents.expired。
#[test]
fn an_intent_with_no_target_is_dropped_not_expired() {
    let mut session = session();
    let desktop = Party::human_surface("desktop");
    join(
        &mut session,
        &desktop,
        &announcement(
            MemberRole::HostRenderer,
            &["react-happily-to-touch"],
            &[EVENT_TOUCH],
        ),
        t0(),
    );

    let submission = session.submit(touch("m1", &desktop, at(100)), &desktop, at(100));
    assert_eq!(submission.outcome, Outcome::Applied);
    assert!(
        sends(&submission.outputs)
            .into_iter()
            .all(|(_, e)| e.message_type != MessageType::Command),
        "沒有遠端 renderer 就沒有 command"
    );
    assert!(
        session.pending_intents().is_empty(),
        "從未派送出去的 intent 不該掛進 pending 等 TTL"
    );
    assert_eq!(counter(&session, "intents.emitted"), Some(1));
    assert_eq!(counter(&session, "intents.dropped"), Some(1));

    let outputs = session.tick(at(60_000));
    assert!(
        audits(&outputs, "character.session.intent-expired").is_empty(),
        "沒有人被問過不等於沒有人回覆"
    );
    assert_eq!(counter(&session, "intents.expired"), None);
}

// ------------------------------------------------------ capability-consent-054

/// 緊急停止要對已經拿到 command 的 renderer 送 `character.behavior.cancel`，
/// 不是只清掉 host 自己的帳。
#[test]
fn emergency_cancels_the_intents_already_sent_to_renderers() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());

    let outputs = session.submit_runtime(
        RuntimeFact::TaskVerified {
            correlation_id: "flow-1".to_string(),
        },
        None,
        at(100),
    );
    assert!(
        sends(&outputs)
            .into_iter()
            .any(|(to, e)| to == &phone && e.name == "character.behavior.request"),
        "celebrate 已經送給 renderer"
    );
    assert_eq!(session.pending_intents().len(), 1);

    let outputs = session.submit_runtime(RuntimeFact::Emergency { engaged: true }, None, at(200));
    assert!(
        sends(&outputs)
            .into_iter()
            .any(|(to, e)| to == &phone && e.name == "character.behavior.cancel"),
        "emergency 必須撤銷已派送的 intent"
    );
    assert!(session.pending_intents().is_empty());
    for (_, envelope) in sends(&outputs) {
        envelope.validate().expect("cancel envelope validates");
    }
}

// ------------------------------------------------------ capability-consent-055

/// 成員不得自帶 `consentGrantId`：consent 只由 Consent Service 授予，AI／裝置不能宣稱自己有。
#[test]
fn member_messages_may_not_carry_a_consent_grant() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());

    let mut envelope = touch("m1", &phone, at(100));
    envelope.consent_grant_id = Some("grant_forged".to_string());
    let submission = session.submit(envelope, &phone, at(100));
    assert_eq!(submission.outcome, Outcome::Rejected);
    assert_eq!(submission.error, Some(ErrorCode::ScopeDenied));
    assert_eq!(
        session.state().activity(),
        Activity::Idle,
        "被拒絕的訊息不得套用"
    );
    let text = serde_json::to_string(&submission.outputs).expect("outputs serialize");
    assert!(!text.contains("grant_forged"), "稽核不得回顯輸入");
}

// ------------------------------------------------------- identity-binding-007

/// `host-renderer` 是可信桌面 surface 的角色：裝置自報一律降級並稽核，
/// 讓共享狀態裡的 role 與真正的派送目標一致。
#[test]
fn a_device_may_not_claim_the_host_renderer_role() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    let outcome = session
        .join(
            phone.clone(),
            &announcement(
                MemberRole::HostRenderer,
                &["react-happily-to-touch"],
                &[EVENT_TOUCH],
            ),
            t0(),
        )
        .expect("join");
    assert_eq!(
        outcome.negotiated.role,
        MemberRole::RemoteRenderer,
        "device 不得自報 host-renderer"
    );
    assert_eq!(
        session.state().members()[0].role,
        MemberRole::RemoteRenderer
    );
    assert_eq!(
        audits(&outcome.outputs, "character.session.role-corrected").len(),
        1
    );

    // 校正後的 role 與真正的派送目標一致：這台手機真的收得到 command。
    let submission = session.submit(touch("m1", &phone, at(100)), &phone, at(100));
    assert!(
        sends(&submission.outputs)
            .into_iter()
            .any(|(to, e)| to == &phone && e.name == "character.behavior.request"),
        "state 說得出的能力必須與實際派送一致"
    );

    // 可信 host surface 仍然可以是 host-renderer。
    let desktop = Party::human_surface("desktop");
    let outcome = session
        .join(
            desktop.clone(),
            &announcement(MemberRole::HostRenderer, &[], &[]),
            at(200),
        )
        .expect("join desktop");
    assert_eq!(outcome.negotiated.role, MemberRole::HostRenderer);
}

// ------------------------------------------------------ reconnect-recovery-042

/// 成員自報的 `expiresAt` 不得無界：離線期間排隊的舊觸摸不得在重連後被當成新鮮互動套用。
#[test]
fn a_self_reported_deadline_cannot_outlive_the_interaction_ttl() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());

    let stale = Envelope::new(
        MessageType::Event,
        EVENT_TOUCH,
        phone.clone(),
        "m-stale",
        at(-600_000),
    )
    .with_session(SESSION)
    .with_expiry(at(3_600_000))
    .with_payload(json!({"kind": "tap"}));
    let submission = session.submit(stale, &phone, at(0));
    assert_eq!(
        submission.outcome,
        Outcome::Expired,
        "十分鐘前的觸摸不得因為自報 expiresAt 很遠就被套用"
    );
    assert_eq!(submission.error, Some(ErrorCode::Expired));
    assert_eq!(session.state().activity(), Activity::Idle);
    assert_eq!(
        audits(&submission.outputs, "aip.clock-skew").len(),
        1,
        "超過 MAX_CLOCK_SKEW_MS 要稽核"
    );
}

// ------------------------------------------------------- session-integrity-056

/// 只有 `runtime.emergency{engaged:false}` 能離開 emergency：任何 `task.*` 真相轉錄
/// 都不得把守衛清掉（CLAUDE.md：AI 不可解除 emergency stop）。
#[test]
fn task_truth_cannot_clear_an_emergency() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());
    session.submit_runtime(RuntimeFact::Emergency { engaged: true }, None, at(100));
    assert_eq!(session.state().truth().state, TruthState::Emergency);

    let outputs = session.submit_runtime(
        RuntimeFact::TaskState {
            truth: TruthState::Unknown,
            correlation_id: Some("unrelated".to_string()),
        },
        None,
        at(200),
    );
    assert_eq!(
        session.state().truth().state,
        TruthState::Emergency,
        "task.state 不得改寫 emergency 真相"
    );
    assert_eq!(session.state().activity(), Activity::Frozen);
    assert!(
        !audits(&outputs, "character.session.emergency").is_empty(),
        "被擋下的真相要留稽核"
    );

    let verified = session.submit_runtime(
        RuntimeFact::TaskVerified {
            correlation_id: "flow-1".to_string(),
        },
        None,
        at(300),
    );
    assert_eq!(session.state().truth().state, TruthState::Emergency);
    assert!(
        sends(&verified).is_empty(),
        "emergency 中不得派送 celebrate"
    );

    let blocked = session.submit(touch("m2", &phone, at(400)), &phone, at(400));
    assert_eq!(blocked.error, Some(ErrorCode::ScopeDenied));

    // 只有明確解除才回得來。
    session.submit_runtime(RuntimeFact::Emergency { engaged: false }, None, at(500));
    assert_eq!(session.state().truth().state, TruthState::None);
}

// ------------------------------------------------------- session-integrity-058

fn drive_revisions(session: &mut CharacterSession, party: &Party, count: usize, from_ms: i64) {
    for index in 0..count {
        let now = at(from_ms + (index as i64) * 1_000);
        session.submit(touch(&format!("t{index}"), party, now), party, now);
        session.tick(now + Duration::milliseconds(500));
    }
}

/// 非乾淨重啟後 host 的 revision 不得倒退，否則成員依 §6 rollback 規則永遠忽略權威 snapshot。
///
/// 取捨（見 `CharacterSession::restore` 的註解）：restore 以「一個持久化間隔」保守跳號，
/// 真正的保證則來自 resume——成員拿出「我看過更高的 revision」的證據時 host 才 epoch+1
/// 並發 `session-reset`。
#[test]
fn restore_never_moves_the_revision_backwards() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());
    drive_revisions(&mut session, &phone, 3, 100);
    let persisted = session.snapshot();
    // 持久化之後又跑了遠超過一個持久化間隔的 revision（模擬非乾淨關機）。
    drive_revisions(&mut session, &phone, 40, 10_000);
    let live_revision = session.revision();
    assert!(
        live_revision > persisted.revision + config().persist_every_revisions,
        "成員已經領先快照超過一個持久化間隔（{live_revision} vs {}）",
        persisted.revision
    );

    let mut restored =
        CharacterSession::restore(config(), &persisted, at(600_000)).expect("restore");
    assert!(
        restored.revision() >= persisted.revision + config().persist_every_revisions,
        "restore 要以持久化間隔保守跳號，避免 revision 倒退"
    );
    assert_eq!(restored.epoch(), persisted.epoch, "還原本身不重建 session");

    let resumed = restored.resume(&phone, live_revision, session.sequence(), 1, at(600_100));
    let envelope = match resumed {
        Resume::EpochMismatch { envelope } => envelope,
        other => panic!("領先的成員證明 host 倒退過，必須拿到 session-reset：{other:?}"),
    };
    assert_eq!(envelope.payload["reason"], json!(REASON_SESSION_RESET));
    assert_eq!(restored.epoch(), 2, "重建過的 session 要換 epoch");
    assert_eq!(
        accept_state_with_epoch(live_revision, 1, &envelope),
        StateDecision::Reset {
            revision: restored.revision()
        },
        "重啟後的權威 snapshot 不得被成員當成 rollback 丟掉"
    );
    envelope.validate().expect("reset snapshot validates");
}

/// 反面：沒有倒退過的 host **不得**被一個成員的宣稱逼著重建 session
/// （否則任何成員送一則 `resume{lastRevision: u64::MAX}` 就能讓所有人丟掉本地狀態）。
#[test]
fn a_member_cannot_rebuild_a_live_session_by_claiming_to_be_ahead() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());
    let epoch = session.epoch();
    let ahead = session.revision() + 500;

    let resumed = session.resume(&phone, ahead, session.sequence(), epoch, at(1_000));
    let envelope = match resumed {
        Resume::Snapshot { envelope } => envelope,
        other => panic!("沒有倒退證據時只給權威 snapshot：{other:?}"),
    };
    assert_eq!(session.epoch(), epoch, "成員的宣稱不得重建 session");
    assert_eq!(
        envelope.payload.get("reason"),
        None,
        "沒有重建就不得宣稱 session-reset"
    );
    assert_eq!(
        counter(&session, "resumes.ahead"),
        Some(1),
        "超前的宣稱要留下計數（誠實記錄，不動狀態）"
    );
    // 成員拿到的是一則普通 snapshot：它自己宣稱的進度較高，依 §6 會忽略——這是刻意的，
    // 因為那個進度是它自己編出來的。
    assert_eq!(
        accept_state_with_epoch(ahead, epoch, &envelope),
        StateDecision::Ignore {
            reason: IgnoreReason::Rollback
        }
    );
}

// ------------------------------------------------------ reconnect-recovery-044

/// `Presence::Reconnecting` 的**產生者是 Transport**（斷線後、還在退避重連的窗口內），
/// 不是 session：session 不能從「45 秒沒聽到聲音」推論出「對方正在重連」（那是推論，不是真相）。
///
/// 這一條只釘死 session 這一半的契約：transport 寫得進 `reconnecting`、它會如實投影進共享狀態、
/// 逾時後由 `tick` 轉成 `offline`、成員一送訊息就轉回 `online`。剩下那一半（`mobile.rs` 斷線收尾
/// 改送 `Reconnecting`）在 `crates/interaction-runtime`，不在本組檔案範圍內。
#[test]
fn reconnecting_is_a_transport_fact_the_session_projects_faithfully() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());

    let outputs = session.presence(&phone, Presence::Reconnecting, at(1_000));
    assert_eq!(
        session.state().members()[0].presence,
        Presence::Reconnecting,
        "transport 寫進來的 presence 要如實投影（感測不靜默）"
    );
    assert_eq!(
        audits(&outputs, "character.session.presence")[0]["presence"],
        json!("reconnecting")
    );

    // 重連窗口過了還是沒聲音 → offline（session 只降級，不假裝還在重連）。
    session.tick(at(1_000 + session.config().presence_timeout_ms));
    assert_eq!(session.state().members()[0].presence, Presence::Offline);

    // 真的回來了：一則已驗證訊息就轉回 online。
    let now = at(1_000 + session.config().presence_timeout_ms + 100);
    assert_eq!(
        session
            .submit(touch("m-back", &phone, now), &phone, now)
            .outcome,
        Outcome::Applied
    );
    assert_eq!(session.state().members()[0].presence, Presence::Online);
}

// ------------------------------------------------------- session-integrity-060

/// host 自己送出的 capability 回覆不得超過 AIP payload 上限，`unsupportedInputs` 也不得無界。
#[test]
fn a_capability_reply_never_exceeds_the_payload_limit() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    let inputs: Vec<String> = (0..1_500).map(|i| format!("device.unknown-{i}")).collect();
    let ann = CapabilityAnnouncement {
        spec_versions: vec!["aip/1.0".to_string()],
        role: Some(MemberRole::RemoteRenderer),
        profiles: vec!["character-session".to_string()],
        sync_classes: vec![SyncClass::Semantic],
        intents: vec!["idle".to_string()],
        inputs,
        ..Default::default()
    };
    let outcome = session.join(phone.clone(), &ann, t0()).expect("join");
    outcome
        .capability_envelope
        .validate()
        .expect("host 送出的 capability 回覆必須自己驗得過");
    assert!(
        outcome.negotiated.unsupported_inputs.len()
            <= interaction_session::MAX_PROJECTED_UNSUPPORTED_INPUTS,
        "unsupportedInputs 必須有界"
    );
}

// ------------------------------------------------------- session-integrity-061

/// hash 自洽不等於乾淨：帶未知鍵的 snapshot 不得被原樣重新廣播給成員。
#[test]
fn restore_rejects_a_poisoned_snapshot_even_with_a_matching_hash() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());
    let good = session.snapshot();

    let mut poisoned = good.clone();
    if let Value::Object(map) = &mut poisoned.state {
        map.insert("evilKey".into(), json!({"safetyText": "all clear"}));
    }
    poisoned.hash = state_hash(&poisoned.state);
    let error = CharacterSession::restore(config(), &poisoned, at(1_000))
        .expect_err("被污染的 snapshot 不得還原");
    assert!(matches!(
        error,
        SessionError::InvalidState | SessionError::HashMismatch
    ));

    // 巢狀污染（成員投影裡）一樣擋掉。
    let mut nested = good.clone();
    if let Some(member) = nested
        .state
        .get_mut("members")
        .and_then(Value::as_array_mut)
        .and_then(|m| m.first_mut())
        .and_then(Value::as_object_mut)
    {
        member.insert("evilKey".into(), json!("all clear"));
    }
    nested.hash = state_hash(&nested.state);
    assert!(CharacterSession::restore(config(), &nested, at(1_000)).is_err());

    // 再深一層：`members[].party` 自己也是一個物件（`Party` 沒有
    // `deny_unknown_fields`），未知鍵藏在這裡一樣不得放行。
    let mut nested_party = good.clone();
    if let Some(party) = nested_party
        .state
        .get_mut("members")
        .and_then(Value::as_array_mut)
        .and_then(|m| m.first_mut())
        .and_then(|m| m.get_mut("party"))
        .and_then(Value::as_object_mut)
    {
        party.insert("evilKey".into(), json!("smuggled"));
    }
    nested_party.hash = state_hash(&nested_party.state);
    assert!(
        CharacterSession::restore(config(), &nested_party, at(1_000)).is_err(),
        "未知鍵藏在 members[].party 裡也必須被拒絕"
    );

    // 乾淨的 snapshot 照樣還原得回來。
    let restored = CharacterSession::restore(config(), &good, at(1_000)).expect("restore");
    let republished = restored.snapshot();
    assert_eq!(republished.hash, state_hash(&republished.state));
    assert!(republished.state.get("evilKey").is_none());
}

// ------------------------------------------------------- hash-numeric-contract-017

/// host 投影上限與 AIP 協商截斷點是兩個不同的數字，關係由 doc comment 記著：
/// 靠註解提醒下一個人不夠，任何一邊改動破壞這個關係都必須立刻紅燈。
#[test]
fn projected_unsupported_inputs_cap_is_at_most_the_aip_cap() {
    const {
        assert!(
            interaction_session::MAX_PROJECTED_UNSUPPORTED_INPUTS
                <= interaction_aip::limits::MAX_UNSUPPORTED_INPUTS,
            "host 的投影上限不得大於 AIP 協商本身的截斷點"
        )
    };
}

// ------------------------------------------------------- session-integrity-062

/// 被拒絕的訊息不得吃掉去重環的位置：否則重送會拿到 `accepted{duplicate:true}`
/// 卻從來沒有被套用（回覆語意與實際發生的事不一致）。
#[test]
fn a_rejected_message_does_not_consume_its_dedupe_slot() {
    let mut session = session();
    let phone = Party::device("iphone-1");
    join(&mut session, &phone, &renderer_announcement(), t0());
    session.submit_runtime(RuntimeFact::Emergency { engaged: true }, None, at(100));

    let blocked = session.submit(touch("m2", &phone, at(200)), &phone, at(200));
    assert_eq!(blocked.error, Some(ErrorCode::ScopeDenied));

    session.submit_runtime(RuntimeFact::Emergency { engaged: false }, None, at(300));
    let resent = session.submit(touch("m2", &phone, at(400)), &phone, at(400));
    assert_eq!(
        resent.outcome,
        Outcome::Applied,
        "被拒絕過的 messageId 重送必須真的套用，而不是被當成重複"
    );
    assert_eq!(session.state().activity(), Activity::Reacting);
    assert_eq!(resent.result.payload.get("duplicate"), None);

    // 真的套用過的訊息還是照舊去重。
    let again = session.submit(touch("m2", &phone, at(500)), &phone, at(500));
    assert_eq!(again.outcome, Outcome::Accepted);
    assert_eq!(again.result.payload["duplicate"], json!(true));
}
