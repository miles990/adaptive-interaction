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
    /// 真正被套用的 cmd id（依序）——「遲到的實體效果」就是看它有沒有
    /// 出現一個 host 早已放棄的 id。
    applied_ids: Arc<Mutex<Vec<String>>>,
}

impl FakeDevice {
    fn qos_values(&self) -> Vec<u8> {
        self.qos_seen.lock().map(|q| q.clone()).unwrap_or_default()
    }

    fn applied(&self) -> Vec<String> {
        self.applied_ids
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default()
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
                                if let Ok(mut applied) = device.applied_ids.lock() {
                                    applied.push(id.clone());
                                }
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

// ---------------------------------------------------------------------------
// 可切斷的 TCP 代理（模擬器）：host → proxy → broker
//
// 為什麼需要它：要證明「斷線邊界之後 broker 不得再送出任何命令」，必須造出
// 一則**已寫上線、但永遠到不了 broker** 的 publish（inflight、沒有 PubAck），
// 然後在那一刻切線。真 broker 上這個時間窗只有微秒級，代理讓它變成確定的。
// ---------------------------------------------------------------------------

use std::sync::atomic::AtomicU64;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};

struct Proxy {
    port: u16,
    /// true＝把 host 送出的位元組讀掉但不轉給 broker
    /// （「已寫上線、永遠到不了」）。
    black_hole: Arc<std::sync::atomic::AtomicBool>,
    /// 世代 +1＝把目前這條連線切斷（模擬斷線）。
    epoch: Arc<AtomicU64>,
}

impl Proxy {
    fn black_hole(&self, on: bool) {
        self.black_hole.store(on, Ordering::SeqCst);
    }

    fn cut(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
}

async fn start_proxy(broker_port: u16) -> Proxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("proxy bind");
    let port = listener.local_addr().expect("proxy addr").port();
    let black_hole = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let epoch = Arc::new(AtomicU64::new(0));
    let (bh, ep) = (black_hole.clone(), epoch.clone());
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            let Ok(server) = TcpStream::connect(("127.0.0.1", broker_port)).await else {
                continue;
            };
            let my_epoch = ep.load(Ordering::SeqCst);
            let (client_rx, client_tx) = client.into_split();
            let (server_rx, server_tx) = server.into_split();
            tokio::spawn(pump(
                client_rx,
                server_tx,
                bh.clone(),
                ep.clone(),
                my_epoch,
                true,
            ));
            tokio::spawn(pump(
                server_rx,
                client_tx,
                bh.clone(),
                ep.clone(),
                my_epoch,
                false,
            ));
        }
    });
    Proxy {
        port,
        black_hole,
        epoch,
    }
}

async fn pump(
    mut from: OwnedReadHalf,
    mut to: OwnedWriteHalf,
    black_hole: Arc<std::sync::atomic::AtomicBool>,
    epoch: Arc<AtomicU64>,
    my_epoch: u64,
    drop_when_black: bool,
) {
    let mut buf = vec![0u8; 4096];
    loop {
        if epoch.load(Ordering::SeqCst) != my_epoch {
            return; // 這條連線被切了
        }
        match tokio::time::timeout(Duration::from_millis(30), from.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) => return,
            Ok(Ok(n)) => {
                if drop_when_black && black_hole.load(Ordering::SeqCst) {
                    continue; // 讀掉但不轉送
                }
                if to.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
            Err(_) => continue, // 只是輪詢窗到期
        }
    }
}

