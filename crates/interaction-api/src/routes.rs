//! Route handlers. Thin: every handler delegates to the shared runtime
//! services (the same ones the CLI daemon and Tauri commands use).

use crate::dto::*;
use crate::error::{ApiError, ApiResult};
use crate::{ApiState, AuthContext, AuthPrincipal};
use axum::extract::{Extension, Path, Query, State};
use axum::Json;
use interaction_core::{
    ActionId, ActuatorId, DiscoveryContext, DomainError, ObservationQuery, PlanId, ReceptorId,
    SemanticIntent,
};
use interaction_policy::ActionSource;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Health / status
// ---------------------------------------------------------------------------

pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

pub async fn ready(State(state): State<ApiState>) -> Json<Value> {
    Json(json!({"status": "ok", "emergencyStop": state.runtime.is_estopped()}))
}

pub async fn status(State(state): State<ApiState>) -> Json<Value> {
    Json(state.runtime.status().await)
}

pub async fn activity_inbox(
    State(state): State<ApiState>,
    Query(filter): Query<interaction_runtime::activity::ActivityInboxFilter>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.activity_inbox(filter).await?))
}

/// Metadata-only capability scan. This does not open any sensor or device
/// stream; unsupported categories remain explicit in the returned report.
pub async fn hardware_scan(State(state): State<ApiState>) -> Json<Value> {
    Json(serde_json::to_value(state.runtime.scan_hardware_capabilities().await).unwrap_or_default())
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesQuery {
    #[serde(default)]
    include_unavailable: bool,
}

pub async fn capabilities(
    State(state): State<ApiState>,
    Query(q): Query<CapabilitiesQuery>,
) -> Json<Value> {
    let snapshot = state
        .runtime
        .capabilities(&DiscoveryContext {
            include_unavailable: q.include_unavailable,
            ..Default::default()
        })
        .await;
    Json(serde_json::to_value(snapshot).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Receptors
// ---------------------------------------------------------------------------

pub async fn receptors_list(State(state): State<ApiState>) -> Json<Value> {
    Json(json!(state.runtime.registry.receptor_manifests().await))
}

pub async fn receptor_inspect(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let manifests = state.runtime.registry.receptor_manifests().await;
    let manifest = manifests
        .into_iter()
        .find(|m| m.id.as_str() == id)
        .ok_or_else(|| ApiError::from(DomainError::NotFound(format!("receptor {id}"))))?;
    let recent = state
        .runtime
        .observe_stored(&ObservationQuery {
            receptor_id: Some(ReceptorId::new(&id)),
            limit: Some(5),
            ..Default::default()
        })
        .await
        .unwrap_or_default();
    Ok(Json(
        json!({"manifest": manifest, "recentObservations": recent}),
    ))
}

pub async fn receptor_patch(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<EnabledPatch>,
) -> ApiResult<Json<Value>> {
    state
        .runtime
        .registry
        .set_receptor_enabled(&ReceptorId::new(&id), body.enabled)
        .await?;
    Ok(Json(json!({"receptorId": id, "enabled": body.enabled})))
}

pub async fn receptor_delete(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    state
        .runtime
        .registry
        .unregister_receptor(&ReceptorId::new(&id))
        .await?;
    Ok(Json(json!({"removed": id})))
}

pub async fn receptor_test(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let obs = state.runtime.observe_fresh(&ReceptorId::new(&id)).await?;
    Ok(Json(json!({"ok": true, "observation": obs})))
}

pub async fn receptor_read(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let obs = state.runtime.observe_fresh(&ReceptorId::new(&id)).await?;
    Ok(Json(serde_json::to_value(obs).unwrap_or_default()))
}

pub async fn receptor_push(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<PushInput>,
) -> ApiResult<Json<Value>> {
    let obs = state
        .runtime
        .ingest(&id, body.facts, body.inferences, body.confidence)
        .await?;
    Ok(Json(serde_json::to_value(obs).unwrap_or_default()))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceptorCreate {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default = "default_push_driver")]
    pub driver: String,
}

fn default_category() -> String {
    "custom".into()
}

fn default_push_driver() -> String {
    "builtin.push".into()
}

pub async fn receptor_create(
    State(state): State<ApiState>,
    Json(body): Json<ReceptorCreate>,
) -> ApiResult<Json<Value>> {
    if body.driver != "builtin.push" {
        return Err(ApiError::from(DomainError::Validation(format!(
            "unknown driver {:?}; only 'builtin.push' receptors can be added at runtime — \
             other drivers register through the Adapter SDK at daemon startup",
            body.driver
        ))));
    }
    let name = body.name.unwrap_or_else(|| body.id.clone());
    state
        .runtime
        .add_push_receptor(&body.id, &name, &body.category, body.sensitive)
        .await?;
    Ok(Json(json!({"created": body.id})))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActuatorCreate {
    pub id: String,
    #[serde(default = "default_mock_channel")]
    pub channel: String,
    #[serde(default = "default_mock_driver")]
    pub driver: String,
}

fn default_mock_channel() -> String {
    "haptic".into()
}

fn default_mock_driver() -> String {
    "builtin.mock-actuator".into()
}

pub async fn actuator_create(
    State(state): State<ApiState>,
    Json(body): Json<ActuatorCreate>,
) -> ApiResult<Json<Value>> {
    if body.driver != "builtin.mock-actuator" {
        return Err(ApiError::from(DomainError::Validation(format!(
            "unknown driver {:?}; only 'builtin.mock-actuator' devices can be added at runtime — \
             other drivers register through the Adapter SDK at daemon startup",
            body.driver
        ))));
    }
    state
        .runtime
        .add_mock_actuator(&body.id, &body.channel)
        .await?;
    Ok(Json(json!({"created": body.id})))
}

// ---------------------------------------------------------------------------
// Actuators
// ---------------------------------------------------------------------------

pub async fn actuators_list(State(state): State<ApiState>) -> Json<Value> {
    Json(json!(state.runtime.registry.actuator_manifests().await))
}

pub async fn actuator_inspect(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let manifests = state.runtime.registry.actuator_manifests().await;
    let manifest = manifests
        .into_iter()
        .find(|m| m.id.as_str() == id)
        .ok_or_else(|| ApiError::from(DomainError::NotFound(format!("actuator {id}"))))?;
    Ok(Json(json!({"manifest": manifest})))
}

pub async fn actuator_patch(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<EnabledPatch>,
) -> ApiResult<Json<Value>> {
    state
        .runtime
        .registry
        .set_actuator_enabled(&ActuatorId::new(&id), body.enabled)
        .await?;
    Ok(Json(json!({"actuatorId": id, "enabled": body.enabled})))
}

pub async fn actuator_delete(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    state.runtime.remove_actuator(&id).await?;
    Ok(Json(json!({"removed": id})))
}

/// Test an actuator through the FULL policied path (never bypasses the governor).
pub async fn actuator_test(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let mut intent = SemanticIntent::new("presence");
    intent.message = Some(format!("[test] actuator {id}"));
    intent.magnitude = Some(0.2);
    intent.duration_ms = Some(500);
    let plan = state
        .runtime
        .create_plan(
            intent,
            vec![id.clone()],
            1,
            1,
            false,
            None,
            Default::default(),
        )
        .await?;
    let receipts = state
        .runtime
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await?;
    Ok(Json(
        json!({"planId": plan.plan_id.as_str(), "receipts": receipts}),
    ))
}

// ---------------------------------------------------------------------------
// Observations / plans / actions
// ---------------------------------------------------------------------------

pub async fn observations_query(
    State(state): State<ApiState>,
    Json(query): Json<ObservationQuery>,
) -> ApiResult<Json<Value>> {
    let observations = state.runtime.observe_stored(&query).await?;
    Ok(Json(json!(observations)))
}

pub async fn plan_create(
    State(state): State<ApiState>,
    Json(input): Json<PlanInput>,
) -> ApiResult<Json<Value>> {
    let plan = create_plan_from_input(&state, input).await?;
    Ok(Json(serde_json::to_value(plan).unwrap_or_default()))
}

pub(crate) async fn create_plan_from_input(
    state: &ApiState,
    input: PlanInput,
) -> Result<interaction_core::Plan, DomainError> {
    let mut intent = SemanticIntent::new(input.intent);
    intent.character = input.character;
    intent.message = input.message;
    intent.magnitude = input.magnitude;
    intent.duration_ms = input.duration_ms;
    intent.preferred_channels = input.preferred_channels;
    intent.payload = input.payload;
    state
        .runtime
        .create_plan(
            intent,
            input.candidates,
            input.min_channels,
            input.max_channels,
            input.allow_no_action,
            input.message_strategy,
            input.metadata,
        )
        .await
}

pub async fn plan_get(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let plan = state.runtime.get_plan(&PlanId::new(&id))?;
    Ok(Json(serde_json::to_value(plan).unwrap_or_default()))
}

pub async fn plan_simulate(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let report = state.runtime.simulate_plan(&PlanId::new(&id)).await?;
    Ok(Json(serde_json::to_value(report).unwrap_or_default()))
}

pub async fn plan_execute(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: Option<Json<ExecuteInput>>,
) -> ApiResult<Json<Value>> {
    let dry_run = body.map(|Json(b)| b.dry_run).unwrap_or(false);
    let plan_id = PlanId::new(&id);
    if dry_run {
        let report = state.runtime.simulate_plan(&plan_id).await?;
        return Ok(Json(json!({"dryRun": true, "simulation": report})));
    }
    let receipts = state
        .runtime
        .execute_plan(&plan_id, ActionSource::ExplicitRequest, false)
        .await?;
    Ok(Json(json!(receipts)))
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActionsQuery {
    #[serde(default)]
    limit: Option<u32>,
}

pub async fn actions_list(
    State(state): State<ApiState>,
    Query(q): Query<ActionsQuery>,
) -> ApiResult<Json<Value>> {
    let receipts = state
        .runtime
        .list_actions(None, q.limit.unwrap_or(50).min(500))?;
    Ok(Json(json!(receipts)))
}

pub async fn action_get(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let receipt = state.runtime.get_action(&ActionId::new(&id))?;
    Ok(Json(serde_json::to_value(receipt).unwrap_or_default()))
}

pub async fn action_cancel(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    // safety-invariants-057：取消也是 agent／session token 打得到的安全遞減
    // 操作，audit 記的是實際呼叫者，不是一律 "api"。
    let receipt = state
        .runtime
        .cancel_action_as(&ActionId::new(&id), &stop_actor(&auth))
        .await?;
    Ok(Json(serde_json::to_value(receipt).unwrap_or_default()))
}

pub async fn action_verify(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let receipt = state.runtime.verify_action(&ActionId::new(&id)).await?;
    Ok(Json(serde_json::to_value(receipt).unwrap_or_default()))
}

// ---------------------------------------------------------------------------
// Policy / session
// ---------------------------------------------------------------------------

pub async fn policy_get(State(state): State<ApiState>) -> Json<Value> {
    Json(serde_json::to_value(state.runtime.policy().await).unwrap_or_default())
}

pub async fn policy_patch(
    State(state): State<ApiState>,
    Json(patch): Json<Value>,
) -> ApiResult<Json<Value>> {
    let updated = state.runtime.update_policy(patch).await?;
    Ok(Json(serde_json::to_value(updated).unwrap_or_default()))
}

pub async fn session_start(
    State(state): State<ApiState>,
    body: Option<Json<SessionStartInput>>,
) -> ApiResult<Json<Value>> {
    let input = body.map(|Json(b)| b).unwrap_or_default();
    let session = state
        .runtime
        .start_session(input.label, input.ttl_minutes, input.consents)
        .await?;
    Ok(Json(serde_json::to_value(session).unwrap_or_default()))
}

pub async fn session_get(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    match state.runtime.current_session().await {
        Some(session) => Ok(Json(serde_json::to_value(session).unwrap_or_default())),
        None => Ok(Json(json!(null))),
    }
}

pub async fn session_consent(
    State(state): State<ApiState>,
    Json(body): Json<ConsentInput>,
) -> ApiResult<Json<Value>> {
    let session = state
        .runtime
        .grant_consent_with_uses(&body.scope, body.expires_minutes, body.max_uses)
        .await?;
    Ok(Json(serde_json::to_value(session).unwrap_or_default()))
}

pub async fn session_revoke(
    State(state): State<ApiState>,
    Json(body): Json<ConsentInput>,
) -> ApiResult<Json<Value>> {
    let session = state.runtime.revoke_consent(&body.scope).await?;
    Ok(Json(serde_json::to_value(session).unwrap_or_default()))
}

pub async fn session_stop(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    state.runtime.stop_session().await?;
    Ok(Json(json!({"stopped": true})))
}

// ---------------------------------------------------------------------------
// Recipes
// ---------------------------------------------------------------------------

pub async fn recipes_list(State(state): State<ApiState>) -> Json<Value> {
    let recipes = state.runtime.list_recipes().await;
    let items: Vec<Value> = recipes
        .into_iter()
        .map(|(recipe, state)| {
            json!({
                "recipe": recipe,
                "state": {
                    "lastFiredAt": state.last_fired_at,
                    "executionsThisSession": state.executions_this_session,
                }
            })
        })
        .collect();
    Json(json!(items))
}

pub async fn recipe_get(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let recipe = state.runtime.get_recipe(&id).await?;
    Ok(Json(serde_json::to_value(recipe).unwrap_or_default()))
}

pub async fn recipe_create(
    State(state): State<ApiState>,
    Json(body): Json<RecipeBody>,
) -> ApiResult<Json<Value>> {
    let text = body
        .as_text()
        .ok_or_else(|| ApiError::from(DomainError::Validation("missing recipe text".into())))?;
    let recipe = state.runtime.upsert_recipe_text(&text).await?;
    Ok(Json(serde_json::to_value(recipe).unwrap_or_default()))
}

pub async fn recipe_validate(Json(body): Json<RecipeBody>) -> ApiResult<Json<Value>> {
    let text = body
        .as_text()
        .ok_or_else(|| ApiError::from(DomainError::Validation("missing recipe text".into())))?;
    match interaction_recipe::parse_and_validate(&text) {
        Ok(recipe) => Ok(Json(json!({"valid": true, "recipe": recipe}))),
        Err(interaction_recipe::RecipeParseError::Invalid(issues)) => {
            Ok(Json(json!({"valid": false, "issues": issues})))
        }
        Err(e) => Ok(Json(
            json!({"valid": false, "issues": [{"field": "$", "message": e.to_string()}]}),
        )),
    }
}

pub async fn recipe_patch(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<EnabledPatch>,
) -> ApiResult<Json<Value>> {
    let recipe = state.runtime.set_recipe_enabled(&id, body.enabled).await?;
    Ok(Json(serde_json::to_value(recipe).unwrap_or_default()))
}

pub async fn recipe_delete(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    state.runtime.remove_recipe(&id).await?;
    Ok(Json(json!({"removed": id})))
}

pub async fn recipe_simulate(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.simulate_recipe(&id).await?))
}

pub async fn recipe_run(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.run_recipe(&id).await?))
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

pub async fn tools_list(State(state): State<ApiState>) -> Json<Value> {
    Json(json!(state.runtime.registry.tool_operations().await))
}

/// Resolve a tool by canonical name (`interaction.execute`) or its
/// platform-normalized alias (`interaction_execute`).
async fn resolve_tool(
    state: &ApiState,
    name: &str,
) -> Result<interaction_core::ToolOperationManifest, DomainError> {
    if let Ok(m) = state.runtime.registry.tool_operation(name).await {
        return Ok(m);
    }
    let all = state.runtime.registry.tool_operations().await;
    all.into_iter()
        .find(|m| interaction_tool_schema::platform_name(&m.name) == name)
        .ok_or_else(|| DomainError::NotFound(format!("tool {name}")))
}

pub async fn tool_get(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    let manifest = resolve_tool(&state, &name).await?;
    Ok(Json(serde_json::to_value(manifest).unwrap_or_default()))
}

pub async fn tools_export(
    State(state): State<ApiState>,
    Path(format): Path<String>,
) -> ApiResult<Json<Value>> {
    let fmt = interaction_tool_schema::ExportFormat::parse(&format).ok_or_else(|| {
        ApiError::from(DomainError::Validation(format!(
            "unknown format {format:?}; expected openai|anthropic|gemini|openapi|json-schema"
        )))
    })?;
    let manifests = state.runtime.registry.tool_operations().await;
    let warnings = interaction_tool_schema::validate_manifests(&manifests);
    Ok(Json(json!({
        "format": format,
        "export": interaction_tool_schema::export(&manifests, fmt),
        "warnings": warnings,
    })))
}

pub async fn tool_call(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
    Path(name): Path<String>,
    body: Option<Json<Value>>,
) -> ApiResult<Json<Value>> {
    let input = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    // The manifest must exist (tool discovery = source of truth).
    let manifest = resolve_tool(&state, &name).await?;
    let is_knowledge = manifest.name.starts_with("interaction.knowledge_");
    match &auth.principal {
        AuthPrincipal::LegacyAgent if is_knowledge => return Err(ApiError::forbidden_scope()),
        AuthPrincipal::AgentSession(capability) if !capability.allows_tool(&manifest.name) => {
            return Err(ApiError::forbidden_scope());
        }
        _ => {}
    }
    let result = dispatch_tool(&state, &auth, &manifest.name, input).await?;
    Ok(Json(result))
}

async fn dispatch_tool(
    state: &ApiState,
    auth: &AuthContext,
    name: &str,
    input: Value,
) -> Result<Value, ApiError> {
    let rt = &state.runtime;
    let str_field =
        |input: &Value, key: &str| input.get(key).and_then(|v| v.as_str()).map(String::from);
    let required = |input: &Value, key: &str| {
        str_field(input, key).ok_or_else(|| {
            ApiError::from(DomainError::Validation(format!(
                "missing required field {key}"
            )))
        })
    };
    match name {
        "interaction.status" => Ok(rt.status().await),
        "interaction.capabilities" => {
            let include = input
                .get("includeUnavailable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let snap = rt
                .capabilities(&DiscoveryContext {
                    include_unavailable: include,
                    ..Default::default()
                })
                .await;
            Ok(serde_json::to_value(snap).unwrap_or_default())
        }
        "interaction.observe" => {
            let fresh = input
                .get("fresh")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if fresh {
                let receptor = required(&input, "receptorId")?;
                let obs = rt.observe_fresh(&ReceptorId::new(&receptor)).await?;
                Ok(json!([obs]))
            } else {
                let query = ObservationQuery {
                    receptor_id: str_field(&input, "receptorId").map(ReceptorId::new),
                    max_age_ms: input.get("maxAgeMs").and_then(|v| v.as_u64()),
                    limit: input
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                    ..Default::default()
                };
                Ok(json!(rt.observe_stored(&query).await?))
            }
        }
        "interaction.plan" => {
            let plan_input: PlanInput = serde_json::from_value(input)
                .map_err(|e| ApiError::from(DomainError::Validation(e.to_string())))?;
            let plan = create_plan_from_input(state, plan_input).await?;
            Ok(serde_json::to_value(plan).unwrap_or_default())
        }
        "interaction.simulate" => {
            let plan_id = required(&input, "planId")?;
            let report = rt.simulate_plan(&PlanId::new(&plan_id)).await?;
            Ok(serde_json::to_value(report).unwrap_or_default())
        }
        "interaction.execute" => {
            let plan_id = required(&input, "planId")?;
            let dry = input
                .get("dryRun")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if dry {
                let report = rt.simulate_plan(&PlanId::new(&plan_id)).await?;
                Ok(json!({"dryRun": true, "simulation": report}))
            } else {
                let receipts = rt
                    .execute_plan(&PlanId::new(&plan_id), ActionSource::ExplicitRequest, false)
                    .await?;
                Ok(json!(receipts))
            }
        }
        "interaction.action_status" => {
            let id = required(&input, "actionId")?;
            Ok(serde_json::to_value(rt.get_action(&ActionId::new(&id))?).unwrap_or_default())
        }
        "interaction.verify" => {
            let id = required(&input, "actionId")?;
            Ok(
                serde_json::to_value(rt.verify_action(&ActionId::new(&id)).await?)
                    .unwrap_or_default(),
            )
        }
        "interaction.cancel" => {
            let id = required(&input, "actionId")?;
            Ok(serde_json::to_value(
                rt.cancel_action_as(&ActionId::new(&id), &stop_actor(auth))
                    .await?,
            )
            .unwrap_or_default())
        }
        "interaction.stop" => {
            let reason = str_field(&input, "reason");
            Ok(rt.emergency_stop("tool-call", reason).await?)
        }
        "interaction.recipe_run" => {
            let id = required(&input, "recipeId")?;
            match &auth.principal {
                AuthPrincipal::Human => Ok(rt.run_recipe(&id).await?),
                AuthPrincipal::LegacyAgent | AuthPrincipal::AgentSession(_) => {
                    Ok(rt.run_recipe_for_agent(&id).await?)
                }
                // 中介層已擋下；這裡只是型別上的最後防線。
                AuthPrincipal::CharacterAdapter { .. } => Err(ApiError::forbidden_adapter_scope()),
            }
        }
        "interaction.policy" => Ok(serde_json::to_value(rt.policy().await).unwrap_or_default()),
        // ---- 知識工具（spec §12）：tool 呼叫端一律視為 AI（agent actor）——
        // 寫入強制 Candidate、審核裁決降為留言。agent token 只能走
        // 這條受限工具路徑，不能改用 human-only 發布端點。
        "interaction.knowledge_search" => {
            let query = required(&input, "query")?;
            let k = input.get("k").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
            match &auth.principal {
                AuthPrincipal::AgentSession(capability) => Ok(rt
                    .knowledge_search_scoped(&query, k, &capability.domains)
                    .await?),
                _ => Ok(rt.knowledge_search(&query, k).await?),
            }
        }
        "interaction.knowledge_get" => {
            let id = required(&input, "nodeId")?;
            let node = rt.knowledge_get(&id).await?;
            enforce_node_domain(auth, &node)?;
            Ok(serde_json::to_value(node).unwrap_or_default())
        }
        "interaction.knowledge_get_source" => {
            let hash = required(&input, "hash")?;
            if let AuthPrincipal::AgentSession(capability) = &auth.principal {
                if !rt
                    .asset_accessible_in_domains(&hash, &capability.domains)
                    .await?
                {
                    return Err(ApiError::from(DomainError::PolicyBlocked(
                        "source 不在此 Agent Session 的 Domain scope".into(),
                    )));
                }
            }
            let record = rt.asset_get(&hash).await?;
            let preview = if matches!(
                record.media_type,
                interaction_core::MediaType::Text | interaction_core::MediaType::Code
            ) {
                rt.asset_content(&hash, 64 * 1024)
                    .await
                    .ok()
                    .map(|b| String::from_utf8_lossy(&b).to_string())
            } else {
                None
            };
            Ok(json!({
                "asset": record,
                "textPreview": preview,
                "note": "原始素材 write-once；二進位內容請走 /v1/assets/{hash}/content",
            }))
        }
        "interaction.knowledge_expand_graph" => {
            let root = required(&input, "root")?;
            match &auth.principal {
                AuthPrincipal::AgentSession(capability) => Ok(rt
                    .knowledge_graph_scoped(&root, &capability.domains)
                    .await?),
                _ => Ok(rt.knowledge_graph(&root, 1).await?),
            }
        }
        "interaction.knowledge_propose_entity" => {
            let mut node = interaction_runtime::knowledge::node_from_input(
                &json!({"nodeType": "entity", "title": input.get("title"), "content": input.get("content"), "domains": input.get("domains")}),
            )
            .map_err(DomainError::Validation)?;
            node.confidence = input
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5);
            enforce_proposal_domains(auth, &node.domains)?;
            let created = match &auth.principal {
                AuthPrincipal::AgentSession(capability) => {
                    rt.knowledge_propose_node_for_session(
                        node,
                        knowledge_actor(auth),
                        capability.session_id.clone(),
                    )
                    .await?
                }
                _ => {
                    rt.knowledge_propose_node(node, knowledge_actor(auth))
                        .await?
                }
            };
            Ok(serde_json::to_value(created).unwrap_or_default())
        }
        "interaction.knowledge_propose_claim" => {
            let mut merged = input.clone();
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("nodeType".into(), json!("claim"));
            }
            let node = interaction_runtime::knowledge::node_from_input(&merged)
                .map_err(DomainError::Validation)?;
            enforce_proposal_domains(auth, &node.domains)?;
            let created = match &auth.principal {
                AuthPrincipal::AgentSession(capability) => {
                    rt.knowledge_propose_node_for_session(
                        node,
                        knowledge_actor(auth),
                        capability.session_id.clone(),
                    )
                    .await?
                }
                _ => {
                    rt.knowledge_propose_node(node, knowledge_actor(auth))
                        .await?
                }
            };
            Ok(serde_json::to_value(created).unwrap_or_default())
        }
        "interaction.knowledge_propose_relation" => {
            let edge = interaction_runtime::knowledge::edge_from_input(&input)
                .map_err(DomainError::Validation)?;
            if matches!(&auth.principal, AuthPrincipal::AgentSession(_)) {
                let from = rt.knowledge_get(edge.from.as_str()).await?;
                let to = rt.knowledge_get(edge.to.as_str()).await?;
                enforce_node_domain(auth, &from)?;
                enforce_node_domain(auth, &to)?;
            }
            let created = match &auth.principal {
                AuthPrincipal::AgentSession(capability) => {
                    rt.knowledge_propose_edge_for_session(
                        edge,
                        knowledge_actor(auth),
                        capability.session_id.clone(),
                    )
                    .await?
                }
                _ => {
                    rt.knowledge_propose_edge(edge, knowledge_actor(auth))
                        .await?
                }
            };
            Ok(serde_json::to_value(created).unwrap_or_default())
        }
        "interaction.knowledge_propose_supersede" => {
            let supersedes = required(&input, "supersedes")?;
            let prior = rt.knowledge_get(&supersedes).await?;
            enforce_node_domain(auth, &prior)?;
            let mut merged = input.clone();
            if let Some(obj) = merged.as_object_mut() {
                obj.insert(
                    "nodeType".into(),
                    serde_json::to_value(prior.node_type).unwrap_or(json!("claim")),
                );
                if !obj.contains_key("domains") {
                    obj.insert("domains".into(), json!(prior.domains));
                }
            }
            let node = interaction_runtime::knowledge::node_from_input(&merged)
                .map_err(DomainError::Validation)?;
            enforce_proposal_domains(auth, &node.domains)?;
            let created = match &auth.principal {
                AuthPrincipal::AgentSession(capability) => {
                    rt.knowledge_propose_node_for_session(
                        node,
                        knowledge_actor(auth),
                        capability.session_id.clone(),
                    )
                    .await?
                }
                _ => {
                    rt.knowledge_propose_node(node, knowledge_actor(auth))
                        .await?
                }
            };
            Ok(serde_json::to_value(created).unwrap_or_default())
        }
        "interaction.knowledge_submit_review" => {
            let id = required(&input, "nodeId")?;
            let note = required(&input, "note")?;
            let target = rt.knowledge_get(&id).await?;
            enforce_node_domain(auth, &target)?;
            let node = rt
                .knowledge_review(&id, "comment", Some(note), knowledge_actor(auth))
                .await?;
            Ok(serde_json::to_value(node).unwrap_or_default())
        }
        other => Err(ApiError::from(DomainError::NotFound(format!(
            "tool {other}"
        )))),
    }
}

