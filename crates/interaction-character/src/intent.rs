//! §4 Character Intent：20 個 intent、15 個 truthState、priority 下限與 Envelope。
//!
//! `truthState` 只能由 Runtime 設定；本模組的 [`IntentEnvelope`] 是唯一帶 `truthState` 的型別，
//! 而它只會由 Runtime 建構後經 Gateway 送往 adapter（adapter → runtime 方向的訊息型別都沒有它）。

use crate::{char_len, parse_protocol_version, Timestamp, PROTOCOL_MAJOR, PROTOCOL_VERSION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// §4.1 詞彙（20）。名稱在 1.x 內不變，只會新增。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CharacterIntent {
    Idle,
    Notice,
    Acknowledge,
    Think,
    Work,
    Wait,
    Ask,
    RequestConsent,
    Blocked,
    Unknown,
    ClaimCompleted,
    VerifiedSuccess,
    Failed,
    Cancelled,
    Offline,
    Emergency,
    Greet,
    Play,
    Rest,
    Sleep,
}

impl CharacterIntent {
    /// 全部 20 個 intent，固定順序（協商時逐一解析）。
    pub const ALL: [CharacterIntent; 20] = [
        CharacterIntent::Idle,
        CharacterIntent::Notice,
        CharacterIntent::Acknowledge,
        CharacterIntent::Think,
        CharacterIntent::Work,
        CharacterIntent::Wait,
        CharacterIntent::Ask,
        CharacterIntent::RequestConsent,
        CharacterIntent::Blocked,
        CharacterIntent::Unknown,
        CharacterIntent::ClaimCompleted,
        CharacterIntent::VerifiedSuccess,
        CharacterIntent::Failed,
        CharacterIntent::Cancelled,
        CharacterIntent::Offline,
        CharacterIntent::Emergency,
        CharacterIntent::Greet,
        CharacterIntent::Play,
        CharacterIntent::Rest,
        CharacterIntent::Sleep,
    ];

    /// kebab-case wire 名稱。
    pub fn as_str(&self) -> &'static str {
        match self {
            CharacterIntent::Idle => "idle",
            CharacterIntent::Notice => "notice",
            CharacterIntent::Acknowledge => "acknowledge",
            CharacterIntent::Think => "think",
            CharacterIntent::Work => "work",
            CharacterIntent::Wait => "wait",
            CharacterIntent::Ask => "ask",
            CharacterIntent::RequestConsent => "request-consent",
            CharacterIntent::Blocked => "blocked",
            CharacterIntent::Unknown => "unknown",
            CharacterIntent::ClaimCompleted => "claim-completed",
            CharacterIntent::VerifiedSuccess => "verified-success",
            CharacterIntent::Failed => "failed",
            CharacterIntent::Cancelled => "cancelled",
            CharacterIntent::Offline => "offline",
            CharacterIntent::Emergency => "emergency",
            CharacterIntent::Greet => "greet",
            CharacterIntent::Play => "play",
            CharacterIntent::Rest => "rest",
            CharacterIntent::Sleep => "sleep",
        }
    }

    /// 由 wire 名稱解析；未知名稱回 `None`（同 major 內未知 intent → `unsupported`）。
    pub fn parse(value: &str) -> Option<CharacterIntent> {
        CharacterIntent::ALL
            .iter()
            .copied()
            .find(|i| i.as_str() == value)
    }

    /// §4.3 priority 下限（Runtime 固定）。非安全 intent 下限為 0（AI 請求上限 50）。
    pub fn priority_floor(&self) -> u8 {
        priority_floor(*self)
    }

    /// §4.3 有 floor 者即安全 intent：永不遺失、零能力時走 `system.text`、可搶占非安全演出。
    pub fn is_safety(&self) -> bool {
        is_safety_intent(*self)
    }

    /// §5：`interruptible=false` 的演出只能被 floor ≥ 75 的 intent 搶占。
    pub fn preempts_non_interruptible(&self) -> bool {
        self.priority_floor() >= NON_INTERRUPTIBLE_PREEMPT_FLOOR
    }

    /// AI（`companion.state.present` 等受 policy 管制的 actuator）可請求的 intent：只有 floor ≤ 50 者。
    pub fn ai_allowed(&self) -> bool {
        self.priority_floor() <= AI_REQUEST_MAX_PRIORITY
    }

    /// 把 AI 不得直接請求的 intent 換成最接近的非安全 intent（`wait`→`think`、`ask`→`notice`、
    /// 其餘安全 intent→`notice`）。Runtime 把 AI 的 behaviorIntent 轉成 envelope 前應先呼叫。
    pub fn ai_safe_substitute(&self) -> CharacterIntent {
        if self.ai_allowed() {
            return *self;
        }
        match self {
            CharacterIntent::Wait => CharacterIntent::Think,
            _ => CharacterIntent::Notice,
        }
    }
}

