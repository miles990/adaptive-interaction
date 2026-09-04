//! §1 Envelope、§2.2 profiles、§5 身分綁定決策、§7 deadline／去重。

use std::collections::{BTreeSet, VecDeque};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{is_valid_name, limits, AipError, ErrorCode, MessageType, Party, PartyKind, Timestamp};

/// AIP 1.0 訊息信封。未知頂層選填欄位保留在 `extra`（round-trip 不遺失、不執行）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub spec_version: String,
    pub message_id: String,
    pub message_type: MessageType,
    pub name: String,
    pub source: Party,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Party>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub occurred_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent_grant_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(skip)]
    pub extra: serde_json::Map<String, Value>,
}

impl Envelope {
    /// 建立一則本實作版本的訊息（payload 預設空物件）。
    pub fn new(
        message_type: MessageType,
        name: impl Into<String>,
        source: Party,
        message_id: impl Into<String>,
        occurred_at: Timestamp,
    ) -> Self {
        Self {
            spec_version: crate::SPEC_VERSION.to_string(),
            message_id: message_id.into(),
            message_type,
            name: name.into(),
            source,
            target: None,
            session_id: None,
            occurred_at,
            correlation_id: None,
            causation_id: None,
            sequence: None,
            base_revision: None,
            expires_at: None,
            consent_grant_id: None,
            payload: Value::Object(serde_json::Map::new()),
            extra: serde_json::Map::new(),
        }
    }

    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }
    pub fn with_target(mut self, target: Party) -> Self {
        self.target = Some(target);
        self
    }
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }
    pub fn with_correlation(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }
    pub fn with_causation(mut self, id: impl Into<String>) -> Self {
        self.causation_id = Some(id.into());
        self
    }
    pub fn with_expiry(mut self, at: Timestamp) -> Self {
        self.expires_at = Some(at);
        self
    }
    pub fn with_sequence(mut self, seq: u64) -> Self {
        self.sequence = Some(seq);
        self
    }
    pub fn with_base_revision(mut self, rev: u64) -> Self {
        self.base_revision = Some(rev);
        self
    }

    /// §7：`expiresAt` 已過（含等於）→ 過期。沒有 expiresAt → 不過期。
    pub fn is_expired(&self, now: Timestamp) -> bool {
        self.expires_at.is_some_and(|at| at <= now)
    }

    /// 解析 bytes → Envelope，先量大小（§11）。不做 profile 驗證（見 [`Envelope::validate`]）。
    pub fn parse(bytes: &[u8]) -> Result<Envelope, AipError> {
        if bytes.len() > limits::MAX_MESSAGE_BYTES {
            return Err(AipError::new(
                ErrorCode::MessageTooLarge,
                format!("message exceeds {} bytes", limits::MAX_MESSAGE_BYTES),
            ));
        }
        serde_json::from_slice::<Envelope>(bytes)
            .map_err(|e| AipError::new(ErrorCode::SchemaInvalid, sanitize_serde_error(&e)))
    }

    /// 序列化並確認不超過上限。
    pub fn encode(&self) -> Result<Vec<u8>, AipError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|_| AipError::new(ErrorCode::Internal, "envelope encode failed"))?;
        if bytes.len() > limits::MAX_MESSAGE_BYTES {
            return Err(AipError::new(
                ErrorCode::MessageTooLarge,
                format!("message exceeds {} bytes", limits::MAX_MESSAGE_BYTES),
            ));
        }
        Ok(bytes)
    }

    /// §2.2 profile 驗證＋§11 上限＋§4 版本＋name 語法。順序固定；第一個失敗即回。
    pub fn validate(&self) -> Result<(), AipError> {
        crate::negotiate_version(&self.spec_version)?;
        if !self.message_type.is_known() {
            return Err(AipError::new(
                ErrorCode::UnsupportedMessageType,
                format!("unknown messageType {}", self.message_type.as_str()),
            ));
        }
        check_id("messageId", &self.message_id)?;
        if !is_valid_name(&self.name) {
            return Err(AipError::new(
                ErrorCode::SchemaInvalid,
                "name violates grammar",
            ));
        }
        check_id("source.id", &self.source.id)?;
        if matches!(self.source.kind, PartyKind::Unknown(_)) {
            return Err(AipError::new(
                ErrorCode::SchemaInvalid,
                "source.kind unknown",
            ));
        }
        if let Some(t) = &self.target {
            check_id("target.id", &t.id)?;
        }
        for (label, v) in [
            ("sessionId", &self.session_id),
            ("correlationId", &self.correlation_id),
            ("causationId", &self.causation_id),
            ("consentGrantId", &self.consent_grant_id),
        ] {
            if let Some(v) = v {
                check_id(label, v)?;
            }
        }
        check_payload(&self.payload)?;
        let need = |cond: bool, what: &str| {
            if cond {
                Ok(())
            } else {
                Err(AipError::new(
                    ErrorCode::SchemaInvalid,
                    format!("{} requires {what}", self.message_type.as_str()),
                ))
            }
        };
        match self.message_type {
            MessageType::Event => {
                need(self.session_id.is_some(), "sessionId")?;
                if self.name.starts_with("character.interaction.") {
                    need(
                        self.expires_at.is_some(),
                        "expiresAt for interaction events",
                    )?;
                }
            }
            MessageType::Command => {
                need(self.session_id.is_some(), "sessionId")?;
                need(self.target.is_some(), "target")?;
                need(self.correlation_id.is_some(), "correlationId")?;
                need(self.expires_at.is_some(), "expiresAt")?;
            }
            MessageType::Query => need(self.target.is_some(), "target")?,
            MessageType::Response => need(self.causation_id.is_some(), "causationId")?,
            MessageType::Result => {
                need(self.causation_id.is_some(), "causationId")?;
                let status = self.payload.get("status").and_then(Value::as_str);
                need(status.is_some(), "payload.status")?;
                let parsed: Option<crate::Outcome> =
                    status.and_then(|s| serde_json::from_value(Value::String(s.to_string())).ok());
                need(parsed.is_some(), "a known payload.status")?;
            }
            MessageType::State => {
                need(self.session_id.is_some(), "sessionId")?;
                need(self.sequence.is_some(), "sequence")?;
                need(
                    self.payload
                        .get("revision")
                        .and_then(Value::as_u64)
                        .is_some(),
                    "payload.revision",
                )?;
                if self.payload.get("kind").and_then(Value::as_str) == Some("patch") {
                    need(self.base_revision.is_some(), "baseRevision for patches")?;
                }
            }
            MessageType::Cancel => need(
                self.causation_id.is_some()
                    || self
                        .payload
                        .get("messageId")
                        .and_then(Value::as_str)
                        .is_some(),
                "causationId or payload.messageId",
            )?,
            MessageType::ApprovalRequest => {
                need(self.correlation_id.is_some(), "correlationId")?;
                need(self.expires_at.is_some(), "expiresAt")?;
                need(
                    matches!(&self.target, Some(t) if t.kind == PartyKind::Human),
                    "target{kind:human}",
                )?;
            }
            MessageType::ApprovalResult => need(self.causation_id.is_some(), "causationId")?,
            MessageType::Error => {
                need(
                    self.payload.get("code").and_then(Value::as_str).is_some(),
                    "payload.code",
                )?;
            }
            MessageType::Heartbeat | MessageType::Capability => {}
            MessageType::Unknown(_) => unreachable!("rejected above"),
        }
        Ok(())
    }
}

