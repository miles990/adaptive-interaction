//! Tauri 2 control center backend.
//!
//! The desktop app embeds the SAME runtime application services used by the
//! CLI daemon and HTTP API (desktop-managed lifecycle). The single-instance
//! lock prevents a second runtime from owning the same devices; if a daemon
//! already runs, the UI surfaces that instead of silently double-driving.
//!
//! Security posture: no shell access, no raw device packets from the WebView,
//! read and write commands are separate, emergency stop is its own command,
//! and every input is re-validated by the runtime's policy governor.

mod supervisor;
mod tray;

use interaction_core::{
    ActionId, ActuatorId, DiscoveryContext, ObservationQuery, PlanId, ReceptorId,
};
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use supervisor::{DesktopPrefs, SupervisorInfo, SupervisorMode, SupervisorState};
use tauri::{Emitter, Manager, State};

/// Desktop app state: embedded runtime handle OR external-daemon connection,
/// plus desktop-local prefs (close behavior, companion) and the tray.
pub struct AppState {
    runtime: Mutex<Option<Runtime>>,
    startup_error: Mutex<Option<String>>,
    supervisor: Mutex<SupervisorInfo>,
    prefs: Mutex<DesktopPrefs>,
    quitting: AtomicBool,
    tray: Mutex<Option<tray::TrayHandles>>,
    /// Character hit-rect (logical px) inside the companion window.
    companion_hit_rect: Mutex<(f64, f64, f64, f64)>,
    companion_interactive: AtomicBool,
}

/// Alias used by the tray module.
pub type DesktopState = AppState;

/// Backend for tray/native actions: direct runtime calls in embedded mode,
/// authorized HTTP in external mode. The WebView is never involved.
#[derive(Clone)]
pub enum Backend {
    Embedded(Runtime),
    External { base: String, token: String },
}

impl Backend {
    pub async fn status(&self) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => Ok(rt.status().await),
            Backend::External { base, token } => {
                supervisor::daemon_get(base, token, "/v1/status").await
            }
        }
    }

    pub async fn pause_status(&self) -> Result<bool, String> {
        match self {
            Backend::Embedded(rt) => Ok(rt.pause_status().await.paused),
            Backend::External { base, token } => {
                let v = supervisor::daemon_get(base, token, "/v1/pause").await?;
                Ok(v.get("paused").and_then(Value::as_bool).unwrap_or(false))
            }
        }
    }

    pub async fn pause(&self, minutes: Option<u64>) -> Result<(), String> {
        match self {
            Backend::Embedded(rt) => {
                let until =
                    minutes.map(|m| chrono::Utc::now() + chrono::Duration::minutes(m as i64));
                rt.pause_proactive(until, Some("tray".into()), "tray")
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            Backend::External { base, token } => supervisor::daemon_post(
                base,
                token,
                "/v1/pause",
                json!({"durationMinutes": minutes, "reason": "tray"}),
            )
            .await
            .map(|_| ()),
        }
    }

    pub async fn resume(&self) -> Result<(), String> {
        match self {
            Backend::Embedded(rt) => rt
                .resume_proactive("tray")
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Backend::External { base, token } => {
                supervisor::daemon_post(base, token, "/v1/pause/clear", json!({}))
                    .await
                    .map(|_| ())
            }
        }
    }

    pub async fn emergency_stop(&self, actor: &str) -> Result<(), String> {
        match self {
            Backend::Embedded(rt) => rt
                .emergency_stop(actor, Some("tray emergency stop".into()))
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Backend::External { base, token } => supervisor::daemon_post(
                base,
                token,
                "/v1/emergency-stop",
                json!({"reason": "tray emergency stop"}),
            )
            .await
            .map(|_| ()),
        }
    }

    pub async fn presentation_pending_command(&self, action_id: &str) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => rt
                .presentation_pending_command(action_id)
                .map_err(|error| error.to_string()),
            Backend::External { base, token } => {
                supervisor::daemon_get(
                    base,
                    token,
                    &format!("/v1/presentation/commands/{action_id}"),
                )
                .await
            }
        }
    }
}

impl AppState {
    pub fn backend(&self) -> Option<Backend> {
        if let Some(rt) = self.runtime.lock().expect("runtime mutex").clone() {
            return Some(Backend::Embedded(rt));
        }
        let info = self.supervisor.lock().expect("supervisor mutex").clone();
        if info.mode == SupervisorMode::External {
            if let Some(token) = info.token {
                return Some(Backend::External {
                    base: info.api_base,
                    token,
                });
            }
        }
        None
    }
}

fn rt(state: &State<'_, AppState>) -> Result<Runtime, String> {
    state
        .runtime
        .lock()
        .expect("runtime mutex")
        .clone()
        .ok_or_else(|| {
            state
                .startup_error
                .lock()
                .expect("error mutex")
                .clone()
                .unwrap_or_else(|| "runtime not started".into())
        })
}

fn err_s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------------------------------------------------------------------------
// Read commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn status(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(runtime.status().await)
}

#[tauri::command]
async fn capabilities(
    state: State<'_, AppState>,
    include_unavailable: bool,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let snap = runtime
        .capabilities(&DiscoveryContext {
            include_unavailable,
            ..Default::default()
        })
        .await;
    serde_json::to_value(snap).map_err(err_s)
}

#[tauri::command]
async fn observations_query(state: State<'_, AppState>, query: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let query: ObservationQuery = serde_json::from_value(query).map_err(err_s)?;
    let out = runtime.observe_stored(&query).await.map_err(err_s)?;
    serde_json::to_value(out).map_err(err_s)
}

#[tauri::command]
async fn actions_list(state: State<'_, AppState>, limit: u32) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let out = runtime.list_actions(None, limit.min(200)).map_err(err_s)?;
    serde_json::to_value(out).map_err(err_s)
}

#[tauri::command]
async fn action_get(state: State<'_, AppState>, action_id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .get_action(&ActionId::new(&action_id))
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn policy_get(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.policy().await).map_err(err_s)
}

#[tauri::command]
async fn session_get(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(runtime
        .current_session()
        .await
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .unwrap_or(Value::Null))
}

#[tauri::command]
async fn recipes_list(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let list = runtime.list_recipes().await;
    Ok(json!(list
        .into_iter()
        .map(|(recipe, rstate)| json!({
            "recipe": recipe,
            "state": {
                "lastFiredAt": rstate.last_fired_at,
                "executionsThisSession": rstate.executions_this_session,
            }
        }))
        .collect::<Vec<_>>()))
}

#[tauri::command]
async fn tools_list(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(json!(runtime.registry.tool_operations().await))
}

#[tauri::command]
async fn tools_export(state: State<'_, AppState>, format: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let fmt = interaction_tool_schema::ExportFormat::parse(&format)
        .ok_or_else(|| format!("unknown format {format}"))?;
    let manifests = runtime.registry.tool_operations().await;
    Ok(interaction_tool_schema::export(&manifests, fmt))
}

