//! 免重啟重新綁定（rebind）在 **MQTT** 這條線上的最小閉環。
//!
//! 為什麼要單獨有這一支：`declarative_lifecycle.rs` 的 rebind 是**傳輸無關**的
//! ——它只認得型別抹除的 `DeviceAipChannel` 與 `LinkReadiness`。但在此之前
//! **每一支** rebind 測試（`declarative_session_loop.rs` 的四支）都跑同一個
//! serial pty 模擬器，所以「傳輸無關」是一句沒有證據的話：serial link 恰好
//! 滿足的假設（例如關閉是同步的、重開一個 pty 幾乎不花時間）如果偷偷寫進了
//! rebind，這裡是唯一會紅的地方。
//!
//! 明確標示：**這是【模擬器】驗收**——in-process `rumqttd` broker ＋
//! `rumqttc` 假裝置（與 `interaction-adapter-declarative/tests/mqtt_loop.rs`
//! 同一套機具）。沒有真的 ESP32、沒有真的網路 broker；本檔案不能拿來宣稱
//! 「MQTT 真板可用」。
//!
//! 覆蓋（最小）：停用 → `Unbound{Disabled}`、啟用 → 狀態誠實地先是
//! `Disconnected`、背景重新綁定收斂成 `Bound`＋`Available`，而且新的一次綁定
//! 真的重新握了手（裝置端的 `who` 計數變多，不是沿用舊授權）。

