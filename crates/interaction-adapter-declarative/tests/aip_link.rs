//! 線協定 v1.1 的 `aip` 訊息（MockRawLink——不碰任何真硬體，明確是模擬）。
//!
//! 覆蓋：裝置→host 與 host→裝置的 `aip` 行、握手／配對完成前收到 aip 一律
//! 拒絕（比照 iPhone 的 auth-ok 閘門）、重連（世代更替）後舊的准入立即失效、
//! envelope 上限、未握手時 send_aip 不得寫出任何位元組。
//!
//! 誠實：`send_aip` 回 Ok 只代表「已寫上線」，不代表對方收到、更不代表對方
//! 套用了——AIP 的回覆是對端自己送回來的另一則 envelope。

use interaction_adapter_declarative::fragment::{
    fragment_envelope_line, FRAG_CAP, MAX_REASSEMBLED_BYTES,
};
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
    /// 裝置在 hello 宣告的能力清單（分片測試用 `aip.frag/1`）。
    caps: Vec<String>,
    /// 這條線的單則上限（`None` ＝沒有上限，像 iPhone 的 wss）。
    max_line_bytes: Option<usize>,
}

impl MockRawLink {
    fn new(device_id: &str, pairing_code: Option<&str>) -> Arc<Self> {
        Self::with_caps(device_id, pairing_code, vec!["led.set".into()], None)
    }