/// 【模擬器】斷線邊界：一則已寫上線但沒收到 PubAck 的 cmd，重連後**絕不**
/// 被補送到裝置。rumqttc 預設會把未 ack 的 QoS1 publish 搬進 pending 並在
/// 重連後優先重播——host 早已把它記成「結果未知、不重送」，裝置卻在數秒後
/// 才動作，那正是 serial 端 drain_stale_queue 明文要避免的遲到實體效果。
/// 同時驗證重連後的新命令照常可用。
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_a_reconnect_never_replays_the_inflight_command() {
    let port = test_port() + 4;
    start_broker(port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    // 裝置直連 broker；只有 host 走代理（我們只切 host 這一側）。
    let device = spawn_fake_device(port, "companion/sim-e", "esp32-sim01", None).await;
    let proxy = start_proxy(port).await;

    let yaml = spec_yaml(proxy.port, "companion/sim-e", "esp32-sim01", None);
    let built = build(&parse_spec(&yaml).expect("spec"), None).expect("build");

    // 1) 正常閉環：握手＋ack。
    let first = built.actuators[0]
        .execute(bounded_action_with_id("live-1", 0.5))
        .await
        .expect("first execute");
    assert_eq!(
        first.current_status,
        interaction_core::ActionStatus::Acknowledged
    );

    // 2) 黑洞：host 寫得出去，但一個位元組也到不了 broker。
    proxy.black_hole(true);
    let stale = built.actuators[0]
        .execute(bounded_action_with_id("stale-1", 0.5))
        .await
        .expect("honest receipt");
    assert_eq!(
        stale.current_status,
        interaction_core::ActionStatus::Dispatched,
        "已送出、沒有 ack＝結果未知：{stale:?}"
    );
    assert_eq!(stale.driver_response["outcomeUnknown"], json!(true));

    // 3) 切線 → 重連（黑洞解除）。
    proxy.cut();
    proxy.black_hole(false);
    tokio::time::sleep(Duration::from_millis(2_500)).await;

    // 4) 重連後的新命令要照常可用（重連本身沒被我們弄壞）。
    let fresh = built.actuators[0]
        .execute(bounded_action_with_id("fresh-1", 0.5))
        .await
        .expect("execute after reconnect");
    assert_eq!(
        fresh.current_status,
        interaction_core::ActionStatus::Acknowledged,
        "重連後的新命令必須照常可用：{fresh:?}"
    );

    // 5) 核心不變量：那則被放棄的命令**永遠**不得抵達裝置。
    let applied = device.applied();
    assert!(
        !applied.iter().any(|id| id == "stale-1"),
        "重連後不得補送上一代未 ack 的命令（遲到的實體效果）：{applied:?}"
    );
    assert!(
        applied.iter().any(|id| id == "fresh-1"),
        "重連後的新命令應該有送到：{applied:?}"
    );
}

/// 【模擬器】等待中的命令遇到重連：結果未知（dispatched＋outcomeUnknown），
/// **不是 failed**——命令可能已經套用，記成失敗會誘發重送＝重複實體效果。
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_a_command_waiting_across_a_reconnect_is_uncertain_not_failed() {
    let port = test_port() + 5;
    start_broker(port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _device = spawn_fake_device(port, "companion/sim-f", "esp32-sim01", None).await;
    let proxy = start_proxy(port).await;

    let yaml = spec_yaml(proxy.port, "companion/sim-f", "esp32-sim01", None);
    let built = build(&parse_spec(&yaml).expect("spec"), None).expect("build");
    built.actuators[0]
        .execute(bounded_action_with_id("warm-1", 0.5))
        .await
        .expect("handshake + first ack");

    // 命令寫得出去但到不了裝置；等待期間把連線切掉。
    proxy.black_hole(true);
    let cutter = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        proxy.cut();
        proxy.black_hole(false);
    };
    let executing = built.actuators[0].execute(bounded_action_with_id("cut-1", 0.5));
    let (receipt, ()) = tokio::join!(executing, cutter);
    let receipt = receipt.expect("honest receipt");

    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Dispatched,
        "等待中重連＝結果未知：{receipt:?}"
    );
    assert_ne!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed,
        "未知不得冒充失敗"
    );
    assert_eq!(receipt.driver_response["outcomeUnknown"], json!(true));
    assert!(
        receipt.errors.is_empty(),
        "結果未知不得寫成錯誤：{:?}",
        receipt.errors
    );
}

/// 【模擬器】裝置存活：broker 連著 ≠ ESP32 還活著。超過 livenessTimeoutMs
/// 沒聽到裝置就必須誠實降級（degraded），不得繼續宣稱「此刻真的能用它」；
/// 再次聽到裝置就恢復。
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_device_silence_degrades_health_even_while_the_broker_is_connected() {
    use interaction_core::HealthStatus;

    let port = test_port() + 6;
    start_broker(port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _device = spawn_fake_device(port, "companion/sim-g", "esp32-sim01", None).await;

    // 參考韌體每 5s 推播一次 state；這裡把窗縮到 400ms 讓測試有界。
    let yaml = format!(
        r#"
schemaVersion: "1.0"
id: esp32-live
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
      topicPrefix: "companion/sim-g"
      expectedDeviceId: "esp32-sim01"
      livenessTimeoutMs: 400
"#
    );
    let built = build(&parse_spec(&yaml).expect("spec"), None).expect("build");
    let actuator = &built.actuators[0];

    actuator
        .execute(bounded_action_with_id("live-a", 0.5))
        .await
        .expect("handshake + ack");
    let healthy = actuator.status().await;
    assert_eq!(
        healthy.status,
        HealthStatus::Healthy,
        "剛聽到裝置：healthy（{healthy:?}）"
    );

    // 裝置安靜下來（韌體會週期推播 state；沉默＝斷電／離線）。
    tokio::time::sleep(Duration::from_millis(700)).await;
    let stale = actuator.status().await;
    assert_ne!(
        stale.status,
        HealthStatus::Healthy,
        "broker 連著不等於裝置在線：{stale:?}"
    );
    let message = stale.message.clone().unwrap_or_default();
    assert!(message.contains("沒聽到裝置"), "{message}");

    // 再次聽到裝置（一次成功的命令）→ 恢復 healthy。
    actuator
        .execute(bounded_action_with_id("live-b", 0.5))
        .await
        .expect("ack");
    assert_eq!(actuator.status().await.status, HealthStatus::Healthy);
}