#[tauri::command]
async fn outbox_recent(state: State<'_, AppState>, limit: u32) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(json!(runtime.outbox.recent(limit.min(200) as usize)))
}

#[tauri::command]
async fn audit_tail(state: State<'_, AppState>, limit: u32) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(json!(runtime
        .store
        .audit_tail(limit.min(200))
        .map_err(err_s)?))
}

#[tauri::command]
async fn events_recent(state: State<'_, AppState>, limit: u32) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(json!(runtime.events.recent(limit.min(500) as usize)))
}

// ---------------------------------------------------------------------------
// Write commands (all revalidated by the policy governor)
// ---------------------------------------------------------------------------

#[tauri::command]
async fn set_receptor_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .registry
        .set_receptor_enabled(&ReceptorId::new(&id), enabled)
        .await
        .map_err(err_s)?;
    Ok(json!({"receptorId": id, "enabled": enabled}))
}

#[tauri::command]
async fn set_actuator_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .registry
        .set_actuator_enabled(&ActuatorId::new(&id), enabled)
        .await
        .map_err(err_s)?;
    Ok(json!({"actuatorId": id, "enabled": enabled}))
}

#[tauri::command]
async fn test_receptor(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .observe_fresh(&ReceptorId::new(&id))
            .await
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn test_actuator(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let mut intent = interaction_core::SemanticIntent::new("presence");
    intent.message = Some(format!("[test] actuator {id}"));
    intent.magnitude = Some(0.2);
    intent.duration_ms = Some(500);
    let plan = runtime
        .create_plan(intent, vec![id], 1, 1, false, None, BTreeMap::new())
        .await
        .map_err(err_s)?;
    let receipts = runtime
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .map_err(err_s)?;
    serde_json::to_value(receipts).map_err(err_s)
}

#[tauri::command]
async fn push_observation(
    state: State<'_, AppState>,
    receptor_id: String,
    facts: BTreeMap<String, Value>,
    confidence: f64,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .ingest(&receptor_id, facts, BTreeMap::new(), confidence)
            .await
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn create_plan(state: State<'_, AppState>, input: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let intent_name = input
        .get("intent")
        .and_then(|v| v.as_str())
        .ok_or("missing intent")?
        .to_string();
    let mut intent = interaction_core::SemanticIntent::new(intent_name);
    intent.message = input
        .get("message")
        .and_then(|v| v.as_str())
        .map(String::from);
    intent.magnitude = input.get("magnitude").and_then(|v| v.as_f64());
    intent.duration_ms = input.get("durationMs").and_then(|v| v.as_u64());
    intent.preferred_channels = input
        .get("preferredChannels")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let candidates: Vec<String> = input
        .get("candidates")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let metadata: BTreeMap<String, Value> = input
        .get("metadata")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let plan = runtime
        .create_plan(
            intent,
            candidates,
            input
                .get("minChannels")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            input
                .get("maxChannels")
                .and_then(|v| v.as_u64())
                .unwrap_or(3) as u32,
            input
                .get("allowNoAction")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            None,
            metadata,
        )
        .await
        .map_err(err_s)?;
    serde_json::to_value(plan).map_err(err_s)
}

#[tauri::command]
async fn simulate_plan(state: State<'_, AppState>, plan_id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .simulate_plan(&PlanId::new(&plan_id))
            .await
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn execute_plan(state: State<'_, AppState>, plan_id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .execute_plan(&PlanId::new(&plan_id), ActionSource::ExplicitRequest, false)
            .await
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn cancel_action(state: State<'_, AppState>, action_id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .cancel_action(&ActionId::new(&action_id))
            .await
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn verify_action(state: State<'_, AppState>, action_id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .verify_action(&ActionId::new(&action_id))
            .await
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn policy_patch(state: State<'_, AppState>, patch: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.update_policy(patch).await.map_err(err_s)?).map_err(err_s)
}

#[tauri::command]
async fn session_start(
    state: State<'_, AppState>,
    label: Option<String>,
    consents: Vec<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .start_session(label, None, consents)
            .await
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn consent_grant(
    state: State<'_, AppState>,
    scope: String,
    expires_minutes: Option<u32>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .grant_consent(&scope, expires_minutes)
            .await
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn consent_revoke(state: State<'_, AppState>, scope: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.revoke_consent(&scope).await.map_err(err_s)?).map_err(err_s)
}

#[tauri::command]
async fn session_stop(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.stop_session().await.map_err(err_s)?;
    Ok(json!({"stopped": true}))
}

#[tauri::command]
async fn recipe_upsert(state: State<'_, AppState>, text: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.upsert_recipe_text(&text).await.map_err(err_s)?).map_err(err_s)
}

#[tauri::command]
async fn recipe_validate(text: String) -> Result<Value, String> {
    match interaction_recipe::parse_and_validate(&text) {
        Ok(recipe) => Ok(json!({"valid": true, "recipe": recipe})),
        Err(interaction_recipe::RecipeParseError::Invalid(issues)) => {
            Ok(json!({"valid": false, "issues": issues}))
        }
        Err(e) => Ok(json!({"valid": false, "issues": [{"field": "$", "message": e.to_string()}]})),
    }
}

#[tauri::command]
async fn recipe_set_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(
        runtime
            .set_recipe_enabled(&id, enabled)
            .await
            .map_err(err_s)?,
    )
    .map_err(err_s)
}

#[tauri::command]
async fn recipe_delete(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.remove_recipe(&id).await.map_err(err_s)?;
    Ok(json!({"removed": id}))
}

#[tauri::command]
async fn recipe_simulate(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.simulate_recipe(&id).await.map_err(err_s)
}

#[tauri::command]
async fn recipe_run(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.run_recipe(&id).await.map_err(err_s)
}

// ---------------------------------------------------------------------------
// Emergency stop: dedicated commands, independent of everything else.
// ---------------------------------------------------------------------------

#[tauri::command]
async fn emergency_stop(
    state: State<'_, AppState>,
    reason: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .emergency_stop("desktop", reason)
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn emergency_stop_clear(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .clear_emergency_stop("desktop")
        .await
        .map_err(err_s)?;
    Ok(json!({"cleared": true}))
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

#[tauri::command]
async fn catalog_get() -> Result<Value, String> {
    serde_json::to_value(interaction_registry::catalog::Catalog::builtin()).map_err(err_s)
}

#[tauri::command]
async fn capabilities_human(
    state: State<'_, AppState>,
    locale: Option<String>,
    include_unavailable: Option<bool>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(runtime
        .human_capabilities(
            locale.as_deref().unwrap_or(""),
            include_unavailable.unwrap_or(true),
        )
        .await)
}

#[tauri::command]
async fn ui_prefs_get(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.ui_preferences().await).map_err(err_s)
}

#[tauri::command]
async fn ui_prefs_patch(state: State<'_, AppState>, patch: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let updated = runtime.update_ui_preferences(patch).await.map_err(err_s)?;
    serde_json::to_value(updated).map_err(err_s)
}

#[tauri::command]
async fn onboarding_get(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(runtime.onboarding_state().await)
}

#[tauri::command]
async fn onboarding_draft(state: State<'_, AppState>, draft: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.save_onboarding_draft(draft).await.map_err(err_s)?;
    Ok(serde_json::json!({"saved": true}))
}

#[tauri::command]
async fn onboarding_commit(state: State<'_, AppState>, commit: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let commit: interaction_runtime::human::OnboardingCommit =
        serde_json::from_value(commit).map_err(err_s)?;
    runtime.commit_onboarding(commit).await.map_err(err_s)
}

#[tauri::command]
async fn pause_get(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.pause_status().await).map_err(err_s)
}

#[tauri::command]
async fn pause_set(
    state: State<'_, AppState>,
    duration_minutes: Option<u64>,
    reason: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let until = duration_minutes
        .map(|m| chrono::Utc::now() + chrono::Duration::minutes(m.min(7 * 24 * 60) as i64));
    let st = runtime
        .pause_proactive(until, reason, "desktop")
        .await
        .map_err(err_s)?;
    serde_json::to_value(st).map_err(err_s)
}

#[tauri::command]
async fn pause_clear(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let st = runtime.resume_proactive("desktop").await.map_err(err_s)?;
    serde_json::to_value(st).map_err(err_s)
}

#[tauri::command]
async fn ai_assists_list(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.pending_ai_assists().await).map_err(err_s)
}

/// Desktop IPC = a human clicked; this surface satisfies
/// `ai.requireHumanConfirmation`.
#[tauri::command]
async fn ai_assist_resolve(
    state: State<'_, AppState>,
    request_id: String,
    decision: String,
    note: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .resolve_ai_assist(&request_id, &decision, note, true)
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn plan_get(state: State<'_, AppState>, plan_id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let plan = runtime
        .get_plan(&interaction_core::PlanId::new(plan_id))
        .map_err(err_s)?;
    serde_json::to_value(plan).map_err(err_s)
}

#[tauri::command]
async fn recipe_summary(
    state: State<'_, AppState>,
    id: String,
    locale: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let locale = locale.unwrap_or_else(|| "zh-TW".into());
    let summary = runtime.recipe_summary(&id, &locale).await.map_err(err_s)?;
    Ok(serde_json::json!({"recipeId": id, "summary": summary}))
}

#[tauri::command]
async fn recipe_simulate_scenario(
    state: State<'_, AppState>,
    id: String,
    scenario: Value,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let scenario: interaction_runtime::human::SimScenario =
        serde_json::from_value(scenario).map_err(err_s)?;
    runtime
        .simulate_recipe_scenario(&id, scenario)
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn recipe_convert(text: String, to: String) -> Result<Value, String> {
    let recipe = match interaction_recipe::parse_and_validate(&text) {
        Ok(r) => r,
        Err(interaction_recipe::RecipeParseError::Invalid(issues)) => {
            return Ok(serde_json::json!({"valid": false, "issues": issues}));
        }
        Err(e) => {
            return Ok(serde_json::json!({
                "valid": false,
                "issues": [{"field": "$", "message": e.to_string()}]
            }));
        }
    };
    let out = match to.as_str() {
        "yaml" => interaction_recipe::to_yaml(&recipe).map_err(err_s)?,
        "json" => interaction_recipe::to_json_pretty(&recipe).map_err(err_s)?,
        other => return Err(format!("to must be yaml|json, got {other:?}")),
    };
    Ok(serde_json::json!({"valid": true, "recipe": recipe, "text": out}))
}

#[tauri::command]
async fn recipe_get(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let recipe = runtime.get_recipe(&id).await.map_err(err_s)?;
    serde_json::to_value(recipe).map_err(err_s)
}

// ---------------------------------------------------------------------------
// Supervisor / lifecycle commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn sensor_mic_listen(state: State<'_, AppState>, duration_ms: u64) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let out = runtime
        .begin_mic_listen(duration_ms, "desktop")
        .await
        .map_err(err_s)?;
    Ok(json!(out))
}

#[tauri::command]
async fn sensors_stop(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.stop_all_sensors("desktop").await.map_err(err_s)?;
    Ok(json!({"stopped": true}))
}

#[tauri::command]
async fn presentation_status(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(runtime.presentation_status())
}

#[tauri::command]
async fn presentation_hello(
    state: State<'_, AppState>,
    visible: bool,
    pack_id: Option<String>,
    behavior_state: Option<Value>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(runtime
        .presentation_hello_with_behavior(visible, pack_id, behavior_state)
        .await)
}

#[tauri::command]
async fn presentation_ack(
    state: State<'_, AppState>,
    action_id: String,
    outcome: String,
    detail: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .presentation_ack(&action_id, &outcome, detail)
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn proactive_dialogue_get(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(runtime.proactive_dialogue_status().await)
}

#[tauri::command]
async fn proactive_dialogue_patch(
    state: State<'_, AppState>,
    patch: Value,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .proactive_dialogue_configure(patch)
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn proactive_dialogue_quiet(
    state: State<'_, AppState>,
    minutes: i64,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(runtime.proactive_dialogue_quiet(minutes).await)
}

// ---- v0.4: agents / memory / knowledge / assets（embedded 模式指令） ----

#[tauri::command]
async fn providers_list(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.providers.list().await).map_err(err_s)
}
#[tauri::command]
async fn agents_discoveries(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(json!({"agents": runtime.agent_discoveries()}))
}

#[tauri::command]
async fn agents_refresh(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(json!({"agents": runtime.refresh_agent_providers().await}))
}

#[tauri::command]
async fn agents_routing(state: State<'_, AppState>, kind: Option<String>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    Ok(runtime.agent_route_suggestion(kind.as_deref()).await)
}

#[tauri::command]
async fn agent_session_approve(
    state: State<'_, AppState>,
    id: String,
    request_id: String,
    approve: bool,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .gateway_resolve_approval(&id, &request_id, approve)
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn agent_session_interrupt(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.gateway_interrupt(&id).await.map_err(err_s)
}

#[tauri::command]
async fn memory_list(
    state: State<'_, AppState>,
    layer: Option<String>,
    limit: Option<u32>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .memory_list(layer.as_deref(), limit.unwrap_or(200))
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn memory_create(state: State<'_, AppState>, input: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let actor = match input.get("asAgent").and_then(|v| v.as_str()) {
        Some(a) => interaction_core::MemoryActor::Agent(a.to_string()),
        None => interaction_core::MemoryActor::Human,
    };
    let item =
        interaction_runtime::memory::memory_from_input(input, actor).map_err(|e| e.to_string())?;
    let created = runtime.memory_create(item).await.map_err(err_s)?;
    serde_json::to_value(created).map_err(err_s)
}

#[tauri::command]
async fn memory_patch(
    state: State<'_, AppState>,
    id: String,
    patch: Value,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let item = runtime.memory_update(&id, patch).await.map_err(err_s)?;
    serde_json::to_value(item).map_err(err_s)
}

#[tauri::command]
async fn memory_delete(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let deleted = runtime.memory_delete(&id).await.map_err(err_s)?;
    Ok(json!({"deleted": deleted}))
}

#[tauri::command]
async fn memory_export(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.memory_export().await.map_err(err_s)
}

#[tauri::command]
async fn memory_clear_session(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let n = runtime
        .memory_clear_session_context()
        .await
        .map_err(err_s)?;
    Ok(json!({"cleared": n}))
}

#[tauri::command]
async fn memory_bundle(
    state: State<'_, AppState>,
    task: String,
    agent_id: String,
    domains: Vec<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .memory_context_bundle(&task, &domains, &agent_id)
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn knowledge_list(
    state: State<'_, AppState>,
    status: Option<String>,
    limit: Option<u32>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .knowledge_list(status.as_deref(), limit.unwrap_or(100))
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn domain_packs(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.domain_packs_list().map_err(err_s)
}

#[tauri::command]
async fn domain_pack_install(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.domain_pack_install(&id).map_err(err_s)
}

#[tauri::command]
async fn domain_pack_uninstall(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.domain_pack_uninstall(&id).map_err(err_s)
}

#[tauri::command]
async fn knowledge_search(
    state: State<'_, AppState>,
    q: String,
    k: Option<u32>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .knowledge_search(&q, k.unwrap_or(10))
        .await
        .map_err(err_s)
}

#[tauri::command]
async fn knowledge_get(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let node = runtime.knowledge_get(&id).await.map_err(err_s)?;
    serde_json::to_value(node).map_err(err_s)
}

#[tauri::command]
async fn knowledge_review(
    state: State<'_, AppState>,
    id: String,
    verdict: String,
    note: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    // 桌面視窗＝人類介面（requireHumanConfirmation 的同一信任面）。
    let node = runtime
        .knowledge_review(&id, &verdict, note, interaction_core::MemoryActor::Human)
        .await
        .map_err(err_s)?;
    serde_json::to_value(node).map_err(err_s)
}

#[tauri::command]
async fn knowledge_graph(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.knowledge_graph(&id, 1).await.map_err(err_s)
}

#[tauri::command]
async fn knowledge_receipts(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.knowledge_receipts(100).await.map_err(err_s)
}

#[tauri::command]
async fn knowledge_update_check(
    state: State<'_, AppState>,
    trigger: Value,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    // 更新決策器是純函式（advisory）：只回報「要不要更新／要不要 AI／
    // 是否必須先問使用者」，不執行任何更新——與 HTTP 的
    // /v1/knowledge/update-check 同一 application service。
    let trigger: interaction_runtime::curator::UpdateTrigger =
        serde_json::from_value(trigger).map_err(err_s)?;
    Ok(runtime.knowledge_update_decision(trigger))
}

#[tauri::command]
async fn knowledge_user_correction(
    state: State<'_, AppState>,
    input: Value,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let input: interaction_runtime::curator::UserCorrectionInput =
        serde_json::from_value(input).map_err(err_s)?;
    runtime.record_user_correction(input).await.map_err(err_s)
}

#[tauri::command]
async fn hardware_scan(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.scan_hardware_capabilities().await).map_err(err_s)
}

#[tauri::command]
async fn activity_inbox(state: State<'_, AppState>, filter: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let filter: interaction_runtime::activity::ActivityInboxFilter =
        serde_json::from_value(filter).map_err(err_s)?;
    runtime.activity_inbox(filter).await.map_err(err_s)
}

#[tauri::command]
async fn assets_list(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.asset_list(200).await.map_err(err_s)
}

#[tauri::command]
async fn asset_import(
    state: State<'_, AppState>,
    path: Option<String>,
    content: Option<String>,
    description: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let record = runtime
        .asset_import(
            path.as_deref(),
            content.as_deref(),
            None,
            "user-import",
            description,
        )
        .await
        .map_err(err_s)?;
    serde_json::to_value(record).map_err(err_s)
}

#[tauri::command]
async fn asset_impact(state: State<'_, AppState>, hash: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.asset_delete_impact(&hash).await.map_err(err_s)
}

#[tauri::command]
async fn asset_derivatives(state: State<'_, AppState>, hash: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let derivatives = runtime.asset_derivatives(&hash).await.map_err(err_s)?;
    Ok(serde_json::json!({"derivatives": derivatives}))
}

#[tauri::command]
async fn asset_preview(state: State<'_, AppState>, hash: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.asset_preview(&hash).await.map_err(err_s)
}

#[tauri::command]
async fn asset_derive(state: State<'_, AppState>, hash: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.asset_derive(&hash).await.map_err(err_s)?).map_err(err_s)
}

#[tauri::command]
async fn asset_delete(state: State<'_, AppState>, hash: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.asset_delete(&hash).await.map_err(err_s)
}

#[tauri::command]
async fn agent_sessions_list(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.list_agent_sessions().await).map_err(err_s)
}

#[tauri::command]
async fn agent_session_create(state: State<'_, AppState>, input: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let input: interaction_runtime::agents::CreateAgentSession =
        serde_json::from_value(input).map_err(err_s)?;
    let record = runtime.create_agent_session(input).await.map_err(err_s)?;
    serde_json::to_value(record).map_err(err_s)
}

#[tauri::command]
async fn agent_session_messages(
    state: State<'_, AppState>,
    id: String,
    direction: String,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let dir = match direction.as_str() {
        "to-session" => interaction_core::MailboxDirection::ToSession,
        _ => interaction_core::MailboxDirection::FromSession,
    };
    // 控制中心檢視不改變送達語意：peek，不 fetch。
    let msgs = runtime.mailbox_peek(&id, dir).await.map_err(err_s)?;
    serde_json::to_value(msgs).map_err(err_s)
}
#[tauri::command]
async fn agent_session_send(
    state: State<'_, AppState>,
    id: String,
    kind: String,
    body: Value,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let body_map: std::collections::BTreeMap<String, Value> =
        serde_json::from_value(body).map_err(err_s)?;
    let msg = runtime
        .mailbox_send(
            &id,
            interaction_core::MailboxDirection::ToSession,
            &kind,
            body_map,
            None,
        )
        .await
        .map_err(err_s)?;
    serde_json::to_value(msg).map_err(err_s)
}

#[tauri::command]
async fn agent_session_close(
    state: State<'_, AppState>,
    id: String,
    reason: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let record = runtime
        .close_agent_session(&id, None, reason.as_deref().unwrap_or("closed"))
        .await
        .map_err(err_s)?;
    serde_json::to_value(record).map_err(err_s)
}

/// 人工驗證 claimed-completed（桌面＝human 身分；claim ≠ verified）。
#[tauri::command]
async fn agent_session_verify(
    state: State<'_, AppState>,
    id: String,
    note: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let record = runtime
        .verify_agent_session(&id, note)
        .await
        .map_err(err_s)?;
    serde_json::to_value(record).map_err(err_s)
}

#[tauri::command]
async fn supervisor_info(state: State<'_, AppState>) -> Result<Value, String> {
    let info = state.supervisor.lock().expect("supervisor mutex").clone();
    serde_json::to_value(info).map_err(err_s)
}

#[tauri::command]
async fn desktop_prefs_get(state: State<'_, AppState>) -> Result<Value, String> {
    let prefs = state.prefs.lock().expect("prefs mutex").clone();
    serde_json::to_value(prefs).map_err(err_s)
}

#[tauri::command]
async fn desktop_prefs_patch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    patch: Value,
) -> Result<Value, String> {
    let updated = {
        let mut prefs = state.prefs.lock().expect("prefs mutex");
        let mut v = serde_json::to_value(&*prefs).map_err(err_s)?;
        if let (Some(obj), Some(p)) = (v.as_object_mut(), patch.as_object()) {
            for (k, val) in p {
                obj.insert(k.clone(), val.clone());
            }
        }
        let candidate: DesktopPrefs = serde_json::from_value(v).map_err(err_s)?;
        if !matches!(
            candidate.close_behavior.as_deref(),
            None | Some("keep-running") | Some("hide-companion") | Some("quit")
        ) {
            return Err("invalid closeBehavior".into());
        }
        if !(64.0..=1024.0).contains(&candidate.companion_size.0)
            || !(64.0..=1024.0).contains(&candidate.companion_size.1)
        {
            return Err("companionSize must stay within 64..1024 logical pixels".into());
        }
        if !(0.2..=1.0).contains(&candidate.companion_opacity) {
            return Err("companionOpacity must stay within 0.2..1.0".into());
        }
        if candidate.companion_position.is_some_and(|(x, y)| {
            !(-20_000.0..=20_000.0).contains(&x) || !(-20_000.0..=20_000.0).contains(&y)
        }) {
            return Err("companionPosition is outside the supported desktop bounds".into());
        }
        // v0.5 遊玩偏好：名字/場景/使魔皆為純呈現資料，仍要有界。
        if candidate.companion_name.chars().count() > 24 {
            return Err("companionName must stay within 24 characters".into());
        }
        if !matches!(
            candidate.companion_scene.as_str(),
            "none" | "nest" | "desk" | "sill" | "night"
        ) {
            return Err("companionScene must be one of none/nest/desk/sill/night".into());
        }
        if candidate.companion_familiars.len() > 3 {
            return Err("at most 3 familiars are supported".into());
        }
        for f in &candidate.companion_familiars {
            if f.id.is_empty()
                || f.id.len() > 32
                || !f.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                return Err("familiar id must be 1..32 ascii alphanumeric/dash".into());
            }
            if f.name.chars().count() > 24 {
                return Err("familiar name must stay within 24 characters".into());
            }
            if !matches!(f.palette.as_str(), "maid-classic" | "maid-dusk" | "maid-sakura") {
                return Err("familiar palette must be a bundled palette".into());
            }
        }
        *prefs = candidate;
        prefs.clone()
    };
    supervisor::save_prefs(&updated)?;
    // Keep the OS autostart entry in sync with the pref (default off).
    #[allow(unused_variables)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let autostart = app.autolaunch();
        let _ = if updated.launch_at_login {
            autostart.enable()
        } else {
            autostart.disable()
        };
    }
    serde_json::to_value(updated).map_err(err_s)
}

/// The user's decision from the first-close dialog.
#[tauri::command]
async fn close_decision(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    behavior: String,
    remember: bool,
) -> Result<(), String> {
    if !matches!(
        behavior.as_str(),
        "keep-running" | "hide-companion" | "quit"
    ) {
        return Err(format!("unknown close behavior {behavior:?}"));
    }
    {
        let mut prefs = state.prefs.lock().expect("prefs mutex");
        prefs.close_behavior = Some(behavior.clone());
        if remember {
            prefs.ask_on_close = false;
        }
        supervisor::save_prefs(&prefs)?;
    }
    apply_close_behavior(&app, &behavior);
    Ok(())
}

#[tauri::command]
async fn full_quit(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.quitting.store(true, Ordering::SeqCst);
    {
        let mut info = state.supervisor.lock().expect("supervisor mutex");
        info.state = SupervisorState::Stopping;
    }
    // RunEvent::Exit performs the graceful embedded-runtime shutdown
    // (cancel open receipts, emergency-stop actuators, persist, release lock).
    // An external daemon is deliberately NOT touched.
    app.exit(0);
    Ok(())
}

fn apply_close_behavior(app: &tauri::AppHandle, behavior: &str) {
    match behavior {
        "quit" => {
            let state: State<'_, AppState> = app.state();
            state.quitting.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        "hide-companion" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.hide();
            }
            if let Some(w) = app.get_webview_window("companion") {
                let _ = w.hide();
            }
        }
        "keep-running" => {
            // hide the control center only.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.hide();
            }
        }
        other => {
            // An unrecognized value (hand-edited desktop.json, future
            // regression) must NOT silently keep the runtime alive — that is
            // the "偷偷持續" the spec forbids. Fall back to asking the human.
            tracing::warn!(behavior = other, "unknown close behavior; showing dialog");
            let _ = app.emit("close-requested", ());
        }
    }
}

pub(crate) fn show_main_window(app: &tauri::AppHandle, settings: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        if settings {
            let _ = app.emit("navigate", "settings");
        }
    }
}

pub(crate) fn toggle_companion_window(app: &tauri::AppHandle) {
    let state: State<'_, AppState> = app.state();
    let visible = {
        let mut prefs = state.prefs.lock().expect("prefs mutex");
        prefs.companion_visible = !prefs.companion_visible;
        let _ = supervisor::save_prefs(&prefs);
        prefs.companion_visible
    };
    if visible {
        ensure_companion_window(app);
    } else if let Some(w) = app.get_webview_window("companion") {
        let _ = w.hide();
    }
    let _ = app.emit("companion-visibility", visible);
}

/// 遊玩場視窗尺寸：companion_size 是「角色」大小；視窗加寬給玩具、
/// 散步與使魔（v0.5 遊玩場），加高給氣泡。上限仍受 companion_size
/// 64..1024 的驗證間接約束。
pub(crate) fn companion_window_size(char_size: (f64, f64)) -> (f64, f64) {
    (char_size.0 * 2.6, char_size.1 * 1.35)
}

/// Create (or show) the desktop companion window: transparent, frameless,
/// draggable via the character, never steals focus at creation.
pub(crate) fn ensure_companion_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("companion") {
        let _ = w.show();
        return;
    }
    let state: State<'_, AppState> = app.state();
    let (position, size, on_top) = {
        let prefs = state.prefs.lock().expect("prefs mutex");
        (
            prefs.companion_position,
            prefs.companion_size,
            prefs.companion_always_on_top,
        )
    };
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        "companion",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("小樞")
    .inner_size(companion_window_size(size).0, companion_window_size(size).1)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .skip_taskbar(true)
    .always_on_top(on_top)
    .focused(false);
    if let Some((x, y)) = position {
        builder = builder.position(x, y);
    }
    match builder.build() {
        Ok(_) => {
            spawn_companion_clickthrough(app.clone());
        }
        Err(e) => tracing::error!(error = %e, "companion window create failed"),
    }
}

/// Click-through for the transparent padding around the character: a small
/// Rust poll toggles ignore-cursor-events from the GLOBAL cursor position, so
/// clicks outside the character's hit-rect pass through to whatever is
/// underneath. The WebView cannot change policy here — it only reports where
/// the character is drawn.
fn spawn_companion_clickthrough(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ignoring = false;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(160)).await;
            let Some(win) = app.get_webview_window("companion") else {
                return; // window gone; a new one spawns a new loop
            };
            if !win.is_visible().unwrap_or(false) {
                continue;
            }
            let state: State<'_, AppState> = app.state();
            if state.companion_interactive.load(Ordering::SeqCst) {
                if ignoring {
                    let _ = win.set_ignore_cursor_events(false);
                    ignoring = false;
                }
                continue;
            }
            let (Ok(cursor), Ok(pos), Ok(size)) = (
                app.cursor_position(),
                win.outer_position(),
                win.outer_size(),
            ) else {
                continue;
            };
            let inside_window = cursor.x >= pos.x as f64
                && cursor.x < (pos.x + size.width as i32) as f64
                && cursor.y >= pos.y as f64
                && cursor.y < (pos.y + size.height as i32) as f64;
            if !inside_window {
                // Re-arm interactivity whenever the cursor is away so the next
                // approach over the character is clickable again.
                if ignoring {
                    let _ = win.set_ignore_cursor_events(false);
                    ignoring = false;
                }
                continue;
            }
            // Character hit-rect in physical px (reported by the renderer;
            // defaults cover the sprite's bounding box).
            let scale = win.scale_factor().unwrap_or(1.0);
            let rect = *state.companion_hit_rect.lock().expect("hit rect");
            let rx = pos.x as f64 + rect.0 * scale;
            let ry = pos.y as f64 + rect.1 * scale;
            let rw = rect.2 * scale;
            let rh = rect.3 * scale;
            let on_character =
                cursor.x >= rx && cursor.x < rx + rw && cursor.y >= ry && cursor.y < ry + rh;
            let want_ignore = !on_character;
            if want_ignore != ignoring {
                let _ = win.set_ignore_cursor_events(want_ignore);
                ignoring = want_ignore;
            }
        }
    });
}

