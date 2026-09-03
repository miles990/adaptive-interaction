//! Runtime integration tests: the full closed loop without HTTP.

use adapters_builtin::MockBehavior;
use async_trait::async_trait;
use interaction_adapter_sdk::{ActuatorManifestBuilder, DriverReceipt};
use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct PaidActuator {
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl Actuator for PaidActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new("paid.test", "Paid test", "paid", "test")
            .risk(RiskClass::Low)
            .cost(CostDescriptor {
                monetary_per_invocation: 1.0,
                resource: 0.0,
            })
            .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Ok(DriverReceipt::start(&action, chrono::Utc::now())
            .dispatched()
            .acknowledged()
            .finish())
    }

    async fn status(&self) -> ComponentHealth {
        ComponentHealth::healthy()
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(action_id.to_string()))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        Ok(())
    }
}

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
async fn disabled_recipe_cannot_be_run_directly() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("disabled-recipe".into()), None, vec![])
        .await
        .unwrap();
    rt.upsert_recipe_text(
        r#"
id: disabled-direct-run
name: disabled direct run
enabled: false
trigger:
  mode: any
  steps:
    - receptor: manual.event
decision: { objective: test, allowNoAction: true }
intent: success
actuation:
  candidates: [conversation]
  minChannels: 0
  maxChannels: 1
"#,
    )
    .await
    .unwrap();

    let error = rt.run_recipe("disabled-direct-run").await.unwrap_err();
    assert!(matches!(error, DomainError::PolicyBlocked(_)));
    assert!(rt.outbox.recent(10).is_empty());
}

#[tokio::test]
async fn concurrent_execute_of_one_plan_dispatches_only_once() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("one-plan".into()), None, vec![])
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

    let (left, right) = tokio::join!(
        rt.execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false),
        rt.execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false),
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert_eq!(rt.outbox.recent(10).len(), 1);
}

#[tokio::test]
async fn concurrent_authorizations_respect_hourly_limit() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("serialized-governor".into()), None, vec![])
        .await
        .unwrap();
    rt.update_policy(json!({
        "channelLimits": {
            "conversation": {"enabled": true, "maxPerHour": 1}
        }
    }))
    .await
    .unwrap();

    let mut plans = Vec::new();
    for _ in 0..4 {
        plans.push(
            rt.create_plan(
                SemanticIntent::new("success"),
                vec!["conversation".into()],
                1,
                1,
                false,
                None,
                BTreeMap::new(),
            )
            .await
            .unwrap(),
        );
    }
    let (a, b, c, d) = tokio::join!(
        rt.execute_plan(&plans[0].plan_id, ActionSource::ExplicitRequest, false),
        rt.execute_plan(&plans[1].plan_id, ActionSource::ExplicitRequest, false),
        rt.execute_plan(&plans[2].plan_id, ActionSource::ExplicitRequest, false),
        rt.execute_plan(&plans[3].plan_id, ActionSource::ExplicitRequest, false),
    );
    let results = [a, b, c, d];
    let dispatched = results
        .into_iter()
        .flat_map(Result::unwrap)
        .filter(|receipt| receipt.current_status != ActionStatus::Blocked)
        .count();
    assert_eq!(dispatched, 1);
    assert_eq!(rt.outbox.recent(10).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_authorizations_reserve_monetary_budget() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("monetary-reservation".into()), None, vec![])
        .await
        .unwrap();
    rt.update_policy(json!({
        "allowedChannels": ["conversation", "paid"],
        "actuatorAllowlist": [
            "conversation", "web-ui", "local-log", "local-notification", "paid.test"
        ],
        "sessionMonetaryBudget": 1.0,
        "channelLimits": {"paid": {"enabled": true}}
    }))
    .await
    .unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    rt.registry
        .register_actuator(Arc::new(PaidActuator {
            executions: executions.clone(),
        }))
        .await
        .unwrap();

    let mut handles = Vec::new();
    for _ in 0..32 {
        let plan = rt
            .create_plan(
                SemanticIntent::new("paid-test"),
                vec!["paid.test".into()],
                1,
                1,
                false,
                None,
                BTreeMap::new(),
            )
            .await
            .unwrap();
        let cloned = rt.clone_handle();
        handles.push(tokio::spawn(async move {
            cloned
                .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
                .await
        }));
    }
    for handle in handles {
        handle.await.unwrap().unwrap();
    }
    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "receipts: {:?}; policy: {:?}",
        rt.list_actions(None, 100).unwrap(),
        rt.policy().await
    );
    assert_eq!(rt.current_session().await.unwrap().monetary_spent, 1.0);
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

