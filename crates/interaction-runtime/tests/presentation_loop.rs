//! Presentation Provider 垂直閉環整合測試：
//! provider 逐項註冊 → 誠實可用性（隱藏/斷線）→ 命令執行 → ack → receipt
//! → 無 ack 標 Uncertain → estop 清空 → consent 預設 → behaviorIntent 白名單。

use interaction_core::*;
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
    (dir, rt)
}

async fn visible_session(rt: &Runtime) {
    rt.presentation_hello(true, Some("shu-agile".into())).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
}

async fn plan_and_execute(
    rt: &Runtime,
    actuator: &str,
    payload: serde_json::Value,
    message: Option<&str>,
) -> Vec<ActionReceipt> {
    let mut intent = SemanticIntent::new("companion-test");
    intent.preferred_channels = vec!["desktop-pet".into()];
    intent.payload = Some(payload);
    intent.message = message.map(|s| s.to_string());
    let plan = rt
        .create_plan(
            intent,
            vec![actuator.into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    rt.execute_plan(
        &plan.plan_id,
        interaction_policy::ActionSource::ExplicitRequest,
        false,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn provider_is_itemized_not_a_blob() {
    let (_g, rt) = runtime().await;
    let providers = rt.providers.list().await;
    let companion = providers
        .iter()
        .find(|p| p.identity.id.as_str() == "provider.companion.shu")
        .expect("companion provider registered");
    assert_eq!(companion.identity.kind, ProviderKind::Companion);
    assert_eq!(companion.receptors.len(), 7, "7 itemized receptors");
    assert_eq!(companion.actuators.len(), 7, "7 itemized actuators");
    // builtin provider 不再囊括 companion 能力（能力歸屬清楚）。
    let builtin = providers
        .iter()
        .find(|p| p.identity.id.as_str() == "provider.local.builtin")
        .unwrap();
    assert!(builtin
        .receptors
        .iter()
        .all(|id| !id.starts_with("companion.")));
    assert!(builtin
        .actuators
        .iter()
        .all(|id| !id.starts_with("companion.")));
}

#[tokio::test]
async fn consent_gated_capabilities_start_disabled() {
    let (_g, rt) = runtime().await;
    let snap = rt
        .capabilities(&DiscoveryContext {
            include_unavailable: true,
            ..Default::default()
        })
        .await;
    let availability = |id: &str| {
        snap.actuators
            .iter()
            .find(|a| a.id.as_str() == id)
            .unwrap_or_else(|| panic!("{id} listed"))
            .availability
    };
    // 敏感／侵入能力預設關閉。
    for id in [
        "companion.sound.play",
        "companion.speak",
        "companion.window.adjust",
        "companion.presence.set",
    ] {
        assert_ne!(
            availability(id),
            Availability::Available,
            "{id} must start disabled"
        );
    }
}

#[tokio::test]
async fn full_loop_bubble_dispatch_ack_completed() {
    let (_g, rt) = runtime().await;
    visible_session(&rt).await;

    let receipts =
        plan_and_execute(&rt, "companion.bubble.show", json!({}), Some("嗨，測試")).await;
    assert_eq!(receipts.len(), 1);
    let receipt = &receipts[0];
    // 誠實階梯：表面尚未確認 → Dispatched，不是 completed。
    assert_eq!(
        receipt.current_status,
        ActionStatus::Dispatched,
        "errors={:?} driver={:?}",
        receipt.errors,
        receipt.driver_response
    );

    // presentation.command 事件真的發出且帶 actionId。
    let events = rt.events.recent(50);
    let cmd = events
        .iter()
        .find(|e| e.event_type == EventType::PresentationCommand)
        .expect("presentation.command emitted");
    assert_eq!(
        cmd.payload.get("actionId").and_then(|v| v.as_str()),
        Some(receipt.action_id.as_str())
    );
    assert_eq!(
        cmd.payload
            .pointer("/params/message")
            .and_then(|v| v.as_str()),
        Some("嗨，測試")
    );

    // 視窗 ack → Completed，證據誠實標示「表面自報，無獨立觀察者」。
    let out = rt
        .presentation_ack(receipt.action_id.as_str(), "displayed", None)
        .await
        .unwrap();
    assert_eq!(out.get("status").unwrap().as_str().unwrap(), "completed");
    let stored = rt.store.receipt(&receipt.action_id).unwrap();
    let evidence = stored.verification.expect("verification evidence attached");
    assert_eq!(evidence.verdict, VerificationVerdict::AcknowledgedOnly);

    // 二次 ack 必須失敗（pending 已消費，不能重複自證）。
    assert!(rt
        .presentation_ack(receipt.action_id.as_str(), "displayed", None)
        .await
        .is_err());
}

#[tokio::test]
async fn no_ack_goes_uncertain_never_completed() {
    let (_g, rt) = runtime().await;
    visible_session(&rt).await;
    let receipts =
        plan_and_execute(&rt, "companion.bubble.show", json!({}), Some("沒人回應")).await;
    let action_id = receipts[0].action_id.clone();

    // TTL 過後 watchdog 掃描 → Uncertain（絕不是 completed）。
    let later = chrono::Utc::now()
        + chrono::Duration::milliseconds(interaction_runtime::presentation::ACK_TTL_MS + 500);
    rt.sweep_presentation_at(later).await;
    let stored = rt.store.receipt(&action_id).unwrap();
    assert_eq!(stored.current_status, ActionStatus::Uncertain);
    // 遲到的 ack 不能把 Uncertain 洗成 completed。
    assert!(rt
        .presentation_ack(action_id.as_str(), "displayed", None)
        .await
        .is_err());
}

#[tokio::test]
async fn hidden_companion_stops_surface_receptors_and_actuators() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    // 已連線但隱藏。
    rt.presentation_hello(false, None).await;

    // 視窗內 receptor 拒絕 ingest。
    let mut facts = BTreeMap::new();
    facts.insert("kind".to_string(), json!("clicked"));
    let err = rt
        .ingest("companion.click", facts.clone(), BTreeMap::new(), 1.0)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Unavailable(_)));

    // 非 companion receptor 不受影響（隱藏 ≠ 停機）。
    let mut ok_facts = BTreeMap::new();
    ok_facts.insert("event".to_string(), json!("still-works"));
    rt.ingest("manual.event", ok_facts, BTreeMap::new(), 1.0)
        .await
        .expect("runtime keeps running while companion hidden");

    // 視覺 actuator 誠實失敗（不假裝顯示了氣泡）。
    let receipts = plan_and_execute(&rt, "companion.bubble.show", json!({}), Some("看不到")).await;
    assert_eq!(receipts[0].current_status, ActionStatus::Failed);

    // 顯示後恢復。
    rt.presentation_hello(true, None).await;
    rt.ingest("companion.click", facts, BTreeMap::new(), 1.0)
        .await
        .expect("visible companion accepts clicks");
}

#[tokio::test]
async fn estop_clears_pending_and_blocks_late_ack() {
    let (_g, rt) = runtime().await;
    visible_session(&rt).await;
    let receipts = plan_and_execute(&rt, "companion.bubble.show", json!({}), Some("停止前")).await;
    let action_id = receipts[0].action_id.clone();

    rt.emergency_stop("test", None).await.unwrap();

    // estop 掃描：open receipt → Stopped；pending 清空 → 遲到 ack 被拒。
    let stored = rt.store.receipt(&action_id).unwrap();
    assert_eq!(stored.current_status, ActionStatus::Stopped);
    assert!(rt
        .presentation_ack(action_id.as_str(), "displayed", None)
        .await
        .is_err());
    assert_eq!(
        rt.presentation_status()
            .get("pendingCommands")
            .unwrap()
            .as_u64()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn behavior_intent_whitelist_enforced_through_full_loop() {
    let (_g, rt) = runtime().await;
    visible_session(&rt).await;
    // 未登記 intent → 執行層誠實 Failed，不會傳到表面。
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "hack-the-planet", "tone": "playful"}),
        None,
    )
    .await;
    assert_eq!(receipts[0].current_status, ActionStatus::Failed);

    // 合法 intent 走完整迴路。
    let ok = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "look-at-confirmation", "tone": "attentive"}),
        Some("這裡需要你選一個做法。"),
    )
    .await;
    assert_eq!(ok[0].current_status, ActionStatus::Dispatched);
    rt.presentation_ack(ok[0].action_id.as_str(), "displayed", None)
        .await
        .unwrap();
    assert_eq!(
        rt.store.receipt(&ok[0].action_id).unwrap().current_status,
        ActionStatus::Completed
    );
}

#[tokio::test]
async fn disconnected_surface_reports_offline_health() {
    let (_g, rt) = runtime().await;
    // 從未 hello → 表面離線，狀態誠實。
    let status = rt.presentation_status();
    assert_eq!(status.get("connected").unwrap().as_bool(), Some(false));
    let receptor = rt
        .registry
        .receptor_any(&ReceptorId::new("companion.click"))
        .await
        .unwrap();
    assert_eq!(receptor.health().await.status, HealthStatus::Offline);
}