/// The renderer reports where the character actually is (logical px within
/// the window) so transparent padding stays click-through.
#[tauri::command]
async fn companion_hit_rect(
    state: State<'_, AppState>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), String> {
    *state.companion_hit_rect.lock().expect("hit rect") = (x, y, w, h);
    Ok(())
}

/// Menus/inputs open → the whole window must accept the cursor.
#[tauri::command]
async fn companion_set_interactive(
    state: State<'_, AppState>,
    interactive: bool,
) -> Result<(), String> {
    state
        .companion_interactive
        .store(interactive, Ordering::SeqCst);
    Ok(())
}

/// Apply companion-related prefs live: visibility, always-on-top, and a
/// reload so pack/persona/expressiveness changes take effect.
#[tauri::command]
async fn companion_apply_prefs(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (visible, position, size, opacity, on_top) = {
        let prefs = state.prefs.lock().expect("prefs mutex");
        (
            prefs.companion_visible,
            prefs.companion_position,
            prefs.companion_size,
            prefs.companion_opacity,
            prefs.companion_always_on_top,
        )
    };
    if visible {
        ensure_companion_window(&app);
        if let Some(w) = app.get_webview_window("companion") {
            let _ = w.set_always_on_top(on_top);
            let win = companion_window_size(size);
            let _ = w.set_size(tauri::LogicalSize::new(win.0, win.1));
            if let Some((x, y)) = position {
                let _ = w.set_position(tauri::LogicalPosition::new(x, y));
            }
        }
        let _ = app.emit("companion-opacity", opacity);
        let _ = app.emit("companion-reload", ());
    } else if let Some(w) = app.get_webview_window("companion") {
        let _ = w.hide();
    }
    Ok(())
}

