//! Runtime integration tests: the full closed loop without HTTP.

use adapters_builtin::MockBehavior;
use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;
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

fn facts(pairs: &[(&str, &str)]) -> BTreeMap<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect()
}

#[tokio::test]
async fn scenario_a_single_receptor_single_actuator() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("test".into()), None, vec![])
        .await
        .unwrap();

    let mut intent = SemanticIntent::new("success");
    intent.preferred_channels = vec!["conversation".into()];
    let plan = rt
        .create_plan(
            intent,
            vec!["conversation".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(plan.steps.len(), 1);

    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert_eq!(receipts.len(), 1);
    let receipt = &receipts[0];
    assert_eq!(receipt.current_status, ActionStatus::Completed);
    // Honest verdict: completed via acknowledgement, not observation.
    assert_eq!(
        receipt.verification.as_ref().unwrap().verdict,
        VerificationVerdict::AcknowledgedOnly
    );
    // The message actually landed in the conversation outbox.
    let outbox = rt.outbox.recent(10);
    assert!(!outbox.is_empty());
    assert_eq!(outbox[0].channel, "conversation");
    assert!(outbox[0].text.is_some());
}

#[tokio::test]
async fn scenario_g_mock_device_full_state_machine() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("test".into()), None, vec!["channel:haptic".into()])
        .await
        .unwrap();
    // Allow the haptic channel + enable the mock actuator (defaults are safe-off).
    rt.update_policy(json!({"allowedChannels": ["conversation","web-ui","notification","log","visual","haptic"]}))
        .await
        .unwrap();
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("mock.actuator"), true)
        .await
        .unwrap();

    let mut intent = SemanticIntent::new("celebrate-progress");
    intent.magnitude = Some(0.9); // will be clamped by device limit 0.8
    intent.duration_ms = Some(2_000);
    intent.preferred_channels = vec!["haptic".into()];
    let mut metadata = BTreeMap::new();
    metadata.insert("verification".to_string(), json!("observed"));
    let plan = rt
        .create_plan(
            intent,
            vec!["mock.actuator".into()],
            1,
            1,
            false,
            None,
            metadata,
        )
        .await
        .unwrap();
    assert_eq!(plan.steps.len(), 1);

    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    let receipt = &receipts[0];
    // Full path: authorized → accepted → dispatched → acknowledged → observed → completed.
    let states: Vec<ActionStatus> = receipt.timestamps.iter().map(|(s, _)| *s).collect();
    assert_eq!(
        states,
        vec![
            ActionStatus::Authorized,
            ActionStatus::Accepted,
            ActionStatus::Dispatched,
            ActionStatus::Acknowledged,
            ActionStatus::Observed,
            ActionStatus::Completed,
        ]
    );
    assert_eq!(
        receipt.verification.as_ref().unwrap().verdict,
        VerificationVerdict::Observed
    );
    // Magnitude was clamped to the device safe limit.
    assert_eq!(receipt.effective_bounded_parameters.magnitude, Some(0.8));
    assert!(receipt
        .policy_decisions
        .iter()
        .any(|d| matches!(d, PolicyDecision::Clamped { .. })));
}

