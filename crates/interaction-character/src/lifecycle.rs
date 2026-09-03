//! §7 Adapter 生命週期狀態機與 §1 Character Role。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Adapter 生命週期（12 個主線狀態 ＋ `crashed`／`reconnecting`，共 14）。
///
/// 規格：`discovered → loading → validated → initializing → negotiating → ready → shown ⇄ hidden
/// → suspended ⇄ resumed → reconfiguring → disposed`，另有 `crashed`／`reconnecting`。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterLifecycleState {
    Discovered,
    Loading,
    Validated,
    Initializing,
    Negotiating,
    Ready,
    Shown,
    Hidden,
    Suspended,
    Resumed,
    Reconfiguring,
    Disposed,
    Crashed,
    Reconnecting,
}

impl AdapterLifecycleState {
    /// 全部 14 個狀態。
    pub const ALL: [AdapterLifecycleState; 14] = [
        AdapterLifecycleState::Discovered,
        AdapterLifecycleState::Loading,
        AdapterLifecycleState::Validated,
        AdapterLifecycleState::Initializing,
        AdapterLifecycleState::Negotiating,
        AdapterLifecycleState::Ready,
        AdapterLifecycleState::Shown,
        AdapterLifecycleState::Hidden,
        AdapterLifecycleState::Suspended,
        AdapterLifecycleState::Resumed,
        AdapterLifecycleState::Reconfiguring,
        AdapterLifecycleState::Disposed,
        AdapterLifecycleState::Crashed,
        AdapterLifecycleState::Reconnecting,
    ];

    /// `disposed` 是唯一終態。
    pub fn is_terminal(&self) -> bool {
        matches!(self, AdapterLifecycleState::Disposed)
    }

    /// 已完成協商、可接收 intent 的狀態。
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            AdapterLifecycleState::Ready
                | AdapterLifecycleState::Shown
                | AdapterLifecycleState::Hidden
                | AdapterLifecycleState::Resumed
        )
    }

    /// `self → next` 是否合法。
    pub fn can_transition_to(&self, next: AdapterLifecycleState) -> bool {
        use AdapterLifecycleState::*;
        if self.is_terminal() {
            return false;
        }
        if *self == next {
            return false;
        }
        match (self, next) {
            // 任何非終態都可以 dispose 或 crash。
            (_, Disposed) | (_, Crashed) => true,
            (Discovered, Loading) => true,
            (Loading, Validated) => true,
            (Validated, Initializing) => true,
            (Initializing, Negotiating) => true,
            (Negotiating, Ready) => true,
            (Ready, Shown) | (Ready, Hidden) | (Ready, Reconfiguring) => true,
            (Shown, Hidden) | (Hidden, Shown) => true,
            (Shown, Suspended) | (Hidden, Suspended) => true,
            (Shown, Reconfiguring) | (Hidden, Reconfiguring) => true,
            (Suspended, Resumed) => true,
            (Resumed, Shown) | (Resumed, Hidden) | (Resumed, Suspended) => true,
            (Resumed, Reconfiguring) => true,
            // 重新設定後可能需要重新協商（能力變了）或直接回 ready。
            (Reconfiguring, Negotiating) | (Reconfiguring, Ready) => true,
            // crash 後只能重連或丟棄；重連成功要重新 hello（negotiating）。
            (Crashed, Reconnecting) => true,
            (Reconnecting, Negotiating) => true,
            _ => false,
        }
    }
}

/// §1 Character Role。`observer`／`notification-only` 不送輸入。
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
    Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum CharacterRole {
    #[default]
    PrimaryCompanion,
    Familiar,
    Worker,
    Observer,
    NotificationOnly,
}

impl CharacterRole {
    pub const ALL: [CharacterRole; 5] = [
        CharacterRole::PrimaryCompanion,
        CharacterRole::Familiar,
        CharacterRole::Worker,
        CharacterRole::Observer,
        CharacterRole::NotificationOnly,
    ];

    /// §6：`observer`／`notification-only` 的角色輸入永不轉送。
    pub fn accepts_input(&self) -> bool {
        !matches!(
            self,
            CharacterRole::Observer | CharacterRole::NotificationOnly
        )
    }

    pub fn is_notification_only(&self) -> bool {
        matches!(self, CharacterRole::NotificationOnly)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            CharacterRole::PrimaryCompanion => "primary-companion",
            CharacterRole::Familiar => "familiar",
            CharacterRole::Worker => "worker",
            CharacterRole::Observer => "observer",
            CharacterRole::NotificationOnly => "notification-only",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AdapterLifecycleState::*;

    #[test]
    fn main_line_is_legal() {
        let path = [
            Discovered,
            Loading,
            Validated,
            Initializing,
            Negotiating,
            Ready,
            Shown,
            Hidden,
            Suspended,
            Resumed,
            Reconfiguring,
            Ready,
            Disposed,
        ];
        for pair in path.windows(2) {
            assert!(
                pair[0].can_transition_to(pair[1]),
                "{:?} -> {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn crash_and_reconnect_paths() {
        assert!(Shown.can_transition_to(Crashed));
        assert!(Crashed.can_transition_to(Reconnecting));
        assert!(Reconnecting.can_transition_to(Negotiating));
        assert!(!Reconnecting.can_transition_to(Ready));
        assert!(!Crashed.can_transition_to(Shown));
    }

    #[test]
    fn disposed_is_frozen_and_no_skips() {
        for s in AdapterLifecycleState::ALL {
            assert!(!Disposed.can_transition_to(s));
        }
        assert!(!Discovered.can_transition_to(Ready));
        assert!(!Ready.can_transition_to(Resumed));
        assert!(!Suspended.can_transition_to(Shown));
    }

    #[test]
    fn roles_input_filter() {
        assert!(CharacterRole::PrimaryCompanion.accepts_input());
        assert!(CharacterRole::Familiar.accepts_input());
        assert!(CharacterRole::Worker.accepts_input());
        assert!(!CharacterRole::Observer.accepts_input());
        assert!(!CharacterRole::NotificationOnly.accepts_input());
        assert_eq!(
            serde_json::to_string(&CharacterRole::NotificationOnly).unwrap_or_default(),
            "\"notification-only\""
        );
    }
}
