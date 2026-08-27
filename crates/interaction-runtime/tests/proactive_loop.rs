//! 主動式對話政策整合測試：自主來源受確定性限制、明確請求不受限、
//! 安全類永不被頻率壓制、限制狀態跨重啟持續。

use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;
use std::collections::BTreeMap;

async fn runtime() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    rt.presentation_hello(true, None).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    (dir, rt)
}

async fn bubble_plan(
    rt: &Runtime,
    text: &str,
    meta: BTreeMap<String, serde_json::Value>,
) -> PlanId {
    let mut intent = SemanticIntent::new("proactive-test");
    intent.preferred_channels = vec!["desktop-pet".into()];
    intent.message = Some(text.into());
    rt.create_plan(
        intent,
        vec!["companion.bubble.show".into()],
        1,
        1,
        false,
        None,
        meta,
    )
    .await
    .unwrap()
    .plan_id
}

#[tokio::test]
async fn autonomous_dialogue_hits_the_hard_hourly_cap() {
    let (_g, rt) = runtime().await;
    // 測試設定：解除間隔限制、只留每小時 3 則上限。
    rt.proactive_dialogue_configure(json!({
        "mode": "lively",
        "minIntervalMinutes": 0,
        "mergeWindowSeconds": 0,
        "noFollowUp": false
    }))
    .await
    .unwrap();

    let mut silenced = 0;
    for i in 0..4 {
        let mut meta = BTreeMap::new();
        meta.insert("proactiveClass".to_string(), json!("suggestion"));
        let plan = bubble_plan(&rt, &format!("建議 {i}"), meta).await;
        let receipts = rt
            .execute_plan(&plan, ActionSource::Autonomous, false)
            .await
            .unwrap();
        if receipts[0].current_status == ActionStatus::Blocked {
            silenced += 1;
            // 決策記錄必須指名 proactive-dialogue 規則（可解釋性）。
            assert!(receipts[0].policy_decisions.iter().any(|d| matches!(
                d,
                PolicyDecision::Silenced { rule, .. } if rule == "proactive-dialogue"
            )));
        }
    }
    assert_eq!(silenced, 1, "第 4 則必須被上限壓下");

    // 明確請求不受主動頻率限制（即使帶 proactiveClass）。
    let mut meta = BTreeMap::new();
    meta.insert("proactiveClass".to_string(), json!("suggestion"));
    let plan = bubble_plan(&rt, "使用者要求的訊息", meta).await;
    let receipts = rt
        .execute_plan(&plan, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert_eq!(receipts[0].current_status, ActionStatus::Dispatched);
}

#[tokio::test]
async fn off_mode_still_lets_safety_class_through() {
    let (_g, rt) = runtime().await;
    rt.proactive_dialogue_configure(json!({"mode": "off"}))
        .await
        .unwrap();

    // 一般建議：自主來源被壓下。
    let mut meta = BTreeMap::new();
    meta.insert("proactiveClass".to_string(), json!("suggestion"));
    let plan = bubble_plan(&rt, "一般建議", meta).await;
    let receipts = rt
        .execute_plan(&plan, ActionSource::Autonomous, false)
        .await
        .unwrap();
    assert_eq!(receipts[0].current_status, ActionStatus::Blocked);

    // 安全類（等待確認等）：即使 off 模式也放行。
    let mut meta = BTreeMap::new();
    meta.insert("proactiveClass".to_string(), json!("safety"));
    let plan = bubble_plan(&rt, "需要確認", meta).await;
    let receipts = rt
        .execute_plan(&plan, ActionSource::Autonomous, false)
        .await
        .unwrap();
    assert_eq!(receipts[0].current_status, ActionStatus::Dispatched);
}

#[tokio::test]
async fn quiet_request_and_user_reply_change_the_gate() {
    let (_g, rt) = runtime().await;
    rt.proactive_dialogue_configure(json!({
        "mode": "lively",
        "minIntervalMinutes": 0,
        "mergeWindowSeconds": 0
    }))
    .await
    .unwrap();

    // 使用者要求安靜一小時 → 一般訊息壓下。
    rt.proactive_dialogue_quiet(60).await;
    let mut meta = BTreeMap::new();
    meta.insert("proactiveClass".to_string(), json!("greeting"));
    let plan = bubble_plan(&rt, "問候", meta).await;
    let receipts = rt
        .execute_plan(&plan, ActionSource::Autonomous, false)
        .await
        .unwrap();
    assert_eq!(receipts[0].current_status, ActionStatus::Blocked);

    let status = rt.proactive_dialogue_status().await;
    assert!(status.get("quietUntil").and_then(|v| v.as_str()).is_some());
}

#[tokio::test]
async fn rate_state_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let rt = Runtime::start(RuntimeOptions {
            home: Some(dir.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        rt.proactive_dialogue_configure(json!({"mode": "off"}))
            .await
            .unwrap();
        rt.proactive_dialogue_quiet(120).await;
        rt.shutdown().await;
    }
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    let status = rt.proactive_dialogue_status().await;
    assert_eq!(
        status.pointer("/config/mode").and_then(|v| v.as_str()),
        Some("off"),
        "模式設定跨重啟保留"
    );
    assert!(
        status.get("quietUntil").and_then(|v| v.as_str()).is_some(),
        "安靜請求跨重啟保留"
    );
}