fn knowledge_actor(auth: &AuthContext) -> interaction_core::MemoryActor {
    match &auth.principal {
        AuthPrincipal::AgentSession(capability) => interaction_core::MemoryActor::Agent(format!(
            "{}@{}",
            capability.agent_id, capability.session_id
        )),
        _ => interaction_core::MemoryActor::Agent("ai-host".into()),
    }
}

fn enforce_node_domain(
    auth: &AuthContext,
    node: &interaction_core::KnowledgeNode,
) -> Result<(), ApiError> {
    if let AuthPrincipal::AgentSession(capability) = &auth.principal {
        if !capability.allows_domain(&node.domains) {
            return Err(ApiError::from(DomainError::PolicyBlocked(
                "knowledge node 不在此 Agent Session 的 Domain scope".into(),
            )));
        }
    }
    Ok(())
}

fn enforce_proposal_domains(auth: &AuthContext, domains: &[String]) -> Result<(), ApiError> {
    if let AuthPrincipal::AgentSession(capability) = &auth.principal {
        let allowed = capability.domains.contains("*")
            || (!domains.is_empty()
                && domains
                    .iter()
                    .all(|domain| capability.domains.contains(domain)));
        if !allowed {
            return Err(ApiError::from(DomainError::PolicyBlocked(
                "knowledge proposal 的每個 Domain 都必須在 Agent Session scope 內".into(),
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Emergency stop / misc
// ---------------------------------------------------------------------------

pub async fn emergency_stop(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
    body: Option<Json<EmergencyStopInput>>,
) -> ApiResult<Json<Value>> {
    let reason = body.and_then(|Json(b)| b.reason);
    // safety-invariants-057：agent／session token 也能按下緊急停止，
    // audit 必須看得出是誰按的（比照 sensors_stop）。
    let result = state
        .runtime
        .emergency_stop(&stop_actor(&auth), reason)
        .await?;
    Ok(Json(result))
}

pub async fn emergency_stop_clear(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    state.runtime.clear_emergency_stop("api").await?;
    Ok(Json(json!({"cleared": true})))
}

pub async fn stop_all(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<Value>> {
    let count = state.runtime.stop_all(&stop_actor(&auth)).await?;
    Ok(Json(json!({"cancelled": count})))
}

#[derive(serde::Deserialize, Default)]
pub struct LimitQuery {
    #[serde(default)]
    limit: Option<u32>,
}

pub async fn outbox(State(state): State<ApiState>, Query(q): Query<LimitQuery>) -> Json<Value> {
    Json(json!(state
        .runtime
        .outbox
        .recent(q.limit.unwrap_or(50).min(200) as usize)))
}

pub async fn audit(
    State(state): State<ApiState>,
    Query(q): Query<LimitQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(state
        .runtime
        .store
        .audit_tail(q.limit.unwrap_or(50).min(500))?)))
}

pub async fn openapi(State(state): State<ApiState>) -> Json<Value> {
    let manifests = state.runtime.registry.tool_operations().await;
    Json(interaction_tool_schema::to_openapi(&manifests))
}

// ---------------------------------------------------------------------------
// Human layer: catalog, human capabilities, preferences, onboarding, pause,
// AI descriptions, AI assists, recipe summaries and scenario simulation.
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HumanQuery {
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub include_unavailable: bool,
}

pub async fn catalog(Query(q): Query<HumanQuery>) -> Json<Value> {
    let catalog = interaction_registry::catalog::Catalog::builtin();
    let _ = q; // full catalog is localized client-side via LocalizedText maps
    Json(serde_json::to_value(catalog).unwrap_or_default())
}

pub async fn capabilities_human(
    State(state): State<ApiState>,
    Query(q): Query<HumanQuery>,
) -> Json<Value> {
    Json(
        state
            .runtime
            .human_capabilities(q.locale.as_deref().unwrap_or(""), q.include_unavailable)
            .await,
    )
}

pub async fn ui_preferences_get(State(state): State<ApiState>) -> Json<Value> {
    Json(serde_json::to_value(state.runtime.ui_preferences().await).unwrap_or_default())
}

pub async fn ui_preferences_patch(
    State(state): State<ApiState>,
    Json(patch): Json<Value>,
) -> ApiResult<Json<Value>> {
    let updated = state.runtime.update_ui_preferences(patch).await?;
    Ok(Json(serde_json::to_value(updated).unwrap_or_default()))
}

pub async fn onboarding_get(State(state): State<ApiState>) -> Json<Value> {
    Json(state.runtime.onboarding_state().await)
}

pub async fn onboarding_draft_put(
    State(state): State<ApiState>,
    Json(draft): Json<Value>,
) -> ApiResult<Json<Value>> {
    state.runtime.save_onboarding_draft(draft).await?;
    Ok(Json(json!({"saved": true})))
}

/// Dry run: same validation as commit, no side effects. The wizard's
/// 「套用前確認」dialog is built from this, so it quotes Runtime truth.
pub async fn onboarding_preview(
    State(state): State<ApiState>,
    Json(commit): Json<interaction_runtime::human::OnboardingCommit>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.preview_onboarding(commit).await?))
}

pub async fn onboarding_commit(
    State(state): State<ApiState>,
    Json(commit): Json<interaction_runtime::human::OnboardingCommit>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.commit_onboarding(commit).await?))
}

pub async fn pause_get(State(state): State<ApiState>) -> Json<Value> {
    Json(serde_json::to_value(state.runtime.pause_status().await).unwrap_or_default())
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PauseBody {
    #[serde(default)]
    pub duration_minutes: Option<u64>,
    #[serde(default)]
    pub until: Option<interaction_core::Timestamp>,
    #[serde(default)]
    pub reason: Option<String>,
}

pub async fn pause_set(
    State(state): State<ApiState>,
    Json(body): Json<PauseBody>,
) -> ApiResult<Json<Value>> {
    let until = body.until.or_else(|| {
        body.duration_minutes
            .map(|m| chrono::Utc::now() + chrono::Duration::minutes(m.min(7 * 24 * 60) as i64))
    });
    let st = state
        .runtime
        .pause_proactive(until, body.reason, "api")
        .await?;
    Ok(Json(serde_json::to_value(st).unwrap_or_default()))
}

pub async fn pause_clear(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    let st = state.runtime.resume_proactive("api").await?;
    Ok(Json(serde_json::to_value(st).unwrap_or_default()))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiDescriptionBody {
    pub locale: String,
    pub text: String,
    pub manifest_hash: String,
}

pub async fn ai_description_put(
    State(state): State<ApiState>,
    Path((kind, id)): Path<(String, String)>,
    Json(body): Json<AiDescriptionBody>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state
            .runtime
            .set_capability_ai_description(
                &kind,
                &id,
                &body.locale,
                &body.text,
                &body.manifest_hash,
            )
            .await?,
    ))
}

pub async fn ai_assists_list(State(state): State<ApiState>) -> Json<Value> {
    Json(json!(state.runtime.pending_ai_assists().await))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistResolveBody {
    pub decision: String,
    #[serde(default)]
    pub note: Option<String>,
}

pub async fn ai_assist_resolve(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<AssistResolveBody>,
) -> ApiResult<Json<Value>> {
    // The HTTP surface is the AI host's surface: it can never satisfy a
    // recipe's requireHumanConfirmation gate.
    Ok(Json(
        state
            .runtime
            .resolve_ai_assist(&id, &body.decision, body.note, false)
            .await?,
    ))
}

pub async fn recipe_summary(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<HumanQuery>,
) -> ApiResult<Json<Value>> {
    let locale = q.locale.as_deref().unwrap_or("zh-TW");
    let summary = state.runtime.recipe_summary(&id, locale).await?;
    Ok(Json(
        json!({"recipeId": id, "locale": locale, "summary": summary}),
    ))
}

pub async fn recipe_simulate_scenario(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(scenario): Json<interaction_runtime::human::SimScenario>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state
            .runtime
            .simulate_recipe_scenario(&id, scenario)
            .await?,
    ))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertBody {
    pub text: String,
    /// `yaml` or `json`.
    pub to: String,
}

/// Convert a recipe between YAML and JSON through the single domain model,
/// preserving unknown fields. This is what keeps visual editing lossless.
pub async fn recipe_convert(Json(body): Json<ConvertBody>) -> ApiResult<Json<Value>> {
    let recipe = match interaction_recipe::parse_and_validate(&body.text) {
        Ok(r) => r,
        Err(interaction_recipe::RecipeParseError::Invalid(issues)) => {
            return Ok(Json(json!({"valid": false, "issues": issues})));
        }
        Err(e) => {
            return Ok(Json(
                json!({"valid": false, "issues": [{"field": "$", "message": e.to_string()}]}),
            ));
        }
    };
    let out = match body.to.as_str() {
        "yaml" => interaction_recipe::to_yaml(&recipe)
            .map_err(|e| ApiError::from(DomainError::Internal(e)))?,
        "json" => interaction_recipe::to_json_pretty(&recipe)
            .map_err(|e| ApiError::from(DomainError::Internal(e)))?,
        other => {
            return Err(ApiError::from(DomainError::Validation(format!(
                "to must be 'yaml' or 'json', got {other:?}"
            ))));
        }
    };
    Ok(Json(json!({"valid": true, "recipe": recipe, "text": out})))
}

// ---------------------------------------------------------------------------
// Providers (devices / services / agents)
// ---------------------------------------------------------------------------

/// 裝置身分指紋屬於人類層的配對資訊（`/v1/mobile` 整條路由 agent token 都讀
/// 不到）：從 `/v1/providers` 繞過去讀一樣不行。非人類 principal 一律拿不到。
fn redact_identity_for(auth: &AuthContext, desc: &mut interaction_core::ProviderDescriptor) {
    if !matches!(auth.principal, AuthPrincipal::Human) {
        desc.identity.fingerprint = None;
    }
}

pub async fn providers_list(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
) -> Json<Value> {
    let mut providers = state.runtime.list_providers().await;
    for desc in &mut providers {
        redact_identity_for(&auth, desc);
    }
    Json(json!(providers))
}

pub async fn provider_get(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let mut desc = state
        .runtime
        .get_provider(&interaction_core::ProviderId::new(&id))
        .await?;
    redact_identity_for(&auth, &mut desc);
    Ok(Json(serde_json::to_value(desc).unwrap_or_default()))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairBody {
    pub pairing_code: String,
}

pub async fn provider_pair(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<PairBody>,
) -> ApiResult<Json<Value>> {
    let desc = state
        .runtime
        .pair_provider(&interaction_core::ProviderId::new(&id), &body.pairing_code)
        .await?;
    Ok(Json(serde_json::to_value(desc).unwrap_or_default()))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionBody {
    pub state: interaction_core::ProviderState,
}

pub async fn provider_transition(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<TransitionBody>,
) -> ApiResult<Json<Value>> {
    let desc = state
        .runtime
        .transition_provider(&interaction_core::ProviderId::new(&id), body.state)
        .await?;
    Ok(Json(serde_json::to_value(desc).unwrap_or_default()))
}

/// 「測試裝置」（spec §9.3）：人類專用（agent／session token 在 lib.rs 的
/// scope 守門一律 403）。只對該 provider 的第一個可讀受器做一次讀取——
/// 不觸發任何動器、不產生外部副作用；成功記 tested、失敗記 ok:false＋原因。
pub async fn provider_test(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let report = state
        .runtime
        .test_provider(&interaction_core::ProviderId::new(&id))
        .await?;
    Ok(Json(report))
}

pub async fn provider_revoke(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let desc = state
        .runtime
        .revoke_provider(&interaction_core::ProviderId::new(&id))
        .await?;
    Ok(Json(serde_json::to_value(desc).unwrap_or_default()))
}

// ---------------------------------------------------------------------------
// Agent sessions (leased, budgeted, mailbox-only communication)
// ---------------------------------------------------------------------------

pub async fn agent_sessions_list(State(state): State<ApiState>) -> Json<Value> {
    Json(json!(state.runtime.list_agent_sessions().await))
}

pub async fn agent_session_create(
    State(state): State<ApiState>,
    Json(input): Json<interaction_runtime::agents::CreateAgentSession>,
) -> ApiResult<Json<Value>> {
    let record = state.runtime.create_agent_session(input).await?;
    Ok(Json(serde_json::to_value(record).unwrap_or_default()))
}

pub async fn agent_session_get(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        serde_json::to_value(state.runtime.get_agent_session(&id).await?).unwrap_or_default(),
    ))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReportBody {
    pub event: String,
    #[serde(default)]
    pub payload: Value,
}

pub async fn agent_session_report(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<SessionReportBody>,
) -> ApiResult<Json<Value>> {
    let record = state
        .runtime
        .report_agent_session(&id, &body.event, body.payload)
        .await?;
    Ok(Json(serde_json::to_value(record).unwrap_or_default()))
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionVerifyBody {
    #[serde(default)]
    pub note: Option<String>,
}

/// 人工驗證 claimed-completed（human token 專屬：agent/session token 對本
/// 路由一律 403——見 lib.rs 的 scope 守門）。
pub async fn agent_session_verify(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: Option<Json<SessionVerifyBody>>,
) -> ApiResult<Json<Value>> {
    let note = body.and_then(|Json(b)| b.note);
    let record = state.runtime.verify_agent_session(&id, note).await?;
    Ok(Json(serde_json::to_value(record).unwrap_or_default()))
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MailboxQuery {
    #[serde(default)]
    pub direction: Option<String>,
}

/// 讀信箱。**誰在讀**決定這是不是「送達」：human token 的 GET 是純觀看
/// （唯讀，不蓋 deliveredAt、不把委派 receipt 推到 acknowledged），只有
/// agent 身分的讀取才帶送達語意。人看一眼信箱不等於 agent 收到了任務。
pub async fn agent_session_messages(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
    Query(q): Query<MailboxQuery>,
) -> ApiResult<Json<Value>> {
    let direction = match q.direction.as_deref() {
        Some("from-session") => interaction_core::MailboxDirection::FromSession,
        _ => interaction_core::MailboxDirection::ToSession,
    };
    let reader = match &auth.principal {
        AuthPrincipal::Human => interaction_runtime::agents::MailboxReader::Human,
        // agent-honesty-021：agent 身分的讀取＝取走（蓋 deliveredAt、推進委派
        // receipt、發 `fetched`），所以只有「自己的 session」才算得上取走。
        // 擁有權在這裡再比對一次，不依賴 middleware 的掛載順序（比照 078）。
        AuthPrincipal::AgentSession(capability) if capability.session_id == id => {
            interaction_runtime::agents::MailboxReader::Agent
        }
        // 跨 session 的 capability token 證明不了擁有權；legacy agent token
        // 是零欄位 variant，架構上不帶任何 session 身分——都不得取走信箱
        // （中介層已擋下，這是型別上的最後防線）。
        AuthPrincipal::AgentSession(_) | AuthPrincipal::LegacyAgent => {
            return Err(ApiError::forbidden_scope())
        }
        AuthPrincipal::CharacterAdapter { .. } => return Err(ApiError::forbidden_adapter_scope()),
    };
    let messages = state.runtime.mailbox_read(&id, direction, reader).await?;
    Ok(Json(json!(messages)))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxSendBody {
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub body: std::collections::BTreeMap<String, Value>,
}

fn default_kind() -> String {
    "message".into()
}

pub async fn agent_session_send(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<MailboxSendBody>,
) -> ApiResult<Json<Value>> {
    let direction = match body.direction.as_deref() {
        Some("from-session") => interaction_core::MailboxDirection::FromSession,
        _ => interaction_core::MailboxDirection::ToSession,
    };
    let message = state
        .runtime
        .mailbox_send(&id, direction, &body.kind, body.body, None)
        .await?;
    Ok(Json(serde_json::to_value(message).unwrap_or_default()))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenewBody {
    #[serde(default = "default_renew")]
    pub extra_minutes: u32,
}

fn default_renew() -> u32 {
    30
}

pub async fn agent_session_renew(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<RenewBody>,
) -> ApiResult<Json<Value>> {
    let record = state
        .runtime
        .renew_agent_session(&id, body.extra_minutes)
        .await?;
    Ok(Json(serde_json::to_value(record).unwrap_or_default()))
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CloseBody {
    #[serde(default)]
    pub handoff: Option<interaction_core::HandoffSummary>,
    #[serde(default)]
    pub reason: Option<String>,
}

pub async fn agent_session_close(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: Option<Json<CloseBody>>,
) -> ApiResult<Json<Value>> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let record = state
        .runtime
        .close_agent_session(
            &id,
            body.handoff,
            body.reason.as_deref().unwrap_or("closed"),
        )
        .await?;
    Ok(Json(serde_json::to_value(record).unwrap_or_default()))
}

// ---------------------------------------------------------------------------
// Sensors (microphone listen windows; always-visible indicators)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MicListenBody {
    #[serde(default = "default_listen_ms")]
    pub duration_ms: u64,
}

fn default_listen_ms() -> u64 {
    10_000
}

pub async fn sensor_mic_listen(
    State(state): State<ApiState>,
    body: Option<Json<MicListenBody>>,
) -> ApiResult<Json<Value>> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let out = state
        .runtime
        .begin_mic_listen(body.duration_ms, "api")
        .await?;
    Ok(Json(json!(out)))
}

/// 停止所有感測（本機麥克風＋每一台已連線 iPhone）。
///
/// 誠實階梯：頂層 `stopped` ＝**所有**來源都確認停止；任一台沒回覆時
/// `uncertain: true`（手機可能還在錄音）。安全遞減操作，agent／session token
/// 也可呼叫——audit actor 記的是實際呼叫者，不是一律 "api"。
pub async fn sensors_stop(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<Value>> {
    let report = state.runtime.stop_all_sensors(&stop_actor(&auth)).await?;
    Ok(Json(
        serde_json::to_value(report).unwrap_or_else(|_| json!({})),
    ))
}

/// 誰按下的停止：agent／session token 也能停感測，audit 必須看得出是誰。
fn stop_actor(auth: &AuthContext) -> String {
    match &auth.principal {
        AuthPrincipal::Human => "api".into(),
        AuthPrincipal::LegacyAgent => "agent".into(),
        AuthPrincipal::AgentSession(capability) => {
            format!("agent:{}@{}", capability.agent_id, capability.session_id)
        }
        AuthPrincipal::CharacterAdapter { adapter_id } => format!("adapter:{adapter_id}"),
    }
}

// ---------------------------------------------------------------------------
// Presentation（桌面角色表面）：presence 心跳＋命令 ack。
// ---------------------------------------------------------------------------

pub async fn presentation_status(State(state): State<ApiState>) -> Json<Value> {
    Json(state.runtime.presentation_status())
}

pub async fn presentation_pending_command(
    State(state): State<ApiState>,
    Path(action_id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state.runtime.presentation_pending_command(&action_id)?,
    ))
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PresentationHelloBody {
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub pack_id: Option<String>,
    #[serde(default)]
    pub behavior_state: Option<Value>,
}

pub async fn presentation_hello(
    State(state): State<ApiState>,
    body: Option<Json<PresentationHelloBody>>,
) -> Json<Value> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    Json(
        state
            .runtime
            .presentation_hello_with_behavior(body.visible, body.pack_id, body.behavior_state)
            .await,
    )
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationAckBody {
    pub action_id: String,
    pub outcome: String,
    #[serde(default)]
    pub detail: Option<String>,
}

pub async fn presentation_ack(
    State(state): State<ApiState>,
    Json(body): Json<PresentationAckBody>,
) -> ApiResult<Json<Value>> {
    let out = state
        .runtime
        .presentation_ack(&body.action_id, &body.outcome, body.detail)
        .await?;
    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// 主動式對話政策（確定性頻率限制）。
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Character Presentation Protocol（docs/character-protocol/README.md）
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterHelloBody {
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub role: Option<interaction_character::CharacterRole>,
    pub manifest: interaction_character::CharacterManifest,
    pub negotiate: interaction_character::Negotiate,
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub pack_id: Option<String>,
    #[serde(default)]
    pub behavior_state: Option<Value>,
    /// 視窗回報的 Reduced Motion（`prefers-reduced-motion` 或使用者偏好）；省略＝false。
    #[serde(default)]
    pub reduced_motion: bool,
}

/// 桌面視窗（可信 host）登記角色並協商；同 instanceId 重送＝重新協商（generation+1）。
pub async fn character_hello(
    State(state): State<ApiState>,
    Json(body): Json<CharacterHelloBody>,
) -> ApiResult<Json<Value>> {
    let out = state
        .runtime
        .character_hello(interaction_runtime::character::CharacterHelloInput {
            instance_id: body.instance_id,
            role: body.role,
            manifest: body.manifest,
            negotiate: body.negotiate,
            visible: body.visible,
            pack_id: body.pack_id,
            behavior_state: body.behavior_state,
            reduced_motion: body.reduced_motion,
        })
        .await?;
    Ok(Json(out))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterReceiptBody {
    pub instance_id: String,
    pub receipt: interaction_character::CommandReceipt,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterEventBody {
    pub instance_id: String,
    pub event: interaction_character::CharacterInputEvent,
}

/// adapter token 只能替自己的 instance（`adapter:<id>`）說話。
fn enforce_character_instance(auth: &AuthContext, instance_id: &str) -> Result<(), ApiError> {
    if let AuthPrincipal::CharacterAdapter { adapter_id } = &auth.principal {
        if interaction_runtime::character::adapter_instance_id(adapter_id) != instance_id {
            return Err(ApiError::forbidden_adapter_scope());
        }
    }
    Ok(())
}

pub async fn character_receipts(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<CharacterReceiptBody>,
) -> ApiResult<Json<Value>> {
    enforce_character_instance(&auth, &body.instance_id)?;
    let out = state
        .runtime
        .character_receipt(&body.instance_id, body.receipt)
        .await?;
    Ok(Json(out))
}

pub async fn character_events(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<CharacterEventBody>,
) -> ApiResult<Json<Value>> {
    enforce_character_instance(&auth, &body.instance_id)?;
    let out = state
        .runtime
        .character_event(&body.instance_id, body.event)
        .await?;
    Ok(Json(out))
}

pub async fn character_instances(State(state): State<ApiState>) -> Json<Value> {
    Json(state.runtime.character_instances())
}

/// 目前桌面角色的 manifest；尚未 hello → 404。
pub async fn character_manifest(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    let manifest = state.runtime.character_manifest().ok_or_else(|| {
        ApiError::from(DomainError::NotFound(
            "no desktop character negotiated yet (POST /v1/character/hello)".into(),
        ))
    })?;
    Ok(Json(serde_json::to_value(manifest).unwrap_or_default()))
}

pub async fn character_adapters_list(State(state): State<ApiState>) -> Json<Value> {
    Json(state.runtime.character_adapters())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAdapterBody {
    pub display_name: String,
    pub manifest: interaction_character::CharacterManifest,
}

/// 註冊外部 adapter：回 adapterId＋**只此一次**的 token（Runtime 只存 sha256）。
pub async fn character_adapter_add(
    State(state): State<ApiState>,
    Json(body): Json<CharacterAdapterBody>,
) -> ApiResult<Json<Value>> {
    let out = state
        .runtime
        .character_adapter_add(&body.display_name, body.manifest)
        .await?;
    Ok(Json(out))
}

pub async fn character_adapter_revoke(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.character_adapter_revoke(&id).await?))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterIntentBody {
    pub intent: String,
    #[serde(default)]
    pub message: Option<String>,
}

/// 人類手動測試：只允許非安全 intent（truthState 固定 none）。
pub async fn character_intent(
    State(state): State<ApiState>,
    Json(body): Json<CharacterIntentBody>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state
            .runtime
            .character_manual_intent(&body.intent, body.message)
            .await?,
    ))
}

// ---------------------------------------------------------------------------
// AIP Character Session（human token 專屬；agent／session／adapter token 一律 403）
// ---------------------------------------------------------------------------

/// `INTERACT_AI_CHARACTER_SESSION=0`：503＋穩定錯誤碼 `session-disabled`
/// （`docs/aip/README.md` §12）。
fn session_disabled() -> ApiError {
    ApiError {
        status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
        code: "session-disabled",
        message: interaction_runtime::character_session::SESSION_DISABLED_MESSAGE.to_string(),
    }
}

fn require_character_session(state: &ApiState) -> Result<(), ApiError> {
    if state.runtime.character_session_enabled() {
        Ok(())
    } else {
        Err(session_disabled())
    }
}

/// 桌面可信 host surface 的身分（human token → `{kind:"human-surface", id:"desktop"}`）。
fn human_surface() -> interaction_runtime::character_session::Party {
    interaction_runtime::character_session::desktop_party()
}

/// `GET /v1/character-session`：權威 snapshot（`state{kind:"snapshot"}` envelope）。
pub async fn character_session_snapshot(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    require_character_session(&state)?;
    let envelope = state
        .runtime
        .character_session_snapshot_envelope(&human_surface())
        .await?;
    Ok(Json(serde_json::to_value(envelope).unwrap_or_default()))
}

/// `GET /v1/character-session/diagnostics`：§10（不含 token、路徑、原始 payload）。
pub async fn character_session_diagnostics(
    State(state): State<ApiState>,
) -> ApiResult<Json<Value>> {
    require_character_session(&state)?;
    Ok(Json(state.runtime.character_session_diagnostics_value()?))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSessionResumeBody {
    #[serde(default)]
    pub last_revision: u64,
    #[serde(default)]
    pub last_sequence: u64,
    /// 對端記得的 sessionEpoch；不同 → session-reset snapshot。
    #[serde(default, alias = "sessionEpoch")]
    pub epoch: u64,
}

/// `POST /v1/character-session/resume`：patches 或 snapshot（§6）。
pub async fn character_session_resume(
    State(state): State<ApiState>,
    Json(body): Json<CharacterSessionResumeBody>,
) -> ApiResult<Json<Value>> {
    require_character_session(&state)?;
    let party = human_surface();
    let resume = state
        .runtime
        .character_session_resume(&party, body.last_revision, body.last_sequence, body.epoch)
        .await?;
    Ok(Json(
        state
            .runtime
            .character_session_resume_value(&party, resume)
            .await,
    ))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSessionEventBody {
    pub envelope: interaction_runtime::character_session::Envelope,
}

/// `POST /v1/character-session/events`：可信 host surface 送語意事件（桌面的點擊）。
/// 身分是綁定出來的：`source` 必須是 `{kind:"human-surface", id:"desktop"}`。
pub async fn character_session_event(
    State(state): State<ApiState>,
    Json(body): Json<CharacterSessionEventBody>,
) -> ApiResult<Json<Value>> {
    require_character_session(&state)?;
    let submission = state
        .runtime
        .character_session_submit(body.envelope, &human_surface())
        .await?;
    Ok(Json(
        serde_json::to_value(submission.result).unwrap_or_default(),
    ))
}

pub async fn proactive_dialogue_get(State(state): State<ApiState>) -> Json<Value> {
    Json(state.runtime.proactive_dialogue_status().await)
}

pub async fn proactive_dialogue_patch(
    State(state): State<ApiState>,
    Json(patch): Json<Value>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state.runtime.proactive_dialogue_configure(patch).await?,
    ))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuietBody {
    pub minutes: i64,
}

pub async fn proactive_dialogue_quiet(
    State(state): State<ApiState>,
    Json(body): Json<QuietBody>,
) -> Json<Value> {
    Json(state.runtime.proactive_dialogue_quiet(body.minutes).await)
}

// ---------------------------------------------------------------------------
// Agent Gateway：本機 agent 發現／approval／中斷／路由建議。
// ---------------------------------------------------------------------------

pub async fn agents_discoveries(State(state): State<ApiState>) -> Json<Value> {
    Json(json!({"agents": state.runtime.agent_discoveries()}))
}

pub async fn agents_refresh(State(state): State<ApiState>) -> Json<Value> {
    Json(json!({"agents": state.runtime.refresh_agent_providers().await}))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveBody {
    pub request_id: String,
    #[serde(default)]
    pub approve: bool,
}

pub async fn agent_session_approve(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<ApproveBody>,
) -> ApiResult<Json<Value>> {
    let out = state
        .runtime
        .gateway_resolve_approval(&id, &body.request_id, body.approve)
        .await?;
    Ok(Json(out))
}

/// 中斷單一 session 的目前 turn。擁有權：session-scoped capability token 只能
/// 中斷自己的 session（middleware 的 `session_request_allowed` 已擋下跨 session，
/// 這裡是 defense-in-depth 的第二層，避免未來路由／middleware 順序變動時裸露這個
/// handler）；legacy 共享 agent token 沒有 session 身分，在 middleware 就是 403。
pub async fn agent_session_interrupt(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !crate::interrupt_principal_allowed(&auth.principal, &id) {
        return Err(ApiError::forbidden_scope());
    }
    Ok(Json(state.runtime.gateway_interrupt(&id).await?))
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RoutingQuery {
    #[serde(default)]
    kind: Option<String>,
}

pub async fn agents_routing(
    State(state): State<ApiState>,
    axum::extract::Query(q): axum::extract::Query<RoutingQuery>,
) -> Json<Value> {
    Json(
        state
            .runtime
            .agent_route_suggestion(q.kind.as_deref())
            .await,
    )
}

// ---------------------------------------------------------------------------
// 記憶層（spec §10/§15/§16）。
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListQuery {
    #[serde(default)]
    layer: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

pub async fn memory_list(
    State(state): State<ApiState>,
    axum::extract::Query(q): axum::extract::Query<MemoryListQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state
            .runtime
            .memory_list(q.layer.as_deref(), q.limit.unwrap_or(200))
            .await?,
    ))
}

pub async fn memory_create(
    State(state): State<ApiState>,
    Json(input): Json<Value>,
) -> ApiResult<Json<Value>> {
    // 此 human-only 端點也允許人類明確以 agent 身分匯入；宣告 agent
    // 只會降權（fact→inference、長期使用者記憶→candidate），永遠不會升權。
    let actor = match input.get("asAgent").and_then(|v| v.as_str()) {
        Some(agent) => interaction_core::MemoryActor::Agent(agent.to_string()),
        None => interaction_core::MemoryActor::Human,
    };
    let item = interaction_runtime::memory::memory_from_input(input, actor)
        .map_err(interaction_core::DomainError::Validation)?;
    let created = state.runtime.memory_create(item).await?;
    Ok(Json(serde_json::to_value(created).unwrap_or_default()))
}

pub async fn memory_get(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let item = state.runtime.memory_get(&id).await?;
    Ok(Json(serde_json::to_value(item).unwrap_or_default()))
}

pub async fn memory_patch(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(patch): Json<Value>,
) -> ApiResult<Json<Value>> {
    let item = state.runtime.memory_update(&id, patch).await?;
    Ok(Json(serde_json::to_value(item).unwrap_or_default()))
}

pub async fn memory_delete(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let deleted = state.runtime.memory_delete(&id).await?;
    Ok(Json(json!({"deleted": deleted})))
}

pub async fn memory_export(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.memory_export().await?))
}

pub async fn memory_clear_session(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    let n = state.runtime.memory_clear_session_context().await?;
    Ok(Json(json!({"cleared": n})))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleBody {
    pub task: String,
    #[serde(default)]
    pub domains: Vec<String>,
    pub agent_id: String,
}

pub async fn memory_context_bundle(
    State(state): State<ApiState>,
    Json(body): Json<BundleBody>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state
            .runtime
            .memory_context_bundle(&body.task, &body.domains, &body.agent_id)
            .await?,
    ))
}

// ---------------------------------------------------------------------------
// 知識系統（人類介面）：素材 CAS＋圖譜＋複審。
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetImportBody {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub media_type: Option<interaction_core::MediaType>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

pub async fn asset_import(
    State(state): State<ApiState>,
    Json(body): Json<AssetImportBody>,
) -> ApiResult<Json<Value>> {
    let record = state
        .runtime
        .asset_import(
            body.path.as_deref(),
            body.content.as_deref(),
            body.media_type,
            body.source.as_deref().unwrap_or("user-import"),
            body.description,
        )
        .await?;
    Ok(Json(serde_json::to_value(record).unwrap_or_default()))
}

pub async fn assets_list(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.asset_list(200).await?))
}

pub async fn asset_get(
    State(state): State<ApiState>,
    Path(hash): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        serde_json::to_value(state.runtime.asset_get(&hash).await?).unwrap_or_default(),
    ))
}

pub async fn asset_impact(
    State(state): State<ApiState>,
    Path(hash): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.asset_delete_impact(&hash).await?))
}

