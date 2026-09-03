//! interaction-character：Character Presentation Protocol（CPP）v1.0 的**權威實作**。
//!
//! 對應規格：`docs/character-protocol/README.md`。這個 crate 是純邏輯：
//! 沒有 tokio、沒有 I/O（只處理呼叫端傳進來的 manifest bytes），時間一律由呼叫端注入
//! （`chrono::DateTime<Utc>` 或 millis），所以每一條規則都可以確定性地測試。
//!
//! 模組對照：
//!
//! | 模組 | 規格章節 |
//! |---|---|
//! | [`manifest`] | §2 Manifest、§2.1 驗證、§2.2 舊 pack 遷移 |
//! | [`capability`] | §3 能力與協商（§3.4 解析演算法） |
//! | [`intent`] | §4 Character Intent／truthState／priority 下限／Envelope |
//! | [`input`] | §6 Input events 正規化與節流 |
//! | [`receipt`] | §7 Command 回執與合法順序 |
//! | [`lifecycle`] | §7 Adapter 生命週期、§1 Character Role |
//! | [`wire`] | §8 Wire messages 與上限 |
//! | [`gateway`] | §0／§5／§7 Gateway 純狀態機（排程、去重、過期、降級、世代） |
//! | [`schema`] | §10 JSON Schema（golden：`schemas/character-protocol.schema.json`） |
//!
//! 不變量（與 `CLAUDE.md` 一致）：`truthState` 只由 Runtime 設定，adapter 訊息在型別層就沒有
//! 這個欄位；accepted ≠ started ≠ completed；acknowledged 之後只會變 `uncertain`，不會被補成
//! `completed`；斷線／crash 時進行中的 command 一律 `uncertain`；所有佇列都有上限。

pub mod capability;
pub mod gateway;
pub mod input;
pub mod intent;
pub mod lifecycle;
pub mod manifest;
pub mod receipt;
pub mod schema;
pub mod wire;

pub use capability::*;
pub use gateway::*;
pub use input::*;
pub use intent::*;
pub use lifecycle::*;
pub use manifest::*;
pub use receipt::*;
pub use schema::*;
pub use wire::*;

/// 協定版本（`major.minor`）。1.x 內保證安全 intent 名稱、truthState 名稱與 priority 下限不變。
pub const PROTOCOL_VERSION: &str = "1.0";
/// 本實作的協定 major。major 不同一律拒絕握手。
pub const PROTOCOL_MAJOR: u32 = 1;
/// 本實作的協定 minor。對方 minor 較新時允許並標記 `newerMinor`。
pub const PROTOCOL_MINOR: u32 = 0;

/// UTC 時間戳（RFC3339 序列化）。
pub type Timestamp = chrono::DateTime<chrono::Utc>;

/// 解析 `major.minor` 版本字串；不接受其他格式（不猜）。
pub fn parse_protocol_version(value: &str) -> Option<(u32, u32)> {
    let (major, minor) = value.trim().split_once('.')?;
    let major: u32 = major.parse().ok()?;
    let minor: u32 = minor.parse().ok()?;
    Some((major, minor))
}

/// 以字元數（非 byte 數）計算長度，所有「≤ N 字」規則都用它。
pub(crate) fn char_len(value: &str) -> usize {
    value.chars().count()
}

/// 截斷輸入內容供錯誤訊息回顯（規則：不得回顯超過 200 字）。
pub(crate) fn truncate_for_echo(value: &str) -> String {
    const MAX: usize = 200;
    if char_len(value) <= MAX {
        return value.to_string();
    }
    let mut out: String = value.chars().take(MAX).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_major_minor_only() {
        assert_eq!(parse_protocol_version("1.0"), Some((1, 0)));
        assert_eq!(parse_protocol_version("1.7"), Some((1, 7)));
        assert_eq!(parse_protocol_version("2.0"), Some((2, 0)));
        assert_eq!(parse_protocol_version("1"), None);
        assert_eq!(parse_protocol_version("1.x"), None);
        assert_eq!(parse_protocol_version("1.0.0"), None);
        assert_eq!(parse_protocol_version(""), None);
    }

    #[test]
    fn echo_is_truncated_to_200_chars() {
        let long = "字".repeat(500);
        let echoed = truncate_for_echo(&long);
        assert_eq!(char_len(&echoed), 201);
        assert!(echoed.ends_with('…'));
        assert_eq!(truncate_for_echo("short"), "short");
    }
}
