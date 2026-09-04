//! Session 的組態、Behavior Intent、Runtime 真相事實、Snapshot 與錯誤型別。

use interaction_aip::{limits, AipError, ErrorCode, Timestamp};
use interaction_character::TruthState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 1.0 的 Behavior Intent 詞彙（`docs/aip/character-session.md` §5）。
pub const INTENT_REACT_HAPPILY_TO_TOUCH: &str = "react-happily-to-touch";
pub const INTENT_CELEBRATE: &str = "celebrate";
pub const INTENT_SETTLE: &str = "settle";
pub const INTENT_IDLE: &str = "idle";
/// Host 願意請求的 intent，固定順序（協商時逐一比對）。
pub const HOST_INTENTS: [&str; 4] = [
    INTENT_REACT_HAPPILY_TO_TOUCH,
    INTENT_CELEBRATE,
    INTENT_SETTLE,
    INTENT_IDLE,
];

/// 1.0 Host 接受的 event name（§4 語意事件目錄裡 member 可送的兩個）。
pub const EVENT_TOUCH: &str = "character.interaction.touch";
pub const EVENT_DISMISS: &str = "character.interaction.dismiss";
pub const HOST_INPUTS: [&str; 2] = [EVENT_TOUCH, EVENT_DISMISS];

/// Host 送出的訊息 name。
pub const NAME_BEHAVIOR_REQUEST: &str = "character.behavior.request";
pub const NAME_SESSION_SNAPSHOT: &str = "character.session.snapshot";
pub const NAME_SESSION_PATCH: &str = "character.session.patch";
pub const NAME_SESSION_CAPABILITY: &str = "character.session.capability";
pub const NAME_SESSION_RESULT: &str = "character.session.result";
/// `payload.reason`：host 重建了 session，接收端必須丟棄本地狀態。
pub const REASON_SESSION_RESET: &str = "session-reset";

/// pending Behavior Intent 的上限（有界；滿了淘汰最舊並稽核）。
pub const MAX_PENDING_INTENTS: usize = 16;

/// Session 組態。時間全部以毫秒表示，由呼叫端注入 `now`，本 crate 不讀時鐘。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    pub session_id: String,
    pub character_id: String,
    /// 事件日誌（delta replay）環大小。
    pub event_log_cap: usize,
    pub max_members: usize,
    /// `activity: reacting` 自動回到 `idle` 的時間。
    pub reaction_ms: i64,
    /// Host 自行合成互動事件時使用的 deadline 長度（§7 建議 5 s）。
    pub touch_ttl_ms: i64,
    /// Behavior Intent 的 deadline 長度（§7 建議 ≤ 10 s）。
    pub intent_ttl_ms: i64,
    /// 多久沒有 heartbeat／presence 就視為 offline。
    pub presence_timeout_ms: i64,
    /// 每個 member 每秒可送的訊息數（token bucket）。
    pub rate_limit_per_sec: u32,
    /// 每累積多少個 revision 就建議 host 持久化一次 snapshot（§6 預設 32）。
    pub persist_every_revisions: u64,
    /// 距離上次持久化多久就再建議一次（§6 預設 60 s）。
    pub persist_interval_ms: i64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            session_id: "session.home".to_string(),
            // 核心不得綁定任何具體角色（`docs/aip/architecture-boundaries.md` §4）：
            // 真正的 characterId 由 host 注入。
            character_id: "character".to_string(),
            event_log_cap: limits::EVENT_LOG_RING,
            max_members: limits::MAX_MEMBERS,
            reaction_ms: 3_000,
            touch_ttl_ms: limits::DEFAULT_INTERACTION_TTL_MS,
            intent_ttl_ms: limits::DEFAULT_INTENT_TTL_MS,
            presence_timeout_ms: 45_000,
            rate_limit_per_sec: 30,
            persist_every_revisions: 32,
            persist_interval_ms: 60_000,
        }
    }
}

impl SessionConfig {
    /// 夾住所有上限，避免 host 傳進 0 或超過 AIP limits 的值造成無界集合。
    pub(crate) fn normalized(mut self) -> Self {
        self.event_log_cap = self.event_log_cap.clamp(1, limits::EVENT_LOG_RING);
        self.max_members = self.max_members.clamp(1, limits::MAX_MEMBERS);
        self.reaction_ms = self.reaction_ms.clamp(0, 600_000);
        self.touch_ttl_ms = self.touch_ttl_ms.clamp(1, 600_000);
        self.intent_ttl_ms = self.intent_ttl_ms.clamp(1, 600_000);
        self.presence_timeout_ms = self.presence_timeout_ms.clamp(1, 3_600_000);
        self.rate_limit_per_sec = self.rate_limit_per_sec.clamp(1, 1_000);
        self.persist_every_revisions = self.persist_every_revisions.clamp(1, 10_000);
        self.persist_interval_ms = self.persist_interval_ms.clamp(1, 3_600_000);
        self
    }
}

