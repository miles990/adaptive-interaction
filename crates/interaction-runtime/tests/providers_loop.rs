//! Provider + declarative-device integration: a spec file in the home config
//! becomes real capabilities behind the SAME governor as everything else.
//! Pairing / install / enable / consent stay separate; revocation disables
//! capabilities immediately; the device only ever sees bounded values.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::providers::{CapabilitySelector, ProviderCapabilityDeclaration};
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

// ---------------------------------------------------------------------------
// 建置時的設定檔警告（明文憑證）必須跟著 provider 走，不得被靜靜吞掉
// ---------------------------------------------------------------------------

/// broker 位址故意指向一個沒有人在聽的 loopback 埠：這個測試不需要真的連上
/// broker（rumqttc 的重連在背景 task 裡），只驗證「明文憑證的警告有沒有被
/// 誠實帶到 provider 記錄上」。
async fn runtime_with_plaintext_credential_spec() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let adapters = dir.path().join("config").join("adapters");
    std::fs::create_dir_all(&adapters).unwrap();
    std::fs::write(
        adapters.join("esp32-plain.yaml"),
        r#"
schemaVersion: "1.0"
id: esp32-plain
displayName: 明文憑證裝置
capabilities:
  - kind: actuator
    id: vibe
    channel: haptic
    transport: mqtt
    timeoutMs: 4000
    command:
      name: "vibe.pulse"
      params: { strength: "{{magnitude}}" }
    mqtt:
      brokerHost: "127.0.0.1"
      brokerPort: 1
      topicPrefix: "interact-ai/esp32-plain"
      expectedDeviceId: "esp32-plain"
      pairingCode: "123456"
"#,
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
    (dir, rt)
}

#[tokio::test]
async fn plaintext_credential_warnings_reach_the_provider_record() {
    let (_g, rt) = runtime_with_plaintext_credential_spec().await;
    let id = ProviderId::new("provider.adapter.esp32-plain");
    let desc = provider_named(&rt, id.as_str()).await;
    let detail = desc.detail.clone().expect("警告必須寫進 provider detail");

    // (a) 原文（點名能力與欄位）給 CLI／進階模式。
    let warnings = interaction_runtime::providers::provider_detail_warnings(Some(&detail));
    assert_eq!(warnings.len(), 1, "{detail}");
    assert!(warnings[0].contains("pairingCode"), "{detail}");
    assert!(warnings[0].contains("secret://"), "{detail}");
    assert!(
        !warnings[0].contains("123456"),
        "警告永遠不得回顯憑證值：{detail}"
    );

    // (b) 一般模式看到的那一句是人話摘要，不外洩欄位名。
    let (note, _tested) = interaction_runtime::providers::split_provider_detail(Some(&detail));
    let note = note.expect("一般模式要有一句人話");
    assert!(note.contains("安全提醒"), "{note}");
    assert!(!note.contains("pairingCode"), "{note}");
    assert!(!note.contains("secret://"), "{note}");

    // (c) 狀態改變（啟用／停用）不得把警告洗掉——那是這台裝置的事實，
    //     不是上一個狀態的註記。
    rt.transition_provider(&id, ProviderState::Disabled)
        .await
        .unwrap();
    rt.transition_provider(&id, ProviderState::Available)
        .await
        .unwrap();
    let after = provider_named(&rt, id.as_str()).await;
    let after_warnings =
        interaction_runtime::providers::provider_detail_warnings(after.detail.as_deref());
    assert_eq!(
        after_warnings, warnings,
        "換狀態之後警告不見了：{:?}",
        after.detail
    );

    // 收工：關掉這個 provider 開出來的連線，別留背景重連 task。
    rt.transition_provider(&id, ProviderState::Disabled)
        .await
        .unwrap();
}

/// 沒有警告的 spec 不得憑空長出 `warnings` 鍵（也不得把純文字註記變成 JSON）。
#[tokio::test]
async fn a_spec_without_plaintext_credentials_gets_no_warnings() {
    let (_g, rt, _device) = runtime_with_device_spec().await;
    let desk = provider_named(&rt, "provider.adapter.desk-light").await;
    assert!(
        interaction_runtime::providers::provider_detail_warnings(desk.detail.as_deref()).is_empty(),
        "{:?}",
        desk.detail
    );
}

// ---------------------------------------------------------------------------
// 對抗審查修復回歸（safety-invariants-074）：停用／撤銷必須跨重啟
// ---------------------------------------------------------------------------