impl std::fmt::Display for CharacterIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// AI 請求的 priority 上限（§4.3「AI 請求上限 50」）。
pub const AI_REQUEST_MAX_PRIORITY: u8 = 50;
/// §5：搶占 `interruptible=false` 演出所需的最低 floor。
pub const NON_INTERRUPTIBLE_PREEMPT_FLOOR: u8 = 75;

/// §4.3 priority 下限表。
pub fn priority_floor(intent: CharacterIntent) -> u8 {
    match intent {
        CharacterIntent::Emergency => 100,
        CharacterIntent::Offline => 95,
        CharacterIntent::Blocked => 90,
        CharacterIntent::Failed => 85,
        CharacterIntent::RequestConsent => 80,
        CharacterIntent::Unknown => 75,
        CharacterIntent::VerifiedSuccess => 70,
        CharacterIntent::ClaimCompleted => 65,
        CharacterIntent::Wait | CharacterIntent::Ask => 60,
        CharacterIntent::Cancelled => 55,
        CharacterIntent::Idle
        | CharacterIntent::Notice
        | CharacterIntent::Acknowledge
        | CharacterIntent::Think
        | CharacterIntent::Work
        | CharacterIntent::Greet
        | CharacterIntent::Play
        | CharacterIntent::Rest
        | CharacterIntent::Sleep => 0,
    }
}

/// 有 floor（> 50）的 intent 即安全 intent。
pub fn is_safety_intent(intent: CharacterIntent) -> bool {
    priority_floor(intent) > AI_REQUEST_MAX_PRIORITY
}

/// §4.2 truthState（15）。只由 Runtime 設定；`verified` 只能來自人類驗證路徑。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum TruthState {
    None,
    Queued,
    Working,
    WaitingInput,
    WaitingConsent,
    Blocked,
    Claimed,
    Verified,
    Failed,
    TimedOut,
    Expired,
    Unknown,
    Cancelled,
    Emergency,
    Offline,
}

impl TruthState {
    /// 全部 15 個 truthState。
    pub const ALL: [TruthState; 15] = [
        TruthState::None,
        TruthState::Queued,
        TruthState::Working,
        TruthState::WaitingInput,
        TruthState::WaitingConsent,
        TruthState::Blocked,
        TruthState::Claimed,
        TruthState::Verified,
        TruthState::Failed,
        TruthState::TimedOut,
        TruthState::Expired,
        TruthState::Unknown,
        TruthState::Cancelled,
        TruthState::Emergency,
        TruthState::Offline,
    ];
}

/// §4.4 `interruptPolicy`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum InterruptPolicy {
    Preempt,
    #[default]
    Queue,
    DropIfBusy,
    Merge,
}

/// §4.4 `resumePolicy`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ResumePolicy {
    ResumePrevious,
    ReturnIdle,
    #[default]
    #[serde(rename = "none")]
    NoResume,
}

/// §4.4／§6 `privacyClass`。
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
    Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum PrivacyClass {
    Public,
    #[default]
    Internal,
    Personal,
    Intimate,
}

/// §4.4 `durationHint`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DurationHint {
    pub ms: u64,
    #[serde(rename = "loop", default)]
    pub looped: bool,
}

/// §4.4 `presentationHints`：只是建議，adapter 可忽略。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct PresentationHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    /// ≤ 200 字。安全 intent 的固定語句由 Runtime／host 決定，不由 adapter 改寫。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, serde_json::Value>,
}

/// §4.4 Envelope：Runtime → Gateway → Adapter 的語意化 intent。**唯一**帶 `truthState` 的型別。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IntentEnvelope {
    pub protocol_version: String,
    pub message_id: String,
    pub character_instance_id: String,
    /// 串起 Agent 工作、硬體事件、receipt 與演出；安全 intent 的多實例去重也以它為鍵。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub timestamp: Timestamp,
    pub intent: CharacterIntent,
    pub truth_state: TruthState,
    /// 最終值 = max(requested, floor)；由 [`normalize_envelope`] 夾住。
    pub priority: u8,
    #[serde(default)]
    pub interrupt_policy: InterruptPolicy,
    #[serde(default)]
    pub resume_policy: ResumePolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_hint: Option<DurationHint>,
    /// ≤ 4 KB 序列化、字串 ≤ 200 字。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_hints: Option<PresentationHints>,
    #[serde(default)]
    pub privacy_class: PrivacyClass,
    /// 必填：過期不播（`expired` 回執）。
    pub expires_at: Timestamp,
}

