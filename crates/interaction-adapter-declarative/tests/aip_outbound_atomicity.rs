//! 出站原子性：一則被分片的 envelope，它的每一片必須在線上**連續**寫出
//! （MockRawLink——不碰任何真硬體，明確是模擬）。
//!
//! 為什麼要有這一支：裝置線 v1.2 的「每裝置同時只有一筆進行中的傳輸」在**接收**
//! 端由單槽 `Reassembler` 強制，但送出端原本沒有對應機制。同一條 link 上兩個併發的
//! `send_aip`（`character_session_apply` 的 `Output::Persist` 就是各自併發派送的）
//! 會在 await 點交錯成 A0 B0 A1 B1…；對端的重組器看到 `xfer` 換掉就 supersede，
//! 之後 `seq != 0` 一律 `unknown-xfer` → **兩則都完全丟失**，而兩個呼叫端都拿到
//! `Ok`，一行稽核都沒有。那正是「已寫上線」被當成「送到了」的那一格。

use interaction_adapter_declarative::fragment::{
    FragmentDrop, Reassembler, ReassemblyStep, FRAG_CAP,
};
use interaction_adapter_declarative::protocol::{
    DeviceLink, DeviceMsg, LinkError, LinkState, RawLink,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

/// serial／mqtt 的行上限（bytes，不含換行）。
const SERIAL_LIMIT: usize = 639;

/// 每次 `send_before` 都先讓出執行緒的假裝置：把「兩個併發的 send_aip 會不會
/// 交錯」這件事變成**確定性**的，而不是碰運氣的競態。
struct YieldingLink {
    inbound: broadcast::Sender<DeviceMsg>,
    /// 線上實際寫出的每一行，依寫出順序。
    wire: Mutex<Vec<Value>>,
    device_id: String,
    generation: AtomicU64,
    connected: AtomicBool,
    caps: Vec<String>,
    /// 第幾次 `send_before` 之後開始失敗（`usize::MAX` ＝永不失敗）。
    fail_after: AtomicUsize,
    writes: AtomicUsize,
}

impl YieldingLink {
    fn new(caps: Vec<String>) -> Arc<Self> {
        let (inbound, _) = broadcast::channel(64);
        Arc::new(Self {
            inbound,
            wire: Mutex::new(vec![]),
            device_id: "esp32-01".into(),
            generation: AtomicU64::new(1),
            connected: AtomicBool::new(true),
            caps,
            fail_after: AtomicUsize::new(usize::MAX),
            writes: AtomicUsize::new(0),
        })
    }

    fn wire(&self) -> Vec<Value> {
        self.wire.lock().map(|w| w.clone()).unwrap_or_default()
    }

    /// 線上的 `aip-frag` 行，依寫出順序（每一項是 `(xfer, seq)`）。
    fn frag_order(&self) -> Vec<(u32, u16)> {
        self.wire()
            .into_iter()
            .filter(|m| m["type"] == "aip-frag")
            .map(|m| {
                (
                    m["xfer"].as_u64().unwrap_or_default() as u32,
                    m["seq"].as_u64().unwrap_or_default() as u16,
                )
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl RawLink for YieldingLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        if self.connected.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(LinkError::Unavailable("mock device unplugged".into()))
        }
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        let msg: Value = serde_json::from_str(&line).expect("host sends json");
        if let Ok(mut wire) = self.wire.lock() {
            wire.push(msg.clone());
        }
        if msg["type"] == "who" {
            let _ = self.inbound.send(DeviceMsg::Hello {
                device_id: self.device_id.clone(),
                fw: Some("mock-1.0".into()),
                proto: Some(1),
                caps: self.caps.clone(),
                pairing: false,
                pairing_locked: false,
            });
        }
        Ok(())
    }

    /// **每一片之前都讓出執行緒**：沒有出站鎖的話，兩個併發的 `send_aip` 一定
    /// 會交錯（這一支測試要的就是這個確定性）。
    async fn send_before(
        &self,
        line: String,
        _deadline: std::time::Instant,
    ) -> Result<(), LinkError> {
        tokio::task::yield_now().await;
        let done = self.writes.fetch_add(1, Ordering::SeqCst);
        if done >= self.fail_after.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable("the mock link went down".into()));
        }
        self.send(line).await
    }

    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        self.inbound.subscribe()
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn max_line_bytes(&self) -> Option<usize> {
        Some(SERIAL_LIMIT)
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

fn big_envelope(marker: &str) -> Value {
    json!({
        "specVersion": "aip/1.0",
        "messageType": "state",
        "marker": marker,
        "pad": marker.repeat(600),
    })
}

/// 把線上寫出的每一行 `aip-frag` 依序餵進一個重組器，回傳組回來的 envelope。
/// 這就是對端（host 的 `Reassembler`／`esp32-serial-sim.py` 的 `handle_aip_frag`
/// 同一套規則）真正會看到的東西。
fn replay(wire: &[Value]) -> Vec<Value> {
    let mut re = Reassembler::new();
    let now = std::time::Instant::now();
    let mut completed = Vec::new();
    let step_of = |step: ReassemblyStep, completed: &mut Vec<Value>| match step {
        ReassemblyStep::Completed(value) => completed.push(value),
        ReassemblyStep::Superseded { step, .. } => {
            if let ReassemblyStep::Completed(value) = *step {
                completed.push(value);
            }
        }
        _ => {}
    };
    for line in wire {
        if line["type"] != "aip-frag" {
            continue;
        }
        let msg = DeviceMsg::AipFrag {
            xfer: line["xfer"].as_u64().unwrap_or_default() as u32,
            seq: line["seq"].as_u64().unwrap_or_default() as u16,
            total: line["total"].as_u64().unwrap_or_default() as u16,
            bytes: line["bytes"].as_u64().unwrap_or_default() as u32,
            crc: line["crc"].as_str().unwrap_or_default().to_string(),
            data: line["data"].as_str().unwrap_or_default().to_string(),
        };
        step_of(re.accept(&msg, 1, now), &mut completed);
    }
    completed
}

/// 兩則併發的大 envelope：線上的片不得交錯，而且**兩則**都必須在對端組得回來。
#[tokio::test]
async fn two_concurrent_fragmented_sends_do_not_interleave_on_the_wire() {
    let raw = YieldingLink::new(vec!["led.set".into(), FRAG_CAP.into()]);
    let link = Arc::new(DeviceLink::new(raw.clone(), "esp32-01".into(), None));
    link.ensure_ready().await.expect("handshake");

    let a = big_envelope("a");
    let b = big_envelope("b");
    let (ra, rb) = tokio::join!(
        link.send_aip(&a, Duration::from_secs(5)),
        link.send_aip(&b, Duration::from_secs(5)),
    );
    ra.expect("send a");
    rb.expect("send b");

    let order = raw.frag_order();
    assert!(order.len() >= 4, "兩則都必須切成多片：{order:?}");

    // 1) 對端真的組得回來——兩則都是（`Ok` 才配稱為「已寫上線」）。
    let completed = replay(&raw.wire());
    assert_eq!(
        completed.len(),
        2,
        "兩則併發的 envelope 都必須在對端組得回來，實得 {}：{order:?}",
        completed.len()
    );
    assert!(completed.contains(&a), "envelope a 沒有組回來");
    assert!(completed.contains(&b), "envelope b 沒有組回來");

    // 2) 線上的片必須以「整筆傳輸」為單位連續出現。
    let mut seen: Vec<u32> = Vec::new();
    for (xfer, _) in &order {
        if seen.last() != Some(xfer) {
            assert!(
                !seen.contains(xfer),
                "傳輸 {xfer} 的片在線上被別的傳輸切斷了：{order:?}"
            );
            seen.push(*xfer);
        }
    }
}

/// 中途寫失敗：不得回 `Ok`。已經寫出去的片對呼叫端是「送達與否未知」
/// （`Uncertain`），不是「什麼都沒送」（`Refused`）——呼叫端據此留稽核。
#[tokio::test]
async fn a_mid_transfer_write_failure_is_reported_as_uncertain_not_ok() {
    let raw = YieldingLink::new(vec!["led.set".into(), FRAG_CAP.into()]);
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    link.ensure_ready().await.expect("handshake");
    // 握手（who）之後的第一片可以寫出，第二片開始失敗。
    raw.fail_after
        .store(raw.writes.load(Ordering::SeqCst) + 1, Ordering::SeqCst);

    let error = link
        .send_aip(&big_envelope("c"), Duration::from_secs(5))
        .await
        .expect_err("中途失敗不得回 Ok");
    match &error {
        LinkError::Uncertain(detail) => {
            assert!(
                detail.contains("fragment"),
                "錯誤必須說得出是第幾片失敗：{detail}"
            );
        }
        other => panic!("expected Uncertain, got {other:?}"),
    }
    // 已經寫出去的片留在線上——對端會因為缺片逾時整筆丟掉並留痕。
    assert!(
        !raw.frag_order().is_empty(),
        "第一片本來就寫出去了：這正是「未知」而不是「沒送」的原因"
    );
}

/// 第一片就失敗：一個位元組都沒寫出去，錯誤照傳輸層原樣回報（不得升級成
/// 「未知」——那會讓呼叫端以為可能已經送到）。
#[tokio::test]
async fn a_failure_on_the_first_fragment_still_says_nothing_was_written() {
    let raw = YieldingLink::new(vec!["led.set".into(), FRAG_CAP.into()]);
    let link = DeviceLink::new(raw.clone(), "esp32-01".into(), None);
    link.ensure_ready().await.expect("handshake");
    raw.fail_after
        .store(raw.writes.load(Ordering::SeqCst), Ordering::SeqCst);

    let error = link
        .send_aip(&big_envelope("d"), Duration::from_secs(5))
        .await
        .expect_err("第一片失敗必須回錯");
    assert!(
        matches!(error, LinkError::Unavailable(_)),
        "expected Unavailable, got {error:?}"
    );
    assert!(
        raw.frag_order().is_empty(),
        "第一片都沒寫成功，線上不得有任何片"
    );
}

/// `FragmentDrop` 是 `Copy`：稽核佇列存的是值，不是借用（編譯期釘住）。
#[test]
fn fragment_drop_is_a_plain_value() {
    let drop = FragmentDrop {
        xfer: 1,
        reason: "superseded",
        received: 0,
        total: 2,
    };
    let copy = drop;
    assert_eq!(copy, drop);
}
