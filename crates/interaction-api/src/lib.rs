//! Local-first HTTP API. Binds 127.0.0.1 by default; every /v1 route except
//! health/ready requires the local capability token (Bearer). SSE event stream
//! supports Last-Event-ID resume.

mod character_ws;
mod dto;
mod error;
mod routes;
mod sse;

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
    pub agent_token: String,
}

#[derive(Clone)]
pub enum AuthPrincipal {
    Human,
    LegacyAgent,
    AgentSession(interaction_runtime::agents::AgentSessionCapability),
    /// 外部 Character adapter token（sha256 儲存）：只能 POST 自己 instance 的
    /// 回執／事件與開 `/v1/character/ws`；拿不到任何人類資料、不能呼叫
    /// actuator、不能改 policy／consent／estop、不能 verify。
    CharacterAdapter {
        adapter_id: String,
    },
}

#[derive(Clone)]
pub struct AuthContext {
    pub principal: AuthPrincipal,
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
        .route("/v1/onboarding/preview", post(routes::onboarding_preview))
        .route("/v1/onboarding/commit", post(routes::onboarding_commit))
        .route("/v1/pause", get(routes::pause_get))
        .route("/v1/pause", post(routes::pause_set))
        .route("/v1/pause/clear", post(routes::pause_clear))
        .route(
            "/v1/capabilities/{kind}/{id}/ai-description",
            axum::routing::put(routes::ai_description_put),
        )
        .route("/v1/ai-assists", get(routes::ai_assists_list))
        .route("/v1/activity/inbox", get(routes::activity_inbox))
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
        .route("/v1/providers", get(routes::providers_list))
        .route("/v1/hardware/scan", post(routes::hardware_scan))
        .route(
            "/v1/sensors/microphone/listen",
            post(routes::sensor_mic_listen),
        )
        .route("/v1/sensors/stop", post(routes::sensors_stop))
        .route("/v1/presentation", get(routes::presentation_status))
        .route(
            "/v1/presentation/commands/{action_id}",
            get(routes::presentation_pending_command),
        )
        .route("/v1/presentation/hello", post(routes::presentation_hello))
        .route("/v1/presentation/ack", post(routes::presentation_ack))
        // Character Presentation Protocol（human；receipts／events 也收 adapter token）。
        .route("/v1/character/hello", post(routes::character_hello))
        .route("/v1/character/receipts", post(routes::character_receipts))
        .route("/v1/character/events", post(routes::character_events))
        .route("/v1/character/instances", get(routes::character_instances))
        .route("/v1/character/manifest", get(routes::character_manifest))
        .route(
            "/v1/character/adapters",
            get(routes::character_adapters_list),
        )
        .route(
            "/v1/character/adapters",
            post(routes::character_adapter_add),
        )
        .route(
            "/v1/character/adapters/{id}",
            delete(routes::character_adapter_revoke),
        )
        .route("/v1/character/intent", post(routes::character_intent))
        .route(
            "/v1/proactive-dialogue",
            get(routes::proactive_dialogue_get).patch(routes::proactive_dialogue_patch),
        )
        .route(
            "/v1/proactive-dialogue/quiet",
            post(routes::proactive_dialogue_quiet),
        )
        .route("/v1/assets", get(routes::assets_list))
        .route("/v1/assets/import", post(routes::asset_import))
        .route("/v1/assets/{hash}", get(routes::asset_get))
        .route("/v1/assets/{hash}", delete(routes::asset_delete))
        .route("/v1/assets/{hash}/impact", get(routes::asset_impact))
        .route("/v1/assets/{hash}/content", get(routes::asset_content))
        .route("/v1/assets/{hash}/preview", get(routes::asset_preview))
        .route(
            "/v1/assets/{hash}/derivatives",
            get(routes::asset_derivatives),
        )
        .route("/v1/assets/{hash}/derive", post(routes::asset_derive))
        .route("/v1/knowledge/search", get(routes::knowledge_search))
        .route(
            "/v1/knowledge/domain-packs",
            get(routes::knowledge_domain_packs),
        )
        .route(
            "/v1/knowledge/domain-packs/{id}/install",
            post(routes::knowledge_domain_pack_install),
        )
        .route(
            "/v1/knowledge/domain-packs/{id}",
            delete(routes::knowledge_domain_pack_uninstall),
        )
        .route("/v1/knowledge/nodes", get(routes::knowledge_nodes_list))
        .route("/v1/knowledge/nodes", post(routes::knowledge_node_create))
        .route("/v1/knowledge/nodes/{id}", get(routes::knowledge_node_get))
        .route(
            "/v1/knowledge/nodes/{id}/review",
            post(routes::knowledge_node_review),
        )
        .route(
            "/v1/knowledge/nodes/{id}/graph",
            get(routes::knowledge_graph),
        )
        .route("/v1/knowledge/edges", post(routes::knowledge_edge_create))
        .route("/v1/knowledge/receipts", get(routes::knowledge_receipts))
        .route(
            "/v1/knowledge/update-check",
            post(routes::knowledge_update_check),
        )
        .route(
            "/v1/knowledge/user-corrections",
            post(routes::knowledge_user_correction),
        )
        .route("/v1/memory", get(routes::memory_list))
        .route("/v1/memory", post(routes::memory_create))
        .route("/v1/memory/export", get(routes::memory_export))
        .route(
            "/v1/memory/clear-session-context",
            post(routes::memory_clear_session),
        )
        .route(
            "/v1/memory/context-bundle",
            post(routes::memory_context_bundle),
        )
        .route("/v1/memory/{id}", get(routes::memory_get))
        .route("/v1/memory/{id}", patch(routes::memory_patch))
        .route("/v1/memory/{id}", delete(routes::memory_delete))
        .route("/v1/agents", get(routes::agents_discoveries))
        .route("/v1/agents/refresh", post(routes::agents_refresh))
        .route("/v1/agents/routing", get(routes::agents_routing))
        .route(
            "/v1/agent-sessions/{id}/approve",
            post(routes::agent_session_approve),
        )
        .route(
            "/v1/agent-sessions/{id}/interrupt",
            post(routes::agent_session_interrupt),
        )
        .route("/v1/agent-sessions", get(routes::agent_sessions_list))
        .route("/v1/agent-sessions", post(routes::agent_session_create))
        .route("/v1/agent-sessions/{id}", get(routes::agent_session_get))
        .route(
            "/v1/agent-sessions/{id}/report",
            post(routes::agent_session_report),
        )
        .route(
            "/v1/agent-sessions/{id}/messages",
            get(routes::agent_session_messages),
        )
        .route(
            "/v1/agent-sessions/{id}/messages",
            post(routes::agent_session_send),
        )
        .route(
            "/v1/agent-sessions/{id}/renew",
            post(routes::agent_session_renew),
        )
        .route(
            "/v1/agent-sessions/{id}/close",
            post(routes::agent_session_close),
        )
        .route(
            "/v1/agent-sessions/{id}/verify",
            post(routes::agent_session_verify),
        )
        .route("/v1/mobile/status", get(routes::mobile_status))
        .route(
            "/v1/mobile/pairing-session",
            post(routes::mobile_pairing_begin),
        )
        .route("/v1/mobile/devices/{id}", delete(routes::mobile_revoke))
        .route(
            "/v1/mobile/devices/{id}/sensors/stop",
            post(routes::mobile_sensors_stop),
        )
        .route("/v1/mobile/devices/{id}/test", post(routes::mobile_test))
        .route("/v1/mobile/ble/scan", post(routes::mobile_ble_scan))
        .route("/v1/providers/{id}", get(routes::provider_get))
        .route("/v1/providers/{id}/pair", post(routes::provider_pair))
        .route(
            "/v1/providers/{id}/transition",
            post(routes::provider_transition),
        )
        .route("/v1/providers/{id}/test", post(routes::provider_test))
        .route("/v1/providers/{id}/revoke", post(routes::provider_revoke))
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
        // 外部 adapter WebSocket：token 走 query（不是 Bearer），自己驗證——
        // 只收 adapter token，human／agent token 一律 401。
        .route("/v1/character/ws", get(character_ws::character_ws))
        .merge(authed)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(cors_layer())
        .with_state(state)
}

