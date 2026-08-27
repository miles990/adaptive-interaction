//! Recipe data model. One domain model validates both YAML and JSON.

use interaction_core::{MessageStrategy, RecipeId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum FusionMode {
    #[default]
    Single,
    All,
    Any,
    Quorum,
    Weighted,
    Sequence,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ActuationMode {
    Single,
    Parallel,
    Sequence,
    Fallback,
    #[default]
    Adaptive,
    Redundant,
}

/// One trigger step: a receptor plus a condition on its observations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriggerStep {
    pub receptor: String,
    /// Condition over observation facts (see [`crate::condition`]).
    #[serde(default)]
    pub condition: Option<crate::ConditionSpec>,
    /// Weight for `weighted` fusion (default 1.0).
    #[serde(default = "default_weight")]
    pub weight: f64,
    /// Unknown fields preserved verbatim so editors and round-trips never
    /// silently drop data written by newer versions or other tools.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriggerSpec {
    #[serde(default)]
    pub mode: FusionMode,
    /// Time window for `sequence` / `all`, e.g. `"10m"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    /// Required matches for `quorum` mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quorum: Option<u32>,
    /// Threshold for `weighted` mode (sum of matched weights).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    pub steps: Vec<TriggerStep>,
    /// Unknown fields preserved verbatim so editors and round-trips never
    /// silently drop data written by newer versions or other tools.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextSpec {
    /// Additional receptors consulted (not required to fire) when planning.
    #[serde(default)]
    pub receptors: Vec<String>,
    /// Ignore observations older than this, e.g. `"5m"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age: Option<String>,
    /// Minimum inference confidence used during fusion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f64>,
    /// Unknown fields preserved verbatim so editors and round-trips never
    /// silently drop data written by newer versions or other tools.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionSpec {
    /// Human-readable objective, e.g. `celebrate-without-interrupting`.
    pub objective: String,
    /// The recipe accepts "do nothing" as a legitimate outcome.
    #[serde(default = "default_true")]
    pub allow_no_action: bool,
    /// Unknown fields preserved verbatim so editors and round-trips never
    /// silently drop data written by newer versions or other tools.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActuationSpec {
    #[serde(default)]
    pub mode: ActuationMode,
    /// Candidate actuator ids, in preference order.
    pub candidates: Vec<String>,
    #[serde(default)]
    pub min_channels: u32,
    #[serde(default = "default_max_channels")]
    pub max_channels: u32,
    /// Probability 0..1 the actuation fires at all (surprise factor).
    #[serde(default = "default_chance")]
    pub chance: f64,
    /// Max random start jitter, e.g. `"2s"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter: Option<String>,
    /// Unknown fields preserved verbatim so editors and round-trips never
    /// silently drop data written by newer versions or other tools.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_max_channels() -> u32 {
    3
}

fn default_chance() -> f64 {
    1.0
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum VerificationStrategy {
    /// Acknowledgement from the driver is enough to complete.
    #[default]
    BestEffort,
    /// Require an observation confirming the effect.
    Observed,
    /// Do not verify (receipt stops at acknowledged/uncertain).
    None,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSpec {
    #[serde(default)]
    pub strategy: VerificationStrategy,
    /// e.g. `"10s"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    /// Receptors to consult for verification evidence.
    #[serde(default)]
    pub receptors: Vec<String>,
    /// Unknown fields preserved verbatim so editors and round-trips never
    /// silently drop data written by newer versions or other tools.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LimitsSpec {
    /// Minimum interval between firings, e.g. `"15m"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown: Option<String>,
    /// The produced plan expires after this, e.g. `"30s"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_executions_per_session: Option<u32>,
    /// Max firings per rolling hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_hour: Option<u32>,
    /// Unknown fields preserved verbatim so editors and round-trips never
    /// silently drop data written by newer versions or other tools.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsentRequirements {
    /// Consent scopes that must be active for the recipe to run,
    /// e.g. `["channel:haptic"]`.
    #[serde(default)]
    pub required: Vec<String>,
    /// Unknown fields preserved verbatim so editors and round-trips never
    /// silently drop data written by newer versions or other tools.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: RecipeId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub trigger: TriggerSpec,
    #[serde(default)]
    pub context: ContextSpec,
    pub decision: DecisionSpec,
    /// Semantic intent produced when the recipe fires.
    #[serde(default = "default_intent")]
    pub intent: String,
    #[serde(default)]
    pub message: MessageStrategy,
    pub actuation: ActuationSpec,
    #[serde(default)]
    pub verification: VerificationSpec,
    #[serde(default)]
    pub limits: LimitsSpec,
    #[serde(default)]
    pub consent: ConsentRequirements,
    /// AI involvement policy for this recipe (default: never involve AI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<crate::AiAssistSpec>,
    /// Extra free-form metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
    /// Unknown fields preserved verbatim so editors and round-trips never
    /// silently drop data written by newer versions or other tools.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
}

fn default_intent() -> String {
    "presence".to_string()
}

fn default_schema_version() -> String {
    interaction_core::SCHEMA_VERSION.to_string()
}

/// Parse a human duration like `10m`, `30s`, `500ms` into milliseconds.
/// Recipes are persisted and later converted to `chrono::Duration`; reject
/// absurd legacy/new values at the schema boundary instead of saturating and
/// allowing a later arithmetic panic. One year is deliberately far above any
/// supported interaction lease while remaining straightforward to audit.
pub const MAX_RECIPE_DURATION_MS: u64 = 365 * 24 * 60 * 60 * 1_000;

pub fn parse_duration_ms(s: &str) -> Result<u64, String> {
    let duration =
        humantime::parse_duration(s).map_err(|e| format!("invalid duration {s:?}: {e}"))?;
    let millis = u64::try_from(duration.as_millis())
        .map_err(|_| format!("duration {s:?} exceeds the supported range"))?;
    if millis > MAX_RECIPE_DURATION_MS {
        return Err(format!(
            "duration {s:?} exceeds the maximum of {}ms",
            MAX_RECIPE_DURATION_MS
        ));
    }
    Ok(millis)
}

#[cfg(test)]
mod duration_tests {
    use super::*;

    #[test]
    fn rejects_duration_that_could_overflow_runtime_time_arithmetic() {
        assert!(parse_duration_ms("999999999d").is_err());
        assert_eq!(parse_duration_ms("365d").unwrap(), MAX_RECIPE_DURATION_MS);
    }
}
