//! Semantic intents, plans and bounded actions.
//!
//! The AI produces *semantic* intents ("celebrate progress at magnitude 0.35").
//! The deterministic policy governor turns a planned step into an immutable
//! [`BoundedAction`]; only bounded actions ever reach a driver.

use crate::{
    ActionId, ActuatorId, CorrelationId, PlanId, PolicyDecision, RiskClass, SessionId, Timestamp,
    SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A semantic interaction request from an AI (or a recipe).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SemanticIntent {
    /// Semantic intent name, e.g. `celebrate-progress`, `warning`, `presence`.
    pub intent: String,
    /// Optional persona/character flavor, e.g. `restrained-delight`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<String>,
    /// Normalized magnitude 0..1. This is a *suggestion*; policy clamps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Preferred channels in priority order, e.g. `["visual", "audio"]`.
    #[serde(default)]
    pub preferred_channels: Vec<String>,
    /// Optional message text suggestion (may be rewritten or silenced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Free-form structured payload for driver-specific hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// The intent is void after this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
}

/// One candidate step inside a plan: a target actuator plus requested parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlannedStep {
    pub actuator_id: ActuatorId,
    pub channel: String,
    /// Requested (pre-bounding) parameters.
    pub requested: ActionParameters,
    /// Utility score assigned by the orchestrator (explainable heuristic).
    pub score: f64,
    /// Human-readable reason this step was selected.
    pub rationale: String,
}

/// Normalized action parameters shared by all channels.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActionParameters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Driver-specific extras (validated against the actuator's schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PlanStatus {
    Draft,
    Simulated,
    Authorized,
    Blocked,
    Executed,
    Expired,
    /// The orchestrator judged that doing nothing is the best action.
    NoAction,
}

/// A plan: the orchestrator's (or AI's) proposed interaction, before/after
/// policy authorization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub plan_id: PlanId,
    pub session_id: SessionId,
    pub intent: SemanticIntent,
    /// Steps chosen for execution (post candidate filtering).
    pub steps: Vec<PlannedStep>,
    /// Candidates that were considered and rejected, with reasons (explainability).
    #[serde(default)]
    pub rejected: Vec<RejectedCandidate>,
    pub status: PlanStatus,
    #[serde(default)]
    pub policy_decisions: Vec<PolicyDecision>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub correlation_id: CorrelationId,
    /// Execution semantics (actuation mode, source recipe, verification
    /// strategy...) understood by the runtime.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    pub schema_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RejectedCandidate {
    pub actuator_id: ActuatorId,
    pub reason: String,
}

impl Plan {
    pub fn is_expired(&self, now: Timestamp) -> bool {
        now > self.expires_at
    }
}

/// Immutable, policy-bounded action handed to a driver. Drivers must treat the
/// effective parameters as final; there is no way to raise them afterwards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundedAction {
    pub action_id: ActionId,
    pub plan_id: PlanId,
    pub session_id: SessionId,
    pub actuator_id: ActuatorId,
    pub intent: String,
    pub risk_class: RiskClass,
    /// Parameters as originally requested (audit trail).
    pub requested: ActionParameters,
    /// Parameters after policy bounding — the only values a driver may use.
    pub effective: ActionParameters,
    /// The policy decisions that produced `effective`.
    pub policy_decisions: Vec<PolicyDecision>,
    /// Hard deadline: the driver must not start (or continue) after this.
    pub expires_at: Timestamp,
    pub issued_at: Timestamp,
    pub correlation_id: CorrelationId,
    /// Extra bounded metadata (e.g. pattern with clamped steps).
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    pub schema_version: String,
}

impl BoundedAction {
    pub fn is_expired(&self, now: Timestamp) -> bool {
        now > self.expires_at
    }
}

/// A bounded pattern timeline (the safe replacement for raw `PATTERN` scripts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PatternSpec {
    /// Ordered steps; the governor enforces `max_pattern_steps`.
    pub steps: Vec<PatternStep>,
    /// Number of repetitions. `repeat: forever` is normalized into a lease
    /// (see runtime) — it never becomes an unbounded loop here.
    #[serde(default = "default_repeat")]
    pub repeat: u32,
    /// Probability 0..1 that each repetition actually fires.
    #[serde(default = "default_chance")]
    pub chance: f64,
    /// Max random jitter added before each step, in ms.
    #[serde(default)]
    pub jitter_ms: u64,
}

fn default_repeat() -> u32 {
    1
}

fn default_chance() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PatternStep {
    pub magnitude: f64,
    pub duration_ms: u64,
    #[serde(default)]
    pub pause_ms: u64,
}

/// Standard verification outcome attached to receipts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerificationEvidence {
    /// Observation ids that support the verdict.
    #[serde(default)]
    pub observation_ids: Vec<crate::ObservationId>,
    pub verdict: VerificationVerdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub verified_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VerificationVerdict {
    /// Effect observed in the environment.
    Observed,
    /// Driver acknowledged but no environmental confirmation.
    AcknowledgedOnly,
    /// Could not determine whether the effect happened.
    Uncertain,
    /// Evidence says the effect did NOT happen.
    Refuted,
}

impl SemanticIntent {
    pub fn new(intent: impl Into<String>) -> Self {
        Self {
            intent: intent.into(),
            character: None,
            magnitude: None,
            duration_ms: None,
            preferred_channels: Vec::new(),
            message: None,
            payload: None,
            expires_at: None,
            correlation_id: None,
        }
    }
}

/// Convenience constructor used by the orchestrator.
pub fn new_plan(
    session_id: SessionId,
    intent: SemanticIntent,
    now: Timestamp,
    ttl_ms: u64,
) -> Plan {
    let correlation_id = intent
        .correlation_id
        .clone()
        .unwrap_or_else(CorrelationId::generate);
    let expires_at = intent
        .expires_at
        .unwrap_or_else(|| now + chrono::Duration::milliseconds(ttl_ms as i64));
    Plan {
        plan_id: PlanId::generate(),
        session_id,
        intent,
        steps: Vec::new(),
        rejected: Vec::new(),
        status: PlanStatus::Draft,
        policy_decisions: Vec::new(),
        created_at: now,
        expires_at,
        correlation_id,
        metadata: BTreeMap::new(),
        schema_version: SCHEMA_VERSION.to_string(),
    }
}
