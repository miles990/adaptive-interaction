//! §11 上限（1.0 常數）。所有集合、訊息、字串都有界。

/// 單則 wire 訊息（含 envelope）序列化後的位元組上限。
pub const MAX_MESSAGE_BYTES: usize = 65_536;
/// `payload` 序列化後的位元組上限。
pub const MAX_PAYLOAD_BYTES: usize = 32_768;
/// `messageId`／`correlationId`／`causationId`／`sessionId`／`source.id` 的字元上限。
pub const MAX_ID_CHARS: usize = 128;
/// `name` 的字元上限。
pub const MAX_NAME_CHARS: usize = 128;
/// payload 內任一字串的字元上限。
pub const MAX_STRING_CHARS: usize = 2_000;
/// payload 的 JSON 巢狀深度上限（根物件深度 1）。
pub const MAX_JSON_DEPTH: usize = 8;
/// 每個 (session, source) 的 messageId 去重環大小。
pub const DEDUPE_RING: usize = 256;
/// Session 事件日誌（delta replay）環大小。
pub const EVENT_LOG_RING: usize = 512;
/// `occurredAt` 與 host 時鐘可容忍的偏差；超出只稽核不拒絕。
pub const MAX_CLOCK_SKEW_MS: i64 = 30_000;
/// 互動事件（`character.interaction.*`）預設 TTL。
pub const DEFAULT_INTERACTION_TTL_MS: i64 = 5_000;
/// Behavior Intent 預設 TTL。
pub const DEFAULT_INTENT_TTL_MS: i64 = 10_000;
/// Session 成員上限。
pub const MAX_MEMBERS: usize = 16;
