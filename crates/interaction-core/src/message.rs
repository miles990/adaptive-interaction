//! Default interaction message intents and text selection strategy types.
//! The actual catalog and adaptive selection live in the runtime; these types
//! are shared so recipes, API and UI agree on the model.

use serde::{Deserialize, Serialize};

/// Built-in message intents. Recipes and AIs may also use free-form strings;
/// this enum enumerates the guaranteed defaults.
pub const DEFAULT_MESSAGE_INTENTS: &[&str] = &[
    "presence",
    "task-start",
    "progress",
    "discovery",
    "success",
    "celebration",
    "warning",
    "failure",
    "recovery",
    "confirmation-required",
    "stopped",
    "emergency-stop",
    "calm",
    "tension",
    "assistance",
];

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MessageMode {
    /// Always use the fixed template(s).
    Fixed,
    /// Random pick among candidates.
    Random,
    /// Adaptive pick (anti-repetition, tone, persona).
    #[default]
    Adaptive,
    /// The AI supplies text at plan time.
    AiGenerated,
    /// Deliberately no text.
    None,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageStrategy {
    #[serde(default)]
    pub mode: MessageMode,
    /// Candidate intents (looked up in the catalog).
    #[serde(default)]
    pub intents: Vec<String>,
    /// User-supplied templates; `{task}`-style placeholders are allowed.
    #[serde(default)]
    pub templates: Vec<String>,
    /// Whether silence is an acceptable outcome.
    #[serde(default)]
    pub allow_silence: bool,
    /// BCP-47 language tag preference, e.g. `zh-Hant`, `en`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Tone hint, e.g. `neutral`, `warm`, `serious`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    /// Suppress a candidate if it was used within this window (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_cooldown_ms: Option<u64>,
    /// Unknown fields preserved verbatim (lossless recipe round-trips).
    #[serde(
        default,
        flatten,
        skip_serializing_if = "std::collections::BTreeMap::is_empty"
    )]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}
