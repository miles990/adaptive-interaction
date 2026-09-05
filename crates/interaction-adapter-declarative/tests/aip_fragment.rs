//! 裝置線 v1.2 的 `aip-frag` 分片（純函式＋重組器；不碰任何真硬體）。
//!
//! 明確標示：這裡沒有裝置，只有 host 端的切片／重組邏輯。真板驗收仍為零。
//!
//! 誠實不變量（每一條都有一支測試）：
//! - 有界：重組緩衝有上限（`MAX_REASSEMBLED_BYTES`），惡意的 `total`／`bytes`
//!   在第一片就被拒絕，不先配置再說。
//! - 整筆丟棄：缺片／重片／亂序／截斷／crc 不符一律整筆丟掉並說得出原因，
//!   絕不把一份殘缺的 envelope 交給上層。
//! - UTF-8：切片不得切在字元中間（切壞了就是製造一份「解不開」的訊息，
//!   而那在 log 上長得像裝置壞掉）。

use interaction_adapter_declarative::fragment::{
    crc32, fragment_envelope_line, FragmentError, Reassembler, ReassemblyStep, MAX_FRAGMENTS,
    MAX_REASSEMBLED_BYTES,
};
use interaction_adapter_declarative::protocol::{encode_host_msg, DeviceMsg, HostMsg};
use std::time::{Duration, Instant};

/// serial／mqtt 的行上限（bytes，不含換行）。
const SERIAL_LIMIT: usize = 639;
/// BLE 的單則上限。
const BLE_LIMIT: usize = 480;

fn envelope_text(payload_len: usize) -> String {
    format!(
        "{{\"specVersion\":\"aip/1.0\",\"pad\":\"{}\"}}",
        "x".repeat(payload_len)
    )
}

