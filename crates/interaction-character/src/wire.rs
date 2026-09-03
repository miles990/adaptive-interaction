//! §8 Wire messages（transport-neutral JSON）、§3.3 握手 payload、上限與速率限制。
//!
//! 方向：`runtime → adapter : hello | negotiated | intent | cancel | heartbeat | error | goodbye`；
//! `adapter → runtime : negotiate | receipt | event | lifecycle | heartbeat | error | goodbye`。
//! adapter → runtime 的每一種 payload 在型別層都沒有 `truthState`／`verified`。

use crate::capability::IntentResolution;
use crate::input::CharacterInputEvent;
use crate::intent::{CharacterIntent, IntentEnvelope};
use crate::lifecycle::{AdapterLifecycleState, CharacterRole};
use crate::manifest::CapabilityDecl;
use crate::receipt::CommandReceipt;
use crate::{Timestamp, PROTOCOL_VERSION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// §8 上限（常數，不可由 adapter 協商放寬）。
pub struct Limits;

impl Limits {
    /// 單則訊息 ≤ 64 KB。
    pub const MAX_MESSAGE_BYTES: usize = 65_536;
    /// 每個 adapter ≤ 50 則/s。
    pub const MAX_MESSAGES_PER_SEC: u32 = 50;
    /// pending intents ≤ 64。
    pub const MAX_PENDING: usize = 64;
    /// outbound 佇列 ≤ 32。
    pub const MAX_OUTBOUND: usize = 32;
    /// heartbeat 每 15 s。
    pub const HEARTBEAT_INTERVAL_MS: i64 = 15_000;
    /// 45 s 無訊息視為斷線。
    pub const DISCONNECT_AFTER_MS: i64 = 45_000;
    /// 重連退避 1 s → 15 s（倍增）。
    pub const RECONNECT_BACKOFF_MIN_MS: u64 = 1_000;
    pub const RECONNECT_BACKOFF_MAX_MS: u64 = 15_000;
    /// messageId 去重環大小。
    pub const DEDUPE_RING: usize = 256;
}

/// 重連退避（倍增、封頂 15 s）。`attempt` 從 0 起算。
pub fn reconnect_backoff_ms(attempt: u32) -> u64 {
    let shift = attempt.min(16);
    (Limits::RECONNECT_BACKOFF_MIN_MS << shift).min(Limits::RECONNECT_BACKOFF_MAX_MS)
}

/// `hello.limits`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HelloLimits {
    pub max_message_bytes: usize,
    pub max_messages_per_second: u32,
    pub max_pending: usize,
}

impl Default for HelloLimits {
    fn default() -> Self {
        HelloLimits {
            max_message_bytes: Limits::MAX_MESSAGE_BYTES,
            max_messages_per_second: Limits::MAX_MESSAGES_PER_SEC,
            max_pending: Limits::MAX_PENDING,
        }
    }
}

/// §3.3 步驟 1：Runtime／Gateway → Adapter。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol_version: String,
    pub runtime_version: String,
    pub character_instance_id: String,
    pub role: CharacterRole,
    pub locale: String,
    pub reduced_motion: bool,
    /// Runtime 會送出的 intent（資訊性；協商仍解析全部 20 個）。
    #[serde(default)]
    pub requires: Vec<CharacterIntent>,
    #[serde(default)]
    pub limits: HelloLimits,
}

/// §3.3 步驟 2：Adapter → Gateway。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Negotiate {
    pub protocol_version: String,
    pub character_id: String,
    pub manifest_version: String,
    #[serde(default)]
    pub capabilities: BTreeMap<String, CapabilityDecl>,
    #[serde(default)]
    pub input_capabilities: BTreeMap<String, CapabilityDecl>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub intents: Vec<String>,
    /// variant ids。
    #[serde(default)]
    pub variants: Vec<String>,
    /// adapter 自己的連線計數（資訊性；Gateway 的 generation 才是權威）。
    #[serde(default)]
    pub generation: u64,
}

impl Negotiate {
    /// 由 manifest 建立「照 manifest 全數提供」的 offer（in-process adapter 與測試用）。
    pub fn from_manifest(manifest: &crate::manifest::CharacterManifest, generation: u64) -> Self {
        Negotiate {
            protocol_version: PROTOCOL_VERSION.to_string(),
            character_id: manifest.character_id.clone(),
            manifest_version: manifest.version.clone(),
            capabilities: manifest.capabilities.clone(),
            input_capabilities: manifest.input_capabilities.clone(),
            channels: manifest.channels.clone(),
            intents: manifest.intents.clone(),
            variants: manifest.variants.iter().map(|v| v.id.clone()).collect(),
            generation,
        }
    }
}

