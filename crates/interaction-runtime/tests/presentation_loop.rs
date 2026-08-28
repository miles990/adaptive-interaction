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

/// 已發出的 `clear-all` presentation 命令則數。
fn clear_all_commands(rt: &Runtime) -> usize {
    rt.events
        .recent(200)
        .into_iter()
        .filter(|e| e.event_type == EventType::PresentationCommand)
        .filter(|e| e.payload.get("command").and_then(|v| v.as_str()) == Some("clear-all"))
        .count()
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

    // 隱藏中的視覺 actuator 在規劃期就被誠實排除（Offline），不再產生
    // 一個必然失敗的 dispatch。
    let mut intent = SemanticIntent::new("companion-test");
    intent.preferred_channels = vec!["desktop-pet".into()];
    intent.message = Some("看不到".into());
    let plan = rt
        .create_plan(
            intent,
            vec!["companion.bubble.show".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(plan.status, PlanStatus::Blocked);
    let err = rt
        .execute_plan(
            &plan.plan_id,
            interaction_policy::ActionSource::ExplicitRequest,
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)));

    // 顯示後恢復。
    rt.presentation_hello(true, None).await;
    rt.ingest("companion.click", facts, BTreeMap::new(), 1.0)
        .await
        .expect("visible companion accepts clicks");
    let receipts =
        plan_and_execute(&rt, "companion.bubble.show", json!({}), Some("現在看得到")).await;
    assert_eq!(receipts[0].current_status, ActionStatus::Dispatched);

    // 規劃時可見、執行前被隱藏 → 執行期健康閘誠實拒絕（不假裝顯示了氣泡）。
    let mut late_intent = SemanticIntent::new("companion-test");
    late_intent.preferred_channels = vec!["desktop-pet".into()];
    late_intent.message = Some("執行前被隱藏".into());
    let late_plan = rt
        .create_plan(
            late_intent,
            vec!["companion.bubble.show".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    rt.presentation_hello(false, None).await;
    let late = rt
        .execute_plan(
            &late_plan.plan_id,
            interaction_policy::ActionSource::ExplicitRequest,
            false,
        )
        .await
        .unwrap();
    assert_eq!(late[0].current_status, ActionStatus::Blocked);
}

#[tokio::test]
async fn legacy_interaction_receptor_gated_by_companion_presence() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let mut facts = BTreeMap::new();
    facts.insert("kind".to_string(), json!("clicked"));

    // 從未連線：legacy fallback receptor 也不接受偽造的視窗內互動。
    let err = rt
        .ingest(
            "desktop.companion.interaction",
            facts.clone(),
            BTreeMap::new(),
            1.0,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Unavailable(_)));

    // 已連線但隱藏：同一個閘門（隱藏角色停止感知，不能繞道 legacy id）。
    rt.presentation_hello(false, None).await;
    let err = rt
        .ingest(
            "desktop.companion.interaction",
            facts.clone(),
            BTreeMap::new(),
            1.0,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Unavailable(_)));

    // 可見時恢復接受。
    rt.presentation_hello(true, None).await;
    rt.ingest("desktop.companion.interaction", facts, BTreeMap::new(), 1.0)
        .await
        .expect("visible companion accepts legacy interaction pushes");
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

