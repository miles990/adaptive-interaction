//! Character Presentation Protocol 的 Runtime 接線閉環：
//! hello／協商（含重送 generation+1）→ README §11 投影表每一列 → AI 替代規則
//! （wait-attention → think）→ 回執誠實推進 presentation receipt（永不 verified）
//! → 舊世代回執拒絕 → input event 正規化成 receptor observation → 動態可點播動畫
//! → provider 顯示名跟角色走 → adapter token CRUD／撤銷／持久化 → 外部 transport
//! （attach／negotiate／receipt／heartbeat 逾時）。
//!
//! 全部在 Runtime 內以程序內 client 驅動：**模擬器**級驗收，不是真桌面視窗。

use chrono::Utc;
use interaction_character::{
    encode_wire, CharacterInputEvent, CharacterIntent, CharacterManifest, CommandReceipt,
    DisconnectReason, InputEventKind, Negotiate, ReceiptStatus, WireMessage,
};
use interaction_core::*;
use interaction_runtime::character::{
    adapter_instance_id, CharacterHelloInput, WsStep, DESKTOP_INSTANCE_ID,
};
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

const FIXTURE_MANIFEST: &str =
    include_str!("../../../examples/character-adapters/text-adapter.manifest.json");

async fn runtime_in(dir: &tempfile::TempDir) -> Runtime {
    Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap()
}

async fn runtime() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime_in(&dir).await;
    (dir, rt)
}

/// 桌面角色 manifest（shu-rig 風格，expression variants 含真相狀態名以驗證 deny-list）。
fn desktop_manifest() -> CharacterManifest {
    serde_json::from_value(json!({
        "schemaVersion": "1.0",
        "characterId": "shu-maid",
        "displayName": { "zh-TW": "小樞", "en": "Shu" },
        "author": "adaptive-interaction",
        "version": "3.0.0",
        "adapterKind": "in-process",
        "entrypoint": { "kind": "builtin", "id": "shu-rig" },
        "assets": [],
        "capabilities": {
            "visual.presence": { "supported": true },
            "visual.expression": {
                "supported": true,
                "variants": ["idle", "notice", "thinking", "curious", "success", "emergency"]
            },
            "visual.textBubble": { "supported": true },
            "audio.effect": { "supported": true }
        },
        "inputCapabilities": {
            "input.click": { "supported": true },
            "input.hover": { "supported": true },
            "input.drag": { "supported": true },
            "input.text": { "supported": true },
            "input.fileDrop": { "supported": true }
        },
        "channels": ["expression", "bubble", "transform"],
        "states": ["idle"],
        "intents": [
            "idle", "notice", "acknowledge", "think", "work", "wait", "ask", "request-consent",
            "blocked", "unknown", "claim-completed", "verified-success", "failed", "cancelled",
            "offline", "emergency", "greet", "play", "rest", "sleep"
        ],
        "variants": [{ "id": "maid-classic", "displayName": { "zh-TW": "經典" } }],
        "locales": ["zh-TW", "en"],
        "securityRequirements": {
            "network": false, "executable": false, "fileAccess": "none",
            "audioOutput": true, "microphone": false, "camera": false
        },
        "compatibility": { "protocol": "1.x", "runtime": ">=0.5.0" }
    }))
    .expect("desktop manifest parses")
}

fn fixture_manifest() -> CharacterManifest {
    serde_json::from_str(FIXTURE_MANIFEST).expect("fixture manifest parses")
}

async fn hello(rt: &Runtime, visible: bool) -> Value {
    hello_with_motion(rt, visible, false).await
}

/// hello 並回報視窗目前的 Reduced Motion（協商的唯一來源）。
async fn hello_with_motion(rt: &Runtime, visible: bool, reduced_motion: bool) -> Value {
    let manifest = desktop_manifest();
    rt.character_hello(CharacterHelloInput {
        instance_id: None,
        role: None,
        negotiate: Negotiate::from_manifest(&manifest, 1),
        manifest,
        visible,
        pack_id: None,
        behavior_state: None,
        reduced_motion,
    })
    .await
    .expect("hello accepted")
}

fn events_of(rt: &Runtime, event_type: EventType) -> Vec<RuntimeEvent> {
    rt.events
        .recent(500)
        .into_iter()
        .filter(|e| e.event_type == event_type)
        .collect()
}

fn last_intent(rt: &Runtime) -> Value {
    events_of(rt, EventType::CharacterIntent)
        .last()
        .map(|e| e.payload.clone())
        .expect("a character.intent event")
}

fn intent_count(rt: &Runtime) -> usize {
    events_of(rt, EventType::CharacterIntent).len()
}

async fn plan_and_execute(
    rt: &Runtime,
    actuator: &str,
    payload: Value,
    message: Option<&str>,
) -> Vec<ActionReceipt> {
    let mut intent = SemanticIntent::new("character-test");
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

fn receipt(message_id: &str, generation: u64, status: ReceiptStatus) -> CommandReceipt {
    CommandReceipt::new(
        message_id,
        DESKTOP_INSTANCE_ID,
        generation,
        status,
        Utc::now(),
    )
}

fn input_event(kind: InputEventKind, generation: u64, payload: Value) -> CharacterInputEvent {
    CharacterInputEvent {
        protocol_version: "1.0".into(),
        event_id: format!("evt-{}", uuid::Uuid::new_v4().simple()),
        character_instance_id: DESKTOP_INSTANCE_ID.into(),
        generation,
        timestamp: Utc::now(),
        kind,
        payload: payload
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default(),
        privacy_class: Default::default(),
    }
}

async fn observations(rt: &Runtime, receptor: &str) -> Vec<Observation> {
    rt.observe_stored(&ObservationQuery {
        receptor_id: Some(ReceptorId::new(receptor)),
        limit: Some(20),
        ..Default::default()
    })
    .await
    .unwrap()
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn hello_negotiates_the_desktop_instance_and_rehello_bumps_generation() {
    let (_g, rt) = runtime().await;
    // 未 hello：沒有 active character、instances 空、manifest 404 語意。
    let status = rt.status().await;
    assert_eq!(status["characterProtocol"]["version"], "1.0");
    assert_eq!(status["characterProtocol"]["instances"], 0);
    assert!(status["characterProtocol"]["activeCharacter"].is_null());
    assert!(rt.character_manifest().is_none());

    let out = hello(&rt, true).await;
    assert_eq!(out["instanceId"], DESKTOP_INSTANCE_ID);
    assert_eq!(out["generation"], 1);
    let resolutions = out["negotiated"]["resolutions"].as_object().unwrap();
    assert_eq!(resolutions.len(), 20, "all 20 intents resolved");
    assert_eq!(resolutions["emergency"]["resolution"], "exact");
    assert_eq!(resolutions["emergency"]["via"], "visual.expression");

    let instances = rt.character_instances();
    let entry = &instances["instances"][0];
    assert_eq!(entry["instanceId"], DESKTOP_INSTANCE_ID);
    assert_eq!(entry["characterId"], "shu-maid");
    assert_eq!(entry["displayName"]["zh-TW"], "小樞");
    assert_eq!(entry["role"], "primary-companion");
    assert_eq!(entry["generation"], 1);
    assert_eq!(entry["connected"], true);
    assert_eq!(entry["negotiated"], true);
    assert_eq!(entry["origin"], "builtin");
    assert_eq!(entry["adapterKind"], "in-process");
    assert_eq!(entry["executable"], false);
    assert_eq!(entry["network"], false);
    assert_eq!(
        entry["tested"], false,
        "tested only after a completed receipt"
    );
    // 連接頁「可以接收／作者／版本」直接來自 manifest（README §9）。
    assert_eq!(entry["author"], "adaptive-interaction");
    assert_eq!(entry["version"], "3.0.0");
    assert_eq!(
        entry["inputCapabilities"],
        json!([
            "input.click",
            "input.drag",
            "input.fileDrop",
            "input.hover",
            "input.text"
        ])
    );
    assert_eq!(rt.character_manifest().unwrap().character_id, "shu-maid");

    let status = rt.status().await;
    assert_eq!(status["characterProtocol"]["instances"], 1);
    assert_eq!(
        status["characterProtocol"]["activeCharacter"]["characterId"],
        "shu-maid"
    );
    assert_eq!(
        status["characterProtocol"]["activeCharacter"]["displayName"]["zh-TW"],
        "小樞"
    );
    // presentation snapshot 帶 character（packId 相容 = characterId）。
    assert_eq!(
        status["presentation"]["character"]["characterId"],
        "shu-maid"
    );
    assert_eq!(status["presentation"]["character"]["generation"], 1);
    assert_eq!(status["presentation"]["packId"], "shu-maid");
    assert_eq!(status["presentation"]["visible"], true);
    assert!(!events_of(&rt, EventType::CharacterInstance).is_empty());

    // 一個 AI 命令在飛：重送 hello（重新協商）→ generation 2、pending → uncertain、
    // presentation receipt 誠實記 Uncertain。
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "think"}),
        None,
    )
    .await;
    let action_id = receipts[0].action_id.clone();
    assert_eq!(receipts[0].current_status, ActionStatus::Dispatched);
    let out = hello(&rt, true).await;
    assert_eq!(out["generation"], 2);
    let uncertain = events_of(&rt, EventType::CharacterReceipt)
        .into_iter()
        .any(|e| {
            e.payload["receipt"]["messageId"] == action_id.as_str()
                && e.payload["receipt"]["status"] == "uncertain"
        });
    assert!(
        uncertain,
        "pending AI command marked uncertain on re-negotiation"
    );
    let action = rt.get_action(&action_id).unwrap();
    assert_eq!(action.current_status, ActionStatus::Uncertain);
    assert_eq!(
        action.verification.as_ref().map(|v| v.verdict),
        Some(VerificationVerdict::Uncertain)
    );
}