pub async fn asset_delete(
    State(state): State<ApiState>,
    Path(hash): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.asset_delete(&hash).await?))
}

pub async fn asset_content(
    State(state): State<ApiState>,
    Path(hash): Path<String>,
) -> ApiResult<impl axum::response::IntoResponse> {
    let bytes = state.runtime.asset_content(&hash, 8 * 1024 * 1024).await?;
    Ok(([("content-type", "application/octet-stream")], bytes))
}

pub async fn asset_preview(
    State(state): State<ApiState>,
    Path(hash): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.asset_preview(&hash).await?))
}

pub async fn asset_derivatives(
    State(state): State<ApiState>,
    Path(hash): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "derivatives": state.runtime.asset_derivatives(&hash).await?
    })))
}

pub async fn asset_derive(
    State(state): State<ApiState>,
    Path(hash): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        serde_json::to_value(state.runtime.asset_derive(&hash).await?).unwrap_or_default(),
    ))
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeListQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

pub async fn knowledge_domain_packs(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.domain_packs_list()?))
}

pub async fn knowledge_domain_pack_install(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.domain_pack_install(&id)?))
}

pub async fn knowledge_domain_pack_uninstall(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.domain_pack_uninstall(&id)?))
}

pub async fn knowledge_nodes_list(
    State(state): State<ApiState>,
    axum::extract::Query(q): axum::extract::Query<KnowledgeListQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state
            .runtime
            .knowledge_list(q.status.as_deref(), q.limit.unwrap_or(100))
            .await?,
    ))
}

