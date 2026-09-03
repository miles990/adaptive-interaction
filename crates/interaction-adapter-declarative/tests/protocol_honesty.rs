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
    /// 裝置端配對鎖定中（連續錯碼）：任何 pair 都回 pair-locked，不比對碼。
    pair_locked: AtomicBool,
    /// read 是否回覆 state？（false＝讓 read 卡在等待，測並行歸屬）
    state_replies: AtomicBool,
    /// 回覆 stop-all 的 ack？（false＝裝置沒確認）
    ack_stop_all: AtomicBool,
    /// 回覆 cancel 的 ack？（false＝讓 cancel 停在等待中，測並行歸屬）
    ack_cancel: AtomicBool,
    /// 對 cancel 回一則 err：(是否帶 id, reason)。`false` 的匿名 err 對應
    /// 韌體 BLE 入站佇列滿時的 `err busy`（那則 cancel 在解析前就被丟掉）。
    cancel_err: Mutex<Option<(bool, String)>>,
    /// state 回覆要用的 deviceId（模擬同一 topic 上的冒名裝置）。
    state_device_id: Mutex<Option<String>>,
    /// 對 cmd/read 回一個「沒有 id」的 err（bad-json／line-too-long…）。
    err_without_id: Mutex<Option<String>>,
    /// send 在「途中」失敗（BLE write 已寫出但沒有回應）：結果未知。
    fail_mid_send: AtomicBool,
    /// send 被呼叫的次數（含失敗的）——用來證明「不重送」。
    send_attempts: AtomicU64,
    /// ensure_open 被呼叫的次數——用來證明 estop 不會對未連線的 link
    /// 重新開埠／掃描（真傳輸那是 2 秒輪詢或 6 秒 BLE 掃描）。
    ensure_open_calls: AtomicU64,
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
            pair_locked: AtomicBool::new(false),
            state_replies: AtomicBool::new(true),
            ack_stop_all: AtomicBool::new(true),
            ack_cancel: AtomicBool::new(true),
            cancel_err: Mutex::new(None),
            state_device_id: Mutex::new(None),
            err_without_id: Mutex::new(None),
            fail_mid_send: AtomicBool::new(false),
            send_attempts: AtomicU64::new(0),
            ensure_open_calls: AtomicU64::new(0),
        })
    }

    /// 裝置吐一則「沒有 id」的錯誤（bad-json／unknown-type／BLE busy）——
    /// 它不屬於任何特定請求。
    fn emit_id_less_error(&self, reason: &str) {
        let _ = self.inbound.send(DeviceMsg::Err {
            id: None,
            reason: reason.into(),
        });
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

    /// 裝置對 cancel 回 err（而不是 ack）：韌體只有 `not-found` 代表
    /// 「確定沒有這個 id 在跑」，其餘（busy／not-paired／rate-limited）
    /// 代表這則 cancel 根本沒被處理。
    fn set_cancel_err(&self, err: Option<(bool, &str)>) {
        if let Ok(mut guard) = self.cancel_err.lock() {
            *guard = err.map(|(with_id, reason)| (with_id, reason.to_string()));
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
        self.ensure_open_calls.fetch_add(1, Ordering::SeqCst);
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
                pairing_locked: self.pair_locked.load(Ordering::SeqCst),
            }),
            Some("pair") => {
                if self.pair_locked.load(Ordering::SeqCst) {
                    // 鎖定期：不比對碼（正確的碼也一樣），誠實回 pair-locked。
                    Some(DeviceMsg::PairFail {
                        reason: Some("pair-locked".into()),
                        retry_after_ms: Some(30_000),
                    })
                } else if self.pairing_code.is_none() {
                    // 裝置端配對停用（PAIRING_CODE 為空）：**不比對**任何碼，
                    // 一律 pair-ok——與參考韌體 handlePair() 相同。
                    self.paired.store(true, Ordering::SeqCst);
                    Some(DeviceMsg::PairOk)
                } else if self.pairing_code.as_deref() == msg["code"].as_str() {
                    self.paired.store(true, Ordering::SeqCst);
                    Some(DeviceMsg::PairOk)
                } else {
                    Some(DeviceMsg::PairFail {
                        reason: None,
                        retry_after_ms: None,
                    })
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
            Some("cancel") => {
                let cancel_err = self.cancel_err.lock().ok().and_then(|g| g.clone());
                if let Some((with_id, reason)) = cancel_err {
                    Some(DeviceMsg::Err {
                        id: with_id.then(|| msg["id"].as_str().unwrap_or_default().to_string()),
                        reason,
                    })
                } else if self.ack_cancel.load(Ordering::SeqCst) {
                    Some(DeviceMsg::Ack {
                        id: msg["id"].as_str().map(String::from),
                        applied: None,
                        dup: None,
                        cancelled: Some(true),
                        stop_all: None,
                    })
                } else {
                    None // cancel ack 遺失：停在等待中
                }
            }
            Some("read") => {
                if let Some(reason) = self.err_without_id_reason() {
                    Some(DeviceMsg::Err { id: None, reason })
                } else if !self.state_replies.load(Ordering::SeqCst) {
                    None // state 遺失：讓 read 停在等待中
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

    // 動器收據：dispatched＋outcomeUnknown（結果未知），不是 acknowledged，
    // **也不是 failed**——命令可能已經套用；記成失敗會誘發重送＝重複實體效果。
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
        interaction_core::ActionStatus::Dispatched,
        "等待中重連＝結果未知：收據停在 dispatched，由 runtime 判成 uncertain：{receipt:?}"
    );
    assert_ne!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed,
        "未知不得冒充失敗（失敗會讓人／AI 合理地重下同一命令）"
    );
    assert_eq!(receipt.driver_response["outcomeUnknown"], json!(true));
    assert!(
        receipt.errors.is_empty(),
        "結果未知不得寫成錯誤：{:?}",
        receipt.errors
    );
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

/// estop：未連線的 link 不得再去「開埠／掃描」。真傳輸的 ensure_open 在
/// serial/mqtt 是 2 秒輪詢、BLE 是最長 6 秒掃描；一台拔線的裝置上掛 4 個
/// 動器就會把其他裝置的 stop-all 卡在後面。沒連上就立刻誠實回報
/// 「沒送出、裝置狀態未知」。
#[tokio::test]
async fn stop_all_on_a_disconnected_link_fails_fast_without_reopening() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), None);
    link.ensure_ready().await.expect("handshake");
    raw.set_connected(false); // 拔線
    let before = raw.ensure_open_calls.load(Ordering::SeqCst);

    let started = std::time::Instant::now();
    let err = link
        .stop_all(Duration::from_secs(2))
        .await
        .expect_err("未連線的 link 沒有東西可停");
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "estop 不得在未連線的 link 上空等：{:?}",
        started.elapsed()
    );
    assert_eq!(
        raw.ensure_open_calls.load(Ordering::SeqCst),
        before,
        "未連線就不該再嘗試開埠／掃描"
    );
    match err {
        LinkError::Unavailable(detail) => {
            assert!(detail.contains("not connected"), "{detail}");
            assert!(detail.contains("NOT sent"), "{detail}");
        }
        other => panic!("expected an honest Unavailable, got {other:?}"),
    }
    assert_eq!(raw.sent_count("stop-all"), 0, "確定沒送出");
}

