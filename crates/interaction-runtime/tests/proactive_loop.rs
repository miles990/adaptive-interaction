//! 主動式對話政策整合測試：自主來源受確定性限制、明確請求不受限、
//! 安全類永不被頻率壓制、限制狀態跨重啟持續。

use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;

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

/// conversation 頻道（builtin.conversation actuator）：governor 勿擾預設
/// 壓制清單不含此頻道，勿擾延後只能靠 proactive gate。
async fn conversation_plan(
    rt: &Runtime,
    text: &str,
    meta: BTreeMap<String, serde_json::Value>,
) -> PlanId {
    let mut intent = SemanticIntent::new("proactive-test");
    intent.preferred_channels = vec!["conversation".into()];
    intent.message = Some(text.into());
    rt.create_plan(intent, vec!["conversation".into()], 1, 1, false, None, meta)
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
async fn governor_quiet_hours_defer_dialogue_even_on_conversation_channel() {
    let (_g, rt) = runtime().await;
    rt.proactive_dialogue_configure(json!({
        "mode": "lively",
        "minIntervalMinutes": 0,
        "mergeWindowSeconds": 0,
        "noFollowUp": false
    }))
    .await
    .unwrap();
    // 設一個涵蓋現在的勿擾窗（±1 小時；跨午夜由 quiet_window_active 處理）。
    let now = chrono::Local::now();
    let start = (now - chrono::Duration::hours(1))
        .format("%H:%M")
        .to_string();
    let end = (now + chrono::Duration::hours(1))
        .format("%H:%M")
        .to_string();
    rt.update_policy(json!({"quietHours": [{"start": start, "end": end}]}))
        .await
        .unwrap();

    // conversation 不在 governor 預設壓制頻道清單內——勿擾延後必須由
    // proactive gate 在 Rust 內確定性強制，而非依賴頻道壓制。
    let mut meta = BTreeMap::new();
    meta.insert("proactiveClass".to_string(), json!("suggestion"));
    let plan = conversation_plan(&rt, "勿擾中的建議", meta).await;
    let receipts = rt
        .execute_plan(&plan, ActionSource::Autonomous, false)
        .await
        .unwrap();
    assert_eq!(receipts[0].current_status, ActionStatus::Blocked);
    assert!(
        receipts[0].policy_decisions.iter().any(|d| matches!(
            d,
            PolicyDecision::Silenced { rule, detail }
                if rule == "proactive-dialogue" && detail.contains("勿擾")
        )),
        "壓制決策必須指名 proactive-dialogue 勿擾延後：{:?}",
        receipts[0].policy_decisions
    );

    // 安全類不受勿擾延後（只去重，永不被頻率或勿擾壓制）。
    let mut meta = BTreeMap::new();
    meta.insert("proactiveClass".to_string(), json!("safety"));
    let plan = conversation_plan(&rt, "需要確認", meta).await;
    let receipts = rt
        .execute_plan(&plan, ActionSource::Autonomous, false)
        .await
        .unwrap();
    assert_ne!(
        receipts[0].current_status,
        ActionStatus::Blocked,
        "安全類在勿擾窗內仍須送達"
    );

    // 使用者明確關閉 dndDefer → 勿擾窗內一般訊息照送。
    rt.proactive_dialogue_configure(json!({"dndDefer": false}))
        .await
        .unwrap();
    let mut meta = BTreeMap::new();
    meta.insert("proactiveClass".to_string(), json!("suggestion"));
    let plan = conversation_plan(&rt, "關閉延後後的建議", meta).await;
    let receipts = rt
        .execute_plan(&plan, ActionSource::Autonomous, false)
        .await
        .unwrap();
    assert_ne!(receipts[0].current_status, ActionStatus::Blocked);
}

#[tokio::test]
async fn gate_persists_rate_state_eagerly_and_survives_crash_restart() {
    // persist 不得靜默失敗的前提是「每次變更都立即持久化」：閘門通過後
    // 狀態必須即時寫入 meta（不靠 shutdown），模擬 crash 重啟後
    // sentThisHour 連續、不歸零。
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
        rt.start_session(Some("t".into()), None, vec![])
            .await
            .unwrap();
        rt.proactive_dialogue_configure(json!({
            "mode": "lively",
            "minIntervalMinutes": 0,
            "mergeWindowSeconds": 0,
            "noFollowUp": false
        }))
        .await
        .unwrap();
        let mut meta = BTreeMap::new();
        meta.insert("proactiveClass".to_string(), json!("suggestion"));
        let plan = conversation_plan(&rt, "建議", meta).await;
        let receipts = rt
            .execute_plan(&plan, ActionSource::Autonomous, false)
            .await
            .unwrap();
        assert_ne!(receipts[0].current_status, ActionStatus::Blocked);
        // 閘門後 meta 立即反映本次發送（configure 失敗會回傳 Err，
        // gate 失敗會記 log——此處驗證成功路徑確實即時落盤）。
        let raw = rt
            .store
            .get_meta(interaction_runtime::proactive::PROACTIVE_META_KEY)
            .unwrap()
            .expect("閘門通過後主動對話狀態必須已持久化");
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed
                .get("recentSends")
                .and_then(|v| v.as_array())
                .map(Vec::len),
            Some(1)
        );
        // 不呼叫 shutdown → 模擬 crash。
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
        status.get("sentThisHour").and_then(|v| v.as_u64()),
        Some(1),
        "crash 重啟後頻率計數連續，不得歸零"
    );
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

