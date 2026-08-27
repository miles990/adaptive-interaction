//! Runtime events published on the SSE stream and consumed by the UI timeline.

use crate::{CorrelationId, EventId, SessionId, Timestamp, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub enum EventType {
    #[serde(rename = "receptor.registered")]
    ReceptorRegistered,
    #[serde(rename = "receptor.online")]
    ReceptorOnline,
    #[serde(rename = "receptor.offline")]
    ReceptorOffline,
    #[serde(rename = "receptor.observation")]
    ReceptorObservation,
    #[serde(rename = "actuator.registered")]
    ActuatorRegistered,
    #[serde(rename = "actuator.online")]
    ActuatorOnline,
    #[serde(rename = "actuator.offline")]
    ActuatorOffline,
    #[serde(rename = "capability.changed")]
    CapabilityChanged,
    #[serde(rename = "provider.registered")]
    ProviderRegistered,
    #[serde(rename = "provider.state-changed")]
    ProviderStateChanged,
    #[serde(rename = "sensor.started")]
    SensorStarted,
    #[serde(rename = "sensor.stopped")]
    SensorStopped,
    #[serde(rename = "plan.created")]
    PlanCreated,
    #[serde(rename = "plan.blocked")]
    PlanBlocked,
    #[serde(rename = "plan.authorized")]
    PlanAuthorized,
    #[serde(rename = "action.accepted")]
    ActionAccepted,
    #[serde(rename = "action.dispatched")]
    ActionDispatched,
    #[serde(rename = "action.acknowledged")]
    ActionAcknowledged,
    #[serde(rename = "action.observed")]
    ActionObserved,
    #[serde(rename = "action.completed")]
    ActionCompleted,
    #[serde(rename = "action.failed")]
    ActionFailed,
    #[serde(rename = "action.uncertain")]
    ActionUncertain,
    #[serde(rename = "action.cancelled")]
    ActionCancelled,
    #[serde(rename = "action.expired")]
    ActionExpired,
    #[serde(rename = "recipe.changed")]
    RecipeChanged,
    #[serde(rename = "policy.changed")]
    PolicyChanged,
    #[serde(rename = "consent.changed")]
    ConsentChanged,
    #[serde(rename = "session.started")]
    SessionStarted,
    #[serde(rename = "session.stopped")]
    SessionStopped,
    #[serde(rename = "emergency.stop")]
    EmergencyStop,
    #[serde(rename = "proactive.paused")]
    ProactivePaused,
    #[serde(rename = "proactive.resumed")]
    ProactiveResumed,
    #[serde(rename = "ai.assist.requested")]
    AiAssistRequested,
    #[serde(rename = "ai.assist.resolved")]
    AiAssistResolved,
    /// A presentation command for the companion surface (bubble, animation,
    /// state intent…). Payload carries the actionId the surface must ack.
    #[serde(rename = "presentation.command")]
    PresentationCommand,
    /// Companion surface presence changed (connected / visible / stale).
    #[serde(rename = "presentation.state")]
    PresentationState,
    /// 知識系統變化（素材/候選/複審/發布/過期），payload 帶 knowledgeReceipt。
    #[serde(rename = "knowledge.updated")]
    KnowledgeUpdated,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::ReceptorRegistered => "receptor.registered",
            EventType::ReceptorOnline => "receptor.online",
            EventType::ReceptorOffline => "receptor.offline",
            EventType::ReceptorObservation => "receptor.observation",
            EventType::ActuatorRegistered => "actuator.registered",
            EventType::ActuatorOnline => "actuator.online",
            EventType::ActuatorOffline => "actuator.offline",
            EventType::CapabilityChanged => "capability.changed",
            EventType::ProviderRegistered => "provider.registered",
            EventType::ProviderStateChanged => "provider.state-changed",
            EventType::SensorStarted => "sensor.started",
            EventType::SensorStopped => "sensor.stopped",
            EventType::PlanCreated => "plan.created",
            EventType::PlanBlocked => "plan.blocked",
            EventType::PlanAuthorized => "plan.authorized",
            EventType::ActionAccepted => "action.accepted",
            EventType::ActionDispatched => "action.dispatched",
            EventType::ActionAcknowledged => "action.acknowledged",
            EventType::ActionObserved => "action.observed",
            EventType::ActionCompleted => "action.completed",
            EventType::ActionFailed => "action.failed",
            EventType::ActionUncertain => "action.uncertain",
            EventType::ActionCancelled => "action.cancelled",
            EventType::ActionExpired => "action.expired",
            EventType::RecipeChanged => "recipe.changed",
            EventType::PolicyChanged => "policy.changed",
            EventType::ConsentChanged => "consent.changed",
            EventType::SessionStarted => "session.started",
            EventType::SessionStopped => "session.stopped",
            EventType::EmergencyStop => "emergency.stop",
            EventType::ProactivePaused => "proactive.paused",
            EventType::ProactiveResumed => "proactive.resumed",
            EventType::AiAssistRequested => "ai.assist.requested",
            EventType::AiAssistResolved => "ai.assist.resolved",
            EventType::PresentationCommand => "presentation.command",
            EventType::PresentationState => "presentation.state",
            EventType::KnowledgeUpdated => "knowledge.updated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvent {
    pub event_id: EventId,
    /// Monotonically increasing sequence for `Last-Event-ID` resume.
    pub sequence: u64,
    pub event_type: EventType,
    pub timestamp: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
    pub schema_version: String,
    #[serde(default)]
    pub payload: Value,
}

impl RuntimeEvent {
    pub fn new(event_type: EventType, timestamp: Timestamp, payload: Value) -> Self {
        Self {
            event_id: EventId::generate(),
            sequence: 0,
            event_type,
            timestamp,
            session_id: None,
            correlation_id: None,
            schema_version: SCHEMA_VERSION.to_string(),
            payload,
        }
    }

    pub fn with_session(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn with_correlation(mut self, correlation_id: CorrelationId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }
}
