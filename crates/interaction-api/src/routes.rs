//! Route handlers. Thin: every handler delegates to the shared runtime
//! services (the same ones the CLI daemon and Tauri commands use).

use crate::dto::*;
use crate::error::{ApiError, ApiResult};
use crate::ApiState;
use axum::extract::{Path, Query, State};
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
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let receipt = state.runtime.cancel_action(&ActionId::new(&id)).await?;
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
        .grant_consent(&body.scope, body.expires_minutes)
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
    Path(name): Path<String>,
    body: Option<Json<Value>>,
) -> ApiResult<Json<Value>> {
    let input = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    // The manifest must exist (tool discovery = source of truth).
    let manifest = resolve_tool(&state, &name).await?;
    let result = dispatch_tool(&state, &manifest.name, input).await?;
    Ok(Json(result))
}

async fn dispatch_tool(state: &ApiState, name: &str, input: Value) -> Result<Value, ApiError> {
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
            Ok(
                serde_json::to_value(rt.cancel_action(&ActionId::new(&id)).await?)
                    .unwrap_or_default(),
            )
        }
        "interaction.stop" => {
            let reason = str_field(&input, "reason");
            Ok(rt.emergency_stop("tool-call", reason).await?)
        }
        "interaction.recipe_run" => {
            let id = required(&input, "recipeId")?;
            Ok(rt.run_recipe(&id).await?)
        }
        "interaction.policy" => Ok(serde_json::to_value(rt.policy().await).unwrap_or_default()),
        other => Err(ApiError::from(DomainError::NotFound(format!(
            "tool {other}"
        )))),
    }
}

// ---------------------------------------------------------------------------
// Emergency stop / misc
// ---------------------------------------------------------------------------

pub async fn emergency_stop(
    State(state): State<ApiState>,
    body: Option<Json<EmergencyStopInput>>,
) -> ApiResult<Json<Value>> {
    let reason = body.and_then(|Json(b)| b.reason);
    let result = state.runtime.emergency_stop("api", reason).await?;
    Ok(Json(result))
}

pub async fn emergency_stop_clear(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    state.runtime.clear_emergency_stop("api").await?;
    Ok(Json(json!({"cleared": true})))
}

pub async fn stop_all(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    let count = state.runtime.stop_all().await?;
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

pub async fn providers_list(State(state): State<ApiState>) -> Json<Value> {
    Json(json!(state.runtime.list_providers().await))
}

pub async fn provider_get(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let desc = state
        .runtime
        .get_provider(&interaction_core::ProviderId::new(&id))
        .await?;
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
pub struct MailboxQuery {
    #[serde(default)]
    pub direction: Option<String>,
}

pub async fn agent_session_messages(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<MailboxQuery>,
) -> ApiResult<Json<Value>> {
    let direction = match q.direction.as_deref() {
        Some("from-session") => interaction_core::MailboxDirection::FromSession,
        _ => interaction_core::MailboxDirection::ToSession,
    };
    let messages = state.runtime.mailbox_fetch(&id, direction).await?;
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

pub async fn sensors_stop(State(state): State<ApiState>) -> ApiResult<Json<Value>> {
    state.runtime.stop_all_sensors("api").await?;
    Ok(Json(json!({"stopped": true})))
}