/// 把一份 envelope 文字切成 frames，並把每一片轉成裝置→host 方向的訊息
/// （host 端的重組器吃的就是這一種）。
fn frames_as_device_msgs(text: &str, limit: usize, xfer: u32) -> Vec<DeviceMsg> {
    fragment_envelope_line(text, limit, xfer)
        .expect("fragmentable")
        .into_iter()
        .map(|host| match host {
            HostMsg::AipFrag {
                xfer,
                seq,
                total,
                bytes,
                crc,
                data,
            } => DeviceMsg::AipFrag {
                xfer,
                seq,
                total,
                bytes,
                crc,
                data,
            },
            other => panic!("expected a fragment, got {other:?}"),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 切片：行上限與 UTF-8 邊界
// ---------------------------------------------------------------------------

/// 剛好 639 bytes 的一行**不分片**（邊界值必須通過，不得多切一刀）；
/// 640 bytes 才開始分片，而且每一片編碼後整行都 ≤ 上限。
#[test]
fn the_line_limit_boundary_is_exact() {
    // 先找出「編碼後剛好 639 bytes」的 envelope。
    let mut pad = 0usize;
    let exact = loop {
        let text = envelope_text(pad);
        let line = encode_host_msg(&HostMsg::Aip {
            envelope: serde_json::from_str(&text).expect("json"),
        });
        if line.len() == SERIAL_LIMIT {
            break text;
        }
        assert!(line.len() < SERIAL_LIMIT, "overshot the limit at pad={pad}");
        pad += 1;
    };
    let line = encode_host_msg(&HostMsg::Aip {
        envelope: serde_json::from_str(&exact).expect("json"),
    });
    assert_eq!(line.len(), SERIAL_LIMIT);

    // 640 bytes：必須切，而且每一片都放得進去。
    let over = envelope_text(pad + 1);
    let frames = fragment_envelope_line(&over, SERIAL_LIMIT, 7).expect("fragmentable");
    assert!(frames.len() >= 2, "640 bytes 必須切成至少兩片");
    for frame in &frames {
        let encoded = encode_host_msg(frame);
        assert!(
            encoded.len() <= SERIAL_LIMIT,
            "每一片編碼後整行必須 ≤ {SERIAL_LIMIT}：{} bytes",
            encoded.len()
        );
    }
}

/// 中文與 emoji 不得被切在 UTF-8 字元中間：每一片的 `data` 自己必須是合法的
/// UTF-8 字串，串回來也必須逐位元組等於原文。
#[test]
fn multibyte_characters_are_never_split_in_the_middle() {
    let text = format!(
        "{{\"specVersion\":\"aip/1.0\",\"note\":\"{}\"}}",
        "中文與表情🙂".repeat(60)
    );
    assert!(text.len() > BLE_LIMIT * 2);
    let frames = fragment_envelope_line(&text, BLE_LIMIT, 1).expect("fragmentable");
    let mut joined = String::new();
    for frame in &frames {
        let encoded = encode_host_msg(frame);
        assert!(encoded.len() <= BLE_LIMIT, "{} bytes", encoded.len());
        match frame {
            HostMsg::AipFrag { data, .. } => joined.push_str(data),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(joined, text, "串回來必須與原文逐位元組相同");
}

/// 超過 `MAX_REASSEMBLED_BYTES`（8 KiB）的 envelope 根本不切：這條線上不存在
/// 一份合法的 8 KiB+ envelope，切了也會在對端被拒。
#[test]
fn an_envelope_over_the_reassembly_cap_is_not_fragmented() {
    let text = envelope_text(MAX_REASSEMBLED_BYTES + 10);
    match fragment_envelope_line(&text, SERIAL_LIMIT, 1) {
        Err(FragmentError::TooLarge { bytes }) => assert!(bytes > MAX_REASSEMBLED_BYTES),
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

/// 片數上限：切出來的片數不得超過 `MAX_FRAGMENTS`（有界）。
#[test]
fn the_fragment_count_stays_bounded() {
    let text = envelope_text(MAX_REASSEMBLED_BYTES - 200);
    let frames = fragment_envelope_line(&text, BLE_LIMIT, 1).expect("fragmentable");
    assert!(
        frames.len() <= MAX_FRAGMENTS as usize,
        "{} fragments",
        frames.len()
    );
}

// ---------------------------------------------------------------------------
// 重組：完整、丟片、重片、亂序、截斷、惡意 total、crc、逾時、cancel
// ---------------------------------------------------------------------------

fn feed(re: &mut Reassembler, msg: &DeviceMsg, at: Instant) -> ReassemblyStep {
    re.accept(msg, 1, at)
}

#[test]
fn a_complete_transfer_reassembles_to_the_original_envelope() {
    let text = envelope_text(1200);
    let frames = frames_as_device_msgs(&text, SERIAL_LIMIT, 42);
    let mut re = Reassembler::new();
    let now = Instant::now();
    let mut done = None;
    for (i, frame) in frames.iter().enumerate() {
        match feed(&mut re, frame, now) {
            ReassemblyStep::Buffered => assert!(i + 1 < frames.len()),
            ReassemblyStep::Completed(value) => done = Some(value),
            other => panic!("{other:?}"),
        }
    }
    let value = done.expect("the last fragment must complete the transfer");
    assert_eq!(
        value,
        serde_json::from_str::<serde_json::Value>(&text).unwrap()
    );
}

#[test]
fn a_missing_fragment_drops_the_whole_transfer() {
    let text = envelope_text(1200);
    let frames = frames_as_device_msgs(&text, SERIAL_LIMIT, 3);
    assert!(frames.len() >= 3);
    let mut re = Reassembler::new();
    let now = Instant::now();
    assert!(matches!(
        feed(&mut re, &frames[0], now),
        ReassemblyStep::Buffered
    ));
    // 跳過第 1 片直接送第 2 片：亂序＝整筆丟棄。
    match feed(&mut re, &frames[2], now) {
        ReassemblyStep::Dropped(drop) => assert_eq!(drop.reason, "out-of-order"),
        other => panic!("{other:?}"),
    }
    // 丟掉之後緩衝必須是空的（不留半份）。
    assert!(!re.has_transfer());
}

#[test]
fn a_repeated_fragment_drops_the_whole_transfer() {
    let text = envelope_text(1200);
    let frames = frames_as_device_msgs(&text, SERIAL_LIMIT, 4);
    let mut re = Reassembler::new();
    let now = Instant::now();
    assert!(matches!(
        feed(&mut re, &frames[0], now),
        ReassemblyStep::Buffered
    ));
    match feed(&mut re, &frames[0], now) {
        ReassemblyStep::Dropped(drop) => assert_eq!(drop.reason, "out-of-order"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_truncated_transfer_is_dropped_by_the_declared_byte_count() {
    let text = envelope_text(1200);
    let mut frames = frames_as_device_msgs(&text, SERIAL_LIMIT, 5);
    // 最後一片被截短：長度對不上宣告的 bytes。
    let last = frames.len() - 1;
    if let DeviceMsg::AipFrag { data, .. } = &mut frames[last] {
        data.truncate(data.len() - 3);
    }
    let mut re = Reassembler::new();
    let now = Instant::now();
    let mut ended = None;
    for frame in &frames {
        match feed(&mut re, frame, now) {
            ReassemblyStep::Buffered => {}
            other => ended = Some(other),
        }
    }
    match ended.expect("the truncated transfer must end in a drop") {
        ReassemblyStep::Dropped(drop) => assert_eq!(drop.reason, "truncated"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_malicious_total_or_byte_count_is_refused_on_the_first_fragment() {
    let mut re = Reassembler::new();
    let now = Instant::now();
    let evil_total = DeviceMsg::AipFrag {
        xfer: 1,
        seq: 0,
        total: u16::MAX,
        bytes: 100,
        crc: "00000000".into(),
        data: "{".into(),
    };
    match feed(&mut re, &evil_total, now) {
        ReassemblyStep::Dropped(drop) => assert_eq!(drop.reason, "bad-total"),
        other => panic!("{other:?}"),
    }
    let evil_bytes = DeviceMsg::AipFrag {
        xfer: 2,
        seq: 0,
        total: 2,
        bytes: (MAX_REASSEMBLED_BYTES + 1) as u32,
        crc: "00000000".into(),
        data: "{".into(),
    };
    match feed(&mut re, &evil_bytes, now) {
        ReassemblyStep::Dropped(drop) => assert_eq!(drop.reason, "bad-bytes"),
        other => panic!("{other:?}"),
    }
    assert!(!re.has_transfer(), "被拒的傳輸不得佔著緩衝");
}

/// 一片就宣稱自己比宣告的總長還長：在**寫進緩衝之前**就拒絕（有界）。
#[test]
fn a_fragment_that_overruns_the_declared_length_is_dropped() {
    let mut re = Reassembler::new();
    let now = Instant::now();
    let frame = DeviceMsg::AipFrag {
        xfer: 9,
        seq: 0,
        total: 2,
        bytes: 4,
        crc: "00000000".into(),
        data: "0123456789".into(),
    };
    match feed(&mut re, &frame, now) {
        ReassemblyStep::Dropped(drop) => assert_eq!(drop.reason, "over-declared-bytes"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_wrong_crc_drops_the_whole_transfer() {
    let text = envelope_text(1200);
    let mut frames = frames_as_device_msgs(&text, SERIAL_LIMIT, 6);
    for frame in &mut frames {
        if let DeviceMsg::AipFrag { crc, .. } = frame {
            *crc = "deadbeef".into();
        }
    }
    let mut re = Reassembler::new();
    let now = Instant::now();
    let mut ended = None;
    for frame in &frames {
        match feed(&mut re, frame, now) {
            ReassemblyStep::Buffered => {}
            other => ended = Some(other),
        }
    }
    match ended.expect("a wrong crc must end in a drop") {
        ReassemblyStep::Dropped(drop) => assert_eq!(drop.reason, "crc-mismatch"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_new_transfer_cancels_the_previous_one_and_says_so() {
    let text = envelope_text(1200);
    let first = frames_as_device_msgs(&text, SERIAL_LIMIT, 10);
    let second = frames_as_device_msgs(&text, SERIAL_LIMIT, 11);
    let mut re = Reassembler::new();
    let now = Instant::now();
    assert!(matches!(
        feed(&mut re, &first[0], now),
        ReassemblyStep::Buffered
    ));
    match feed(&mut re, &second[0], now) {
        ReassemblyStep::Superseded { dropped, .. } => {
            assert_eq!(dropped.xfer, 10);
            assert_eq!(dropped.reason, "superseded");
        }
        other => panic!("{other:?}"),
    }
    assert!(re.has_transfer(), "新的傳輸必須接手緩衝");
}

/// 逾時（自最後一片起 2 秒）：整筆丟棄並說得出原因；不得無限期佔著緩衝。
#[test]
fn a_stalled_transfer_times_out_and_is_dropped() {
    let text = envelope_text(1200);
    let frames = frames_as_device_msgs(&text, SERIAL_LIMIT, 12);
    let mut re = Reassembler::new();
    let start = Instant::now();
    assert!(matches!(
        feed(&mut re, &frames[0], start),
        ReassemblyStep::Buffered
    ));
    assert!(re.expire(1, start + Duration::from_millis(500)).is_none());
    let dropped = re
        .expire(1, start + Duration::from_millis(2_100))
        .expect("a stalled transfer must be dropped");
    assert_eq!(dropped.reason, "timeout");
    assert_eq!(dropped.xfer, 12);
    assert!(!re.has_transfer());
}

/// 連線世代改變（重連／握手作廢）：進行中的傳輸整筆丟棄。
#[test]
fn a_reconnect_cancels_the_in_flight_transfer() {
    let text = envelope_text(1200);
    let frames = frames_as_device_msgs(&text, SERIAL_LIMIT, 13);
    let mut re = Reassembler::new();
    let now = Instant::now();
    assert!(matches!(
        feed(&mut re, &frames[0], now),
        ReassemblyStep::Buffered
    ));
    let dropped = re.expire(2, now).expect("a new generation must drop it");
    assert_eq!(dropped.reason, "link-reset");
    assert!(!re.has_transfer());
}

/// 明確取消（hello／斷線／revoke／stop-all／rebind）留得下痕跡。
#[test]
fn an_explicit_cancel_reports_what_it_threw_away() {
    let text = envelope_text(1200);
    let frames = frames_as_device_msgs(&text, SERIAL_LIMIT, 14);
    let mut re = Reassembler::new();
    let now = Instant::now();
    assert!(matches!(
        feed(&mut re, &frames[0], now),
        ReassemblyStep::Buffered
    ));
    let dropped = re.cancel("stop-all").expect("cancel must report the loss");
    assert_eq!(dropped.reason, "stop-all");
    assert_eq!(dropped.xfer, 14);
    assert!(
        re.cancel("stop-all").is_none(),
        "沒有東西可丟時不得憑空報告"
    );
}

/// 完整後的 JSON 仍然要能解析：解不開就是整筆丟棄（不交半份給上層）。
#[test]
fn a_complete_but_unparseable_payload_is_dropped() {
    let text = "{\"specVersion\":\"aip/1.0\",".to_string();
    let bytes = text.len() as u32;
    let crc = crc32(text.as_bytes());
    let frame = DeviceMsg::AipFrag {
        xfer: 15,
        seq: 0,
        total: 1,
        bytes,
        crc: format!("{crc:08x}"),
        data: text,
    };
    let mut re = Reassembler::new();
    match feed(&mut re, &frame, Instant::now()) {
        ReassemblyStep::Dropped(drop) => assert_eq!(drop.reason, "bad-json"),
        other => panic!("{other:?}"),
    }
}

/// crc32 就是標準的 IEEE crc32（與 python `zlib.crc32` 相同）——模擬器與 host
/// 必須算出同一個值，否則每一次傳輸都會被誤判成損毀。
#[test]
fn crc32_matches_the_standard_ieee_vectors() {
    assert_eq!(crc32(b""), 0x0000_0000);
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(
        crc32(b"The quick brown fox jumps over the lazy dog"),
        0x414F_A339
    );
}