fn cors_layer() -> tower_http::cors::CorsLayer {
    // Loopback-only UI origins (any port: Tauri, vite dev, browser E2E).
    // CORS is defense-in-depth here — the bearer token is the actual gate,
    // and the API never binds beyond loopback by default.
    tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(|origin, _| {
            let Ok(origin) = origin.to_str() else {
                return false;
            };
            if origin == "tauri://localhost" || origin == "https://tauri.localhost" {
                return true;
            }
            let Some(rest) = origin
                .strip_prefix("http://")
                .or_else(|| origin.strip_prefix("https://"))
            else {
                return false;
            };
            let host = rest.split(':').next().unwrap_or(rest);
            matches!(host, "localhost" | "127.0.0.1" | "[::1]")
        }))
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}

async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<ApiState>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let candidate = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let human = candidate
        .map(|v| constant_time_eq(v.as_bytes(), state.token.as_bytes()))
        .unwrap_or(false);
    let agent = candidate
        .map(|v| constant_time_eq(v.as_bytes(), state.agent_token.as_bytes()))
        .unwrap_or(false);
    let session = if !human && !agent {
        match candidate {
            Some(value) => state.runtime.agent_session_capability(value).await,
            None => None,
        }
    } else {
        None
    };
    // Character adapter token：獨立分權，永遠不進 agent／human 的路由判斷。
    let adapter = if !human && !agent && session.is_none() {
        candidate.and_then(|value| state.runtime.character_adapter_for_token(value))
    } else {
        None
    };
    if !human && !agent && session.is_none() && adapter.is_none() {
        return ApiError::unauthorized().into_response();
    }
    if agent && !agent_request_allowed(request.method(), request.uri().path()) {
        return ApiError::forbidden_scope().into_response();
    }
    if let Some(capability) = &session {
        if !session_request_allowed(request.method(), request.uri().path(), capability) {
            return ApiError::forbidden_scope().into_response();
        }
    }
    if adapter.is_some() && !adapter_request_allowed(request.method(), request.uri().path()) {
        return ApiError::forbidden_adapter_scope().into_response();
    }
    let principal = if human {
        AuthPrincipal::Human
    } else if agent {
        AuthPrincipal::LegacyAgent
    } else if let Some(capability) = session {
        AuthPrincipal::AgentSession(capability)
    } else if let Some(adapter_id) = adapter {
        AuthPrincipal::CharacterAdapter { adapter_id }
    } else {
        return ApiError::unauthorized().into_response();
    };
    request.extensions_mut().insert(AuthContext { principal });
    next.run(request).await
}

