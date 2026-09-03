//! DeviceLink 誠實核心測試（MockRawLink——不碰任何真硬體，明確是模擬）。
//!
//! 覆蓋：hello 身分驗證、配對碼、ack 對應、ack 逾時＝結果未知且不重送、
//! cancel、read state、重連（世代更替）後強制重新握手、健康度誠實
//! （斷線／未握手／已關閉不得回報 healthy）、hello.caps 能力識別、
//! shutdown 後不得再送出任何東西。

use interaction_adapter_declarative::link_caps::{LinkActuator, LinkReceptor};
use interaction_adapter_declarative::protocol::{
    encode_host_msg, DeviceLink, DeviceMsg, HostMsg, LinkError, LinkState, RawLink,
};
use interaction_adapter_declarative::{CapabilitySpec, CommandSpec};
use interaction_core::{Actuator as _, HealthStatus, Receptor as _};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

/// 可程式化的假裝置：收到 host 訊息 → 依腳本回覆。
struct MockRawLink {
    inbound: broadcast::Sender<DeviceMsg>,
    sent: Mutex<Vec<Value>>,
    device_id: String,
    pairing_code: Option<String>,
    generation: AtomicU64,
    /// 回覆 ack？（false＝模擬 ack 遺失）
    ack_replies: AtomicBool,
    /// 傳輸是否連線中（可切換：模擬拔線／重連）。
    connected: AtomicBool,
    /// 已被 shutdown()。
    closed: AtomicBool,
    /// 裝置在 hello 宣告的能力。
    caps: Mutex<Vec<String>>,
    /// hello.proto（None＝舊韌體不報版本）。
    proto: Mutex<Option<u32>>,
    /// hello.pairing：裝置自報「我需要配對碼」。
    declares_pairing: AtomicBool,
    /// 裝置端目前是否處於已配對狀態（重開機／broker 重連會被重置）。
    paired: AtomicBool,
    /// 回覆 stop-all 的 ack？（false＝裝置沒確認）
    ack_stop_all: AtomicBool,
    /// state 回覆要用的 deviceId（模擬同一 topic 上的冒名裝置）。
    state_device_id: Mutex<Option<String>>,
    /// 對 cmd/read 回一個「沒有 id」的 err（bad-json／line-too-long…）。
    err_without_id: Mutex<Option<String>>,
    /// send 在「途中」失敗（BLE write 已寫出但沒有回應）：結果未知。
    fail_mid_send: AtomicBool,
    /// send 被呼叫的次數（含失敗的）——用來證明「不重送」。
    send_attempts: AtomicU64,
}

impl MockRawLink {
    fn new(device_id: &str, pairing_code: Option<&str>) -> Arc<Self> {
        let (inbound, _) = broadcast::channel(32);
        Arc::new(Self {
            inbound,
            sent: Mutex::new(vec![]),
            device_id: device_id.into(),
            pairing_code: pairing_code.map(String::from),
            generation: AtomicU64::new(1),
            ack_replies: AtomicBool::new(true),
            connected: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            caps: Mutex::new(vec!["led.set".into(), "vibe.pulse".into()]),
            proto: Mutex::new(Some(1)),
            declares_pairing: AtomicBool::new(pairing_code.is_some()),
            paired: AtomicBool::new(false),
            ack_stop_all: AtomicBool::new(true),
            state_device_id: Mutex::new(None),
            err_without_id: Mutex::new(None),
            fail_mid_send: AtomicBool::new(false),
            send_attempts: AtomicU64::new(0),
        })
    }

    /// 裝置端配對狀態被重置（ESP32 重開機／MQTT 重連）——host 端不知情。
    fn simulate_device_pairing_reset(&self) {
        self.paired.store(false, Ordering::SeqCst);
    }

    fn set_proto(&self, proto: Option<u32>) {
        if let Ok(mut guard) = self.proto.lock() {
            *guard = proto;
        }
    }

    fn set_state_device_id(&self, id: Option<&str>) {
        if let Ok(mut guard) = self.state_device_id.lock() {
            *guard = id.map(String::from);
        }
    }

    fn set_err_without_id(&self, reason: Option<&str>) {
        if let Ok(mut guard) = self.err_without_id.lock() {
            *guard = reason.map(String::from);
        }
    }

    fn err_without_id_reason(&self) -> Option<String> {
        self.err_without_id.lock().ok().and_then(|g| g.clone())
    }

    fn sent_count(&self, msg_type: &str) -> usize {
        self.sent
            .lock()
            .map(|s| s.iter().filter(|v| v["type"] == msg_type).count())
            .unwrap_or(0)
    }

    fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// 模擬拔線／重連（重連時世代 +1，強制重新握手）。
    fn set_connected(&self, up: bool) {
        self.connected.store(up, Ordering::SeqCst);
        if up {
            self.bump_generation();
        }
    }

