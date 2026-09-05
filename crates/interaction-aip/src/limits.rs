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
/// 協商結果裡 `unsupportedInputs` 的上限。對方宣告的 `inputs` 是外部輸入、本身無界，
/// 而 host 的協商回覆是一則要送上線的 AIP 訊息：不截斷就會超過 [`MAX_PAYLOAD_BYTES`]，
/// 變成 host 自己送出一則規範接收端必須拒絕的訊息（session-integrity-060）。
///
/// 這是**協商結果的有界性要求**，不是 wire 上的欄位長度上限；但它仍然發布進
/// `schemas/aip-1.0.schema.json` 的 `limits` 表，因為 TypeScript 與 Swift 也各自實作
/// [`crate::negotiate_capabilities`]，截斷點必須是同一個數字。發布之後三端都從 codegen
/// 讀它（`AIP_LIMITS.maxUnsupportedInputs`），不再有人手寫同值的字面量。
pub const MAX_UNSUPPORTED_INPUTS: usize = 32;

/// 一則 `character.session.resume` 回覆最多攜帶幾則 patch。
///
/// 誠實的 host 最多只能回放事件日誌環裡的東西，所以這個數字就是 [`EVENT_LOG_RING`]：
/// 更大代表對方送來的不是它自己日誌裡的東西，更小則會讓接收端把合法回覆截斷成
/// 「我以為我追上了」。超過上限**不得靜默截斷**：接收端改走 realign（再要一次權威讀取）。
pub const MAX_RESUME_PATCHES: usize = EVENT_LOG_RING;

/// 連續要求重新對齊（realign）的上限；達到就是 unrecoverable，不再自動重試。
///
/// realign 的效果是「再打一次 resume／權威讀取」。host 送來的東西一直對不上時
/// （snapshot 自己的 hash 就錯、epoch 每次都不同），沒有上限就是一個打不完的請求迴圈；
/// 達上限要照實說「狀態未知」，不得繼續假裝正在同步。任一次成功套用（apply／reset／
/// recover）清零。
pub const MAX_REALIGN_ATTEMPTS: u32 = 3;

/// `MAX_RESUME_PATCHES` 與事件日誌環必須是同一個數字（上面那段文字不能只靠註解提醒）。
const _: () = assert!(
    MAX_RESUME_PATCHES == EVENT_LOG_RING,
    "resume 回覆的上界就是 host 事件日誌環的大小"
);
