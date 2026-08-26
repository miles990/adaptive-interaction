//! Receptor / Actuator traits — the adapter contract.
//!
//! These are object-safe async traits. The core crate stays runtime-agnostic:
//! it only requires `Send` futures (works with Tokio or any executor).

use crate::{
    ActionId, ActionReceipt, ActuatorError, ActuatorManifest, BoundedAction, ComponentHealth,
    Observation, ReceptorError, ReceptorManifest, SessionId,
};
use async_trait::async_trait;
use futures::stream::BoxStream;

/// Context handed to receptors/actuators when a session starts.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub session_id: SessionId,
}

#[async_trait]
pub trait Receptor: Send + Sync {
    fn manifest(&self) -> ReceptorManifest;

    /// Called when a session begins; receptors may open connections here.
    async fn start(&self, context: SessionContext) -> Result<(), ReceptorError>;

    /// One-shot read (poll mode). Event/stream receptors may return their
    /// latest buffered observation.
    async fn read(&self) -> Result<Observation, ReceptorError>;

    async fn health(&self) -> ComponentHealth;

    async fn stop(&self) -> Result<(), ReceptorError>;
}

/// Optional streaming interface; not all receptors implement it.
pub trait StreamingReceptor: Receptor {
    fn subscribe(&self) -> BoxStream<'static, Observation>;
}

#[async_trait]
pub trait Actuator: Send + Sync {
    fn manifest(&self) -> ActuatorManifest;

    /// Execute an immutable bounded action. Implementations must respect
    /// `action.effective` exactly and must not exceed it.
    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError>;

    async fn status(&self) -> ComponentHealth;

    /// Cancel a previously accepted action if the driver supports it.
    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError>;

    /// Hard stop everything this actuator is doing. Must be fast, must not
    /// depend on queues, and must be safe to call repeatedly.
    async fn emergency_stop(&self) -> Result<(), ActuatorError>;
}
