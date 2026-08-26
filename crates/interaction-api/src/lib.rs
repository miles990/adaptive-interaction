//! Local-first HTTP API. Binds 127.0.0.1 by default; every /v1 route except
//! health/ready requires the local capability token (Bearer). SSE event stream
//! supports Last-Event-ID resume.

mod dto;
mod error;
mod routes;
mod sse;

use axum::http::HeaderValue;
use axum::middleware;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use interaction_runtime::Runtime;
use std::net::SocketAddr;
use tower_http::limit::RequestBodyLimitLayer;

pub use error::ApiError;

#[derive(Clone)]
pub struct ApiState {
    pub runtime: Runtime,
    pub token: String,
}

pub const MAX_BODY_BYTES: usize = 256 * 1024;

pub fn router(state: ApiState) -> Router {
    let authed = Router::new()
        .route("/v1/status", get(routes::status))
        .route("/v1/catalog", get(routes::catalog))
        .route("/v1/capabilities/human", get(routes::capabilities_human))
        .route("/v1/ui/preferences", get(routes::ui_preferences_get))
        .route("/v1/ui/preferences", patch(routes::ui_preferences_patch))
        .route("/v1/onboarding", get(routes::onboarding_get))
        .route(
            "/v1/onboarding/draft",
            axum::routing::put(routes::onboarding_draft_put),
        )
        .route("/v1/onboarding/commit", post(routes::onboarding_commit))
        .route("/v1/pause", get(routes::pause_get))
        .route("/v1/pause", post(routes::pause_set))
        .route("/v1/pause/clear", post(routes::pause_clear))
        .route(
            "/v1/capabilities/{kind}/{id}/ai-description",
            axum::routing::put(routes::ai_description_put),
        )
        .route("/v1/ai-assists", get(routes::ai_assists_list))
        .route(
            "/v1/ai-assists/{id}/resolve",
            post(routes::ai_assist_resolve),
        )
        .route("/v1/recipes/convert", post(routes::recipe_convert))
        .route("/v1/recipes/{id}/summary", get(routes::recipe_summary))
        .route(
            "/v1/recipes/{id}/simulate-scenario",
            post(routes::recipe_simulate_scenario),
        )
        .route("/v1/capabilities", get(routes::capabilities))
        .route("/v1/receptors", get(routes::receptors_list))
        .route("/v1/receptors", post(routes::receptor_create))
        .route("/v1/receptors/{id}", get(routes::receptor_inspect))
        .route("/v1/receptors/{id}", patch(routes::receptor_patch))
        .route("/v1/receptors/{id}", delete(routes::receptor_delete))
        .route("/v1/receptors/{id}/test", post(routes::receptor_test))
        .route("/v1/receptors/{id}/read", post(routes::receptor_read))
        .route("/v1/receptors/{id}/push", post(routes::receptor_push))
        .route("/v1/actuators", get(routes::actuators_list))
        .route("/v1/actuators", post(routes::actuator_create))
        .route("/v1/actuators/{id}", get(routes::actuator_inspect))
        .route("/v1/actuators/{id}", patch(routes::actuator_patch))
        .route("/v1/actuators/{id}", delete(routes::actuator_delete))
        .route("/v1/actuators/{id}/test", post(routes::actuator_test))
        .route("/v1/recipes", get(routes::recipes_list))
        .route("/v1/recipes", post(routes::recipe_create))
        .route("/v1/recipes/validate", post(routes::recipe_validate))
        .route("/v1/recipes/{id}", get(routes::recipe_get))
        .route("/v1/recipes/{id}", patch(routes::recipe_patch))
        .route("/v1/recipes/{id}", delete(routes::recipe_delete))
        .route("/v1/recipes/{id}/simulate", post(routes::recipe_simulate))
        .route("/v1/recipes/{id}/run", post(routes::recipe_run))
        .route("/v1/observations/query", post(routes::observations_query))
        .route("/v1/plans", post(routes::plan_create))
        .route("/v1/plans/{id}", get(routes::plan_get))
        .route("/v1/plans/{id}/simulate", post(routes::plan_simulate))
        .route("/v1/plans/{id}/execute", post(routes::plan_execute))
        .route("/v1/actions", get(routes::actions_list))
        .route("/v1/actions/{id}", get(routes::action_get))
        .route("/v1/actions/{id}/cancel", post(routes::action_cancel))
        .route("/v1/actions/{id}/verify", post(routes::action_verify))
        .route("/v1/policy", get(routes::policy_get))
        .route("/v1/policy", patch(routes::policy_patch))
        .route("/v1/session/start", post(routes::session_start))
        .route("/v1/session", get(routes::session_get))
        .route("/v1/session/consent", post(routes::session_consent))
        .route("/v1/session/revoke", post(routes::session_revoke))
        .route("/v1/session/stop", post(routes::session_stop))
        .route("/v1/tools", get(routes::tools_list))
        .route("/v1/tools/export/{format}", get(routes::tools_export))
        .route("/v1/tools/{name}", get(routes::tool_get))
        .route("/v1/tools/{name}/call", post(routes::tool_call))
        .route("/v1/emergency-stop", post(routes::emergency_stop))
        .route(
            "/v1/emergency-stop/clear",
            post(routes::emergency_stop_clear),
        )
        .route("/v1/stop-all", post(routes::stop_all))
        .route("/v1/outbox", get(routes::outbox))
        .route("/v1/audit", get(routes::audit))
        .route("/v1/events", get(sse::events))
        .route("/v1/openapi.json", get(routes::openapi))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .route("/health", get(routes::health))
        .route("/ready", get(routes::ready))
        .route("/v1/health", get(routes::health))
        .route("/v1/ready", get(routes::ready))
        .merge(authed)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(cors_layer())
        .with_state(state)
}

fn cors_layer() -> tower_http::cors::CorsLayer {
    // Only local UI origins; the API never binds beyond loopback by default.
    tower_http::cors::CorsLayer::new()
        .allow_origin([
            HeaderValue::from_static("tauri://localhost"),
            HeaderValue::from_static("http://localhost:1420"),
            HeaderValue::from_static("http://127.0.0.1:1420"),
        ])
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}

async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<ApiState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let authorized = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|candidate| constant_time_eq(candidate.as_bytes(), state.token.as_bytes()))
        .unwrap_or(false);
    if !authorized {
        return ApiError::unauthorized().into_response();
    }
    next.run(request).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

use axum::response::IntoResponse;

/// Bind and serve until the runtime's shutdown token fires.
/// Returns the actually bound address (useful with port 0 in tests).
pub async fn serve(
    runtime: Runtime,
    host: &str,
    port: u16,
    token: String,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>), std::io::Error> {
    let state = ApiState {
        runtime: runtime.clone(),
        token,
    };
    let app = router(state);
    let listener = tokio::net::TcpListener::bind((host, port)).await?;
    let addr = listener.local_addr()?;
    let shutdown = runtime.shutdown_token.clone();
    let handle = tokio::spawn(async move {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await;
        if let Err(e) = result {
            tracing::error!(error = %e, "api server error");
        }
    });
    Ok((addr, handle))
}