#[tokio::test]
async fn consent_required_blocks_then_grant_allows() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("test".into()), None, vec![])
        .await
        .unwrap();
    rt.update_policy(json!({"allowedChannels": ["conversation","haptic"]}))
        .await
        .unwrap();
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("mock.actuator"), true)
        .await
        .unwrap();

    let mut intent = SemanticIntent::new("presence");
    intent.preferred_channels = vec!["haptic".into()];
    let plan = rt
        .create_plan(
            intent.clone(),
            vec!["mock.actuator".into()],
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
    assert_eq!(receipts[0].current_status, ActionStatus::Blocked);
    assert!(receipts[0]
        .policy_decisions
        .iter()
        .any(|d| matches!(d, PolicyDecision::Blocked { rule, .. } if rule == "consent.required")));

    // Grant consent → allowed.
    rt.grant_consent("channel:haptic", None).await.unwrap();
    let plan2 = rt
        .create_plan(
            intent,
            vec!["mock.actuator".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let receipts2 = rt
        .execute_plan(&plan2.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert!(matches!(
        receipts2[0].current_status,
        ActionStatus::Completed | ActionStatus::Acknowledged | ActionStatus::Dispatched
    ));
}

#[tokio::test]
async fn scenario_d_offline_actuator_falls_back() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("test".into()), None, vec!["channel:haptic".into()])
        .await
        .unwrap();
    rt.update_policy(json!({"allowedChannels": ["conversation","haptic"]}))
        .await
        .unwrap();
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("mock.actuator"), true)
        .await
        .unwrap();

    let mut intent = SemanticIntent::new("warning");
    intent.preferred_channels = vec!["haptic".into(), "conversation".into()];
    let mut metadata = BTreeMap::new();
    metadata.insert("actuationMode".to_string(), json!("fallback"));
    let plan = rt
        .create_plan(
            intent,
            vec!["mock.actuator".into(), "conversation".into()],
            1,
            2,
            false,
            None,
            metadata,
        )
        .await
        .unwrap();
    assert_eq!(
        plan.steps[0].actuator_id.as_str(),
        "mock.actuator",
        "preferred first"
    );

    // Device goes offline between planning and execution.
    rt.mock_actuator.set_behavior(MockBehavior::Offline);

    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert_eq!(receipts.len(), 2);
    // First receipt honestly says the preferred actuator was NOT executed.
    assert_eq!(receipts[0].actuator_id.as_str(), "mock.actuator");
    assert_eq!(receipts[0].current_status, ActionStatus::Blocked);
    // Fallback succeeded.
    assert_eq!(receipts[1].actuator_id.as_str(), "conversation");
    assert_eq!(receipts[1].current_status, ActionStatus::Completed);
}

#[tokio::test]
async fn scenario_e_consent_revocation_cancels_and_emergency_stop() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("test".into()), None, vec!["channel:haptic".into()])
        .await
        .unwrap();

    // Emergency stop blocks everything and does not auto-resume.
    rt.emergency_stop("test", Some("drill".into()))
        .await
        .unwrap();
    assert!(rt.is_estopped());
    let err = rt
        .create_plan(
            SemanticIntent::new("presence"),
            vec![],
            0,
            1,
            true,
            None,
            BTreeMap::new(),
        )
        .await
        .map(|p| p.plan_id);
    // Plan creation is allowed (read-only-ish) but execution must fail.
    if let Ok(plan_id) = err {
        let exec = rt
            .execute_plan(&plan_id, ActionSource::ExplicitRequest, false)
            .await;
        assert!(matches!(exec, Err(DomainError::EmergencyStop)));
    }
    // Consents were revoked by the e-stop.
    let session = rt.current_session().await.unwrap();
    assert!(!session.has_consent(&ConsentScope::Channel("haptic".into()), chrono::Utc::now()));

    // Explicit clear re-arms.
    rt.clear_emergency_stop("test").await.unwrap();
    assert!(!rt.is_estopped());
}