pub async fn knowledge_node_create(
    State(state): State<ApiState>,
    Json(input): Json<Value>,
) -> ApiResult<Json<Value>> {
    // 人類介面：asAgent 只會降權（→Candidate）。
    let actor = match input.get("asAgent").and_then(|v| v.as_str()) {
        Some(a) => interaction_core::MemoryActor::Agent(a.to_string()),
        None => interaction_core::MemoryActor::Human,
    };
    let mut node = interaction_runtime::knowledge::node_from_input(&input)
        .map_err(interaction_core::DomainError::Validation)?;
    // 人類可直接建立 active（例如已確認的設計原則）。
    if matches!(actor, interaction_core::MemoryActor::Human)
        && input
            .get("activate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        node.status = interaction_core::KnowledgeStatus::Active;
    }
    let created = state.runtime.knowledge_propose_node(node, actor).await?;
    Ok(Json(serde_json::to_value(created).unwrap_or_default()))
}

pub async fn knowledge_node_get(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        serde_json::to_value(state.runtime.knowledge_get(&id).await?).unwrap_or_default(),
    ))
}

pub async fn knowledge_edge_create(
    State(state): State<ApiState>,
    Json(input): Json<Value>,
) -> ApiResult<Json<Value>> {
    let actor = match input.get("asAgent").and_then(|v| v.as_str()) {
        Some(a) => interaction_core::MemoryActor::Agent(a.to_string()),
        None => interaction_core::MemoryActor::Human,
    };
    let mut edge = interaction_runtime::knowledge::edge_from_input(&input)
        .map_err(interaction_core::DomainError::Validation)?;
    if matches!(actor, interaction_core::MemoryActor::Human)
        && input
            .get("activate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        edge.status = interaction_core::KnowledgeStatus::Active;
    }
    let created = state.runtime.knowledge_propose_edge(edge, actor).await?;
    Ok(Json(serde_json::to_value(created).unwrap_or_default()))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBody {
    pub verdict: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub as_agent: Option<String>,
}

pub async fn knowledge_node_review(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<ReviewBody>,
) -> ApiResult<Json<Value>> {
    let actor = match body.as_agent {
        Some(a) => interaction_core::MemoryActor::Agent(a),
        None => interaction_core::MemoryActor::Human,
    };
    let node = state
        .runtime
        .knowledge_review(&id, &body.verdict, body.note, actor)
        .await?;
    Ok(Json(serde_json::to_value(node).unwrap_or_default()))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub k: Option<u32>,
}

pub async fn knowledge_search(
    State(state): State<ApiState>,
    axum::extract::Query(q): axum::extract::Query<SearchQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        state
            .runtime
            .knowledge_search(&q.q, q.k.unwrap_or(10))
            .await?,
    ))
}