#[tauri::command]
async fn companion_reset_position(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut prefs = state.prefs.lock().expect("prefs mutex");
        prefs.companion_position = None;
        supervisor::save_prefs(&prefs)?;
    }
    if let Some(window) = app.get_webview_window("companion") {
        window.close().map_err(err_s)?;
    }
    ensure_companion_window(&app);
    Ok(())
}

/// Apply only a governor-authorized `companion.window.adjust` command. The
/// WebView supplies an action id, never the native parameters: those are read
/// back from the Runtime's pending-command registry to prevent authority
/// widening at the JS/native boundary.
#[tauri::command]
async fn companion_window_adjust(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    action_id: String,
) -> Result<Value, String> {
    let backend = state
        .backend()
        .ok_or_else(|| "runtime is not available".to_string())?;
    let pending = backend.presentation_pending_command(&action_id).await?;
    if pending.get("command").and_then(Value::as_str) != Some("window-adjust") {
        return Err("action is not an authorized companion window adjustment".into());
    }
    let params = pending
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| "authorized window adjustment has no parameters".to_string())?;

    let (position, size, opacity, on_top) = {
        let mut prefs = state.prefs.lock().expect("prefs mutex");
        let current_position = prefs.companion_position.unwrap_or((0.0, 0.0));
        let position = (
            params
                .get("x")
                .and_then(Value::as_f64)
                .unwrap_or(current_position.0),
            params
                .get("y")
                .and_then(Value::as_f64)
                .unwrap_or(current_position.1),
        );
        let size = (
            params
                .get("width")
                .and_then(Value::as_f64)
                .unwrap_or(prefs.companion_size.0),
            params
                .get("height")
                .and_then(Value::as_f64)
                .unwrap_or(prefs.companion_size.1),
        );
        let opacity = params
            .get("opacity")
            .and_then(Value::as_f64)
            .unwrap_or(prefs.companion_opacity);
        let on_top = params
            .get("alwaysOnTop")
            .and_then(Value::as_bool)
            .unwrap_or(prefs.companion_always_on_top);
        prefs.companion_position = Some(position);
        prefs.companion_size = size;
        prefs.companion_opacity = opacity;
        prefs.companion_always_on_top = on_top;
        supervisor::save_prefs(&prefs)?;
        (position, size, opacity, on_top)
    };

    let window = app
        .get_webview_window("companion")
        .ok_or_else(|| "companion window is not present".to_string())?;
    window
        .set_position(tauri::LogicalPosition::new(position.0, position.1))
        .map_err(err_s)?;
    window
        .set_size(tauri::LogicalSize::new(size.0, size.1))
        .map_err(err_s)?;
    window.set_always_on_top(on_top).map_err(err_s)?;
    app.emit("companion-opacity", opacity).map_err(err_s)?;
    Ok(json!({
        "actionId": action_id,
        "position": position,
        "size": size,
        "opacity": opacity,
        "alwaysOnTop": on_top,
    }))
}

