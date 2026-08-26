//! Policy configuration and decisions. The governor itself lives in
//! `interaction-policy`; this module defines the shared data model.

use crate::{RiskClass, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How proactive the runtime may be without an explicit request.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum InitiativeLevel {
    /// Never initiate; only respond to explicit requests.
    Passive,
    /// May surface low-risk, non-interrupting signals.
    #[default]
    Suggest,
    /// May initiate bounded interactions on allowed channels.
    Active,
}

/// A daily quiet window in local wall-clock time ("HH:MM" 24h).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuietHours {
    pub start: String,
    pub end: String,
    /// Channels silenced during the window; empty = all audible/intrusive ones.
    #[serde(default)]
    pub silenced_channels: Vec<String>,
}

/// Per-channel deterministic limits.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_magnitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_hour: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_ms: Option<u64>,
    /// Cumulative active-duration budget per session, in ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_budget_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PolicyConfig {
    /// Master switch. When false, only read-only operations are allowed.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub initiative: InitiativeLevel,
    /// Explicit receptor allowlist; empty = builtin defaults only.
    #[serde(default)]
    pub receptor_allowlist: Vec<String>,
    /// Explicit actuator allowlist; empty = builtin low-risk defaults only.
    #[serde(default)]
    pub actuator_allowlist: Vec<String>,
    /// Tool operations that may be invoked.
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    /// Channels the runtime may use at all.
    #[serde(default)]
    pub allowed_channels: Vec<String>,
    #[serde(default)]
    pub quiet_hours: Vec<QuietHours>,
    /// Per-channel limits keyed by channel name; `*` applies to all.
    #[serde(default)]
    pub channel_limits: BTreeMap<String, ChannelLimits>,
    /// Actions at or above this risk require explicit human approval.
    #[serde(default = "default_approval_risk")]
    pub require_approval_at: RiskClass,
    /// Global default TTL for plans/actions, ms.
    #[serde(default = "default_ttl")]
    pub default_ttl_ms: u64,
    /// Hard cap for any pattern's steps.
    #[serde(default = "default_pattern_steps")]
    pub max_pattern_steps: u32,
    /// Hard cap on total scheduled (queued+running) actions.
    #[serde(default = "default_max_scheduled")]
    pub max_scheduled_actions: u32,
    /// Monetary budget per session in USD.
    #[serde(default)]
    pub session_monetary_budget: f64,
    /// Deterministic delegation limits for agent sessions.
    #[serde(default)]
    pub delegation: crate::agent::DelegationLimits,
    /// Whether high-risk physical output may resume automatically after crash.
    /// This is deliberately not configurable to `true` via the public API.
    #[serde(default)]
    pub resume_high_risk_after_restart: bool,
    pub schema_version: String,
}

fn default_true() -> bool {
    true
}

fn default_approval_risk() -> RiskClass {
    RiskClass::High
}

fn default_ttl() -> u64 {
    30_000
}

fn default_pattern_steps() -> u32 {
    64
}

fn default_max_scheduled() -> u32 {
    128
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            initiative: InitiativeLevel::default(),
            receptor_allowlist: vec![
                "session.input".into(),
                "task.lifecycle".into(),
                "agent.activity".into(),
                "system.time".into(),
                "manual.event".into(),
                "webhook.input".into(),
                "mock.receptor".into(),
                "agent.session".into(),
                "desktop.companion.interaction".into(),
                "desktop.pointer.activity".into(),
            ],
            actuator_allowlist: vec![
                "conversation".into(),
                "web-ui".into(),
                "local-log".into(),
                "local-notification".into(),
                "mock.actuator".into(),
                "agent.delegate".into(),
            ],
            tool_allowlist: vec!["interaction.*".into()],
            allowed_channels: vec![
                "conversation".into(),
                "web-ui".into(),
                "notification".into(),
                "log".into(),
                "visual".into(),
                "agent".into(),
            ],
            quiet_hours: Vec::new(),
            channel_limits: BTreeMap::new(),
            require_approval_at: RiskClass::High,
            default_ttl_ms: default_ttl(),
            max_pattern_steps: default_pattern_steps(),
            max_scheduled_actions: default_max_scheduled(),
            session_monetary_budget: 0.0,
            delegation: crate::agent::DelegationLimits::default(),
            resume_high_risk_after_restart: false,
            schema_version: crate::SCHEMA_VERSION.to_string(),
        }
    }
}

/// One decision the governor made while bounding a request. The list of
/// decisions is the audit trail explaining requested -> effective.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum PolicyDecision {
    #[serde(rename_all = "camelCase")]
    Allowed { rule: String },
    #[serde(rename_all = "camelCase")]
    Clamped {
        rule: String,
        field: String,
        from: f64,
        to: f64,
    },
    #[serde(rename_all = "camelCase")]
    Silenced { rule: String, detail: String },
    #[serde(rename_all = "camelCase")]
    Blocked { rule: String, reason: String },
    #[serde(rename_all = "camelCase")]
    ApprovalRequired { rule: String, reason: String },
}

impl PolicyDecision {
    pub fn is_blocking(&self) -> bool {
        matches!(
            self,
            PolicyDecision::Blocked { .. } | PolicyDecision::ApprovalRequired { .. }
        )
    }
}

/// Result of evaluating a whole plan step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorizationOutcome {
    Authorized,
    Blocked,
    ApprovalRequired,
}

/// Consent scoping: what a session has agreed to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind", content = "id")]
pub enum ConsentScope {
    Channel(String),
    Actuator(String),
    Receptor(String),
    ToolOperation(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Consent {
    pub scope: ConsentScope,
    pub granted_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<Timestamp>,
}

impl Consent {
    pub fn is_active(&self, now: Timestamp) -> bool {
        self.revoked_at.is_none() && self.expires_at.map(|e| now <= e).unwrap_or(true)
    }
}