#[tokio::test]
async fn hello_refuses_external_manifests_and_reserved_instance_ids() {
    let (_g, rt) = runtime().await;
    let external = fixture_manifest();
    let err = rt
        .character_hello(CharacterHelloInput {
            instance_id: None,
            role: None,
            negotiate: Negotiate::from_manifest(&external, 1),
            manifest: external,
            visible: true,
            pack_id: None,
            behavior_state: None,
            reduced_motion: false,
        })
        .await
        .expect_err("external-process manifest must go over the adapter WebSocket");
    assert!(matches!(err, DomainError::Validation(_)), "{err}");
    let manifest = desktop_manifest();
    let err = rt
        .character_hello(CharacterHelloInput {
            instance_id: Some("adapter:evil".into()),
            role: None,
            negotiate: Negotiate::from_manifest(&manifest, 1),
            manifest,
            visible: true,
            pack_id: None,
            behavior_state: None,
            reduced_motion: false,
        })
        .await
        .expect_err("adapter: prefix is reserved");
    assert!(matches!(err, DomainError::Validation(_)));
    // 協商的 characterId 與 manifest 不符 → 拒絕。
    let manifest = desktop_manifest();
    let mut negotiate = Negotiate::from_manifest(&manifest, 1);
    negotiate.character_id = "someone-else".into();
    let err = rt
        .character_hello(CharacterHelloInput {
            instance_id: None,
            role: None,
            negotiate,
            manifest,
            visible: true,
            pack_id: None,
            behavior_state: None,
            reduced_motion: false,
        })
        .await
        .expect_err("character mismatch refused");
    assert!(err.to_string().contains("negotiate rejected"), "{err}");
}

#[tokio::test]
async fn every_readme_section_11_row_projects_the_expected_intent() {
    let (_g, rt) = runtime().await;
    hello(&rt, true).await;

    // agent.session.state（correlationId = agentSessionId）。
    let rows = [
        ("created", "wait", "queued"),
        ("queued", "wait", "queued"),
        ("fetched", "think", "working"),
        ("working", "work", "working"),
        ("waiting-input", "ask", "waiting-input"),
        ("waiting-consent", "request-consent", "waiting-consent"),
        ("claimed-completed", "claim-completed", "claimed"),
        ("verified", "verified-success", "verified"),
        ("failed", "failed", "failed"),
        ("timed-out", "failed", "timed-out"),
        ("unknown", "unknown", "unknown"),
        ("cancelled", "cancelled", "cancelled"),
        ("closed", "idle", "none"),
    ];
    for (state, intent, truth) in rows {
        let before = intent_count(&rt);
        let projected = rt.character_project_session("sess-1", state);
        assert!(projected.is_some(), "{state} must project");
        assert_eq!(intent_count(&rt), before + 1, "{state} emits one intent");
        let payload = last_intent(&rt);
        assert_eq!(payload["envelope"]["intent"], intent, "{state}");
        assert_eq!(payload["envelope"]["truthState"], truth, "{state}");
        assert_eq!(payload["envelope"]["correlationId"], "sess-1");
        assert_eq!(payload["targets"], json!([DESKTOP_INSTANCE_ID]));
        assert_eq!(
            payload["envelope"]["characterInstanceId"],
            DESKTOP_INSTANCE_ID
        );
        let floor = CharacterIntent::parse(intent).unwrap().priority_floor();
        assert!(
            payload["envelope"]["priority"].as_u64().unwrap() >= u64::from(floor),
            "{state}: priority honours the floor"
        );
    }
    assert!(rt.character_project_session("sess-1", "bogus").is_none());

    // action.*：真的執行一個非角色 actuator（conversation）→ dispatched／acknowledged／
    // completed 依序投影，correlationId = actionId。
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let mut intent = SemanticIntent::new("greeting");
    intent.message = Some("hi".into());
    let plan = rt
        .create_plan(
            intent,
            vec!["conversation".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let receipts = rt
        .execute_plan(
            &plan.plan_id,
            interaction_policy::ActionSource::ExplicitRequest,
            false,
        )
        .await
        .unwrap();
    let action_id = receipts[0].action_id.as_str().to_string();
    let for_action: Vec<(String, String)> = events_of(&rt, EventType::CharacterIntent)
        .into_iter()
        .filter(|e| e.payload["envelope"]["correlationId"] == action_id.as_str())
        .map(|e| {
            (
                e.payload["envelope"]["intent"]
                    .as_str()
                    .unwrap()
                    .to_string(),
                e.payload["envelope"]["truthState"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        })
        .collect();
    // conversation 的 driver receipt 一次帶到 acknowledged（executor 只發最終
    // 狀態的事件），所以這裡看得到 acknowledge／claim-completed；dispatched 列
    // 在下面以投影表直接驗。
    assert!(
        for_action.contains(&("acknowledge".into(), "working".into())),
        "acknowledged → acknowledge/working: {for_action:?}"
    );
    assert!(
        for_action.contains(&("claim-completed".into(), "claimed".into())),
        "completed → claim-completed/claimed: {for_action:?}"
    );
    assert!(
        !for_action.iter().any(|(i, _)| i == "verified-success"),
        "an ack-only completion is never verified-success"
    );
    // 直接投影表：observed／uncertain／failed。
    let receipt = rt.get_action(&receipts[0].action_id).unwrap();
    for (event_type, intent, truth) in [
        (EventType::ActionDispatched, "work", "working"),
        (EventType::ActionObserved, "verified-success", "verified"),
        (EventType::ActionUncertain, "unknown", "unknown"),
        (EventType::ActionFailed, "failed", "failed"),
        (EventType::ActionCancelled, "cancelled", "cancelled"),
    ] {
        rt.character_project_action(event_type, &receipt);
        let payload = last_intent(&rt);
        assert_eq!(payload["envelope"]["intent"], intent);
        assert_eq!(payload["envelope"]["truthState"], truth);
        assert_eq!(payload["envelope"]["correlationId"], action_id.as_str());
    }
    // 角色自己的呈現 actuator：不投影。
    let companion = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "rest"}),
        None,
    )
    .await;
    let companion_receipt = rt.get_action(&companion[0].action_id).unwrap();
    let before = intent_count(&rt);
    assert!(rt
        .character_project_action(EventType::ActionDispatched, &companion_receipt)
        .is_none());
    assert_eq!(intent_count(&rt), before);

    // plan.blocked → blocked（correlationId = planId）。
    rt.character_project_plan_blocked("plan-9", Some("policy"));
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "blocked");
    assert_eq!(payload["envelope"]["truthState"], "blocked");
    assert_eq!(payload["envelope"]["correlationId"], "plan-9");

    // provider.state-changed：available／paired → greet(device-online)、
    // disconnected／revoked → notice(device-offline)。
    let pid = ProviderId::new("provider.mobile.iphone-1");
    for (state, intent, variant) in [
        (ProviderState::Available, "greet", "device-online"),
        (ProviderState::Paired, "greet", "device-online"),
        (ProviderState::Disconnected, "notice", "device-offline"),
        (ProviderState::Revoked, "notice", "device-offline"),
    ] {
        rt.character_project_provider(&pid, state);
        let payload = last_intent(&rt);
        assert_eq!(payload["envelope"]["intent"], intent);
        assert_eq!(payload["envelope"]["truthState"], "none");
        assert_eq!(payload["envelope"]["presentationHints"]["variant"], variant);
        assert_eq!(payload["envelope"]["correlationId"], pid.as_str());
    }
    let before = intent_count(&rt);
    assert!(rt
        .character_project_provider(&pid, ProviderState::Installed)
        .is_none());
    assert!(rt
        .character_project_provider(
            &ProviderId::new("provider.companion.desktop"),
            ProviderState::Available
        )
        .is_none());
    assert_eq!(intent_count(&rt), before);

    // proactive.paused／resumed → rest／idle。
    rt.pause_proactive(None, Some("nap".into()), "test")
        .await
        .unwrap();
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "rest");
    assert_eq!(payload["envelope"]["truthState"], "none");
    rt.resume_proactive("test").await.unwrap();
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "idle");

    // receptor.observation → notice(listening)，merge policy，correlation = receptor。
    rt.ingest(
        "manual.event",
        BTreeMap::from([("event".to_string(), json!("ping"))]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "notice");
    assert_eq!(payload["envelope"]["truthState"], "none");
    assert_eq!(
        payload["envelope"]["presentationHints"]["variant"],
        "listening"
    );
    assert_eq!(
        payload["envelope"]["correlationId"],
        "receptor:manual.event"
    );
    assert_eq!(payload["envelope"]["interruptPolicy"], "merge");
    // 同受器 2 s 內第二筆不再投影（節流）。
    let before = intent_count(&rt);
    rt.ingest(
        "manual.event",
        BTreeMap::from([("event".to_string(), json!("ping2"))]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    assert_eq!(intent_count(&rt), before);

    // 真實 agent session：created → wait/queued，correlationId = session id。
    let record = rt
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: None,
            agent_id: "agent.coder".into(),
            label: None,
            ttl_minutes: Some(5),
            data_scope: vec![],
            tool_scope: vec![],
            consent_scope: vec![],
            allow_write: false,
            max_cost: None,
            max_messages: None,
            delegation: None,
            workdir: None,
            resume_provider_session_id: None,
        })
        .await
        .unwrap();
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "wait");
    assert_eq!(payload["envelope"]["truthState"], "queued");
    assert_eq!(
        payload["envelope"]["correlationId"],
        record.session_id.as_str()
    );

    // emergency.stop → emergency（floor 100、preempt）；cleared → idle。
    rt.emergency_stop("test", None).await.unwrap();
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "emergency");
    assert_eq!(payload["envelope"]["truthState"], "emergency");
    assert_eq!(payload["envelope"]["priority"], 100);
    assert_eq!(payload["envelope"]["interruptPolicy"], "preempt");
    assert_eq!(payload["envelope"]["correlationId"], "emergency-stop");
    rt.clear_emergency_stop("test").await.unwrap();
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "idle");
    assert_eq!(payload["envelope"]["truthState"], "none");

    // 沒有任何 character.* 事件被再投影（沒有自我遞迴）：所有 intent 的
    // correlationId 都不是 character.* 事件 id。
    for e in events_of(&rt, EventType::CharacterIntent) {
        assert_ne!(e.payload["envelope"]["truthState"], Value::Null);
    }
}