#[tauri::command]
async fn companion_open_control_center(
    app: tauri::AppHandle,
    tab: Option<String>,
) -> Result<(), String> {
    show_main_window(&app, false);
    if let Some(tab) = tab {
        let _ = app.emit("navigate", tab);
    }
    Ok(())
}

pub(crate) fn full_quit_from_tray(app: &tauri::AppHandle) {
    let state: State<'_, AppState> = app.state();
    state.quitting.store(true, Ordering::SeqCst);
    app.exit(0);
}

/// Refresh tray texts/glyph from the live backend state.
pub(crate) async fn refresh_tray(app: &tauri::AppHandle) {
    let state: State<'_, AppState> = app.state();
    let backend = state.backend();
    let (ready, external) = {
        let info = state.supervisor.lock().expect("supervisor mutex");
        (
            matches!(
                info.state,
                SupervisorState::Ready
                    | SupervisorState::EmbeddedOwned
                    | SupervisorState::ConnectedToExternal
            ),
            info.mode == SupervisorMode::External,
        )
    };
    let (mut estop, mut paused, mut sessions) = (false, false, 0usize);
    let (mut mic_active, mut camera_active) = (false, false);
    let mut reachable = ready;
    if let Some(b) = backend {
        match b.status().await {
            Ok(s) => {
                estop = s
                    .get("emergencyStop")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                paused = s
                    .pointer("/proactivePause/paused")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                sessions = s.get("agentSessions").and_then(Value::as_u64).unwrap_or(0) as usize;
                // Sensor privacy indicator: reflect real capture in the tray
                // glyph (the "no silent capture" contract). status.activeSensors
                // is present in both embedded and external modes.
                if let Some(list) = s.get("activeSensors").and_then(Value::as_array) {
                    mic_active = list
                        .iter()
                        .any(|x| x.get("kind").and_then(Value::as_str) == Some("microphone"));
                    camera_active = list
                        .iter()
                        .any(|x| x.get("kind").and_then(Value::as_str) == Some("camera"));
                }
            }
            Err(_) => reachable = false,
        }
    } else {
        reachable = false;
    }
    let view = tray::tray_view(
        reachable,
        external,
        estop,
        paused,
        sessions,
        mic_active,
        camera_active,
    );
    let companion_visible = state.prefs.lock().expect("prefs mutex").companion_visible;
    {
        let guard = state.tray.lock().expect("tray mutex");
        if let Some(handles) = guard.as_ref() {
            let _ = handles.info_status.set_text(&view.status_text);
            let _ = handles.info_pause.set_text(&view.pause_text);
            let _ = handles.info_sessions.set_text(&view.sessions_text);
            let _ = handles.toggle_pause.set_text(&view.pause_action_text);
            let _ = handles.toggle_companion.set_text(if companion_visible {
                "隱藏桌面角色"
            } else {
                "顯示桌面角色"
            });
            #[cfg(target_os = "macos")]
            {
                let _ = handles.tray.set_title(view.title_glyph);
            }
        }
    };
}

