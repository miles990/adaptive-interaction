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

mod character_bridge;
mod character_store;
mod host_safety;
mod supervisor;
mod tray;

use host_safety::{HostSafetyView, HOST_SAFETY_EVENT};
use interaction_core::{
    ActionId, ActuatorId, DiscoveryContext, EventType, ObservationQuery, PlanId, ReceptorId,
};
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use supervisor::{DesktopPrefs, SupervisorInfo, SupervisorMode, SupervisorState};
use tauri::{Emitter, EventTarget, Manager, State};

/// Desktop app state: embedded runtime handle OR external-daemon connection,
/// plus desktop-local prefs (close behavior, companion) and the tray.
pub struct AppState {
    runtime: Mutex<Option<Runtime>>,
    startup_error: Mutex<Option<String>>,
    supervisor: Mutex<SupervisorInfo>,
    prefs: Mutex<DesktopPrefs>,
    quitting: AtomicBool,
    tray: Mutex<Option<tray::TrayHandles>>,
    /// Bounded hit REGIONS (logical px) inside the companion window: the
    /// character, each familiar, each grabbable toy and each genuinely
    /// interactive UI surface get their own rectangle. The cursor is only
    /// intercepted where one of them actually is — the blank space between
    /// them belongs to the desktop (companion-gameplay-032).
    companion_hit_regions: Mutex<Vec<HitRect>>,
    /// When the last hit-region report was ACCEPTED. Host-side rate limit so a
    /// buggy or hostile WebView cannot flood the IPC (`MIN_HIT_REGION_INTERVAL_MS`).
    companion_hit_regions_at: Mutex<Option<std::time::Instant>>,
    /// Last applied ignore-cursor-events state, shared by the poll and the
    /// hit-rect report path so a fresh box is applied at once (perf-claims-018).
    companion_clickthrough: Mutex<ClickThroughGate>,
    companion_interactive: AtomicBool,
    /// 最近一次推導的 host 安全視圖（overlay 掛好 listener 時重送用）。
    host_safety: Mutex<Option<HostSafetyView>>,
    /// 事件驅動的 tray／overlay 刷新：進行中旗標＋「結束後再跑一次」旗標（合併尖峰，不堆 task）。
    host_refresh_inflight: AtomicBool,
    host_refresh_again: AtomicBool,
    /// app 啟動時間：啟動寬限（`host_safety::STARTING_GRACE_SECS`）以此計算。
    launched_at: std::time::Instant,
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

