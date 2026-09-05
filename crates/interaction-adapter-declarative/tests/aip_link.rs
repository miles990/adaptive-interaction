//! 線協定 v1.1 的 `aip` 訊息（MockRawLink——不碰任何真硬體，明確是模擬）。
//!
//! 覆蓋：裝置→host 與 host→裝置的 `aip` 行、握手／配對完成前收到 aip 一律
//! 拒絕（比照 iPhone 的 auth-ok 閘門）、重連（世代更替）後舊的准入立即失效、
//! envelope 上限、未握手時 send_aip 不得寫出任何位元組。
//!
//! 誠實：`send_aip` 回 Ok 只代表「已寫上線」，不代表對方收到、更不代表對方
//! 套用了——AIP 的回覆是對端自己送回來的另一則 envelope。

use interaction_adapter_declarative::protocol::{
    encode_host_msg, parse_device_msg, AipAdmission, DeviceLink, DeviceMsg, HostMsg, LinkError,
    LinkState, RawLink, MAX_AIP_ENVELOPE_BYTES,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

/// 最小可程式化假裝置：只回 hello／pair-ok，其餘不回。
struct MockRawLink {
    inbound: broadcast::Sender<DeviceMsg>,
    sent: Mutex<Vec<Value>>,
    device_id: String,
    pairing_code: Option<String>,
    generation: AtomicU64,
    connected: AtomicBool,
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
            connected: AtomicBool::new(true),
        })
    }

    fn sent_types(&self) -> Vec<String> {
        self.sent
            .lock()
            .map(|s| {
                s.iter()
                    .map(|m| m["type"].as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn sent_lines(&self) -> Vec<Value> {
        self.sent.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn reconnect(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl RawLink for MockRawLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        if self.connected.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(LinkError::Unavailable("mock device unplugged".into()))
        }
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable("mock device unplugged".into()));
        }
        let msg: Value = serde_json::from_str(&line).expect("host sends json");
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(msg.clone());
        }
        match msg["type"].as_str() {
            Some("who") => {
                let _ = self.inbound.send(DeviceMsg::Hello {
                    device_id: self.device_id.clone(),
                    fw: Some("mock-1.0".into()),
                    proto: Some(1),
                    caps: vec!["led.set".into()],
                    pairing: self.pairing_code.is_some(),
                    pairing_locked: false,
                });
            }
            Some("pair") => {
                let _ = self.inbound.send(DeviceMsg::PairOk);
            }
            _ => {}
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
        self.connected.load(Ordering::SeqCst)
    }

    fn link_state(&self) -> LinkState {
        if self.connected.load(Ordering::SeqCst) {
            LinkState::Connected
        } else {
            LinkState::Disconnected
        }
    }

    fn shutdown(&self) {
        self.connected.store(false, Ordering::SeqCst);
    }

    fn describe(&self) -> String {
        "mock".into()
    }
}

// ---------------------------------------------------------------------------
// 線格式：一行一則 JSON，兩個方向都是 `{"type":"aip","envelope":{…}}`
// ---------------------------------------------------------------------------

#[test]
fn aip_lines_round_trip_in_both_directions() {
    let parsed = parse_device_msg(r#"{"type":"aip","envelope":{"specVersion":"aip/1.0"}}"#)
        .expect("device aip line parses");
    match parsed {
        DeviceMsg::Aip { envelope } => {
            assert_eq!(envelope["specVersion"], "aip/1.0");
        }
        other => panic!("expected DeviceMsg::Aip, got {other:?}"),
    }

    let line = encode_host_msg(&HostMsg::Aip {
        envelope: json!({"specVersion": "aip/1.0", "messageType": "state"}),
    });
    let value: Value = serde_json::from_str(&line).expect("host aip line is json");
    assert_eq!(value["type"], "aip");
    assert_eq!(value["envelope"]["messageType"], "state");
}

// ---------------------------------------------------------------------------
// 准入閘門：hello＋配對完成前收到的 aip 一律不放行（比照 iPhone auth-ok）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn device_aip_is_refused_until_the_handshake_and_pairing_completed() {
    let raw = MockRawLink::new("esp32-01", Some("9927"));
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), Some("9927".into()));

    let before = DeviceMsg::Aip {
        envelope: json!({"specVersion": "aip/1.0"}),
    };
    assert_eq!(
        link.admit_aip(&before),
        Some(AipAdmission::RefusedNotPaired),
        "未握手／未配對前的 aip 不得放行"
    );
    assert_eq!(link.aip_refused_before_pairing(), 1);

    link.ensure_ready().await.expect("handshake");
    match link.admit_aip(&before) {
        Some(AipAdmission::Admitted(envelope)) => {
            assert_eq!(envelope["specVersion"], "aip/1.0");
        }
        other => panic!("配對完成後應放行，得到 {other:?}"),
    }
    assert_eq!(link.aip_refused_before_pairing(), 1, "放行不得計入拒絕");

    // 非 aip 的訊息不歸這個閘門管。
    assert_eq!(link.admit_aip(&DeviceMsg::PairOk), None);

    // 重連＝握手作廢：舊准入立即失效，不得沿用上一條連線的配對。
    raw.reconnect();
    assert_eq!(
        link.admit_aip(&before),
        Some(AipAdmission::RefusedNotPaired),
        "重連後未重新握手就不得再放行 aip"
    );
    assert_eq!(link.aip_refused_before_pairing(), 2);
}