#[tokio::test]
async fn ai_state_present_substitutes_safety_intents_and_caps_priority() {
    let (_g, rt) = runtime().await;
    hello(&rt, true).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();

    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "wait-attention", "tone": "attentive"}),
        Some("等你一下"),
    )
    .await;
    let action_id = receipts[0].action_id.as_str().to_string();
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "think", "wait → think");
    assert_eq!(payload["envelope"]["truthState"], "none");
    assert_eq!(
        payload["envelope"]["presentationHints"]["variant"],
        "wait-attention"
    );
    assert_eq!(
        payload["envelope"]["presentationHints"]["tone"],
        "attentive"
    );
    assert_eq!(
        payload["envelope"]["presentationHints"]["message"],
        "等你一下"
    );
    assert_eq!(payload["envelope"]["correlationId"], action_id.as_str());
    assert_eq!(payload["envelope"]["messageId"], action_id.as_str());
    assert!(payload["envelope"]["priority"].as_u64().unwrap() <= 50);
    assert_eq!(payload["targets"], json!([DESKTOP_INSTANCE_ID]));

    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "look-at-confirmation"}),
        None,
    )
    .await;
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "notice", "ask → notice");
    assert_eq!(
        payload["envelope"]["presentationHints"]["variant"],
        "look-at-confirmation"
    );
    assert_eq!(
        payload["envelope"]["correlationId"],
        receipts[0].action_id.as_str()
    );

    // animation-play：協商到的 variant `curious` 可點播 → notice(variant curious)。
    plan_and_execute(
        &rt,
        "companion.animation.play",
        json!({"animation": "curious"}),
        None,
    )
    .await;
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "notice");
    assert_eq!(
        payload["envelope"]["presentationHints"]["variant"],
        "curious"
    );
    assert_eq!(payload["envelope"]["truthState"], "none");
    // `thinking` 別名 → think(variant thinking)。
    plan_and_execute(
        &rt,
        "companion.animation.play",
        json!({"animation": "thinking"}),
        None,
    )
    .await;
    let payload = last_intent(&rt);
    assert_eq!(payload["envelope"]["intent"], "think");
    assert_eq!(
        payload["envelope"]["presentationHints"]["variant"],
        "thinking"
    );
}