    /// 停止所有感測（本機麥克風＋每一台已連線 iPhone，有界等待確認）。
    /// tray 也走這條——不經 WebView，UI 掛掉也停得了。
    pub async fn sensors_stop(&self, actor: &str) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => rt
                .stop_all_sensors(actor)
                .await
                .map_err(|e| e.to_string())
                .and_then(|report| serde_json::to_value(report).map_err(|e| e.to_string())),
            Backend::External { base, token } => {
                supervisor::daemon_post(base, token, "/v1/sensors/stop", json!({})).await
            }
        }
    }

    pub async fn mobile_sensors_stop(&self, device_id: &str) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => rt
                .mobile_sensors_stop(device_id)
                .await
                .map_err(|e| e.to_string()),
            Backend::External { base, token } => {
                supervisor::daemon_post(
                    base,
                    token,
                    &format!("/v1/mobile/devices/{device_id}/sensors/stop"),
                    json!({}),
                )
                .await
            }
        }
    }

    pub async fn mobile_test(&self, device_id: &str) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => rt.mobile_test(device_id).await.map_err(|e| e.to_string()),
            Backend::External { base, token } => {
                supervisor::daemon_post(
                    base,
                    token,
                    &format!("/v1/mobile/devices/{device_id}/test"),
                    json!({}),
                )
                .await
            }
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

    // ---- Character Presentation Protocol：兩種模式共用同一組 HTTP 契約 ----
    // Embedded 走 Runtime 方法（character_bridge），External 打 /v1/character/*。

    pub async fn character_hello(&self, body: Value) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => character_bridge::hello(rt, body).await,
            Backend::External { base, token } => {
                supervisor::daemon_post(base, token, "/v1/character/hello", body).await
            }
        }
    }

    pub async fn character_receipt(
        &self,
        instance_id: &str,
        receipt: Value,
    ) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => character_bridge::receipt(rt, instance_id, receipt).await,
            Backend::External { base, token } => {
                supervisor::daemon_post(
                    base,
                    token,
                    "/v1/character/receipts",
                    json!({"instanceId": instance_id, "receipt": receipt}),
                )
                .await
            }
        }
    }

    pub async fn character_event(&self, instance_id: &str, event: Value) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => character_bridge::event(rt, instance_id, event).await,
            Backend::External { base, token } => {
                supervisor::daemon_post(
                    base,
                    token,
                    "/v1/character/events",
                    json!({"instanceId": instance_id, "event": event}),
                )
                .await
            }
        }
    }

    pub async fn character_instances(&self) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => character_bridge::instances(rt).await,
            Backend::External { base, token } => {
                supervisor::daemon_get(base, token, "/v1/character/instances").await
            }
        }
    }

    pub async fn character_manifest(&self) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => character_bridge::manifest(rt).await,
            Backend::External { base, token } => {
                supervisor::daemon_get(base, token, "/v1/character/manifest").await
            }
        }
    }

    /// 外部 character adapter 清單（不含 token）。
    pub async fn character_adapters(&self) -> Result<Value, String> {
        match self {
            Backend::Embedded(rt) => character_bridge::adapters(rt).await,
            Backend::External { base, token } => {
                supervisor::daemon_get(base, token, "/v1/character/adapters").await
            }
        }
    }

    /// 撤銷外部 character adapter：token 失效並立即斷線。
    pub async fn character_adapter_revoke(&self, adapter_id: &str) -> Result<Value, String> {
        if adapter_id.is_empty()
            || adapter_id.len() > 128
            || !adapter_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err("invalid adapter id".into());
        }
        match self {
            Backend::Embedded(rt) => character_bridge::adapter_revoke(rt, adapter_id).await,
            Backend::External { base, token } => {
                supervisor::daemon_delete(
                    base,
                    token,
                    &format!("/v1/character/adapters/{adapter_id}"),
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

/// 套用前的試算：與 commit 同一套驗證，但不改任何東西。
#[tauri::command]
async fn onboarding_preview(state: State<'_, AppState>, commit: Value) -> Result<Value, String> {
    let runtime = rt(&state)?;
    let commit: interaction_runtime::human::OnboardingCommit =
        serde_json::from_value(commit).map_err(err_s)?;
    runtime.preview_onboarding(commit).await.map_err(err_s)
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

/// 停止所有感測。回傳誠實報告：頂層 `stopped` ＝所有來源都確認停止，
/// `uncertain` ＝有來源沒回覆（手機可能還在錄音），`devices[]` 逐台列出結果。
#[tauri::command]
async fn sensors_stop(state: State<'_, AppState>) -> Result<Value, String> {
    let backend = state.backend().ok_or_else(|| {
        state
            .startup_error
            .lock()
            .expect("error mutex")
            .clone()
            .unwrap_or_else(|| "runtime not started".into())
    })?;
    backend.sensors_stop("desktop").await
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
    // list_providers()（不是 registry.list()）才會附上「已測試」證據。
    serde_json::to_value(runtime.list_providers().await).map_err(err_s)
}

/// 「測試裝置」：唯讀測一次（不觸發動器）。桌面是人類介面，指令只在此暴露。
#[tauri::command]
async fn provider_test(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .test_provider(&interaction_core::ProviderId::new(&id))
        .await
        .map_err(err_s)
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

// v0.5 Phase 6：iPhone Mobile Provider（human-only 桌面指令）。
#[tauri::command]
async fn mobile_status(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.mobile_status().await.map_err(err_s)
}

#[tauri::command]
async fn mobile_pairing_begin(state: State<'_, AppState>) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.mobile_pairing_begin().await.map_err(err_s)
}

#[tauri::command]
async fn mobile_revoke(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime.mobile_revoke(&id).await.map_err(err_s)
}

/// BLE 閘道：請已連線 iPhone 代掃描周邊。手機沒回＝結果未知（誠實 Err），
/// 不假裝掃到 0 台。
#[tauri::command]
async fn mobile_ble_scan(
    state: State<'_, AppState>,
    duration_ms: Option<u64>,
    device_id: Option<String>,
) -> Result<Value, String> {
    let runtime = rt(&state)?;
    runtime
        .mobile_ble_scan(duration_ms.unwrap_or(4_000), device_id.as_deref())
        .await
        .map_err(err_s)
}

/// 只停這一台手機的感測（有界等待確認）。手機沒回覆＝`outcome: "unknown"`，
/// 不得顯示成已停止；沒連線＝`unreachable`（沒有任何東西被停）。
#[tauri::command]
async fn mobile_sensors_stop(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let backend = state
        .backend()
        .ok_or_else(|| "runtime not available".to_string())?;
    backend.mobile_sensors_stop(&id).await
}

/// 測試這台手機的連線（Ping／Pong）。`ok` 只代表連線有回應，
/// 不代表 App 的感測／動器功能正常。
#[tauri::command]
async fn mobile_test(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let backend = state
        .backend()
        .ok_or_else(|| "runtime not available".to_string())?;
    backend.mobile_test(&id).await
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

// ---------------------------------------------------------------------------
// Character Presentation Protocol（桌面視窗＝可信 host）：兩種模式都可用。
// 參數與回傳形狀與 /v1/character/* 完全相同（camelCase）。
// ---------------------------------------------------------------------------

fn backend_or_err(state: &State<'_, AppState>) -> Result<Backend, String> {
    state
        .backend()
        .ok_or_else(|| "runtime is not available".to_string())
}

/// `POST /v1/character/hello` 的 body：省略的選填欄位不送（讓 Runtime 套預設）。
fn character_hello_body(
    instance_id: Option<String>,
    role: Option<String>,
    manifest: Value,
    negotiate: Value,
    visible: bool,
    pack_id: Option<String>,
    behavior_state: Option<Value>,
) -> Value {
    let mut body = serde_json::Map::new();
    if let Some(v) = instance_id {
        body.insert("instanceId".into(), Value::String(v));
    }
    if let Some(v) = role {
        body.insert("role".into(), Value::String(v));
    }
    body.insert("manifest".into(), manifest);
    body.insert("negotiate".into(), negotiate);
    body.insert("visible".into(), Value::Bool(visible));
    if let Some(v) = pack_id {
        body.insert("packId".into(), Value::String(v));
    }
    if let Some(v) = behavior_state {
        body.insert("behaviorState".into(), v);
    }
    Value::Object(body)
}

/// 桌面視窗的 CPP 握手：manifest 摘要＋negotiate → negotiated（generation 由 Runtime 決定）。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn character_hello(
    state: State<'_, AppState>,
    instance_id: Option<String>,
    role: Option<String>,
    manifest: Value,
    negotiate: Value,
    visible: bool,
    pack_id: Option<String>,
    behavior_state: Option<Value>,
) -> Result<Value, String> {
    let backend = backend_or_err(&state)?;
    backend
        .character_hello(character_hello_body(
            instance_id,
            role,
            manifest,
            negotiate,
            visible,
            pack_id,
            behavior_state,
        ))
        .await
}

/// 呈現回執（accepted≠started≠completed；completed 只代表演完，verification 永遠 acknowledged-only）。
#[tauri::command]
async fn character_receipt(
    state: State<'_, AppState>,
    instance_id: String,
    receipt: Value,
) -> Result<Value, String> {
    let backend = backend_or_err(&state)?;
    backend.character_receipt(&instance_id, receipt).await
}

/// 受限的角色輸入事件 → Runtime 正規化後成為 receptor observation（仍經 policy／consent）。
#[tauri::command]
async fn character_event(
    state: State<'_, AppState>,
    instance_id: String,
    event: Value,
) -> Result<Value, String> {
    let backend = backend_or_err(&state)?;
    backend.character_event(&instance_id, event).await
}

#[tauri::command]
async fn character_instances(state: State<'_, AppState>) -> Result<Value, String> {
    let backend = backend_or_err(&state)?;
    backend.character_instances().await
}

/// 目前桌面角色的 manifest；尚未 hello 時 Err（同 HTTP 404）。
#[tauri::command]
async fn character_manifest(state: State<'_, AppState>) -> Result<Value, String> {
    let backend = backend_or_err(&state)?;
    backend.character_manifest().await
}

/// 外部 character adapter 清單（連接頁「使用的裝置」用）；永不回傳 token。
#[tauri::command]
async fn character_adapters(state: State<'_, AppState>) -> Result<Value, String> {
    let backend = backend_or_err(&state)?;
    backend.character_adapters().await
}

/// 撤銷外部 character adapter（人類操作；WebView 不持有任何 adapter token）。
#[tauri::command]
async fn character_adapter_revoke(
    state: State<'_, AppState>,
    adapter_id: String,
) -> Result<Value, String> {
    let backend = backend_or_err(&state)?;
    backend.character_adapter_revoke(&adapter_id).await
}

// ---- 角色匯入（host 本機檔案；驗證交給 interaction-character） ----

/// `character_import` 的資產參數：`{id, base64}`。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportAssetArg {
    id: String,
    base64: String,
}

fn characters_root() -> std::path::PathBuf {
    character_store::characters_root(&supervisor::interaction_home())
}

/// 匯入一個 in-process 角色（manifest 原文＋資產）。只驗證、只寫檔；不執行、不連線。
/// 回 `{characterId, displayName, report, assets}`。
#[tauri::command]
async fn character_import(
    manifest_text: String,
    assets: Vec<ImportAssetArg>,
) -> Result<Value, String> {
    let root = characters_root();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let mut decoded = Vec::with_capacity(assets.len());
        for asset in assets {
            let bytes = character_store::decode_asset_base64(&asset.base64)?;
            decoded.push(character_store::ImportAssetInput {
                id: asset.id,
                bytes,
            });
        }
        character_store::import(&root, &manifest_text, &decoded)
    })
    .await
    .map_err(err_s)??;
    serde_json::to_value(outcome).map_err(err_s)
}

/// 已匯入角色清單（`valid:false` 的損毀資料夾也誠實列出，可移除）。
#[tauri::command]
async fn character_list_imported() -> Result<Value, String> {
    let root = characters_root();
    let list = tauri::async_runtime::spawn_blocking(move || character_store::list(&root))
        .await
        .map_err(err_s)?;
    serde_json::to_value(list).map_err(err_s)
}

/// 已匯入資產 → data URL（≤ 8 MB；路徑與 magic bytes 讀取時再核對一次）。
#[tauri::command]
async fn character_asset(character_id: String, asset_id: String) -> Result<String, String> {
    let root = characters_root();
    tauri::async_runtime::spawn_blocking(move || {
        character_store::asset_data_url(&root, &character_id, &asset_id)
    })
    .await
    .map_err(err_s)?
}

/// 移除已匯入角色；內建角色一律拒絕。
#[tauri::command]
async fn character_remove(character_id: String) -> Result<Value, String> {
    let root = characters_root();
    let id = character_id.clone();
    tauri::async_runtime::spawn_blocking(move || character_store::remove(&root, &id))
        .await
        .map_err(err_s)??;
    Ok(json!({"removed": character_id}))
}

/// overlay 視窗掛好 listener 後呼叫：host 只把**快取的** HostSafetyView 重送一次。
/// 只接受 label 為 `overlay` 的呼叫者；其他視窗拿不到、也改不了 overlay 的內容
/// （main／companion 的 capability 也沒有 event emit 權限）。
#[tauri::command]
async fn overlay_attach(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if window.label() != OVERLAY_LABEL {
        return Err("only the overlay window may attach".into());
    }
    let view = state.host_safety.lock().expect("host safety mutex").clone();
    if let Some(view) = view {
        app.emit_to(EventTarget::labeled(OVERLAY_LABEL), HOST_SAFETY_EVENT, view)
            .map_err(err_s)?;
    }
    Ok(())
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
            if !matches!(
                f.palette.as_str(),
                "maid-classic" | "maid-dusk" | "maid-sakura"
            ) {
                return Err("familiar palette must be a bundled palette".into());
            }
        }
        // 本機安靜期：epoch ms，必須有限且不得往未來無限延伸（上限一年）。
        if !candidate.companion_proactive_quiet_until.is_finite()
            || candidate.companion_proactive_quiet_until < 0.0
            || candidate.companion_proactive_quiet_until > MAX_QUIET_UNTIL_MS
        {
            return Err("companionProactiveQuietUntil must be a bounded epoch millisecond".into());
        }
        // 角色偏好（manifest.preferencesSchema 的值）：有界、只收純量；
        // 任何不合規的內容整筆拒絕，不靜默丟棄。
        validate_companion_preferences(&candidate.companion_preferences)?;
        // 角色互動記憶：有界（≤8 玩具/反應、≤20 事件），不做任何推論。
        let mut candidate = candidate;
        candidate.companion_interaction_memory = candidate.companion_interaction_memory.bounded();
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

/// 關閉行為會藏哪些視窗。可信 overlay（`OVERLAY_LABEL`）永遠不在名單內：
/// 估停／感測／離線指示不能因為使用者關掉控制中心或藏起角色而消失。
pub(crate) fn windows_hidden_by_close_behavior(behavior: &str) -> &'static [&'static str] {
    match behavior {
        "hide-companion" => &["main", "companion"],
        // hide the control center only.
        "keep-running" => &["main"],
        _ => &[],
    }
}

fn apply_close_behavior(app: &tauri::AppHandle, behavior: &str) {
    match behavior {
        "quit" => {
            let state: State<'_, AppState> = app.state();
            state.quitting.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        "hide-companion" | "keep-running" => {
            for label in windows_hidden_by_close_behavior(behavior) {
                if let Some(w) = app.get_webview_window(label) {
                    let _ = w.hide();
                }
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
    apply_companion_visibility(app, visible);
}

/// Tell the Runtime whether the companion surface is there. Hidden → the
/// Presentation receptors/actuators must go offline at once (spec §6.1), not
/// after the hidden WebView's own heartbeat (which is not guaranteed to be
/// sent) or the 45 s presence timeout. Shared by every host path that shows
/// or hides the character so tray, prefs and runtime never disagree.
pub(crate) async fn announce_companion_presence(
    runtime: Runtime,
    visible: bool,
    pack: String,
) -> Value {
    runtime
        .presentation_hello_with_behavior(visible, Some(pack), None)
        .await
}

/// The ONE place that turns `prefs.companion_visible` into reality: show or
/// hide the companion window, tell the Runtime (see
/// `announce_companion_presence`), tell the WebView, and refresh the tray
/// label. Callers persist the pref first; the tray toggle, the control-center
/// switch (`companion_apply_prefs`) and the runtime's `companion.presence.set`
/// (`companion_set_visible`) all end here, so hiding by any route is the same
/// hide (director-pipeline-021／-024).
pub(crate) fn apply_companion_visibility(app: &tauri::AppHandle, visible: bool) {
    if visible {
        ensure_companion_window(app);
    } else if let Some(w) = app.get_webview_window("companion") {
        let _ = w.hide();
    }
    let state: State<'_, AppState> = app.state();
    let runtime = state.runtime.lock().expect("runtime mutex").clone();
    if let Some(runtime) = runtime {
        let pack = state
            .prefs
            .lock()
            .expect("prefs mutex")
            .companion_pack
            .clone();
        tauri::async_runtime::spawn(announce_companion_presence(runtime, visible, pack));
    }
    let _ = app.emit("companion-visibility", visible);
    // Tray label（顯示／隱藏桌面角色）reads the pref: refresh it now, event-driven.
    request_host_refresh(app);
}

/// Honest window state for the callers that ack a runtime command: `true`
/// only when a companion window exists AND the OS reports it visible.
fn companion_window_is_visible(app: &tauri::AppHandle) -> bool {
    app.get_webview_window("companion")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// 遊玩場視窗尺寸：companion_size 是「角色」大小；視窗加寬給玩具、
/// 散步與使魔（v0.5 遊玩場），加高給氣泡。上限仍受 companion_size
/// 64..1024 的驗證間接約束。
pub(crate) fn companion_window_size(char_size: (f64, f64)) -> (f64, f64) {
    (char_size.0 * 2.6, char_size.1 * 1.35)
}

/// 角色視窗標題＝使用者設定的角色名字（不再寫死任何角色）；空白時用中立文案。
pub(crate) fn companion_window_title(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "桌面角色".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `companion.window.adjust` 套用計畫：位置／角色尺寸／視窗尺寸／不透明度／置頂。
/// 視窗尺寸一律經 `companion_window_size`（與 `ensure_companion_window`／
/// `companion_apply_prefs` 同一個乘數）；否則 AI 調整後的視窗會在下一次
/// apply／reset 時被重新縮放成另一個大小。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WindowAdjustPlan {
    pub position: (f64, f64),
    /// 角色尺寸（寫回 `prefs.companion_size`）。
    pub size: (f64, f64),
    /// 實際套到視窗的尺寸。
    pub window_size: (f64, f64),
    pub opacity: f64,
    pub on_top: bool,
}

pub(crate) fn plan_window_adjust(
    prefs: &DesktopPrefs,
    params: &serde_json::Map<String, Value>,
) -> Result<WindowAdjustPlan, String> {
    let current_position = prefs.companion_position.unwrap_or((0.0, 0.0));
    let num =
        |key: &str, fallback: f64| params.get(key).and_then(Value::as_f64).unwrap_or(fallback);
    let position = (num("x", current_position.0), num("y", current_position.1));
    let size = (
        num("width", prefs.companion_size.0),
        num("height", prefs.companion_size.1),
    );
    let opacity = num("opacity", prefs.companion_opacity);
    let on_top = params
        .get("alwaysOnTop")
        .and_then(Value::as_bool)
        .unwrap_or(prefs.companion_always_on_top);
    // 與 desktop_prefs_patch 相同的邊界（Runtime 也驗過；這裡是 native 的最後一道）。
    if !(64.0..=1024.0).contains(&size.0) || !(64.0..=1024.0).contains(&size.1) {
        return Err("window adjustment size must stay within 64..1024 logical pixels".into());
    }
    if !(0.2..=1.0).contains(&opacity) {
        return Err("window adjustment opacity must stay within 0.2..1.0".into());
    }
    if !(-20_000.0..=20_000.0).contains(&position.0)
        || !(-20_000.0..=20_000.0).contains(&position.1)
    {
        return Err("window adjustment position is outside the supported desktop bounds".into());
    }
    Ok(WindowAdjustPlan {
        position,
        size,
        window_size: companion_window_size(size),
        opacity,
        on_top,
    })
}

/// Create (or show) the desktop companion window: transparent, frameless,
/// draggable via the character, never steals focus at creation.
pub(crate) fn ensure_companion_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("companion") {
        let _ = w.show();
        return;
    }
    let state: State<'_, AppState> = app.state();
    let (position, size, on_top, name) = {
        let prefs = state.prefs.lock().expect("prefs mutex");
        (
            prefs.companion_position,
            prefs.companion_size,
            prefs.companion_always_on_top,
            prefs.companion_name.clone(),
        )
    };
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        "companion",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title(companion_window_title(&name))
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

/// Poll interval for the click-through decision (ms).
///
/// The poll only has to catch the CURSOR moving. The renderer reports the
/// hit regions at most every ~60ms while the character walks or a toy rolls
/// (see `hitRegionsReportPolicy`), and `companion_hit_regions` re-evaluates click-through
/// against the live cursor the moment a report lands, so a character moving
/// under a still cursor no longer waits for the next tick on top of the
/// report throttle. Host-side worst case is therefore ~max(80ms poll, 60ms
/// report) plus IPC/OS dispatch, not the old 80+60ms sum — and it is a bound
/// from reading the code, not an end-to-end measurement (the perf rig only
/// times the in-WebView segment; see docs/acceptance-evidence.md).
pub(crate) const CLICKTHROUGH_POLL_MS: u64 = 80;

/// Upper bound for the local "stay quiet" pref (epoch ms, ~year 2100).
pub(crate) const MAX_QUIET_UNTIL_MS: f64 = 4_102_444_800_000.0;

/// 角色偏好表（`DesktopPrefs.companion_preferences`）的上限：最多幾個角色。
pub(crate) const MAX_COMPANION_PREFERENCE_CHARACTERS: usize = 16;
/// 每個角色最多幾個偏好鍵（與 manifest `preferencesSchema.properties ≤ 32` 一致）。
pub(crate) const MAX_COMPANION_PREFERENCE_KEYS: usize = 32;
/// 字串偏好值的長度上限（與 manifest `preferencesSchema` string `maxLength ≤ 200` 一致）。
pub(crate) const MAX_COMPANION_PREFERENCE_STRING_CHARS: usize = 200;

/// `characterId` 規則（docs/character-protocol/README.md §2）：`^[a-z0-9][a-z0-9._-]{0,63}$`。
pub(crate) fn is_valid_character_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    let rest = chars.as_str();
    rest.len() <= 63
        && rest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// 偏好鍵規則：`^[a-zA-Z0-9_.-]{1,64}$`。
pub(crate) fn is_valid_preference_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// 角色偏好表驗證：≤16 個角色、每角色 ≤32 鍵、值只能是 bool／有限數字／≤200 字字串。
/// 其他型別（null、陣列、物件）一律拒絕——偏好只是純量，不是任意 JSON 倉庫。
/// 錯誤訊息只回顯 id／鍵，不回顯值。
pub(crate) fn validate_companion_preferences(
    map: &BTreeMap<String, BTreeMap<String, Value>>,
) -> Result<(), String> {
    if map.len() > MAX_COMPANION_PREFERENCE_CHARACTERS {
        return Err(format!(
            "companionPreferences may hold at most {MAX_COMPANION_PREFERENCE_CHARACTERS} characters"
        ));
    }
    for (character_id, values) in map {
        if !is_valid_character_id(character_id) {
            return Err(
                "companionPreferences key must be a characterId (^[a-z0-9][a-z0-9._-]{0,63}$)"
                    .into(),
            );
        }
        if values.len() > MAX_COMPANION_PREFERENCE_KEYS {
            return Err(format!(
                "companionPreferences[{character_id}] may hold at most {MAX_COMPANION_PREFERENCE_KEYS} keys"
            ));
        }
        for (key, value) in values {
            if !is_valid_preference_key(key) {
                return Err(format!(
                    "companionPreferences[{character_id}] has an invalid key (must match ^[a-zA-Z0-9_.-]{{1,64}}$)"
                ));
            }
            match value {
                Value::Bool(_) => {}
                Value::Number(n) => {
                    if !n.as_f64().is_some_and(f64::is_finite) {
                        return Err(format!(
                            "companionPreferences[{character_id}].{key} must be a finite number"
                        ));
                    }
                }
                Value::String(s) => {
                    if s.chars().count() > MAX_COMPANION_PREFERENCE_STRING_CHARS {
                        return Err(format!(
                            "companionPreferences[{character_id}].{key} must stay within {MAX_COMPANION_PREFERENCE_STRING_CHARS} characters"
                        ));
                    }
                }
                Value::Null | Value::Array(_) | Value::Object(_) => {
                    return Err(format!(
                        "companionPreferences[{character_id}].{key} must be a boolean, a number, or a string"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// One hit rectangle in logical px, window-relative: `(x, y, w, h)`.
pub(crate) type HitRect = (f64, f64, f64, f64);

/// How many regions the host will keep from one report. The renderer sends the
/// character, its familiars, the grabbable toys and the open UI surfaces; more
/// than this is either a bug or an attempt to fence off the whole window, so
/// the extra ones are dropped (with a warning) instead of trusted.
pub(crate) const MAX_HIT_REGIONS: usize = 16;

/// A single region may not be ≥80% of the window in BOTH axes — that is not a
/// character, that is "make the whole window opaque to the desktop".
pub(crate) const MAX_HIT_REGION_WINDOW_FRACTION: f64 = 0.8;

/// …and the regions together may not cover more than this share of the window,
/// so 16 merely-large boxes cannot add up to the same land grab.
pub(crate) const MAX_HIT_REGION_TOTAL_AREA_FRACTION: f64 = 0.8;

/// Host-side floor between two ACCEPTED reports (ms).
///
/// The renderer already throttles (`HIT_REGION_MIN_INTERVAL_MS` = 50ms in
/// src/companion/hitRegions.ts, with a ≤60ms quiet heartbeat), but the host may
/// not depend on the WebView behaving. 45ms sits under the renderer's own 50ms
/// floor, so honest reports are never dropped while a runaway caller is still
/// bounded to ~22 reports/s.
pub(crate) const MIN_HIT_REGION_INTERVAL_MS: u64 = 45;

/// Should this report be accepted, or is it arriving too fast?
///
/// Pure so the rate limit is testable: `elapsed_ms` is `None` for the very
/// first report (always accepted).
pub(crate) fn hit_regions_accept(elapsed_ms: Option<u64>, min_interval_ms: u64) -> bool {
    match elapsed_ms {
        None => true,
        Some(ms) => ms >= min_interval_ms,
    }
}

/// Validate a renderer-reported region list against the window (logical px).
///
/// Fail-closed: any bad region rejects the WHOLE report, and the caller keeps
/// the previous (known-good) regions rather than falling back to "everything"
/// or "nothing". Rejects: an empty list, non-finite / non-positive boxes,
/// boxes entirely outside the window, a box that is ≥80% of the window in both
/// axes, and a list whose total area is >80% of the window. Extra regions past
/// `MAX_HIT_REGIONS` are dropped with a warning (the first ones win: the
/// renderer puts the character and the open UI first).
pub(crate) fn sanitize_hit_regions(
    regions: &[HitRect],
    win_w: f64,
    win_h: f64,
) -> Result<Vec<HitRect>, String> {
    if regions.is_empty() {
        return Err("hit regions must not be empty".into());
    }
    if !win_w.is_finite() || !win_h.is_finite() || win_w <= 0.0 || win_h <= 0.0 {
        return Err("window size must be positive".into());
    }
    if regions.len() > MAX_HIT_REGIONS {
        tracing::warn!(
            reported = regions.len(),
            kept = MAX_HIT_REGIONS,
            "companion reported more hit regions than allowed; extra regions dropped"
        );
    }
    let mut out = Vec::with_capacity(regions.len().min(MAX_HIT_REGIONS));
    let mut area = 0.0_f64;
    for &(x, y, w, h) in regions.iter().take(MAX_HIT_REGIONS) {
        let (cx, cy, cw, ch) = clamp_hit_rect(x, y, w, h, win_w, win_h)?;
        if cw >= win_w * MAX_HIT_REGION_WINDOW_FRACTION
            && ch >= win_h * MAX_HIT_REGION_WINDOW_FRACTION
        {
            return Err("a hit region may not cover the whole companion window".into());
        }
        area += cw * ch;
        out.push((cx, cy, cw, ch));
    }
    if area > win_w * win_h * MAX_HIT_REGION_TOTAL_AREA_FRACTION {
        return Err("hit regions may not cover the whole companion window".into());
    }
    Ok(out)
}

/// Everything the click-through decision looks at, in physical px except the
/// hit regions (logical, window-relative — exactly as the renderer reports them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClickThroughProbe<'a> {
    /// Global cursor position.
    pub cursor: (f64, f64),
    /// Companion window outer position.
    pub window_pos: (f64, f64),
    /// Companion window outer size.
    pub window_size: (f64, f64),
    /// Window scale factor (logical → physical).
    pub scale: f64,
    /// Character/familiar/toy/UI hit regions (logical px inside the window).
    /// Already sanitized by `sanitize_hit_regions`.
    pub hit_regions: &'a [HitRect],
    /// A focused text input / drop confirmation → the whole window accepts the
    /// cursor. Passive surfaces (speech bubble, safety labels, quick menu) do
    /// NOT set this: they report their own region instead, so the transparent
    /// space around them still belongs to the desktop.
    pub interactive: bool,
}

/// Pure click-through decision: should the window ignore cursor events?
///
/// `true` only when the cursor is inside the window but on transparent
/// padding — i.e. inside NO reported region. An interactive window and a
/// cursor outside the window both answer `false` — the latter re-arms
/// interactivity so the next approach over the character is clickable again.
pub(crate) fn clickthrough_want_ignore(p: &ClickThroughProbe<'_>) -> bool {
    if p.interactive {
        return false;
    }
    let (cx, cy) = p.cursor;
    let (wx, wy) = p.window_pos;
    let (ww, wh) = p.window_size;
    let inside_window = cx >= wx && cx < wx + ww && cy >= wy && cy < wy + wh;
    if !inside_window {
        return false;
    }
    // Intercept only where something is actually drawn: the union's blank
    // space (character on the left, a toy thrown far right) is desktop.
    let on_object = p.hit_regions.iter().any(|&(rx, ry, rw, rh)| {
        let rx = wx + rx * p.scale;
        let ry = wy + ry * p.scale;
        let rw = rw * p.scale;
        let rh = rh * p.scale;
        cx >= rx && cx < rx + rw && cy >= ry && cy < ry + rh
    });
    !on_object
}

/// Last applied ignore-cursor-events state. `decide` returns `Some(next)`
/// only when the window actually has to be toggled, so both callers (the
/// poll and the hit-rect report) stay idempotent and never double-toggle.
#[derive(Debug, Default)]
pub(crate) struct ClickThroughGate {
    ignoring: bool,
}

impl ClickThroughGate {
    pub(crate) fn decide(&mut self, want_ignore: bool) -> Option<bool> {
        if want_ignore == self.ignoring {
            None
        } else {
            self.ignoring = want_ignore;
            Some(want_ignore)
        }
    }

    /// A freshly built window accepts the cursor: forget stale state.
    pub(crate) fn reset(&mut self) {
        self.ignoring = false;
    }
}

/// One click-through evaluation against the live cursor. Shared by the poll
/// and by `companion_hit_rect`, so a fresh box from the renderer is applied
/// immediately instead of waiting for the next tick.
fn apply_companion_clickthrough(app: &tauri::AppHandle, win: &tauri::WebviewWindow) {
    if !win.is_visible().unwrap_or(false) {
        return;
    }
    let state: State<'_, AppState> = app.state();
    let (Ok(cursor), Ok(pos), Ok(size)) = (
        app.cursor_position(),
        win.outer_position(),
        win.outer_size(),
    ) else {
        return;
    };
    // Hit regions in logical px (reported by the renderer; defaults cover the
    // sprite's bounding box).
    let hit_regions = state
        .companion_hit_regions
        .lock()
        .expect("hit regions")
        .clone();
    let probe = ClickThroughProbe {
        cursor: (cursor.x, cursor.y),
        window_pos: (pos.x as f64, pos.y as f64),
        window_size: (size.width as f64, size.height as f64),
        scale: win.scale_factor().unwrap_or(1.0),
        hit_regions: &hit_regions,
        interactive: state.companion_interactive.load(Ordering::SeqCst),
    };
    let want_ignore = clickthrough_want_ignore(&probe);
    // Hold the gate across the toggle so a concurrent report and poll cannot
    // interleave decide/apply and leave the window in the other state.
    let mut gate = state
        .companion_clickthrough
        .lock()
        .expect("clickthrough gate");
    if let Some(next) = gate.decide(want_ignore) {
        let _ = win.set_ignore_cursor_events(next);
    }
}

/// Click-through for the transparent padding around the character: a small
/// Rust poll toggles ignore-cursor-events from the GLOBAL cursor position, so
/// clicks outside the character's hit-rect pass through to whatever is
/// underneath. The WebView cannot change policy here — it only reports where
/// the character is drawn (and that report re-evaluates at once, see
/// `companion_hit_rect`).
fn spawn_companion_clickthrough(app: tauri::AppHandle) {
    {
        // The window was just built and accepts the cursor; a gate left over
        // from a previous companion window must not skip the first toggle.
        let state: State<'_, AppState> = app.state();
        state
            .companion_clickthrough
            .lock()
            .expect("clickthrough gate")
            .reset();
    }
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(CLICKTHROUGH_POLL_MS)).await;
            let Some(win) = app.get_webview_window("companion") else {
                return; // window gone; a new one spawns a new loop
            };
            apply_companion_clickthrough(&app, &win);
        }
    });
}

/// Clamp a renderer-reported hit-rect into the window (logical px).
///
/// A bad rect is a real click-through hazard: a NaN or negative box would
/// make the whole transparent window swallow the cursor (or none of it).
/// Rejects non-finite / non-positive sizes; otherwise clamps the box so it
/// always stays inside `0..win_w` × `0..win_h`.
pub(crate) fn clamp_hit_rect(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    win_w: f64,
    win_h: f64,
) -> Result<(f64, f64, f64, f64), String> {
    if ![x, y, w, h, win_w, win_h].iter().all(|v| v.is_finite()) {
        return Err("hit rect must be finite numbers".into());
    }
    if w <= 0.0 || h <= 0.0 {
        return Err("hit rect width/height must be positive".into());
    }
    if win_w <= 0.0 || win_h <= 0.0 {
        return Err("window size must be positive".into());
    }
    let cx = x.clamp(0.0, win_w);
    let cy = y.clamp(0.0, win_h);
    let cw = w.min(win_w - cx).max(0.0);
    let ch = h.min(win_h - cy).max(0.0);
    if cw <= 0.0 || ch <= 0.0 {
        return Err("hit rect falls outside the companion window".into());
    }
    Ok((cx, cy, cw, ch))
}

/// One rectangle from the renderer. `id` is advisory (diagnostics only): the
/// host never lets the WebView name a region into extra authority.
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct HitRegionInput {
    #[serde(default)]
    #[allow(dead_code)]
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Companion window size in logical px. Falls back to the pref-derived stage
/// size when the window is not up yet — the caller's numbers are never trusted
/// as the window bounds.
fn companion_logical_size(app: &tauri::AppHandle, state: &State<'_, AppState>) -> (f64, f64) {
    app.get_webview_window("companion")
        .as_ref()
        .and_then(|win| {
            let scale = win.scale_factor().unwrap_or(1.0);
            win.inner_size()
                .ok()
                .map(|s| (s.width as f64 / scale, s.height as f64 / scale))
        })
        .unwrap_or_else(|| {
            let size = state.prefs.lock().expect("prefs mutex").companion_size;
            companion_window_size(size)
        })
}

/// Store a validated region list and re-evaluate click-through at once.
///
/// Rate-limited (`MIN_HIT_REGION_INTERVAL_MS`) and fail-closed: a rejected
/// report leaves the previous regions in place and logs a warning, so a buggy
/// renderer degrades to a slightly stale character box instead of either
/// swallowing the whole desktop or letting every click fall through the
/// character.
fn store_hit_regions(
    app: &tauri::AppHandle,
    state: &State<'_, AppState>,
    regions: &[HitRect],
) -> Result<(), String> {
    {
        let mut last = state
            .companion_hit_regions_at
            .lock()
            .expect("hit regions clock");
        let now = std::time::Instant::now();
        let elapsed = last.map(|t| now.duration_since(t).as_millis() as u64);
        if !hit_regions_accept(elapsed, MIN_HIT_REGION_INTERVAL_MS) {
            return Ok(()); // too fast; keep the regions we already have
        }
        *last = Some(now);
    }
    let (win_w, win_h) = companion_logical_size(app, state);
    let sanitized = match sanitize_hit_regions(regions, win_w, win_h) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "companion hit-region report rejected; keeping the previous regions");
            return Err(e);
        }
    };
    *state.companion_hit_regions.lock().expect("hit regions") = sanitized;
    // Fresh boxes mean the character (or a toy, or a menu) moved: re-evaluate
    // against the live cursor now rather than on the next poll tick, so the lag
    // is the report throttle alone, not throttle + poll.
    if let Some(win) = app.get_webview_window("companion") {
        apply_companion_clickthrough(app, &win);
    }
    Ok(())
}

/// The renderer reports every place something is actually drawn (logical px
/// within the window): the character, each familiar, each grabbable toy and
/// each open interactive UI surface. The transparent space BETWEEN them stays
/// click-through (companion-gameplay-032).
#[tauri::command]
async fn companion_hit_regions(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    regions: Vec<HitRegionInput>,
) -> Result<(), String> {
    let rects: Vec<HitRect> = regions.iter().map(|r| (r.x, r.y, r.w, r.h)).collect();
    store_hit_regions(&app, &state, &rects)
}

/// Compatibility shim for a single rectangle (older callers / sprite and text
/// entrypoints that really are one surface).
#[tauri::command]
async fn companion_hit_rect(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), String> {
    store_hit_regions(&app, &state, &[(x, y, w, h)])
}

/// A focused text input / drop confirmation → the whole window must accept the
/// cursor (native text selection and the OS drop target both need it).
///
/// Passive or self-contained surfaces (speech bubble, safety labels, the quick
/// menu) must NOT come through here: they report their own hit region, so the
/// transparent space around them still clicks through to the desktop.
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

/// Apply companion-related prefs live: visibility, always-on-top, title, and a
/// reload so character/persona/expressiveness changes take effect.
///
/// 只碰 `companion`；可信 overlay（`OVERLAY_LABEL`）的顯示與否由 `refresh_tray`
/// 依安全狀態決定，藏角色不會藏掉估停／感測指示。
#[tauri::command]
async fn companion_apply_prefs(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (visible, position, size, opacity, on_top, name) = {
        let prefs = state.prefs.lock().expect("prefs mutex");
        (
            prefs.companion_visible,
            prefs.companion_position,
            prefs.companion_size,
            prefs.companion_opacity,
            prefs.companion_always_on_top,
            prefs.companion_name.clone(),
        )
    };
    // Show／hide goes through the same path as the tray toggle and the
    // runtime's presence-set: window + Runtime hello + WebView event + tray
    // label together. Hiding here used to only `w.hide()`, leaving the
    // Runtime to find out from a heartbeat the hidden WebView may never send
    // (director-pipeline-024).
    apply_companion_visibility(&app, visible);
    if visible {
        if let Some(w) = app.get_webview_window("companion") {
            let _ = w.set_title(&companion_window_title(&name));
            let _ = w.set_always_on_top(on_top);
            let win = companion_window_size(size);
            let _ = w.set_size(tauri::LogicalSize::new(win.0, win.1));
            if let Some((x, y)) = position {
                let _ = w.set_position(tauri::LogicalPosition::new(x, y));
            }
        }
        let _ = app.emit("companion-opacity", opacity);
        let _ = app.emit("companion-reload", ());
    }
    Ok(())
}

/// Show or hide the companion for a caller that has to answer honestly —
/// the WebView acking the Runtime's `companion.presence.set` (presentation
/// actuator, AI-callable). Persists the pref, applies it through the same
/// path as the tray toggle, and returns the window state the OS reports;
/// `Err` when the window did not follow, so the caller acks `failed`, never
/// `completed` for a hide/show that did not happen (director-pipeline-021).
#[tauri::command]
async fn companion_set_visible(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    visible: bool,
) -> Result<Value, String> {
    {
        let mut prefs = state.prefs.lock().expect("prefs mutex");
        prefs.companion_visible = visible;
        supervisor::save_prefs(&prefs)?;
    }
    apply_companion_visibility(&app, visible);
    confirm_companion_visibility(visible, companion_window_is_visible(&app))
}

/// Honesty ladder for `companion_set_visible`: the answer is the OS-reported
/// window state, and a mismatch with the request is an error (→ the WebView
/// acks `failed`), never a `completed` for a show/hide that did not happen.
pub(crate) fn confirm_companion_visibility(requested: bool, actual: bool) -> Result<Value, String> {
    let word = |v: bool| if v { "visible" } else { "hidden" };
    if actual != requested {
        return Err(format!(
            "companion window is {} after asking for {}",
            word(actual),
            word(requested)
        ));
    }
    Ok(json!({ "visible": actual }))
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

    let plan = {
        let mut prefs = state.prefs.lock().expect("prefs mutex");
        let plan = plan_window_adjust(&prefs, params)?;
        prefs.companion_position = Some(plan.position);
        prefs.companion_size = plan.size;
        prefs.companion_opacity = plan.opacity;
        prefs.companion_always_on_top = plan.on_top;
        supervisor::save_prefs(&prefs)?;
        plan
    };

    let window = app
        .get_webview_window("companion")
        .ok_or_else(|| "companion window is not present".to_string())?;
    window
        .set_position(tauri::LogicalPosition::new(
            plan.position.0,
            plan.position.1,
        ))
        .map_err(err_s)?;
    // 角色尺寸 → 視窗尺寸走同一個乘數（與 ensure_companion_window／apply_prefs 一致）。
    window
        .set_size(tauri::LogicalSize::new(
            plan.window_size.0,
            plan.window_size.1,
        ))
        .map_err(err_s)?;
    window.set_always_on_top(plan.on_top).map_err(err_s)?;
    app.emit("companion-opacity", plan.opacity).map_err(err_s)?;
    Ok(json!({
        "actionId": action_id,
        "position": plan.position,
        "size": plan.size,
        "windowSize": plan.window_size,
        "opacity": plan.opacity,
        "alwaysOnTop": plan.on_top,
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

// ---------------------------------------------------------------------------
// 可信 host overlay：estop／感測／離線的固定文字指示，獨立於角色 renderer。
// ---------------------------------------------------------------------------

/// 可信 host overlay 視窗標籤。
pub(crate) const OVERLAY_LABEL: &str = "overlay";
/// overlay 視窗邏輯尺寸（放得下 estop＋離線＋麥克風＋攝影機四行）。
pub(crate) const OVERLAY_SIZE: (f64, f64) = (340.0, 200.0);
/// 距主螢幕工作區右上角的邊距（邏輯 px）。
pub(crate) const OVERLAY_MARGIN: f64 = 12.0;

/// 主螢幕工作區（實體 px）→ overlay 左上角邏輯座標（右上角對齊）。
pub(crate) fn overlay_anchor_top_right(
    work_x: f64,
    work_y: f64,
    work_w: f64,
    scale: f64,
) -> (f64, f64) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let left = work_x / scale;
    let right = (work_x + work_w) / scale;
    let top = work_y / scale;
    (
        (right - OVERLAY_SIZE.0 - OVERLAY_MARGIN).max(left),
        top + OVERLAY_MARGIN,
    )
}

fn overlay_position(app: &tauri::AppHandle) -> Option<(f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let area = monitor.work_area();
    Some(overlay_anchor_top_right(
        area.position.x as f64,
        area.position.y as f64,
        area.size.width as f64,
        monitor.scale_factor(),
    ))
}

/// 建立 overlay：透明、無邊框、無陰影、不進工作列、永遠置頂、不取焦點，
/// 建好立刻忽略游標事件（永遠 click-through，不像 companion 需要 hit-rect 輪詢）。
/// 內容只由 `host-safety` 事件驅動；WebView 端不呼叫任何 api.*。
fn create_overlay_window(app: &tauri::AppHandle) {
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        OVERLAY_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("安全狀態")
    .inner_size(OVERLAY_SIZE.0, OVERLAY_SIZE.1)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .focused(false)
    .accept_first_mouse(false);
    if let Some((x, y)) = overlay_position(app) {
        builder = builder.position(x, y);
    }
    match builder.build() {
        Ok(win) => {
            if let Err(e) = win.set_ignore_cursor_events(true) {
                tracing::warn!(error = %e, "overlay ignore-cursor-events failed");
            }
        }
        Err(e) => tracing::error!(error = %e, "overlay window create failed"),
    }
}

/// 依安全視圖同步 overlay：需要時建立並推送內容；不需要時關閉。
///
/// 不用 hide()/show()：macOS 的 show 會把視窗設為 key window（搶焦點），而
/// overlay 永遠不可以搶焦點；重新建立時 `focused(false)` 才有效。建立瞬間的
/// 那一次 emit 可能趕在 listener 之前，因此 overlay 掛好後會呼叫 `overlay_attach`
/// 把快取的視圖再拿一次。
fn sync_overlay_window(app: &tauri::AppHandle, view: &HostSafetyView) {
    let state: State<'_, AppState> = app.state();
    *state.host_safety.lock().expect("host safety mutex") = Some(view.clone());
    let existing = app.get_webview_window(OVERLAY_LABEL);
    if view.active {
        if existing.is_none() {
            create_overlay_window(app);
        }
        let _ = app.emit_to(EventTarget::labeled(OVERLAY_LABEL), HOST_SAFETY_EVENT, view);
    } else if let Some(win) = existing {
        let _ = win.close();
    }
}

/// 哪些 Runtime 事件會改變 host 安全視圖（estop 進入／解除、感測開始／停止、
/// 手機連線變化＝手機麥克風可能停了）：要立刻刷新 tray 與 overlay，不等 4 秒輪詢。
pub(crate) fn host_safety_relevant(event_type: &EventType) -> bool {
    matches!(
        event_type,
        EventType::EmergencyStop
            | EventType::SensorStarted
            | EventType::SensorStopped
            // 停止結果未知：tray／overlay 必須立刻改口說「結果未知」，
            // 不能等 4 秒輪詢，更不能讓畫面看起來像已經停了。
            | EventType::SensorStopUncertain
            | EventType::ProviderStateChanged
    )
}

/// 事件驅動的刷新（合併：進行中就記一筆「再跑一次」，不堆 task、不無界）。
pub(crate) fn request_host_refresh(app: &tauri::AppHandle) {
    let state: State<'_, AppState> = app.state();
    if state
        .host_refresh_inflight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        state.host_refresh_again.store(true, Ordering::SeqCst);
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            refresh_tray(&handle).await;
            let state: State<'_, AppState> = handle.state();
            if !state.host_refresh_again.swap(false, Ordering::SeqCst) {
                break;
            }
        }
        let state: State<'_, AppState> = handle.state();
        state.host_refresh_inflight.store(false, Ordering::SeqCst);
        // 收尾縫隙：剛才那一瞬間又有人要求刷新 → 再排一次，不遺漏。
        if state.host_refresh_again.swap(false, Ordering::SeqCst) {
            request_host_refresh(&handle);
        }
    });
}

/// Refresh tray texts/glyph AND the trusted overlay from the live backend state.
/// 兩者共用同一份 `HostSafetyView`（host_safety.rs），所以永遠一致；
/// 兩種模式都走 `Backend::status`（embedded 直呼、external 打 HTTP）。
pub(crate) async fn refresh_tray(app: &tauri::AppHandle) {
    let state: State<'_, AppState> = app.state();
    let backend = state.backend();
    let (ready, starting, external) = {
        let info = state.supervisor.lock().expect("supervisor mutex");
        (
            matches!(
                info.state,
                SupervisorState::Ready
                    | SupervisorState::EmbeddedOwned
                    | SupervisorState::ConnectedToExternal
            ),
            info.state == SupervisorState::Starting
                && state.launched_at.elapsed().as_secs() < host_safety::STARTING_GRACE_SECS,
            info.mode == SupervisorMode::External,
        )
    };
    let status = match backend {
        Some(b) => b.status().await.ok(),
        None => None,
    };
    let sessions = status
        .as_ref()
        .and_then(|s| s.get("agentSessions"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    // Sensor privacy indicator: status.activeSensors is present in both
    // embedded and external modes (local capture + iPhone streaming).
    let view = HostSafetyView::derive(ready, starting, status.as_ref(), chrono::Utc::now());
    let tray_view = tray::tray_view(&view, external, sessions);
    let companion_visible = state.prefs.lock().expect("prefs mutex").companion_visible;
    {
        let guard = state.tray.lock().expect("tray mutex");
        if let Some(handles) = guard.as_ref() {
            let _ = handles.info_status.set_text(&tray_view.status_text);
            let _ = handles.info_pause.set_text(&tray_view.pause_text);
            let _ = handles.info_sessions.set_text(&tray_view.sessions_text);
            let _ = handles.toggle_pause.set_text(&tray_view.pause_action_text);
            let _ = handles.toggle_companion.set_text(if companion_visible {
                "隱藏桌面角色"
            } else {
                "顯示桌面角色"
            });
            #[cfg(target_os = "macos")]
            {
                let _ = handles.tray.set_title(tray_view.title_glyph);
            }
        }
    };
    sync_overlay_window(app, &view);
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
            // Forward runtime events to the WebView, and let estop/sensor
            // events refresh tray + overlay immediately (event-driven, not 4 s).
            let mut rx = runtime.events.subscribe();
            let event_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            let _ = event_handle.emit("runtime-event", &event);
                            if host_safety_relevant(&event.event_type) {
                                request_host_refresh(&event_handle);
                            }
                        }
                        // 慢消費者落後：跳過被覆蓋的事件、繼續轉送；不能因此永遠停掉。
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                            tracing::warn!(dropped, "runtime event forwarder lagged");
                            request_host_refresh(&event_handle);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
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
        // Native folder picker (work page). Returns a path string only; the
        // capability grant is scoped to the `main` window and there is no fs
        // plugin, so the WebView still cannot touch the filesystem.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            runtime: Mutex::new(None),
            startup_error: Mutex::new(None),
            supervisor: Mutex::new(SupervisorInfo::starting()),
            prefs: Mutex::new(supervisor::load_prefs()),
            quitting: AtomicBool::new(false),
            tray: Mutex::new(None),
            // Default region ≈ the sprite's body area at scale 1.1.
            companion_hit_regions: Mutex::new(vec![(30.0, 30.0, 120.0, 150.0)]),
            companion_hit_regions_at: Mutex::new(None),
            companion_clickthrough: Mutex::new(ClickThroughGate::default()),
            companion_interactive: AtomicBool::new(false),
            host_safety: Mutex::new(None),
            host_refresh_inflight: AtomicBool::new(false),
            host_refresh_again: AtomicBool::new(false),
            launched_at: std::time::Instant::now(),
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
            onboarding_preview,
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
            companion_hit_regions,
            companion_set_interactive,
            companion_open_control_center,
            companion_apply_prefs,
            companion_set_visible,
            companion_window_adjust,
            companion_reset_position,
            agent_sessions_list,
            agent_session_send,
            agent_session_close,
            agent_session_verify,
            mobile_status,
            mobile_pairing_begin,
            mobile_revoke,
            mobile_sensors_stop,
            mobile_test,
            mobile_ble_scan,
            sensor_mic_listen,
            presentation_status,
            presentation_hello,
            presentation_ack,
            proactive_dialogue_get,
            proactive_dialogue_patch,
            proactive_dialogue_quiet,
            providers_list,
            provider_test,
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
            character_hello,
            character_receipt,
            character_event,
            character_instances,
            character_manifest,
            character_adapters,
            character_adapter_revoke,
            character_import,
            character_list_imported,
            character_asset,
            character_remove,
            overlay_attach,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (Phase 7 review #1): the click-through poll must not lag so
    /// far behind the renderer that a walking character's box is stale. The
    /// WebView reports at most every ~60ms; the poll has to keep up.
    #[test]
    fn clickthrough_poll_keeps_up_with_hit_rect_reports() {
        // `HIT_RECT_MAX_QUIET_MS` in src/companion/rig/stage.ts — the renderer
        // refreshes the box at least this often while the character walks.
        let renderer_max_quiet_ms: u64 = 60;
        let poll = CLICKTHROUGH_POLL_MS;
        assert!(
            poll <= renderer_max_quiet_ms + 40,
            "poll {poll}ms lags too far behind the renderer's {renderer_max_quiet_ms}ms reports"
        );
        assert!(
            poll >= 40,
            "polling faster than 40ms wastes CPU for no gain"
        );
    }

    /// Pure click-through decision (physical px, 2x Retina): the transparent
    /// padding ignores the cursor, the character accepts it, an interactive
    /// window (menu open) and a cursor outside the window always re-arm.
    #[test]
    fn clickthrough_decision_ignores_padding_and_accepts_character() {
        // Window at (100, 200) physical, 1040×568 physical (520×284 logical @2x);
        // character box (30, 20, 52, 124) logical → (160, 240)..(264, 488) physical.
        let character = [(30.0, 20.0, 52.0, 124.0)];
        let base = ClickThroughProbe {
            cursor: (0.0, 0.0),
            window_pos: (100.0, 200.0),
            window_size: (1040.0, 568.0),
            scale: 2.0,
            hit_regions: &character,
            interactive: false,
        };
        // On the character: accept the cursor.
        assert!(!clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (200.0, 300.0),
            ..base
        }));
        // Inside the window but on transparent padding: click-through.
        assert!(clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (900.0, 300.0),
            ..base
        }));
        // Boundary: the box is half-open, so its right/bottom edge is padding.
        assert!(!clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (263.9, 487.9),
            ..base
        }));
        assert!(clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (264.0, 300.0),
            ..base
        }));
        // Outside the window: re-arm so the next approach is clickable.
        assert!(!clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (10.0, 10.0),
            ..base
        }));
        // Menus/inputs open: the whole window accepts the cursor, padding included.
        assert!(!clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (900.0, 300.0),
            interactive: true,
            ..base
        }));
    }

    /// Regression (companion-gameplay-032): the host must intercept the cursor
    /// only where an object actually is. The renderer draws a character on the
    /// left and a toy thrown far to the right; the empty band between them
    /// belongs to the desktop, so a click there has to pass through.
    ///
    /// A single union rectangle (30, 20, 378, 128) would swallow that band;
    /// two bounded regions do not.
    #[test]
    fn clickthrough_ignores_gaps_between_objects() {
        // Character (30, 20, 52, 124) logical; toy (380, 120, 28, 28) logical.
        let regions = [(30.0, 20.0, 52.0, 124.0), (380.0, 120.0, 28.0, 28.0)];
        let base = ClickThroughProbe {
            cursor: (0.0, 0.0),
            window_pos: (100.0, 200.0),
            window_size: (1040.0, 568.0),
            scale: 2.0,
            hit_regions: &regions,
            interactive: false,
        };
        // On the character: intercept.
        assert!(!clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (200.0, 300.0),
            ..base
        }));
        // On the toy: intercept.
        assert!(!clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (870.0, 450.0),
            ..base
        }));
        // Between them: nothing is drawn there, so the desktop gets the click.
        assert!(
            clickthrough_want_ignore(&ClickThroughProbe {
                cursor: (500.0, 300.0),
                ..base
            }),
            "the blank band between the character and a far toy must click through"
        );
        // The union of the same two boxes DOES swallow that point — this is
        // exactly the bug the regions fix (companion-gameplay-032).
        let union = [(30.0, 20.0, 378.0, 128.0)];
        assert!(!clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (500.0, 300.0),
            hit_regions: &union,
            ..base
        }));
        // A focused input / drop confirmation still takes the whole window.
        assert!(!clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (500.0, 300.0),
            interactive: true,
            ..base
        }));
        // Outside the window: re-arm regardless of how many regions there are.
        assert!(!clickthrough_want_ignore(&ClickThroughProbe {
            cursor: (10.0, 10.0),
            ..base
        }));
    }

    /// Several characters (the maid plus her familiars) and several toys: every
    /// one of them intercepts, and the gaps between them all click through.
    #[test]
    fn clickthrough_handles_many_regions_and_their_gaps() {
        // logical: character, two familiars, one toy.
        let regions = [
            (30.0, 20.0, 52.0, 124.0),
            (150.0, 100.0, 30.0, 26.0),
            (260.0, 100.0, 30.0, 26.0),
            (400.0, 130.0, 28.0, 28.0),
        ];
        let base = ClickThroughProbe {
            cursor: (0.0, 0.0),
            window_pos: (0.0, 0.0),
            window_size: (520.0, 284.0),
            scale: 1.0,
            hit_regions: &regions,
            interactive: false,
        };
        for (x, y) in [(40.0, 30.0), (160.0, 110.0), (270.0, 110.0), (410.0, 140.0)] {
            assert!(
                !clickthrough_want_ignore(&ClickThroughProbe {
                    cursor: (x, y),
                    ..base
                }),
                "({x},{y}) is on an object and must intercept"
            );
        }
        for (x, y) in [
            (120.0, 110.0),
            (220.0, 110.0),
            (350.0, 140.0),
            (40.0, 200.0),
        ] {
            assert!(
                clickthrough_want_ignore(&ClickThroughProbe {
                    cursor: (x, y),
                    ..base
                }),
                "({x},{y}) is blank and must click through to the desktop"
            );
        }
    }

    /// Bounds and fail-closed validation of a report.
    #[test]
    fn hit_regions_are_bounded_clamped_and_fail_closed() {
        let (w, h) = (520.0, 284.0);
        // Normal report survives untouched.
        let ok = sanitize_hit_regions(
            &[(30.0, 20.0, 52.0, 124.0), (400.0, 130.0, 28.0, 28.0)],
            w,
            h,
        )
        .expect("valid");
        assert_eq!(ok.len(), 2);
        assert_eq!(ok[0], (30.0, 20.0, 52.0, 124.0));
        // Clamped into the window (and the character stays first).
        let clamped = sanitize_hit_regions(&[(-40.0, -10.0, 90.0, 60.0)], w, h).expect("clamped");
        assert_eq!(clamped, vec![(0.0, 0.0, 90.0, 60.0)]);
        // Count cap: only the first MAX_HIT_REGIONS survive.
        let many: Vec<HitRect> = (0..40).map(|i| (i as f64, 0.0, 4.0, 4.0)).collect();
        let kept = sanitize_hit_regions(&many, w, h).expect("capped");
        assert_eq!(kept.len(), MAX_HIT_REGIONS);
        assert_eq!(kept[0], (0.0, 0.0, 4.0, 4.0));
        // Empty report: rejected (never "the whole window is transparent").
        assert!(sanitize_hit_regions(&[], w, h).is_err());
        // Whole-window grab: rejected.
        assert!(sanitize_hit_regions(&[(0.0, 0.0, w, h)], w, h).is_err());
        assert!(sanitize_hit_regions(&[(0.0, 0.0, w * 0.85, h * 0.85)], w, h).is_err());
        // Wide-but-short and tall-but-narrow bars are still legitimate.
        assert!(sanitize_hit_regions(&[(0.0, 0.0, w, 20.0)], w, h).is_ok());
        assert!(sanitize_hit_regions(&[(0.0, 0.0, 20.0, h)], w, h).is_ok());
        // …but boxes that each squeak past the per-region cap may not add up to
        // the whole window either (stacked/overlapping is the obvious dodge).
        let land_grab: Vec<HitRect> = (0..4).map(|_| (0.0, 0.0, w * 0.95, h * 0.75)).collect();
        assert!(sanitize_hit_regions(&land_grab[..1], w, h).is_ok()); // one is fine
        assert!(sanitize_hit_regions(&land_grab, w, h).is_err());
        // Garbage anywhere in the list rejects the WHOLE report (fail-closed).
        assert!(sanitize_hit_regions(
            &[(30.0, 20.0, 52.0, 124.0), (f64::NAN, 0.0, 4.0, 4.0)],
            w,
            h
        )
        .is_err());
        assert!(
            sanitize_hit_regions(&[(30.0, 20.0, 52.0, 124.0), (0.0, 0.0, -4.0, 4.0)], w, h)
                .is_err()
        );
        assert!(
            sanitize_hit_regions(&[(30.0, 20.0, 52.0, 124.0), (900.0, 0.0, 4.0, 4.0)], w, h)
                .is_err()
        );
    }

    /// Host-side rate limit: the renderer's own 50ms floor passes, a flood does
    /// not, and the first report is always accepted.
    #[test]
    fn hit_region_reports_are_rate_limited_on_the_host() {
        assert!(hit_regions_accept(None, MIN_HIT_REGION_INTERVAL_MS));
        // The renderer's honest cadence (≥50ms) is never dropped.
        assert!(hit_regions_accept(Some(50), MIN_HIT_REGION_INTERVAL_MS));
        assert!(hit_regions_accept(Some(60), MIN_HIT_REGION_INTERVAL_MS));
        // A runaway caller is bounded.
        assert!(!hit_regions_accept(Some(0), MIN_HIT_REGION_INTERVAL_MS));
        assert!(!hit_regions_accept(Some(16), MIN_HIT_REGION_INTERVAL_MS));
        // The floor itself stays under the renderer's 50ms and within 60ms
        // (compile-time: a constant assertion, so clippy wants a const block).
        const {
            assert!(MIN_HIT_REGION_INTERVAL_MS <= 60);
            assert!(MIN_HIT_REGION_INTERVAL_MS < 50);
        }
    }

    /// Regression (perf-claims-018): a fresh hit-rect report must re-evaluate
    /// click-through at once — a character walking under a still cursor used
    /// to wait for the next 80ms poll on top of the ≤60ms report throttle.
    /// `companion_hit_rect` and the poll share one gate, so the report itself
    /// flips ignore-cursor-events and the following poll has nothing to do.
    #[test]
    fn hit_rect_update_reevaluates_without_waiting_for_poll() {
        let mut gate = ClickThroughGate::default();
        let still_cursor = (900.0, 300.0);
        const OLD_RECT: &[HitRect] = &[(30.0, 20.0, 52.0, 124.0)]; // character far left
        let old_rect = OLD_RECT;
        let probe = |hit_regions: &'static [HitRect]| ClickThroughProbe {
            cursor: still_cursor,
            window_pos: (100.0, 200.0),
            window_size: (1040.0, 568.0),
            scale: 2.0,
            hit_regions,
            interactive: false,
        };
        // Poll tick: cursor on padding → start ignoring.
        assert_eq!(
            gate.decide(clickthrough_want_ignore(&probe(old_rect))),
            Some(true)
        );
        // The character walks under the cursor; the renderer reports the new box.
        // Cursor (900,300) physical = window-relative (400, 50) logical @2x.
        const NEW_RECT: &[HitRect] = &[(380.0, 20.0, 52.0, 124.0)];
        let new_rect = NEW_RECT;
        // The report path applies the decision immediately (no poll in between).
        assert_eq!(
            gate.decide(clickthrough_want_ignore(&probe(new_rect))),
            Some(false)
        );
        // The next poll sees the same answer and must not toggle again.
        assert_eq!(
            gate.decide(clickthrough_want_ignore(&probe(new_rect))),
            None
        );
        // A new companion window starts out accepting the cursor: reset forgets
        // the stale "ignoring" state so the first padding poll re-applies it.
        assert_eq!(
            gate.decide(clickthrough_want_ignore(&probe(old_rect))),
            Some(true)
        );
        gate.reset();
        assert_eq!(
            gate.decide(clickthrough_want_ignore(&probe(old_rect))),
            Some(true)
        );
    }

    /// The CHANGELOG section still being written: `## [Unreleased]` while it is
    /// unnamed, or — once a release is being prepared and the heading is renamed
    /// to `## [x.y.z] — …` — the topmost version section. Returns its body (the
    /// text up to the next `## [` heading).
    ///
    /// Release prep renames the heading; the claim checks below must follow it
    /// instead of silently having nothing to check (or panicking on a heading
    /// that is simply no longer called "Unreleased").
    fn changelog_topmost_section(text: &str) -> &str {
        let start = text
            .find("## [Unreleased]")
            .or_else(|| text.find("\n## [").map(|i| i + 1))
            .expect("CHANGELOG has a version section");
        let after_heading = start
            + text[start..]
                .find('\n')
                .expect("version heading ends with a newline");
        let rest = &text[after_heading..];
        let end = rest.find("\n## [").unwrap_or(rest.len());
        &rest[..end]
    }

    /// Doc-consistency (perf-claims-024): the topmost CHANGELOG section (the one
    /// still being written — `## [Unreleased]`, or the version heading it was
    /// renamed to during release prep) must not carry the pre-Phase-7
    /// "Rust 每 500ms 收到新互動框" claim, and every click-through poll interval
    /// it quotes must equal `CLICKTHROUGH_POLL_MS`.
    #[test]
    fn changelog_unreleased_click_through_claims_match_code() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../CHANGELOG.md");
        let text = std::fs::read_to_string(path).expect("CHANGELOG.md readable");
        let unreleased = changelog_topmost_section(&text);
        assert!(
            !unreleased.contains("每 500ms 收到新互動框"),
            "stale Phase 3 hit-rect claim still in the topmost CHANGELOG section"
        );
        let mut quoted = 0;
        for (idx, _) in unreleased.match_indices("點擊穿透輪詢") {
            let tail = &unreleased[idx + "點擊穿透輪詢".len()..];
            let tail = tail.trim_start_matches([' ', '\u{3000}']);
            let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() {
                continue;
            }
            let ms: u64 = digits.parse().expect("poll interval digits");
            assert_eq!(
                ms, CLICKTHROUGH_POLL_MS,
                "CHANGELOG quotes a {ms}ms click-through poll; code uses {CLICKTHROUGH_POLL_MS}ms"
            );
            quoted += 1;
        }
        assert!(
            quoted >= 1,
            "the topmost CHANGELOG section should quote the click-through poll interval at least once"
        );
    }

    /// The section finder itself: it must follow a renamed heading (release prep
    /// turns `## [Unreleased]` into `## [0.5.0] — …`) and must stop at the next
    /// version heading, so a claim from an older release can never satisfy — or
    /// break — the checks above.
    #[test]
    fn changelog_topmost_section_follows_a_renamed_heading() {
        let unreleased =
            "# CHANGELOG\n\n## [Unreleased]\n新的一句\n\n## [0.4.0] - 2026-08-28\n舊的一句\n";
        assert_eq!(
            changelog_topmost_section(unreleased),
            "\n新的一句\n",
            "still finds the classic Unreleased heading"
        );
        let renamed = "# CHANGELOG\n\n## [0.5.0] — 2026-09-03（準備中）\n新的一句\n\n## [0.4.0] - 2026-08-28\n舊的一句\n";
        assert_eq!(
            changelog_topmost_section(renamed),
            "\n新的一句\n",
            "release prep renamed the heading; the topmost version section is still the one checked"
        );
        assert!(
            !changelog_topmost_section(renamed).contains("舊的一句"),
            "must stop at the next version heading"
        );
        // The real file: whichever shape it is in, the section is non-empty and
        // is not the 0.4.x history.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../CHANGELOG.md");
        let text = std::fs::read_to_string(path).expect("CHANGELOG.md readable");
        let section = changelog_topmost_section(&text);
        assert!(!section.trim().is_empty(), "topmost section is empty");
        assert!(
            !section.contains("## [0.4.1]"),
            "topmost section leaked into an older release"
        );
    }

    /// Regression (director-pipeline-021／-024): every host path that hides the
    /// character must tell the Runtime itself, without waiting for a WebView
    /// hello the hidden window may never send. The shared announcer flips the
    /// Runtime's presentation surface to hidden at once (and back).
    #[tokio::test]
    async fn announce_companion_presence_marks_runtime_hidden_without_webview_hello() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime = Runtime::start(RuntimeOptions {
            home: Some(dir.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: true,
            spawn_watchdog: false,
        })
        .await
        .expect("runtime");
        // Fresh runtime: no surface has said hello yet.
        let before = runtime.presentation_status();
        assert_eq!(before["connected"], false);
        assert_eq!(before["visible"], false);

        // The WebView says it is up (what the companion window does on load).
        runtime
            .presentation_hello_with_behavior(true, Some("shu-maid".into()), None)
            .await;
        assert_eq!(runtime.presentation_status()["visible"], true);

        // Host hides the window (tray／control-center／presence-set): the
        // Runtime must see "hidden" immediately — no WebView involvement.
        announce_companion_presence(runtime.clone(), false, "shu-maid".into()).await;
        let hidden = runtime.presentation_status();
        assert_eq!(hidden["connected"], true, "hidden ≠ disconnected");
        assert_eq!(hidden["visible"], false);
        assert_eq!(hidden["packId"], "shu-maid");

        // Showing again is the same call with `true`.
        announce_companion_presence(runtime.clone(), true, "shu-maid".into()).await;
        assert_eq!(runtime.presentation_status()["visible"], true);
    }

    /// `companion_set_visible` reports the window state the OS gives back,
    /// never the requested one: with no companion window at all, asking for
    /// "visible" is a failure the caller must ack as `failed`.
    #[test]
    fn set_visible_result_is_the_actual_window_state() {
        assert_eq!(
            confirm_companion_visibility(true, true),
            Ok(json!({ "visible": true }))
        );
        assert_eq!(
            confirm_companion_visibility(false, false),
            Ok(json!({ "visible": false }))
        );
        let err = confirm_companion_visibility(true, false).expect_err("no window → failed");
        assert!(err.contains("hidden after asking for visible"), "{err}");
        assert!(confirm_companion_visibility(false, true).is_err());
    }

    #[test]
    fn hit_rect_clamps_into_the_window() {
        // 超出視窗：夾回視窗內，寬高跟著縮。
        let r = clamp_hit_rect(-40.0, -10.0, 600.0, 900.0, 520.0, 284.0).expect("clamped");
        assert_eq!(r, (0.0, 0.0, 520.0, 284.0));
        // 一般情況原樣通過。
        let r = clamp_hit_rect(30.0, 20.0, 52.0, 124.0, 520.0, 284.0).expect("ok");
        assert_eq!(r, (30.0, 20.0, 52.0, 124.0));
        // 右下角溢出：只縮寬高，不移動起點。
        let r = clamp_hit_rect(500.0, 270.0, 200.0, 200.0, 520.0, 284.0).expect("ok");
        assert_eq!(r, (500.0, 270.0, 20.0, 14.0));
    }

    #[test]
    fn hit_rect_rejects_nan_and_non_positive_sizes() {
        assert!(clamp_hit_rect(f64::NAN, 0.0, 10.0, 10.0, 520.0, 284.0).is_err());
        assert!(clamp_hit_rect(0.0, f64::INFINITY, 10.0, 10.0, 520.0, 284.0).is_err());
        assert!(clamp_hit_rect(0.0, 0.0, -10.0, 10.0, 520.0, 284.0).is_err());
        assert!(clamp_hit_rect(0.0, 0.0, 10.0, 0.0, 520.0, 284.0).is_err());
        assert!(clamp_hit_rect(0.0, 0.0, 10.0, 10.0, 0.0, 284.0).is_err());
        // 完全落在視窗外：拒絕，而不是回一個空框讓整個視窗吃掉游標。
        assert!(clamp_hit_rect(600.0, 10.0, 40.0, 40.0, 520.0, 284.0).is_err());
    }

    fn prefs_map(raw: Value) -> BTreeMap<String, BTreeMap<String, Value>> {
        serde_json::from_value(raw).expect("preference map parses")
    }

    /// 角色偏好表：合規的純量值通過，整張表原樣保留（patch 取代整張表）。
    #[test]
    fn companion_preferences_accept_bounded_scalars() {
        let map = prefs_map(json!({
            "shu-maid": { "variant": "maid-dusk", "bubbleSpeed": 1.5, "sound": false },
            "my.char_2-x": { "count": 3 }
        }));
        assert_eq!(validate_companion_preferences(&map), Ok(()));
        // 空表也合法（使用者清掉所有偏好）。
        assert_eq!(validate_companion_preferences(&BTreeMap::new()), Ok(()));
        // 邊界：剛好 16 個角色、每角色剛好 32 鍵、字串剛好 200 字。
        let mut big = BTreeMap::new();
        for i in 0..MAX_COMPANION_PREFERENCE_CHARACTERS {
            let mut values = BTreeMap::new();
            for k in 0..MAX_COMPANION_PREFERENCE_KEYS {
                values.insert(format!("k{k}"), Value::String("字".repeat(200)));
            }
            big.insert(format!("c{i}"), values);
        }
        assert_eq!(validate_companion_preferences(&big), Ok(()));
    }

    #[test]
    fn companion_preferences_reject_too_many_characters_or_keys() {
        let mut too_many = BTreeMap::new();
        for i in 0..=MAX_COMPANION_PREFERENCE_CHARACTERS {
            too_many.insert(format!("c{i}"), BTreeMap::new());
        }
        let err = validate_companion_preferences(&too_many).unwrap_err();
        assert!(err.contains("at most 16 characters"), "{err}");

        let mut values = BTreeMap::new();
        for k in 0..=MAX_COMPANION_PREFERENCE_KEYS {
            values.insert(format!("k{k}"), Value::Bool(true));
        }
        let map = BTreeMap::from([("shu-maid".to_string(), values)]);
        let err = validate_companion_preferences(&map).unwrap_err();
        assert!(err.contains("at most 32 keys"), "{err}");
    }

    #[test]
    fn companion_preferences_reject_bad_ids_keys_and_values() {
        // characterId 規則。
        for bad in [
            "",
            "Shu",
            "-shu",
            "shu maid",
            &format!("a{}", "b".repeat(64)),
        ] {
            let map = BTreeMap::from([(bad.to_string(), BTreeMap::new())]);
            let err = validate_companion_preferences(&map).unwrap_err();
            assert!(err.contains("characterId"), "{bad:?}: {err}");
        }
        assert!(is_valid_character_id(&format!("a{}", "b".repeat(63))));
        // 鍵規則。
        for bad in ["", "has space", "中文", &"k".repeat(65)] {
            let map = prefs_map(json!({ "shu-maid": { bad: true } }));
            let err = validate_companion_preferences(&map).unwrap_err();
            assert!(err.contains("invalid key"), "{bad:?}: {err}");
        }
        // 值：null／陣列／物件／超長字串都拒絕，而且錯誤訊息不回顯值本身。
        for bad in [json!(null), json!([1]), json!({"nested": 1})] {
            let map = prefs_map(json!({ "shu-maid": { "variant": bad } }));
            let err = validate_companion_preferences(&map).unwrap_err();
            assert!(err.contains("boolean, a number, or a string"), "{err}");
        }
        let long = "x".repeat(201);
        let map = prefs_map(json!({ "shu-maid": { "note": long } }));
        let err = validate_companion_preferences(&map).unwrap_err();
        assert!(err.contains("200 characters"), "{err}");
        assert!(!err.contains("xxxx"), "must not echo the value");
    }

    /// 修正（map-00 gaps）：`companion_window_adjust` 過去把角色尺寸直接 set_size，
    /// 沒套 `companion_window_size` 乘數，下一次 apply／reset 會把視窗縮回去。
    #[test]
    fn window_adjust_applies_the_playfield_multiplier() {
        let prefs = DesktopPrefs::default();
        let params: serde_json::Map<String, Value> =
            serde_json::from_value(json!({"width": 300.0, "height": 320.0, "x": 10.0, "y": 20.0}))
                .expect("map");
        let plan = plan_window_adjust(&prefs, &params).expect("plan");
        assert_eq!(plan.size, (300.0, 320.0));
        assert_eq!(plan.window_size, companion_window_size((300.0, 320.0)));
        assert_ne!(
            plan.window_size, plan.size,
            "the window must be wider/taller than the character (playfield)"
        );
        assert_eq!(plan.position, (10.0, 20.0));
        assert_eq!(plan.opacity, prefs.companion_opacity);
        assert_eq!(plan.on_top, prefs.companion_always_on_top);
        // 缺的欄位沿用 prefs，視窗尺寸仍經同一個乘數。
        let plan = plan_window_adjust(&prefs, &serde_json::Map::new()).expect("plan");
        assert_eq!(plan.size, prefs.companion_size);
        assert_eq!(
            plan.window_size,
            companion_window_size(prefs.companion_size)
        );
        // 邊界與 desktop_prefs_patch 一致（Runtime 驗過，native 仍是最後一道）。
        for bad in [
            json!({"width": 10.0}),
            json!({"height": 5000.0}),
            json!({"opacity": 0.0}),
            json!({"opacity": 1.5}),
            json!({"x": 99_999.0}),
        ] {
            let params: serde_json::Map<String, Value> =
                serde_json::from_value(bad.clone()).expect("map");
            assert!(plan_window_adjust(&prefs, &params).is_err(), "{bad}");
        }
    }

    /// 可信 overlay 不得被 close-behavior（hide-companion／keep-running）藏掉。
    #[test]
    fn close_behavior_never_hides_the_trusted_overlay() {
        for behavior in ["hide-companion", "keep-running", "quit", "bogus"] {
            let hidden = windows_hidden_by_close_behavior(behavior);
            assert!(
                !hidden.contains(&OVERLAY_LABEL),
                "{behavior} must not hide the overlay"
            );
        }
        assert_eq!(
            windows_hidden_by_close_behavior("hide-companion"),
            &["main", "companion"]
        );
        assert_eq!(windows_hidden_by_close_behavior("keep-running"), &["main"]);
        assert!(windows_hidden_by_close_behavior("quit").is_empty());
        assert!(windows_hidden_by_close_behavior("bogus").is_empty());
    }

    #[test]
    fn overlay_anchors_to_the_top_right_of_the_work_area() {
        // 2x Retina：工作區 (0,50) 寬 2880 實體 px → 邏輯右上角。
        let (x, y) = overlay_anchor_top_right(0.0, 50.0, 2880.0, 2.0);
        assert_eq!(x, 1440.0 - OVERLAY_SIZE.0 - OVERLAY_MARGIN);
        assert_eq!(y, 25.0 + OVERLAY_MARGIN);
        // 主螢幕在左邊（負座標）也對齊該工作區的右上角。
        let (x, _) = overlay_anchor_top_right(-1920.0, 0.0, 1920.0, 1.0);
        assert_eq!(x, -OVERLAY_SIZE.0 - OVERLAY_MARGIN);
        // 工作區比 overlay 還窄：貼左緣，不跑到螢幕外。
        let (x, _) = overlay_anchor_top_right(0.0, 0.0, 100.0, 1.0);
        assert_eq!(x, 0.0);
        // 荒謬的 scale 退回 1。
        let (x, _) = overlay_anchor_top_right(0.0, 0.0, 1000.0, 0.0);
        assert_eq!(x, 1000.0 - OVERLAY_SIZE.0 - OVERLAY_MARGIN);
    }

    #[test]
    fn host_refresh_triggers_on_safety_events_only() {
        assert!(host_safety_relevant(&EventType::EmergencyStop));
        assert!(host_safety_relevant(&EventType::SensorStarted));
        assert!(host_safety_relevant(&EventType::SensorStopped));
        // 停止結果未知：tray／overlay 必須立刻改口，不能等 4 秒輪詢。
        assert!(host_safety_relevant(&EventType::SensorStopUncertain));
        assert!(host_safety_relevant(&EventType::ProviderStateChanged));
        assert!(!host_safety_relevant(&EventType::ReceptorObservation));
        assert!(!host_safety_relevant(&EventType::PresentationCommand));
        assert!(!host_safety_relevant(&EventType::ActionCompleted));
    }

    #[test]
    fn companion_title_uses_the_configured_name() {
        assert_eq!(companion_window_title("小樞"), "小樞");
        assert_eq!(companion_window_title("  Mika  "), "Mika");
        assert_eq!(companion_window_title("   "), "桌面角色");
        assert_eq!(
            companion_window_title(&DesktopPrefs::default().companion_name),
            "小樞"
        );
    }

    #[test]
    fn character_hello_body_matches_the_http_contract() {
        let body = character_hello_body(
            None,
            None,
            json!({"characterId": "x"}),
            json!({"type": "negotiate"}),
            true,
            None,
            None,
        );
        let obj = body.as_object().expect("object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["manifest", "negotiate", "visible"]);
        assert_eq!(obj["visible"], json!(true));

        let body = character_hello_body(
            Some("desktop-companion".into()),
            Some("primary-companion".into()),
            json!({}),
            json!({}),
            false,
            Some("shu-maid".into()),
            Some(json!({"mode": "idle"})),
        );
        let obj = body.as_object().expect("object");
        for key in [
            "instanceId",
            "role",
            "manifest",
            "negotiate",
            "visible",
            "packId",
            "behaviorState",
        ] {
            assert!(obj.contains_key(key), "missing {key}");
        }
        assert_eq!(obj["instanceId"], json!("desktop-companion"));
        assert_eq!(obj["visible"], json!(false));
        assert_eq!(obj["behaviorState"]["mode"], json!("idle"));
    }
}