/// 在同一個 home 上重開一個 runtime（模擬 daemon 重啟）。
async fn restart_runtime(home: &std::path::Path) -> Runtime {
    Runtime::start(RuntimeOptions {
        home: Some(home.to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap()
}

/// 撤銷一台宣告式裝置之後重啟 daemon：spec 會被重新載入，但受器不得回到
/// 啟用、也不得能再讀到它（重啟不是重新授權，更不是撤銷的後門）。
#[tokio::test]
async fn a_revoked_declarative_device_stays_off_across_a_restart() {
    let (home, rt, _device) = runtime_with_device_spec().await;
    let desk = ProviderId::new("provider.adapter.desk-light");
    let status = ReceptorId::new("desk-light.status");

    // 撤銷前：讀得到（這正是撤銷要收回的東西）。
    rt.registry
        .set_receptor_enabled(&status, true)
        .await
        .unwrap();
    assert!(rt.observe_fresh(&status).await.is_ok(), "撤銷前應該讀得到");
    rt.revoke_provider(&desk).await.unwrap();
    assert!(
        rt.registry.receptor(&status).await.is_err(),
        "撤銷當下就必須停用"
    );
    rt.shutdown().await;

    let rt = restart_runtime(home.path()).await;
    assert_eq!(
        rt.get_provider(&desk).await.unwrap().state,
        ProviderState::Revoked,
        "撤銷是黏著的"
    );
    assert!(
        rt.registry.receptor(&status).await.is_err(),
        "撤銷後重啟不得讓裝置受器回到啟用"
    );
    assert!(
        rt.observe_fresh(&status).await.is_err(),
        "撤銷後重啟不得能再讀這台裝置"
    );
    assert!(
        rt.store
            .audit_tail(100)
            .unwrap()
            .into_iter()
            .any(|a| a["kind"] == json!("provider.kept-off-at-start")),
        "重啟時維持關閉要留痕"
    );
}

/// 人類按下的「停用」同樣要跨重啟；重新啟用之後，下一次啟動才恢復
/// （恢復的條件是人類的啟用，不是重開 daemon）。
#[tokio::test]
async fn a_disabled_declarative_device_stays_off_until_a_human_enables_it_again() {
    let (home, rt, _device) = runtime_with_device_spec().await;
    let desk = ProviderId::new("provider.adapter.desk-light");
    let status = ReceptorId::new("desk-light.status");
    rt.registry
        .set_receptor_enabled(&status, true)
        .await
        .unwrap();
    rt.transition_provider(&desk, ProviderState::Disabled)
        .await
        .unwrap();
    rt.shutdown().await;

    let rt = restart_runtime(home.path()).await;
    assert!(
        rt.registry.receptor(&status).await.is_err(),
        "停用後重啟不得自動恢復"
    );

    // 人類重新啟用 → 下一次啟動才回來（證明修復沒有把裝置永久鎖死）。
    rt.transition_provider(&desk, ProviderState::Available)
        .await
        .unwrap();
    rt.shutdown().await;
    let rt = restart_runtime(home.path()).await;
    assert!(
        rt.registry.receptor(&status).await.is_ok(),
        "人類重新啟用之後，重啟應該恢復這台裝置"
    );
}

// ---------------------------------------------------------------------------
// 9.8 provider 狀態必須真的擋住執行期：停用／撤銷的 provider 不得再被觀察或
// 派工，能力清單也不得繼續宣稱 Available（誠實階梯：狀態 ≠ 標籤）。
// ---------------------------------------------------------------------------

/// 讓書桌燈動器可以被派工的最小前置（能力啟用＋政策允許＋session 同意）。
async fn arm_desk_light_actuator(rt: &Runtime) {
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
}

/// 先把計畫做好（此時 provider 還開著），停用之後才執行：擋必須發生在
/// 派工前的那一刻，而不是只靠規劃時的能力清單。
async fn plan_desk_light(rt: &Runtime) -> Plan {
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
    assert_eq!(plan.steps.len(), 1, "{plan:?}");
    plan
}

async fn run_plan(rt: &Runtime, plan: &Plan) -> ActionReceipt {
    rt.execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap()
        .remove(0)
}

#[tokio::test]
async fn a_disabled_provider_blocks_observe_even_when_the_receptor_stays_enabled() {
    let (_g, rt, _device) = runtime_with_device_spec().await;
    let desk = ProviderId::new("provider.adapter.desk-light");
    let status = ReceptorId::new("desk-light.status");
    rt.registry
        .set_receptor_enabled(&status, true)
        .await
        .unwrap();
    assert!(rt.observe_fresh(&status).await.is_ok(), "停用前讀得到");

    rt.transition_provider(&desk, ProviderState::Disabled)
        .await
        .unwrap();
    // 停用會把能力層旗標一併關掉（v0.5.1 已知限制 #4 的修正）。這個測試要驗的
    // 是**另一層**：即使旗標是開的，provider 狀態本身也必須擋得住。所以刻意把
    // 旗標手動扳回開啟，讓唯一可能的擋來自 provider 狀態。
    rt.registry
        .set_receptor_enabled(&status, true)
        .await
        .unwrap();
    assert!(
        rt.registry.receptor(&status).await.is_ok(),
        "前提：能力層旗標是開的（擋只能來自 provider 狀態）"
    );
    let err = rt.observe_fresh(&status).await.unwrap_err().to_string();
    assert!(err.contains("desk-light.status"), "{err}");
    assert!(err.contains("provider.adapter.desk-light"), "{err}");
    assert!(err.contains("disabled"), "{err}");

    // 人類重新啟用 ⇒ 立刻回來（不得永久鎖死）。
    rt.transition_provider(&desk, ProviderState::Available)
        .await
        .unwrap();
    assert!(rt.observe_fresh(&status).await.is_ok(), "重新啟用後要恢復");
}

#[tokio::test]
async fn a_disabled_provider_blocks_execution_with_a_blocked_receipt() {
    let (_g, rt, device) = runtime_with_device_spec().await;
    let desk = ProviderId::new("provider.adapter.desk-light");
    arm_desk_light_actuator(&rt).await;
    let plan = plan_desk_light(&rt).await;
    rt.transition_provider(&desk, ProviderState::Disabled)
        .await
        .unwrap();
    // 同上：旗標手動扳回開啟，證明 blocked receipt 是 provider 狀態擋下來的，
    // 不是「動器剛好被關掉」的副作用。
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("desk-light.set"), true)
        .await
        .unwrap();
    assert!(
        rt.registry
            .actuator(&ActuatorId::new("desk-light.set"))
            .await
            .is_ok(),
        "前提：能力層旗標是開的（擋只能來自 provider 狀態）"
    );

    let receipt = run_plan(&rt, &plan).await;
    // 誠實形狀：blocked receipt（不是 raw error），理由點名 provider。
    assert_eq!(receipt.current_status, ActionStatus::Blocked);
    let reason = receipt
        .policy_decisions
        .iter()
        .find_map(|d| match d {
            PolicyDecision::Blocked { rule, reason } if rule == "provider.not-operational" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a provider block: {:?}", receipt.policy_decisions));
    assert!(reason.contains("provider.adapter.desk-light"), "{reason}");
    assert!(
        device.set_calls.lock().unwrap().is_empty(),
        "停用中的 provider 不得真的把命令送到裝置"
    );

    // 停用之後才規劃：能力清單已經誠實地不可用，規劃階段就先擋下來（防禦
    // 縱深，不是取代派工前的閘門）。
    let mut intent = SemanticIntent::new("calm");
    intent.magnitude = Some(0.5);
    intent.preferred_channels = vec!["light".into()];
    let late = rt
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
    assert_eq!(late.status, PlanStatus::Blocked);
    assert!(late.steps.is_empty());
}

#[tokio::test]
async fn capability_availability_reports_disabled_while_the_provider_is_off() {
    let (_g, rt, _device) = runtime_with_device_spec().await;
    let desk = ProviderId::new("provider.adapter.desk-light");
    let status = ReceptorId::new("desk-light.status");
    rt.registry
        .set_receptor_enabled(&status, true)
        .await
        .unwrap();
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("desk-light.set"), true)
        .await
        .unwrap();

    let availability = |snap: &CapabilitySnapshot| {
        let r = snap
            .receptors
            .iter()
            .find(|m| m.id.as_str() == "desk-light.status")
            .expect("receptor listed")
            .availability;
        let a = snap
            .actuators
            .iter()
            .find(|m| m.id.as_str() == "desk-light.set")
            .expect("actuator listed")
            .availability;
        (r, a)
    };
    let all = DiscoveryContext {
        include_unavailable: true,
        ..Default::default()
    };
    assert_eq!(
        availability(&rt.capabilities(&all).await),
        (Availability::Available, Availability::Available)
    );

    rt.transition_provider(&desk, ProviderState::Disabled)
        .await
        .unwrap();
    assert_eq!(
        availability(&rt.capabilities(&all).await),
        (Availability::Disabled, Availability::Disabled),
        "provider 關掉時能力清單不得繼續宣稱可用"
    );
}

#[tokio::test]
async fn a_revoked_provider_stays_blocked_even_if_a_capability_is_re_enabled() {
    let (_g, rt, device) = runtime_with_device_spec().await;
    let desk = ProviderId::new("provider.adapter.desk-light");
    let status = ReceptorId::new("desk-light.status");
    arm_desk_light_actuator(&rt).await;
    let plan = plan_desk_light(&rt).await;
    rt.revoke_provider(&desk).await.unwrap();

    // 撤銷會關掉能力旗標；把旗標重新打開不得成為繞過撤銷的後門。
    rt.registry
        .set_receptor_enabled(&status, true)
        .await
        .unwrap();
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("desk-light.set"), true)
        .await
        .unwrap();

    let err = rt.observe_fresh(&status).await.unwrap_err().to_string();
    assert!(err.contains("revoked"), "{err}");
    let receipt = run_plan(&rt, &plan).await;
    assert_eq!(receipt.current_status, ActionStatus::Blocked);
    assert!(receipt.policy_decisions.iter().any(|d| matches!(
        d,
        PolicyDecision::Blocked { rule, .. } if rule == "provider.not-operational"
    )));
    assert!(
        device.set_calls.lock().unwrap().is_empty(),
        "撤銷後不得再送任何命令到裝置"
    );
}

/// 升級邊界：舊版留下的「已停用」provider 記錄沒有 provider-off 記號時，
/// 第一次重啟必須採安全預設（能力維持關閉），並留下要人類確認的痕跡。
#[tokio::test]
async fn a_legacy_disabled_provider_without_the_off_marker_stays_locked_after_restart() {
    let (home, rt, _device) = runtime_with_device_spec().await;
    let desk = ProviderId::new("provider.adapter.desk-light");
    let status = ReceptorId::new("desk-light.status");
    rt.registry
        .set_receptor_enabled(&status, true)
        .await
        .unwrap();
    rt.shutdown().await;

    // 直接改寫落地記錄，模擬舊版寫下的 state=disabled（沒有 provider-off meta）。
    {
        let store =
            interaction_storage::Store::open(&home.path().join("state").join("interaction.db"))
                .unwrap();
        let body = store
            .all_providers()
            .unwrap()
            .into_iter()
            .map(|b| serde_json::from_str::<Value>(&b).unwrap())
            .find(|v| v["identity"]["id"] == json!(desk.as_str()))
            .expect("persisted desk provider");
        let mut body = body;
        body["state"] = json!("disabled");
        body["detail"] = json!("停用（舊版寫下的記錄）");
        store
            .save_provider(desk.as_str(), &serde_json::to_string(&body).unwrap())
            .unwrap();
        assert!(
            store
                .get_meta(&format!("provider-off:{}", desk.as_str()))
                .unwrap()
                .is_none(),
            "前置條件：沒有 provider-off 記號"
        );
    }

    let rt = restart_runtime(home.path()).await;
    assert_eq!(
        rt.get_provider(&desk).await.unwrap().state,
        ProviderState::Disabled
    );
    assert!(
        rt.registry.receptor(&status).await.is_err(),
        "舊記錄沒有記號時，第一次重啟必須採安全預設（能力維持關閉）"
    );
    assert!(rt.observe_fresh(&status).await.is_err());
    let audit = rt.store.audit_tail(200).unwrap();
    assert!(
        audit
            .iter()
            .any(|row| row["kind"] == json!("provider.legacy-off-assumed")),
        "採安全預設必須留痕（請使用者確認）"
    );
}

// ---------------------------------------------------------------------------
// protocol-conformance-030：「裝置宣稱不需配對」不得等同「配對已驗證」
//
// DeviceLink 在裝置 hello.pairing=false 時記下 pairing_unverified（spec 配了
// 配對碼，但沒有任何一方比對過它——參考韌體對任何碼都回 pair-ok），
// link_caps 把 `pairingUnverified` 寫進 driver 收據。這一段測 executor →
// providers 的投影：provider 的「已測試」證據必須把這件事說出來，否則 CLI／
// UI 會把「從未驗證過的配對」顯示成與真配對完全無法區分的已測試／已啟用。
// ---------------------------------------------------------------------------

use interaction_adapter_declarative::link_caps::LinkActuator;
use interaction_adapter_declarative::protocol::{DeviceLink, DeviceMsg, LinkError, RawLink};
use interaction_adapter_declarative::{CapabilitySpec, CommandSpec};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::broadcast;

/// 最小的假傳輸：只回 hello／pair／cmd 三種訊息，其餘一律忽略。
/// `device_code = None` 模擬 PAIRING_CODE 為空的韌體：hello.pairing=false，
/// 且對**任何**碼都回 pair-ok（＝那組碼從未被比對過）。
struct MockRawLink {
    inbound: broadcast::Sender<DeviceMsg>,
    device_id: String,
    device_code: Option<String>,
    paired: AtomicBool,
    closed: AtomicBool,
}

impl MockRawLink {
    fn new(device_id: &str, device_code: Option<&str>) -> Arc<Self> {
        let (inbound, _) = broadcast::channel(16);
        Arc::new(Self {
            inbound,
            device_id: device_id.into(),
            device_code: device_code.map(String::from),
            paired: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        })
    }
}

#[async_trait::async_trait]
impl RawLink for MockRawLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable("mock link closed".into()));
        }
        Ok(())
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable("mock link closed".into()));
        }
        let msg: Value = serde_json::from_str(&line).expect("host sends json");
        let reply = match msg["type"].as_str() {
            Some("who") => Some(DeviceMsg::Hello {
                device_id: self.device_id.clone(),
                fw: Some("1.0.0".into()),
                proto: Some(1),
                caps: vec!["vibe.pulse".into()],
                pairing: self.device_code.is_some(),
                pairing_locked: false,
            }),
            Some("pair") => match &self.device_code {
                // 韌體停用配對：不比對任何碼，一律 pair-ok（參考韌體行為）。
                None => {
                    self.paired.store(true, Ordering::SeqCst);
                    Some(DeviceMsg::PairOk)
                }
                Some(code) if Some(code.as_str()) == msg["code"].as_str() => {
                    self.paired.store(true, Ordering::SeqCst);
                    Some(DeviceMsg::PairOk)
                }
                Some(_) => Some(DeviceMsg::PairFail {
                    reason: None,
                    retry_after_ms: None,
                }),
            },
            Some("cmd") => Some(DeviceMsg::Ack {
                id: msg["id"].as_str().map(String::from),
                applied: None,
                dup: None,
                cancelled: None,
                stop_all: None,
            }),
            _ => None,
        };
        if let Some(reply) = reply {
            let _ = self.inbound.send(reply);
        }
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        self.inbound.subscribe()
    }

    fn connected(&self) -> bool {
        !self.closed.load(Ordering::SeqCst)
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn describe(&self) -> String {
        "mock".into()
    }
}