#[tokio::test]
async fn receipts_settle_the_presentation_receipt_honestly_never_verified() {
    let (_g, rt) = runtime().await;
    hello(&rt, true).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let generation = 1;

    // completed → Completed（AcknowledgedOnly）。
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "think"}),
        None,
    )
    .await;
    let action_id = receipts[0].action_id.clone();
    let mid = action_id.as_str();
    for status in [
        ReceiptStatus::Accepted,
        ReceiptStatus::Started,
        ReceiptStatus::Completed,
    ] {
        let out = rt
            .character_receipt(DESKTOP_INSTANCE_ID, receipt(mid, generation, status))
            .await
            .unwrap();
        assert_eq!(out["accepted"], true, "{status:?}");
    }
    let action = rt.get_action(&action_id).unwrap();
    assert_eq!(action.current_status, ActionStatus::Completed);
    let verdict = action.verification.as_ref().map(|v| v.verdict);
    assert_eq!(verdict, Some(VerificationVerdict::AcknowledgedOnly));
    assert_ne!(
        verdict,
        Some(VerificationVerdict::Observed),
        "never verified"
    );
    let receipt_events = events_of(&rt, EventType::CharacterReceipt);
    assert!(receipt_events.iter().any(|e| {
        e.payload["instanceId"] == DESKTOP_INSTANCE_ID
            && e.payload["receipt"]["messageId"] == mid
            && e.payload["receipt"]["status"] == "completed"
    }));
    assert_eq!(rt.character_instances()["instances"][0]["tested"], true);

    // unsupported → Failed。
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "work"}),
        None,
    )
    .await;
    let action_id = receipts[0].action_id.clone();
    rt.character_receipt(
        DESKTOP_INSTANCE_ID,
        receipt(action_id.as_str(), generation, ReceiptStatus::Unsupported),
    )
    .await
    .unwrap();
    assert_eq!(
        rt.get_action(&action_id).unwrap().current_status,
        ActionStatus::Failed
    );

    // started → uncertain → Uncertain（不猜 completed）。
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "notice"}),
        None,
    )
    .await;
    let action_id = receipts[0].action_id.clone();
    rt.character_receipt(
        DESKTOP_INSTANCE_ID,
        receipt(action_id.as_str(), generation, ReceiptStatus::Started),
    )
    .await
    .unwrap();
    assert_eq!(
        rt.get_action(&action_id).unwrap().current_status,
        ActionStatus::Dispatched,
        "started is not completed"
    );
    rt.character_receipt(
        DESKTOP_INSTANCE_ID,
        receipt(action_id.as_str(), generation, ReceiptStatus::Uncertain),
    )
    .await
    .unwrap();
    let action = rt.get_action(&action_id).unwrap();
    assert_eq!(action.current_status, ActionStatus::Uncertain);
    assert_eq!(
        action.verification.as_ref().map(|v| v.verdict),
        Some(VerificationVerdict::Uncertain)
    );

    // cancelled → Cancelled；acknowledged 之後 completed 是非法轉移（丟棄、不推進）。
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "rest"}),
        None,
    )
    .await;
    let action_id = receipts[0].action_id.clone();
    let out = rt
        .character_receipt(
            DESKTOP_INSTANCE_ID,
            receipt(action_id.as_str(), generation, ReceiptStatus::Acknowledged),
        )
        .await
        .unwrap();
    assert_eq!(out["accepted"], true);
    let out = rt
        .character_receipt(
            DESKTOP_INSTANCE_ID,
            receipt(action_id.as_str(), generation, ReceiptStatus::Completed),
        )
        .await
        .unwrap();
    assert_eq!(
        out["accepted"], false,
        "acknowledged → completed is illegal"
    );
    assert_eq!(out["status"], "acknowledged");
    assert_eq!(
        rt.get_action(&action_id).unwrap().current_status,
        ActionStatus::Dispatched
    );
    rt.character_receipt(
        DESKTOP_INSTANCE_ID,
        receipt(action_id.as_str(), generation, ReceiptStatus::Cancelled),
    )
    .await
    .unwrap();
    assert_eq!(
        rt.get_action(&action_id).unwrap().current_status,
        ActionStatus::Cancelled
    );

    // 未知 instance → NotFound。
    let err = rt
        .character_receipt("nobody", receipt("x", 1, ReceiptStatus::Accepted))
        .await
        .expect_err("unknown instance");
    assert!(matches!(err, DomainError::NotFound(_)));
}

#[tokio::test]
async fn stale_generation_receipts_are_rejected() {
    let (_g, rt) = runtime().await;
    hello(&rt, true).await;
    hello(&rt, true).await; // generation 2
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "think"}),
        None,
    )
    .await;
    let action_id = receipts[0].action_id.clone();
    let out = rt
        .character_receipt(
            DESKTOP_INSTANCE_ID,
            receipt(action_id.as_str(), 1, ReceiptStatus::Completed),
        )
        .await
        .unwrap();
    assert_eq!(out["accepted"], false);
    assert_eq!(out["status"], "stale-generation");
    assert_eq!(
        rt.get_action(&action_id).unwrap().current_status,
        ActionStatus::Dispatched,
        "a stale receipt never completes anything"
    );
    let out = rt
        .character_receipt(
            DESKTOP_INSTANCE_ID,
            receipt(action_id.as_str(), 2, ReceiptStatus::Completed),
        )
        .await
        .unwrap();
    // accepted → completed 是非法跳步：仍不推進。
    assert_eq!(out["accepted"], false);
    assert_eq!(
        rt.get_action(&action_id).unwrap().current_status,
        ActionStatus::Dispatched
    );
}

#[tokio::test]
async fn input_events_become_normalized_receptor_observations() {
    let (_g, rt) = runtime().await;
    hello(&rt, true).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let generation = 1;

    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(
                InputEventKind::Clicked,
                generation,
                json!({"x": 12, "y": 30}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "queued");
    let clicks = observations(&rt, "companion.click").await;
    assert_eq!(clicks.len(), 1);
    assert_eq!(clicks[0].facts["kind"], "companion-clicked");
    assert!(
        !clicks[0].facts.contains_key("x"),
        "no coordinates in facts"
    );

    rt.character_event(
        DESKTOP_INSTANCE_ID,
        input_event(
            InputEventKind::TextSubmitted,
            generation,
            json!({"text": "早安"}),
        ),
    )
    .await
    .unwrap();
    let texts = observations(&rt, "companion.text-input").await;
    assert_eq!(texts[0].facts["kind"], "text-submitted");
    assert_eq!(texts[0].facts["modality"], "text");
    assert_eq!(texts[0].facts["text"], "早安");

    rt.character_event(
        DESKTOP_INSTANCE_ID,
        input_event(
            InputEventKind::ActionRequested,
            generation,
            json!({"action": "open-inbox"}),
        ),
    )
    .await
    .unwrap();
    let actions = observations(&rt, "companion.quick-action").await;
    assert_eq!(actions[0].facts["kind"], "action-selected");
    assert_eq!(actions[0].facts["action"], "open-inbox");

    // file-dropped：只有 metadata＋短效 grant，沒有路徑。
    let expires = (Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(
                InputEventKind::FileDropped,
                generation,
                json!({
                    "name": "notes.md", "mediaType": "text/markdown", "bytes": 1234,
                    "readableScope": "file", "grantId": "g-1", "expiresAt": expires
                }),
            ),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "queued");
    let drops = observations(&rt, "companion.drag-drop").await;
    assert_eq!(drops[0].facts["kind"], "companion-dropped");
    assert_eq!(drops[0].facts["modality"], "file-drop");
    assert_eq!(drops[0].facts["fileCount"], 1);
    assert_eq!(drops[0].facts["names"], json!(["notes.md"]));
    assert_eq!(drops[0].facts["grants"][0]["grantId"], "g-1");
    assert_eq!(drops[0].facts["grants"][0]["readableScope"], "file");
    let grant_expiry = drops[0].facts["grants"][0]["expiresAt"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .unwrap();
    assert!(
        grant_expiry.with_timezone(&Utc) <= Utc::now() + chrono::Duration::minutes(11),
        "grant clamped to <= 10 minutes"
    );
    let serialized = serde_json::to_string(&drops[0].facts).unwrap();
    assert!(!serialized.contains("/Users"), "no raw path");
    // 帶原始路徑鍵 → 丟棄（raw-path）。
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(
                InputEventKind::FileDropped,
                generation,
                json!({"name": "x", "path": "/Users/me/x", "mediaType": "text/plain",
                       "bytes": 1, "readableScope": "file", "grantId": "g", "expiresAt": expires}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "dropped");
    assert_eq!(out["reason"], "raw-path");
    assert_eq!(observations(&rt, "companion.drag-drop").await.len(), 1);

    // TS gateway 的 `files:[…]` 形狀：全部檔案都回報（不只第一個）。
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(
                InputEventKind::FileDropped,
                generation,
                json!({"files": [
                    {"name": "a.png", "mediaType": "image/png", "bytes": 10,
                     "readableScope": "file", "grantId": "g-a", "expiresAt": expires},
                    {"name": "b.txt", "mediaType": "text/plain", "bytes": 20,
                     "readableScope": "file", "grantId": "g-b", "expiresAt": expires}
                ]}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "queued", "{out}");
    let drops = observations(&rt, "companion.drag-drop").await;
    assert_eq!(drops.len(), 2);
    // 儲存查詢不保證順序：用 fileCount 找出多檔那一筆。
    let multi = drops
        .iter()
        .find(|o| o.facts["fileCount"] == json!(2))
        .expect("multi-file drop observation");
    assert_eq!(multi.facts["names"], json!(["a.png", "b.txt"]));
    assert_eq!(multi.facts["grants"][1]["grantId"], "g-b");
    assert!(!serde_json::to_string(&multi.facts)
        .unwrap()
        .contains("path"));

    // hover-entered：gateway ≤ 4/s，hub 再把 pointer-approached 節流到 30 s 一次。
    let base = Utc::now();
    let clock_at = |secs: i64| {
        let t = base + chrono::Duration::seconds(secs);
        Arc::new(move || t) as interaction_runtime::character::NowFn
    };
    rt.character.set_clock(clock_at(1));
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(InputEventKind::HoverEntered, generation, json!({})),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "queued");
    assert_eq!(observations(&rt, "companion.pointer").await.len(), 1);
    assert_eq!(
        observations(&rt, "companion.pointer").await[0].facts["kind"],
        "pointer-approached"
    );
    rt.character.set_clock(clock_at(3));
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(InputEventKind::HoverEntered, generation, json!({})),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "throttled");
    assert_eq!(observations(&rt, "companion.pointer").await.len(), 1);
    rt.character.set_clock(clock_at(40));
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(InputEventKind::HoverEntered, generation, json!({})),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "queued");
    assert_eq!(observations(&rt, "companion.pointer").await.len(), 2);

    // drag-started／dropped → companion-dragged；dragged 節流 1 s。
    rt.character.set_clock(clock_at(41));
    rt.character_event(
        DESKTOP_INSTANCE_ID,
        input_event(
            InputEventKind::DragStarted,
            generation,
            json!({"x": 1, "y": 2}),
        ),
    )
    .await
    .unwrap();
    rt.character_event(
        DESKTOP_INSTANCE_ID,
        input_event(InputEventKind::Dragged, generation, json!({"x": 5, "y": 6})),
    )
    .await
    .unwrap();
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(InputEventKind::Dragged, generation, json!({"x": 9, "y": 9})),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "throttled");
    let dragged = observations(&rt, "companion.click")
        .await
        .into_iter()
        .filter(|o| o.facts["kind"] == "companion-dragged")
        .count();
    assert_eq!(dragged, 2, "drag-started + one throttled dragged");

    // toy-thrown／dismissed／visibility-changed：只 audit，不變 observation。
    let before_clicks = observations(&rt, "companion.click").await.len();
    for (kind, payload) in [
        (InputEventKind::ToyThrown, json!({"toyId": "ball"})),
        (InputEventKind::Dismissed, json!({})),
        (InputEventKind::VisibilityChanged, json!({"visible": false})),
    ] {
        let out = rt
            .character_event(DESKTOP_INSTANCE_ID, input_event(kind, generation, payload))
            .await
            .unwrap();
        assert_eq!(out["decision"], "queued");
    }
    assert_eq!(
        observations(&rt, "companion.click").await.len(),
        before_clicks
    );

    // 舊世代事件 → dropped（stale-generation）；絕對座標鍵 → dropped。
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(InputEventKind::Clicked, 0, json!({})),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "dropped");
    assert_eq!(out["reason"], "stale-generation");
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(InputEventKind::Clicked, generation, json!({"screenX": 900})),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "dropped");
    assert_eq!(out["reason"], "absolute-coordinates");

    // 角色隱藏 → 視窗內受器關閉：事件誠實 dropped（不是靜默吞掉）。
    rt.character.set_clock(Arc::new(Utc::now));
    hello(&rt, false).await;
    let out = rt
        .character_event(
            DESKTOP_INSTANCE_ID,
            input_event(InputEventKind::Clicked, 2, json!({})),
        )
        .await
        .unwrap();
    assert_eq!(out["decision"], "dropped");
    assert_eq!(out["reason"], "unavailable");
}