/// `parameters` 序列化上限。
pub const MAX_PARAMETERS_BYTES: usize = 4096;
/// envelope 內任何字串值的上限（字元）。
pub const MAX_ENVELOPE_STRING_CHARS: usize = 200;
/// `durationHint.ms` 上限（§9：durationRange 上限 60 s）。
pub const MAX_DURATION_HINT_MS: u64 = 60_000;
/// JSON 巢狀深度上限（防止深度炸彈）。
pub const MAX_PARAMETER_DEPTH: usize = 8;

/// Envelope 驗證錯誤。訊息不回顯超過 200 字的內容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(tag = "code", rename_all = "kebab-case")]
pub enum EnvelopeError {
    #[error("protocol version {offered} is not compatible with {PROTOCOL_VERSION}")]
    ProtocolVersion { offered: String },
    #[error("messageId must be 1..=128 chars")]
    MessageId,
    #[error("characterInstanceId must be 1..=128 chars")]
    CharacterInstanceId,
    #[error("parameters serialize to {bytes} bytes (max {MAX_PARAMETERS_BYTES})")]
    ParametersTooLarge { bytes: usize },
    #[error("string at {path} exceeds {MAX_ENVELOPE_STRING_CHARS} chars")]
    StringTooLong { path: String },
    #[error("parameters nested deeper than {MAX_PARAMETER_DEPTH}")]
    TooDeep,
    #[error("expiresAt must be after timestamp")]
    ExpiresBeforeTimestamp,
    #[error("durationHint.ms {ms} exceeds {MAX_DURATION_HINT_MS}")]
    DurationTooLong { ms: u64 },
    #[error("intent {intent} may not be requested by an AI (floor > {AI_REQUEST_MAX_PRIORITY})")]
    AiIntentNotAllowed { intent: CharacterIntent },
    #[error("AI requests may not carry truthState other than none")]
    AiTruthState,
}

