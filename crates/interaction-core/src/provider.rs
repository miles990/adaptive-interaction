//! Capability Provider model: WHO provides a receptor/actuator/tool.
//!
//! A provider can be a local adapter, an external device, an external
//! service, another application, an AI provider / agent / session, the
//! desktop companion, or a human input surface. Providers have an explicit
//! lifecycle — discovered ≠ paired ≠ installed ≠ enabled ≠ authorized — and
//! none of those steps may be merged into one implicit action.

use crate::human::HumanMeta;
use crate::{ProviderId, Timestamp};
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    /// Built-in local adapter (same process, same trust domain).
    Local,
    /// External physical device (serial/BLE/network).
    Device,
    /// External network service.
    Service,
    /// Another application on this machine.
    Application,
    /// An AI model/service provider (e.g. an API vendor account).
    AiProvider,
    /// A configured agent profile under an AI provider.
    AiAgent,
    /// One live delegated session of an agent (short-lived, leased).
    AiSession,
    /// The desktop companion surface.
    Companion,
    /// A human input surface.
    Human,
    #[default]
    Unknown,
}

/// How much identity assurance we have. Trust NEVER implies authorization —
/// consent and policy still gate every capability individually.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum TrustLevel {
    #[default]
    Untrusted,
    /// Seen on the network / announced, identity unverified.
    Discovered,
    /// Completed a pairing ceremony (shared secret / key exchange).
    Paired,
    /// Identity verified against a stored fingerprint on every session.
    Verified,
    /// Compiled into this runtime.
    Builtin,
}

/// Provider lifecycle (spec §19.1). State transitions are explicit; the
/// registry refuses shortcuts like discovered → available.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderState {
    #[default]
    Discovered,
    Unpaired,
    Paired,
    Installed,
    Disabled,
    Available,
    Busy,
    Degraded,
    Disconnected,
    Expired,
    Revoked,
    Closed,
}

impl ProviderState {
    /// Legal next states (deterministic lifecycle; no shortcuts).
    pub fn can_transition_to(self, next: ProviderState) -> bool {
        use ProviderState::*;
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (Discovered, Unpaired)
                | (Discovered, Paired)
                | (Unpaired, Paired)
                | (Paired, Installed)
                | (Installed, Disabled)
                | (Disabled, Available)
                | (Installed, Available)
                | (Available, Busy)
                | (Busy, Available)
                | (Available, Degraded)
                | (Degraded, Available)
                | (Available, Disconnected)
                | (Degraded, Disconnected)
                | (Busy, Disconnected)
                | (Disconnected, Available)
                | (Disconnected, Degraded)
                | (Available, Disabled)
                | (Degraded, Disabled)
                | (Disconnected, Disabled)
                // Expiry / revocation / closure can hit from any live state.
                | (Paired, Revoked)
                | (Installed, Revoked)
                | (Disabled, Revoked)
                | (Available, Revoked)
                | (Busy, Revoked)
                | (Degraded, Revoked)
                | (Disconnected, Revoked)
                | (Available, Expired)
                | (Busy, Expired)
                | (Degraded, Expired)
                | (Disconnected, Expired)
                | (Installed, Expired)
                | (Disabled, Expired)
                | (Expired, Closed)
                | (Revoked, Closed)
                | (Available, Closed)
                | (Busy, Closed)
                | (Degraded, Closed)
                | (Disconnected, Closed)
                | (Disabled, Closed)
                | (Installed, Closed)
        )
    }

    /// Only these states may execute/observe anything.
    pub fn is_operational(self) -> bool {
        matches!(
            self,
            ProviderState::Available | ProviderState::Busy | ProviderState::Degraded
        )
    }
}

/// Stable identity of a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderIdentity {
    pub id: ProviderId,
    pub kind: ProviderKind,
    pub display_name: String,
    #[serde(default)]
    pub trust_level: TrustLevel,
    /// Where it came from: `builtin`, `local-network`, `usb`, `cloud:host`, …
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub version: String,
    /// Public-key or shared-secret fingerprint established at pairing.
    /// An IP address is NEVER an identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human: Option<HumanMeta>,
}

/// Full provider record surfaced by the registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDescriptor {
    pub identity: ProviderIdentity,
    pub state: ProviderState,
    /// Capability ids this provider contributes (individually authorized).
    #[serde(default)]
    pub receptors: Vec<String>,
    #[serde(default)]
    pub actuators: Vec<String>,
    #[serde(default)]
    pub tool_operations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_refuses_shortcuts() {
        use ProviderState::*;
        // The mandated ceremony: discover → pair → install → (disabled) → enable.
        assert!(Discovered.can_transition_to(Paired));
        assert!(Paired.can_transition_to(Installed));
        assert!(Installed.can_transition_to(Disabled));
        assert!(Disabled.can_transition_to(Available));
        // Shortcuts are refused: pairing/install/enable can't be merged.
        assert!(!Discovered.can_transition_to(Installed));
        assert!(!Discovered.can_transition_to(Available));
        assert!(!Paired.can_transition_to(Available));
        assert!(!Unpaired.can_transition_to(Available));
        // Revocation is terminal-ish: only closure follows.
        assert!(Available.can_transition_to(Revoked));
        assert!(Revoked.can_transition_to(Closed));
        assert!(!Revoked.can_transition_to(Available));
        assert!(!Expired.can_transition_to(Available));
    }

    #[test]
    fn only_operational_states_execute() {
        use ProviderState::*;
        for s in [
            Discovered,
            Unpaired,
            Paired,
            Installed,
            Disabled,
            Expired,
            Revoked,
            Closed,
            Disconnected,
        ] {
            assert!(!s.is_operational(), "{s:?} must not be operational");
        }
        for s in [Available, Busy, Degraded] {
            assert!(s.is_operational());
        }
    }

    #[test]
    fn identity_roundtrips_and_defaults_conservatively() {
        let json =
            r#"{"id":"provider.device.desk-01","kind":"device","displayName":"書桌互動裝置"}"#;
        let id: ProviderIdentity = serde_json::from_str(json).unwrap();
        assert_eq!(id.kind, ProviderKind::Device);
        assert_eq!(id.trust_level, TrustLevel::Untrusted); // default: untrusted
        assert!(id.fingerprint.is_none());
        let back = serde_json::to_string(&id).unwrap();
        let again: ProviderIdentity = serde_json::from_str(&back).unwrap();
        assert_eq!(id, again);
    }
}