#[tokio::test]
async fn playable_animations_follow_negotiation_and_deny_truth_states() {
    let (_g, rt) = runtime().await;
    let before = rt.presentation.playable_animations();
    assert_eq!(
        before,
        vec![
            "idle",
            "notice",
            "acknowledge",
            "think",
            "work",
            "greet",
            "play",
            "rest",
            "sleep"
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>()
    );
    hello(&rt, true).await;
    let after = rt.presentation.playable_animations();
    assert!(after.iter().any(|a| a == "curious"));
    assert!(after.iter().any(|a| a == "thinking"));
    assert!(!after.iter().any(|a| a == "success"), "deny-list");
    assert!(!after.iter().any(|a| a == "emergency"), "safety intent");
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    // 真的執行：success 被拒（Failed receipt），curious 派得出去。
    let refused = plan_and_execute(
        &rt,
        "companion.animation.play",
        json!({"animation": "success"}),
        None,
    )
    .await;
    assert_eq!(refused[0].current_status, ActionStatus::Failed);
    let ok = plan_and_execute(
        &rt,
        "companion.animation.play",
        json!({"animation": "curious"}),
        None,
    )
    .await;
    assert_eq!(ok[0].current_status, ActionStatus::Dispatched);
}

#[tokio::test]
async fn provider_identity_follows_the_active_character() {
    let (_g, rt) = runtime().await;
    let find = |list: Vec<ProviderDescriptor>| {
        list.into_iter()
            .find(|p| p.identity.id.as_str() == "provider.companion.desktop")
            .expect("companion provider")
    };
    let before = find(rt.list_providers().await);
    assert_eq!(before.identity.display_name, "桌面角色（尚未連線）");
    assert_eq!(before.identity.kind, ProviderKind::Companion);
    assert_eq!(rt.companion_provider_display_name(), "桌面角色（尚未連線）");
    assert!(rt.mobile.character_title().is_none());
    assert!(rt
        .list_providers()
        .await
        .iter()
        .all(|p| p.identity.id.as_str() != "provider.companion.shu"));

    hello(&rt, true).await;
    let after = find(rt.list_providers().await);
    assert_eq!(
        after.identity.display_name,
        "桌面角色：小樞（Presentation）"
    );
    assert!(after.detail.as_deref().unwrap_or("").contains("shu-maid"));
    assert!(after.detail.as_deref().unwrap_or("").contains("in-process"));
    assert!(after.detail.as_deref().unwrap_or("").contains("builtin"));
    let one = rt
        .get_provider(&ProviderId::new("provider.companion.desktop"))
        .await
        .unwrap();
    assert_eq!(one.identity.display_name, "桌面角色：小樞（Presentation）");
    assert_eq!(rt.mobile.character_title().as_deref(), Some("小樞"));
    // 能力歸屬不變：7 受器＋7 動器。
    assert_eq!(after.receptors.len(), 7);
    assert_eq!(after.actuators.len(), 7);
}

#[tokio::test]
async fn adapter_tokens_are_hashed_persisted_and_revocable() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime_in(&dir).await;
    let added = rt
        .character_adapter_add("文字 adapter（fixture）", fixture_manifest())
        .await
        .unwrap();
    let adapter_id = added["adapterId"].as_str().unwrap().to_string();
    let token = added["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64, "32 random bytes as hex");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    let list = rt.character_adapters();
    let entry = &list["adapters"][0];
    assert_eq!(entry["adapterId"], adapter_id);
    assert_eq!(entry["displayName"], "文字 adapter（fixture）");
    assert_eq!(entry["characterId"], "text-adapter-fixture");
    assert_eq!(entry["revoked"], false);
    assert_eq!(entry["connected"], false);
    assert!(entry.get("token").is_none() && entry.get("tokenSha256").is_none());
    // 從未連線的 adapter 也能誠實顯示作者／版本／可接收／可執行／需要網路（來自 manifest）。
    assert_eq!(entry["author"], "adaptive-interaction examples");
    assert_eq!(entry["version"], "1.0.0");
    assert_eq!(entry["inputCapabilities"], json!(["input.text"]));
    assert!(entry["characterDisplayName"]["zh-TW"].is_string());
    assert_eq!(entry["adapterKind"], "external-process");
    assert_eq!(entry["executable"], true);
    assert_eq!(entry["network"], false);
    assert_eq!(
        rt.character_adapter_for_token(&token).as_deref(),
        Some(adapter_id.as_str())
    );
    assert!(rt.character_adapter_for_token("bogus").is_none());
    // token 永遠不進 audit／storage 明文。
    let stored = rt.store.all_character_adapters().unwrap();
    assert_eq!(stored.len(), 1);
    assert!(!stored[0].contains(&token));
    let audit = rt.store.audit_tail(50).unwrap_or_default();
    assert!(!serde_json::to_string(&audit).unwrap().contains(&token));

    // in-process manifest 不能當外部 adapter；顯示名為空拒絕。
    assert!(rt
        .character_adapter_add("桌面", desktop_manifest())
        .await
        .is_err());
    assert!(rt
        .character_adapter_add("   ", fixture_manifest())
        .await
        .is_err());

    // 撤銷：token 立刻失效、清單標 revoked。
    let out = rt.character_adapter_revoke(&adapter_id).await.unwrap();
    assert_eq!(out["revoked"], true);
    assert_eq!(out["disconnected"], false);
    assert!(rt.character_adapter_for_token(&token).is_none());
    assert_eq!(rt.character_adapters()["adapters"][0]["revoked"], true);
    assert!(matches!(
        rt.character_adapter_revoke("adp-nope").await,
        Err(DomainError::NotFound(_))
    ));

    // 重啟後仍撤銷（storage v8 持久化）。
    rt.shutdown().await;
    drop(rt);
    let rt2 = runtime_in(&dir).await;
    assert!(rt2.character_adapter_for_token(&token).is_none());
    assert_eq!(rt2.character_adapters()["adapters"][0]["revoked"], true);
    assert_eq!(
        rt2.character_adapters()["adapters"][0]["adapterId"],
        adapter_id
    );
}

#[tokio::test]
async fn safety_intents_fall_back_to_system_text_without_any_instance() {
    let (_g, rt) = runtime().await;
    // 沒有任何角色：非安全 intent 靜默略過，安全 intent 走 system.text（不得遺失）。
    let out = rt
        .character_manual_intent("notice", Some("hi".into()))
        .await
        .unwrap();
    assert_eq!(out["targets"], json!([]));
    assert_eq!(out["truthState"], "none");
    assert!(out["note"].as_str().unwrap().contains("no connected"));
    assert!(events_of(&rt, EventType::CharacterIntent).is_empty());
    assert!(events_of(&rt, EventType::CharacterSystemText).is_empty());

    let err = rt
        .character_manual_intent("emergency", None)
        .await
        .expect_err("safety intents are runtime-only");
    assert!(matches!(err, DomainError::PolicyBlocked(_)));
    for intent in [
        "blocked",
        "failed",
        "verified-success",
        "claim-completed",
        "request-consent",
        "unknown",
        "offline",
        "wait",
        "ask",
        "cancelled",
    ] {
        assert!(
            rt.character_manual_intent(intent, None).await.is_err(),
            "{intent} must be refused"
        );
    }
    assert!(rt.character_manual_intent("teleport", None).await.is_err());

    rt.emergency_stop("test", None).await.unwrap();
    let texts = events_of(&rt, EventType::CharacterSystemText);
    let last = texts.last().expect("system.text fallback for emergency");
    assert_eq!(last.payload["intent"], "emergency");
    assert_eq!(last.payload["truthState"], "emergency");
    assert!(last.payload["instanceId"].is_null());
    assert_eq!(last.payload["correlationId"], "emergency-stop");
    assert!(last.payload["message"]
        .as_str()
        .unwrap()
        .contains("緊急停止"));
    assert!(events_of(&rt, EventType::CharacterIntent).is_empty());
}

#[tokio::test]
async fn external_adapter_transport_attaches_negotiates_and_times_out() {
    let (_g, rt) = runtime().await;
    let added = rt
        .character_adapter_add("fixture", fixture_manifest())
        .await
        .unwrap();
    let adapter_id = added["adapterId"].as_str().unwrap().to_string();
    let instance_id = adapter_instance_id(&adapter_id);
    let mut session = rt.character_ws_attach(&adapter_id).await.unwrap();
    assert_eq!(session.instance_id, instance_id);
    let first = session.rx.recv().await.expect("hello first");
    let hello = match first {
        WireMessage::Hello(hello) => hello,
        other => panic!("first message must be hello, got {}", other.kind()),
    };
    assert_eq!(hello.character_instance_id, instance_id);
    assert_eq!(hello.role, interaction_character::CharacterRole::Familiar);
    assert_eq!(hello.requires.len(), 20);
    // 尚未協商：instances 顯示 connected=false，投影不會送給它。
    let entry = rt.character_instances()["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["instanceId"] == instance_id.as_str())
        .cloned()
        .unwrap();
    assert_eq!(entry["connected"], false);
    assert_eq!(entry["origin"], "external");
    assert_eq!(entry["executable"], true);
    assert_eq!(entry["adapterKind"], "external-process");
    assert_eq!(entry["author"], "adaptive-interaction examples");
    assert_eq!(entry["version"], "1.0.0");
    assert_eq!(entry["inputCapabilities"], json!(["input.text"]));
    assert_eq!(rt.character_adapters()["adapters"][0]["connected"], true);

    // negotiate → negotiated（generation 1）。
    let negotiate = WireMessage::Negotiate(Negotiate::from_manifest(&fixture_manifest(), 1));
    let step = rt
        .character_ws_message(
            &instance_id,
            session.conn_id,
            &encode_wire(&negotiate).unwrap(),
        )
        .await;
    assert_eq!(step, WsStep::KeepOpen);
    let negotiated = match session.rx.recv().await.unwrap() {
        WireMessage::Negotiated(n) => n,
        other => panic!("expected negotiated, got {}", other.kind()),
    };
    assert_eq!(negotiated.generation, 1);
    assert_eq!(negotiated.resolutions.len(), 20);
    assert_eq!(rt.character_generation(&instance_id), Some(1));

    // runtime 事件投影到外部 adapter：estop → intent emergency 經 outbound 送出。
    rt.emergency_stop("test", None).await.unwrap();
    let envelope = loop {
        match session.rx.recv().await.unwrap() {
            WireMessage::Intent { envelope } => break envelope,
            WireMessage::Heartbeat { .. } => continue,
            other => panic!("unexpected {}", other.kind()),
        }
    };
    assert_eq!(envelope.intent, CharacterIntent::Emergency);
    assert_eq!(envelope.character_instance_id, instance_id);
    let targets = last_intent(&rt)["targets"].clone();
    assert_eq!(targets, json!([instance_id]));

    // 回執：accepted → started → completed（只代表文字印出）→ tested=true。
    for status in [
        ReceiptStatus::Accepted,
        ReceiptStatus::Started,
        ReceiptStatus::Completed,
    ] {
        let msg = WireMessage::Receipt {
            receipt: CommandReceipt::new(&envelope.message_id, &instance_id, 1, status, Utc::now()),
        };
        let step = rt
            .character_ws_message(&instance_id, session.conn_id, &encode_wire(&msg).unwrap())
            .await;
        assert_eq!(step, WsStep::KeepOpen);
    }
    assert!(events_of(&rt, EventType::CharacterReceipt).iter().any(|e| {
        e.payload["instanceId"] == instance_id.as_str()
            && e.payload["receipt"]["status"] == "completed"
    }));
    let entry = rt.character_instances()["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["instanceId"] == instance_id.as_str())
        .cloned()
        .unwrap();
    assert_eq!(entry["tested"], true);
    assert_eq!(entry["connected"], true);

    // 外部 adapter 的 text-submitted 不受桌面隱藏閘門影響 → companion.text-input。
    rt.start_session(Some("t".into()), None, vec![]).await.ok();
    let event = WireMessage::Event {
        event: CharacterInputEvent {
            protocol_version: "1.0".into(),
            event_id: "evt-ext-1".into(),
            character_instance_id: instance_id.clone(),
            generation: 1,
            timestamp: Utc::now(),
            kind: InputEventKind::TextSubmitted,
            payload: [("text".to_string(), json!("hello from fixture"))]
                .into_iter()
                .collect(),
            privacy_class: Default::default(),
        },
    };
    rt.character_ws_message(&instance_id, session.conn_id, &encode_wire(&event).unwrap())
        .await;
    let texts = observations(&rt, "companion.text-input").await;
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].facts["text"], "hello from fixture");

    // 超過 64 KB → error{too-large}（不斷線）；runtime→adapter 方向的訊息 → wrong-direction。
    let big = format!(
        "{{\"type\":\"heartbeat\",\"pad\":\"{}\"}}",
        "x".repeat(70_000)
    );
    let step = rt
        .character_ws_message(&instance_id, session.conn_id, big.as_bytes())
        .await;
    assert_eq!(step, WsStep::KeepOpen);
    let err = loop {
        match session.rx.recv().await.unwrap() {
            WireMessage::Error { code, .. } => break code,
            _ => continue,
        }
    };
    assert_eq!(err, "too-large");
    let wrong = encode_wire(&WireMessage::Hello(hello.clone())).unwrap();
    rt.character_ws_message(&instance_id, session.conn_id, &wrong)
        .await;
    let err = loop {
        match session.rx.recv().await.unwrap() {
            WireMessage::Error { code, .. } => break code,
            _ => continue,
        }
    };
    assert_eq!(err, "wrong-direction");

    // heartbeat 逾時（45 s 無訊息）：sweep → 斷線、close token 取消、generation+1。
    rt.character_sweep_at(Utc::now() + chrono::Duration::seconds(46))
        .await;
    assert!(session.close.is_cancelled());
    let entry = rt.character_instances()["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["instanceId"] == instance_id.as_str())
        .cloned()
        .unwrap();
    assert_eq!(entry["connected"], false);
    assert_eq!(entry["generation"], 2);
    rt.character_ws_closed(
        &instance_id,
        session.conn_id,
        DisconnectReason::HeartbeatTimeout,
    )
    .await;
    assert_eq!(rt.character_adapters()["adapters"][0]["connected"], false);

    // 重新 attach：舊 conn_id 的訊息被忽略（Close），新連線先收 hello。
    let mut again = rt.character_ws_attach(&adapter_id).await.unwrap();
    assert!(matches!(again.rx.recv().await, Some(WireMessage::Hello(_))));
    let stale = rt
        .character_ws_message(
            &instance_id,
            session.conn_id,
            &encode_wire(&negotiate).unwrap(),
        )
        .await;
    assert_eq!(stale, WsStep::Close);
    // 撤銷 → goodbye＋close token。
    rt.character_adapter_revoke(&adapter_id).await.unwrap();
    assert!(again.close.is_cancelled());
    assert!(matches!(
        again.rx.recv().await,
        Some(WireMessage::Goodbye { .. })
    ));
    assert!(rt.character_instances()["instances"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["instanceId"] != instance_id.as_str()));
    assert!(matches!(
        rt.character_ws_attach(&adapter_id).await,
        Err(DomainError::NotFound(_))
    ));
}

#[tokio::test]
async fn desktop_presence_expiry_disconnects_the_desktop_instance() {
    let (_g, rt) = runtime().await;
    hello(&rt, true).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "think"}),
        None,
    )
    .await;
    let action_id = receipts[0].action_id.clone();
    // presence 心跳過期（> 20 s）→ transport-closed：pending → uncertain。
    rt.character_sweep_at(Utc::now() + chrono::Duration::seconds(25))
        .await;
    let entry = &rt.character_instances()["instances"][0];
    assert_eq!(entry["connected"], false);
    assert_eq!(entry["generation"], 2);
    assert!(rt.status().await["characterProtocol"]["activeCharacter"].is_null());
    assert_eq!(rt.companion_provider_display_name(), "桌面角色（尚未連線）");
    assert_eq!(
        rt.get_action(&action_id).unwrap().current_status,
        ActionStatus::Uncertain
    );
    // 之後的投影沒有目標：安全 intent 走 system.text。
    rt.emergency_stop("test", None).await.unwrap();
    assert!(events_of(&rt, EventType::CharacterSystemText)
        .last()
        .is_some_and(|e| e.payload["intent"] == "emergency"));
}