    fn set_caps(&self, caps: &[&str]) {
        if let Ok(mut guard) = self.caps.lock() {
            *guard = caps.iter().map(|c| c.to_string()).collect();
        }
    }

    fn caps(&self) -> Vec<String> {
        self.caps.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl RawLink for MockRawLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable("mock link closed".into()));
        }
        if !self.connected.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable("mock device unplugged".into()));
        }
        Ok(())
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        self.send_attempts.fetch_add(1, Ordering::SeqCst);
        if self.closed.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable("mock link closed".into()));
        }
        if !self.connected.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable("mock device unplugged".into()));
        }
        if self.fail_mid_send.load(Ordering::SeqCst) {
            // 位元組可能已經進了裝置：結果未知，呼叫端不得重送。
            return Err(LinkError::Uncertain("mock write failed mid-flight".into()));
        }
        let msg: Value = serde_json::from_str(&line).expect("host sends json");
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(msg.clone());
        }
        let reply = match msg["type"].as_str() {
            Some("who") => Some(DeviceMsg::Hello {
                device_id: self.device_id.clone(),
                fw: Some("1.0.0".into()),
                proto: self.proto.lock().ok().and_then(|g| *g),
                caps: self.caps(),
                pairing: self.declares_pairing.load(Ordering::SeqCst),
            }),
            Some("pair") => {
                if self.pairing_code.as_deref() == msg["code"].as_str() {
                    self.paired.store(true, Ordering::SeqCst);
                    Some(DeviceMsg::PairOk)
                } else {
                    Some(DeviceMsg::PairFail)
                }
            }
            Some("cmd") => {
                if let Some(reason) = self.err_without_id_reason() {
                    // 裝置明確拒絕，但（依韌體）沒有回 id。
                    Some(DeviceMsg::Err { id: None, reason })
                } else if self.pairing_code.is_some() && !self.paired.load(Ordering::SeqCst) {
                    // 裝置端配對狀態已重置：明確拒絕這則命令。
                    Some(DeviceMsg::Err {
                        id: msg["id"].as_str().map(String::from),
                        reason: "not-paired".into(),
                    })
                } else if self.ack_replies.load(Ordering::SeqCst) {
                    Some(DeviceMsg::Ack {
                        id: msg["id"].as_str().map(String::from),
                        applied: Some(json!({"clamped": true})),
                        dup: None,
                        cancelled: None,
                        stop_all: None,
                    })
                } else {
                    None // ack 遺失
                }
            }
            Some("cancel") => Some(DeviceMsg::Ack {
                id: msg["id"].as_str().map(String::from),
                applied: None,
                dup: None,
                cancelled: Some(true),
                stop_all: None,
            }),
            Some("read") => {
                if let Some(reason) = self.err_without_id_reason() {
                    Some(DeviceMsg::Err { id: None, reason })
                } else {
                    Some(DeviceMsg::State {
                        device_id: Some(
                            self.state_device_id
                                .lock()
                                .ok()
                                .and_then(|g| g.clone())
                                .unwrap_or_else(|| self.device_id.clone()),
                        ),
                        facts: json!({"lux": 123, "distanceMm": 456}),
                    })
                }
            }
            Some("stop-all") => {
                if self.ack_stop_all.load(Ordering::SeqCst) {
                    Some(DeviceMsg::Ack {
                        id: None,
                        applied: None,
                        dup: None,
                        cancelled: None,
                        stop_all: Some(true),
                    })
                } else {
                    None // 裝置沒有確認停止
                }
            }
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

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst) && !self.closed.load(Ordering::SeqCst)
    }

    fn link_state(&self) -> LinkState {
        if self.closed.load(Ordering::SeqCst) {
            LinkState::Closed
        } else if self.connected.load(Ordering::SeqCst) {
            LinkState::Connected
        } else {
            LinkState::Disconnected
        }
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn describe(&self) -> String {
        "mock".into()
    }
}

#[tokio::test]
async fn identity_mismatch_is_refused_before_any_command() {
    let raw = MockRawLink::new("impostor-device", None);
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), None);
    let err = link
        .command("a1", "led.set", json!({}), Duration::from_millis(500))
        .await
        .unwrap_err();
    assert!(matches!(err, LinkError::Refused(_)), "{err}");
    // 身分不符：cmd 從未送出。
    assert_eq!(raw.sent_count("cmd"), 0);
}

#[tokio::test]
async fn pairing_gate_wrong_code_refused_right_code_passes() {
    let raw = MockRawLink::new("esp32-desk01", Some("4321"));
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), Some("1111".into()));
    let err = link.ensure_ready().await.unwrap_err();
    assert!(matches!(err, LinkError::Refused(_)), "{err}");

    let raw2 = MockRawLink::new("esp32-desk01", Some("4321"));
    let link2 = DeviceLink::new(raw2.clone(), "esp32-desk01".into(), Some("4321".into()));
    link2.ensure_ready().await.unwrap();
    let ack = link2
        .command(
            "a1",
            "led.set",
            json!({"r": 255}),
            Duration::from_millis(500),
        )
        .await
        .unwrap();
    assert!(matches!(ack, DeviceMsg::Ack { .. }));
    // 握手只做一次（冪等）。
    link2.ensure_ready().await.unwrap();
    assert_eq!(raw2.sent_count("who"), 1);
    assert_eq!(raw2.sent_count("pair"), 1);
}

