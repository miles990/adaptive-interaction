//! §8 離線事件政策。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OfflinePolicy {
    DropIfOffline,
    ExpireByDeadline,
    QueueIdempotent,
    RequireReconfirmation,
    StateReconcile,
}

/// 1.0 的固定歸類表。未知 name → `DropIfOffline`（最保守：不排隊、不重播）。
pub fn offline_policy(name: &str, has_consent_grant: bool) -> OfflinePolicy {
    if has_consent_grant {
        return OfflinePolicy::RequireReconfirmation;
    }
    match name {
        n if n.starts_with("character.interaction.touch") => OfflinePolicy::ExpireByDeadline,
        n if n.starts_with("character.interaction.") => OfflinePolicy::DropIfOffline,
        n if n.starts_with("character.behavior.") => OfflinePolicy::DropIfOffline,
        n if n.starts_with("character.preference.") => OfflinePolicy::QueueIdempotent,
        n if n.starts_with("character.session.") => OfflinePolicy::StateReconcile,
        n if n.starts_with("task.") || n.starts_with("runtime.") => OfflinePolicy::StateReconcile,
        "approval.request" => OfflinePolicy::RequireReconfirmation,
        _ => OfflinePolicy::DropIfOffline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert_eq!(
            offline_policy("character.interaction.touch", false),
            OfflinePolicy::ExpireByDeadline
        );
        assert_eq!(
            offline_policy("character.interaction.dismiss", false),
            OfflinePolicy::DropIfOffline
        );
        assert_eq!(
            offline_policy("character.behavior.request", false),
            OfflinePolicy::DropIfOffline
        );
        assert_eq!(
            offline_policy("task.verified", false),
            OfflinePolicy::StateReconcile
        );
        assert_eq!(
            offline_policy("character.preference.volume", false),
            OfflinePolicy::QueueIdempotent
        );
        assert_eq!(
            offline_policy("character.behavior.request", true),
            OfflinePolicy::RequireReconfirmation
        );
        assert_eq!(
            offline_policy("weird.thing", false),
            OfflinePolicy::DropIfOffline
        );
    }
}
