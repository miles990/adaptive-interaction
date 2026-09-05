//! 裝置線 v1.2：一則 AIP envelope 的**分片**（`aip-frag`）與重組。
//!
//! 為什麼要有它：參考韌體的序列行上限是 639 bytes（`g_serialBuf[640]`），
//! MQTT 相同，BLE 是 480 bytes。Character Session 協商的第二則回覆
//! （`state{kind:"snapshot"}`）與任何含 `members` 的 patch 都比那個大——在此
//! 之前它們在寫上線之前就被拒絕，於是一台「已加入」的裝置從第一秒起就拿不到
//! 初始快照。分片讓那些訊息真的送得到，而且**不放寬任何上限**：每一片編碼後
//! 的整行仍然 ≤ 行上限。
//!
//! 協定版本仍是 v1：`aip-frag` 與 `aip` 一樣是**追加**訊息型別——舊韌體不認得
//! 就忽略、舊 host 當未知訊息丟棄，兩端都不壞。只有在 `hello.caps` 宣告
//! [`FRAG_CAP`] 的裝置身上才會使用它。
//!
//! 誠實不變量：
//! - **有界**：重組緩衝上限 [`MAX_REASSEMBLED_BYTES`]、片數上限
//!   [`MAX_FRAGMENTS`]、每台裝置同時只有一筆進行中的傳輸。惡意的 `total`／
//!   `bytes` 在**第一片**就被拒絕，不先配置再說。
//! - **整筆丟棄**：缺片／重片／亂序／截斷／crc 不符／解不開一律整筆丟掉並說
//!   得出原因。半份 envelope 絕不交給上層——那會把「傳輸壞了」演成「裝置說了
//!   一句沒有意義的話」。
//! - **不靜默**：每一次丟棄都回傳一個 [`FragmentDrop`]，呼叫端負責留稽核。
//! - **UTF-8**：切片只切在字元邊界。切壞了就是製造一份解不開的訊息，而那在
//!   log 上長得像裝置壞掉。

use crate::protocol::{encode_host_msg, DeviceMsg, HostMsg};
use serde_json::Value;
use std::time::{Duration, Instant};

/// `hello.caps` 裡宣告「我聽得懂 `aip-frag`」的字串。沒有宣告就不使用分片
/// （對端會把它當未知訊息整行丟掉，而我們會以為送出去了）。
pub const FRAG_CAP: &str = "aip.frag/1";

/// 重組後的 envelope 位元組上限。與 [`crate::protocol::MAX_AIP_ENVELOPE_BYTES`]
/// 相同：分片是為了穿過**行**上限，不是為了放寬 envelope 上限。
pub const MAX_REASSEMBLED_BYTES: usize = crate::protocol::MAX_AIP_ENVELOPE_BYTES;

/// 一筆傳輸最多幾片。8 KiB ÷ 最小的每片有效載荷（BLE 480 − 表頭）仍遠小於
/// 這個數字；它只是不讓一個壞掉的（或惡意的）對端宣稱「這筆要 65535 片」。
pub const MAX_FRAGMENTS: u16 = 64;

/// 從**最後一片**算起，一筆未完成的傳輸還能存活多久。
pub const FRAGMENT_TIMEOUT: Duration = Duration::from_secs(2);

/// 一筆被丟棄的傳輸（稽核用）。`reason` 是固定字串（不回顯裝置輸入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentDrop {
    pub xfer: u32,
    pub reason: &'static str,
    /// 丟棄時已經收下幾片（診斷用：0 代表第一片就被拒）。
    pub received: u16,
    /// 對端宣告的總片數。
    pub total: u16,
}