/// §3.3 步驟 3：Gateway → Adapter。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Negotiated {
    pub character_instance_id: String,
    pub generation: u64,
    pub reduced_motion: bool,
    pub resolutions: BTreeMap<CharacterIntent, IntentResolution>,
    pub accepted_channels: Vec<String>,
    /// `acceptedChannels` 中的 namespaced custom channel（nonSafety）。
    #[serde(default)]
    pub non_safety_channels: Vec<String>,
    pub ignored_channels: Vec<String>,
    /// 最終有效宣告（只含 supported）。
    pub capabilities: BTreeMap<String, CapabilityDecl>,
    #[serde(default)]
    pub input_capabilities: BTreeMap<String, CapabilityDecl>,
}

/// §8 wire message（tagged `type`，kebab-case）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum WireMessage {
    Hello(Hello),
    Negotiate(Negotiate),
    Negotiated(Negotiated),
    Intent {
        envelope: IntentEnvelope,
    },
    Cancel {
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Heartbeat {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        generation: Option<u64>,
    },
    Error {
        code: String,
        message: String,
    },
    Goodbye {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Receipt {
        receipt: CommandReceipt,
    },
    Event {
        event: CharacterInputEvent,
    },
    Lifecycle {
        state: AdapterLifecycleState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<Timestamp>,
    },
}

impl WireMessage {
    pub fn kind(&self) -> &'static str {
        match self {
            WireMessage::Hello(_) => "hello",
            WireMessage::Negotiate(_) => "negotiate",
            WireMessage::Negotiated(_) => "negotiated",
            WireMessage::Intent { .. } => "intent",
            WireMessage::Cancel { .. } => "cancel",
            WireMessage::Heartbeat { .. } => "heartbeat",
            WireMessage::Error { .. } => "error",
            WireMessage::Goodbye { .. } => "goodbye",
            WireMessage::Receipt { .. } => "receipt",
            WireMessage::Event { .. } => "event",
            WireMessage::Lifecycle { .. } => "lifecycle",
        }
    }

    /// 是否為 adapter → runtime 方向允許的訊息。
    pub fn is_adapter_to_runtime(&self) -> bool {
        matches!(
            self,
            WireMessage::Negotiate(_)
                | WireMessage::Receipt { .. }
                | WireMessage::Event { .. }
                | WireMessage::Lifecycle { .. }
                | WireMessage::Heartbeat { .. }
                | WireMessage::Error { .. }
                | WireMessage::Goodbye { .. }
        )
    }

    /// 是否為 runtime → adapter 方向允許的訊息。
    pub fn is_runtime_to_adapter(&self) -> bool {
        matches!(
            self,
            WireMessage::Hello(_)
                | WireMessage::Negotiated(_)
                | WireMessage::Intent { .. }
                | WireMessage::Cancel { .. }
                | WireMessage::Heartbeat { .. }
                | WireMessage::Error { .. }
                | WireMessage::Goodbye { .. }
        )
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        WireMessage::Error {
            code: code.into(),
            message: crate::truncate_for_echo(&message.into()),
        }
    }
}

/// wire 解析錯誤。訊息不回顯輸入內容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(tag = "code", rename_all = "kebab-case")]
pub enum WireError {
    #[error("message is {bytes} bytes (max {max})")]
    TooLarge { bytes: usize, max: usize },
    #[error("message is not valid UTF-8")]
    Utf8,
    #[error("message is not a valid wire message: {category}")]
    Malformed { category: String },
}

impl WireError {
    pub fn code(&self) -> &'static str {
        match self {
            WireError::TooLarge { .. } => "too-large",
            WireError::Utf8 => "malformed",
            WireError::Malformed { .. } => "malformed",
        }
    }
}