#[tokio::test]
async fn ack_timeout_is_unknown_and_never_resent() {
    let raw = MockRawLink::new("esp32-desk01", None);
    raw.ack_replies.store(false, Ordering::SeqCst);
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), None);
    let err = link
        .command(
            "a1",
            "vibe.pulse",
            json!({"strength": 0.5}),
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
    match err {
        LinkError::Timeout(detail) => {
            assert!(detail.contains("UNKNOWN"), "{detail}");
        }
        other => panic!("expected timeout, got {other}"),
    }
    // 絕不自動重送（實體效果不得重複觸發）。
    assert_eq!(raw.sent_count("cmd"), 1);
}

#[tokio::test]
async fn reconnect_forces_a_fresh_handshake() {
    let raw = MockRawLink::new("esp32-desk01", Some("4321"));
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), Some("4321".into()));
    link.ensure_ready().await.unwrap();
    assert_eq!(raw.sent_count("who"), 1);
    // 模擬重連（世代 +1）→ 下一次操作必須重新 hello/pair。
    raw.bump_generation();
    link.ensure_ready().await.unwrap();
    assert_eq!(raw.sent_count("who"), 2);
    assert_eq!(raw.sent_count("pair"), 2);
}

#[tokio::test]
async fn read_state_returns_pointerable_facts_and_cancel_acks() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), None);
    let state = link.read_state(Duration::from_millis(500)).await.unwrap();
    assert_eq!(state.pointer("/facts/lux"), Some(&json!(123)));
    assert_eq!(state.pointer("/facts/distanceMm"), Some(&json!(456)));

    let cancel = link.cancel("a9", Duration::from_millis(500)).await.unwrap();
    assert!(matches!(
        cancel,
        DeviceMsg::Ack {
            cancelled: Some(true),
            ..
        }
    ));
    link.stop_all(Duration::from_millis(300)).await.unwrap();
    assert_eq!(raw.sent_count("stop-all"), 1);
}

#[tokio::test]
async fn host_msgs_serialize_to_the_wire_protocol() {
    let cmd = encode_host_msg(&HostMsg::Cmd {
        id: "a1".into(),
        nonce: "deadbeef".into(),
        name: "led.set".into(),
        params: json!({"r": 255}),
    });
    let v: Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(v["type"], "cmd");
    assert_eq!(v["id"], "a1");
    assert_eq!(v["name"], "led.set");
    assert_eq!(v["params"]["r"], 255);
    let who: Value = serde_json::from_str(&encode_host_msg(&HostMsg::Who)).unwrap();
    assert_eq!(who["type"], "who");
    let stop: Value = serde_json::from_str(&encode_host_msg(&HostMsg::StopAll)).unwrap();
    assert_eq!(stop["type"], "stop-all");
}

// ---------------------------------------------------------------------------
// 健康度誠實（v0.5 對抗審查修復）：health/status 不得硬編 healthy
// ---------------------------------------------------------------------------

fn receptor_spec() -> CapabilitySpec {
    serde_json::from_value(json!({
        "kind": "receptor",
        "id": "env",
        "transport": "serial",
        "timeoutMs": 500,
        "facts": {"lux": "/facts/lux"},
    }))
    .expect("receptor spec")
}

fn actuator_spec(id: &str) -> CapabilitySpec {
    serde_json::from_value(json!({
        "kind": "actuator",
        "id": id,
        "channel": "haptic",
        "transport": "serial",
        "timeoutMs": 500,
    }))
    .expect("actuator spec")
}