/// 一台掛在假傳輸上的裝置：`device_wants_pairing=false` ⇒ 配對碼未經比對。
async fn runtime_with_mock_device(
    device_wants_pairing: bool,
) -> (tempfile::TempDir, Runtime, ProviderId, String) {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();

    let device_code = device_wants_pairing.then_some("4321");
    let raw = MockRawLink::new("esp32-desk01", device_code);
    let link = Arc::new(DeviceLink::new(
        raw,
        "esp32-desk01".into(),
        Some("4321".into()), // spec 兩邊都配了碼
    ));
    let spec: CapabilitySpec = serde_json::from_value(json!({
        "kind": "actuator",
        "id": "vibe",
        "channel": "haptic",
        "transport": "serial",
        "timeoutMs": 500,
    }))
    .expect("actuator spec");
    let actuator = LinkActuator::new(
        spec,
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link,
        "serial",
    );
    let actuator_id = actuator.manifest().id.as_str().to_string();
    rt.registry
        .register_actuator(Arc::new(actuator))
        .await
        .unwrap();

    let provider_id = ProviderId::new("provider.adapter.esp32-desk");
    let mut descriptor = interaction_registry::providers::discovered(ProviderIdentity {
        id: provider_id.clone(),
        kind: ProviderKind::Device,
        display_name: "ESP32 桌面裝置".into(),
        trust_level: TrustLevel::Paired,
        origin: "declarative".into(),
        version: "1".into(),
        fingerprint: None,
        human: None,
    });
    descriptor.state = ProviderState::Installed;
    descriptor.actuators = vec![actuator_id.clone()];
    rt.providers.register(descriptor).await.unwrap();
    (dir, rt, provider_id, actuator_id)
}