#[tokio::test]
async fn oversized_device_aip_is_refused_without_being_admitted() {
    let raw = MockRawLink::new("esp32-01", None);
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    link.ensure_ready().await.expect("handshake");

    let huge = DeviceMsg::Aip {
        envelope: json!({"payload": "x".repeat(MAX_AIP_ENVELOPE_BYTES + 64)}),
    };
    match link.admit_aip(&huge) {
        Some(AipAdmission::RefusedTooLarge { bytes }) => {
            assert!(bytes > MAX_AIP_ENVELOPE_BYTES, "{bytes}");
        }
        other => panic!("超限的 envelope 不得放行，得到 {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 出站：身分不符的 link 一個位元組都不得寫出 aip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn send_aip_writes_nothing_when_the_device_identity_does_not_match() {
    let raw = MockRawLink::new("someone-else", None);
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);

    let error = link
        .send_aip(
            &json!({"specVersion": "aip/1.0"}),
            Duration::from_millis(500),
        )
        .await
        .expect_err("身分不符必須拒絕");
    assert!(
        matches!(error, LinkError::Refused(_)),
        "expected Refused, got {error:?}"
    );
    assert!(
        !raw.sent_types().iter().any(|t| t == "aip"),
        "拒絕的連線不得寫出 aip：{:?}",
        raw.sent_types()
    );
}

#[tokio::test]
async fn send_aip_writes_one_line_after_the_handshake() {
    let raw = MockRawLink::new("esp32-01", Some("9927"));
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), Some("9927".into()));

    link.send_aip(
        &json!({"specVersion": "aip/1.0", "messageType": "state"}),
        Duration::from_millis(500),
    )
    .await
    .expect("send_aip");

    let types = raw.sent_types();
    assert_eq!(
        types,
        vec!["who", "pair", "aip"],
        "aip 必須排在 hello/pair 之後"
    );
    let aip = raw
        .sent_lines()
        .into_iter()
        .find(|m| m["type"] == "aip")
        .expect("aip line");
    assert_eq!(aip["envelope"]["messageType"], "state");
}

#[tokio::test]
async fn send_aip_refuses_an_oversized_envelope_without_writing() {
    let raw = MockRawLink::new("esp32-01", None);
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    link.ensure_ready().await.expect("handshake");

    let error = link
        .send_aip(
            &json!({"payload": "x".repeat(MAX_AIP_ENVELOPE_BYTES + 64)}),
            Duration::from_millis(500),
        )
        .await
        .expect_err("超限必須拒絕");
    assert!(
        matches!(error, LinkError::Refused(_)),
        "expected Refused, got {error:?}"
    );
    assert!(
        !raw.sent_types().iter().any(|t| t == "aip"),
        "超限的 envelope 不得寫出任何位元組：{:?}",
        raw.sent_types()
    );
}