fn bounded_action(action_id: &str) -> interaction_core::BoundedAction {
    use interaction_core::*;
    let now = chrono::Utc::now();
    BoundedAction {
        action_id: ActionId::new(action_id),
        plan_id: PlanId::new("plan-1"),
        session_id: SessionId::new("sess-1"),
        actuator_id: ActuatorId::new("mock.dev"),
        intent: "test".into(),
        risk_class: RiskClass::BoundedSideEffect,
        requested: ActionParameters {
            magnitude: Some(0.5),
            ..Default::default()
        },
        effective: ActionParameters {
            magnitude: Some(0.5),
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

#[tokio::test]
async fn receptor_health_follows_the_real_link_state_not_a_hard_coded_healthy() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    let receptor = LinkReceptor {
        spec: receptor_spec(),
        adapter_id: "esp32-desk".into(),
        link: link.clone(),
        transport_label: "serial",
    };

    // 連上但還沒握手：不是 healthy（也不是 offline——首次讀取才握手）。
    assert_eq!(receptor.health().await.status, HealthStatus::Degraded);

    // 讀一次＝握手完成 → healthy。
    receptor.read().await.expect("read");
    assert_eq!(receptor.health().await.status, HealthStatus::Healthy);

    // 裝置拔線：以前這裡照樣回 healthy（hard-coded success），現在必須 offline。
    raw.set_connected(false);
    let health = receptor.health().await;
    assert_eq!(health.status, HealthStatus::Offline, "{health:?}");
    assert!(
        health.message.unwrap_or_default().contains("未連線"),
        "offline 要說出原因"
    );
    // 斷線時讀取也必須誠實失敗，不得拿舊值冒充新觀察。
    assert!(receptor.read().await.is_err());

    // 重連（世代 +1）：握手作廢 → 還不能回 healthy。
    raw.set_connected(true);
    assert_eq!(receptor.health().await.status, HealthStatus::Degraded);
    // 重新握手後才恢復 healthy。
    receptor.read().await.expect("read after reconnect");
    assert_eq!(receptor.health().await.status, HealthStatus::Healthy);
    assert_eq!(raw.sent_count("who"), 2, "重連必須重新握手");
}

#[tokio::test]
async fn actuator_status_follows_the_link_and_shutdown_is_offline() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    let actuator = LinkActuator::new(
        actuator_spec("vibe"),
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link.clone(),
        "serial",
    );
    assert_eq!(actuator.status().await.status, HealthStatus::Degraded);
    actuator
        .execute(bounded_action("a1"))
        .await
        .expect("execute");
    assert_eq!(actuator.status().await.status, HealthStatus::Healthy);

    // provider disable/revoke：連線關閉 → offline，且不得再送出任何東西。
    RawLink::shutdown(&*raw);
    let health = actuator.status().await;
    assert_eq!(health.status, HealthStatus::Offline, "{health:?}");
    assert!(health.message.unwrap_or_default().contains("關閉"));
    assert!(!raw.connected(), "shutdown 後 connected() 必須為 false");
    assert!(
        RawLink::send(&*raw, "{\"type\":\"who\"}".into())
            .await
            .is_err(),
        "關閉後 send 必須回 Err"
    );
    let sent_before = raw.sent_count("cmd");
    let receipt = actuator
        .execute(bounded_action("a2"))
        .await
        .expect("receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed
    );
    assert_eq!(raw.sent_count("cmd"), sent_before, "關閉後不得再送 cmd");
}

#[tokio::test]
async fn a_capability_the_device_never_advertised_is_refused_before_the_wire() {
    let raw = MockRawLink::new("esp32-desk01", None);
    // 裝置只宣告 led.set。
    raw.set_caps(&["led.set"]);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    let actuator = LinkActuator::new(
        actuator_spec("vibe"),
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link.clone(),
        "serial",
    );
    let receipt = actuator
        .execute(bounded_action("a1"))
        .await
        .expect("honest receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed
    );
    let text = serde_json::to_string(&receipt).unwrap_or_default();
    assert!(
        text.contains("capability-not-advertised"),
        "收據要標明是能力未宣告：{text}"
    );
    // 關鍵：cmd 從未上線（沒有未知的實體效果）。
    assert_eq!(raw.sent_count("cmd"), 0);
    // 而且這個動器要誠實顯示成 offline，不是 healthy。
    let health = actuator.status().await;
    assert_eq!(health.status, HealthStatus::Offline, "{health:?}");
    assert!(health
        .message
        .unwrap_or_default()
        .contains("裝置未宣告此能力"));

    // 有宣告的能力照走：同一條連線不受影響。
    let led = LinkActuator::new(
        actuator_spec("led"),
        CommandSpec {
            name: "led.set".into(),
            params: None,
        },
        "esp32-desk".into(),
        link,
        "serial",
    );
    let ok = led.execute(bounded_action("a2")).await.expect("receipt");
    assert_eq!(
        ok.current_status,
        interaction_core::ActionStatus::Acknowledged
    );
    assert_eq!(raw.sent_count("cmd"), 1);
}

#[tokio::test]
async fn a_device_that_advertises_nothing_is_not_blocked() {
    // 舊韌體不送 caps：沒有宣告 ≠ 宣告沒有——不阻擋，只是不做能力識別。
    let raw = MockRawLink::new("esp32-desk01", None);
    raw.set_caps(&[]);
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), None);
    link.ensure_ready().await.expect("handshake");
    assert_eq!(link.advertises("vibe.pulse"), None);
    link.command("a1", "vibe.pulse", json!({}), Duration::from_millis(300))
        .await
        .expect("still allowed");
    assert_eq!(raw.sent_count("cmd"), 1);
}

