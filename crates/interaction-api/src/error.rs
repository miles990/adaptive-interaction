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
            message:
                "missing or invalid bearer token (see state/api-token or state/api-agent-token)"
                    .into(),
        }
    }

    pub fn forbidden_scope() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "token_scope_forbidden",
            message: "agent token cannot perform this human/control-center operation".into(),
        }
    }

    /// Character adapter token：只能 POST 回執／事件與開 `/v1/character/ws`。
    pub fn forbidden_adapter_scope() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "token_scope_forbidden",
            message: "character adapter token can only post /v1/character/receipts and /v1/character/events or open /v1/character/ws".into(),
        }
    }

    /// `/v1/character/ws` 拒絕 human／agent token（只收 adapter token）。
    pub fn adapter_token_required() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "/v1/character/ws accepts only a character adapter token (POST /v1/character/adapters); human and agent tokens are refused".into(),
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
