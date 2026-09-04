//! §2 Message types 與 name 命名空間。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 十二種 message type。未知值反序列化成 [`MessageType::Unknown`]，**不得執行**（§4.1）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MessageType {
    Event,
    Command,
    Query,
    Response,
    Result,
    State,
    Cancel,
    ApprovalRequest,
    ApprovalResult,
    Error,
    Heartbeat,
    Capability,
    /// 本實作不認得的 type：保留原字串供 error／稽核，永不執行。
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(String),
}

impl MessageType {
    /// 全部已知 type（固定順序，供 schema／文件／測試列舉）。
    pub const KNOWN: [MessageType; 12] = [
        MessageType::Event,
        MessageType::Command,
        MessageType::Query,
        MessageType::Response,
        MessageType::Result,
        MessageType::State,
        MessageType::Cancel,
        MessageType::ApprovalRequest,
        MessageType::ApprovalResult,
        MessageType::Error,
        MessageType::Heartbeat,
        MessageType::Capability,
    ];

    pub fn as_str(&self) -> &str {
        match self {
            MessageType::Event => "event",
            MessageType::Command => "command",
            MessageType::Query => "query",
            MessageType::Response => "response",
            MessageType::Result => "result",
            MessageType::State => "state",
            MessageType::Cancel => "cancel",
            MessageType::ApprovalRequest => "approval-request",
            MessageType::ApprovalResult => "approval-result",
            MessageType::Error => "error",
            MessageType::Heartbeat => "heartbeat",
            MessageType::Capability => "capability",
            MessageType::Unknown(raw) => raw.as_str(),
        }
    }

    pub fn is_known(&self) -> bool {
        !matches!(self, MessageType::Unknown(_))
    }
}

/// 訊息／能力的參與方種類。未知值 → [`PartyKind::Unknown`]（保留原字串）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PartyKind {
    /// Runtime 本體（唯一可送 `task.*`／`runtime.*` 的來源）。
    Runtime,
    /// Character Session（作為 target）。
    Session,
    /// 已配對裝置（iPhone 等）。
    Device,
    /// Renderer（桌面視窗內建 adapter、外部 adapter）。
    Renderer,
    /// 可信人類操作面（桌面控制中心）。
    HumanSurface,
    /// 人類本人（approval 的 target）。
    Human,
    /// AI agent／session。
    Agent,
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(String),
}

/// 參與方參照。**只是宣稱**；可信身分由 Transport 綁定後比對（§5）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Party {
    pub kind: PartyKind,
    pub id: String,
}

impl Party {
    pub fn new(kind: PartyKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
    pub fn runtime() -> Self {
        Self::new(PartyKind::Runtime, "runtime")
    }
    pub fn session(id: impl Into<String>) -> Self {
        Self::new(PartyKind::Session, id)
    }
    pub fn device(id: impl Into<String>) -> Self {
        Self::new(PartyKind::Device, id)
    }
    pub fn renderer(id: impl Into<String>) -> Self {
        Self::new(PartyKind::Renderer, id)
    }
    pub fn human_surface(id: impl Into<String>) -> Self {
        Self::new(PartyKind::HumanSurface, id)
    }
}

/// §2.3 1.0 保留的 name 命名空間前綴。
pub const NAME_NAMESPACES: [&str; 6] = [
    "character.interaction.",
    "character.behavior.",
    "character.session.",
    "task.",
    "runtime.",
    "device.",
];

/// name 語法：`^[a-z][a-z0-9]*(\.[a-z][a-z0-9-]*)+$`，≤ [`crate::limits::MAX_NAME_CHARS`]。
pub fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.chars().count() > crate::limits::MAX_NAME_CHARS {
        return false;
    }
    let mut segments = name.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    let head_ok = |s: &str| {
        let mut chars = s.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    };
    if !head_ok(first) {
        return false;
    }
    let mut count = 0;
    for seg in segments {
        count += 1;
        let mut chars = seg.chars();
        let ok = matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return false;
        }
    }
    count >= 1
}

/// 只有 Runtime 可以送出的 name 前綴（device／renderer 送來一律 `scope-denied`）。
pub fn is_runtime_only_name(name: &str) -> bool {
    name.starts_with("task.") || name.starts_with("runtime.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_message_type_round_trips_and_is_never_known() {
        let v: MessageType = serde_json::from_str("\"teleport\"").unwrap();
        assert_eq!(v, MessageType::Unknown("teleport".into()));
        assert!(!v.is_known());
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"teleport\"");
        for known in MessageType::KNOWN {
            let s = serde_json::to_string(&known).unwrap();
            let back: MessageType = serde_json::from_str(&s).unwrap();
            assert_eq!(back, known);
            assert!(back.is_known());
        }
    }

    #[test]
    fn name_grammar() {
        assert!(is_valid_name("character.interaction.touch"));
        assert!(is_valid_name("task.verified"));
        assert!(is_valid_name("character.session.resume-now"));
        assert!(!is_valid_name("Character.touch"));
        assert!(!is_valid_name("touch"));
        assert!(!is_valid_name("a..b"));
        assert!(!is_valid_name("a.-b"));
        assert!(!is_valid_name(&format!("a.{}", "b".repeat(200))));
        assert!(is_runtime_only_name("task.state"));
        assert!(!is_runtime_only_name("character.interaction.touch"));
    }
}