/// README §8.2：adapter token 只能用於自己的回執／事件（與 `/v1/character/ws`，
/// 那條路由自己驗 query token）。其餘一律 403——包括 `/v1/status`、estop、
/// agent sessions、actuator、policy／consent／verify。
fn adapter_request_allowed(method: &axum::http::Method, path: &str) -> bool {
    method == axum::http::Method::POST
        && matches!(path, "/v1/character/receipts" | "/v1/character/events")
}

fn session_request_allowed(
    method: &axum::http::Method,
    path: &str,
    capability: &interaction_runtime::agents::AgentSessionCapability,
) -> bool {
    use axum::http::Method;
    if method == Method::GET && (path == "/v1/tools" || path.starts_with("/v1/tools/")) {
        return true;
    }
    if method == Method::POST && path.starts_with("/v1/tools/") && path.ends_with("/call") {
        return true;
    }
    // 安全遞減操作（停止）：與 estop 同級，AI 想主動停感測不必整個 estop。
    if method == Method::POST
        && matches!(
            path,
            "/v1/emergency-stop" | "/v1/stop-all" | "/v1/sensors/stop"
        )
    {
        return true;
    }
    method == Method::POST
        && path
            .strip_prefix("/v1/agent-sessions/")
            .and_then(|value| value.strip_suffix("/interrupt"))
            .is_some_and(|id| id == capability.session_id)
}

/// safety-invariants-078：誰可以中斷 `{id}` 這個 session。middleware 已經用路徑
/// 形狀擋掉 legacy token 與跨 session 的 capability token；handler 再比對一次，
/// 讓「擁有權」這條規則不依賴路由順序或 middleware 的正確掛載。
///
/// * human：控制中心保留管理能力，可中斷任何 session；
/// * session-scoped capability：只能中斷自己的 session；
/// * legacy agent／character adapter：沒有 session 身分，一律拒絕。
pub(crate) fn interrupt_principal_allowed(principal: &AuthPrincipal, session_id: &str) -> bool {
    match principal {
        AuthPrincipal::Human => true,
        AuthPrincipal::AgentSession(capability) => capability.session_id == session_id,
        AuthPrincipal::LegacyAgent | AuthPrincipal::CharacterAdapter { .. } => false,
    }
}