async fn acknowledge_once(rt: &Runtime, actuator_id: &str, expect_receipt_flag: bool) {
    rt.registry
        .set_actuator_enabled(&ActuatorId::new(actuator_id), true)
        .await
        .unwrap();
    rt.update_policy(json!({
        "actuatorAllowlist": [actuator_id],
        "allowedChannels": ["conversation", "web-ui", "log", "haptic"],
    }))
    .await
    .unwrap();
    rt.start_session(
        Some("test".into()),
        None,
        vec![format!("actuator:{actuator_id}")],
    )
    .await
    .unwrap();
    let mut intent = SemanticIntent::new("calm");
    intent.magnitude = Some(0.5);
    intent.preferred_channels = vec!["haptic".into()];
    let plan = rt
        .create_plan(
            intent,
            vec![actuator_id.to_string()],
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
    assert_eq!(
        receipts[0].current_status,
        ActionStatus::Acknowledged,
        "前置條件：裝置真的回了 ack（{:?}）",
        receipts[0]
    );
    // 前置條件：這個測試的輸入是 driver 收據上的旗標（由 link_caps 寫入）。
    assert_eq!(
        receipts[0]
            .driver_response
            .get("pairingUnverified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        expect_receipt_flag,
        "driver 收據：{:?}",
        receipts[0].driver_response
    );
}

#[tokio::test]
async fn a_device_that_never_verified_the_pairing_code_downgrades_the_provider_evidence() {
    let (_g, rt, provider_id, actuator_id) = runtime_with_mock_device(false).await;
    acknowledge_once(&rt, &actuator_id, true).await;

    let tested = tested_of(&provider_named(&rt, provider_id.as_str()).await)
        .expect("acknowledged 命令必須留下證據");
    assert_eq!(tested["ok"], Value::Bool(true));
    // how 的三個既有取值不變（不新增第四種字串，避免相容性破壞）。
    assert_eq!(tested["how"], Value::from("capability"));
    assert_eq!(
        tested["pairingUnverified"],
        Value::Bool(true),
        "裝置說它不需要配對＝那組碼從未被比對過，證據等級必須誠實降級：{tested}"
    );
    let note = tested["note"].as_str().unwrap();
    assert!(
        note.contains("無法證明配對碼被比對過") && note.contains("裝置自報"),
        "人話註記要說出身分證據只有裝置自報的 deviceId：{note}"
    );
    assert!(
        !note.contains("完成配對"),
        "不得沿用「已完成配對」的說法：{note}"
    );
}

#[tokio::test]
async fn a_device_that_really_verified_the_pairing_code_keeps_a_clean_record() {
    let (_g, rt, provider_id, actuator_id) = runtime_with_mock_device(true).await;
    acknowledge_once(&rt, &actuator_id, false).await;

    let tested = tested_of(&provider_named(&rt, provider_id.as_str()).await)
        .expect("acknowledged 命令必須留下證據");
    assert_eq!(tested["ok"], Value::Bool(true));
    assert_eq!(tested["how"], Value::from("capability"));
    assert!(
        tested.get("pairingUnverified").is_none()
            || tested["pairingUnverified"] == Value::Bool(false),
        "真的比對過配對碼的裝置不得被標成未驗證：{tested}"
    );
    let note = tested["note"].as_str().unwrap();
    assert!(
        !note.contains("無法證明配對碼被比對過") && !note.contains("配對碼未經比對"),
        "{note}"
    );
}

// ---------------------------------------------------------------------------
// v0.6.0：跨切面能力語意一律由 provider 自己宣告
// ---------------------------------------------------------------------------

async fn plain_runtime() -> (tempfile::TempDir, Runtime) {
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

/// 呈現面 actuator、高風險受器、感測來源的人話種類名，全部由 provider 註冊時
/// 自己宣告——runtime 核心（character／activity／sensors）不得再認得任何具名
/// 裝置的能力字面值。這裡用一個跟 iPhone 完全無關的假 provider 證明去耦合：
/// 只要它宣告了，三個語意就同時生效；沒宣告的能力一個都不算。
#[tokio::test]
async fn provider_declarations_drive_presentation_and_sensor_semantics() {
    let (_g, rt) = plain_runtime().await;
    let decls = rt.capability_declarations();

    // 開機時內建 provider 已經各自宣告過（誰宣告的由 provider 自己說）。
    assert!(
        !decls.declaration_ids().is_empty(),
        "內建 provider 開機時就要宣告自己的能力語意"
    );

    // 沒宣告過的 provider：三個語意都不成立。
    assert!(!decls.is_presentation_surface("robot.face"));
    assert!(decls.class_label_of_receptor("robot.mic-level").is_none());
    assert!(!decls
        .high_risk_receptors()
        .iter()
        .any(|r| r == "robot.mic-level"));

    rt.declare_provider_capabilities(
        ProviderCapabilityDeclaration::new("provider.fake.robot")
            .with_class_label("測試機器人")
            .with_presentation_surface(CapabilitySelector::exact("robot.face"))
            .with_presentation_surface(CapabilitySelector::prefix("robot.screen."))
            .with_receptor("robot.mic-level")
            .with_high_risk_receptor("robot.mic-level"),
    );

    assert!(decls.is_presentation_surface("robot.face"));
    assert!(decls.is_presentation_surface("robot.screen.left"));
    assert!(
        !decls.is_presentation_surface("robot.arm"),
        "沒宣告成呈現面的動器仍然是一般動器"
    );
    assert_eq!(
        decls.class_label_of_receptor("robot.mic-level").as_deref(),
        Some("測試機器人")
    );
    assert!(decls
        .high_risk_receptors()
        .iter()
        .any(|r| r == "robot.mic-level"));
    assert!(decls
        .declaration_ids()
        .iter()
        .any(|id| id == "provider.fake.robot"));
}

/// 收件匣的感測事件標題用 provider 宣告的人話種類名，不是能力 id 的字面前綴。
/// 沒有任何 provider 宣告過的感測器一律退回中性字樣，原始 id 不上標題。
#[tokio::test]
async fn inbox_sensor_titles_use_the_provider_declared_class_label() {
    let (_g, rt) = plain_runtime().await;
    rt.declare_provider_capabilities(
        ProviderCapabilityDeclaration::new("provider.fake.robot")
            .with_class_label("測試機器人")
            .with_receptor("robot.mic-level"),
    );
    rt.events.emit(
        EventType::SensorStarted,
        json!({"sensor": "robot.mic-level", "deviceId": "robot-a1b2"}),
    );
    rt.events.emit(
        EventType::SensorStarted,
        json!({"sensor": "nobody.declared-this"}),
    );

    let inbox = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter::default())
        .await
        .unwrap();
    let titles: Vec<String> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == json!("safety-event"))
        .filter_map(|item| item["title"].as_str().map(String::from))
        .collect();
    assert!(
        titles.iter().any(|t| t == "感測開始：測試機器人"),
        "標題要用 provider 宣告的種類名：{titles:?}"
    );
    assert!(
        titles.iter().any(|t| t == "感測開始：感測器"),
        "沒人宣告的感測器退回中性字樣：{titles:?}"
    );
    assert!(
        titles
            .iter()
            .all(|t| !t.contains("robot.mic-level") && !t.contains("robot-a1b2")),
        "原始能力 id／裝置 id 不得上標題：{titles:?}"
    );
}

/// 一個跟任何真實裝置都無關的受器 fixture：旗標測試只需要「registry 裡真的
/// 存在一個可以被開關的受器」，不需要它真的讀得到東西。
struct FakeRobotMic;

#[async_trait::async_trait]
impl Receptor for FakeRobotMic {
    fn manifest(&self) -> ReceptorManifest {
        serde_json::from_value(json!({
            "id": "robot.mic-level",
            "name": "測試機器人音量",
            "description": "fixture receptor for the provider capability-flag tests",
            "category": "device",
            "provides": ["level"],
            "mode": "poll",
            "sensitivity": "intimate",
            "requiresConsent": true,
            "driver": "fixture.robot",
            "version": "0.0.0",
            "schemaVersion": "1.0"
        }))
        .expect("fixture manifest parses")
    }

    async fn start(&self, _context: SessionContext) -> Result<(), ReceptorError> {
        Ok(())
    }

    async fn read(&self) -> Result<Observation, ReceptorError> {
        Err(ReceptorError::Unavailable("fixture".into()))
    }

    async fn health(&self) -> ComponentHealth {
        ComponentHealth::healthy()
    }

    async fn stop(&self) -> Result<(), ReceptorError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 宣告表的生命週期：register / update / remove（M2 §3.2 ownership）
// ---------------------------------------------------------------------------

/// **update**：同一個 `declaration_id` 再宣告一次＝整份覆寫，不是累加補丁。
/// 上一版宣告過的呈現面／高風險受器必須真的消失——留下殘影的話，一個
/// provider 改了自己的能力表之後，核心會永遠記得一個它已經不再提供的能力。
#[tokio::test]
async fn redeclaring_a_provider_replaces_the_whole_declaration() {
    let (_g, rt) = plain_runtime().await;
    rt.declare_provider_capabilities(
        ProviderCapabilityDeclaration::new("provider.fake.robot")
            .with_class_label("第一版機器人")
            .with_presentation_surface(CapabilitySelector::exact("robot.face"))
            .with_receptor("robot.mic-level")
            .with_high_risk_receptor("robot.mic-level"),
    );
    let decls = rt.capability_declarations();
    assert!(decls.is_presentation_surface("robot.face"));

    // 第二版：呈現面換人、受器不再是高風險、種類名改了。
    rt.declare_provider_capabilities(
        ProviderCapabilityDeclaration::new("provider.fake.robot")
            .with_class_label("第二版機器人")
            .with_presentation_surface(CapabilitySelector::exact("robot.chest"))
            .with_receptor("robot.mic-level"),
    );

    assert!(
        !decls.is_presentation_surface("robot.face"),
        "舊的呈現面必須消失（整份覆寫）：{:?}",
        decls.declaration("provider.fake.robot")
    );
    assert!(decls.is_presentation_surface("robot.chest"));
    assert!(
        !decls
            .high_risk_receptors()
            .iter()
            .any(|r| r == "robot.mic-level"),
        "舊的高風險宣告必須消失：{:?}",
        decls.high_risk_receptors()
    );
    assert_eq!(
        decls.class_label_of_receptor("robot.mic-level").as_deref(),
        Some("第二版機器人")
    );
    assert_eq!(
        decls
            .declaration_ids()
            .iter()
            .filter(|id| *id == "provider.fake.robot")
            .count(),
        1,
        "覆寫不得留下第二筆"
    );
}

/// **remove**：整族來源不再存在時撤回宣告，之後所有語意查詢都不得再包含它。
/// （下一波「宣告式 adapter 的 spec 被刪掉」會用到這條路。）
#[tokio::test]
async fn retracting_a_declaration_removes_its_capability_semantics() {
    let (_g, rt) = plain_runtime().await;
    rt.declare_provider_capabilities(
        ProviderCapabilityDeclaration::new("provider.fake.robot")
            .with_class_label("測試機器人")
            .with_presentation_surface(CapabilitySelector::exact("robot.face"))
            .with_receptor("robot.mic-level")
            .with_high_risk_receptor("robot.mic-level"),
    );
    let decls = rt.capability_declarations();
    assert!(decls.is_presentation_surface("robot.face"));

    assert!(
        rt.retract_provider_capabilities("provider.fake.robot"),
        "撤回一筆存在的宣告要回報 true"
    );

    assert!(!decls.is_presentation_surface("robot.face"));
    assert!(decls.class_label_of_receptor("robot.mic-level").is_none());
    assert!(!decls
        .high_risk_receptors()
        .iter()
        .any(|r| r == "robot.mic-level"));
    assert!(!decls
        .declaration_ids()
        .iter()
        .any(|id| id == "provider.fake.robot"));
    assert!(decls.declaration("provider.fake.robot").is_none());

    assert!(
        !rt.retract_provider_capabilities("provider.fake.robot"),
        "撤回不存在的宣告要誠實回報 false，不得假裝做了事"
    );
    assert!(
        !rt.retract_provider_capabilities("provider.never.declared"),
        "沒宣告過的 id 一樣是 false"
    );
}

/// 停用／撤銷**一台裝置**不動整族的能力宣告——這是刻意的設計，不是漏網。
///
/// 宣告說的是「這個能力 id 是什麼意思」（哪些是呈現面、哪些是高風險受器），
/// 不是「這台裝置現在可不可以用」。後者由 provider 狀態＋registry 旗標決定。
/// 若停用一台就把宣告刪掉，`stop_all_sensors` 會從此不知道那些受器是高風險，
/// 停止結果未知時就不再誠實補「可能還在擷取」——停用反而讓系統變得更不安全。
#[tokio::test]
async fn disabling_one_device_never_retracts_the_family_declaration() {
    let (_g, rt) = plain_runtime().await;
    rt.declare_provider_capabilities(
        ProviderCapabilityDeclaration::new("provider.fake.family")
            .with_class_label("測試機器人")
            .with_presentation_surface(CapabilitySelector::exact("robot.face"))
            .with_receptor("robot.mic-level")
            .with_high_risk_receptor("robot.mic-level"),
    );
    let device = interaction_core::ProviderId::new("provider.fake.family.unit-1");
    let mut desc =
        interaction_registry::providers::discovered(interaction_core::ProviderIdentity {
            id: device.clone(),
            kind: interaction_core::ProviderKind::Device,
            display_name: "測試機器人 #1".into(),
            trust_level: interaction_core::TrustLevel::Untrusted,
            origin: "test".into(),
            version: String::new(),
            fingerprint: None,
            human: None,
        });
    desc.receptors = vec!["robot.mic-level".into()];
    desc.state = interaction_core::ProviderState::Installed;
    rt.providers.register(desc).await.unwrap();

    rt.transition_provider(&device, interaction_core::ProviderState::Disabled)
        .await
        .unwrap();
    rt.revoke_provider(&device).await.unwrap();

    let decls = rt.capability_declarations();
    assert!(
        decls.is_presentation_surface("robot.face"),
        "撤銷一台裝置不得刪掉整族的呈現面宣告：{:?}",
        decls.declaration_ids()
    );
    assert!(
        decls
            .high_risk_receptors()
            .iter()
            .any(|r| r == "robot.mic-level"),
        "撤銷一台裝置不得讓整族的高風險受器宣告消失"
    );
    assert!(decls.declaration("provider.fake.family").is_some());
}

// ---------------------------------------------------------------------------
// v0.5.1 已知限制 #4：transition_provider 不會關掉 registry 旗標
// ---------------------------------------------------------------------------

/// 人類把一個 provider 轉成 Disabled／Revoked 之後，它底下的受器／動器在
/// registry 裡的 enabled 旗標必須真的翻掉——而不是只靠 ProviderGate 攔。
///
/// 為什麼旗標本身要翻：旗標是「這個能力現在開著嗎」的單一事實，狀態列、能力
/// 清單、`stop_all_sensors` 的「仍然啟用」判斷都讀它。閘門攔得住派工，卻讓
/// 使用者看到一個仍然「啟用中」的麥克風——那正是不誠實。
#[tokio::test]
async fn transitioning_a_provider_to_disabled_turns_off_its_capability_flags() {
    let (_g, rt) = plain_runtime().await;
    let device = interaction_core::ProviderId::new("provider.fake.flags");
    let receptor = interaction_core::ReceptorId::new("robot.mic-level");

    rt.registry
        .register_receptor(std::sync::Arc::new(FakeRobotMic))
        .await
        .expect("fixture receptor registers");
    let mut desc =
        interaction_registry::providers::discovered(interaction_core::ProviderIdentity {
            id: device.clone(),
            kind: interaction_core::ProviderKind::Device,
            display_name: "旗標測試機器人".into(),
            trust_level: interaction_core::TrustLevel::Untrusted,
            origin: "test".into(),
            version: String::new(),
            fingerprint: None,
            human: None,
        });
    desc.receptors = vec!["robot.mic-level".into()];
    desc.state = interaction_core::ProviderState::Installed;
    rt.providers.register(desc).await.unwrap();
    rt.registry
        .set_receptor_enabled(&receptor, true)
        .await
        .unwrap();
    assert!(
        rt.registry.receptor(&receptor).await.is_ok(),
        "前提：受器一開始是啟用的"
    );

    rt.transition_provider(&device, interaction_core::ProviderState::Disabled)
        .await
        .unwrap();

    assert!(
        rt.registry.receptor(&receptor).await.is_err(),
        "provider 被停用之後，它的受器旗標必須真的關掉（v0.5.1 已知限制 #4）"
    );
}

/// 重新啟用 provider **不得**自動把能力打開：高風險能力在停用／重啟之後一律
/// 要人類重新啟用（與 `mobile_disable_high_risk_receptors` 同一條不變量）。
#[tokio::test]
async fn re_enabling_a_provider_never_re_arms_its_capabilities() {
    let (_g, rt) = plain_runtime().await;
    let device = interaction_core::ProviderId::new("provider.fake.rearm");
    let receptor = interaction_core::ReceptorId::new("robot.mic-level");

    rt.registry
        .register_receptor(std::sync::Arc::new(FakeRobotMic))
        .await
        .expect("fixture receptor registers");
    let mut desc =
        interaction_registry::providers::discovered(interaction_core::ProviderIdentity {
            id: device.clone(),
            kind: interaction_core::ProviderKind::Device,
            display_name: "重啟測試機器人".into(),
            trust_level: interaction_core::TrustLevel::Untrusted,
            origin: "test".into(),
            version: String::new(),
            fingerprint: None,
            human: None,
        });
    desc.receptors = vec!["robot.mic-level".into()];
    desc.state = interaction_core::ProviderState::Installed;
    rt.providers.register(desc).await.unwrap();
    rt.registry
        .set_receptor_enabled(&receptor, true)
        .await
        .unwrap();

    rt.transition_provider(&device, interaction_core::ProviderState::Disabled)
        .await
        .unwrap();
    rt.transition_provider(&device, interaction_core::ProviderState::Available)
        .await
        .unwrap();

    assert!(
        rt.registry.receptor(&receptor).await.is_err(),
        "重新啟用 provider 不得自動把受器打開——那是人類的決定"
    );
}

/// 唯讀投影：維運看得到「核心現在相信的能力語意是什麼」。宣告表過去只存在於
/// 記憶體裡，沒有任何介面看得到——出問題時無從分辨「provider 沒宣告」還是
/// 「核心讀錯」。這個投影是純唯讀的：它不得提供任何寫入或撤回的路徑。
#[tokio::test]
async fn the_declaration_table_has_a_read_only_projection() {
    let (_g, rt) = plain_runtime().await;
    rt.declare_provider_capabilities(
        ProviderCapabilityDeclaration::new("provider.fake.robot")
            .with_class_label("測試機器人")
            .with_presentation_surface(CapabilitySelector::exact("robot.face"))
            .with_presentation_surface(CapabilitySelector::prefix("robot.screen."))
            .with_receptor("robot.mic-level")
            .with_high_risk_receptor("robot.mic-level"),
    );

    let report = rt.capability_declarations_report();
    let items = report["declarations"]
        .as_array()
        .expect("declarations 陣列");
    let robot = items
        .iter()
        .find(|d| d["id"] == json!("provider.fake.robot"))
        .expect("宣告過的 provider 要出現在投影裡");

    assert_eq!(robot["classLabel"], json!("測試機器人"));
    assert_eq!(robot["receptors"], json!(["robot.mic-level"]));
    assert_eq!(robot["highRiskReceptors"], json!(["robot.mic-level"]));
    assert_eq!(
        robot["presentationSurfaces"],
        json!([
            {"match": "exact", "value": "robot.face"},
            {"match": "prefix", "value": "robot.screen."},
        ]),
        "呈現面的比對方式要看得出來（exact 還是整族前綴）"
    );

    // 內建 provider 開機時就宣告過：投影不得只看得到測試塞進去的那一筆。
    assert!(
        items.len() >= 2,
        "內建 provider 的宣告也要在投影裡：{report}"
    );
    // 沒有 classLabel 的宣告不得憑空生一個字樣出來。
    rt.declare_provider_capabilities(ProviderCapabilityDeclaration::new("provider.fake.bare"));
    let report = rt.capability_declarations_report();
    let bare = report["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == json!("provider.fake.bare"))
        .unwrap();
    assert_eq!(bare["classLabel"], Value::Null);
    assert_eq!(bare["presentationSurfaces"], json!([]));
}