/// 解析單則 wire 訊息，強制 ≤ 64 KB。
pub fn parse_wire(bytes: &[u8]) -> Result<WireMessage, WireError> {
    if bytes.len() > Limits::MAX_MESSAGE_BYTES {
        return Err(WireError::TooLarge {
            bytes: bytes.len(),
            max: Limits::MAX_MESSAGE_BYTES,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| WireError::Utf8)?;
    serde_json::from_str(text).map_err(|e| WireError::Malformed {
        category: match e.classify() {
            serde_json::error::Category::Syntax => "syntax".into(),
            serde_json::error::Category::Data => "data".into(),
            serde_json::error::Category::Eof => "eof".into(),
            serde_json::error::Category::Io => "io".into(),
        },
    })
}

/// 序列化單則 wire 訊息，強制 ≤ 64 KB（超過即錯，不送）。
pub fn encode_wire(message: &WireMessage) -> Result<Vec<u8>, WireError> {
    let bytes = serde_json::to_vec(message).map_err(|_| WireError::Malformed {
        category: "encode".into(),
    })?;
    if bytes.len() > Limits::MAX_MESSAGE_BYTES {
        return Err(WireError::TooLarge {
            bytes: bytes.len(),
            max: Limits::MAX_MESSAGE_BYTES,
        });
    }
    Ok(bytes)
}

/// Token bucket 速率限制（時間注入，確定性）。
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimiter {
    capacity: f64,
    tokens: f64,
    refill_per_ms: f64,
    last_ms: i64,
}

impl RateLimiter {
    /// `per_second` 則/s；容量等於每秒額度（允許短暫 burst，但平均不超過）。
    pub fn new(per_second: u32, now_ms: i64) -> Self {
        let capacity = f64::from(per_second.max(1));
        RateLimiter {
            capacity,
            tokens: capacity,
            refill_per_ms: capacity / 1000.0,
            last_ms: now_ms,
        }
    }

    /// 是否允許這一則；超過 → `false`（呼叫端回 `error{code:"rate-limited"}` 並丟棄）。
    pub fn allow(&mut self, now_ms: i64) -> bool {
        let elapsed = (now_ms - self.last_ms).max(0) as f64;
        self.last_ms = self.last_ms.max(now_ms);
        self.tokens = (self.tokens + elapsed * self.refill_per_ms).min(self.capacity);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    pub fn available(&self) -> u32 {
        self.tokens.floor() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_message_rejected() {
        let big = vec![b' '; Limits::MAX_MESSAGE_BYTES + 1];
        assert!(matches!(parse_wire(&big), Err(WireError::TooLarge { .. })));
        let exact = format!(
            "{{\"type\":\"heartbeat\"{}}}",
            " ".repeat(Limits::MAX_MESSAGE_BYTES - 20)
        );
        assert!(matches!(
            parse_wire(exact.as_bytes()),
            Ok(WireMessage::Heartbeat { .. })
        ));
    }

    #[test]
    fn malformed_and_unknown_type() {
        assert!(matches!(
            parse_wire(b"{not json"),
            Err(WireError::Malformed { .. })
        ));
        assert!(matches!(
            parse_wire(b"{\"type\":\"teleport\"}"),
            Err(WireError::Malformed { .. })
        ));
        assert_eq!(parse_wire(&[0xFF, 0xFE]), Err(WireError::Utf8));
    }

    #[test]
    fn tag_is_kebab_case_and_flat_for_handshake() {
        let hello = WireMessage::Hello(Hello {
            protocol_version: PROTOCOL_VERSION.into(),
            runtime_version: "0.5.0".into(),
            character_instance_id: "inst".into(),
            role: CharacterRole::PrimaryCompanion,
            locale: "zh-TW".into(),
            reduced_motion: false,
            requires: vec![CharacterIntent::Emergency],
            limits: HelloLimits::default(),
        });
        let v = serde_json::to_value(&hello).unwrap_or_default();
        assert_eq!(v["type"], "hello");
        assert_eq!(v["protocolVersion"], "1.0");
        assert_eq!(v["role"], "primary-companion");
        assert_eq!(v["limits"]["maxMessageBytes"], 65536);
        let cancel = WireMessage::Cancel {
            message_id: "m".into(),
            reason: Some("x".into()),
        };
        let v = serde_json::to_value(&cancel).unwrap_or_default();
        assert_eq!(v["type"], "cancel");
        assert_eq!(v["messageId"], "m");
    }

    #[test]
    fn direction_tables() {
        let recv = WireMessage::Receipt {
            receipt: CommandReceipt::new(
                "m",
                "i",
                1,
                crate::receipt::ReceiptStatus::Accepted,
                Timestamp::default(),
            ),
        };
        assert!(recv.is_adapter_to_runtime());
        assert!(!recv.is_runtime_to_adapter());
        let neg = WireMessage::Negotiated(Negotiated {
            character_instance_id: "i".into(),
            generation: 1,
            reduced_motion: false,
            resolutions: BTreeMap::new(),
            accepted_channels: vec![],
            non_safety_channels: vec![],
            ignored_channels: vec![],
            capabilities: BTreeMap::new(),
            input_capabilities: BTreeMap::new(),
        });
        assert!(neg.is_runtime_to_adapter());
        assert!(!neg.is_adapter_to_runtime());
    }

    #[test]
    fn rate_limiter_is_deterministic() {
        let mut rl = RateLimiter::new(50, 0);
        let allowed = (0..60).filter(|_| rl.allow(0)).count();
        assert_eq!(allowed, 50);
        assert!(!rl.allow(0));
        // 100 ms 後補回 5 個 token。
        let again = (0..10).filter(|_| rl.allow(100)).count();
        assert_eq!(again, 5);
        // 時間倒退不會補 token。
        assert!(!rl.allow(50));
        assert_eq!(reconnect_backoff_ms(0), 1_000);
        assert_eq!(reconnect_backoff_ms(1), 2_000);
        assert_eq!(reconnect_backoff_ms(3), 8_000);
        assert_eq!(reconnect_backoff_ms(4), 15_000);
        assert_eq!(reconnect_backoff_ms(40), 15_000);
    }
}
