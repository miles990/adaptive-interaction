//! Per-recipe AI involvement policy: when (if ever) an external AI is asked
//! to help, what it may see, and what happens when it is unavailable.
//!
//! The runtime itself never calls an LLM. When a recipe's decision gate defers
//! to AI, the runtime publishes an `ai.assist.requested` event that an attached
//! AI host may act on; if none does within `max_wait_ms`, the deterministic
//! `on_unavailable` behavior applies. Deterministic events never involve AI.

use serde::{Deserialize, Serialize};

/// When AI may be involved in this recipe.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AiAssistMode {
    /// Never involve AI; fully deterministic (the default).
    #[default]
    Never,
    /// Only when the deterministic rules cannot decide (ambiguous /
    /// low-confidence / contradictory observations).
    WhenUncertain,
    /// AI may help interpret observations before planning.
    Interpret,
    /// AI may help choose among candidate channels.
    ChooseChannel,
    /// AI only generates message text; channel choice stays deterministic.
    GenerateText,
    /// AI may draft recipe changes, but a human must confirm them.
    DraftOnly,
}

/// Deterministic behavior when AI is needed but unavailable / times out.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AiUnavailableBehavior {
    /// Proceed with the deterministic plan (conservative candidate order).
    #[default]
    Fallback,
    /// Do nothing this round.
    NoAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistSpec {
    #[serde(default)]
    pub mode: AiAssistMode,
    /// Below this inference confidence the situation counts as "uncertain".
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f64,
    /// How long to wait for an AI host before falling back (ms).
    #[serde(default = "default_max_wait_ms")]
    pub max_wait_ms: u64,
    #[serde(default)]
    pub on_unavailable: AiUnavailableBehavior,
    /// Fact/data categories that may be shared with the AI. Empty = only
    /// non-sensitive event metadata.
    #[serde(default)]
    pub data_scope: Vec<String>,
    /// Whether AI proposals need explicit human confirmation before acting.
    #[serde(default)]
    pub require_human_confirmation: bool,
    /// Hard cap on assist requests per day for this recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_call_cap: Option<u32>,
    /// Whether sensitive observations may leave this machine to reach the AI.
    /// Defaults to false and is additionally bounded by global policy.
    #[serde(default)]
    pub allow_sensitive_egress: bool,
}

impl Default for AiAssistSpec {
    fn default() -> Self {
        Self {
            mode: AiAssistMode::Never,
            min_confidence: default_min_confidence(),
            max_wait_ms: default_max_wait_ms(),
            on_unavailable: AiUnavailableBehavior::Fallback,
            data_scope: Vec::new(),
            require_human_confirmation: false,
            daily_call_cap: None,
            allow_sensitive_egress: false,
        }
    }
}

fn default_min_confidence() -> f64 {
    0.6
}

fn default_max_wait_ms() -> u64 {
    5_000
}

/// The gate's verdict for one firing, recorded in traces and timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum AiGateOutcome {
    /// Deterministic rules handled it; AI was not needed.
    #[serde(rename_all = "camelCase")]
    NotNeeded { reason: String },
    /// AI help was requested from any attached host.
    #[serde(rename_all = "camelCase")]
    Requested { reason: String, deadline_ms: u64 },
    /// AI was needed but unavailable; deterministic fallback proceeded.
    #[serde(rename_all = "camelCase")]
    UnavailableFallback { reason: String },
    /// AI was needed but unavailable; the recipe chose no-action.
    #[serde(rename_all = "camelCase")]
    UnavailableNoAction { reason: String },
    /// This recipe never involves AI.
    #[serde(rename_all = "camelCase")]
    Disabled {},
}