/// Restricted AI/tool-plane boundary. Human-only route families stay denied
/// until explicitly reviewed. Safety-decreasing operations (stop, revoke,
/// cancel) and canonical tool calls remain available.
fn agent_request_allowed(method: &axum::http::Method, path: &str) -> bool {
    use axum::http::Method;
    if method == Method::GET {
        return !path.starts_with("/v1/memory")
            && !path.starts_with("/v1/assets")
            && !path.starts_with("/v1/knowledge")
            && !path.starts_with("/v1/agent-sessions")
            && !path.starts_with("/v1/activity")
            && path != "/v1/audit"
            && path != "/v1/outbox"
            && !path.starts_with("/v1/ui/preferences")
            && !path.starts_with("/v1/onboarding")
            // 配對指紋/裝置清單屬人類層：agent token 不可讀。
            && !path.starts_with("/v1/mobile")
            // 角色 instance／adapter 登記屬可信 host 層：agent token 不可讀。
            && !path.starts_with("/v1/character");
    }
    if matches!(
        path,
        "/v1/emergency-stop"
            | "/v1/stop-all"
            | "/v1/session/revoke"
            | "/v1/session/stop"
            // 停止所有感測也是安全遞減操作（audit 記的是實際 principal）。
            | "/v1/sensors/stop"
    ) {
        return true;
    }
    if path.starts_with("/v1/actions/") && path.ends_with("/cancel") {
        return true;
    }
    // safety-invariants-078：`/v1/agent-sessions/{id}/interrupt` 指名單一 session，
    // 不像 estop／stop-all／sensors/stop 那樣是全域安全遞減操作——它有「誰的
    // session」這個語意，必須能證明擁有權。`AuthPrincipal::LegacyAgent` 是零欄位
    // variant，架構上不帶任何 session 身分（也建不了、列不到 session），所以在
    // 這個「純路徑比對」的位置根本沒有資料可比。因此不再放行：中斷 session 必須
    // 用 session-scoped capability token（`INTERACT_AI_SESSION_TOKEN`，見
    // `session_request_allowed`，只准中斷自己）或 human token。
    if path.starts_with("/v1/tools/") && path.ends_with("/call") {
        return true;
    }
    path == "/v1/observations/query"
        || path == "/v1/plans"
        || (path.starts_with("/v1/plans/")
            && (path.ends_with("/simulate") || path.ends_with("/execute")))
        || path == "/v1/hardware/scan"
        || path == "/v1/knowledge/update-check"
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
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
    let agent_token = runtime
        .config_service
        .load_or_create_agent_token()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let state = ApiState {
        runtime: runtime.clone(),
        token,
        agent_token,
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

#[cfg(test)]
mod auth_scope_tests {
    use super::*;
    use axum::http::Method;

    #[test]
    fn adapter_tokens_only_reach_receipts_and_events() {
        assert!(adapter_request_allowed(
            &Method::POST,
            "/v1/character/receipts"
        ));
        assert!(adapter_request_allowed(
            &Method::POST,
            "/v1/character/events"
        ));
        for (method, path) in [
            (Method::GET, "/v1/status"),
            (Method::POST, "/v1/emergency-stop"),
            (Method::POST, "/v1/emergency-stop/clear"),
            (Method::POST, "/v1/agent-sessions/x/verify"),
            (Method::POST, "/v1/agent-sessions/x/interrupt"),
            (Method::GET, "/v1/character/instances"),
            (Method::POST, "/v1/character/hello"),
            (Method::POST, "/v1/character/adapters"),
            (Method::POST, "/v1/plans"),
            (Method::POST, "/v1/tools/interaction.status/call"),
            (Method::PATCH, "/v1/policy"),
            (Method::GET, "/v1/character/receipts"),
        ] {
            assert!(
                !adapter_request_allowed(&method, path),
                "{method} {path} must be refused for adapter tokens"
            );
        }
        // agent token 也拿不到角色層的人類資料。
        assert!(!agent_request_allowed(
            &Method::GET,
            "/v1/character/instances"
        ));
        assert!(!agent_request_allowed(
            &Method::GET,
            "/v1/character/adapters"
        ));
        assert!(!agent_request_allowed(&Method::POST, "/v1/character/hello"));
        assert!(!agent_request_allowed(
            &Method::POST,
            "/v1/character/intent"
        ));
    }

    /// safety-invariants-078：`POST /v1/agent-sessions/{id}/interrupt` 指名單一
    /// session，語意上必須有擁有權。`AuthPrincipal::LegacyAgent` 是零欄位
    /// variant，架構上不帶任何 session 身分（GET 分支連 `/v1/agent-sessions`
    /// 清單都讀不到，POST `/v1/agent-sessions` 也建不了 session），因此無從證明
    /// 擁有權——中斷別人的 session 必須改用 session-scoped capability token
    /// （`INTERACT_AI_SESSION_TOKEN`）或 human token。
    #[test]
    fn legacy_agent_token_cannot_interrupt_any_agent_session() {
        for path in [
            "/v1/agent-sessions/any-id/interrupt",
            "/v1/agent-sessions/sess-01J0/interrupt",
        ] {
            assert!(
                !agent_request_allowed(&Method::POST, path),
                "legacy agent token must not reach POST {path}"
            );
        }
        // 前提：legacy token 本來就建不了 session，也列不到 session，
        // 所以「自己的 session」對它並不存在。
        assert!(!agent_request_allowed(&Method::POST, "/v1/agent-sessions"));
        assert!(!agent_request_allowed(&Method::GET, "/v1/agent-sessions"));
        assert!(!agent_request_allowed(
            &Method::GET,
            "/v1/agent-sessions/any-id"
        ));
        // 其餘 agent-session 家族的寫入操作維持原本的 403。
        for path in [
            "/v1/agent-sessions/any-id/approve",
            "/v1/agent-sessions/any-id/renew",
            "/v1/agent-sessions/any-id/close",
            "/v1/agent-sessions/any-id/report",
            "/v1/agent-sessions/any-id/messages",
            "/v1/agent-sessions/any-id/verify",
        ] {
            assert!(!agent_request_allowed(&Method::POST, path));
        }
        // 真正的全域安全遞減操作不受影響。
        assert!(agent_request_allowed(&Method::POST, "/v1/emergency-stop"));
        assert!(agent_request_allowed(&Method::POST, "/v1/stop-all"));
        assert!(agent_request_allowed(&Method::POST, "/v1/sensors/stop"));
        assert!(agent_request_allowed(
            &Method::POST,
            "/v1/actions/a1/cancel"
        ));
    }

    /// session-scoped capability token 只能中斷自己的 session。
    #[test]
    fn session_scoped_token_interrupts_only_its_own_session() {
        let capability = interaction_runtime::agents::AgentSessionCapability {
            session_id: "sess-own".into(),
            agent_id: "agent-under-test".into(),
            tool_scope: Default::default(),
            domains: Default::default(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        };
        assert!(session_request_allowed(
            &Method::POST,
            "/v1/agent-sessions/sess-own/interrupt",
            &capability
        ));
        assert!(!session_request_allowed(
            &Method::POST,
            "/v1/agent-sessions/sess-other/interrupt",
            &capability
        ));
        assert!(!session_request_allowed(
            &Method::POST,
            "/v1/agent-sessions",
            &capability
        ));
    }

    /// Handler 層的第二道擁有權比對（defense-in-depth）：即使 middleware 的路徑
    /// 形狀比對被繞過／改動，這個判定仍必須把非擁有者擋下。
    #[test]
    fn interrupt_principal_check_is_ownership_scoped() {
        let capability = |session_id: &str| interaction_runtime::agents::AgentSessionCapability {
            session_id: session_id.into(),
            agent_id: "agent-under-test".into(),
            tool_scope: Default::default(),
            domains: Default::default(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        };
        // human 保留管理能力。
        assert!(interrupt_principal_allowed(&AuthPrincipal::Human, "sess-a"));
        // session-scoped：只有自己的 session。
        assert!(interrupt_principal_allowed(
            &AuthPrincipal::AgentSession(capability("sess-a")),
            "sess-a"
        ));
        assert!(!interrupt_principal_allowed(
            &AuthPrincipal::AgentSession(capability("sess-a")),
            "sess-b"
        ));
        // legacy／adapter 沒有 session 身分：一律拒絕。
        assert!(!interrupt_principal_allowed(
            &AuthPrincipal::LegacyAgent,
            "sess-a"
        ));
        assert!(!interrupt_principal_allowed(
            &AuthPrincipal::CharacterAdapter {
                adapter_id: "adapter-1".into()
            },
            "sess-a"
        ));
    }
}
