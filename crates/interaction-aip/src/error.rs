//! §12 穩定錯誤碼與 `error` payload。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 穩定錯誤碼；1.x 內只增不改。未知值保留原字串。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    SchemaInvalid,
    UnsupportedVersion,
    UnsupportedMessageType,
    UnknownName,
    UnsupportedCapability,
    PayloadTooLarge,
    MessageTooLarge,
    Expired,
    Duplicate,
    RevisionMismatch,
    SequenceGap,
    IdentityMismatch,
    NotAMember,
    ScopeDenied,
    RateLimited,
    SessionNotFound,
    SessionDisabled,
    Cancelled,
    Internal,
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(String),
}

impl ErrorCode {
    pub const KNOWN: [ErrorCode; 19] = [
        ErrorCode::SchemaInvalid,
        ErrorCode::UnsupportedVersion,
        ErrorCode::UnsupportedMessageType,
        ErrorCode::UnknownName,
        ErrorCode::UnsupportedCapability,
        ErrorCode::PayloadTooLarge,
        ErrorCode::MessageTooLarge,
        ErrorCode::Expired,
        ErrorCode::Duplicate,
        ErrorCode::RevisionMismatch,
        ErrorCode::SequenceGap,
        ErrorCode::IdentityMismatch,
        ErrorCode::NotAMember,
        ErrorCode::ScopeDenied,
        ErrorCode::RateLimited,
        ErrorCode::SessionNotFound,
        ErrorCode::SessionDisabled,
        ErrorCode::Cancelled,
        ErrorCode::Internal,
    ];

    pub fn as_str(&self) -> &str {
        match self {
            ErrorCode::SchemaInvalid => "schema-invalid",
            ErrorCode::UnsupportedVersion => "unsupported-version",
            ErrorCode::UnsupportedMessageType => "unsupported-message-type",
            ErrorCode::UnknownName => "unknown-name",
            ErrorCode::UnsupportedCapability => "unsupported-capability",
            ErrorCode::PayloadTooLarge => "payload-too-large",
            ErrorCode::MessageTooLarge => "message-too-large",
            ErrorCode::Expired => "expired",
            ErrorCode::Duplicate => "duplicate",
            ErrorCode::RevisionMismatch => "revision-mismatch",
            ErrorCode::SequenceGap => "sequence-gap",
            ErrorCode::IdentityMismatch => "identity-mismatch",
            ErrorCode::NotAMember => "not-a-member",
            ErrorCode::ScopeDenied => "scope-denied",
            ErrorCode::RateLimited => "rate-limited",
            ErrorCode::SessionNotFound => "session-not-found",
            ErrorCode::SessionDisabled => "session-disabled",
            ErrorCode::Cancelled => "cancelled",
            ErrorCode::Internal => "internal",
            ErrorCode::Unknown(raw) => raw.as_str(),
        }
    }

    /// 可用**同一 messageId** 重送的錯誤（idempotent）。
    pub fn retryable(&self) -> bool {
        matches!(self, ErrorCode::RateLimited | ErrorCode::Internal)
    }
}

/// `error.payload`。`message` ≤ 200 字、不回顯輸入；`details` 不得含 secret／路徑／token。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Map<String, serde_json::Value>>,
}

impl ErrorPayload {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let retryable = code.retryable();
        let mut message: String = message.into();
        if message.chars().count() > 200 {
            message = message.chars().take(200).collect();
        }
        Self {
            code,
            message,
            retryable,
            details: None,
        }
    }
}

/// AIP 層的處理錯誤（Rust 內部用；wire 上是 [`ErrorPayload`]）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct AipError {
    pub code: ErrorCode,
    pub message: String,
}

impl AipError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn payload(&self) -> ErrorPayload {
        ErrorPayload::new(self.code.clone(), self.message.clone())
    }
}
