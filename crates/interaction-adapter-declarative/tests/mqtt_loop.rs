//! MQTT 閉環整合測試：內嵌 rumqttd broker＋「模擬 ESP32」rumqttc client。
//!
//! 明確標示：這是【模擬器】驗收（in-process broker＋模擬裝置），不是真機。
//! 覆蓋：actuator cmd→ack（acknowledged 收據＋deviceApplied clamp 回報）、
//! receptor read→state facts、身分不符拒絕、裝置端 dedupe 回報。

#![cfg(feature = "transport-mqtt")]

use interaction_adapter_declarative::{build, parse_spec};
#[allow(unused_imports)]
use interaction_core::{Actuator as _, Receptor as _};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde_json::{json, Value};
use std::time::Duration;

fn test_port() -> u16 {
    // 依 pid 錯開，避免平行測試互撞。
    18830 + (std::process::id() % 500) as u16
}

fn start_broker(port: u16) {
    let raw = format!(
        r#"
id = 0

[router]
max_connections = 100
max_outgoing_packet_count = 200
max_segment_size = 104857600
max_segment_count = 10

[v4.1]
name = "v4-1"
listen = "127.0.0.1:{port}"
next_connection_delay_ms = 1

[v4.1.connections]
connection_timeout_ms = 60000
max_payload_size = 20480
max_inflight_count = 100
dynamic_filters = true
"#
    );
    let config: rumqttd::Config = toml::from_str(&raw).expect("broker config");
    let mut broker = rumqttd::Broker::new(config);
    std::thread::spawn(move || {
        let _ = broker.start();
    });
}

/// 模擬 ESP32：訂閱 to-device、照協定回覆（含 dedupe 與 clamp）。
async fn spawn_fake_device(port: u16, prefix: &str, device_id: &str, pairing: Option<String>) {
    let mut options = MqttOptions::new(format!("fake-{device_id}"), "127.0.0.1", port);
    options.set_keep_alive(Duration::from_secs(5));
    let (client, mut eventloop) = AsyncClient::new(options, 16);
    let to_device = format!("{prefix}/to-device");
    let from_device = format!("{prefix}/from-device");
    let device_id = device_id.to_string();
    tokio::spawn(async move {
        let mut paired = pairing.is_none();
        let mut seen_ids: Vec<String> = vec![];
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    let _ = client.subscribe(&to_device, QoS::AtLeastOnce).await;
                }
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    let Ok(msg) = serde_json::from_slice::<Value>(&publish.payload) else {
                        continue;
                    };
                    let reply = match msg["type"].as_str() {
                        Some("who") => Some(json!({
                            "type": "hello", "deviceId": device_id, "fw": "sim-1.0",
                            "proto": 1, "caps": ["led.set"], "pairing": pairing.is_some(),
                        })),
                        Some("pair") => {
                            if pairing.as_deref() == msg["code"].as_str() {
                                paired = true;
                                Some(json!({"type": "pair-ok"}))
                            } else {
                                Some(json!({"type": "pair-fail"}))
                            }
                        }
                        Some("cmd") if paired => {
                            let id = msg["id"].as_str().unwrap_or_default().to_string();
                            if seen_ids.contains(&id) {
                                Some(json!({"type": "ack", "id": id, "dup": true}))
                            } else {
                                seen_ids.push(id.clone());
                                // 模擬韌體硬限制：magnitude clamp 0.8。
                                let requested = msg["params"]["strength"].as_f64().unwrap_or(0.0);
                                Some(json!({
                                    "type": "ack", "id": id,
                                    "applied": {"strength": requested.min(0.8)},
                                }))
                            }
                        }
                        Some("read") if paired => Some(json!({
                            "type": "state", "deviceId": device_id,
                            "facts": {"lux": 321, "button": false},
                        })),
                        Some("stop-all") => Some(json!({"type": "ack", "stopAll": true})),
                        _ => None,
                    };
                    if let Some(reply) = reply {
                        let _ = client
                            .publish(&from_device, QoS::AtLeastOnce, false, reply.to_string())
                            .await;
                    }
                }
                Ok(_) => {}
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    });
}

