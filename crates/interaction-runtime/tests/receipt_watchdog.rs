//! Watchdog 誠實度：裝置命令「已送出、從未 ack」的收據必須變成 **uncertain**。
//!
//! 為什麼這條規則存在：link 傳輸（serial／mqtt／ble）與 HTTP 動器在 ack 逾時、
//! 送出途中失敗、等待中重連時，只會誠實回一張 `Dispatched` ＋
//! `outcomeUnknown` 的收據——它們不知道結果，也絕不重送實體命令。
//! 若沒有人接手，這張收據會一路顯示成「進行中」直到 plan TTL 才變 Expired：
//! - Expired 讓人以為「沒發生」——但它**可能已經發生了**；
//! - Failed 同樣是謊，而且會誘發重下同一命令＝重複的實體效果。
//!
//! 兩者都不誠實，正解只有 uncertain。
//!
//! 這裡直接呼叫 watchdog 每個 tick 呼叫的同一個函式（`sweep_receipts_at`），
//! 用注入的時鐘取代等待真實時間；不啟動真 watchdog，避免測試產生
//! mDNS 廣播或 agent 探索子程序等外部副作用。

use chrono::{Duration, Utc};
use interaction_core::{
    ActionId, ActionParameters, ActionReceipt, ActionStatus, ActuatorId, BoundedAction,
    CorrelationId, EventType, PlanId, RiskClass, SessionId, Timestamp,
};
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;