#[tokio::test]
async fn task_slot_aborts_the_previous_task() {
    use interaction_adapter_declarative::protocol::TaskSlot;
    let counter = Arc::new(AtomicU64::new(0));
    let slot = TaskSlot::new();
    let first_counter = counter.clone();
    slot.replace(tokio::spawn(async move {
        loop {
            first_counter.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }));
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(slot.is_active());
    // 換新的 task：舊的必須被 abort（BLE 每次重連前要做的事）。
    slot.replace(tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
    }));
    tokio::time::sleep(Duration::from_millis(30)).await;
    let frozen = counter.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        counter.load(Ordering::SeqCst),
        frozen,
        "舊 task 必須已停（否則每次重連都留一條殭屍 task）"
    );
    slot.abort();
    assert!(!slot.is_active());
}

/// provider 被 disable／revoke 時，runtime 走的就是這個入口：連線必須真的
/// 關掉（不是只停止派工），而且關閉後健康度誠實 offline、送不出任何東西。
#[tokio::test]
async fn provider_disable_closes_the_registered_links() {
    use interaction_adapter_declarative::protocol::LinkShutdown;
    use interaction_adapter_declarative::{register_provider_links, shutdown_provider_links};

    let raw = MockRawLink::new("esp32-desk01", None);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    let receptor = LinkReceptor {
        spec: receptor_spec(),
        adapter_id: "esp32-desk".into(),
        link: link.clone(),
        transport_label: "serial",
    };
    let links: Vec<Arc<dyn LinkShutdown>> = vec![link.clone()];
    register_provider_links("provider.adapter.shutdown-test", &links);

    receptor.read().await.expect("read");
    assert_eq!(receptor.health().await.status, HealthStatus::Healthy);

    let closed = shutdown_provider_links("provider.adapter.shutdown-test");
    assert_eq!(closed, vec!["mock".to_string()], "應回報關掉了哪條連線");
    assert!(!raw.connected(), "關閉後 connected() 必須為 false");
    assert!(
        RawLink::send(&*raw, "{\"type\":\"who\"}".into())
            .await
            .is_err(),
        "關閉後不得再送出任何東西"
    );
    let health = receptor.health().await;
    assert_eq!(health.status, HealthStatus::Offline, "{health:?}");
    assert!(receptor.read().await.is_err(), "關閉後讀取必須誠實失敗");

    // 冪等：再關一次沒有東西可關（不會 panic、不會重複關閉）。
    assert!(shutdown_provider_links("provider.adapter.shutdown-test").is_empty());
}

/// 握手逾時 ≠ ack 逾時：裝置從沒回 hello 時 cmd 根本沒送出，
/// 收據不得宣稱 dispatched（那是另一種硬編的「已送出」）。
#[tokio::test]
async fn a_handshake_timeout_is_not_reported_as_dispatched() {
    struct SilentLink {
        inbound: broadcast::Sender<DeviceMsg>,
        sent: Mutex<Vec<Value>>,
    }
    #[async_trait::async_trait]
    impl RawLink for SilentLink {
        async fn ensure_open(&self) -> Result<(), LinkError> {
            Ok(())
        }
        async fn send(&self, line: String) -> Result<(), LinkError> {
            if let Ok(mut sent) = self.sent.lock() {
                sent.push(serde_json::from_str(&line).unwrap_or(Value::Null));
            }
            Ok(()) // 裝置完全不回應
        }
        fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
            self.inbound.subscribe()
        }
        fn connected(&self) -> bool {
            true
        }
        fn shutdown(&self) {}
        fn describe(&self) -> String {
            "silent".into()
        }
    }

    let (inbound, _) = broadcast::channel(8);
    let raw = Arc::new(SilentLink {
        inbound,
        sent: Mutex::new(vec![]),
    });
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    let actuator = LinkActuator::new(
        actuator_spec("vibe"),
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link,
        "serial",
    );
    let receipt = actuator
        .execute(bounded_action("a1"))
        .await
        .expect("honest receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed
    );
    let sent = raw.sent.lock().map(|s| s.clone()).unwrap_or_default();
    assert!(
        sent.iter().all(|m| m["type"] != "cmd"),
        "握手沒完成就不該有 cmd 上線：{sent:?}"
    );
    assert!(
        !receipt
            .timestamps
            .iter()
            .any(|(s, _)| *s == interaction_core::ActionStatus::Dispatched),
        "沒送出去的動作不得標成 dispatched：{receipt:?}"
    );
}

// ---------------------------------------------------------------------------
// v0.5 Phase 7 對抗審查第三輪：link 層安全底線回歸
// ---------------------------------------------------------------------------