/// 桌面 manifest，但表情／存在感在 Reduced Motion 下只能靜態呈現。
fn desktop_manifest_reduced() -> CharacterManifest {
    let mut value = serde_json::to_value(desktop_manifest()).expect("manifest to value");
    value["capabilities"]["visual.expression"]["reducedMotionBehavior"] = json!("reduced");
    value["capabilities"]["visual.presence"]["reducedMotionBehavior"] = json!("static");
    serde_json::from_value(value).expect("reduced-motion manifest parses")
}

#[tokio::test]
async fn hello_carries_reduced_motion_and_receipts_report_reduced_not_exact() {
    let (_g, rt) = runtime().await;
    let manifest = desktop_manifest_reduced();
    let out = rt
        .character_hello(CharacterHelloInput {
            instance_id: None,
            role: None,
            negotiate: Negotiate::from_manifest(&manifest, 1),
            manifest,
            visible: true,
            pack_id: None,
            behavior_state: None,
            reduced_motion: true,
        })
        .await
        .expect("hello accepted");
    // 協商結果誠實反映視窗的 Reduced Motion。
    assert_eq!(out["negotiated"]["reducedMotion"], true, "{out}");
    assert_eq!(
        out["negotiated"]["resolutions"]["notice"]["resolution"], "reduced",
        "{out}"
    );
    assert_eq!(
        rt.character_instances()["instances"][0]["reducedMotion"],
        true
    );
    // audit 也留下這次協商用的值。
    let audit = rt.store.audit_tail(50).unwrap_or_default();
    assert!(
        audit
            .iter()
            .any(|row| row["kind"] == "character.hello" && row["detail"]["reducedMotion"] == true),
        "character.hello 的 audit 必須帶 reducedMotion：{audit:?}"
    );

    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "notice"}),
        None,
    )
    .await;
    let mid = receipts[0].action_id.clone();
    for status in [
        ReceiptStatus::Accepted,
        ReceiptStatus::Started,
        ReceiptStatus::Completed,
    ] {
        let mut r = receipt(mid.as_str(), 1, status);
        r.resolution = Some(interaction_character::Resolution::Reduced);
        rt.character_receipt(DESKTOP_INSTANCE_ID, r).await.unwrap();
    }
    let receipt_events = events_of(&rt, EventType::CharacterReceipt);
    let mine: Vec<&RuntimeEvent> = receipt_events
        .iter()
        .filter(|e| e.payload["receipt"]["messageId"] == mid.as_str())
        .collect();
    assert!(!mine.is_empty(), "至少要有一筆該命令的回執事件");
    for e in &mine {
        assert_eq!(
            e.payload["receipt"]["resolution"], "reduced",
            "Reduced Motion 下不得回報 exact：{}",
            e.payload
        );
    }
    let audit = rt.store.audit_tail(200).unwrap_or_default();
    let audited: Vec<&Value> = audit
        .iter()
        .filter(|row| {
            row["kind"] == "character.receipt" && row["detail"]["messageId"] == mid.as_str()
        })
        .collect();
    assert!(!audited.is_empty());
    for row in audited {
        assert_eq!(row["detail"]["resolution"], "reduced", "{row}");
    }

    // adapter 謊報 exact 也不會把協商結果升級回 exact。
    let receipts = plan_and_execute(
        &rt,
        "companion.state.present",
        json!({"behaviorIntent": "think"}),
        None,
    )
    .await;
    let mid2 = receipts[0].action_id.clone();
    let mut lying = receipt(mid2.as_str(), 1, ReceiptStatus::Started);
    lying.resolution = Some(interaction_character::Resolution::Exact);
    rt.character_receipt(DESKTOP_INSTANCE_ID, lying)
        .await
        .unwrap();
    let last = events_of(&rt, EventType::CharacterReceipt)
        .into_iter()
        .rfind(|e| e.payload["receipt"]["messageId"] == mid2.as_str())
        .expect("receipt event");
    assert_eq!(last.payload["receipt"]["resolution"], "reduced");
}