/// 連線中的 link：stop-all 照送、照等 ack（fast-fail 只針對未連線）。
#[tokio::test]
async fn stop_all_still_reaches_a_connected_link() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), None);
    link.stop_all(Duration::from_millis(500))
        .await
        .expect("connected link acks stop-all");
    assert_eq!(raw.sent_count("stop-all"), 1);
}

/// 同一台裝置上有平行步驟時，裝置吐的「沒有 id」錯誤不得被任一等待者
/// 認領成自己的結果：那則錯誤可能屬於另一個請求，而我這個命令**可能已經
/// 被套用**。認領＝把已套用的命令記成 device-refused → 人或 AI 重送 →
/// 重複實體效果。無法歸屬時誠實記成「結果未知」。
#[tokio::test(flavor = "multi_thread")]
async fn an_id_less_error_is_not_attributed_while_another_request_is_in_flight() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    link.ensure_ready().await.expect("handshake");
    raw.ack_replies.store(false, Ordering::SeqCst); // 兩個命令都收不到 ack

    let spawn_cmd = |id: &'static str| {
        let link = link.clone();
        tokio::spawn(async move {
            link.command(id, "vibe.pulse", json!({}), Duration::from_millis(700))
                .await
        })
    };
    let first = spawn_cmd("a1");
    let second = spawn_cmd("a2");
    // 兩個都在等待中了，裝置才吐出一則匿名錯誤。
    tokio::time::sleep(Duration::from_millis(200)).await;
    raw.emit_id_less_error("bad-json");

    for handle in [first, second] {
        match handle.await.expect("task") {
            Err(LinkError::Timeout(detail)) => {
                assert!(detail.contains("UNKNOWN"), "{detail}");
                assert!(
                    detail.contains("cannot be attributed"),
                    "要說清楚為什麼是未知：{detail}"
                );
            }
            other => panic!("匿名錯誤不得被任一等待者認領：{other:?}"),
        }
    }
    assert_eq!(raw.sent_count("cmd"), 2, "各送一次，絕不重送");
}

