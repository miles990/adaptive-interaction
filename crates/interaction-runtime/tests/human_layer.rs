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
        .update_ui_preferences(
            json!({"mode": "advanced", "customNames": {"receptor:system.time": "我的時鐘"}}),
        )
        .await
        .unwrap();
    assert_eq!(updated.mode, "advanced");

    // Invalid mode refused.
    let err = rt.update_ui_preferences(json!({"mode": "wizard"})).await;
    assert!(err.is_err());

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
        .resolve_ai_assist(&request_id, "maybe", None)
        .await
        .is_err());
    assert_eq!(rt.pending_ai_assists().await.len(), 1);

    let out = rt
        .resolve_ai_assist(&request_id, "proceed", Some("user seems present".into()))
        .await
        .unwrap();
    assert_eq!(out["outcome"]["result"], json!("fired"));
    assert!(receipts_count(&rt).await > 0);
    // Second resolution of the same request is refused (already claimed).
    assert!(rt
        .resolve_ai_assist(&request_id, "proceed", None)
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
