//! Component health & availability.

use crate::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Offline,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComponentHealth {
    pub status: HealthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<Timestamp>,
}

impl ComponentHealth {
    pub fn healthy() -> Self {
        Self {
            status: HealthStatus::Healthy,
            message: None,
            checked_at: None,
        }
    }

    pub fn offline(message: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Offline,
            message: Some(message.into()),
            checked_at: None,
        }
    }

    pub fn degraded(message: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Degraded,
            message: Some(message.into()),
            checked_at: None,
        }
    }

    pub fn at(mut self, ts: Timestamp) -> Self {
        self.checked_at = Some(ts);
        self
    }

    pub fn is_usable(&self) -> bool {
        matches!(self.status, HealthStatus::Healthy | HealthStatus::Degraded)
    }
}

/// Whether a capability is currently available for planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Availability {
    Available,
    Disabled,
    Offline,
    /// Registered but requires consent that has not been granted in the session.
    ConsentRequired,
    /// Permission was revoked at runtime.
    Revoked,
    Unknown,
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Availability::Available)
    }
}