fn check_id(label: &str, value: &str) -> Result<(), AipError> {
    if value.is_empty() || value.chars().count() > limits::MAX_ID_CHARS {
        return Err(AipError::new(
            ErrorCode::SchemaInvalid,
            format!("{label} must be 1..={} chars", limits::MAX_ID_CHARS),
        ));
    }
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(AipError::new(
            ErrorCode::SchemaInvalid,
            format!("{label} contains whitespace or control chars"),
        ));
    }
    Ok(())
}

/// payload：大小、深度、字串長度（§11）。
pub fn check_payload(payload: &Value) -> Result<(), AipError> {
    let bytes = serde_json::to_vec(payload)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if bytes > limits::MAX_PAYLOAD_BYTES {
        return Err(AipError::new(
            ErrorCode::PayloadTooLarge,
            format!("payload exceeds {} bytes", limits::MAX_PAYLOAD_BYTES),
        ));
    }
    fn walk(v: &Value, depth: usize) -> Result<(), AipError> {
        if depth > limits::MAX_JSON_DEPTH {
            return Err(AipError::new(
                ErrorCode::SchemaInvalid,
                "payload nesting too deep",
            ));
        }
        match v {
            Value::String(s) if s.chars().count() > limits::MAX_STRING_CHARS => Err(AipError::new(
                ErrorCode::SchemaInvalid,
                format!("payload string exceeds {} chars", limits::MAX_STRING_CHARS),
            )),
            Value::Array(items) => items.iter().try_for_each(|i| walk(i, depth + 1)),
            Value::Object(map) => map.values().try_for_each(|i| walk(i, depth + 1)),
            _ => Ok(()),
        }
    }
    walk(payload, 1)
}