#[tokio::test]
async fn dynamic_mock_device_has_observability_pairing() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("dyn".into()), None, vec![])
        .await
        .unwrap();
    rt.add_mock_actuator("dev.device", "haptic").await.unwrap();

    // Paired status receptor exists.
    let caps = rt
        .capabilities(&DiscoveryContext {
            include_unavailable: true,
            ..Default::default()
        })
        .await;
    assert!(caps
        .receptors
        .iter()
        .any(|r| r.id.as_str() == "dev.device.device-status"));

    // Open every gate, then observed verification must close the loop.
    rt.update_policy(json!({
        "allowedChannels": ["conversation","haptic"],
        "actuatorAllowlist": ["conversation","dev.device"],
    }))
    .await
    .unwrap();
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("dev.device"), true)
        .await
        .unwrap();
    rt.grant_consent("actuator:dev.device", None).await.unwrap();

    let mut intent = SemanticIntent::new("celebrate-progress");
    intent.magnitude = Some(0.5);
    intent.preferred_channels = vec!["haptic".into()];
    let mut metadata = BTreeMap::new();
    metadata.insert("verification".to_string(), json!("observed"));
    let plan = rt
        .create_plan(
            intent,
            vec!["dev.device".into()],
            1,
            1,
            false,
            None,
            metadata,
        )
        .await
        .unwrap();
    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert_eq!(receipts[0].current_status, ActionStatus::Completed);
    assert_eq!(
        receipts[0].verification.as_ref().unwrap().verdict,
        VerificationVerdict::Observed,
        "dynamically added device must be observable end to end"
    );

    // Removal drops the pairing too.
    rt.remove_actuator("dev.device").await.unwrap();
    let caps = rt
        .capabilities(&DiscoveryContext {
            include_unavailable: true,
            ..Default::default()
        })
        .await;
    assert!(!caps
        .receptors
        .iter()
        .any(|r| r.id.as_str() == "dev.device.device-status"));
}

#[tokio::test]
async fn pushed_observation_cannot_forge_action_verification() {
    // A caller who pushes an observation carrying facts.actionId must NOT be
    // able to self-attest that a real action was "observed"/completed. The
    // ingest path renames actionId → claimActionId (regression for the
    // forged-evidence finding).
    let (_g, rt) = runtime().await;
    let obs = rt
        .ingest(
            "manual.event",
            facts(&[("actionId", "action-victim"), ("state", "done")]),
            std::collections::BTreeMap::new(),
            1.0,
        )
        .await
        .unwrap();
    assert!(
        !obs.facts.contains_key("actionId"),
        "pushed actionId must be scrubbed"
    );
    assert_eq!(
        obs.facts.get("claimActionId"),
        Some(&json!("action-victim")),
        "scrubbed value is kept under claimActionId (never used as evidence)"
    );
}

/// v0.5 Phase 7 回歸：`status().quietHours` 必須反映 policy.quietHours 是否
/// 正在生效——角色視窗（CompanionApp）與 Interaction Director 只認這個鍵。
/// 之前 status() 從未輸出該鍵，quiet 基態在生產環境永遠不可達。
#[tokio::test]
async fn status_reports_active_quiet_hours() {
    let (_g, rt) = runtime().await;
    let before = rt.status().await;
    assert_eq!(
        before["quietHours"],
        json!(false),
        "no quiet window configured"
    );

    // 全天窗口（00:00–23:59）：無論測試何時跑都在窗內。
    rt.update_policy(json!({"quietHours": [{"start": "00:00", "end": "23:59"}]}))
        .await
        .unwrap();
    let during = rt.status().await;
    assert_eq!(during["quietHours"], json!(true));

    // 清掉窗口 → 回 false（不能黏住）。
    rt.update_policy(json!({"quietHours": []})).await.unwrap();
    let after = rt.status().await;
    assert_eq!(after["quietHours"], json!(false));
}

fn stub_receipt(
    action_id: &str,
    actuator: &str,
    status: ActionStatus,
    at: chrono::DateTime<chrono::Utc>,
) -> ActionReceipt {
    ActionReceipt {
        action_id: ActionId::new(action_id),
        plan_id: PlanId::new(format!("plan-{action_id}")),
        session_id: SessionId::new("sess-inbox"),
        actuator_id: ActuatorId::new(actuator),
        intent: format!("intent-{action_id}"),
        requested_parameters: ActionParameters::default(),
        effective_bounded_parameters: ActionParameters::default(),
        policy_decisions: vec![],
        current_status: status,
        timestamps: vec![(status, at)],
        errors: vec![],
        driver_response: BTreeMap::new(),
        verification: None,
        expires_at: None,
        correlation_id: CorrelationId::new(format!("corr-{action_id}")),
        schema_version: SCHEMA_VERSION.to_string(),
    }
}