/// Decide embedded vs external and bring the backend up (spec §6).
async fn start_supervised(handle: tauri::AppHandle) {
    let api_base = supervisor::configured_api_base();

    // 1) An external daemon already owns the runtime? Connect, don't compete.
    if supervisor::daemon_ready(&api_base).await {
        let token = supervisor::read_api_token();
        {
            let state: State<'_, AppState> = handle.state();
            let mut info = state.supervisor.lock().expect("supervisor mutex");
            info.mode = SupervisorMode::External;
            info.state = SupervisorState::ConnectedToExternal;
            info.api_base = api_base.clone();
            info.token = token.clone();
            info.detail = Some("connected to external interact-ai daemon".into());
        }
        let _ = handle.emit("supervisor-state", "connected-to-external");
        if token.is_none() {
            let state: State<'_, AppState> = handle.state();
            let mut info = state.supervisor.lock().expect("supervisor mutex");
            info.state = SupervisorState::Degraded;
            info.detail = Some("daemon is running but its API token is unreadable".into());
            let _ = handle.emit(
                "runtime-error",
                "偵測到外部 Runtime，但無法讀取它的 API token（state/api-token）。",
            );
            return;
        }
        // Health loop: the app must never look healthy while the daemon is
        // gone. Demote to Disconnected on failures; recover automatically.
        let health_handle = handle.clone();
        let base = api_base.clone();
        tauri::async_runtime::spawn(async move {
            let mut healthy = true;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let ok = supervisor::daemon_ready(&base).await;
                if ok != healthy {
                    healthy = ok;
                    let state: State<'_, AppState> = health_handle.state();
                    {
                        let mut info = state.supervisor.lock().expect("supervisor mutex");
                        info.state = if ok {
                            SupervisorState::ConnectedToExternal
                        } else {
                            SupervisorState::Disconnected
                        };
                    }
                    let _ = health_handle.emit(
                        "supervisor-state",
                        if ok {
                            "connected-to-external"
                        } else {
                            "disconnected"
                        },
                    );
                    refresh_tray(&health_handle).await;
                }
            }
        });
        return;
    }

    // 2) No daemon: this app owns an embedded runtime.
    match Runtime::start(RuntimeOptions {
        home: None,
        acquire_lock: true,
        in_memory_db: false,
        spawn_watchdog: true,
    })
    .await
    {
        Ok(runtime) => {
            // Forward runtime events to the WebView.
            let mut rx = runtime.events.subscribe();
            let event_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = rx.recv().await {
                    let _ = event_handle.emit("runtime-event", &event);
                }
            });
            // Serve the HTTP API so CLI/agents share this instance. A bind
            // failure must NOT be swallowed: if something else already holds
            // the port, the app must say so (Degraded) instead of reporting a
            // healthy EmbeddedOwned while its HTTP surface is absent — that is
            // the state the CLI would misdirect its token to.
            let config = runtime.config.read().await.clone();
            let mut api_error: Option<String> = None;
            match runtime.config_service.load_or_create_token() {
                Ok(token) => {
                    if let Err(e) = interaction_api::serve(
                        runtime.clone(),
                        &config.api_host,
                        config.api_port,
                        token,
                    )
                    .await
                    {
                        api_error = Some(format!(
                            "HTTP API could not bind {}:{} ({e}). Another process may own \
                             the port; CLI/agent access is unavailable.",
                            config.api_host, config.api_port
                        ));
                    }
                }
                Err(e) => api_error = Some(format!("API token unavailable: {e}")),
            }
            let state: State<'_, AppState> = handle.state();
            // If the user asked to fully quit while the runtime was still
            // starting, RunEvent::Exit already fired with no runtime to stop.
            // Shut this one down cleanly now instead of leaking it.
            if state.quitting.load(Ordering::SeqCst) {
                runtime.shutdown().await;
                return;
            }
            *state.runtime.lock().expect("runtime mutex") = Some(runtime);
            {
                let mut info = state.supervisor.lock().expect("supervisor mutex");
                info.mode = SupervisorMode::Embedded;
                info.state = if api_error.is_some() {
                    SupervisorState::Degraded
                } else {
                    SupervisorState::EmbeddedOwned
                };
                info.api_base = api_base;
                info.detail = api_error.clone();
            }
            if let Some(err) = api_error {
                tracing::error!(error = %err, "embedded API serve failed");
                let _ = handle.emit("supervisor-state", "degraded");
                let _ = handle.emit("runtime-error", err);
            } else {
                let _ = handle.emit("supervisor-state", "embedded-owned");
            }
            // The embedded UI itself talks over Tauri IPC, so the control
            // center is usable even when the HTTP API is degraded.
            let _ = handle.emit("runtime-ready", true);
            refresh_tray(&handle).await;
        }
        Err(e) => {
            let state: State<'_, AppState> = handle.state();
            *state.startup_error.lock().expect("error mutex") =
                Some(format!("runtime start failed: {e}"));
            {
                let mut info = state.supervisor.lock().expect("supervisor mutex");
                info.state = SupervisorState::Disconnected;
                info.detail = Some(e.to_string());
            }
            let _ = handle.emit("runtime-error", e.to_string());
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second desktop app focuses the first instead of double-owning.
            show_main_window(app, false);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState {
            runtime: Mutex::new(None),
            startup_error: Mutex::new(None),
            supervisor: Mutex::new(SupervisorInfo::starting()),
            prefs: Mutex::new(supervisor::load_prefs()),
            quitting: AtomicBool::new(false),
            tray: Mutex::new(None),
            // Default hit-rect ≈ the sprite's body area at scale 1.1.
            companion_hit_rect: Mutex::new((30.0, 30.0, 120.0, 150.0)),
            companion_interactive: AtomicBool::new(false),
        })
        .setup(|app| {
            // Status-bar presence first: the tray exists even while starting.
            match tray::build(app.handle()) {
                Ok(handles) => {
                    let state: State<'_, AppState> = app.state();
                    *state.tray.lock().expect("tray mutex") = Some(handles);
                }
                Err(e) => tracing::error!(error = %e, "tray init failed"),
            }
            // Desktop companion (Phase 2): create per prefs.
            {
                let state: State<'_, AppState> = app.state();
                let (show, visible) = {
                    let prefs = state.prefs.lock().expect("prefs mutex");
                    (prefs.show_companion_on_start, prefs.companion_visible)
                };
                if show && visible {
                    ensure_companion_window(app.handle());
                }
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                start_supervised(handle).await;
            });
            // Periodic tray refresh (poll: robust in both modes).
            let handle2 = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    refresh_tray(&handle2).await;
                    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Close-to-hide (spec §5): closing the control center hides it;
            // the runtime and tray keep running. Only "完全結束" shuts down.
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state: State<'_, AppState> = window.state();
                if state.quitting.load(Ordering::SeqCst) {
                    return; // real quit path: let it close
                }
                api.prevent_close();
                let (behavior, ask) = {
                    let prefs = state.prefs.lock().expect("prefs mutex");
                    (prefs.close_behavior.clone(), prefs.ask_on_close)
                };
                let app = window.app_handle().clone();
                match behavior {
                    Some(b) if !ask => apply_close_behavior(&app, &b),
                    _ => {
                        // First close (or "keep asking"): the WebView shows the
                        // explanation dialog; the decision returns via the
                        // close_decision command.
                        let _ = app.emit("close-requested", ());
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            status,
            capabilities,
            observations_query,
            actions_list,
            action_get,
            policy_get,
            session_get,
            recipes_list,
            tools_list,
            tools_export,
            outbox_recent,
            audit_tail,
            events_recent,
            set_receptor_enabled,
            set_actuator_enabled,
            test_receptor,
            test_actuator,
            push_observation,
            create_plan,
            simulate_plan,
            execute_plan,
            cancel_action,
            verify_action,
            policy_patch,
            session_start,
            consent_grant,
            consent_revoke,
            session_stop,
            recipe_upsert,
            recipe_validate,
            recipe_set_enabled,
            recipe_delete,
            recipe_simulate,
            recipe_run,
            emergency_stop,
            emergency_stop_clear,
            catalog_get,
            capabilities_human,
            ui_prefs_get,
            ui_prefs_patch,
            onboarding_get,
            onboarding_draft,
            onboarding_commit,
            pause_get,
            pause_set,
            pause_clear,
            ai_assists_list,
            ai_assist_resolve,
            plan_get,
            recipe_summary,
            recipe_simulate_scenario,
            recipe_convert,
            recipe_get,
            supervisor_info,
            desktop_prefs_get,
            desktop_prefs_patch,
            close_decision,
            full_quit,
            companion_hit_rect,
            companion_set_interactive,
            companion_open_control_center,
            companion_apply_prefs,
            companion_window_adjust,
            companion_reset_position,
            agent_sessions_list,
            agent_session_send,
            agent_session_close,
            agent_session_verify,
            sensor_mic_listen,
            presentation_status,
            presentation_hello,
            presentation_ack,
            proactive_dialogue_get,
            proactive_dialogue_patch,
            proactive_dialogue_quiet,
            providers_list,
            agent_session_create,
            agent_session_messages,
            agents_discoveries,
            agents_refresh,
            agents_routing,
            agent_session_approve,
            agent_session_interrupt,
            memory_list,
            memory_create,
            memory_patch,
            memory_delete,
            memory_export,
            memory_clear_session,
            memory_bundle,
            knowledge_list,
            domain_packs,
            domain_pack_install,
            domain_pack_uninstall,
            knowledge_search,
            knowledge_get,
            knowledge_review,
            knowledge_graph,
            knowledge_receipts,
            knowledge_update_check,
            knowledge_user_correction,
            hardware_scan,
            activity_inbox,
            assets_list,
            asset_import,
            asset_derivatives,
            asset_preview,
            asset_derive,
            asset_impact,
            asset_delete,
            sensors_stop,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                // App-level quit (Cmd+Q, tray 完全結束, AppleScript) — the
                // embedded runtime must shut down cleanly here. An external
                // daemon is deliberately left running.
                tauri::RunEvent::Exit => {
                    let state: State<'_, AppState> = app_handle.state();
                    let runtime = state.runtime.lock().expect("runtime mutex").clone();
                    if let Some(runtime) = runtime {
                        tauri::async_runtime::block_on(runtime.shutdown());
                    }
                }
                // macOS dock icon click while the window is hidden.
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    show_main_window(app_handle, false);
                }
                _ => {}
            }
        });
}