fn sanitize_serde_error(e: &serde_json::Error) -> String {
    // 只回類別與位置，不回顯內容（§5）。
    let kind = if e.is_syntax() {
        "syntax"
    } else if e.is_data() {
        "data"
    } else if e.is_eof() {
        "eof"
    } else {
        "io"
    };
    format!(
        "invalid envelope ({kind} at line {} column {})",
        e.line(),
        e.column()
    )
}

/// §5 身分綁定決策：Transport 驗證出的身分 vs `source` 宣稱。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityDecision {
    /// 宣稱與綁定身分相符。
    Accept,
    /// 不符：拒絕並稽核；**不得**修正後執行。
    Reject { bound: Party, claimed: Party },
}

pub fn bind_identity(bound: &Party, claimed: &Party) -> IdentityDecision {
    if bound == claimed {
        IdentityDecision::Accept
    } else {
        IdentityDecision::Reject {
            bound: bound.clone(),
            claimed: claimed.clone(),
        }
    }
}

/// §7 有界去重環（每個 (session, source) 一份）。滿了淘汰最舊。
#[derive(Debug, Clone)]
pub struct DedupeRing {
    cap: usize,
    order: VecDeque<String>,
    set: BTreeSet<String>,
}

impl Default for DedupeRing {
    fn default() -> Self {
        Self::new(limits::DEDUPE_RING)
    }
}