/// estop **一律**要對角色視窗送出 clear-all：待送佇列是空的不代表畫面是
/// 乾淨的——已經送出去並被 ack 的泡泡／動作還掛在視窗上活著。只有在
/// `cleared` 非空時才通知，那些 transient 就會撐過緊急停止。
#[tokio::test]
async fn estop_always_orders_a_clear_all_even_with_nothing_pending() {
    let (_g, rt) = runtime().await;
    visible_session(&rt).await;
    // 送出一則泡泡並讓視窗 ack：待送佇列因此清空，但畫面上那則還在演。
    let receipts =
        plan_and_execute(&rt, "companion.bubble.show", json!({}), Some("已經顯示了")).await;
    rt.presentation_ack(receipts[0].action_id.as_str(), "displayed", None)
        .await
        .unwrap();
    assert_eq!(
        rt.presentation_status()
            .get("pendingCommands")
            .unwrap()
            .as_u64()
            .unwrap(),
        0,
        "precondition: nothing is queued any more"
    );
    assert_eq!(clear_all_commands(&rt), 0);

    rt.emergency_stop("test", None).await.unwrap();

    let clears = clear_all_commands(&rt);
    assert!(
        clears >= 1,
        "estop must order the companion window to clear whatever is on screen"
    );
    let last = rt
        .events
        .recent(200)
        .into_iter()
        .filter(|e| e.event_type == EventType::PresentationCommand)
        .rfind(|e| e.payload.get("command").and_then(|v| v.as_str()) == Some("clear-all"))
        .unwrap();
    assert_eq!(
        last.payload.get("reason").and_then(|v| v.as_str()),
        Some("emergency-stop")
    );
    assert_eq!(
        last.payload.get("clearedPending").and_then(|v| v.as_u64()),
        Some(0),
        "the record must stay honest: nothing was queued, the order is for the screen"
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
async fn sensitive_presentation_effects_are_consent_gated_and_keep_authoritative_params() {
    let (_g, rt) = runtime().await;
    visible_session(&rt).await;
    for id in [
        "companion.sound.play",
        "companion.speak",
        "companion.window.adjust",
    ] {
        rt.registry
            .set_actuator_enabled(&ActuatorId::new(id), true)
            .await
            .unwrap();
        rt.grant_consent(&format!("actuator:{id}"), None)
            .await
            .unwrap();
    }

    for (id, payload, message, key, expected) in [
        (
            "companion.sound.play",
            json!({"sound": "soft-pop"}),
            None,
            "sound",
            json!("soft-pop"),
        ),
        (
            "companion.speak",
            json!({}),
            Some("需要確認。"),
            "text",
            json!("需要確認。"),
        ),
        (
            "companion.window.adjust",
            json!({"x": 20, "width": 240, "opacity": 0.8}),
            None,
            "opacity",
            json!(0.8),
        ),
    ] {
        let receipts = plan_and_execute(&rt, id, payload, message).await;
        assert_eq!(receipts[0].current_status, ActionStatus::Dispatched);
        let pending = rt
            .presentation_pending_command(receipts[0].action_id.as_str())
            .unwrap();
        assert_eq!(pending["params"][key], expected);
        rt.presentation_ack(receipts[0].action_id.as_str(), "completed", None)
            .await
            .unwrap();
    }

    assert!(rt
        .presentation_pending_command("caller-invented-action")
        .is_err());
}

#[tokio::test]
async fn ack_in_accepted_window_walks_chain_to_completed() {
    let (_g, rt) = runtime().await;
    visible_session(&rt).await;
    let receipts =
        plan_and_execute(&rt, "companion.bubble.show", json!({}), Some("競態氣泡")).await;
    let action_id = receipts[0].action_id.clone();
    let dispatched = rt.store.receipt(&action_id).unwrap();
    assert_eq!(dispatched.current_status, ActionStatus::Dispatched);

    // 重現競態的可觀測狀態：presentation.command 已發、pending 已登記，
    // 但 executor 的 Dispatched persist 尚未落地（store 仍是 Accepted）。
    let mut accepted = dispatched.clone();
    accepted.current_status = ActionStatus::Accepted;
    accepted
        .timestamps
        .retain(|(s, _)| !matches!(s, ActionStatus::Dispatched));
    assert!(rt.store.upsert_receipt(&accepted, "desktop-pet").unwrap());

    // ack 必須合法走完 Accepted→Dispatched→Acknowledged→Completed，
    // 不得無聲 no-op（否則 pending 已被消費、receipt 掛到 TTL Expired）。
    let out = rt
        .presentation_ack(action_id.as_str(), "displayed", None)
        .await
        .unwrap();
    assert_eq!(out.get("status").unwrap().as_str().unwrap(), "completed");
    let stored = rt.store.receipt(&action_id).unwrap();
    assert_eq!(stored.current_status, ActionStatus::Completed);
    assert_eq!(
        stored.verification.expect("evidence attached").verdict,
        VerificationVerdict::AcknowledgedOnly
    );
    // 完整事件階梯有發出（dispatched→acknowledged→completed）。
    let events = rt.events.recent(100);
    for ev in [EventType::ActionAcknowledged, EventType::ActionCompleted] {
        assert!(
            events.iter().any(|e| e.event_type == ev
                && e.payload.get("actionId").and_then(|v| v.as_str()) == Some(action_id.as_str())),
            "missing {ev:?} for {action_id}"
        );
    }

    // executor 遲到的 Dispatched persist 被 sticky-terminal 守衛拒絕，
    // 不能把已完成的 receipt 倒轉回 Dispatched（receipt 不孤兒化）。
    assert!(!rt.store.upsert_receipt(&dispatched, "desktop-pet").unwrap());
    assert_eq!(
        rt.store.receipt(&action_id).unwrap().current_status,
        ActionStatus::Completed
    );
}

#[tokio::test]
async fn ack_after_terminal_sweep_emits_no_lifecycle_events() {
    let (_g, rt) = runtime().await;
    visible_session(&rt).await;
    let receipts =
        plan_and_execute(&rt, "companion.bubble.show", json!({}), Some("停止競態")).await;
    let action_id = receipts[0].action_id.clone();

    // 重現 estop 掃描先把 receipt 寫成終態、presentation actuator 還沒
    // 清掉 pending 的窗口（estop 先寫 open receipts、之後才輪到 actuator
    // 的 emergency_stop 清佇列）。函式中段的交錯無法黑箱重現；本測試
    // 釘住 guarded persist-then-emit 的可觀測契約：persist 被拒 → 不發
    // 任何 lifecycle 事件、以 store 終態誠實回報。
    let mut stopped = rt.store.receipt(&action_id).unwrap();
    stopped
        .transition(ActionStatus::Stopped, chrono::Utc::now())
        .unwrap();
    assert!(rt.store.upsert_receipt(&stopped, "desktop-pet").unwrap());

    let out = rt
        .presentation_ack(action_id.as_str(), "displayed", None)
        .await
        .unwrap();
    assert_eq!(out.get("status").unwrap().as_str().unwrap(), "stopped");
    assert_eq!(
        rt.store.receipt(&action_id).unwrap().current_status,
        ActionStatus::Stopped
    );
    // /v1/events 訂閱者絕不能看到這個動作的 acknowledged/completed。
    let events = rt.events.recent(200);
    assert!(
        events.iter().all(|e| {
            let same_action =
                e.payload.get("actionId").and_then(|v| v.as_str()) == Some(action_id.as_str());
            !(same_action
                && matches!(
                    e.event_type,
                    EventType::ActionAcknowledged | EventType::ActionCompleted
                ))
        }),
        "spurious lifecycle event emitted for a terminalized receipt"
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

/// 純呈現（L0）動作的「結果未知」是角色演出沒被確認，不是需要人類裁決的
/// 外部副作用。它必須仍留在歷史裡，但不得灌爆右上角的「待我決定」。
/// 同樣是 uncertain，實體通道（haptic）則一定要人看見。
#[tokio::test]
async fn desktop_pet_uncertain_is_not_a_pending_decision_but_haptic_is() {
    let (_g, rt) = runtime().await;
    visible_session(&rt).await;
    // iPhone 動器也要在場：`iphone.character` 同樣走 desktop-pet 通道，
    // 但它是送到另一台實體裝置的外部副作用，不能被角色演出的豁免蓋掉。
    rt.mobile_ensure_started().await.unwrap();

    // 真的走完 presentation 閉環：命令送出、沒人 ack、watchdog 掃成 Uncertain。
    let receipts =
        plan_and_execute(&rt, "companion.bubble.show", json!({}), Some("沒人回應")).await;
    let pet_action = receipts[0].action_id.clone();
    let later = chrono::Utc::now()
        + chrono::Duration::milliseconds(interaction_runtime::presentation::ACK_TTL_MS + 500);
    rt.sweep_presentation_at(later).await;
    assert_eq!(
        rt.store.receipt(&pet_action).unwrap().current_status,
        ActionStatus::Uncertain
    );

    // 對照組：同樣 Uncertain，但落在 haptic 通道的 mock 裝置動器上。
    let haptic = ActionReceipt {
        action_id: ActionId::new("act-haptic-uncertain"),
        plan_id: PlanId::new("plan-haptic"),
        session_id: SessionId::new("sess-haptic"),
        actuator_id: ActuatorId::new("mock.actuator"),
        intent: "震動提醒".into(),
        requested_parameters: ActionParameters::default(),
        effective_bounded_parameters: ActionParameters::default(),
        policy_decisions: vec![],
        current_status: ActionStatus::Uncertain,
        timestamps: vec![(ActionStatus::Uncertain, chrono::Utc::now())],
        errors: vec![],
        driver_response: BTreeMap::new(),
        verification: None,
        expires_at: None,
        correlation_id: CorrelationId::new("corr-haptic"),
        schema_version: SCHEMA_VERSION.to_string(),
    };
    assert!(rt.store.upsert_receipt(&haptic, "haptic").unwrap());

    // 對照組二：同樣 Uncertain、同樣 desktop-pet 通道，但落在 iPhone 上。
    let phone = ActionReceipt {
        action_id: ActionId::new("act-iphone-character-uncertain"),
        actuator_id: ActuatorId::new("iphone.character"),
        intent: "手機角色狀態".into(),
        correlation_id: CorrelationId::new("corr-iphone"),
        ..haptic.clone()
    };
    assert!(rt.store.upsert_receipt(&phone, "iphone").unwrap());

    let inbox = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter::default())
        .await
        .unwrap();
    let items = inbox["items"].as_array().unwrap();
    let needs = |action_id: &str| -> bool {
        items
            .iter()
            .find(|item| item["itemId"].as_str() == Some(action_id))
            .unwrap_or_else(|| panic!("{action_id} missing from the inbox history"))
            ["needsDecision"]
            .as_bool()
            .unwrap()
    };
    assert!(
        !needs(pet_action.as_str()),
        "desktop-pet uncertain must not become a pending human decision"
    );
    assert!(
        needs("act-haptic-uncertain"),
        "haptic uncertain must stay a pending human decision"
    );
    assert!(
        needs("act-iphone-character-uncertain"),
        "iphone.character uncertain must stay a pending human decision"
    );
    assert_eq!(inbox["pendingCount"].as_u64(), Some(2));
}