/// 餵一片進重組器之後發生了什麼。
#[derive(Debug, Clone, PartialEq)]
pub enum ReassemblyStep {
    /// 收下了，整份還沒完成。
    Buffered,
    /// 整份完成並通過 crc／JSON 檢查。
    Completed(Value),
    /// 這一筆整個丟掉了（呼叫端必須留稽核）。
    Dropped(FragmentDrop),
    /// 新的 `xfer` 到達：前一筆被取消，新的這一筆已經收下（或也立刻被丟）。
    Superseded {
        dropped: FragmentDrop,
        step: Box<ReassemblyStep>,
    },
}

/// 切片失敗的原因（切不出來就不送——不製造注定被丟棄的位元組）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentError {
    /// envelope 本身超過 [`MAX_REASSEMBLED_BYTES`]：這條線上不存在這種合法訊息。
    TooLarge { bytes: usize },
    /// 行上限小到連一個表頭都放不下（設定錯誤）。
    LimitTooSmall { limit: usize, overhead: usize },
    /// 切出來會超過 [`MAX_FRAGMENTS`] 片。
    TooManyFragments { needed: usize },
}

impl std::fmt::Display for FragmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FragmentError::TooLarge { bytes } => write!(
                f,
                "envelope is {bytes} bytes, over the {MAX_REASSEMBLED_BYTES} byte reassembly cap"
            ),
            FragmentError::LimitTooSmall { limit, overhead } => write!(
                f,
                "the per-line limit ({limit}) is smaller than one fragment header ({overhead})"
            ),
            FragmentError::TooManyFragments { needed } => write!(
                f,
                "the envelope would need {needed} fragments, over the {MAX_FRAGMENTS} cap"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// crc32（IEEE，反射多項式 0xEDB88320）——與 python `zlib.crc32` 相同
// ---------------------------------------------------------------------------

/// 標準 IEEE crc32。自己實作（十幾行）而不是引一個新依賴：模擬器用
/// `zlib.crc32`、host 用這個，兩邊必須算出同一個值。
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// 8 位小寫十六進位（線上格式）。
pub fn crc32_hex(bytes: &[u8]) -> String {
    format!("{:08x}", crc32(bytes))
}

// ---------------------------------------------------------------------------
// 切片
// ---------------------------------------------------------------------------

/// 一片表頭在**最壞情況**下佔用的位元組數（所有數字都取最大位數）。
///
/// 為什麼取最壞值而不是實際值：`total` 要切完才知道，而切多少又取決於
/// `total` 的位數。用最壞值一次算完，切出來的每一行保證 ≤ 上限
/// （可能少放幾個 byte，但絕不超過）。
fn fragment_overhead() -> usize {
    encode_host_msg(&HostMsg::AipFrag {
        xfer: u32::MAX,
        seq: u16::MAX,
        total: u16::MAX,
        bytes: u32::MAX,
        crc: "00000000".into(),
        data: String::new(),
    })
    .len()
}

/// 一個字元寫進 JSON 字串之後佔幾個位元組（serde_json 的規則：非 ASCII 原樣
/// 輸出 UTF-8，不用 `\u` escape）。
fn escaped_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' => 2,
        '\u{8}' | '\u{c}' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// 把一份 envelope 的 JSON 文字切成 `aip-frag` 訊息。
///
/// `limit` ＝這條線的單則上限（bytes，不含換行）。回傳的每一則編碼後都 ≤ 它。
/// 切點只落在字元邊界；串接所有 `data` 逐位元組等於 `text`。
pub fn fragment_envelope_line(
    text: &str,
    limit: usize,
    xfer: u32,
) -> Result<Vec<HostMsg>, FragmentError> {
    let bytes = text.len();
    if bytes > MAX_REASSEMBLED_BYTES {
        return Err(FragmentError::TooLarge { bytes });
    }
    let overhead = fragment_overhead();
    if limit <= overhead {
        return Err(FragmentError::LimitTooSmall { limit, overhead });
    }
    let budget = limit - overhead;
    let crc = crc32_hex(text.as_bytes());

    // 先切出每一片的文字（只看字元邊界與 escape 後的長度）。
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;
    for c in text.chars() {
        let cost = escaped_len(c);
        if used + cost > budget && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            used = 0;
        }
        if cost > budget {
            // 單一字元就超過預算：上限小到放不下任何內容。
            return Err(FragmentError::LimitTooSmall { limit, overhead });
        }
        current.push(c);
        used += cost;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    if chunks.len() > MAX_FRAGMENTS as usize {
        return Err(FragmentError::TooManyFragments {
            needed: chunks.len(),
        });
    }
    let total = chunks.len() as u16;
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(i, data)| HostMsg::AipFrag {
            xfer,
            seq: i as u16,
            total,
            bytes: bytes as u32,
            crc: crc.clone(),
            data,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// 重組
// ---------------------------------------------------------------------------

struct InFlight {
    xfer: u32,
    total: u16,
    bytes: u32,
    crc: String,
    /// 握手世代：重連過的話這一筆不再屬於任何一條成立的連線。
    generation: u64,
    next_seq: u16,
    buf: String,
    last_at: Instant,
}

impl InFlight {
    fn drop_report(&self, reason: &'static str) -> FragmentDrop {
        FragmentDrop {
            xfer: self.xfer,
            reason,
            received: self.next_seq,
            total: self.total,
        }
    }
}

/// 每條 link 一個：同時只有**一筆**進行中的傳輸（有界）。
#[derive(Default)]
pub struct Reassembler {
    current: Option<InFlight>,
}

impl Reassembler {
    pub fn new() -> Self {
        Self { current: None }
    }

    /// 目前有沒有一筆進行中的傳輸（測試／診斷用）。
    pub fn has_transfer(&self) -> bool {
        self.current.is_some()
    }

    /// 明確取消（hello 重新握手／斷線／revoke／stop-all／rebind）。
    /// 有東西被丟掉才回報——沒有的話不得憑空製造一筆稽核。
    pub fn cancel(&mut self, reason: &'static str) -> Option<FragmentDrop> {
        self.current.take().map(|f| f.drop_report(reason))
    }

    /// 逾時／世代守衛：呼叫端定期呼叫（收訊迴圈的輪詢窗）。
    pub fn expire(&mut self, generation: u64, now: Instant) -> Option<FragmentDrop> {
        let current = self.current.as_ref()?;
        if current.generation != generation {
            return self.cancel("link-reset");
        }
        if now.duration_since(current.last_at) >= FRAGMENT_TIMEOUT {
            return self.cancel("timeout");
        }
        None
    }

    /// 餵一片進來。`msg` 不是 `aip-frag` 就不該呼叫這裡。
    pub fn accept(&mut self, msg: &DeviceMsg, generation: u64, now: Instant) -> ReassemblyStep {
        let DeviceMsg::AipFrag {
            xfer,
            seq,
            total,
            bytes,
            crc,
            data,
        } = msg
        else {
            return ReassemblyStep::Dropped(FragmentDrop {
                xfer: 0,
                reason: "not-a-fragment",
                received: 0,
                total: 0,
            });
        };

        // 先處理「換了一筆」：新的 xfer 到達＝前一筆取消（並稽核）。
        let mut superseded = None;
        if let Some(current) = &self.current {
            if current.xfer != *xfer || current.generation != generation {
                superseded = self.cancel("superseded");
            }
        }

        let step = self.accept_inner(*xfer, *seq, *total, *bytes, crc, data, generation, now);
        match superseded {
            Some(dropped) => ReassemblyStep::Superseded {
                dropped,
                step: Box::new(step),
            },
            None => step,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_inner(
        &mut self,
        xfer: u32,
        seq: u16,
        total: u16,
        bytes: u32,
        crc: &str,
        data: &str,
        generation: u64,
        now: Instant,
    ) -> ReassemblyStep {
        let refuse = |reason: &'static str, received: u16| {
            ReassemblyStep::Dropped(FragmentDrop {
                xfer,
                reason,
                received,
                total,
            })
        };
        if self.current.is_none() {
            // 一筆傳輸只能從第 0 片開始：半路插進來的片沒有表頭可信。
            if seq != 0 {
                return refuse("unknown-xfer", 0);
            }
            if total == 0 || total > MAX_FRAGMENTS {
                return refuse("bad-total", 0);
            }
            if bytes == 0 || bytes as usize > MAX_REASSEMBLED_BYTES {
                return refuse("bad-bytes", 0);
            }
            if crc.len() != 8 || !crc.bytes().all(|b| b.is_ascii_hexdigit()) {
                return refuse("bad-crc", 0);
            }
            if data.len() > bytes as usize {
                return refuse("over-declared-bytes", 0);
            }
            self.current = Some(InFlight {
                xfer,
                total,
                bytes,
                crc: crc.to_ascii_lowercase(),
                generation,
                next_seq: 0,
                buf: String::new(),
                last_at: now,
            });
        }

        let Some(current) = self.current.as_mut() else {
            return refuse("unknown-xfer", 0);
        };
        // 表頭必須逐片一致：中途改 total／bytes／crc 就是另一筆傳輸。
        if total != current.total
            || bytes != current.bytes
            || !crc.eq_ignore_ascii_case(&current.crc)
        {
            let report = current.drop_report("header-mismatch");
            self.current = None;
            return ReassemblyStep::Dropped(report);
        }
        if seq != current.next_seq {
            let report = current.drop_report("out-of-order");
            self.current = None;
            return ReassemblyStep::Dropped(report);
        }
        if current.buf.len() + data.len() > current.bytes as usize {
            let report = current.drop_report("over-declared-bytes");
            self.current = None;
            return ReassemblyStep::Dropped(report);
        }
        current.buf.push_str(data);
        current.next_seq = current.next_seq.saturating_add(1);
        current.last_at = now;
        if current.next_seq < current.total {
            return ReassemblyStep::Buffered;
        }

        // 收齊了：長度 → crc → JSON，三關都過才交給上層。
        // （`take()` 一定是 `Some`——上面剛寫進去；但 production code 不用
        //  `expect()`：真的沒有就當成一筆丟掉的傳輸，不 panic。）
        let Some(finished) = self.current.take() else {
            return refuse("lost-buffer", 0);
        };
        if finished.buf.len() != finished.bytes as usize {
            return ReassemblyStep::Dropped(finished.drop_report("truncated"));
        }
        if crc32_hex(finished.buf.as_bytes()) != finished.crc {
            return ReassemblyStep::Dropped(finished.drop_report("crc-mismatch"));
        }
        match serde_json::from_str::<Value>(&finished.buf) {
            Ok(value) => ReassemblyStep::Completed(value),
            Err(_) => ReassemblyStep::Dropped(finished.drop_report("bad-json")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表頭的最壞情況大小必須真的蓋得住任何一組實際值——不然切出來的行會
    /// 超過上限，而超過的那一行在裝置端是整行丟棄。
    #[test]
    fn the_worst_case_overhead_covers_every_real_header() {
        let overhead = fragment_overhead();
        let real = encode_host_msg(&HostMsg::AipFrag {
            xfer: 1,
            seq: 0,
            total: 2,
            bytes: 700,
            crc: "abcdef01".into(),
            data: String::new(),
        });
        assert!(real.len() <= overhead, "{} > {overhead}", real.len());
    }

    /// escape 後才是線上的長度：一段全是 `"` 的內容每個字元佔兩個 byte。
    #[test]
    fn quotes_cost_two_bytes_each_when_budgeting() {
        assert_eq!(escaped_len('"'), 2);
        assert_eq!(escaped_len('\\'), 2);
        assert_eq!(escaped_len('中'), 3);
        assert_eq!(escaped_len('a'), 1);
        assert_eq!(escaped_len('\u{1}'), 6);
    }
}