impl DedupeRing {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.clamp(1, limits::DEDUPE_RING),
            order: VecDeque::new(),
            set: BTreeSet::new(),
        }
    }

    /// 回 `true` 表示第一次看到（記下）；`false` 表示重複。
    pub fn note(&mut self, message_id: &str) -> bool {
        if self.set.contains(message_id) {
            return false;
        }
        if self.order.len() >= self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        self.order.push_back(message_id.to_string());
        self.set.insert(message_id.to_string());
        true
    }

    pub fn contains(&self, message_id: &str) -> bool {
        self.set.contains(message_id)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn t0() -> Timestamp {
        Utc.with_ymd_and_hms(2026, 9, 4, 12, 30, 0).unwrap()
    }

    fn touch() -> Envelope {
        Envelope::new(
            MessageType::Event,
            "character.interaction.touch",
            Party::device("iphone-1"),
            "msg_1",
            t0(),
        )
        .with_session("session.home")
        .with_expiry(t0() + chrono::Duration::seconds(5))
        .with_payload(json!({"kind": "tap"}))
    }

    #[test]
    fn valid_event_round_trips_with_unknown_optional_fields() {
        let mut e = touch();
        e.extra.insert("futureField".into(), json!({"keep": true}));
        e.validate().unwrap();
        let bytes = e.encode().unwrap();
        let back = Envelope::parse(&bytes).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.extra["futureField"]["keep"], true);
    }

    #[test]
    fn profiles_enforce_required_fields() {
        let mut e = touch();
        e.session_id = None;
        assert_eq!(e.validate().unwrap_err().code, ErrorCode::SchemaInvalid);
        let mut e = touch();
        e.expires_at = None;
        assert!(
            e.validate().is_err(),
            "interaction events must carry a deadline"
        );
        let cmd = Envelope::new(
            MessageType::Command,
            "character.behavior.request",
            Party::runtime(),
            "c1",
            t0(),
        )
        .with_session("session.home");
        assert!(cmd.validate().is_err());
        let ok = cmd
            .with_target(Party::device("iphone-1"))
            .with_correlation("flow_1")
            .with_expiry(t0() + chrono::Duration::seconds(10));
        ok.validate().unwrap();
        let res = Envelope::new(
            MessageType::Result,
            "character.interaction.touch",
            Party::runtime(),
            "r1",
            t0(),
        )
        .with_causation("msg_1")
        .with_payload(json!({"status": "teleported"}));
        assert!(res.validate().is_err(), "unknown outcome value rejected");
        let res = res.with_payload(json!({"status": "accepted"}));
        res.validate().unwrap();
    }

    #[test]
    fn unknown_message_type_is_rejected_not_executed() {
        let raw = json!({
            "specVersion": "aip/1.0", "messageId": "m", "messageType": "teleport", "name": "a.b",
            "source": {"kind": "device", "id": "d"}, "occurredAt": "2026-09-04T12:30:00Z", "payload": {}
        });
        let e = Envelope::parse(serde_json::to_vec(&raw).unwrap().as_slice()).unwrap();
        assert_eq!(
            e.validate().unwrap_err().code,
            ErrorCode::UnsupportedMessageType
        );
    }

    #[test]
    fn version_major_mismatch_and_sizes() {
        let mut e = touch();
        e.spec_version = "aip/2.0".into();
        assert_eq!(
            e.validate().unwrap_err().code,
            ErrorCode::UnsupportedVersion
        );
        let mut e = touch();
        e.spec_version = "aip/1.9".into();
        e.validate().unwrap();
        let big = touch().with_payload(json!({"blob": "x".repeat(limits::MAX_PAYLOAD_BYTES)}));
        assert_eq!(big.validate().unwrap_err().code, ErrorCode::PayloadTooLarge);
        let deep =
            touch().with_payload(json!({"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"i":1}}}}}}}}}));
        assert_eq!(deep.validate().unwrap_err().code, ErrorCode::SchemaInvalid);
        let huge = vec![b' '; limits::MAX_MESSAGE_BYTES + 1];
        assert_eq!(
            Envelope::parse(&huge).unwrap_err().code,
            ErrorCode::MessageTooLarge
        );
        let err = Envelope::parse(b"{\"secret\":\"do-not-echo\"").unwrap_err();
        assert!(!err.message.contains("do-not-echo"));
    }

    #[test]
    fn deadline_identity_and_dedupe() {
        let e = touch();
        assert!(!e.is_expired(t0()));
        assert!(e.is_expired(t0() + chrono::Duration::seconds(5)));
        assert_eq!(
            bind_identity(&Party::device("iphone-1"), &e.source),
            IdentityDecision::Accept
        );
        assert!(matches!(
            bind_identity(&Party::device("iphone-2"), &e.source),
            IdentityDecision::Reject { .. }
        ));
        assert!(matches!(
            bind_identity(&Party::renderer("iphone-1"), &e.source),
            IdentityDecision::Reject { .. }
        ));
        let mut ring = DedupeRing::new(2);
        assert!(ring.note("a"));
        assert!(!ring.note("a"));
        assert!(ring.note("b"));
        assert!(ring.note("c"));
        assert!(!ring.contains("a"), "oldest evicted");
        assert_eq!(ring.len(), 2);
    }
}