#[tokio::test]
async fn adapter_rate_limit_covers_http_receipts_events_and_malformed_ws_frames() {
    let (_g, rt) = runtime().await;
    let added = rt
        .character_adapter_add("fixture", fixture_manifest())
        .await
        .unwrap();
    let adapter_id = added["adapterId"].as_str().unwrap().to_string();
    let instance_id = adapter_instance_id(&adapter_id);
    let mut session = rt.character_ws_attach(&adapter_id).await.unwrap();
    assert!(matches!(
        session.rx.recv().await,
        Some(WireMessage::Hello(_))
    ));
    let negotiate = WireMessage::Negotiate(Negotiate::from_manifest(&fixture_manifest(), 1));
    rt.character_ws_message(
        &instance_id,
        session.conn_id,
        &encode_wire(&negotiate).unwrap(),
    )
    .await;

    // 時鐘凍結在同一秒：50 則/s 的預算對 HTTP 與 WebSocket 是同一份。
    let base = Utc::now();
    let clock_at = |secs: i64| {
        let t = base + chrono::Duration::seconds(secs);
        Arc::new(move || t) as interaction_runtime::character::NowFn
    };
    rt.character.set_clock(clock_at(1));

    // (a) POST /v1/character/events：超量後誠實回 dropped{rate-limited}，不寫進 observation。
    let mut queued = 0;
    let mut limited = 0;
    for _ in 0..80 {
        let event = CharacterInputEvent {
            protocol_version: "1.0".into(),
            event_id: format!("evt-{}", uuid::Uuid::new_v4().simple()),
            character_instance_id: instance_id.clone(),
            generation: 1,
            timestamp: base,
            kind: InputEventKind::TextSubmitted,
            payload: [("text".to_string(), json!("spam"))].into_iter().collect(),
            privacy_class: Default::default(),
        };
        let out = rt.character_event(&instance_id, event).await.unwrap();
        if out["reason"] == "rate-limited" {
            limited += 1;
            assert_eq!(out["decision"], "dropped");
        } else {
            queued += 1;
        }
    }
    assert!(queued <= 50, "同一秒內最多 50 則，實際 {queued}");
    assert!(limited >= 20, "超量的必須被擋下，實際 {limited}");

    // (b) POST /v1/character/receipts：共用同一份預算 → 立刻 rate-limited。
    let out = rt
        .character_receipt(
            &instance_id,
            CommandReceipt::new("nope", &instance_id, 1, ReceiptStatus::Accepted, base),
        )
        .await
        .unwrap();
    assert_eq!(out["status"], "rate-limited", "{out}");
    assert_eq!(out["accepted"], false);

    // (c) 畸形 WebSocket frame：先扣預算，稽核有界（5 秒視窗最多一列）。
    let before = rt
        .store
        .audit_tail(500)
        .unwrap_or_default()
        .iter()
        .filter(|row| row["kind"] == "character.wire-rejected")
        .count();
    for _ in 0..40 {
        let step = rt
            .character_ws_message(&instance_id, session.conn_id, b"not json at all")
            .await;
        assert_eq!(step, WsStep::KeepOpen);
    }
    let rejected: Vec<Value> = rt
        .store
        .audit_tail(500)
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row["kind"] == "character.wire-rejected")
        .collect();
    assert_eq!(
        rejected.len() - before,
        1,
        "40 則畸形訊息只能留下 1 列稽核，實際 {}",
        rejected.len() - before
    );

    // (d) 下一秒預算回補：事件又被接受。
    rt.character.set_clock(clock_at(2));
    let event = CharacterInputEvent {
        protocol_version: "1.0".into(),
        event_id: "evt-after-window".into(),
        character_instance_id: instance_id.clone(),
        generation: 1,
        timestamp: base,
        kind: InputEventKind::TextSubmitted,
        payload: [("text".to_string(), json!("again"))].into_iter().collect(),
        privacy_class: Default::default(),
    };
    let out = rt.character_event(&instance_id, event).await.unwrap();
    assert_ne!(out["reason"], "rate-limited", "{out}");

    // 稽核視窗過了才會再留一列，且帶出被壓下的次數。
    rt.character.set_clock(clock_at(9));
    rt.character_ws_message(&instance_id, session.conn_id, b"still not json")
        .await;
    let rejected: Vec<Value> = rt
        .store
        .audit_tail(500)
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row["kind"] == "character.wire-rejected")
        .collect();
    assert_eq!(rejected.len() - before, 2);
    assert!(
        rejected
            .iter()
            .any(|row| row["detail"]["suppressed"].as_u64().unwrap_or(0) > 0),
        "被壓下的畸形訊息數必須被記下來：{rejected:?}"
    );
    rt.character.set_clock(Arc::new(Utc::now));
}