/// 清單 9：estop 的 stop-all 必須等裝置 ack。裝置沒確認＝「已送出／未確認」，
/// 誠實回 Err——否則 runtime 的 stoppedActuators 會把沒停下來的裝置算進去。
#[tokio::test]
async fn stop_all_without_a_device_ack_is_reported_as_unconfirmed() {
    let raw = MockRawLink::new("esp32-desk01", None);
    raw.ack_stop_all.store(false, Ordering::SeqCst);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    let err = link
        .stop_all(Duration::from_millis(300))
        .await
        .expect_err("no ack must not be reported as success");
    match &err {
        LinkError::Timeout(detail) => assert!(detail.contains("no ack"), "{detail}"),
        other => panic!("expected an unconfirmed stop-all, got {other}"),
    }
    // stop-all 還是有送出去（誠實：已送出、未確認）。
    assert_eq!(raw.sent_count("stop-all"), 1);

    // 動器層：estop 回 Err → runtime 不得把它算成已停止。
    let actuator = LinkActuator::new(
        actuator_spec("vibe"),
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link.clone(),
        "serial",
    );
    let stop = actuator.emergency_stop().await;
    assert!(stop.is_err(), "未確認的 stop-all 不得回 Ok");

    // 裝置會 ack 時才算真的停下來。
    raw.ack_stop_all.store(true, Ordering::SeqCst);
    actuator
        .emergency_stop()
        .await
        .expect("acked stop-all is a real stop");
}

/// 清單 10：topic／埠不是身分——state.deviceId 不符的訊息不得被當成
/// 這台裝置的觀察（同一個 MQTT topic 上任何人都能發 state）。
#[tokio::test]
async fn state_from_a_foreign_device_id_is_discarded() {
    let raw = MockRawLink::new("esp32-desk01", None);
    raw.set_state_device_id(Some("evil-device"));
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    let err = link
        .read_state(Duration::from_millis(300))
        .await
        .expect_err("foreign state must not be accepted");
    assert!(matches!(err, LinkError::Timeout(_)), "{err}");

    // 匿名 state（沒有 deviceId）同樣不算數。
    raw.set_state_device_id(None);
    let receptor = LinkReceptor {
        spec: receptor_spec(),
        adapter_id: "esp32-desk".into(),
        link: link.clone(),
        transport_label: "serial",
    };
    let obs = receptor.read().await.expect("own deviceId is accepted");
    assert_eq!(obs.facts.get("lux"), Some(&json!(123)));
}

/// 清單 11：等待中的 cmd 遇到重連（世代改變）→ 立刻以 link-reset 收場，
/// 而且**不重送**——重連後的連線還沒重新握手，舊命令不得原樣送出。
#[tokio::test]
async fn a_reconnect_ends_the_waiting_command_as_link_reset() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    link.ensure_ready().await.expect("handshake");
    raw.ack_replies.store(false, Ordering::SeqCst); // 裝置不回 ack

    let bumper = raw.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        bumper.bump_generation(); // 重連
    });
    let err = link
        .command("a1", "vibe.pulse", json!({}), Duration::from_secs(5))
        .await
        .expect_err("a reconnect must end the wait");
    assert!(matches!(err, LinkError::Reset(_)), "{err}");
    assert_eq!(raw.sent_count("cmd"), 1, "絕不重送實體命令");

    // 動器收據：failed(link-reset)＋結果未知，不是 acknowledged。
    let actuator = LinkActuator::new(
        actuator_spec("vibe"),
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link,
        "serial",
    );
    let bumper = raw.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        bumper.bump_generation();
    });
    let receipt = actuator
        .execute(bounded_action("a2"))
        .await
        .expect("honest receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed
    );
    assert_eq!(receipt.driver_response["outcomeUnknown"], json!(true));
    let text = serde_json::to_string(&receipt).unwrap_or_default();
    assert!(text.contains("link-reset"), "{text}");
}

/// 清單 11（下半）：每則 cmd 帶 deadline——過期的命令一律不寫上線
/// （斷線期間排隊、重連後才送達的實體效果比誠實失敗更糟）。
#[tokio::test]
async fn an_expired_command_is_never_written_to_the_wire() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let before = raw.sent_count("cmd");
    let err = RawLink::send_before(
        &*raw,
        json!({"type":"cmd","id":"a1"}).to_string(),
        std::time::Instant::now(),
    )
    .await
    .expect_err("an expired message must not be sent");
    assert!(matches!(err, LinkError::Unavailable(_)), "{err}");
    assert_eq!(raw.sent_count("cmd"), before, "過期命令不得上線");
}

