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

/// 需要人類再確認的 name（§8）：`approval-request` 的線上名字。
///
/// `character.session.approval` 同時符合 `character.session.` 前綴，所以這條**必須**先判斷；
/// 反過來排的話，唯一真正存在的 approval name 會被歸成 `state-reconcile`，
/// 等於離線後可以自動對齊——那是人類決定，不得自動重送。
fn is_approval_name(name: &str) -> bool {
    name.starts_with("approval.") || name.ends_with(".approval")
}

/// 1.0 的固定歸類表。未知 name → `DropIfOffline`（最保守：不排隊、不重播）。
pub fn offline_policy(name: &str, has_consent_grant: bool) -> OfflinePolicy {
    if has_consent_grant {
        return OfflinePolicy::RequireReconfirmation;
    }
    match name {
        n if is_approval_name(n) => OfflinePolicy::RequireReconfirmation,
        n if n.starts_with("character.interaction.touch") => OfflinePolicy::ExpireByDeadline,
        n if n.starts_with("character.interaction.") => OfflinePolicy::DropIfOffline,
        n if n.starts_with("character.behavior.") => OfflinePolicy::DropIfOffline,
        n if n.starts_with("character.preference.") => OfflinePolicy::QueueIdempotent,
        n if n.starts_with("character.session.") => OfflinePolicy::StateReconcile,
        n if n.starts_with("task.") || n.starts_with("runtime.") => OfflinePolicy::StateReconcile,
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

    /// §8：`approval-request` 的線上名字是 `character.session.approval`，
    /// 它同時也符合 `character.session.` 前綴——先命中哪一條決定了它會不會被自動重送。
    #[test]
    fn approval_names_need_a_fresh_human_decision() {
        assert_eq!(
            offline_policy("character.session.approval", false),
            OfflinePolicy::RequireReconfirmation
        );
        assert_eq!(
            offline_policy("approval.request", false),
            OfflinePolicy::RequireReconfirmation
        );
        // 其餘 character.session.* 仍然是「以最新狀態對齊」。
        assert_eq!(
            offline_policy("character.session.presence", false),
            OfflinePolicy::StateReconcile
        );
    }
}
