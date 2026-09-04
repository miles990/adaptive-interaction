//! interaction-session：**權威 Character Session** 的純領域實作。
//!
//! 契約：`docs/aip/character-session.md`（envelope 與版本規則見 `docs/aip/README.md`）。
//! 這個 crate 沒有 I/O、沒有 tokio、沒有 transport：時間由呼叫端注入，所有規則都可確定性測試
//! （`docs/aip/architecture-boundaries.md` §1 的依賴方向）。
//!
//! | 模組 | 規格章節 |
//! |---|---|
//! | [`state`]（re-export） | §3 語意狀態 |
//! | [`director`] | §4 語意事件目錄（純函式） |
//! | [`cpp`] | §5 Behavior Intent → CPP 投影 |
//! | [`patch`]（re-export） | §6 revision／snapshot／patch／replay 的純函式 |
//! | [`ports`] | architecture-boundaries §2 Ports |
//! | [`CharacterSession`] | §1／§2／§8 權威 host：安全管線、membership、sequence |
//!
//! # 不變量
//!
//! - `SemanticState` 的欄位是 `pub(crate)`：只有 [`CharacterSession`] 能改，port 沒有 setter。
//! - `truth` 只轉錄 Runtime 的真相；`verified` 只能經 [`CharacterSession::submit_runtime`] 產生，
//!   device／renderer 送 `result{status:"verified"}` 一律 `rejected{scope-denied}`。
//! - 每則外部訊息只回**一則** result；重複 messageId 回 `accepted{duplicate:true}`，不重套用。
//! - 所有集合有界：成員 ≤ `max_members`、事件日誌 ≤ `event_log_cap`、去重環 ≤ 256、
//!   pending intent ≤ [`MAX_PENDING_INTENTS`]、counters 的鍵來自固定的錯誤碼集合。
//! - 錯誤訊息不回顯輸入內容、不含路徑。
//!
//! # 與契約的落差（實作註記）
//!
//! 這些是 `docs/aip/character-session.md` 沒有寫死、由本實作補齊的細節，同一段註記也回填到文件：
//!
//! 1. 「無」的選填鍵**省略**而不是寫 `null`（RFC 7396 的 `null` 是刪除語意，寫 `null` 會讓兩端 hash 分歧）。
//! 2. `attention.id` 與 `lastInteraction.source` 用 `"<kind>:<id>"` 字串（§3 範例的形狀），
//!    Rust 型別仍是 `Party`。
//! 3. capability 宣告的 `inputs` 只約束 `event`；`heartbeat`／`capability`／`query`／`cancel`／`result`
//!    的 name 不受 inputs 限制（`inputs` 的定義是「可產生的 event name」）。
//! 4. `task.state{truth:"verified"}` 只轉錄真相，**不**產生 `celebrate`；慶祝只由 `task.verified` 產生。
//! 5. `attention` 的擁有者是 Director：touch → `member`、`task.*` 帶 correlation → `task`、
//!    dismiss／emergency → `none`。
//! 6. Host 送出的 messageId 形如 `aip-<epoch>-<epochMillis>-<n>`。

pub mod cpp;
pub mod director;
mod patch;
pub mod ports;
mod session;
mod state;
mod types;

pub use cpp::{behavior_to_cpp, CppProjection};
pub use patch::{
    accept_state, accept_state_with_epoch, apply_patch, merge_diff, state_hash, IgnoreReason,
    StateDecision,
};
pub use session::{
    CharacterSession, Diagnostics, JoinOutcome, LogEntry, Output, Resume, Submission,
};
pub use state::{
    format_party, parse_party, Activity, Attention, LastInteraction, Member, MemberView, Mood,
    MoodKind, Presence, SemanticState, TruthView,
};
pub use types::{
    BehaviorIntent, IntentOrigin, RuntimeFact, SessionConfig, SessionError, Snapshot,
    EVENT_DISMISS, EVENT_TOUCH, HOST_INPUTS, HOST_INTENTS, INTENT_CELEBRATE, INTENT_IDLE,
    INTENT_REACT_HAPPILY_TO_TOUCH, INTENT_SETTLE, MAX_PENDING_INTENTS, NAME_BEHAVIOR_REQUEST,
    NAME_SESSION_CAPABILITY, NAME_SESSION_PATCH, NAME_SESSION_RESULT, NAME_SESSION_SNAPSHOT,
    REASON_SESSION_RESET,
};

/// 本 crate 實作的 AIP profile 名稱（`capability.profiles`）。
pub const PROFILE: &str = "character-session";