/// 清單 12：hello 的自報必須被當真——裝置說「我需要配對碼」但 spec 沒有，
/// 或協定版本不同，握手都要誠實失敗（而不是照送 cmd 換一串 not-paired）。
#[tokio::test]
async fn hello_pairing_and_proto_declarations_are_honoured() {
    // 裝置要配對、spec 沒有 pairingCode → 握手失敗，cmd 從未送出。
    let raw = MockRawLink::new("esp32-desk01", None);
    raw.declares_pairing.store(true, Ordering::SeqCst);
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), None);
    let err = link.ensure_ready().await.expect_err("pairing is required");
    match &err {
        LinkError::Refused(detail) => assert!(detail.contains("pairing code"), "{detail}"),
        other => panic!("expected Refused, got {other}"),
    }
    assert_eq!(raw.sent_count("cmd"), 0);

    // 協定版本不同 → 拒絕（不猜對方的訊息語意）。
    let raw2 = MockRawLink::new("esp32-desk01", None);
    raw2.set_proto(Some(2));
    let link2 = DeviceLink::new(raw2.clone(), "esp32-desk01".into(), None);
    let err = link2.ensure_ready().await.expect_err("proto mismatch");
    match &err {
        LinkError::Refused(detail) => assert!(detail.contains("protocol v2"), "{detail}"),
        other => panic!("expected Refused, got {other}"),
    }
    assert_eq!(raw2.sent_count("cmd"), 0);

    // 舊韌體不報 proto：不阻擋（沒有宣告 ≠ 宣告不同版本）。
    let raw3 = MockRawLink::new("esp32-desk01", None);
    raw3.set_proto(None);
    let link3 = DeviceLink::new(raw3.clone(), "esp32-desk01".into(), None);
    link3
        .ensure_ready()
        .await
        .expect("no proto is not a refusal");
}

/// 清單 13：裝置明確拒絕（沒帶 id 的 err）不得被演成「逾時、結果未知」。
#[tokio::test]
async fn an_id_less_device_error_ends_the_request_as_a_refusal() {
    let raw = MockRawLink::new("esp32-desk01", None);
    raw.set_err_without_id(Some("line-too-long"));
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));

    let reply = link
        .command("a1", "vibe.pulse", json!({}), Duration::from_secs(3))
        .await
        .expect("an explicit refusal is a reply, not a timeout");
    match reply {
        DeviceMsg::Err { reason, .. } => assert_eq!(reason, "line-too-long"),
        other => panic!("expected err, got {other:?}"),
    }

    // read 也一樣：拒絕就是拒絕，不是逾時。
    let err = link
        .read_state(Duration::from_millis(500))
        .await
        .expect_err("device refused the read");
    match &err {
        LinkError::Refused(detail) => assert!(detail.contains("line-too-long"), "{detail}"),
        other => panic!("expected Refused, got {other}"),
    }

    // 動器收據：failed（device-refused），不是 ackTimeout。
    let actuator = LinkActuator::new(
        actuator_spec("vibe"),
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link,
        "serial",
    );
    let receipt = actuator
        .execute(bounded_action("a2"))
        .await
        .expect("receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed
    );
    assert!(!receipt.driver_response.contains_key("ackTimeout"));
}

/// 清單 14：裝置端配對狀態被重置（ESP32 重開機／MQTT 重連）後，
/// 這一次的 cmd 誠實失敗（不自動重送實體命令），下一個請求前重新握手。
#[tokio::test]
async fn a_device_pairing_reset_re_handshakes_before_the_next_command() {
    let raw = MockRawLink::new("esp32-desk01", Some("4321"));
    let link = Arc::new(DeviceLink::new(
        raw.clone(),
        "esp32-desk01".into(),
        Some("4321".into()),
    ));
    let actuator = LinkActuator::new(
        actuator_spec("vibe"),
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link.clone(),
        "serial",
    );
    let ok = actuator
        .execute(bounded_action("a1"))
        .await
        .expect("receipt");
    assert_eq!(
        ok.current_status,
        interaction_core::ActionStatus::Acknowledged
    );
    assert_eq!(raw.sent_count("who"), 1);
    assert_eq!(raw.sent_count("pair"), 1);

    // 裝置重開機：配對狀態沒了，host 端不知情。
    raw.simulate_device_pairing_reset();
    let refused = actuator
        .execute(bounded_action("a2"))
        .await
        .expect("receipt");
    assert_eq!(
        refused.current_status,
        interaction_core::ActionStatus::Failed,
        "not-paired 是明確失敗，不是逾時"
    );
    let text = serde_json::to_string(&refused).unwrap_or_default();
    assert!(text.contains("not-paired"), "{text}");
    assert_eq!(raw.sent_count("cmd"), 2, "失敗的實體命令絕不自動重送");
    // 這一輪還沒重新握手（先誠實失敗）。
    assert_eq!(raw.sent_count("who"), 1);

    // 下一個請求：重新 hello/pair，然後才送 cmd。
    let recovered = actuator
        .execute(bounded_action("a3"))
        .await
        .expect("receipt");
    assert_eq!(
        recovered.current_status,
        interaction_core::ActionStatus::Acknowledged
    );
    assert_eq!(raw.sent_count("who"), 2, "重置後必須重新握手");
    assert_eq!(raw.sent_count("pair"), 2);
}

