//! Action receipts and the action status state machine.
//!
//! The state machine encodes the crucial distinction between "the API accepted
//! the request" and "the effect was actually observed". `Accepted`/queued MUST
//! NEVER be reported as completed.

use crate::{
    ActionId, ActionParameters, ActuatorId, BoundedAction, CorrelationId, PlanId, PolicyDecision,
    SessionId, Timestamp, VerificationEvidence, SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ActionStatus {
    /// Plan step exists but has not been authorized.
    Planned,
    /// Policy governor approved and produced a bounded action.
    Authorized,
    /// Runtime accepted the action into its queue. NOT completion.
    Accepted,
    /// Driver has sent the command toward the target.
    Dispatched,
    /// Target (device/service) confirmed receipt of the command.
    Acknowledged,
    /// The effect was observed in the environment.
    Observed,
    /// Terminal: goal achieved and verified to the configured standard.
    Completed,
    /// Terminal: policy refused the action.
    Blocked,
    /// Terminal: execution failed.
    Failed,
    /// Terminal: outcome could not be determined.
    Uncertain,
    /// Terminal: cancelled by user, AI, policy or shutdown.
    Cancelled,
    /// Terminal: TTL elapsed before completion.
    Expired,
    /// Terminal: halted by emergency stop.
    Stopped,
}

impl ActionStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ActionStatus::Completed
                | ActionStatus::Blocked
                | ActionStatus::Failed
                | ActionStatus::Uncertain
                | ActionStatus::Cancelled
                | ActionStatus::Expired
                | ActionStatus::Stopped
        )
    }

    /// Whether `self -> next` is a legal transition.
    pub fn can_transition_to(&self, next: ActionStatus) -> bool {
        use ActionStatus::*;
        if self.is_terminal() {
            return false;
        }
        match (self, next) {
            // Happy path is strictly forward.
            (Planned, Authorized) | (Planned, Blocked) => true,
            (Authorized, Accepted) => true,
            (Accepted, Dispatched) => true,
            (Dispatched, Acknowledged) => true,
            (Acknowledged, Observed) => true,
            (Observed, Completed) => true,
            // Verification may complete directly from acknowledged when the
            // configured standard is "acknowledged is enough" (best-effort).
            (Acknowledged, Completed) => true,
            // Any non-terminal state can fail / cancel / expire / stop / go uncertain.
            (_, Failed) | (_, Cancelled) | (_, Expired) | (_, Stopped) | (_, Uncertain) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptError {
    pub code: String,
    pub message: String,
    pub at: Timestamp,
}

/// Full lifecycle record of one action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActionReceipt {
    pub action_id: ActionId,
    pub plan_id: PlanId,
    pub session_id: SessionId,
    pub actuator_id: ActuatorId,
    pub intent: String,
    pub requested_parameters: ActionParameters,
    pub effective_bounded_parameters: ActionParameters,
    #[serde(default)]
    pub policy_decisions: Vec<PolicyDecision>,
    pub current_status: ActionStatus,
    /// Timestamp of each state entered, in order.
    pub timestamps: Vec<(ActionStatus, Timestamp)>,
    #[serde(default)]
    pub errors: Vec<ReceiptError>,
    /// Raw driver response snippets (redacted upstream if sensitive).
    #[serde(default)]
    pub driver_response: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationEvidence>,
    /// TTL deadline inherited from the bounded action; the watchdog expires
    /// non-terminal receipts past this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    pub correlation_id: CorrelationId,
    pub schema_version: String,
}

impl ActionReceipt {
    pub fn for_action(action: &BoundedAction, now: Timestamp) -> Self {
        Self {
            action_id: action.action_id.clone(),
            plan_id: action.plan_id.clone(),
            session_id: action.session_id.clone(),
            actuator_id: action.actuator_id.clone(),
            intent: action.intent.clone(),
            requested_parameters: action.requested.clone(),
            effective_bounded_parameters: action.effective.clone(),
            policy_decisions: action.policy_decisions.clone(),
            current_status: ActionStatus::Authorized,
            timestamps: vec![(ActionStatus::Authorized, now)],
            errors: Vec::new(),
            driver_response: BTreeMap::new(),
            verification: None,
            expires_at: Some(action.expires_at),
            correlation_id: action.correlation_id.clone(),
            schema_version: SCHEMA_VERSION.to_string(),
        }
    }

    /// Apply a transition; rejects illegal moves.
    pub fn transition(&mut self, next: ActionStatus, now: Timestamp) -> Result<(), IllegalTransition> {
        if !self.current_status.can_transition_to(next) {
            return Err(IllegalTransition { from: self.current_status, to: next });
        }
        self.current_status = next;
        self.timestamps.push((next, now));
        Ok(())
    }

    pub fn push_error(&mut self, code: impl Into<String>, message: impl Into<String>, now: Timestamp) {
        self.errors.push(ReceiptError { code: code.into(), message: message.into(), at: now });
    }

    pub fn is_terminal(&self) -> bool {
        self.current_status.is_terminal()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("illegal action status transition {from:?} -> {to:?}")]
pub struct IllegalTransition {
    pub from: ActionStatus,
    pub to: ActionStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_is_legal() {
        use ActionStatus::*;
        let path = [Planned, Authorized, Accepted, Dispatched, Acknowledged, Observed, Completed];
        for pair in path.windows(2) {
            assert!(pair[0].can_transition_to(pair[1]), "{:?} -> {:?}", pair[0], pair[1]);
        }
    }

    #[test]
    fn queued_is_not_completed() {
        // Accepted (queued) must not jump straight to Completed.
        assert!(!ActionStatus::Accepted.can_transition_to(ActionStatus::Completed));
        assert!(!ActionStatus::Dispatched.can_transition_to(ActionStatus::Completed));
    }

    #[test]
    fn terminal_states_are_frozen() {
        for terminal in [
            ActionStatus::Completed,
            ActionStatus::Cancelled,
            ActionStatus::Stopped,
            ActionStatus::Expired,
            ActionStatus::Failed,
        ] {
            assert!(!terminal.can_transition_to(ActionStatus::Accepted));
            assert!(!terminal.can_transition_to(ActionStatus::Completed));
        }
    }

    #[test]
    fn emergency_stop_wins_from_anywhere_non_terminal() {
        for s in [
            ActionStatus::Planned,
            ActionStatus::Authorized,
            ActionStatus::Accepted,
            ActionStatus::Dispatched,
            ActionStatus::Acknowledged,
            ActionStatus::Observed,
        ] {
            assert!(s.can_transition_to(ActionStatus::Stopped), "{s:?}");
        }
    }
}
