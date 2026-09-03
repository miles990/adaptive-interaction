//! Integration tests for the human layer: proactive pause (≠ emergency stop),
//! UI preferences, onboarding draft/commit, human capability projection,
//! AI-assisted descriptions, and the recipe AI decision gate.

use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::human::{OnboardingCommit, SimScenario};
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::collections::BTreeMap;

async fn runtime() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    (dir, rt)
}

fn facts(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect()
}

const FIRE_RECIPE: &str = r#"
id: fire-on-task
name: fire on task completed
trigger:
  mode: single
  steps:
    - receptor: task.lifecycle
      condition:
        event: task.completed
decision:
  objective: test
  allowNoAction: false
intent: success
actuation:
  mode: single
  candidates: [conversation]
  minChannels: 1
  maxChannels: 1
"#;

async fn receipts_count(rt: &Runtime) -> usize {
    rt.list_actions(None, 100).map(|r| r.len()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Pause vs emergency stop
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pause_blocks_recipes_but_not_explicit_requests() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.upsert_recipe_text(FIRE_RECIPE).await.unwrap();

    rt.pause_proactive(None, Some("focus time".into()), "test")
        .await
        .unwrap();
    assert!(rt.pause_status().await.paused);
    // Emergency stop is a different state and is NOT engaged.
    assert!(!rt.is_estopped());

    // A matching event does not fire the recipe while paused.
    rt.ingest(
        "task.lifecycle",
        facts(&[("event", "task.completed")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    assert_eq!(receipts_count(&rt).await, 0, "paused: recipe must not fire");

    // But an explicit user/AI request still executes (pause ≠ estop).
    let plan = rt
        .create_plan(
            SemanticIntent::new("success"),
            vec!["conversation".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert_eq!(receipts[0].current_status, ActionStatus::Completed);

    // Resume → the next event fires.
    rt.resume_proactive("test").await.unwrap();
    let before = receipts_count(&rt).await;
    rt.ingest(
        "task.lifecycle",
        facts(&[("event", "task.completed")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    assert!(
        receipts_count(&rt).await > before,
        "after resume the recipe must fire again"
    );
}

#[tokio::test]
async fn pause_expires_lazily_and_persists_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let rt = Runtime::start(RuntimeOptions {
            home: Some(dir.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        rt.pause_proactive(None, None, "test").await.unwrap();
        rt.shutdown().await;
    }
    // Pause survives a restart (a UI crash must not silently re-enable
    // proactivity).
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    assert!(rt.pause_status().await.paused, "pause must survive restart");

    // A pause with an already-elapsed window auto-clears on next read.
    rt.resume_proactive("test").await.unwrap();
    let until = chrono::Utc::now() + chrono::Duration::milliseconds(50);
    rt.pause_proactive(Some(until), None, "test").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert!(!rt.pause_status().await.paused, "elapsed pause auto-clears");
}

// ---------------------------------------------------------------------------
// UI preferences
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ui_preferences_persist_and_validate() {
    let (_g, rt) = runtime().await;
    let prefs = rt.ui_preferences().await;
    assert_eq!(prefs.mode, "simple", "simple mode is the default");
    assert_eq!(prefs.locale, "zh-TW");

    let updated = rt
        .update_ui_preferences(json!({
            "mode": "advanced",
            "appearance": "light",
            "scalePercent": 120,
            "reduceMotion": true,
                "disabledAgents": ["claude-code"],
                "agentRoutes": {"programming": "claude-code"},
            "customNames": {"receptor:system.time": "我的時鐘"}
        }))
        .await
        .unwrap();
    assert_eq!(updated.mode, "advanced");
    assert_eq!(updated.appearance, "light");
    assert_eq!(updated.scale_percent, 120);
    assert!(updated.reduce_motion);
    assert_eq!(updated.disabled_agents, ["claude-code"]);
    assert_eq!(updated.agent_routes["programming"], "claude-code");
    let route = rt.agent_route_suggestion(Some("code")).await;
    assert_eq!(route["role"], "programming");
    assert_eq!(route["suggestion"], "claude-code");

    // Invalid mode refused.
    let err = rt.update_ui_preferences(json!({"mode": "wizard"})).await;
    assert!(err.is_err());
    assert!(rt
        .update_ui_preferences(json!({"scalePercent": 500}))
        .await
        .is_err());
    assert!(rt
        .update_ui_preferences(json!({"disabledAgents": ["not-a-provider"]}))
        .await
        .is_err());

    // Custom name feeds the human projection as a user override.
    let caps = rt.human_capabilities("zh-TW", true).await;
    let card = caps["receptors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "system.time")
        .unwrap()
        .clone();
    assert_eq!(card["displayName"], "我的時鐘");
    assert_eq!(card["nameSource"], "user");
}

// ---------------------------------------------------------------------------
// Human capability projection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn projection_uses_catalog_and_conservative_fallback() {
    let (_g, rt) = runtime().await;
    let caps = rt.human_capabilities("zh-TW", true).await;
    let actuators = caps["actuators"].as_array().unwrap();
    let conv = actuators
        .iter()
        .find(|c| c["id"] == "conversation")
        .unwrap();
    // Builtin adapters carry their own descriptions; catalog supplies the
    // display name for well-known ids when the adapter has no localized one.
    assert_eq!(conv["displayName"], "對話訊息");
    assert!(conv["manifestHash"].as_str().unwrap().len() == 16);

    // An unknown dynamic receptor gets the deterministic fallback + notice.
    rt.add_push_receptor("vendor.mystery.sensor", "", "misc", false)
        .await
        .unwrap();
    let caps = rt.human_capabilities("zh-TW", true).await;
    let card = caps["receptors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "vendor.mystery.sensor")
        .unwrap()
        .clone();
    assert_eq!(card["displayName"], "Vendor · Mystery · Sensor");
    // Dynamic push receptors carry an adapter description, so they are not
    // "undescribed"; but their data flow stays honest.
    assert!(
        card["data"]["leavesDevice"] == json!("unknown")
            || card["data"]["leavesDevice"] == json!(false)
    );
}

// ---------------------------------------------------------------------------
// AI-assisted descriptions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ai_description_bound_to_manifest_hash() {
    let (_g, rt) = runtime().await;
    let caps = rt.human_capabilities("zh-TW", true).await;
    let conv = caps["actuators"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "conversation")
        .unwrap()
        .clone();
    let hash = conv["manifestHash"].as_str().unwrap().to_string();

    // Wrong hash → refused (stale manifest view).
    let err = rt
        .set_capability_ai_description("actuator", "conversation", "zh-TW", "x", "deadbeefdeadbeef")
        .await;
    assert!(matches!(err, Err(DomainError::Conflict(_))));

    // Correct hash → stored, surfaced in the projection.
    rt.set_capability_ai_description(
        "actuator",
        "conversation",
        "zh-TW",
        "用日常對話的方式回覆你，例如完成工作時輕聲說一句。",
        &hash,
    )
    .await
    .unwrap();
    let caps = rt.human_capabilities("zh-TW", true).await;
    let conv = caps["actuators"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "conversation")
        .unwrap()
        .clone();
    assert!(conv["aiDescription"].as_str().unwrap().contains("日常對話"));
    // The AI text never touches resolved facts.
    assert_eq!(conv["effect"]["externalSideEffect"], json!(false));
}

// ---------------------------------------------------------------------------
// Onboarding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn onboarding_draft_and_commit() {
    let (_g, rt) = runtime().await;
    let state = rt.onboarding_state().await;
    assert_eq!(state["completed"], json!(false));
    assert!(state["starterRecipes"].as_array().unwrap().len() >= 3);

    rt.save_onboarding_draft(json!({"step": 3, "senses": ["task.lifecycle"]}))
        .await
        .unwrap();
    let state = rt.onboarding_state().await;
    assert_eq!(state["draft"]["step"], json!(3));

    // Unknown component id → whole commit refused, nothing applied.
    let bad = OnboardingCommit {
        enable_actuators: vec!["no.such.actuator".into()],
        ..Default::default()
    };
    assert!(rt.commit_onboarding(bad).await.is_err());
    assert_eq!(rt.onboarding_state().await["completed"], json!(false));

    // Valid commit: installs a starter recipe, tightens policy, completes.
    let commit = OnboardingCommit {
        starter_recipes: vec!["starter-task-complete".into()],
        policy_patch: Some(json!({"initiative": "suggest"})),
        preferences: Some(json!({"locale": "zh-TW"})),
        ..Default::default()
    };
    let result = rt.commit_onboarding(commit).await.unwrap();
    assert_eq!(result["completed"], json!(true));
    let state = rt.onboarding_state().await;
    assert_eq!(state["completed"], json!(true));
    assert!(state["draft"].is_null(), "commit clears the draft");
    assert!(rt.get_recipe("starter-task-complete").await.is_ok());
}

// ---------------------------------------------------------------------------
// AI decision gate
// ---------------------------------------------------------------------------

const AI_RECIPE_NO_ACTION: &str = r#"
id: ai-gated
name: ai gated recipe
trigger:
  mode: single
  steps:
    - receptor: user.presence
      condition:
        state: present
decision:
  objective: assist
  allowNoAction: false
intent: presence
actuation:
  mode: single
  candidates: [conversation]
  minChannels: 1
  maxChannels: 1
ai:
  mode: when-uncertain
  minConfidence: 0.9
  maxWaitMs: 100
  onUnavailable: no-action
"#;

#[tokio::test]
async fn deterministic_events_never_call_ai() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.upsert_recipe_text(AI_RECIPE_NO_ACTION).await.unwrap();

    // High-confidence fact → gate says "not needed", fires deterministically.
    rt.ingest(
        "user.presence",
        facts(&[("state", "present")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    assert!(
        receipts_count(&rt).await > 0,
        "unambiguous event fires without AI"
    );
    assert!(
        rt.pending_ai_assists().await.is_empty(),
        "no assist request for a deterministic event"
    );
}

#[tokio::test]
async fn uncertain_evidence_defers_to_ai_no_action_fallback() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.upsert_recipe_text(AI_RECIPE_NO_ACTION).await.unwrap();

    // Low-confidence inference-driven event → deferred to AI.
    let mut inf = BTreeMap::new();
    inf.insert("possibleState".to_string(), json!("away"));
    rt.ingest("user.presence", facts(&[("state", "present")]), inf, 0.3)
        .await
        .unwrap();
    assert_eq!(receipts_count(&rt).await, 0, "uncertain: must not fire yet");
    assert_eq!(rt.pending_ai_assists().await.len(), 1, "assist requested");

    // No AI answers → onUnavailable=no-action → still nothing fired.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(receipts_count(&rt).await, 0, "no-action fallback holds");
    assert!(rt.pending_ai_assists().await.is_empty(), "request expired");
}

#[tokio::test]
async fn ai_unavailable_fallback_fires_deterministic_plan() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let recipe = AI_RECIPE_NO_ACTION.replace("onUnavailable: no-action", "onUnavailable: fallback");
    rt.upsert_recipe_text(&recipe).await.unwrap();

    let mut inf = BTreeMap::new();
    inf.insert("possibleState".to_string(), json!("away"));
    rt.ingest("user.presence", facts(&[("state", "present")]), inf, 0.3)
        .await
        .unwrap();
    assert_eq!(receipts_count(&rt).await, 0, "deferred first");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        receipts_count(&rt).await > 0,
        "fallback fires the deterministic plan after the AI wait elapses"
    );
}

#[tokio::test]
async fn resolving_assist_proceed_fires_once() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    // Long wait so the timeout doesn't race the explicit resolution.
    let recipe = AI_RECIPE_NO_ACTION.replace("maxWaitMs: 100", "maxWaitMs: 30000");
    rt.upsert_recipe_text(&recipe).await.unwrap();

    let mut inf = BTreeMap::new();
    inf.insert("possibleState".to_string(), json!("away"));
    rt.ingest("user.presence", facts(&[("state", "present")]), inf, 0.3)
        .await
        .unwrap();
    let pending = rt.pending_ai_assists().await;
    assert_eq!(pending.len(), 1);
    let request_id = pending[0].request_id.clone();

    // Invalid decision refused, request stays pending.
    assert!(rt
        .resolve_ai_assist(&request_id, "maybe", None, false)
        .await
        .is_err());
    assert_eq!(rt.pending_ai_assists().await.len(), 1);

    let out = rt
        .resolve_ai_assist(
            &request_id,
            "proceed",
            Some("user seems present".into()),
            false,
        )
        .await
        .unwrap();
    assert_eq!(out["outcome"]["result"], json!("fired"));
    assert!(receipts_count(&rt).await > 0);
    // Second resolution of the same request is refused (already claimed).
    assert!(rt
        .resolve_ai_assist(&request_id, "proceed", None, false)
        .await
        .is_err());
}

// ---------------------------------------------------------------------------
// Scenario simulation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_simulation_reports_without_side_effects() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.upsert_recipe_text(FIRE_RECIPE).await.unwrap();

    let before = receipts_count(&rt).await;
    // Quiet-hours scenario: conversation is not a quiet-silenced channel, so
    // the plan still authorizes — but the report must show the injected state.
    let report = rt
        .simulate_recipe_scenario(
            "fire-on-task",
            SimScenario {
                quiet_hours: true,
                event: Some(
                    serde_json::from_value(json!({
                        "receptor": "task.lifecycle",
                        "facts": {"event": "task.completed"}
                    }))
                    .unwrap(),
                ),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(report["scenario"]["quietHours"], json!(true));
    assert_eq!(report["stages"][0]["stage"], json!("trigger"));
    assert_eq!(
        report["stages"][0]["ok"],
        json!(true),
        "synthetic event matches"
    );
    assert_eq!(
        receipts_count(&rt).await,
        before,
        "simulation has NO side effects"
    );

    // Emergency-stop scenario blocks at policy.
    let report = rt
        .simulate_recipe_scenario(
            "fire-on-task",
            SimScenario {
                emergency_stop: true,
                event: Some(
                    serde_json::from_value(json!({
                        "receptor": "task.lifecycle",
                        "facts": {"event": "task.completed"}
                    }))
                    .unwrap(),
                ),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(report["wouldExecute"], json!(false));

    // Recently-fired scenario trips the cooldown stage.
    let cooldown_recipe = FIRE_RECIPE.to_string() + "\nlimits:\n  cooldown: 15m\n";
    rt.upsert_recipe_text(&cooldown_recipe).await.unwrap();
    let report = rt
        .simulate_recipe_scenario(
            "fire-on-task",
            SimScenario {
                recently_fired: true,
                event: Some(
                    serde_json::from_value(json!({
                        "receptor": "task.lifecycle",
                        "facts": {"event": "task.completed"}
                    }))
                    .unwrap(),
                ),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let limits_stage = report["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["stage"] == "limits")
        .unwrap();
    assert_eq!(limits_stage["ok"], json!(false));
    assert_eq!(report["wouldExecute"], json!(false));
}

#[tokio::test]
async fn recipe_summary_derives_from_structure() {
    let (_g, rt) = runtime().await;
    rt.upsert_recipe_text(FIRE_RECIPE).await.unwrap();
    let summary = rt.recipe_summary("fire-on-task", "zh-TW").await.unwrap();
    // Display names resolved through the same projection the UI uses.
    assert!(summary.contains("任務狀態"), "{summary}");
    assert!(summary.contains("不需要 AI"), "{summary}");
}

// ---------------------------------------------------------------------------
// Review-hardening regressions
// ---------------------------------------------------------------------------

const AI_RECIPE_FALLBACK: &str = r#"
id: ai-gated-fb
name: ai gated fallback
trigger:
  mode: single
  steps:
    - receptor: user.presence
      condition:
        state: present
decision:
  objective: assist
  allowNoAction: false
intent: presence
actuation:
  mode: single
  candidates: [conversation]
  minChannels: 1
  maxChannels: 1
ai:
  mode: when-uncertain
  minConfidence: 0.9
  maxWaitMs: 150
  onUnavailable: fallback
"#;

#[tokio::test]
async fn assist_fallback_respects_recipe_disabled_during_wait() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.upsert_recipe_text(AI_RECIPE_FALLBACK).await.unwrap();

    let mut inf = BTreeMap::new();
    inf.insert("possibleState".to_string(), json!("away"));
    rt.ingest("user.presence", facts(&[("state", "present")]), inf, 0.3)
        .await
        .unwrap();
    assert_eq!(rt.pending_ai_assists().await.len(), 1);
    // The user disables the recipe while the assist is pending.
    rt.set_recipe_enabled("ai-gated-fb", false).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    assert_eq!(
        receipts_count(&rt).await,
        0,
        "a recipe disabled during the assist window must NOT fire on fallback"
    );
}

#[tokio::test]
async fn assist_requires_human_confirmation_gates_api_resolution() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let recipe = AI_RECIPE_FALLBACK
        .replace("id: ai-gated-fb", "id: ai-gated-hc")
        .replace("maxWaitMs: 150", "maxWaitMs: 30000")
        + "  requireHumanConfirmation: true\n";
    rt.upsert_recipe_text(&recipe).await.unwrap();

    let mut inf = BTreeMap::new();
    inf.insert("possibleState".to_string(), json!("away"));
    rt.ingest("user.presence", facts(&[("state", "present")]), inf, 0.3)
        .await
        .unwrap();
    let pending = rt.pending_ai_assists().await;
    assert_eq!(pending.len(), 1);
    let id = pending[0].request_id.clone();

    // AI surface (human_confirmed=false) cannot self-approve.
    let err = rt.resolve_ai_assist(&id, "proceed", None, false).await;
    assert!(
        matches!(err, Err(DomainError::ApprovalRequired(_))),
        "{err:?}"
    );
    assert_eq!(
        rt.pending_ai_assists().await.len(),
        1,
        "request stays pending"
    );
    assert_eq!(receipts_count(&rt).await, 0);

    // Human surface (desktop IPC) can.
    let out = rt
        .resolve_ai_assist(&id, "proceed", None, true)
        .await
        .unwrap();
    assert_eq!(out["outcome"]["result"], json!("fired"));
    assert!(receipts_count(&rt).await > 0);
}

#[tokio::test]
async fn assist_requiring_human_confirmation_never_autofires_on_timeout() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let recipe = AI_RECIPE_FALLBACK.replace("id: ai-gated-fb", "id: ai-gated-hc2")
        + "  requireHumanConfirmation: true\n";
    rt.upsert_recipe_text(&recipe).await.unwrap();

    let mut inf = BTreeMap::new();
    inf.insert("possibleState".to_string(), json!("away"));
    rt.ingest("user.presence", facts(&[("state", "present")]), inf, 0.3)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(
        receipts_count(&rt).await,
        0,
        "onUnavailable=fallback must downgrade to no-action when human confirmation is required"
    );
}

/// Find the `components` entry of a commit result's `applied` list.
fn components_step(result: &Value) -> Value {
    result["applied"]
        .as_array()
        .expect("applied list")
        .iter()
        .find(|step| step["step"] == json!("components"))
        .expect("components step")
        .clone()
}

/// How many `receptor.offline` events this receptor has produced so far.
fn offline_events(rt: &Runtime, receptor_id: &str) -> usize {
    rt.events
        .recent(500)
        .into_iter()
        .filter(|e| {
            e.event_type == EventType::ReceptorOffline
                && e.payload.get("receptorId").and_then(Value::as_str) == Some(receptor_id)
        })
        .count()
}

fn availability_of(rt_human: &Value, kind: &str, id: &str) -> String {
    rt_human[kind]
        .as_array()
        .expect("cards")
        .iter()
        .find(|card| card["id"] == json!(id))
        .unwrap_or_else(|| panic!("{kind} {id} missing"))["availability"]
        .as_str()
        .expect("availability")
        .to_string()
}

/// The wizard's 「套用前確認」 dialog is built from this preview, so it must
/// report the real before/after state and change absolutely nothing.
#[tokio::test]
async fn onboarding_preview_reports_changes_without_touching_anything() {
    let (_g, rt) = runtime().await;
    rt.add_push_receptor("probe.motion", "Probe motion", "device", false)
        .await
        .unwrap();
    let commit = OnboardingCommit {
        disable_receptors: vec!["probe.motion".into()],
        starter_recipes: vec!["starter-task-complete".into()],
        preferences: Some(json!({"locale": "zh-TW"})),
        ..Default::default()
    };
    let preview = rt.preview_onboarding(commit).await.unwrap();
    assert_eq!(preview["receptors"][0]["id"], json!("probe.motion"));
    assert_eq!(preview["receptors"][0]["from"], json!("on"));
    assert_eq!(preview["receptors"][0]["to"], json!("off"));
    assert_eq!(preview["receptors"][0]["changed"], json!(true));
    assert_eq!(preview["changed"], json!(true));
    assert_eq!(
        preview["starterRecipes"][0]["id"],
        json!("starter-task-complete")
    );
    assert_eq!(
        preview["starterRecipes"][0]["exists"],
        json!(false),
        "not installed yet"
    );

    // Nothing happened: the receptor is still on, no recipe was installed,
    // onboarding is still incomplete.
    let human = rt.human_capabilities("zh-TW", true).await;
    assert_eq!(
        availability_of(&human, "receptors", "probe.motion"),
        "available"
    );
    assert!(rt.get_recipe("starter-task-complete").await.is_err());
    assert_eq!(rt.onboarding_state().await["completed"], json!(false));

    // An unknown id is refused exactly like commit refuses it.
    let bad = OnboardingCommit {
        enable_receptors: vec!["no.such.receptor".into()],
        ..Default::default()
    };
    assert!(matches!(
        rt.preview_onboarding(bad).await,
        Err(DomainError::NotFound(_))
    ));
}

/// Re-running onboarding must not re-apply state a component is already in:
/// a no-op `set_*_enabled` would emit a misleading online/offline event and
/// count as a change the user never approved.
#[tokio::test]
async fn onboarding_commit_skips_unchanged_components() {
    let (_g, rt) = runtime().await;
    rt.add_push_receptor("probe.motion", "Probe motion", "device", false)
        .await
        .unwrap();
    let disable = || OnboardingCommit {
        disable_receptors: vec!["probe.motion".into()],
        ..Default::default()
    };

    let first = rt.commit_onboarding(disable()).await.unwrap();
    let step = components_step(&first);
    assert_eq!(step["receptors"][0]["changed"], json!(true));
    assert_eq!(step["receptorsChanged"], json!(["probe.motion"]));
    let human = rt.human_capabilities("zh-TW", true).await;
    assert_eq!(
        availability_of(&human, "receptors", "probe.motion"),
        "disabled"
    );

    // Second run: already off → reported as unchanged and left alone. No
    // second offline event may be emitted for a state that did not change.
    let offline_before = offline_events(&rt, "probe.motion");
    let second = rt.commit_onboarding(disable()).await.unwrap();
    let step = components_step(&second);
    assert_eq!(step["receptors"][0]["from"], json!("off"));
    assert_eq!(step["receptors"][0]["changed"], json!(false));
    assert_eq!(step["receptorsChanged"], json!([]));
    assert_eq!(
        offline_events(&rt, "probe.motion"),
        offline_before,
        "an unchanged component must not emit another offline event"
    );

    // Enabling something already enabled is a no-op too.
    let third = rt
        .commit_onboarding(OnboardingCommit {
            enable_receptors: vec!["task.lifecycle".into()],
            ..Default::default()
        })
        .await
        .unwrap();
    let step = components_step(&third);
    assert_eq!(step["receptors"][0]["from"], json!("on"));
    assert_eq!(step["receptors"][0]["changed"], json!(false));
    assert_eq!(step["receptorsChanged"], json!([]));
}

/// A re-run that installs no starter recipe must not overwrite an automation
/// the user has since edited.
#[tokio::test]
async fn onboarding_rerun_keeps_edited_starter_recipe() {
    let (_g, rt) = runtime().await;
    rt.commit_onboarding(OnboardingCommit {
        starter_recipes: vec!["starter-task-complete".into()],
        ..Default::default()
    })
    .await
    .unwrap();
    // Preview now honestly says installing again would overwrite it.
    let preview = rt
        .preview_onboarding(OnboardingCommit {
            starter_recipes: vec!["starter-task-complete".into()],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(preview["starterRecipes"][0]["exists"], json!(true));

    // The user edits the recipe (same body, their own name), then re-runs the
    // wizard without starters.
    let base = interaction_runtime::human::starter_recipes()
        .into_iter()
        .find(|(id, _, _)| *id == "starter-task-complete")
        .expect("starter recipe")
        .2;
    let mine = "任務完成提醒（我改過）";
    let yaml = base
        .lines()
        .map(|line| {
            if line.starts_with("name: ") {
                format!("name: {mine}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    rt.upsert_recipe_text(&yaml).await.unwrap();
    rt.commit_onboarding(OnboardingCommit::default())
        .await
        .unwrap();
    assert_eq!(
        rt.get_recipe("starter-task-complete").await.unwrap().name,
        mine,
        "a re-run without starters must not overwrite the edited recipe"
    );
}

#[tokio::test]
async fn onboarding_commit_refuses_consent_gated_components() {
    let (_g, rt) = runtime().await;
    rt.add_push_receptor("camera.fake", "Fake camera", "sensor", true)
        .await
        .unwrap();
    let commit = OnboardingCommit {
        enable_receptors: vec!["camera.fake".into()],
        ..Default::default()
    };
    let err = rt.commit_onboarding(commit).await;
    assert!(
        matches!(err, Err(DomainError::ConsentRequired(_))),
        "{err:?}"
    );
    assert_eq!(rt.onboarding_state().await["completed"], json!(false));
    // The mock actuator is consent-gated too.
    let commit = OnboardingCommit {
        enable_actuators: vec!["mock.actuator".into()],
        ..Default::default()
    };
    assert!(rt.commit_onboarding(commit).await.is_err());
}

#[tokio::test]
async fn recipe_files_follow_the_id_not_the_filename() {
    let dir = tempfile::tempdir().unwrap();
    let recipes_dir = dir.path().join("config/recipes");
    std::fs::create_dir_all(&recipes_dir).unwrap();
    // A recipe backed by a differently-named .yml file.
    std::fs::write(
        recipes_dir.join("my-old-name.yml"),
        FIRE_RECIPE.replace("fire on task completed", "v1"),
    )
    .unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    assert!(rt.get_recipe("fire-on-task").await.is_ok());
    // Editing rewrites {id}.yaml AND removes the stale backing file.
    rt.upsert_recipe_text(&FIRE_RECIPE.replace("fire on task completed", "v2"))
        .await
        .unwrap();
    assert!(
        !recipes_dir.join("my-old-name.yml").exists(),
        "stale file forked"
    );
    assert!(recipes_dir.join("fire-on-task.yaml").exists());
    // Removing deletes every backing file — no resurrection on restart.
    rt.remove_recipe("fire-on-task").await.unwrap();
    assert!(!recipes_dir.join("fire-on-task.yaml").exists());
    rt.shutdown().await;
    let rt2 = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    assert!(
        rt2.get_recipe("fire-on-task").await.is_err(),
        "recipe resurrected"
    );
}

#[tokio::test]
async fn builtin_manifests_declare_honest_semantics() {
    let (_g, rt) = runtime().await;
    let caps = rt.human_capabilities("zh-TW", true).await;
    let conv = caps["actuators"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "conversation")
        .unwrap()
        .clone();
    // Builtin adapters now formally declare their effect semantics.
    assert_eq!(conv["effect"]["confirmationLevel"], json!("delivered"));
    assert_eq!(conv["effect"]["interruptiveness"], json!("low"));
    let time = caps["receptors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "system.time")
        .unwrap()
        .clone();
    assert_eq!(time["data"]["leavesDevice"], json!(false));
    // Webhook honestly declares its external side effect.
    let webhook = caps["actuators"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "webhook.output")
        .unwrap()
        .clone();
    assert_eq!(webhook["effect"]["externalSideEffect"], json!(true));
}

/// 首次成功體驗（FirstSuccess）的「看過」旗標必須真的保存：PATCH 合併、GET 回傳，
/// 不改動其他偏好；預設 false。前端偵測到沒回傳就會退回 localStorage，所以這裡
/// 直接釘住 host 有保存。
#[tokio::test]
async fn first_success_seen_persists_through_ui_preferences() {
    let (_g, rt) = runtime().await;
    assert!(
        !rt.ui_preferences().await.first_success_seen,
        "never seen by default"
    );
    let updated = rt
        .update_ui_preferences(json!({"firstSuccessSeen": true}))
        .await
        .unwrap();
    assert!(updated.first_success_seen);
    // GET 回傳同一個值，其他偏好不受影響。
    let again = rt.ui_preferences().await;
    assert!(again.first_success_seen);
    assert_eq!(again.mode, "simple");
    // 之後只改別的欄位：旗標保留（merge，不是整份覆蓋）。
    let after = rt
        .update_ui_preferences(json!({"mode": "advanced"}))
        .await
        .unwrap();
    assert!(after.first_success_seen);
    assert_eq!(after.mode, "advanced");
    // camelCase 序列化名稱是前端契約的一部分。
    let raw = serde_json::to_value(&after).unwrap();
    assert_eq!(raw["firstSuccessSeen"], json!(true));
    // 型別錯誤要被拒絕，不能悄悄變成 true。
    assert!(rt
        .update_ui_preferences(json!({"firstSuccessSeen": "yes"}))
        .await
        .is_err());
    assert!(rt.ui_preferences().await.first_success_seen);
}

/// regression ia-settings-018：commit 不是原子的（政策已寫進磁碟、能力已經上線，
/// 後面的步驟才失敗），但錯誤原本只是一句話，精靈就只顯示「套用失敗」——
/// 使用者被告知什麼都沒發生，實際上一半已經生效。錯誤必須逐步說清楚。
#[tokio::test]
async fn onboarding_commit_failure_reports_which_steps_applied() {
    let (_g, rt) = runtime().await;
    let before = rt.policy().await.initiative;
    assert_ne!(
        format!("{before:?}"),
        "Passive",
        "前置條件：預設不是 passive"
    );

    // 政策 → 自動互動都會成功，最後一步（偏好設定）驗證失敗。
    let commit = OnboardingCommit {
        policy_patch: Some(json!({"initiative": "passive"})),
        starter_recipes: vec!["starter-task-complete".into()],
        preferences: Some(json!({"mode": "not-a-mode"})),
        ..Default::default()
    };
    let err = rt
        .commit_onboarding(commit)
        .await
        .expect_err("最後一步失敗");
    let message = err.to_string();

    // 誠實：說出已套用的、失敗的、還沒套用的，且不假裝可以還原。
    assert!(message.contains("已套用"), "{message}");
    assert!(message.contains("安全規則"), "{message}");
    assert!(message.contains("自動互動"), "{message}");
    assert!(message.contains("偏好設定"), "{message}");
    assert!(message.contains("完成設定"), "{message}");
    assert!(message.contains("不會自動還原"), "{message}");
    // 錯誤種類不變（HTTP 狀態碼與 code 保持原樣）。
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");

    // 而且真的是半套用：政策已經改了，但 onboarding 尚未標記完成。
    assert_eq!(format!("{:?}", rt.policy().await.initiative), "Passive");
    assert!(rt.get_recipe("starter-task-complete").await.is_ok());
    assert_eq!(rt.onboarding_state().await["completed"], json!(false));

    // 半套用的事實也要進稽核軌跡，之後查得回來。
    let audit = rt.store.audit_tail(50).unwrap_or_default();
    assert!(
        audit
            .iter()
            .any(|entry| entry["kind"] == json!("onboarding.partial")),
        "半套用要留下稽核紀錄：{audit:?}"
    );
}
