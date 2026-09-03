//! Provider + declarative-device integration: a spec file in the home config
//! becomes real capabilities behind the SAME governor as everything else.
//! Pairing / install / enable / consent stay separate; revocation disables
//! capabilities immediately; the device only ever sees bounded values.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Device {
    set_calls: Arc<Mutex<Vec<Value>>>,
}

async fn spawn_device() -> (SocketAddr, Device) {
    let device = Device::default();
    let app = Router::new()
        .route(
            "/status",
            get(|| async { Json(json!({"power": false, "brightness": 0})) }),
        )
        .route(
            "/set",
            post(|State(d): State<Device>, Json(b): Json<Value>| async move {
                d.set_calls.lock().unwrap().push(b);
                Json(json!({"queued": true}))
            }),
        )
        .with_state(device.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, device)
}

async fn runtime_with_device_spec() -> (tempfile::TempDir, Runtime, Device) {
    let dir = tempfile::tempdir().unwrap();
    let (addr, device) = spawn_device().await;
    let adapters = dir.path().join("config").join("adapters");
    std::fs::create_dir_all(&adapters).unwrap();
    std::fs::write(
        adapters.join("desk-light.yaml"),
        format!(
            r#"
schemaVersion: "1.0"
id: desk-light
displayName: 書桌燈
capabilities:
  - kind: receptor
    id: status
    transport: http
    request: {{ method: GET, url: "http://{addr}/status" }}
    facts:
      power: "/power"
  - kind: actuator
    id: set
    channel: light
    transport: http
    confirmation: acknowledged
    request:
      method: POST
      url: "http://{addr}/set"
      body: {{ brightness: "{{{{magnitude}}}}" }}
"#
        ),
    )
    .unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    (dir, rt, device)
}

#[tokio::test]
async fn declarative_device_registers_installed_and_disabled() {
    let (_g, rt, _device) = runtime_with_device_spec().await;

    // Provider record exists in Installed state (human-owned config).
    let providers = rt.list_providers().await;
    let desk = providers
        .iter()
        .find(|p| p.identity.id.as_str() == "provider.adapter.desk-light")
        .expect("desk provider");
    assert_eq!(desk.state, ProviderState::Installed);
    assert!(desk.actuators.contains(&"desk-light.set".to_string()));

    // The actuator is consent-gated → NOT enabled by default.
    let snap = rt
        .capabilities(&DiscoveryContext {
            include_unavailable: true,
            ..Default::default()
        })
        .await;
    let act = snap
        .actuators
        .iter()
        .find(|a| a.id.as_str() == "desk-light.set")
        .expect("actuator listed");
    assert!(act.requires_consent);
    assert_ne!(
        act.availability,
        Availability::Available,
        "device output must start disabled"
    );
}

