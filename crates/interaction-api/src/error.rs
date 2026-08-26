//! Domain error → HTTP mapping with stable machine-readable codes.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use interaction_core::DomainError;
use serde_json::json;

pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "missing or invalid bearer token (see state/api-token)".into(),
        }
    }
}

impl From<DomainError> for ApiError {
    fn from(err: DomainError) -> Self {
        let status = match &err {
            DomainError::NotFound(_) => StatusCode::NOT_FOUND,
            DomainError::Validation(_) => StatusCode::BAD_REQUEST,
            DomainError::PolicyBlocked(_)
            | DomainError::ApprovalRequired(_)
            | DomainError::ConsentRequired(_) => StatusCode::FORBIDDEN,
            DomainError::Conflict(_) | DomainError::SessionInactive(_) => StatusCode::CONFLICT,
            DomainError::Expired(_) => StatusCode::GONE,
            DomainError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            DomainError::EmergencyStop => StatusCode::LOCKED,
            DomainError::Receptor(_) | DomainError::Actuator(_) => StatusCode::BAD_GATEWAY,
            DomainError::Storage(_) | DomainError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            code: err.code(),
            message: err.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {"code": self.code, "message": self.message}
        });
        (self.status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