pub async fn knowledge_graph(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.knowledge_graph(&id, 1).await?))
}

// ---------------------------------------------------------------------------
// 知識更新決策器＋Receipts（spec §13/§17）。
// ---------------------------------------------------------------------------

pub async fn knowledge_receipts(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.knowledge_receipts(100).await?))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckBody {
    pub trigger: interaction_runtime::curator::UpdateTrigger,
}

pub async fn knowledge_update_check(
    State(state): State<ApiState>,
    Json(body): Json<UpdateCheckBody>,
) -> Json<Value> {
    Json(state.runtime.knowledge_update_decision(body.trigger))
}

pub async fn knowledge_user_correction(
    State(state): State<ApiState>,
    Json(input): Json<interaction_runtime::curator::UserCorrectionInput>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.record_user_correction(input).await?))
}

// ---------------------------------------------------------------------------
// iPhone Mobile Provider（v0.5 Phase 6；human-only routes）
// ---------------------------------------------------------------------------

pub async fn mobile_status(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.mobile_status().await?))
}

pub async fn mobile_pairing_begin(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.mobile_pairing_begin().await?))
}

pub async fn mobile_revoke(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.mobile_revoke(&id).await?))
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BleScanBody {
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// 指名由哪一台手機代掃。缺席時只有恰好一台手機連線才成立——
    /// 多台連線時 Runtime 誠實回 Err，不替使用者挑一台。
    #[serde(default)]
    pub device_id: Option<String>,
}

