//! MQTT 閉環整合測試：內嵌 rumqttd broker＋「模擬 ESP32」rumqttc client。
//!
//! 明確標示：這是【模擬器】驗收（in-process broker＋模擬裝置），不是真機。
//! 覆蓋：actuator cmd→ack（acknowledged 收據＋deviceApplied clamp 回報）、
//! receptor read→state facts、身分不符拒絕、裝置端 dedupe 回報、
//! publish QoS＝AtLeastOnce、被踢掉重連後強制重新握手。

#![cfg(feature = "transport-mqtt")]

use interaction_adapter_declarative::{build, parse_spec};
#[allow(unused_imports)]
use interaction_core::{Actuator as _, Receptor as _};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
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

/// 模擬裝置的可觀察狀態（測試用；全部是模擬器端的計數，不是真機）。
#[derive(Clone, Default)]
struct FakeDevice {
    /// 收到幾次 `who`（＝host 做了幾次握手）。
    hello_count: Arc<AtomicU32>,
    /// 真正「套用」了幾次命令（dup 不算——冪等的意義就在這裡）。
    apply_count: Arc<AtomicU32>,
    /// 裝置端實際收到的 publish QoS（host 必須用 AtLeastOnce）。
    qos_seen: Arc<Mutex<Vec<u8>>>,
}

impl FakeDevice {
    fn qos_values(&self) -> Vec<u8> {
        self.qos_seen.lock().map(|q| q.clone()).unwrap_or_default()
    }
}

fn qos_code(qos: QoS) -> u8 {
    match qos {
        QoS::AtMostOnce => 0,
        QoS::AtLeastOnce => 1,
        QoS::ExactlyOnce => 2,
    }
}