    /// 帶能力宣告與單則上限的假裝置（分片路徑用）。
    fn with_caps(
        device_id: &str,
        pairing_code: Option<&str>,
        caps: Vec<String>,
        max_line_bytes: Option<usize>,
    ) -> Arc<Self> {
        let (inbound, _) = broadcast::channel(64);
        Arc::new(Self {
            inbound,
            sent: Mutex::new(vec![]),
            device_id: device_id.into(),
            pairing_code: pairing_code.map(String::from),
            generation: AtomicU64::new(1),
            connected: AtomicBool::new(true),
            caps,
            max_line_bytes,
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
                    caps: self.caps.clone(),
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

    fn max_line_bytes(&self) -> Option<usize> {
        self.max_line_bytes
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

// ---------------------------------------------------------------------------
// 裝置線 v1.2：分片（`aip-frag`）
// ---------------------------------------------------------------------------

/// 一則放得進行上限的 envelope 照舊走單行 `aip`——分片不得無條件開啟
/// （多切一刀就是多一次可能丟片的機會）。
#[tokio::test]
async fn a_small_envelope_still_goes_out_as_one_aip_line() {
    let raw = MockRawLink::with_caps(
        "esp32-01",
        None,
        vec!["led.set".into(), FRAG_CAP.into()],
        Some(639),
    );
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    link.send_aip(
        &json!({"specVersion": "aip/1.0", "messageType": "state"}),
        Duration::from_millis(500),
    )
    .await
    .expect("send_aip");
    assert_eq!(
        raw.sent_types(),
        vec!["who", "aip"],
        "放得進去的訊息不得被切開：{:?}",
        raw.sent_types()
    );
}

/// 放不進行上限、但裝置宣告了 `aip.frag/1`：切片送出，每一行都 ≤ 上限，
/// 而且串回來逐位元組等於原 envelope。
#[tokio::test]
async fn an_oversized_envelope_is_fragmented_when_the_device_advertises_it() {
    let raw = MockRawLink::with_caps(
        "esp32-01",
        None,
        vec!["led.set".into(), FRAG_CAP.into()],
        Some(639),
    );
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    let envelope = json!({
        "specVersion": "aip/1.0",
        "messageType": "state",
        "pad": "y".repeat(1_500),
    });
    link.send_aip(&envelope, Duration::from_millis(500))
        .await
        .expect("fragmented send");

    let frames: Vec<Value> = raw
        .sent_lines()
        .into_iter()
        .filter(|m| m["type"] == "aip-frag")
        .collect();
    assert!(frames.len() >= 3, "1.5 KiB 必須切成多片：{}", frames.len());
    assert!(
        !raw.sent_types().iter().any(|t| t == "aip"),
        "分片路徑不得同時再送一則整行 aip"
    );
    let mut joined = String::new();
    for (i, frame) in frames.iter().enumerate() {
        let line = serde_json::to_string(frame).expect("line");
        assert!(
            line.len() <= 639,
            "第 {i} 片 {} bytes 超過行上限",
            line.len()
        );
        assert_eq!(frame["seq"], i as u64, "片序必須連續");
        assert_eq!(frame["total"], frames.len() as u64);
        joined.push_str(frame["data"].as_str().expect("data"));
    }
    assert_eq!(
        serde_json::from_str::<Value>(&joined).expect("reassembled json"),
        envelope
    );
}

/// 裝置**沒有**宣告 `aip.frag/1`：維持既有行為——一個位元組都不寫，並且
/// 說得出原因（`over-line-limit-no-fragmentation`），不靜默丟棄。
#[tokio::test]
async fn an_oversized_envelope_is_refused_when_the_device_cannot_reassemble() {
    let raw = MockRawLink::with_caps("esp32-01", None, vec!["led.set".into()], Some(639));
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    let error = link
        .send_aip(
            &json!({"specVersion": "aip/1.0", "pad": "y".repeat(1_500)}),
            Duration::from_millis(500),
        )
        .await
        .expect_err("不支援分片時必須拒絕");
    match &error {
        LinkError::Refused(detail) => {
            assert!(
                detail.contains("over-line-limit-no-fragmentation"),
                "拒絕原因必須說得出是「超過行上限而對端不會重組」：{detail}"
            );
        }
        other => panic!("expected Refused, got {other:?}"),
    }
    assert!(
        !raw.sent_types()
            .iter()
            .any(|t| t == "aip" || t == "aip-frag"),
        "拒絕時不得寫出任何位元組：{:?}",
        raw.sent_types()
    );
}

/// 入站：多片組回一則 envelope，對呼叫端仍然只是**一則** `Admitted`。
#[tokio::test]
async fn inbound_fragments_reassemble_into_one_admitted_envelope() {
    let raw = MockRawLink::with_caps(
        "esp32-01",
        None,
        vec!["led.set".into(), FRAG_CAP.into()],
        Some(639),
    );
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    link.ensure_ready().await.expect("handshake");

    let envelope =
        json!({"specVersion": "aip/1.0", "messageType": "event", "pad": "z".repeat(1_200)});
    let text = serde_json::to_string(&envelope).expect("text");
    let frames = fragment_envelope_line(&text, 639, 77).expect("fragments");
    let mut admitted = None;
    for (i, frame) in frames.iter().enumerate() {
        let HostMsg::AipFrag {
            xfer,
            seq,
            total,
            bytes,
            crc,
            data,
        } = frame
        else {
            panic!("expected a fragment");
        };
        let msg = DeviceMsg::AipFrag {
            xfer: *xfer,
            seq: *seq,
            total: *total,
            bytes: *bytes,
            crc: crc.clone(),
            data: data.clone(),
        };
        match link.admit_aip(&msg) {
            Some(AipAdmission::FragmentBuffered) => assert!(i + 1 < frames.len()),
            Some(AipAdmission::Admitted(value)) => admitted = Some(value),
            other => panic!("第 {i} 片得到 {other:?}"),
        }
    }
    assert_eq!(admitted.expect("reassembled"), envelope);
}

/// 未握手的連線上收到分片：與整行 `aip` 同一道閘門——不得重組、不得放行。
#[tokio::test]
async fn inbound_fragments_are_refused_before_the_handshake() {
    let raw = MockRawLink::with_caps(
        "esp32-01",
        Some("9927"),
        vec!["led.set".into(), FRAG_CAP.into()],
        Some(639),
    );
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), Some("9927".into()));
    let frame = DeviceMsg::AipFrag {
        xfer: 1,
        seq: 0,
        total: 2,
        bytes: 40,
        crc: "00000000".into(),
        data: "{".into(),
    };
    assert_eq!(link.admit_aip(&frame), Some(AipAdmission::RefusedNotPaired));
    assert_eq!(link.aip_refused_before_pairing(), 1);
}

/// 亂序的一片：整筆丟棄，並且回報得出原因（呼叫端要留稽核）。
#[tokio::test]
async fn an_out_of_order_inbound_fragment_is_dropped_with_a_reason() {
    let raw = MockRawLink::with_caps(
        "esp32-01",
        None,
        vec!["led.set".into(), FRAG_CAP.into()],
        Some(639),
    );
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    link.ensure_ready().await.expect("handshake");

    let text = serde_json::to_string(&json!({"specVersion": "aip/1.0", "pad": "z".repeat(1_200)}))
        .expect("text");
    let frames = fragment_envelope_line(&text, 639, 5).expect("fragments");
    let to_device = |frame: &HostMsg| match frame {
        HostMsg::AipFrag {
            xfer,
            seq,
            total,
            bytes,
            crc,
            data,
        } => DeviceMsg::AipFrag {
            xfer: *xfer,
            seq: *seq,
            total: *total,
            bytes: *bytes,
            crc: crc.clone(),
            data: data.clone(),
        },
        other => panic!("{other:?}"),
    };
    assert_eq!(
        link.admit_aip(&to_device(&frames[0])),
        Some(AipAdmission::FragmentBuffered)
    );
    match link.admit_aip(&to_device(&frames[2])) {
        Some(AipAdmission::FragmentDropped(drop)) => assert_eq!(drop.reason, "out-of-order"),
        other => panic!("亂序必須整筆丟棄，得到 {other:?}"),
    }
}

/// 重連（世代改變）／stop-all 都會取消進行中的入站傳輸，而且**留得下痕跡**。
#[tokio::test]
async fn a_reconnect_cancels_an_in_flight_inbound_transfer() {
    let raw = MockRawLink::with_caps(
        "esp32-01",
        None,
        vec!["led.set".into(), FRAG_CAP.into()],
        Some(639),
    );
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    link.ensure_ready().await.expect("handshake");
    let text = serde_json::to_string(&json!({"specVersion": "aip/1.0", "pad": "z".repeat(1_200)}))
        .expect("text");
    let frames = fragment_envelope_line(&text, 639, 8).expect("fragments");
    let HostMsg::AipFrag {
        xfer,
        seq,
        total,
        bytes,
        crc,
        data,
    } = &frames[0]
    else {
        panic!("fragment");
    };
    assert_eq!(
        link.admit_aip(&DeviceMsg::AipFrag {
            xfer: *xfer,
            seq: *seq,
            total: *total,
            bytes: *bytes,
            crc: crc.clone(),
            data: data.clone(),
        }),
        Some(AipAdmission::FragmentBuffered)
    );
    raw.reconnect();
    let dropped = link
        .expire_fragments()
        .expect("重連必須取消進行中的傳輸並回報");
    assert_eq!(dropped.reason, "link-reset");
    assert!(link.expire_fragments().is_none(), "不得重複回報同一筆");
}

/// 超過重組上限的 envelope 連切都不切（8 KiB 是這條線的天花板，分片沒有
/// 放寬它）。
#[tokio::test]
async fn fragmentation_never_raises_the_envelope_cap() {
    let raw = MockRawLink::with_caps(
        "esp32-01",
        None,
        vec!["led.set".into(), FRAG_CAP.into()],
        Some(639),
    );
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    let error = link
        .send_aip(
            &json!({"pad": "x".repeat(MAX_REASSEMBLED_BYTES + 64)}),
            Duration::from_millis(500),
        )
        .await
        .expect_err("超過 8 KiB 必須拒絕");
    assert!(matches!(error, LinkError::Refused(_)), "{error:?}");
    assert!(
        !raw.sent_types()
            .iter()
            .any(|t| t == "aip" || t == "aip-frag"),
        "{:?}",
        raw.sent_types()
    );
}