#[tokio::test]
async fn governor_bounds_device_output_and_receipt_stays_acknowledged() {
    let (_g, rt, device) = runtime_with_device_spec().await;
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("desk-light.set"), true)
        .await
        .unwrap();
    // The human explicitly allowlists the new device (policy stays deny-by-
    // default for unknown actuators) and grants session consent.
    rt.update_policy(json!({"actuatorAllowlist": ["desk-light.set"], "allowedChannels": ["conversation", "web-ui", "log", "light"]}))
        .await
        .unwrap();
    rt.start_session(
        Some("test".into()),
        None,
        vec!["actuator:desk-light.set".into()],
    )
    .await
    .unwrap();

    let mut intent = SemanticIntent::new("calm");
    intent.magnitude = Some(1.0); // ask for maximum
    intent.preferred_channels = vec!["light".into()];
    let plan = rt
        .create_plan(
            intent,
            vec!["desk-light.set".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert_eq!(receipts.len(), 1);
    let receipt = &receipts[0];

    // Honest terminal state: acknowledged (device accepted), NOT completed.
    if receipt.current_status == ActionStatus::Blocked {
        panic!("blocked: {:?}", receipt.policy_decisions);
    }
    assert_eq!(receipt.current_status, ActionStatus::Acknowledged);

    // The device saw a policy-bounded magnitude, not the requested 1.0.
    let calls = device.set_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let sent = calls[0]["brightness"].as_f64().unwrap();
    let effective = receipt.effective_bounded_parameters.magnitude.unwrap();
    assert!(
        (sent - effective).abs() < 1e-9,
        "device got exactly the bounded value"
    );
    assert!(sent <= 1.0);
    if let Some(requested) = receipt.requested_parameters.magnitude {
        assert!(effective <= requested);
    }
}

#[tokio::test]
async fn pairing_ceremony_and_revocation_disable_capabilities() {
    let (_g, rt, _device) = runtime_with_device_spec().await;

    // A network-discovered provider goes through the full ceremony.
    let id = ProviderId::new("provider.device.net-01");
    rt.providers
        .register(interaction_registry::providers::discovered(
            ProviderIdentity {
                id: id.clone(),
                kind: ProviderKind::Device,
                display_name: "網路裝置".into(),
                trust_level: TrustLevel::Discovered,
                origin: "local-network".into(),
                version: "1".into(),
                fingerprint: None,
                human: None,
            },
        ))
        .await
        .unwrap();

    // Too-short code refused; pairing derives a fingerprint (never an IP).
    assert!(rt.pair_provider(&id, "12").await.is_err());
    let paired = rt.pair_provider(&id, "492817").await.unwrap();
    assert_eq!(paired.state, ProviderState::Paired);
    assert_eq!(paired.identity.trust_level, TrustLevel::Paired);
    assert!(paired.identity.fingerprint.as_deref().unwrap().len() == 64);

    // install → disabled → available, then revoke.
    rt.transition_provider(&id, ProviderState::Installed)
        .await
        .unwrap();
    rt.transition_provider(&id, ProviderState::Disabled)
        .await
        .unwrap();
    rt.transition_provider(&id, ProviderState::Available)
        .await
        .unwrap();

    // Revoking the DECLARATIVE provider disables its live capabilities.
    let desk = ProviderId::new("provider.adapter.desk-light");
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("desk-light.set"), true)
        .await
        .unwrap();
    let revoked = rt.revoke_provider(&desk).await.unwrap();
    assert_eq!(revoked.state, ProviderState::Revoked);
    let snap = rt
        .capabilities(&DiscoveryContext {
            include_unavailable: true,
            ..Default::default()
        })
        .await;
    let act = snap
        .actuators
        .iter()
        .find(|a| a.id.as_str() == "desk-light.set")
        .unwrap();
    assert_ne!(act.availability, Availability::Available);
    // Revoked is sticky: no way back to available.
    assert!(rt
        .transition_provider(&desk, ProviderState::Available)
        .await
        .is_err());
}