/// 模擬 ESP32：訂閱 to-device、照協定回覆（含 dedupe 與 clamp）。
async fn spawn_fake_device(
    port: u16,
    prefix: &str,
    device_id: &str,
    pairing: Option<String>,
) -> FakeDevice {
    let mut options = MqttOptions::new(format!("fake-{device_id}"), "127.0.0.1", port);
    options.set_keep_alive(Duration::from_secs(5));
    let (client, mut eventloop) = AsyncClient::new(options, 16);
    let to_device = format!("{prefix}/to-device");
    let from_device = format!("{prefix}/from-device");
    let device_id = device_id.to_string();
    let observed = FakeDevice::default();
    let device = observed.clone();
    tokio::spawn(async move {
        let mut paired = pairing.is_none();
        let mut seen_ids: Vec<String> = vec![];
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    let _ = client.subscribe(&to_device, QoS::AtLeastOnce).await;
                }
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    if publish.topic == to_device {
                        if let Ok(mut qos) = device.qos_seen.lock() {
                            qos.push(qos_code(publish.qos));
                        }
                    }
                    let Ok(msg) = serde_json::from_slice::<Value>(&publish.payload) else {
                        continue;
                    };
                    let reply = match msg["type"].as_str() {
                        Some("who") => {
                            device.hello_count.fetch_add(1, Ordering::SeqCst);
                            // 每次重新握手都回到「未配對」——重連不得沿用舊授權。
                            paired = pairing.is_none();
                            Some(json!({
                                "type": "hello", "deviceId": device_id, "fw": "sim-1.0",
                                "proto": 1,
                                "caps": ["led.set", "vibe.pulse", "sensors.read"],
                                "pairing": pairing.is_some(),
                            }))
                        }
                        Some("pair") => {
                            if pairing.as_deref() == msg["code"].as_str() {
                                paired = true;
                                Some(json!({"type": "pair-ok"}))
                            } else {
                                Some(json!({"type": "pair-fail"}))
                            }
                        }
                        // 未配對就送 cmd/read：誠實拒絕（韌體同樣行為）。
                        Some("cmd") | Some("read") if !paired => Some(json!({
                            "type": "err", "id": msg["id"], "reason": "not-paired",
                        })),
                        Some("cmd") => {
                            let id = msg["id"].as_str().unwrap_or_default().to_string();
                            if seen_ids.contains(&id) {
                                // 冪等：重複的 id 只回 dup ack，不重放實體效果。
                                Some(json!({"type": "ack", "id": id, "dup": true}))
                            } else {
                                seen_ids.push(id.clone());
                                device.apply_count.fetch_add(1, Ordering::SeqCst);
                                // 模擬韌體硬限制：magnitude clamp 0.8。
                                let requested = msg["params"]["strength"].as_f64().unwrap_or(0.0);
                                Some(json!({
                                    "type": "ack", "id": id,
                                    "applied": {"strength": requested.min(0.8)},
                                }))
                            }
                        }
                        Some("read") => Some(json!({
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
    observed
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
    bounded_action_with_id(&format!("act-{}", rand_suffix()), magnitude)
}

fn bounded_action_with_id(action_id: &str, magnitude: f64) -> interaction_core::BoundedAction {
    use interaction_core::*;
    let now = chrono::Utc::now();
    BoundedAction {
        action_id: ActionId::new(action_id),
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
    let device =
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

    // receptor：read → state facts。
    let obs = built.receptors[0].read().await.expect("read");
    assert_eq!(obs.facts.get("lux"), Some(&json!(321)));
    assert_eq!(obs.facts.get("button"), Some(&json!(false)));

    // 傳輸品質：host 送出的每一則都必須是 QoS 1（at-least-once）。
    // QoS 0 會讓命令悄悄消失（誠實階梯上「送出了」就不能是謊）。
    let qos = device.qos_values();
    assert!(!qos.is_empty(), "裝置端應收到 host 的 publish");
    assert!(
        qos.iter().all(|q| *q == 1),
        "host publish 必須是 QoS AtLeastOnce，實測 {qos:?}"
    );
    std::env::remove_var("INTERACT_AI_SECRET_SIM_PAIR");
}

/// 同一個 action id 送兩次（模擬 at-least-once 造成的重複投遞）：
/// 裝置端 dedupe → 第二次是 dup ack，收據要誠實標 `deduplicated`，
/// 而且實體效果只套用一次。
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_duplicate_delivery_is_deduplicated_by_the_device() {
    let port = test_port() + 2;
    start_broker(port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let device = spawn_fake_device(port, "companion/sim-c", "esp32-sim01", None).await;

    let yaml = spec_yaml(port, "companion/sim-c", "esp32-sim01", None);
    let spec = parse_spec(&yaml).expect("spec");
    let built = build(&spec, None).expect("build");

    let action = bounded_action_with_id("dup-act-1", 1.0);
    let first = built.actuators[0]
        .execute(action.clone())
        .await
        .expect("first execute");
    assert_eq!(
        first.current_status,
        interaction_core::ActionStatus::Acknowledged
    );
    assert!(
        !first.driver_response.contains_key("deduplicated"),
        "第一次不是重複：{first:?}"
    );

    // 完全相同的 action 再送一次（協定層的重送情境）。
    let second = built.actuators[0]
        .execute(action)
        .await
        .expect("second execute");
    assert_eq!(
        second.current_status,
        interaction_core::ActionStatus::Acknowledged
    );
    assert_eq!(
        second.driver_response["deduplicated"],
        json!(true),
        "重複投遞必須誠實標成 deduplicated：{second:?}"
    );
    assert_eq!(
        device.apply_count.load(Ordering::SeqCst),
        1,
        "重複的 cmd 不得重放實體效果"
    );
}

/// 連線被踢掉（同 client id 搶佔）→ adapter 重連 → 世代 +1 →
/// 下一個命令必須重新 hello/pair 握手後才會被接受。
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_reconnect_forces_a_fresh_handshake_before_any_command() {
    let port = test_port() + 3;
    start_broker(port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let device =
        spawn_fake_device(port, "companion/sim-d", "esp32-sim01", Some("9927".into())).await;

    std::env::set_var("INTERACT_AI_SECRET_SIM_PAIR2", "9927");
    let yaml = spec_yaml(
        port,
        "companion/sim-d",
        "esp32-sim01",
        Some("secret://sim-pair2"),
    );
    let spec = parse_spec(&yaml).expect("spec");
    let built = build(&spec, None).expect("build");

    built.actuators[0]
        .execute(bounded_action_with_id("recon-act-1", 0.5))
        .await
        .expect("first execute");
    assert_eq!(device.hello_count.load(Ordering::SeqCst), 1);

    // 用同一個 client id 連上 broker：MQTT 規範要求 broker 踢掉舊 session。
    // adapter 的 spec.id 是 esp32-sim → client id interact-ai-esp32-sim。
    let mut impostor_opts = MqttOptions::new("interact-ai-esp32-sim", "127.0.0.1", port);
    impostor_opts.set_keep_alive(Duration::from_secs(5));
    let (impostor, mut impostor_loop) = AsyncClient::new(impostor_opts, 8);
    let takeover = tokio::spawn(async move {
        for _ in 0..40 {
            if impostor_loop.poll().await.is_err() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    });
    tokio::time::sleep(Duration::from_millis(800)).await;
    // 讓位：斷開搶佔者，adapter 才能重連回來。
    let _ = impostor.disconnect().await;
    takeover.abort();
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    // 重連後的第一個命令：必須先重新 hello（＋pair）才會被接受。
    let receipt = built.actuators[0]
        .execute(bounded_action_with_id("recon-act-2", 0.5))
        .await
        .expect("execute after reconnect");
    let hellos = device.hello_count.load(Ordering::SeqCst);
    assert!(
        hellos >= 2,
        "重連後必須重新握手（hello 次數 {hellos}）：{receipt:?}"
    );
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Acknowledged,
        "重新握手後命令要成功：{receipt:?}"
    );
    assert_eq!(
        device.apply_count.load(Ordering::SeqCst),
        2,
        "兩個不同 action id ⇒ 套用兩次"
    );
    std::env::remove_var("INTERACT_AI_SECRET_SIM_PAIR2");
}

#[tokio::test(flavor = "multi_thread")]
async fn mqtt_identity_mismatch_is_refused() {
    let port = test_port() + 1;
    start_broker(port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    // 裝置自稱 impostor：adapter 期待 esp32-sim01 → 必須拒絕。
    let _device = spawn_fake_device(port, "companion/sim-b", "impostor", None).await;

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