use interaction_core::{ProviderId, ProviderState};
use interaction_runtime::declarative_lifecycle::{DeclarativeLifecycle, UnboundReason};
use interaction_runtime::{Runtime, RuntimeOptions};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 依 pid 錯開埠號，避免與其他測試程序互撞（同一個檔案裡只有一支測試，
/// 所以檔案內不需要再錯開）。
fn test_port() -> u16 {
    19_450 + (std::process::id() % 400) as u16
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

/// 模擬裝置：只回答 rebind 這條路徑真正會用到的東西（`who`／`pair`／`read`／
/// `stop-all`），並數 host 握了幾次手。
async fn spawn_fake_device(port: u16, prefix: &str, device_id: &str) -> Arc<AtomicU32> {
    let mut options = MqttOptions::new(format!("fake-{device_id}"), "127.0.0.1", port);
    options.set_keep_alive(Duration::from_secs(5));
    let (client, mut eventloop) = AsyncClient::new(options, 16);
    let to_device = format!("{prefix}/to-device");
    let from_device = format!("{prefix}/from-device");
    let device_id = device_id.to_string();
    let hello_count = Arc::new(AtomicU32::new(0));
    let counter = hello_count.clone();
    tokio::spawn(async move {
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
                        Some("who") => {
                            counter.fetch_add(1, Ordering::SeqCst);
                            Some(json!({
                                "type": "hello", "deviceId": device_id, "fw": "sim-1.0",
                                "proto": 1,
                                "caps": ["sensors.read"],
                                "pairing": false,
                            }))
                        }
                        Some("read") => Some(json!({
                            "type": "state", "deviceId": device_id,
                            "facts": {"lux": 321},
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
    hello_count
}

fn spec_json(port: u16, prefix: &str, spec_id: &str, device_id: &str) -> Value {
    let mqtt = json!({
        "brokerHost": "127.0.0.1",
        "brokerPort": port,
        "topicPrefix": prefix,
        "expectedDeviceId": device_id,
    });
    json!({
        "schemaVersion": "1",
        "id": spec_id,
        "displayName": "MQTT 模擬裝置（fixture）",
        "capabilities": [
            {
                "kind": "receptor",
                "id": "env",
                "name": "環境光",
                "category": "environment",
                "transport": "mqtt",
                "mqtt": mqtt,
                // 3600 秒：這支測試不靠輪詢，不要讓背景讀取干擾時序。
                "pollIntervalMs": 3_600_000,
                "facts": {"lux": "/facts/lux"},
            },
        ],
    })
}

/// 有界輪詢：條件成立就回 true，逾時回 false（絕不無限等待）。
async fn wait_until<F, Fut>(timeout: Duration, mut probe: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if probe().await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn start_runtime(home: &tempfile::TempDir) -> Runtime {
    Runtime::start(RuntimeOptions {
        home: Some(home.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .expect("runtime")
}

/// 停用一台 **MQTT** 宣告式裝置再啟用：同一個行程裡必須重新握手、重新綁定，
/// 不需要重新啟動 daemon。
#[tokio::test(flavor = "multi_thread")]
async fn mqtt_reenable_rebinds_without_restart() {
    let port = test_port();
    start_broker(port);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let prefix = format!("companion/rebind-{}", std::process::id());
    let device_id = "esp32-sim01";
    let hellos = spawn_fake_device(port, &prefix, device_id).await;

    let spec_id = format!("mqtt-rebind-{}", std::process::id());
    let spec: interaction_adapter_declarative::DeclarativeSpec =
        serde_json::from_value(spec_json(port, &prefix, &spec_id, device_id)).expect("spec");
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&spec).await.expect("register");
    let pid = ProviderId::new(format!("provider.adapter.{spec_id}"));

    // 先真的握上手才有東西可以「重新」綁定。
    //
    // 等的是**裝置端**看到的 `who`，不是 `DeclarativeLifecycle::Bound`：後者在
    // `register_declarative_spec` 回來時就已經是 `Bound` 了（綁定 task 開出去了），
    // 拿它當「已連上」會讓這支測試在 broker 還沒接上時就通過——那正是它要抓的
    // 那種不誠實。
    assert!(
        wait_until(Duration::from_secs(30), || {
            let hellos = hellos.clone();
            async move { hellos.load(Ordering::SeqCst) >= 1 }
        })
        .await,
        "MQTT 裝置要先真的握上手（hello=0；broker/裝置沒接上）"
    );
    assert_eq!(
        rt.declarative_lifecycle(pid.as_str()),
        Some(DeclarativeLifecycle::Bound),
        "握上手之後綁定必須是 Bound"
    );
    let hellos_before = hellos.load(Ordering::SeqCst);

    // 停用：綁定拆掉，而且說得出原因。
    rt.transition_provider(&pid, ProviderState::Disabled)
        .await
        .expect("disable");
    assert_eq!(
        rt.declarative_lifecycle(pid.as_str()),
        Some(DeclarativeLifecycle::Unbound {
            reason: UnboundReason::Disabled
        }),
        "停用的原因必須說得出來，不是一個布林"
    );

    // 啟用：狀態誠實地先退到 disconnected（還沒握上手就不得說可用）。
    let back = rt
        .transition_provider(&pid, ProviderState::Available)
        .await
        .expect("re-enable");
    assert_eq!(
        back.state,
        ProviderState::Disconnected,
        "還沒握上手就不得說可用：{back:?}"
    );

    // 背景重新綁定：同一個行程裡收斂成 Bound＋Available，而且裝置端看得到
    // **新的** `who`——這是新的一次握手，不是沿用舊授權。
    assert!(
        wait_until(Duration::from_secs(45), || {
            let rt = rt.clone();
            let pid = pid.clone();
            let hellos = hellos.clone();
            async move {
                hellos.load(Ordering::SeqCst) > hellos_before
                    && rt.declarative_lifecycle(pid.as_str()) == Some(DeclarativeLifecycle::Bound)
                    && rt
                        .list_providers()
                        .await
                        .into_iter()
                        .any(|p| p.identity.id == pid && p.state == ProviderState::Available)
                    // 第 8 步的 `provider.rebound` 稽核在狀態收斂之後才落地；握手回呼
                    // 可能先一步把狀態推到 Available，所以稽核也要等到。
                    && rt
                        .store
                        .audit_tail(400)
                        .unwrap_or_default()
                        .iter()
                        .any(|r| r["kind"] == json!("provider.rebound"))
            }
        })
        .await,
        "MQTT 也要免重啟重新綁定（lifecycle={:?}，hello {hellos_before}→{}）",
        rt.declarative_lifecycle(pid.as_str()),
        hellos.load(Ordering::SeqCst)
    );

    let rebound: Vec<Value> = rt
        .store
        .audit_tail(400)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r["kind"] == json!("provider.rebound"))
        .collect();
    assert_eq!(rebound.len(), 1, "重新綁定要留稽核：{rebound:?}");
    assert_eq!(
        rebound[0]["detail"]["handshake"],
        json!("ready"),
        "{rebound:?}"
    );
}