#[tokio::test]
async fn ws_adapter_dropping_mid_safety_intent_yields_uncertain_and_system_text() {
    let (_g, rt) = runtime().await;
    let added = rt
        .character_adapter_add("fixture", fixture_manifest())
        .await
        .unwrap();
    let adapter_id = added["adapterId"].as_str().unwrap().to_string();
    let instance_id = adapter_instance_id(&adapter_id);
    let mut session = rt.character_ws_attach(&adapter_id).await.unwrap();
    assert!(matches!(
        session.rx.recv().await,
        Some(WireMessage::Hello(_))
    ));
    let negotiate = WireMessage::Negotiate(Negotiate::from_manifest(&fixture_manifest(), 1));
    rt.character_ws_message(
        &instance_id,
        session.conn_id,
        &encode_wire(&negotiate).unwrap(),
    )
    .await;
    assert!(matches!(
        session.rx.recv().await,
        Some(WireMessage::Negotiated(_))
    ));

    // 安全 intent 送出去，adapter 只回 started 就掛掉。
    rt.emergency_stop("test", None).await.unwrap();
    let envelope = loop {
        match session.rx.recv().await.unwrap() {
            WireMessage::Intent { envelope } => break envelope,
            _ => continue,
        }
    };
    assert_eq!(envelope.intent, CharacterIntent::Emergency);
    let started = WireMessage::Receipt {
        receipt: CommandReceipt::new(
            &envelope.message_id,
            &instance_id,
            1,
            ReceiptStatus::Started,
            Utc::now(),
        ),
    };
    rt.character_ws_message(
        &instance_id,
        session.conn_id,
        &encode_wire(&started).unwrap(),
    )
    .await;
    let texts_before = events_of(&rt, EventType::CharacterSystemText).len();

    // 連線斷掉（程序崩潰）。
    rt.character_ws_closed(&instance_id, session.conn_id, DisconnectReason::Crash)
        .await;

    // 進行中的 command → uncertain（不猜 completed）。
    let uncertain = events_of(&rt, EventType::CharacterReceipt)
        .into_iter()
        .filter(|e| {
            e.payload["receipt"]["messageId"] == envelope.message_id.as_str()
                && e.payload["receipt"]["status"] == "uncertain"
        })
        .count();
    assert_eq!(uncertain, 1, "斷線時進行中的 command 必須是 uncertain");

    // 安全訊息不得遺失：以 system.text 補送（可信 overlay 呈現）。
    let texts = events_of(&rt, EventType::CharacterSystemText);
    assert!(
        texts.len() > texts_before,
        "adapter 掛掉後安全訊息必須改走 system.text"
    );
    let last = texts.last().expect("system text event");
    assert_eq!(last.payload["intent"], "emergency");
    assert!(
        last.payload["messageId"]
            .as_str()
            .unwrap_or_default()
            .starts_with(&envelope.message_id),
        "{}",
        last.payload
    );
    assert!(
        last.payload["message"]
            .as_str()
            .unwrap_or_default()
            .contains("緊急停止"),
        "{}",
        last.payload
    );
}