// ---------------------------------------------------------------------------
// link-transports-048：握手期間重連 → 舊世代的握手不得沿用到新連線上
// ---------------------------------------------------------------------------

/// 一條「在回覆 who／pair 之前就重連過」的假連線：世代先 +1，才把
/// hello／pair-ok 推進同一條 broadcast channel——模擬真板重連後主動重送
/// hello（韌體每次連上就送）而 host 還等在舊世代的 wait 上。
struct HandshakeRaceLink {
    inbound: broadcast::Sender<DeviceMsg>,
    sent: Mutex<Vec<Value>>,
    generation: AtomicU64,
    /// 在哪一種 host 訊息之後推進世代（"who" 或 "pair"）。
    bump_after: &'static str,
    device_id: String,
}

impl HandshakeRaceLink {
    fn new(bump_after: &'static str) -> Arc<Self> {
        let (inbound, _) = broadcast::channel(32);
        Arc::new(Self {
            inbound,
            sent: Mutex::new(vec![]),
            generation: AtomicU64::new(1),
            bump_after,
            device_id: "esp32-desk01".into(),
        })
    }

    fn sent_count(&self, msg_type: &str) -> usize {
        self.sent
            .lock()
            .map(|s| s.iter().filter(|v| v["type"] == msg_type).count())
            .unwrap_or(0)
    }
}