/// 右上角 Inbox 的「待我決定」是全部待辦的數量，不是本頁剛好裝得下的數量。
/// 舊實作在 truncate 之後才數，較舊的待決定項目會被靜靜漏掉。
#[tokio::test]
async fn inbox_pending_count_is_computed_before_page_truncation() {
    let (_g, rt) = runtime().await;
    let base = chrono::Utc::now();

    // 3 筆較舊、需要人類決定的動作（結果未知，實體通道）。
    for i in 0..3 {
        let receipt = stub_receipt(
            &format!("old-pending-{i}"),
            "mock.actuator",
            ActionStatus::Uncertain,
            base - chrono::Duration::hours(2) + chrono::Duration::seconds(i),
        );
        assert!(rt.store.upsert_receipt(&receipt, "haptic").unwrap());
    }
    // 25 筆較新、不需要決定的動作，足以把上面 3 筆擠出第一頁。
    for i in 0..25 {
        let receipt = stub_receipt(
            &format!("recent-done-{i}"),
            "conversation",
            ActionStatus::Completed,
            base + chrono::Duration::seconds(i),
        );
        assert!(rt.store.upsert_receipt(&receipt, "conversation").unwrap());
    }

    let inbox = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter {
            limit: Some(20),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(inbox["count"].as_u64(), Some(20), "page size honours limit");
    assert_eq!(inbox["totalBeforeLimit"].as_u64(), Some(28));
    assert_eq!(
        inbox["pendingCount"].as_u64(),
        Some(3),
        "pending count must cover every waiting item, not just this page"
    );
    // 這一頁確實一筆待決定都看不到 —— 正是舊實作漏報 0 的情境。
    let items = inbox["items"].as_array().unwrap();
    assert!(items
        .iter()
        .all(|item| item["needsDecision"].as_bool() == Some(false)));
}

/// regression（ia-settings）：通知中心只拿 `limit:20` 的最近一頁再自己過濾
/// needsDecision，最近 20 筆都不是待決定時就會宣稱「目前沒有待決定事項」，
/// 而徽章同時顯示 pendingCount>0。後端提供 `needsDecision` 篩選：介面可以
/// 直接拿到**全部**待決定項；pendingCount 仍在分頁截斷前算完。
#[tokio::test]
async fn inbox_needs_decision_filter_returns_pending_items_beyond_the_first_page() {
    let (_g, rt) = runtime().await;
    let base = chrono::Utc::now();
    for i in 0..3 {
        let receipt = stub_receipt(
            &format!("old-pending-{i}"),
            "mock.actuator",
            ActionStatus::Uncertain,
            base - chrono::Duration::hours(2) + chrono::Duration::seconds(i),
        );
        assert!(rt.store.upsert_receipt(&receipt, "haptic").unwrap());
    }
    for i in 0..25 {
        let receipt = stub_receipt(
            &format!("recent-done-{i}"),
            "conversation",
            ActionStatus::Completed,
            base + chrono::Duration::seconds(i),
        );
        assert!(rt.store.upsert_receipt(&receipt, "conversation").unwrap());
    }

    // 前端送的是 camelCase `needsDecision`（deny_unknown_fields：名稱是契約）。
    let filter: interaction_runtime::activity::ActivityInboxFilter =
        serde_json::from_value(json!({"needsDecision": true, "limit": 2})).unwrap();
    assert_eq!(filter.needs_decision, Some(true));
    let pending_page = rt.activity_inbox(filter).await.unwrap();
    assert_eq!(
        pending_page["count"].as_u64(),
        Some(2),
        "limit still applies"
    );
    assert_eq!(pending_page["totalBeforeLimit"].as_u64(), Some(3));
    assert_eq!(
        pending_page["pendingCount"].as_u64(),
        Some(3),
        "the badge count is computed before truncation"
    );
    assert!(pending_page["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["needsDecision"] == json!(true)));
    assert_eq!(pending_page["filters"]["needsDecision"], json!(true));

    // 夠大的 limit：三筆全部列出，一筆都不會被最近的 25 筆擠掉。
    let all_pending = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter {
            needs_decision: Some(true),
            limit: Some(20),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all_pending["count"].as_u64(), Some(3));
    let ids: Vec<&str> = all_pending["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["itemId"].as_str())
        .collect();
    for i in 0..3 {
        assert!(
            ids.contains(&format!("old-pending-{i}").as_str()),
            "{ids:?}"
        );
    }

    // `false`：只要不需決定的；缺席：全部（既有行為不變）。
    let done = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter {
            needs_decision: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(done["totalBeforeLimit"].as_u64(), Some(25));
    assert!(done["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["needsDecision"] == json!(false)));
    let everything = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter::default())
        .await
        .unwrap();
    assert_eq!(everything["totalBeforeLimit"].as_u64(), Some(28));
    assert_eq!(everything["pendingCount"].as_u64(), Some(3));
}

/// regression（ia-settings-011）：「待我決定」不能只從最近 200 筆收據裡碰
/// 運氣。uncertain／blocked 是黏著終態、又沒有 ack／dismiss 介面，一旦被
/// 200 筆較新的收據擠出歷史視窗，舊實作的 pendingCount 就會掉成 0，介面
/// 接著宣稱「目前沒有待決定事項」——一筆結果未知的實體動作就這樣無聲消失。
/// 待決定項現在改成直接依狀態查，並用 `pendingCountExact` 誠實表態。
#[tokio::test]
async fn inbox_pending_items_survive_the_history_window_overflow() {
    let (_g, rt) = runtime().await;
    let base = chrono::Utc::now();

    // 3 筆較舊、需要人類決定的動作（結果未知，實體通道）。
    for i in 0..3 {
        let receipt = stub_receipt(
            &format!("old-pending-{i}"),
            "mock.actuator",
            ActionStatus::Uncertain,
            base - chrono::Duration::hours(2) + chrono::Duration::seconds(i),
        );
        assert!(rt.store.upsert_receipt(&receipt, "haptic").unwrap());
    }
    // 200 筆較新的收據：剛好把「最近 200 筆」的歷史視窗整個填滿。
    for i in 0..200 {
        let receipt = stub_receipt(
            &format!("recent-done-{i}"),
            "conversation",
            ActionStatus::Completed,
            base + chrono::Duration::seconds(i),
        );
        assert!(rt.store.upsert_receipt(&receipt, "conversation").unwrap());
    }

    // 前提：歷史視窗裡真的一筆待決定都不剩（舊實作就是從這裡數出 0）。
    let window = rt.list_actions(None, 200).unwrap();
    assert_eq!(window.len(), 200);
    assert!(window
        .iter()
        .all(|receipt| receipt.current_status == ActionStatus::Completed));

    let inbox = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter::default())
        .await
        .unwrap();
    assert_eq!(
        inbox["pendingCount"].as_u64(),
        Some(3),
        "pushed-out pending items must still be counted, not silently dropped"
    );
    assert_eq!(
        inbox["pendingCountExact"],
        json!(true),
        "3 待決定項遠低於掃描上限，數字是精確的"
    );
    // 歷史 200 筆 ＋ 視窗外補回來的 3 筆待決定（不重複、也不多補不需決定的）。
    assert_eq!(inbox["totalBeforeLimit"].as_u64(), Some(203));

    let pending = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter {
            needs_decision: Some(true),
            limit: Some(20),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(pending["count"].as_u64(), Some(3));
    let ids: Vec<&str> = pending["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["itemId"].as_str())
        .collect();
    for i in 0..3 {
        assert!(
            ids.contains(&format!("old-pending-{i}").as_str()),
            "{ids:?}"
        );
    }
}

/// regression（ia-settings）：收件匣安全事件的標題曾是原始 event_type
/// （`emergency.stop`／`sensor.started`），而「解除緊急停止」也走
/// EmergencyStop 事件（payload.cleared=true），被投影成 status "emergency"
/// ——使用者剛解除就看到一筆新的「緊急停止」。解除必須是
/// `emergency-cleared`，標題一律人話。
#[tokio::test]
async fn inbox_safety_events_distinguish_emergency_clear_and_use_human_titles() {
    let (_g, rt) = runtime().await;
    rt.emergency_stop("test", Some("drill".into()))
        .await
        .unwrap();
    rt.clear_emergency_stop("test").await.unwrap();
    rt.events.emit(
        EventType::SensorStarted,
        json!({"sensor": "microphone", "reason": "test"}),
    );
    rt.events
        .emit(EventType::SensorStopped, json!({"sensor": "microphone"}));

    let inbox = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter::default())
        .await
        .unwrap();
    let safety: Vec<&serde_json::Value> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == json!("safety-event"))
        .collect();
    assert!(safety.len() >= 4, "{safety:?}");
    // 標題永遠不是原始 event_type。
    for item in &safety {
        let title = item["title"].as_str().unwrap();
        assert!(
            !["emergency.stop", "sensor.started", "sensor.stopped"].contains(&title),
            "raw event_type leaked as title: {item}"
        );
        assert_eq!(item["needsDecision"], json!(false));
    }
    let cleared = safety
        .iter()
        .find(|item| item["detail"]["payload"]["cleared"] == json!(true))
        .expect("the clear event is in the inbox");
    assert_eq!(cleared["status"], json!("emergency-cleared"));
    assert_eq!(cleared["title"], json!("緊急停止已解除"));
    let engaged = safety
        .iter()
        .find(|item| {
            item["detail"]["eventType"] == json!("emergency.stop")
                && item["detail"]["payload"]["cleared"] != json!(true)
        })
        .expect("the engage event is in the inbox");
    assert_eq!(engaged["status"], json!("emergency"));
    assert_eq!(engaged["title"], json!("緊急停止已啟動"));
    let started = safety
        .iter()
        .find(|item| item["status"] == json!("sensor.started"))
        .expect("sensor start is a safety event");
    assert_eq!(started["title"], json!("感測開始：麥克風"));
    assert_eq!(started["deviceId"], json!("microphone"));
    let stopped = safety
        .iter()
        .find(|item| item["status"] == json!("sensor.stopped"))
        .unwrap();
    assert_eq!(stopped["title"], json!("感測停止：麥克風"));
    // status 篩選：`emergency` 仍同時命中啟動與解除（contains），
    // `emergency-cleared` 只命中解除。
    let only_cleared = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter {
            status: Some("emergency-cleared".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(only_cleared["count"].as_u64(), Some(1));
}

/// regression ia-settings-016：手機的感測事件 payload 同時帶 `deviceId`
/// （`iphone-<hex>`，token 衍生的內部 id）與 `sensor`，而收件匣標題原本直接把
/// `deviceId` 當「感測器名稱」印出來——「感測開始：iphone-a1b2c3d4」。原始裝置 id
/// 不得進一般模式的標題（同檔的 stop-uncertain 早就會解析成人話名稱）。
#[tokio::test]
async fn inbox_sensor_titles_never_leak_the_raw_device_id() {
    let (_g, rt) = runtime().await;
    let device_id = "iphone-a1b2c3d4";
    // mobile.rs 實際送出的形狀（deviceId 優先於 sensor 被取用）。
    rt.events.emit(
        EventType::SensorStarted,
        json!({"sensor": "iphone.mic-level", "deviceId": device_id, "source": "iphone"}),
    );
    rt.events.emit(
        EventType::SensorStopped,
        json!({"sensor": "iphone.mic-level", "deviceId": device_id, "source": "iphone"}),
    );
    // 認不得的種類也不得外洩原始字串（退回通用標籤）。
    rt.events.emit(
        EventType::SensorStarted,
        json!({"sensor": "weird.internal"}),
    );

    let inbox = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter::default())
        .await
        .unwrap();
    let titles: Vec<String> = inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == json!("safety-event"))
        .filter_map(|item| item["title"].as_str().map(String::from))
        .collect();
    assert!(
        titles.iter().all(|t| !t.contains(device_id)),
        "原始裝置 id 外洩到標題：{titles:?}"
    );
    assert!(
        titles.iter().all(|t| !t.contains("iphone.mic-level")),
        "原始感測 id 外洩到標題：{titles:?}"
    );
    assert!(
        titles.iter().any(|t| t == "感測開始：iPhone"),
        "手機感測開始要說人話：{titles:?}"
    );
    assert!(
        titles.iter().any(|t| t == "感測停止：iPhone"),
        "手機感測停止要說人話：{titles:?}"
    );
    assert!(
        titles.iter().any(|t| t == "感測開始：感測器"),
        "認不得的種類退回通用標籤：{titles:?}"
    );
    // deviceId 欄位本身仍在（篩選與進階模式要用），只是不進標題。
    assert!(
        inbox["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["deviceId"] == json!(device_id)),
        "deviceId 欄位不該消失：{inbox}"
    );
}