#[tokio::test]
async fn provider_records_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let id = ProviderId::new("provider.device.persisted");
    {
        let rt = Runtime::start(RuntimeOptions {
            home: Some(dir.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        rt.providers
            .register(interaction_registry::providers::discovered(
                ProviderIdentity {
                    id: id.clone(),
                    kind: ProviderKind::Device,
                    display_name: "persist me".into(),
                    trust_level: TrustLevel::Discovered,
                    origin: "local-network".into(),
                    version: "1".into(),
                    fingerprint: None,
                    human: None,
                },
            ))
            .await
            .unwrap();
        rt.pair_provider(&id, "886644").await.unwrap();
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
    let restored = rt.get_provider(&id).await.unwrap();
    assert_eq!(restored.state, ProviderState::Paired);
    assert!(restored.identity.fingerprint.is_some());
}

#[tokio::test]
async fn operational_provider_does_not_auto_recover_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let id = ProviderId::new("provider.device.armed");
    {
        let rt = Runtime::start(RuntimeOptions {
            home: Some(dir.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        rt.providers
            .register(interaction_registry::providers::discovered(
                ProviderIdentity {
                    id: id.clone(),
                    kind: ProviderKind::Device,
                    display_name: "armed device".into(),
                    trust_level: TrustLevel::Discovered,
                    origin: "local-network".into(),
                    version: "1".into(),
                    fingerprint: None,
                    human: None,
                },
            ))
            .await
            .unwrap();
        rt.pair_provider(&id, "778899").await.unwrap();
        rt.transition_provider(&id, ProviderState::Installed)
            .await
            .unwrap();
        rt.transition_provider(&id, ProviderState::Disabled)
            .await
            .unwrap();
        rt.transition_provider(&id, ProviderState::Available)
            .await
            .unwrap();
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
    let restored = rt.get_provider(&id).await.unwrap();
    // Crash/restart must NOT bring a device back Available on its own.
    assert_eq!(restored.state, ProviderState::Disabled);
    assert!(!restored.state.is_operational());
}

// ---------------------------------------------------------------------------
// spec §9.3：只發現 ≠ 已配對 ≠ 已測試 ≠ 已啟用。
// 掃描到 metadata／設定檔存在／狀態 Installed 都不算測過；「已測試」只由
// runtime 真的觀察到的成功／失敗寫入。
// ---------------------------------------------------------------------------

fn tested_of(desc: &ProviderDescriptor) -> Option<Value> {
    let detail = desc.detail.as_deref()?;
    let parsed: Value = serde_json::from_str(detail).ok()?;
    parsed.get("tested").cloned()
}

async fn provider_named(rt: &Runtime, id: &str) -> ProviderDescriptor {
    rt.get_provider(&ProviderId::new(id)).await.unwrap()
}

#[tokio::test]
async fn declared_device_is_not_tested_until_something_actually_succeeds() {
    let (_g, rt, _device) = runtime_with_device_spec().await;
    let desk = provider_named(&rt, "provider.adapter.desk-light").await;
    // 設定檔存在 ⇒ Installed，但沒有任何「已測試」證據。
    assert_eq!(desk.state, ProviderState::Installed);
    assert!(
        tested_of(&desk).is_none(),
        "只讀到 YAML 不得宣稱已測試：{:?}",
        desk.detail
    );
}

#[tokio::test]
async fn human_provider_test_reads_one_receptor_and_records_evidence() {
    let (_g, rt, device) = runtime_with_device_spec().await;
    let id = ProviderId::new("provider.adapter.desk-light");
    let report = rt.test_provider(&id).await.unwrap();

    assert_eq!(report["ok"], Value::Bool(true), "report: {report}");
    assert_eq!(report["receptorId"], Value::from("desk-light.status"));
    assert_eq!(report["tested"]["how"], Value::from("human"));
    assert_eq!(report["tested"]["ok"], Value::Bool(true));
    // 唯讀：測試不得觸發任何動器（裝置沒收到任何 /set）。
    assert!(device.set_calls.lock().unwrap().is_empty());

    // 證據出現在 provider descriptor 上，並且跨 list/get 一致。
    let desk = provider_named(&rt, id.as_str()).await;
    let tested = tested_of(&desk).expect("tested recorded");
    assert_eq!(tested["how"], Value::from("human"));
    assert_eq!(tested["ok"], Value::Bool(true));
    assert!(tested["at"].as_str().is_some());
    let listed = rt
        .list_providers()
        .await
        .into_iter()
        .find(|p| p.identity.id == id)
        .unwrap();
    assert_eq!(tested_of(&listed), Some(tested));

    // 稽核留痕（誰、測了什麼、結果）。
    let audit = rt.store.audit_tail(20).unwrap();
    assert!(audit.iter().any(|row| {
        row.get("kind").and_then(|v| v.as_str()) == Some("provider.tested")
            && row
                .get("detail")
                .and_then(|d| d.get("ok"))
                .and_then(|v| v.as_bool())
                == Some(true)
    }));

    // 之後的生命週期轉換（安裝→停用→啟用）不得把證據洗掉：
    // 「已測試」與「已啟用」是兩個不同的階，各自成立。
    rt.transition_provider(&id, ProviderState::Disabled)
        .await
        .unwrap();
    let enabled = rt
        .transition_provider(&id, ProviderState::Available)
        .await
        .unwrap();
    assert_eq!(enabled.state, ProviderState::Available);
    let after = tested_of(&provider_named(&rt, id.as_str()).await).expect("evidence kept");
    assert_eq!(after["how"], Value::from("human"));
    assert_eq!(after["ok"], Value::Bool(true));
}

#[tokio::test]
async fn provider_test_reports_the_reason_when_it_cannot_read() {
    let (_g, rt, _device) = runtime_with_device_spec().await;
    let id = ProviderId::new("provider.adapter.desk-light");
    // 受器被停用 ⇒ 測試不得偷偷打開它，必須誠實回報讀不到。
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("desk-light.status"), false)
        .await
        .unwrap();
    let report = rt.test_provider(&id).await.unwrap();
    assert_eq!(report["ok"], Value::Bool(false), "report: {report}");
    assert!(report["reason"].as_str().unwrap().contains("disabled"));
    let tested = tested_of(&provider_named(&rt, id.as_str()).await).expect("failure recorded");
    assert_eq!(tested["ok"], Value::Bool(false));
    assert_eq!(tested["how"], Value::from("human"));
}

#[tokio::test]
async fn successful_receptor_read_records_capability_evidence() {
    let (_g, rt, _device) = runtime_with_device_spec().await;
    let id = ProviderId::new("provider.adapter.desk-light");
    assert!(tested_of(&provider_named(&rt, id.as_str()).await).is_none());

    // 沒有人按按鈕：一次真的成功的讀取就足以證明「連得上」。
    rt.observe_fresh(&ReceptorId::new("desk-light.status"))
        .await
        .unwrap();

    let tested = tested_of(&provider_named(&rt, id.as_str()).await).expect("tested recorded");
    // HTTP 宣告式裝置沒有 hello/pair 握手鏈路 ⇒ 證據等級是 capability。
    assert_eq!(tested["how"], Value::from("capability"));
    assert_eq!(tested["ok"], Value::Bool(true));
    assert!(tested["note"]
        .as_str()
        .unwrap()
        .contains("desk-light.status"));
    // note 是一般模式原樣顯示的人話：感知來源＋讀取成功，不出現受器／hello／pair-ok；
    // 沒有實體連線握手就不得宣稱「報上身分並完成配對」。
    let note = tested["note"].as_str().unwrap();
    assert!(note.contains("感知來源"), "{note}");
    assert!(note.contains("讀取成功"), "{note}");
    for jargon in ["受器", "hello", "pair-ok", "配對"] {
        assert!(!note.contains(jargon), "{jargon} leaked into {note}");
    }
}

#[tokio::test]
async fn tested_evidence_survives_restart_but_never_re_arms_the_device() {
    let dir = tempfile::tempdir().unwrap();
    let id = ProviderId::new("provider.device.tested");
    {
        let rt = Runtime::start(RuntimeOptions {
            home: Some(dir.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        rt.providers
            .register(interaction_registry::providers::discovered(
                ProviderIdentity {
                    id: id.clone(),
                    kind: ProviderKind::Device,
                    display_name: "tested device".into(),
                    trust_level: TrustLevel::Discovered,
                    origin: "local-network".into(),
                    version: "1".into(),
                    fingerprint: None,
                    human: None,
                },
            ))
            .await
            .unwrap();
        rt.pair_provider(&id, "445566").await.unwrap();
        rt.transition_provider(&id, ProviderState::Installed)
            .await
            .unwrap();
        rt.transition_provider(&id, ProviderState::Disabled)
            .await
            .unwrap();
        rt.transition_provider(&id, ProviderState::Available)
            .await
            .unwrap();
        // 沒有受器可測 ⇒ 誠實記為 ok:false（不是「已測試」）。
        let report = rt.test_provider(&id).await.unwrap();
        assert_eq!(report["ok"], Value::Bool(false));
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
    let restored = provider_named(&rt, id.as_str()).await;
    // 證據留著（它記的是過去確實發生過的事）……
    let tested = tested_of(&restored).expect("evidence survives restart");
    assert_eq!(tested["ok"], Value::Bool(false));
    // ……但重啟不得讓裝置自己回到可用狀態，人話註記也還在。
    assert_eq!(restored.state, ProviderState::Disabled);
    let note: Value = serde_json::from_str(restored.detail.as_deref().unwrap()).unwrap();
    assert_eq!(
        note["note"],
        Value::from("re-armed on restart requires explicit enable")
    );
}

#[tokio::test]
async fn acknowledged_device_command_records_evidence_for_its_provider() {
    let (_g, rt, _device) = runtime_with_device_spec().await;
    let id = ProviderId::new("provider.adapter.desk-light");
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("desk-light.set"), true)
        .await
        .unwrap();
    rt.update_policy(json!({"actuatorAllowlist": ["desk-light.set"], "allowedChannels": ["conversation", "web-ui", "log", "light"]}))
        .await
        .unwrap();
    rt.start_session(
        Some("test".into()),
        None,
        vec!["actuator:desk-light.set".into()],
    )
    .await
    .unwrap();
    let mut intent = SemanticIntent::new("calm");
    intent.magnitude = Some(0.5);
    intent.preferred_channels = vec!["light".into()];
    let plan = rt
        .create_plan(
            intent,
            vec!["desk-light.set".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert_eq!(receipts[0].current_status, ActionStatus::Acknowledged);

    let tested = tested_of(&provider_named(&rt, id.as_str()).await).expect("tested recorded");
    assert_eq!(tested["ok"], Value::Bool(true));
    assert_eq!(tested["how"], Value::from("capability"));
    assert!(tested["note"].as_str().unwrap().contains("acknowledged"));
    // 人話：回應方式＋已回覆收到，明說不代表已完成；不出現動器。
    let note = tested["note"].as_str().unwrap();
    assert!(note.contains("回應方式"), "{note}");
    assert!(note.contains("不代表已完成"), "{note}");
    assert!(!note.contains("動器"), "{note}");
}
