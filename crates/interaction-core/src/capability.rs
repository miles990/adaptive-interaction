//! Capability snapshots: what the AI is allowed to assume exists right now.

use crate::{
    ActuatorManifest, PolicyConfig, ReceptorManifest, SessionId, Timestamp, ToolOperationManifest,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    /// When true, include disabled/offline capabilities with their status.
    #[serde(default)]
    pub include_unavailable: bool,
}

/// A human-readable constraint the AI should surface when planning
/// (e.g. "quiet hours active until 08:00").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityConstraint {
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySnapshot {
    pub receptors: Vec<ReceptorManifest>,
    pub actuators: Vec<ActuatorManifest>,
    pub tool_operations: Vec<ToolOperationManifest>,
    #[serde(default)]
    pub constraints: Vec<CapabilityConstraint>,
    /// Policy view relevant to planning (limits, initiative, quiet hours).
    pub session_policy: PolicyConfig,
    pub generated_at: Timestamp,
    /// Monotonic-ish version: changes whenever registry content changes.
    pub version: u64,
    pub schema_version: String,
}

impl CapabilitySnapshot {
    /// Snapshots older than this should be refreshed before planning.
    pub const MAX_AGE_MS: u64 = 60_000;

    pub fn is_fresh(&self, now: Timestamp) -> bool {
        let age = now.signed_duration_since(self.generated_at);
        age.num_milliseconds() >= 0 && (age.num_milliseconds() as u64) <= Self::MAX_AGE_MS
    }
}
