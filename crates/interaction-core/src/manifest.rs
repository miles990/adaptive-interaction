//! Manifests: receptors, actuators and tool operations describe themselves
//! through manifests so the registry, orchestrator and UI can reason about
//! them without knowing driver details.

use crate::{ActuatorId, Availability, ComponentHealth, OperationId, ReceptorId, ToolId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Risk classification of an operation / actuator.
///
/// Ordering matters: later variants are riskier. `Ord` is derived so policy can
/// compare against thresholds.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RiskClass {
    /// Pure read, no side effect.
    ReadOnly,
    /// Local, reversible, low-noise side effect (log line, UI hint).
    Low,
    /// Side effect bounded by deterministic policy limits (sound, haptic, notification).
    BoundedSideEffect,
    /// Writes that leave the machine (webhook, message, GitHub comment).
    ExternalWrite,
    /// Hard-to-reverse or high-impact operations (merge PR, purchase, physical device).
    High,
    /// Destructive / safety-critical. Always requires explicit human approval.
    Critical,
}

/// How sensitive the data produced by a receptor is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Sensitivity {
    /// Non-personal machine state (system time, task lifecycle).
    Public,
    /// Workspace-level information (file names, task titles).
    Internal,
    /// Personal but not biometric (user text, presence).
    Personal,
    /// Camera, microphone, location, physiological signals.
    Intimate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ReceptorMode {
    Poll,
    Event,
    Stream,
}

/// Interaction channels are open-ended strings; these constants cover builtins.
pub mod channels {
    pub const CONVERSATION: &str = "conversation";
    pub const WEB_UI: &str = "web-ui";
    pub const NOTIFICATION: &str = "notification";
    pub const LOG: &str = "log";
    pub const AUDIO: &str = "audio";
    pub const VISUAL: &str = "visual";
    pub const LIGHT: &str = "light";
    pub const HAPTIC: &str = "haptic";
    pub const WEBHOOK: &str = "webhook";
    pub const DESKTOP_PET: &str = "desktop-pet";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReceptorManifest {
    pub id: ReceptorId,
    pub name: String,
    pub description: String,
    /// Free-form category, e.g. `session`, `task`, `environment`, `device`.
    pub category: String,
    /// Fact keys this receptor can provide, e.g. `["event", "state"]`.
    #[serde(default)]
    pub provides: Vec<String>,
    pub mode: ReceptorMode,
    pub sensitivity: Sensitivity,
    pub requires_consent: bool,
    /// Typical end-to-end latency in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Suggested polling interval for `mode = poll`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_interval_ms: Option<u64>,
    /// JSON Schema describing driver configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_schema: Option<Value>,
    #[serde(default = "default_health")]
    pub health: ComponentHealth,
    #[serde(default = "default_availability")]
    pub availability: Availability,
    /// Driver identifier, e.g. `builtin.system-time`.
    pub driver: String,
    pub version: String,
    pub schema_version: String,
    /// Optional human-readable layer (presentation + data semantics).
    /// Never a safety truth source; formal fields above always win.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human: Option<crate::HumanMeta>,
}

fn default_health() -> ComponentHealth {
    ComponentHealth::healthy()
}

fn default_availability() -> Availability {
    Availability::Available
}

/// Cost hints used by the orchestrator's utility scoring.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CostDescriptor {
    /// Monetary cost per invocation in USD (0 for local channels).
    #[serde(default)]
    pub monetary_per_invocation: f64,
    /// Abstract resource cost 0..1 (CPU, battery, attention budget).
    #[serde(default)]
    pub resource: f64,
}

/// Deterministic per-actuator limits enforced by the safety governor.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActuatorLimits {
    /// Hard ceiling on normalized magnitude 0..1 (device safe limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_magnitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    /// Max invocations per rolling hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_hour: Option<u32>,
    /// Max steps in a pattern timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pattern_steps: Option<u32>,
    /// Max payload size in bytes accepted by the driver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_payload_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActuatorManifest {
    pub id: ActuatorId,
    pub name: String,
    pub description: String,
    /// Primary interaction channel, see [`channels`].
    pub channel: String,
    /// Capability tags, e.g. `["text", "pattern", "cancel"]`.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// JSON Schema for driver-specific action parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters_schema: Option<Value>,
    pub supports_cancel: bool,
    pub supports_pattern: bool,
    pub requires_consent: bool,
    /// True when effects leave the local machine.
    pub external_side_effect: bool,
    pub reversible: bool,
    pub risk_class: RiskClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub cost: CostDescriptor,
    #[serde(default)]
    pub limits: ActuatorLimits,
    #[serde(default = "default_health")]
    pub health: ComponentHealth,
    #[serde(default = "default_availability")]
    pub availability: Availability,
    pub driver: String,
    pub version: String,
    pub schema_version: String,
    /// Optional human-readable layer (presentation + effect semantics).
    /// Never a safety truth source; formal fields above always win.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human: Option<crate::HumanMeta>,
}

/// Role a tool operation plays in the interaction loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ToolRole {
    Receptor,
    Actuator,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolOperationManifest {
    pub tool: ToolId,
    pub operation: OperationId,
    /// Fully qualified stable name, e.g. `interaction.observe`.
    pub name: String,
    pub description: String,
    pub roles: Vec<ToolRole>,
    pub input_schema: Value,
    pub output_schema: Value,
    pub risk: RiskClass,
    pub reversible: bool,
    pub external_side_effect: bool,
    pub requires_approval: bool,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostDescriptor>,
    #[serde(default = "default_availability")]
    pub availability: Availability,
    pub schema_version: String,
    /// Optional human-readable layer. Never a safety truth source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human: Option<crate::HumanMeta>,
}

/// Generic string-keyed metadata bag used in several manifests.
pub type Metadata = BTreeMap<String, Value>;