#[tokio::test]
async fn recipe_autonomous_loop_fires_on_observations() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("test".into()), None, vec![])
        .await
        .unwrap();
    // Default recipe was seeded from assets.
    let recipes = rt.list_recipes().await;
    assert!(recipes
        .iter()
        .any(|(r, _)| r.id.as_str() == "adaptive-task-completion"));

    // Feed the trigger sequence: task completed, then user present.
    rt.ingest(
        "task.lifecycle",
        facts(&[("event", "task.completed")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    rt.ingest(
        "user.presence",
        facts(&[("state", "present")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();

    // The recipe fired autonomously and produced output on a low-risk channel.
    let outbox = rt.outbox.recent(10);
    assert!(
        !outbox.is_empty(),
        "recipe should have produced at least one message; got none"
    );
    // Cooldown: firing again immediately is suppressed.
    let before = rt.outbox.recent(100).len();
    rt.ingest(
        "task.lifecycle",
        facts(&[("event", "task.completed")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    rt.ingest(
        "user.presence",
        facts(&[("state", "present")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    assert_eq!(
        rt.outbox.recent(100).len(),
        before,
        "cooldown must suppress immediate re-fire"
    );
}

#[tokio::test]
async fn no_action_is_a_legitimate_outcome() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("test".into()), None, vec![])
        .await
        .unwrap();
    // Restrict candidates to a consent-gated actuator without granting consent,
    // with allowNoAction → the plan chooses no action instead of failing.
    let plan = rt
        .create_plan(
            SemanticIntent::new("presence"),
            vec!["nonexistent.actuator".into()],
            0,
            1,
            true,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(plan.status, PlanStatus::NoAction);
    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert!(receipts.is_empty());
}

#[tokio::test]
async fn crash_recovery_marks_open_actions_uncertain() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_path_buf();
    {
        let rt = Runtime::start(RuntimeOptions {
            home: Some(home.clone()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        rt.start_session(Some("crash".into()), None, vec![])
            .await
            .unwrap();
        // Simulate an open action left behind (no clean shutdown).
        let plan = rt
            .create_plan(
                SemanticIntent::new("progress"),
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
        // Simulate a genuinely in-flight action left behind by a crash: a
        // FRESH receipt (new action id) that never reached a terminal state.
        // (Terminal receipts are sticky and can no longer be regressed.)
        let mut open = receipts[0].clone();
        open.action_id = ActionId::generate();
        open.current_status = ActionStatus::Dispatched;
        open.timestamps = vec![
            (ActionStatus::Authorized, chrono::Utc::now()),
            (ActionStatus::Accepted, chrono::Utc::now()),
            (ActionStatus::Dispatched, chrono::Utc::now()),
        ];
        assert!(rt.store.upsert_receipt(&open, "conversation").unwrap());
        // NOTE: no rt.shutdown() → clean_shutdown stays "false".
    }
    let rt2 = Runtime::start(RuntimeOptions {
        home: Some(home),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    let receipts = rt2.list_actions(None, 10).unwrap();
    assert!(
        receipts.iter().all(|r| r.current_status.is_terminal()),
        "open actions must not survive a restart as runnable"
    );
    assert!(receipts
        .iter()
        .any(|r| r.current_status == ActionStatus::Uncertain));
}

#[tokio::test]
async fn simulate_has_no_side_effects() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("test".into()), None, vec![])
        .await
        .unwrap();
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
    let report = rt.simulate_plan(&plan.plan_id).await.unwrap();
    assert!(report.would_execute);
    assert!(
        rt.outbox.recent(10).is_empty(),
        "simulation must not produce output"
    );
    assert!(
        rt.list_actions(None, 10).unwrap().is_empty(),
        "simulation must not create receipts"
    );
}

#[tokio::test]
async fn recipe_events_are_consumed_no_refire_on_unrelated_observation() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("consume".into()), None, vec![])
        .await
        .unwrap();
    // Recipe WITHOUT cooldown: only event consumption prevents re-firing.
    rt.upsert_recipe_text(
        r#"
id: consume-test
name: consumption
enabled: true
trigger:
  mode: sequence
  within: 10m
  steps:
    - receptor: task.lifecycle
      condition: { event: task.completed }
    - receptor: user.presence
      condition: { state: present }
decision: { objective: t, allowNoAction: true }
intent: success
actuation:
  candidates: [conversation]
  minChannels: 0
  maxChannels: 1
"#,
    )
    .await
    .unwrap();
    // Disable the seeded default recipe so counts are isolated.
    let _ = rt
        .set_recipe_enabled("adaptive-task-completion", false)
        .await;

    rt.ingest(
        "task.lifecycle",
        facts(&[("event", "task.completed")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    rt.ingest(
        "user.presence",
        facts(&[("state", "present")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    let after_first = rt.outbox.recent(100).len();
    assert!(after_first >= 1, "recipe should have fired once");

    // A NEW presence event alone must NOT re-fire: the task.completed event
    // was consumed by the first firing.
    rt.ingest(
        "user.presence",
        facts(&[("state", "present")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    assert_eq!(
        rt.outbox.recent(100).len(),
        after_first,
        "consumed trigger event must not fire the recipe again"
    );

    // A fresh task.completed event re-arms the chain.
    rt.ingest(
        "task.lifecycle",
        facts(&[("event", "task.completed")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    rt.ingest(
        "user.presence",
        facts(&[("state", "present")]),
        BTreeMap::new(),
        1.0,
    )
    .await
    .unwrap();
    assert!(
        rt.outbox.recent(100).len() > after_first,
        "new evidence should fire the recipe again"
    );
}

#[tokio::test]
async fn recipe_state_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_path_buf();
    {
        let rt = Runtime::start(RuntimeOptions {
            home: Some(home.clone()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        rt.start_session(Some("persist".into()), None, vec![])
            .await
            .unwrap();
        rt.ingest(
            "task.lifecycle",
            facts(&[("event", "task.completed")]),
            BTreeMap::new(),
            1.0,
        )
        .await
        .unwrap();
        rt.ingest(
            "user.presence",
            facts(&[("state", "present")]),
            BTreeMap::new(),
            1.0,
        )
        .await
        .unwrap();
        let (_, state) = rt
            .list_recipes()
            .await
            .into_iter()
            .find(|(r, _)| r.id.as_str() == "adaptive-task-completion")
            .unwrap();
        assert!(state.last_fired_at.is_some(), "recipe should have fired");
        rt.shutdown().await;
    }
    // Restart: cooldown state must survive; the same session is still active.
    let rt2 = Runtime::start(RuntimeOptions {
        home: Some(home),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    let (_, state) = rt2
        .list_recipes()
        .await
        .into_iter()
        .find(|(r, _)| r.id.as_str() == "adaptive-task-completion")
        .unwrap();
    assert!(
        state.last_fired_at.is_some(),
        "cooldown state must survive a restart"
    );
    assert_eq!(
        state.executions_this_session, 1,
        "same session keeps its budget"
    );
}

#[tokio::test]
async fn monetary_spend_accumulates_and_blocks() {
    let (_g, rt) = runtime().await;
    let session = rt
        .start_session(Some("money".into()), None, vec![])
        .await
        .unwrap();
    assert_eq!(rt.current_session().await.unwrap().monetary_spent, 0.0);
    rt.charge_session_cost_public(&session.session_id, 1.25)
        .await;
    rt.charge_session_cost_public(&session.session_id, 0.50)
        .await;
    let updated = rt.current_session().await.unwrap();
    assert!((updated.monetary_spent - 1.75).abs() < f64::EPSILON);
    // Persisted too.
    let stored = rt.store.session(&session.session_id).unwrap();
    assert!((stored.monetary_spent - 1.75).abs() < f64::EPSILON);
}

#[tokio::test]
async fn emergency_clear_rearms_latched_devices() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("rearm".into()), None, vec!["channel:haptic".into()])
        .await
        .unwrap();
    rt.update_policy(json!({"allowedChannels":["conversation","haptic"]}))
        .await
        .unwrap();
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("mock.actuator"), true)
        .await
        .unwrap();

    rt.emergency_stop("test", None).await.unwrap();
    assert!(rt.mock_actuator.was_stopped(), "estop latches the device");
    rt.clear_emergency_stop("test").await.unwrap();
    // Consents were revoked by the estop; grant again (explicit human action).
    rt.grant_consent("channel:haptic", None).await.unwrap();

    let mut intent = SemanticIntent::new("presence");
    intent.preferred_channels = vec!["haptic".into()];
    let plan = rt
        .create_plan(
            intent,
            vec!["mock.actuator".into()],
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
    assert!(
        matches!(
            receipts[0].current_status,
            ActionStatus::Completed | ActionStatus::Acknowledged
        ),
        "after explicit clear the device must work again, got {:?} ({:?})",
        receipts[0].current_status,
        receipts[0].errors,
    );
}