/// 清單 15：send「途中」失敗（BLE write 可能已送達）＝結果未知：
/// 不重試（重試會重複實體效果）、也不冒充失敗成功。
#[tokio::test]
async fn a_mid_send_failure_is_uncertain_and_never_retried() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    link.ensure_ready().await.expect("handshake");

    let mut spec = actuator_spec("vibe");
    spec.retry = Some(interaction_adapter_declarative::RetrySpec {
        attempts: 3,
        backoff_ms: 0,
    });
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
    raw.fail_mid_send.store(true, Ordering::SeqCst);
    let before = raw.send_attempts.load(Ordering::SeqCst);
    let receipt = actuator
        .execute(bounded_action("a1"))
        .await
        .expect("honest receipt");
    assert_eq!(
        raw.send_attempts.load(Ordering::SeqCst) - before,
        1,
        "送出途中失敗＝結果未知，不得重送實體命令"
    );
    assert_eq!(receipt.driver_response["sendOutcomeUnknown"], json!(true));
    assert_ne!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed,
        "結果未知不得冒充失敗（watchdog 會標成 uncertain）"
    );
    assert_ne!(
        receipt.current_status,
        interaction_core::ActionStatus::Acknowledged
    );
}

// ---------------------------------------------------------------------------
// 對抗審查 2e02284：estop 的 ack 窗口必須蓋過韌體的重連阻塞
// ---------------------------------------------------------------------------

/// stop-all 的 ack 會晚到的假裝置：韌體在 broker 不通時 `maintainMqtt()` 的
/// `connect()` 同步阻塞最多 ≈1.5s，期間 Serial／BLE 上的 stop-all 要等阻塞
/// 結束才被處理——所以 ack 可能 1.2s 之後才回來。
struct DelayedStopAllLink {
    inbound: broadcast::Sender<DeviceMsg>,
    delay: Duration,
    sent: Mutex<Vec<Value>>,
}

#[async_trait::async_trait]
impl RawLink for DelayedStopAllLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        Ok(())
    }
    async fn send(&self, line: String) -> Result<(), LinkError> {
        let msg: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(msg.clone());
        }
        if msg["type"] == "stop-all" {
            let inbound = self.inbound.clone();
            let delay = self.delay;
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                let _ = inbound.send(DeviceMsg::Ack {
                    id: None,
                    applied: None,
                    dup: None,
                    cancelled: None,
                    stop_all: Some(true),
                });
            });
        }
        Ok(())
    }
    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        self.inbound.subscribe()
    }
    fn connected(&self) -> bool {
        true
    }
    fn shutdown(&self) {}
    fn describe(&self) -> String {
        "delayed-stop-all".into()
    }
}

fn delayed_stop_all_actuator(delay: Duration) -> LinkActuator<Arc<DelayedStopAllLink>> {
    let (inbound, _) = broadcast::channel(8);
    let raw = Arc::new(DelayedStopAllLink {
        inbound,
        delay,
        sent: Mutex::new(vec![]),
    });
    let link = Arc::new(DeviceLink::new(raw, "esp32-desk01".into(), None));
    LinkActuator::new(
        actuator_spec("vibe"),
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link,
        "serial",
    )
}

/// 舊版只等 1s：真板 Wi-Fi 通、broker 不通時，stop-all 的 ack 撞上一次重連
/// 阻塞（≈1.5s）就晚於 1s 到達 → estop 被誤記成 UNCONFIRMED（假陰性，且發生
/// 在最不該有雜訊的路徑）。現在的窗口 = runtime 每個 actuator 的 estop 上限
/// （2s）：1.2s 才到的 ack 仍算已確認停止。
#[tokio::test]
async fn a_stop_all_ack_delayed_by_a_firmware_reconnect_block_still_counts() {
    let actuator = delayed_stop_all_actuator(Duration::from_millis(1_200));
    actuator
        .emergency_stop()
        .await
        .expect("an ack that arrives after a ≈1.2s firmware block is still an ack");
    assert_eq!(
        actuator
            .link
            .raw()
            .sent
            .lock()
            .map(|s| s.len())
            .unwrap_or(0),
        1
    );
}

/// 窗口外才到的 ack（2.5s）仍然誠實 Err：estop 不會為了等而無限等，
/// runtime 的 stoppedActuators 不得把它算成已停止；而且 stop-all 只送一次。
#[tokio::test]
async fn a_stop_all_ack_after_the_window_is_still_unconfirmed() {
    let actuator = delayed_stop_all_actuator(Duration::from_millis(2_500));
    let err = actuator
        .emergency_stop()
        .await
        .expect_err("an ack that never arrives within the window must not count");
    let text = err.to_string();
    assert!(text.contains("UNCONFIRMED"), "{text}");
    assert_eq!(
        actuator
            .link
            .raw()
            .sent
            .lock()
            .map(|s| s.len())
            .unwrap_or(0),
        1,
        "stop-all is sent exactly once (no resend storm)"
    );
}
