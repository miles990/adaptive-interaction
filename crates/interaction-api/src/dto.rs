//! Request DTOs shared by routes and the tool-call dispatcher.

use interaction_core::MessageStrategy;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanInput {
    pub intent: String,
    #[serde(default)]
    pub character: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub magnitude: Option<f64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub preferred_channels: Vec<String>,
    #[serde(default)]
    pub candidates: Vec<String>,
    #[serde(default)]
    pub min_channels: u32,
    #[serde(default = "default_max_channels")]
    pub max_channels: u32,
    #[serde(default = "default_true")]
    pub allow_no_action: bool,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default)]
    pub message_strategy: Option<MessageStrategy>,
    /// Execution semantics: actuationMode, verification, ...
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

fn default_max_channels() -> u32 {
    3
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteInput {
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartInput {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub ttl_minutes: Option<u32>,
    #[serde(default)]
    pub consents: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentInput {
    pub scope: String,
    #[serde(default)]
    pub expires_minutes: Option<u32>,
    /// Real "only this once": the first authorized dispatch spends the consent.
    /// Absent = unlimited within the TTL (the behaviour older clients rely on).
    #[serde(default)]
    pub max_uses: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnabledPatch {
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushInput {
    #[serde(default)]
    pub facts: BTreeMap<String, Value>,
    #[serde(default)]
    pub inferences: BTreeMap<String, Value>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

fn default_confidence() -> f64 {
    1.0
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EmergencyStopInput {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecipeBody {
    /// Recipe as YAML or JSON text; or structured JSON under `recipe`.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub recipe: Option<Value>,
}

impl RecipeBody {
    pub fn as_text(&self) -> Option<String> {
        if let Some(t) = &self.text {
            return Some(t.clone());
        }
        self.recipe.as_ref().map(|v| v.to_string())
    }
}
