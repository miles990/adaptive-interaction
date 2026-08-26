//! Structured error types shared across the platform.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReceptorError {
    #[error("receptor unavailable: {0}")]
    Unavailable(String),
    #[error("receptor read timed out after {0} ms")]
    Timeout(u64),
    #[error("receptor configuration invalid: {0}")]
    Config(String),
    #[error("receptor io error: {0}")]
    Io(String),
    #[error("receptor requires consent: {0}")]
    ConsentRequired(String),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Error)]
pub enum ActuatorError {
    #[error("actuator unavailable: {0}")]
    Unavailable(String),
    #[error("actuator rejected action: {0}")]
    Rejected(String),
    #[error("action {0} not found")]
    NotFound(String),
    #[error("actuator execution timed out after {0} ms")]
    Timeout(u64),
    #[error("bounded action expired before execution")]
    Expired,
    #[error("payload exceeds limit: {0}")]
    PayloadTooLarge(String),
    #[error("actuator io error: {0}")]
    Io(String),
    #[error("{0}")]
    Other(String),
}

/// Top-level domain error surfaced to API/CLI with stable codes.
#[derive(Debug, Error)]
pub enum DomainError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("policy blocked: {0}")]
    PolicyBlocked(String),
    #[error("approval required: {0}")]
    ApprovalRequired(String),
    #[error("consent required: {0}")]
    ConsentRequired(String),
    #[error("session inactive: {0}")]
    SessionInactive(String),
    #[error("expired: {0}")]
    Expired(String),
    #[error("unavailable: {0}")]
    Unavailable(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("receptor error: {0}")]
    Receptor(#[from] ReceptorError),
    #[error("actuator error: {0}")]
    Actuator(#[from] ActuatorError),
    #[error("emergency stop engaged")]
    EmergencyStop,
    #[error("internal error: {0}")]
    Internal(String),
}

impl DomainError {
    /// Stable machine-readable code for API/CLI mapping.
    pub fn code(&self) -> &'static str {
        match self {
            DomainError::NotFound(_) => "not_found",
            DomainError::Conflict(_) => "conflict",
            DomainError::Validation(_) => "validation_failed",
            DomainError::PolicyBlocked(_) => "policy_blocked",
            DomainError::ApprovalRequired(_) => "approval_required",
            DomainError::ConsentRequired(_) => "consent_required",
            DomainError::SessionInactive(_) => "session_inactive",
            DomainError::Expired(_) => "expired",
            DomainError::Unavailable(_) => "unavailable",
            DomainError::Storage(_) => "storage_error",
            DomainError::Receptor(_) => "receptor_error",
            DomainError::Actuator(_) => "actuator_error",
            DomainError::EmergencyStop => "emergency_stop",
            DomainError::Internal(_) => "internal_error",
        }
    }
}

pub type DomainResult<T> = Result<T, DomainError>;