pub async fn mobile_ble_scan(
    State(state): State<ApiState>,
    body: Option<Json<BleScanBody>>,
) -> ApiResult<Json<Value>> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let duration = body.duration_ms.unwrap_or(4_000);
    Ok(Json(
        state
            .runtime
            .mobile_ble_scan(duration, body.device_id.as_deref())
            .await?,
    ))
}

/// 只停這一台手機的感測（有界等待確認；沒回覆＝outcome `unknown`）。
pub async fn mobile_sensors_stop(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.mobile_sensors_stop(&id).await?))
}

/// 測試這台手機的連線（WebSocket Ping／Pong）。
/// `ok` 只代表 socket 有回答，不代表 App 功能正常。
pub async fn mobile_test(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(state.runtime.mobile_test(&id).await?))
}

#[cfg(test)]
mod character_session_tests {
    use super::*;

    /// `INTERACT_AI_CHARACTER_SESSION=0` 的回應形狀是契約的一部分：
    /// HTTP 503＋穩定錯誤碼 `session-disabled`（`docs/aip/README.md` §12）。
    #[test]
    fn a_disabled_session_answers_503_with_the_stable_code() {
        let error = session_disabled();
        assert_eq!(error.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.code, "session-disabled");
        assert!(!error.message.is_empty());
        // 錯誤訊息不得回顯輸入、不得帶路徑。
        assert!(!error.message.contains('/'));
    }

    /// 可信 host surface 的身分是綁定出來的，不是呼叫端說了算。
    #[test]
    fn the_http_surface_is_always_the_desktop_human_surface() {
        let party = human_surface();
        assert_eq!(
            serde_json::to_value(&party).unwrap_or_default(),
            json!({"kind": "human-surface", "id": "desktop"})
        );
    }
}