fn check_strings(value: &serde_json::Value, path: &str, depth: usize) -> Result<(), EnvelopeError> {
    if depth > MAX_PARAMETER_DEPTH {
        return Err(EnvelopeError::TooDeep);
    }
    match value {
        serde_json::Value::String(s) => {
            if char_len(s) > MAX_ENVELOPE_STRING_CHARS {
                return Err(EnvelopeError::StringTooLong {
                    path: path.to_string(),
                });
            }
        }
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                check_strings(item, &format!("{path}[{i}]"), depth + 1)?;
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if char_len(k) > MAX_ENVELOPE_STRING_CHARS {
                    return Err(EnvelopeError::StringTooLong {
                        path: format!("{path}.<key>"),
                    });
                }
                check_strings(v, &format!("{path}.{k}"), depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// 驗證並正規化 envelope：protocol major、id 長度、`parameters` ≤ 4 KB／字串 ≤ 200 字、
/// `expiresAt` 晚於 `timestamp`、`durationHint` ≤ 60 s；回傳 priority 夾成 `max(requested, floor)` 的副本。
pub fn normalize_envelope(envelope: &IntentEnvelope) -> Result<IntentEnvelope, EnvelopeError> {
    match parse_protocol_version(&envelope.protocol_version) {
        Some((major, _)) if major == PROTOCOL_MAJOR => {}
        _ => {
            return Err(EnvelopeError::ProtocolVersion {
                offered: crate::truncate_for_echo(&envelope.protocol_version),
            })
        }
    }
    let id_len = char_len(&envelope.message_id);
    if id_len == 0 || id_len > 128 {
        return Err(EnvelopeError::MessageId);
    }
    let inst_len = char_len(&envelope.character_instance_id);
    if inst_len == 0 || inst_len > 128 {
        return Err(EnvelopeError::CharacterInstanceId);
    }
    if let Some(cid) = &envelope.correlation_id {
        if char_len(cid) > MAX_ENVELOPE_STRING_CHARS {
            return Err(EnvelopeError::StringTooLong {
                path: "correlationId".into(),
            });
        }
    }
    let params = serde_json::Value::Object(
        envelope
            .parameters
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    let bytes = serde_json::to_vec(&params)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if bytes > MAX_PARAMETERS_BYTES {
        return Err(EnvelopeError::ParametersTooLarge { bytes });
    }
    check_strings(&params, "parameters", 0)?;
    if let Some(hints) = &envelope.presentation_hints {
        for (name, value) in [
            ("tone", &hints.tone),
            ("message", &hints.message),
            ("variant", &hints.variant),
        ] {
            if let Some(v) = value {
                if char_len(v) > MAX_ENVELOPE_STRING_CHARS {
                    return Err(EnvelopeError::StringTooLong {
                        path: format!("presentationHints.{name}"),
                    });
                }
            }
        }
        let channels = serde_json::Value::Object(
            hints
                .channels
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );
        check_strings(&channels, "presentationHints.channels", 0)?;
    }
    if envelope.expires_at <= envelope.timestamp {
        return Err(EnvelopeError::ExpiresBeforeTimestamp);
    }
    if let Some(hint) = &envelope.duration_hint {
        if hint.ms > MAX_DURATION_HINT_MS {
            return Err(EnvelopeError::DurationTooLong { ms: hint.ms });
        }
    }
    let mut out = envelope.clone();
    out.priority = envelope.priority.max(envelope.intent.priority_floor());
    Ok(out)
}

/// 同 [`normalize_envelope`]，但只回報是否有效（不產生副本）。
pub fn validate_envelope(envelope: &IntentEnvelope) -> Result<(), EnvelopeError> {
    normalize_envelope(envelope).map(|_| ())
}

impl IntentEnvelope {
    /// Runtime 產生 envelope 的建構子（§11 truth projection）：priority 至少為 floor。
    #[allow(clippy::too_many_arguments)]
    pub fn from_runtime(
        message_id: impl Into<String>,
        character_instance_id: impl Into<String>,
        correlation_id: Option<String>,
        intent: CharacterIntent,
        truth_state: TruthState,
        requested_priority: u8,
        timestamp: Timestamp,
        expires_at: Timestamp,
    ) -> IntentEnvelope {
        IntentEnvelope {
            protocol_version: PROTOCOL_VERSION.to_string(),
            message_id: message_id.into(),
            character_instance_id: character_instance_id.into(),
            correlation_id,
            timestamp,
            intent,
            truth_state,
            priority: requested_priority.max(intent.priority_floor()),
            interrupt_policy: if intent.is_safety() {
                InterruptPolicy::Preempt
            } else {
                InterruptPolicy::Queue
            },
            resume_policy: ResumePolicy::default(),
            duration_hint: None,
            parameters: BTreeMap::new(),
            presentation_hints: None,
            privacy_class: PrivacyClass::default(),
            expires_at,
        }
    }

    /// AI 請求（`companion.state.present`）轉成 envelope：**強制** `truthState: none`、
    /// priority ≤ 50，且只接受 floor ≤ 50 的 intent（其餘回 [`EnvelopeError::AiIntentNotAllowed`]；
    /// 呼叫端可先用 [`CharacterIntent::ai_safe_substitute`] 降級）。
    pub fn from_ai_request(
        message_id: impl Into<String>,
        character_instance_id: impl Into<String>,
        correlation_id: Option<String>,
        intent: CharacterIntent,
        requested_priority: u8,
        timestamp: Timestamp,
        expires_at: Timestamp,
    ) -> Result<IntentEnvelope, EnvelopeError> {
        if !intent.ai_allowed() {
            return Err(EnvelopeError::AiIntentNotAllowed { intent });
        }
        let mut envelope = IntentEnvelope::from_runtime(
            message_id,
            character_instance_id,
            correlation_id,
            intent,
            TruthState::None,
            requested_priority.min(AI_REQUEST_MAX_PRIORITY),
            timestamp,
            expires_at,
        );
        envelope.priority = envelope.priority.min(AI_REQUEST_MAX_PRIORITY);
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn t0() -> Timestamp {
        Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0)
            .single()
            .unwrap_or_default()
    }

    #[test]
    fn floors_match_spec_table() {
        assert_eq!(priority_floor(CharacterIntent::Emergency), 100);
        assert_eq!(priority_floor(CharacterIntent::Offline), 95);
        assert_eq!(priority_floor(CharacterIntent::Blocked), 90);
        assert_eq!(priority_floor(CharacterIntent::Failed), 85);
        assert_eq!(priority_floor(CharacterIntent::RequestConsent), 80);
        assert_eq!(priority_floor(CharacterIntent::Unknown), 75);
        assert_eq!(priority_floor(CharacterIntent::VerifiedSuccess), 70);
        assert_eq!(priority_floor(CharacterIntent::ClaimCompleted), 65);
        assert_eq!(priority_floor(CharacterIntent::Wait), 60);
        assert_eq!(priority_floor(CharacterIntent::Ask), 60);
        assert_eq!(priority_floor(CharacterIntent::Cancelled), 55);
        for i in [
            CharacterIntent::Idle,
            CharacterIntent::Notice,
            CharacterIntent::Acknowledge,
            CharacterIntent::Think,
            CharacterIntent::Work,
            CharacterIntent::Greet,
            CharacterIntent::Play,
            CharacterIntent::Rest,
            CharacterIntent::Sleep,
        ] {
            assert_eq!(priority_floor(i), 0, "{i}");
            assert!(!i.is_safety());
            assert!(i.ai_allowed());
        }
        assert_eq!(
            CharacterIntent::ALL
                .iter()
                .filter(|i| i.is_safety())
                .count(),
            11
        );
    }

    #[test]
    fn wire_names_round_trip() {
        for intent in CharacterIntent::ALL {
            let json = serde_json::to_string(&intent).unwrap_or_default();
            assert_eq!(json, format!("\"{}\"", intent.as_str()));
            assert_eq!(CharacterIntent::parse(intent.as_str()), Some(intent));
        }
        assert_eq!(CharacterIntent::parse("dance"), None);
        assert_eq!(
            serde_json::to_string(&ResumePolicy::NoResume).unwrap_or_default(),
            "\"none\""
        );
        assert_eq!(
            serde_json::to_string(&TruthState::WaitingConsent).unwrap_or_default(),
            "\"waiting-consent\""
        );
    }

    #[test]
    fn priority_is_clamped_to_floor() {
        let env = IntentEnvelope::from_runtime(
            "m1",
            "inst",
            None,
            CharacterIntent::Blocked,
            TruthState::Blocked,
            10,
            t0(),
            t0() + chrono::Duration::seconds(30),
        );
        assert_eq!(env.priority, 90);
        let mut lowered = env.clone();
        lowered.priority = 1;
        let normalized = normalize_envelope(&lowered).unwrap_or(lowered.clone());
        assert_eq!(normalized.priority, 90);
    }

    #[test]
    fn parameters_over_4kb_rejected() {
        let mut env = IntentEnvelope::from_runtime(
            "m1",
            "inst",
            None,
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t0(),
            t0() + chrono::Duration::seconds(30),
        );
        for i in 0..40 {
            env.parameters
                .insert(format!("k{i}"), serde_json::Value::String("x".repeat(150)));
        }
        assert!(matches!(
            normalize_envelope(&env),
            Err(EnvelopeError::ParametersTooLarge { .. })
        ));
    }

    #[test]
    fn long_strings_rejected() {
        let mut env = IntentEnvelope::from_runtime(
            "m1",
            "inst",
            None,
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t0(),
            t0() + chrono::Duration::seconds(30),
        );
        env.parameters.insert(
            "nested".into(),
            serde_json::json!({ "deep": ["a", "b".repeat(201)] }),
        );
        assert!(matches!(
            normalize_envelope(&env),
            Err(EnvelopeError::StringTooLong { .. })
        ));
    }

    #[test]
    fn expires_at_must_follow_timestamp() {
        let env = IntentEnvelope::from_runtime(
            "m1",
            "inst",
            None,
            CharacterIntent::Work,
            TruthState::Working,
            10,
            t0(),
            t0(),
        );
        assert_eq!(
            normalize_envelope(&env),
            Err(EnvelopeError::ExpiresBeforeTimestamp)
        );
    }

    #[test]
    fn ai_requests_are_capped_and_truth_none() {
        let env = IntentEnvelope::from_ai_request(
            "m1",
            "inst",
            None,
            CharacterIntent::Work,
            200,
            t0(),
            t0() + chrono::Duration::seconds(5),
        )
        .expect("work is AI-allowed");
        assert_eq!(env.priority, 50);
        assert_eq!(env.truth_state, TruthState::None);
        assert!(matches!(
            IntentEnvelope::from_ai_request(
                "m2",
                "inst",
                None,
                CharacterIntent::VerifiedSuccess,
                1,
                t0(),
                t0() + chrono::Duration::seconds(5),
            ),
            Err(EnvelopeError::AiIntentNotAllowed { .. })
        ));
        assert_eq!(
            CharacterIntent::Wait.ai_safe_substitute(),
            CharacterIntent::Think
        );
        assert_eq!(
            CharacterIntent::Emergency.ai_safe_substitute(),
            CharacterIntent::Notice
        );
    }
}