async fn test_runtime(home: &tempfile::TempDir) -> Runtime {
    Runtime::start(RuntimeOptions {
        home: Some(home.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .expect("runtime starts")
}

fn action(id: &str, expires_at: Timestamp, issued_at: Timestamp) -> BoundedAction {
    BoundedAction {
        action_id: ActionId::new(id),
        plan_id: PlanId::new("plan-watchdog"),
        session_id: SessionId::new("sess-watchdog"),
        actuator_id: ActuatorId::new("esp32-desk.vibe"),
        intent: "nudge".into(),
        risk_class: RiskClass::BoundedSideEffect,
        requested: ActionParameters {
            magnitude: Some(0.5),
            ..Default::default()
        },
        effective: ActionParameters {
            magnitude: Some(0.5),
            ..Default::default()
        },
        policy_decisions: vec![],
        expires_at,
        issued_at,
        correlation_id: CorrelationId::new("c-watchdog"),
        metadata: Default::default(),
        schema_version: "1.0".into(),
    }
}

/// 一張「驅動誠實回報結果未知」的收據：dispatched 時間戳＝命令真正送出的
/// 時刻（`dispatched_secs_ago` 秒前），並帶著 link/HTTP 傳輸實際會寫的註記。
fn dispatched_without_ack(
    id: &str,
    dispatched_secs_ago: i64,
    expires_at: Timestamp,
    note: &str,
) -> ActionReceipt {
    let now = Utc::now();
    let sent_at = now - Duration::seconds(dispatched_secs_ago);
    let action = action(id, expires_at, sent_at);
    let mut receipt = ActionReceipt::for_action(&action, sent_at);
    receipt
        .transition(ActionStatus::Accepted, sent_at)
        .expect("accepted");
    receipt
        .transition(ActionStatus::Dispatched, sent_at)
        .expect("dispatched");
    receipt
        .driver_response
        .insert(note.to_string(), json!(true));
    receipt
        .driver_response
        .insert("transport".into(), json!("serial"));
    receipt
}

/// ack 期限過了仍沒有 ack：uncertain＋ActionUncertain 事件，
/// 而且**沒有**經過 Failed（未知不得冒充失敗）。
#[tokio::test]
async fn a_dispatched_receipt_without_an_ack_becomes_uncertain() {
    let home = tempfile::tempdir().unwrap();
    let rt = test_runtime(&home).await;
    // TTL 還很遠：這一輪不可能是 TTL 造成的。
    let receipt = dispatched_without_ack(
        "act-ack-timeout",
        10,
        Utc::now() + Duration::minutes(30),
        "ackTimeout",
    );
    rt.store.upsert_receipt(&receipt, "haptic").unwrap();

    rt.sweep_receipts_at(Utc::now()).await;

    let swept = rt.store.receipt(&ActionId::new("act-ack-timeout")).unwrap();
    assert_eq!(
        swept.current_status,
        ActionStatus::Uncertain,
        "已送出、沒有 ack＝結果未知：{swept:?}"
    );
    assert!(
        !swept
            .timestamps
            .iter()
            .any(|(s, _)| matches!(s, ActionStatus::Failed | ActionStatus::Expired)),
        "不得先被判成 failed／expired：{:?}",
        swept.timestamps
    );
    let verdict = swept.verification.as_ref().expect("要留下判定理由");
    assert_eq!(
        verdict.verdict,
        interaction_core::VerificationVerdict::Uncertain
    );

    let events = rt.events.recent(50);
    assert!(
        events
            .iter()
            .any(|e| e.event_type == EventType::ActionUncertain
                && e.payload["actionId"] == json!("act-ack-timeout")),
        "必須發出 ActionUncertain（UI／角色層靠它離開「進行中」）：{events:?}"
    );
}

/// 送出途中失敗（BLE write 已寫出但沒有回應）與等待中重連（link-reset）
/// 走同一條規則。
#[tokio::test]
async fn every_outcome_unknown_note_is_swept_the_same_way() {
    let home = tempfile::tempdir().unwrap();
    let rt = test_runtime(&home).await;
    let far = Utc::now() + Duration::minutes(30);
    for (id, note) in [
        ("act-mid-send", "sendOutcomeUnknown"),
        ("act-link-reset", "outcomeUnknown"),
        ("act-http", "httpTimeout"),
    ] {
        rt.store
            .upsert_receipt(&dispatched_without_ack(id, 10, far, note), "haptic")
            .unwrap();
    }

    rt.sweep_receipts_at(Utc::now()).await;

    for id in ["act-mid-send", "act-link-reset", "act-http"] {
        let swept = rt.store.receipt(&ActionId::new(id)).unwrap();
        assert_eq!(swept.current_status, ActionStatus::Uncertain, "{id}");
    }
}

/// ack 期限**還沒到**的收據不得被動：裝置可能正要回 ack，
/// 提早標未知同樣是不誠實（而且會讓正常的慢裝置永遠拿不到 acknowledged）。
#[tokio::test]
async fn a_fresh_dispatch_is_left_alone_until_the_ack_window_passes() {
    let home = tempfile::tempdir().unwrap();
    let rt = test_runtime(&home).await;
    let receipt = dispatched_without_ack(
        "act-fresh",
        0,
        Utc::now() + Duration::minutes(30),
        "ackTimeout",
    );
    rt.store.upsert_receipt(&receipt, "haptic").unwrap();

    rt.sweep_receipts_at(Utc::now()).await;

    let swept = rt.store.receipt(&ActionId::new("act-fresh")).unwrap();
    assert_eq!(swept.current_status, ActionStatus::Dispatched);
}

/// 同時過了 ack 期限與 TTL：uncertain 優先。Expired 會讓人以為「沒發生」，
/// 但這個命令**可能已經在裝置上發生了**——那正是必須被看見的事。
#[tokio::test]
async fn uncertain_wins_over_expired_when_both_deadlines_have_passed() {
    let home = tempfile::tempdir().unwrap();
    let rt = test_runtime(&home).await;
    let receipt = dispatched_without_ack(
        "act-both",
        30,
        Utc::now() - Duration::seconds(5), // TTL 也過了
        "ackTimeout",
    );
    rt.store.upsert_receipt(&receipt, "haptic").unwrap();

    rt.sweep_receipts_at(Utc::now()).await;

    let swept = rt.store.receipt(&ActionId::new("act-both")).unwrap();
    assert_eq!(
        swept.current_status,
        ActionStatus::Uncertain,
        "結果未知比「逾期」更接近事實：{swept:?}"
    );
}

/// 沒有「結果未知」註記的收據仍走原本的 TTL 規則（這次改動不得偷偷把
/// 所有 dispatched 收據都變成 uncertain）。
#[tokio::test]
async fn receipts_without_the_note_still_expire_on_ttl() {
    let home = tempfile::tempdir().unwrap();
    let rt = test_runtime(&home).await;
    let now = Utc::now();
    let sent_at = now - Duration::seconds(30);
    let act = action("act-plain", now - Duration::seconds(5), sent_at);
    let mut receipt = ActionReceipt::for_action(&act, sent_at);
    receipt
        .transition(ActionStatus::Accepted, sent_at)
        .expect("accepted");
    receipt
        .transition(ActionStatus::Dispatched, sent_at)
        .expect("dispatched");
    rt.store.upsert_receipt(&receipt, "haptic").unwrap();

    rt.sweep_receipts_at(Utc::now()).await;

    let swept = rt.store.receipt(&ActionId::new("act-plain")).unwrap();
    assert_eq!(swept.current_status, ActionStatus::Expired);
}
