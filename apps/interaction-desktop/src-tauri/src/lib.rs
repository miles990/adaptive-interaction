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

use interaction_core::{ActionId, ActuatorId, DiscoveryContext, ObservationQuery, PlanId, ReceptorId};
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

/// Runtime handle or the reason it could not start (e.g. daemon holds the lock).
pub struct AppState {
    runtime: Mutex<Option<Runtime>>,
    startup_error: Mutex<Option<String>>,
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
        .capabilities(&DiscoveryContext { include_unavailable, ..Default::default() })
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
    serde_json::to_value(runtime.get_action(&ActionId::new(&action_id)).map_err(err_s)?)
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
    Ok(json!(runtime.store.audit_tail(limit.min(200)).map_err(err_s)?))
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
    serde_json::to_value(runtime.observe_fresh(&ReceptorId::new(&id)).await.map_err(err_s)?)
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
    intent.message = input.get("message").and_then(|v| v.as_str()).map(String::from);
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
            input.get("minChannels").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            input.get("maxChannels").and_then(|v| v.as_u64()).unwrap_or(3) as u32,
            input.get("allowNoAction").and_then(|v| v.as_bool()).unwrap_or(true),
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
    serde_json::to_value(runtime.simulate_plan(&PlanId::new(&plan_id)).await.map_err(err_s)?)
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
    serde_json::to_value(runtime.cancel_action(&ActionId::new(&action_id)).await.map_err(err_s)?)
        .map_err(err_s)
}

#[tauri::command]
async fn verify_action(state: State<'_, AppState>, action_id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.verify_action(&ActionId::new(&action_id)).await.map_err(err_s)?)
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
    serde_json::to_value(runtime.start_session(label, None, consents).await.map_err(err_s)?)
        .map_err(err_s)
}

#[tauri::command]
async fn consent_grant(
    state: State<'_, AppState>,
    scope: String,
    expires_minutes: Option<u32>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    serde_json::to_value(runtime.grant_consent(&scope, expires_minutes).await.map_err(err_s)?)
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
    serde_json::to_value(runtime.set_recipe_enabled(&id, enabled).await.map_err(err_s)?)
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
async fn emergency_stop(state: State<'_, AppState>, reason: Option<String>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.emergency_stop("desktop", reason).await.map_err(err_s)
}

#[tauri::command]
async fn emergency_stop_clear(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.clear_emergency_stop("desktop").await.map_err(err_s)?;
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


pub fn run() {
    tauri::Builder::default()
        .manage(AppState { runtime: Mutex::new(None), startup_error: Mutex::new(None) })
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
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
                        // Also serve the HTTP API so CLI/agents share this instance.
                        let config = runtime.config.read().await.clone();
                        if let Ok(token) = runtime.config_service.load_or_create_token() {
                            let _ = interaction_api::serve(
                                runtime.clone(),
                                &config.api_host,
                                config.api_port,
                                token,
                            )
                            .await;
                        }
                        let state: State<'_, AppState> = handle.state();
                        *state.runtime.lock().expect("runtime mutex") = Some(runtime);
                        let _ = handle.emit("runtime-ready", true);
                    }
                    Err(e) => {
                        let state: State<'_, AppState> = handle.state();
                        *state.startup_error.lock().expect("error mutex") =
                            Some(format!("runtime start failed: {e}"));
                        let _ = handle.emit("runtime-error", e.to_string());
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // desktop-managed lifecycle: closing the window shuts the runtime
            // down cleanly (no hidden continuation of physical output).
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let state: State<'_, AppState> = window.state();
                let runtime = state.runtime.lock().expect("runtime mutex").clone();
                if let Some(runtime) = runtime {
                    tauri::async_runtime::block_on(runtime.shutdown());
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
            recipe_summary,
            recipe_simulate_scenario,
            recipe_convert,
            recipe_get,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // App-level quit (Cmd+Q, menu, AppleScript) bypasses window close
            // events — the runtime must still shut down cleanly here.
            if let tauri::RunEvent::Exit = event {
                let state: State<'_, AppState> = app_handle.state();
                let runtime = state.runtime.lock().expect("runtime mutex").clone();
                if let Some(runtime) = runtime {
                    tauri::async_runtime::block_on(runtime.shutdown());
                }
            }
        });
}
