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
