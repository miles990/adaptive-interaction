//! DeviceLink 誠實核心測試（MockRawLink——不碰任何真硬體，明確是模擬）。
//!
//! 覆蓋：hello 身分驗證、配對碼、ack 對應、ack 逾時＝結果未知且不重送、
//! cancel、read state、重連（世代更替）後強制重新握手。

use interaction_adapter_declarative::protocol::{
    encode_host_msg, DeviceLink, DeviceMsg, HostMsg, LinkError, RawLink,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
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
    ack_replies: std::sync::atomic::AtomicBool,
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
            ack_replies: std::sync::atomic::AtomicBool::new(true),
        })
    }

    fn sent_count(&self, msg_type: &str) -> usize {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v["type"] == msg_type)
            .count()
    }

    fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl RawLink for MockRawLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        Ok(())
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        let msg: Value = serde_json::from_str(&line).expect("host sends json");
        self.sent.lock().unwrap().push(msg.clone());
        let reply = match msg["type"].as_str() {
            Some("who") => Some(DeviceMsg::Hello {
                device_id: self.device_id.clone(),
                fw: Some("1.0.0".into()),
                proto: Some(1),
                caps: vec!["led.set".into()],
                pairing: self.pairing_code.is_some(),
            }),
            Some("pair") => {
                if self.pairing_code.as_deref() == msg["code"].as_str() {
                    Some(DeviceMsg::PairOk)
                } else {
                    Some(DeviceMsg::PairFail)
                }
            }
            Some("cmd") => {
                if self.ack_replies.load(Ordering::SeqCst) {
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
            Some("read") => Some(DeviceMsg::State {
                device_id: Some(self.device_id.clone()),
                facts: json!({"lux": 123, "distanceMm": 456}),
            }),
            Some("stop-all") => Some(DeviceMsg::Ack {
                id: None,
                applied: None,
                dup: None,
                cancelled: None,
                stop_all: Some(true),
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