/// Behavior Intent 的來源分類（§5 `origin`）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum IntentOrigin {
    Interaction,
    Truth,
    Ambient,
}

impl IntentOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            IntentOrigin::Interaction => "interaction",
            IntentOrigin::Truth => "truth",
            IntentOrigin::Ambient => "ambient",
        }
    }
}

/// §5 Behavior Intent：`command{name:"character.behavior.request"}` 的語意內容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorIntent {
    pub intent: String,
    pub intensity: f64,
    pub interruptible: bool,
    pub origin: IntentOrigin,
    pub hints: Map<String, Value>,
    pub correlation_id: String,
    pub expires_at: Timestamp,
}

impl BehaviorIntent {
    /// `command` 的 payload（§5）。`correlationId`／`expiresAt` 屬 envelope，不重複進 payload。
    pub fn payload(&self) -> Value {
        let mut map = Map::new();
        map.insert("intent".into(), Value::String(self.intent.clone()));
        map.insert(
            "intensity".into(),
            serde_json::Number::from_f64(crate::state::clamp_unit(self.intensity))
                .map(Value::Number)
                .unwrap_or(Value::Null),
        );
        map.insert("interruptible".into(), Value::Bool(self.interruptible));
        map.insert(
            "origin".into(),
            Value::String(self.origin.as_str().to_string()),
        );
        map.insert("hints".into(), Value::Object(self.hints.clone()));
        Value::Object(map)
    }
}

/// Runtime 的可信真相事實（§4 `task.*`／`runtime.*`）。只能經 `submit_runtime` 進入 Session。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RuntimeFact {
    /// `task.state`：Session 只轉錄真相，不推論。
    TaskState {
        truth: TruthState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        correlation_id: Option<String>,
    },
    /// `task.verified`：**只有** Runtime 的人類驗證路徑能產生。
    TaskVerified { correlation_id: String },
    /// `runtime.emergency`。
    Emergency { engaged: bool },
    /// 使用者偏好：減少動態效果。
    ReducedMotion(bool),
}

/// 權威狀態快照。`hash` = `state_hash(state)`；`state` 就是 [`crate::SemanticState`] 的 serde JSON。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub session_id: String,
    pub epoch: u64,
    pub revision: u64,
    pub sequence: u64,
    pub state: Value,
    pub hash: String,
    pub at: Timestamp,
}

/// Session 層錯誤。訊息不回顯輸入內容、不含路徑（§5）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("session already has the maximum number of members")]
    MembersFull,
    #[error("snapshot hash does not match its state")]
    HashMismatch,
    #[error("snapshot state is not a valid semantic state")]
    InvalidState,
    #[error("snapshot belongs to a different session")]
    SessionMismatch,
    #[error("capability negotiation failed: {0}")]
    Negotiation(AipError),
}

impl SessionError {
    /// 對應的穩定錯誤碼（§12）。
    pub fn code(&self) -> ErrorCode {
        match self {
            SessionError::MembersFull => ErrorCode::ScopeDenied,
            SessionError::HashMismatch | SessionError::InvalidState => ErrorCode::SchemaInvalid,
            SessionError::SessionMismatch => ErrorCode::SessionNotFound,
            SessionError::Negotiation(e) => e.code.clone(),
        }
    }
}

impl From<AipError> for SessionError {
    fn from(value: AipError) -> Self {
        SessionError::Negotiation(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_aip_limits() {
        let c = SessionConfig::default();
        assert_eq!(c.event_log_cap, limits::EVENT_LOG_RING);
        assert_eq!(c.max_members, limits::MAX_MEMBERS);
        assert_eq!(c.reaction_ms, 3_000);
        assert_eq!(c.touch_ttl_ms, limits::DEFAULT_INTERACTION_TTL_MS);
        assert_eq!(c.intent_ttl_ms, limits::DEFAULT_INTENT_TTL_MS);
        assert_eq!(c.presence_timeout_ms, 45_000);
        assert!(
            !c.character_id.contains("shu") && !c.character_id.contains("maid"),
            "核心不得綁定 reference character"
        );
    }

    #[test]
    fn config_is_clamped_to_bounded_values() {
        let c = SessionConfig {
            event_log_cap: usize::MAX,
            max_members: 9_999,
            rate_limit_per_sec: 0,
            reaction_ms: -5,
            ..SessionConfig::default()
        }
        .normalized();
        assert_eq!(c.event_log_cap, limits::EVENT_LOG_RING);
        assert_eq!(c.max_members, limits::MAX_MEMBERS);
        assert_eq!(c.rate_limit_per_sec, 1);
        assert_eq!(c.reaction_ms, 0);
    }
}