#[tokio::test]
async fn ai_generated_recipe_creates_a_bounded_agent_session_and_only_renders_validated_candidate()
{
    std::env::set_var(
        "INTERACT_AI_CLAUDE_BIN",
        format!(
            "{}/tests/fixtures/fake_claude.sh",
            env!("CARGO_MANIFEST_DIR")
        ),
    );
    std::env::set_var("FAKE_MODE", "proactive");
    let (_home, rt) = runtime().await;
    rt.proactive_dialogue_configure(json!({
        "mode": "natural",
        "generativeAgent": "claude-code",
        "dailyGenerativeSessions": 2,
        "dailyGenerativeCostUsd": 0.5,
        "minIntervalMinutes": 0,
        "mergeWindowSeconds": 0,
        "noFollowUp": false
    }))
    .await
    .unwrap();
    rt.add_push_receptor("event.proactive", "主動候選事件", "task", false)
        .await
        .unwrap();
    rt.upsert_recipe_text(
        r#"
id: proactive-agent-e2e
name: 主動 Agent 候選
enabled: true
trigger:
  mode: single
  steps:
    - receptor: event.proactive
      condition: { ready: true }
decision: { objective: offer-low-risk-suggestion, allowNoAction: true }
intent: proactive-generated
message: { mode: ai-generated, allowSilence: true }
actuation:
  mode: single
  candidates: [conversation]
  minChannels: 1
  maxChannels: 1
verification: { strategy: best-effort, timeout: 10s }
limits: { cooldown: 1s, expiresAfter: 60s, maxPerHour: 2 }
"#,
    )
    .await
    .unwrap();

    rt.ingest(
        "event.proactive",
        BTreeMap::from([("ready".into(), json!(true))]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();

    let expected = "有一項低風險建議，想看時再點我。";
    for _ in 0..120 {
        if rt
            .outbox
            .recent(20)
            .iter()
            .any(|message| message.text.as_deref() == Some(expected))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        rt.outbox
            .recent(20)
            .iter()
            .any(|message| message.text.as_deref() == Some(expected)),
        "only the schema-validated agent candidate is rendered"
    );
    let sessions = rt.list_agent_sessions().await;
    let generated = sessions
        .iter()
        .find(|session| session.label.as_deref() == Some("主動式對話候選"))
        .expect("real limited agent session created");
    assert!(!generated.allow_write);
    assert_eq!(generated.tool_scope, ["conversation.generate"]);
    assert_eq!(generated.context_bundles.len(), 1);
    assert_eq!(generated.context_bundles[0].bundle["includes"], json!([]));
    let status = rt.proactive_dialogue_status().await;
    assert_eq!(status["generativeToday"]["sessions"], 1);
    assert_eq!(status["generativeToday"]["costUsd"], 0.01);

    std::env::remove_var("FAKE_MODE");
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

#[tokio::test]
async fn proactive_configuration_has_hard_limits_explicit_agent_and_deep_custom_merge() {
    let (_home, rt) = runtime().await;

    let configured = rt
        .proactive_dialogue_configure(json!({
            "mode": "custom",
            "custom": {"greeting": true},
            "generativeAgent": "claude-code",
            "dailyGenerativeSessions": 4,
            "dailyGenerativeCostUsd": 2.5
        }))
        .await
        .unwrap();
    assert_eq!(configured["config"]["custom"]["greeting"], true);
    assert_eq!(
        configured["config"]["custom"]["completion"], true,
        "a partial nested patch must preserve the other custom trigger switches"
    );
    assert_eq!(configured["config"]["generativeAgent"], "claude-code");

    for invalid in [
        json!({"maxPerHour": 13}),
        json!({"minIntervalMinutes": 61}),
        json!({"mergeWindowSeconds": 301}),
        json!({"dailyGenerativeSessions": 51}),
        json!({"dailyGenerativeCostUsd": 101.0}),
        json!({"generativeAgent": "auto-fallback"}),
        json!({"unknownPolicyKnob": true}),
    ] {
        assert!(
            matches!(
                rt.proactive_dialogue_configure(invalid).await,
                Err(DomainError::Validation(_))
            ),
            "invalid or unknown policy fields must fail closed"
        );
    }
}