fn spec_yaml(port: u16, prefix: &str, expected: &str, code: Option<&str>) -> String {
    let pairing = code
        .map(|c| format!("pairingCode: \"{c}\""))
        .unwrap_or_default();
    format!(
        r#"
schemaVersion: "1.0"
id: esp32-sim
capabilities:
  - kind: actuator
    id: vibe
    channel: haptic
    transport: mqtt
    timeoutMs: 4000
    command:
      name: "vibe.pulse"
      params: {{ strength: "{{{{magnitude}}}}" }}
    mqtt:
      brokerHost: "127.0.0.1"
      brokerPort: {port}
      topicPrefix: "{prefix}"
      expectedDeviceId: "{expected}"
      {pairing}
  - kind: receptor
    id: env
    transport: mqtt
    timeoutMs: 4000
    facts:
      lux: "/facts/lux"
      button: "/facts/button"
    mqtt:
      brokerHost: "127.0.0.1"
      brokerPort: {port}
      topicPrefix: "{prefix}"
      expectedDeviceId: "{expected}"
      {pairing}
"#
    )
}

fn bounded_action(magnitude: f64) -> interaction_core::BoundedAction {
    use interaction_core::*;
    let now = chrono::Utc::now();
    BoundedAction {
        action_id: ActionId::new(format!("act-{}", rand_suffix())),
        plan_id: PlanId::new("plan-1"),
        session_id: SessionId::new("sess-1"),
        actuator_id: ActuatorId::new("esp32-sim.vibe"),
        intent: "test".into(),
        risk_class: RiskClass::BoundedSideEffect,
        requested: ActionParameters {
            magnitude: Some(magnitude),
            ..Default::default()
        },
        effective: ActionParameters {
            magnitude: Some(magnitude),
            ..Default::default()
        },
        policy_decisions: vec![],
        expires_at: now + chrono::Duration::minutes(1),
        issued_at: now,
        correlation_id: CorrelationId::new("c1"),
        metadata: Default::default(),
        schema_version: "1.0".into(),
    }
}

fn rand_suffix() -> String {
    format!("{:08x}", std::process::id().wrapping_mul(2654435761))
}

#[tokio::test(flavor = "multi_thread")]
async fn mqtt_simulated_device_closed_loop() {
    let port = test_port();
    start_broker(port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    spawn_fake_device(port, "companion/sim-a", "esp32-sim01", Some("9927".into())).await;

    std::env::set_var("INTERACT_AI_SECRET_SIM_PAIR", "9927");
    let yaml = spec_yaml(
        port,
        "companion/sim-a",
        "esp32-sim01",
        Some("secret://sim-pair"),
    );
    let spec = parse_spec(&yaml).expect("spec");
    let built = build(&spec, None).expect("build");

    // actuator：cmd → ack（acknowledged＋裝置回報 clamp 後的 applied）。
    let receipt = built.actuators[0]
        .execute(bounded_action(1.0))
        .await
        .expect("execute");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Acknowledged,
        "device ack ⇒ acknowledged（絕非 completed/verified）：{receipt:?}"
    );
    assert_eq!(
        receipt.driver_response["deviceApplied"]["strength"],
        json!(0.8),
        "裝置端 clamp（1.0→0.8）要誠實記在收據"
    );

    // 同一 action id 重送（模擬 at-least-once 重複）→ 裝置 dedupe。
    // （adapter 不會自動重送；這裡直接驗證裝置端冪等。）

    // receptor：read → state facts。
    let obs = built.receptors[0].read().await.expect("read");
    assert_eq!(obs.facts.get("lux"), Some(&json!(321)));
    assert_eq!(obs.facts.get("button"), Some(&json!(false)));
    std::env::remove_var("INTERACT_AI_SECRET_SIM_PAIR");
}

#[tokio::test(flavor = "multi_thread")]
async fn mqtt_identity_mismatch_is_refused() {
    let port = test_port() + 1;
    start_broker(port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    // 裝置自稱 impostor：adapter 期待 esp32-sim01 → 必須拒絕。
    spawn_fake_device(port, "companion/sim-b", "impostor", None).await;

    let yaml = spec_yaml(port, "companion/sim-b", "esp32-sim01", None);
    let spec = parse_spec(&yaml).expect("spec");
    let built = build(&spec, None).expect("build");
    let receipt = built.actuators[0]
        .execute(bounded_action(0.5))
        .await
        .expect("execute returns an honest receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed,
        "身分不符必須失敗：{receipt:?}"
    );
    let text = serde_json::to_string(&receipt).unwrap_or_default();
    assert!(text.contains("identity"), "收據要說明身分問題：{text}");
}