#[async_trait::async_trait]
impl RawLink for HandshakeRaceLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        Ok(())
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        let msg: Value = serde_json::from_str(&line).expect("host sends json");
        let kind = msg["type"].as_str().unwrap_or_default().to_string();
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(msg);
        }
        // 重連發生在「裝置回覆之前」：世代先 +1，回覆才進來。
        if kind == self.bump_after {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        let reply = match kind.as_str() {
            "who" => Some(DeviceMsg::Hello {
                device_id: self.device_id.clone(),
                fw: Some("1.0.0".into()),
                proto: Some(1),
                caps: vec!["led.set".into()],
                pairing: true,
                pairing_locked: false,
            }),
            "pair" => Some(DeviceMsg::PairOk),
            "cmd" => Some(DeviceMsg::Ack {
                id: None,
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

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn connected(&self) -> bool {
        true
    }

    fn shutdown(&self) {}

    fn describe(&self) -> String {
        "handshake-race mock".into()
    }
}

/// 清單：hello 等待期間重連 → ensure_ready 必須誠實回 Reset，
/// 且後續 command() 一個 byte 都不得寫上線（那條連線的身分從未在這個
/// 世代被比對過）。舊版會用舊世代標記「已握手」並照樣送 cmd。
#[tokio::test]
async fn a_reconnect_during_the_hello_wait_invalidates_the_handshake() {
    let raw = HandshakeRaceLink::new("who");
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), Some("4321".into()));

    match link.ensure_ready().await {
        Err(LinkError::Reset(detail)) => {
            assert!(detail.contains("reconnected"), "{detail}");
            assert!(detail.contains("hello"), "要說清楚是哪一段被打斷：{detail}");
        }
        other => panic!("握手期間重連必須作廢握手，得到 {other:?}"),
    }
    // 握手沒完成：readiness 誠實回「尚未握手」。
    assert_eq!(
        link.readiness(),
        interaction_adapter_declarative::protocol::LinkReadiness::NotHandshaken
    );
    // 而且 cmd 從未寫上線（不是「送出了、結果未知」）。
    let err = link
        .command("a1", "led.set", json!({}), Duration::from_millis(300))
        .await
        .expect_err("no cmd may be sent on a connection whose identity was never verified");
    match &err {
        LinkError::Unavailable(detail) => assert!(detail.contains("no cmd was sent"), "{detail}"),
        other => panic!("expected Unavailable (definitely not sent), got {other}"),
    }
    assert_eq!(raw.sent_count("cmd"), 0, "cmd 一個 byte 都不得寫上線");
}

/// 同一條規則要覆蓋 pair 那一段的等待（不是只有 hello）。
#[tokio::test]
async fn a_reconnect_during_the_pairing_wait_invalidates_the_handshake() {
    let raw = HandshakeRaceLink::new("pair");
    let link = DeviceLink::new(raw.clone(), "esp32-desk01".into(), Some("4321".into()));
    match link.ensure_ready().await {
        Err(LinkError::Reset(detail)) => assert!(detail.contains("pairing"), "{detail}"),
        other => panic!("配對等待期間重連必須作廢握手，得到 {other:?}"),
    }
    assert_eq!(raw.sent_count("cmd"), 0);
}

// ---------------------------------------------------------------------------
// protocol-conformance-029 / link-transports-053：in_flight 必須涵蓋每一種請求
// ---------------------------------------------------------------------------

/// 同一條 link 上受器輪詢的 read 與動器的 cmd 天然並行（一個 adapter 的所有
/// capability 共用同一個 DeviceLink）。裝置對 read 回一則**沒有 id** 的錯誤
/// 時，並行的 cmd 不得認領它——認領＝把一個**已經套用**的命令記成
/// device-refused，人或 AI 就會重下同一命令＝重複實體效果。
#[tokio::test(flavor = "multi_thread")]
async fn a_concurrent_read_makes_an_id_less_error_unattributable_to_the_cmd() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    link.ensure_ready().await.expect("handshake");
    raw.ack_replies.store(false, Ordering::SeqCst); // cmd 收不到 ack
    raw.state_replies.store(false, Ordering::SeqCst); // read 也停在等待

    let cmd = {
        let link = link.clone();
        tokio::spawn(async move {
            link.command("a1", "vibe.pulse", json!({}), Duration::from_millis(700))
                .await
        })
    };
    let read = {
        let link = link.clone();
        tokio::spawn(async move { link.read_state(Duration::from_millis(700)).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    raw.emit_id_less_error("bad-json");

    match cmd.await.expect("cmd task") {
        Err(LinkError::Timeout(detail)) => {
            assert!(detail.contains("UNKNOWN"), "{detail}");
            assert!(detail.contains("cannot be attributed"), "{detail}");
        }
        other => panic!("並行 read 在飛時，cmd 不得認領匿名錯誤：{other:?}"),
    }
    // 對稱地：read 也不得把它當成「裝置拒絕了這次讀取」。
    match read.await.expect("read task") {
        Err(LinkError::Timeout(detail)) => {
            assert!(detail.contains("cannot be attributed"), "{detail}")
        }
        other => panic!("並行 cmd 在飛時，read 不得認領匿名錯誤：{other:?}"),
    }
    assert_eq!(raw.sent_count("cmd"), 1, "絕不重送");
}

/// 反向：cancel 在有並行 cmd 時，也不得把匿名錯誤翻成「沒有可取消的效果」
/// ——那是在安全路徑上給出確定卻錯誤的結論（震動／蜂鳴可能還在跑）。
#[tokio::test(flavor = "multi_thread")]
async fn a_concurrent_cmd_makes_an_id_less_error_unattributable_to_the_cancel() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    link.ensure_ready().await.expect("handshake");
    raw.ack_replies.store(false, Ordering::SeqCst); // cmd 收不到 ack
    raw.ack_cancel.store(false, Ordering::SeqCst); // cancel 也停在等待

    let cmd = {
        let link = link.clone();
        tokio::spawn(async move {
            link.command("a2", "vibe.pulse", json!({}), Duration::from_millis(700))
                .await
        })
    };
    let cancel = {
        let link = link.clone();
        tokio::spawn(async move { link.cancel("a2", Duration::from_millis(700)).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    raw.emit_id_less_error("not-paired");

    match cancel.await.expect("cancel task") {
        Err(LinkError::Timeout(detail)) => {
            assert!(detail.contains("cannot be attributed"), "{detail}");
            assert!(
                detail.contains("UNKNOWN"),
                "取消結果未知就要說未知，不能說「沒有可取消的效果」：{detail}"
            );
        }
        other => panic!("cancel 不得認領無法歸屬的匿名錯誤：{other:?}"),
    }
    match cmd.await.expect("cmd task") {
        Err(LinkError::Timeout(detail)) => assert!(detail.contains("UNKNOWN"), "{detail}"),
        other => panic!("cmd 應停在 UNKNOWN：{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// link-transports-050：一個 fact 都沒解出來不是一次成功的觀察
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_state_that_resolves_no_declared_fact_is_not_an_observation() {
    let raw = MockRawLink::new("esp32-desk01", None);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-desk01".into(), None));
    let mut spec = receptor_spec();
    // 裝置回的是 lux/distanceMm；spec 指向改過名的欄位。
    spec.facts = std::collections::BTreeMap::from([
        ("lumens".to_string(), "/facts/renamedLux".to_string()),
        ("range".to_string(), "/facts/renamedDistance".to_string()),
    ]);
    let receptor = LinkReceptor {
        spec,
        adapter_id: "esp32-desk".into(),
        link,
        transport_label: "serial",
    };
    let err = receptor
        .read()
        .await
        .expect_err("zero resolved facts is not a successful observation");
    let text = err.to_string();
    assert!(text.contains("/facts/renamedLux"), "{text}");
    assert!(text.contains("lux"), "也要說裝置實際回了哪些鍵：{text}");
}

// ---------------------------------------------------------------------------
// link-transports-051：pair-locked 的原因與可重試時間不得被丟掉
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pairing_lockout_is_not_reported_as_a_wrong_pairing_code() {
    let raw = MockRawLink::new("esp32-desk01", Some("4321"));
    raw.pair_locked.store(true, Ordering::SeqCst);
    let link = Arc::new(DeviceLink::new(
        raw.clone(),
        "esp32-desk01".into(),
        // 碼是「對的」——鎖定期內裝置根本不比對。
        Some("4321".into()),
    ));
    let err = link.ensure_ready().await.expect_err("locked out");
    match &err {
        LinkError::Refused(detail) => {
            assert!(detail.starts_with("pairing-locked"), "{detail}");
            assert!(
                detail.contains("30 s"),
                "要帶上裝置算好的重試時間：{detail}"
            );
            assert!(
                detail.contains("may well be correct"),
                "不得說成配對碼錯誤：{detail}"
            );
        }
        other => panic!("expected Refused, got {other}"),
    }

    // 收據原因要與「身分／配對被拒」分開，人才不會去改一個其實正確的碼。
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
        .expect("receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed
    );
    assert!(
        receipt.errors.iter().any(|e| e.code == "pairing-locked"),
        "{receipt:?}"
    );
}

// ---------------------------------------------------------------------------
// protocol-conformance-030：裝置停用配對時的 pair-ok 不是配對證據
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_device_that_never_asked_for_pairing_leaves_the_code_unverified() {
    // 裝置的 PAIRING_CODE 是空字串：hello.pairing=false，且對任何碼都回 pair-ok。
    let raw = MockRawLink::new("esp32-desk01", None);
    raw.declares_pairing.store(false, Ordering::SeqCst);
    let link = Arc::new(DeviceLink::new(
        raw.clone(),
        "esp32-desk01".into(),
        Some("1234".into()), // spec 以為有配對
    ));
    link.ensure_ready().await.expect("device accepts any code");
    assert!(
        link.pairing_unverified(),
        "裝置沒要求配對＝這組碼從未被比對過，不得靜默當成已配對"
    );

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
        .expect("receipt");
    assert_eq!(
        receipt.driver_response.get("pairingUnverified"),
        Some(&json!(true)),
        "收據必須說出「這次的身分證據只有裝置自報的 deviceId」：{receipt:?}"
    );

    // 對照組：裝置真的要求配對時，收據不得帶這個註記。
    let raw2 = MockRawLink::new("esp32-desk01", Some("4321"));
    let link2 = Arc::new(DeviceLink::new(
        raw2,
        "esp32-desk01".into(),
        Some("4321".into()),
    ));
    let actuator2 = LinkActuator::new(
        actuator_spec("vibe"),
        CommandSpec {
            name: "vibe.pulse".into(),
            params: None,
        },
        "esp32-desk".into(),
        link2.clone(),
        "serial",
    );
    let receipt2 = actuator2
        .execute(bounded_action("a2"))
        .await
        .expect("receipt");
    assert!(!link2.pairing_unverified());
    assert!(!receipt2.driver_response.contains_key("pairingUnverified"));
}

// ---------------------------------------------------------------------------
// link-transports-052：等待期間丟掉的訊息不得被講成「裝置沒回」
// ---------------------------------------------------------------------------

/// 一條「什麼都不回，但把 undecodable 計數往上加」的假連線：模擬裝置有回、
/// 但 host 端解不開（BLE 被 ATT MTU 截斷、serial 亂碼）。
struct NoisyLink {
    inbound: broadcast::Sender<DeviceMsg>,
    undecodable: AtomicU64,
}

impl NoisyLink {
    fn new() -> Arc<Self> {
        let (inbound, _) = broadcast::channel(8);
        Arc::new(Self {
            inbound,
            undecodable: AtomicU64::new(0),
        })
    }
}

#[async_trait::async_trait]
impl RawLink for NoisyLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        Ok(())
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        let msg: Value = serde_json::from_str(&line).expect("host sends json");
        if msg["type"] == "who" {
            let _ = self.inbound.send(DeviceMsg::Hello {
                device_id: "esp32-desk01".into(),
                fw: None,
                proto: Some(1),
                caps: vec![],
                pairing: false,
                pairing_locked: false,
            });
        } else {
            // 裝置「有回」，但那幾則我們解不開。
            self.undecodable.fetch_add(3, Ordering::SeqCst);
        }
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        self.inbound.subscribe()
    }

    fn connected(&self) -> bool {
        true
    }

    fn undecodable_messages(&self) -> u64 {
        self.undecodable.load(Ordering::SeqCst)
    }

    fn shutdown(&self) {}

    fn describe(&self) -> String {
        "noisy mock".into()
    }
}

#[tokio::test]
async fn a_timeout_after_undecodable_replies_does_not_claim_the_device_was_silent() {
    let raw = NoisyLink::new();
    let link = DeviceLink::new(raw, "esp32-desk01".into(), None);
    let err = link
        .command("a1", "led.set", json!({}), Duration::from_millis(300))
        .await
        .expect_err("no ack arrived");
    match &err {
        LinkError::Timeout(detail) => {
            assert!(detail.contains("could not be decoded"), "{detail}");
            assert!(detail.contains("did answer something"), "{detail}");
        }
        other => panic!("expected Timeout, got {other}"),
    }
    // read 也一樣：不得只說「device did not answer read」。
    let err = link
        .read_state(Duration::from_millis(300))
        .await
        .expect_err("no state arrived");
    assert!(err.to_string().contains("could not be decoded"), "{err}");
}

// ---------------------------------------------------------------------------
// link-transports-028：只有 `not-found` 才是「裝置回報沒有可取消的效果」
// ---------------------------------------------------------------------------

/// 韌體的 cancel 有三種結局：`ack cancelled:true`（真的停了）、
/// `err not-found`（解析成功、確定沒有這個 id 在跑＝沒有可取消的效果）、
/// 以及**別的** err——`busy`（BLE 入站佇列滿，這則 cancel 在解析前就被丟掉，
/// 震動／蜂鳴仍在跑，而且那則 err 沒有 id）、`not-paired`（裝置重開機）。
/// 後者代表「這則 cancel 沒被處理」，結果必須是 UNKNOWN；把它講成
/// 「裝置回報沒有可取消的效果」是在安全路徑上給一個裝置從未做過的確定宣稱。
#[tokio::test]
async fn only_a_not_found_error_means_the_device_has_no_cancellable_effect() {
    use interaction_core::{ActionId, ActuatorError};

    for (with_id, reason) in [
        (false, "busy"),
        (true, "not-paired"),
        (true, "rate-limited"),
    ] {
        let raw = MockRawLink::new("esp32-desk01", None);
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
        actuator
            .execute(bounded_action("a1"))
            .await
            .expect("receipt");
        raw.set_cancel_err(Some((with_id, reason)));
        let err = actuator
            .cancel(&ActionId::new("a1"))
            .await
            .expect_err("device did not confirm the cancel");
        let text = err.to_string();
        assert!(
            !text.contains("no cancellable effect"),
            "err {reason}（with_id={with_id}）代表這則 cancel 沒被處理，\
             不得講成「裝置回報沒有可取消的效果」：{text}"
        );
        assert!(text.contains("UNKNOWN"), "取消結果未知就要說未知：{text}");
        assert!(text.contains(reason), "裝置給的原因不得被丟掉：{text}");
        assert!(
            matches!(err, ActuatorError::Unavailable(_)),
            "未知不是 NotFound：{err:?}"
        );
    }

    // 對照組：`not-found` 是裝置真的表過態——才可以說「沒有可取消的效果」。
    let raw = MockRawLink::new("esp32-desk01", None);
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
    actuator
        .execute(bounded_action("a1"))
        .await
        .expect("receipt");
    raw.set_cancel_err(Some((true, "not-found")));
    let err = actuator
        .cancel(&ActionId::new("a1"))
        .await
        .expect_err("nothing to cancel");
    assert!(
        matches!(err, ActuatorError::NotFound(_)),
        "裝置明確說 not-found：{err:?}"
    );
    assert!(err.to_string().contains("no cancellable effect"), "{err}");
}

// ---------------------------------------------------------------------------
// protocol-conformance-042：「這條通道已經配對過」不是「裝置不比對配對碼」
// ---------------------------------------------------------------------------

/// 韌體：`doc["pairing"] = pairingEnabled() && !g_linkPaired[link];`——同一條
/// 通道配對成功之後，之後的 hello 就會說 `pairing:false`，但 `pair` 仍然逐位
/// 比對 PAIRING_CODE。host 重連（Serial 偵測不到 USB 拔插、MQTT 連線沒斷時
/// 裝置端的已配對狀態不會重置）時，不得把「碼確實被比對過」寫成
/// 「配對碼從未被比對」——那是對真實裝置行為的錯誤陳述。
#[tokio::test]
async fn an_already_paired_channel_is_not_reported_as_a_never_compared_code() {
    let raw = MockRawLink::new("esp32-desk01", Some("4321"));
    let link = Arc::new(DeviceLink::new(
        raw.clone(),
        "esp32-desk01".into(),
        Some("4321".into()),
    ));
    // 第一次握手：裝置要求配對（hello.pairing=true）→ 碼真的被比對過。
    link.ensure_ready().await.expect("first handshake");
    assert!(!link.pairing_unverified(), "第一次握手就比對過碼");

    // 重連：host 重新握手，裝置端仍記得這條通道已配對 → hello.pairing=false，
    // 但 pair 照樣逐位比對（mock 與韌體同規則）。
    raw.set_connected(false);
    raw.set_connected(true);
    raw.declares_pairing.store(false, Ordering::SeqCst);
    link.ensure_ready().await.expect("second handshake");
    assert_eq!(raw.sent_count("pair"), 2, "重連後仍然送碼給裝置比對");
    assert!(
        !link.pairing_unverified(),
        "這台裝置示範過它真的比對配對碼；hello.pairing=false 只代表\
         「這條通道已經配對」，不得斷言「配對碼未經比對」"
    );

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
        .expect("receipt");
    assert!(
        !receipt.driver_response.contains_key("pairingUnverified"),
        "{receipt:?}"
    );
    // 但也不得反過來假裝這次握手重新比對過：收據誠實說出「這次沒有重比」。
    assert_eq!(
        receipt.driver_response.get("pairingNotRecompared"),
        Some(&json!(true)),
        "{receipt:?}"
    );
}
