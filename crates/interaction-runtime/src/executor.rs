//! Plan execution: authorization → bounded action → driver dispatch →
//! verification. Emits events and persists receipts at every transition.
//! `QUEUED`/accepted is never reported as completed.

use crate::runtime::Runtime;
use chrono::Utc;
use interaction_core::{
    ActionId, ActionReceipt, ActionStatus, ActuatorManifest, AuthorizationOutcome, BoundedAction,
    DomainError, DomainResult, EventType, PatternSpec, Plan, PlanStatus, PlannedStep,
    PolicyDecision, Session, Timestamp, VerificationEvidence, VerificationVerdict,
};
use interaction_policy::{ActionSource, AuthorizationRequest, Governor, UsageContext};
use serde_json::json;

const DISPATCH_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_VERIFY_TIMEOUT_MS: u64 = 5_000;

/// Simulation output: per-step decisions, no side effects.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationReport {
    pub plan_id: String,
    pub steps: Vec<SimulatedStep>,
    pub would_execute: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulatedStep {
    pub actuator_id: String,
    pub channel: String,
    pub outcome: AuthorizationOutcome,
    pub decisions: Vec<PolicyDecision>,
    pub effective: interaction_core::ActionParameters,
}

/// A receipt whose action was refused before acceptance.
fn refused_receipt(
    plan: &Plan,
    step: &PlannedStep,
    decisions: Vec<PolicyDecision>,
    now: Timestamp,
) -> ActionReceipt {
    ActionReceipt {
        action_id: ActionId::generate(),
        plan_id: plan.plan_id.clone(),
        session_id: plan.session_id.clone(),
        actuator_id: step.actuator_id.clone(),
        intent: plan.intent.intent.clone(),
        requested_parameters: step.requested.clone(),
        effective_bounded_parameters: Default::default(),
        policy_decisions: decisions,
        current_status: ActionStatus::Blocked,
        timestamps: vec![(ActionStatus::Planned, now), (ActionStatus::Blocked, now)],
        errors: Vec::new(),
        driver_response: Default::default(),
        verification: None,
        expires_at: Some(plan.expires_at),
        correlation_id: plan.correlation_id.clone(),
        schema_version: interaction_core::SCHEMA_VERSION.to_string(),
    }
}

impl Runtime {
    async fn usage_for(
        &self,
        session: &Session,
        manifest: &ActuatorManifest,
    ) -> DomainResult<UsageContext> {
        let now = Utc::now();
        let (fired, last) = self.store.actuator_usage(manifest.id.as_str(), now)?;
        let channel_used = self.store.channel_usage_ms(&session.session_id, &manifest.channel)?;
        let scheduled = self.store.scheduled_action_count()?;
        Ok(UsageContext {
            actuator_fired_last_hour: fired,
            actuator_last_fired_at: last,
            channel_budget_used_ms: channel_used,
            monetary_spent: session.monetary_spent,
            scheduled_actions: scheduled,
        })
    }

    fn authorize_step(
        &self,
        policy: &interaction_core::PolicyConfig,
        session: &Session,
        manifest: &ActuatorManifest,
        step: &PlannedStep,
        intent: &str,
        source: ActionSource,
        usage: &UsageContext,
    ) -> interaction_policy::AuthorizationResult {
        let req = AuthorizationRequest {
            actuator: manifest,
            requested: &step.requested,
            intent,
            source,
            local_time: chrono::Local::now().time(),
            now: Utc::now(),
            emergency_stop_engaged: self.is_estopped(),
        };
        Governor::authorize(policy, session, &req, usage)
    }

    /// Dry-run the governor over every step. No side effects, no receipts.
    pub async fn simulate_plan(&self, plan_id: &interaction_core::PlanId) -> DomainResult<SimulationReport> {
        let mut plan = self.store.plan(plan_id)?;
        let session = self.require_session().await?;
        let policy = self.policy().await;
        let mut steps = Vec::new();
        let mut would_execute = false;
        for step in &plan.steps {
            let manifest = match self.registry.actuator(&step.actuator_id).await {
                Ok(a) => a.manifest(),
                Err(e) => {
                    steps.push(SimulatedStep {
                        actuator_id: step.actuator_id.as_str().into(),
                        channel: step.channel.clone(),
                        outcome: AuthorizationOutcome::Blocked,
                        decisions: vec![PolicyDecision::Blocked {
                            rule: "registry.availability".into(),
                            reason: e.to_string(),
                        }],
                        effective: Default::default(),
                    });
                    continue;
                }
            };
            let usage = self.usage_for(&session, &manifest).await?;
            let result = self.authorize_step(
                &policy,
                &session,
                &manifest,
                step,
                &plan.intent.intent,
                ActionSource::ExplicitRequest,
                &usage,
            );
            if result.outcome == AuthorizationOutcome::Authorized {
                would_execute = true;
            }
            steps.push(SimulatedStep {
                actuator_id: step.actuator_id.as_str().into(),
                channel: step.channel.clone(),
                outcome: result.outcome,
                decisions: result.decisions,
                effective: result.effective,
            });
        }
        if plan.status == PlanStatus::Draft {
            plan.status = PlanStatus::Simulated;
            self.store.upsert_plan(&plan)?;
        }
        Ok(SimulationReport { plan_id: plan.plan_id.as_str().into(), steps, would_execute })
    }

    /// Execute a plan. Honors the plan's actuation mode; returns all receipts
    /// (including blocked/failed ones) so callers see exactly what happened.
    pub async fn execute_plan(
        &self,
        plan_id: &interaction_core::PlanId,
        source: ActionSource,
        dry_run: bool,
    ) -> DomainResult<Vec<ActionReceipt>> {
        if dry_run {
            let report = self.simulate_plan(plan_id).await?;
            return Err(DomainError::Validation(format!(
                "dry run: {} steps, wouldExecute={}; use simulate for details",
                report.steps.len(),
                report.would_execute
            )));
        }
        let mut plan = self.store.plan(plan_id)?;
        let now = Utc::now();
        if self.is_estopped() {
            return Err(DomainError::EmergencyStop);
        }
        match plan.status {
            PlanStatus::Blocked => {
                return Err(DomainError::PolicyBlocked("plan is blocked".into()))
            }
            PlanStatus::Executed => {
                return Err(DomainError::Conflict("plan already executed".into()))
            }
            PlanStatus::NoAction => {
                return Ok(Vec::new());
            }
            _ => {}
        }
        if plan.is_expired(now) {
            plan.status = PlanStatus::Expired;
            self.store.upsert_plan(&plan)?;
            return Err(DomainError::Expired(format!("plan {plan_id}")));
        }
        let session = self.require_session().await?;
        if session.session_id != plan.session_id {
            return Err(DomainError::SessionInactive(format!(
                "plan belongs to session {}, current is {}",
                plan.session_id, session.session_id
            )));
        }

        let mode = plan
            .metadata
            .get("actuationMode")
            .and_then(|v| v.as_str())
            .unwrap_or("parallel")
            .to_string();

        let mut receipts = Vec::new();
        match mode.as_str() {
            "fallback" => {
                for step in plan.steps.clone() {
                    let receipt = self.run_step(&plan, &step, source).await?;
                    let succeeded = matches!(
                        receipt.current_status,
                        ActionStatus::Acknowledged
                            | ActionStatus::Observed
                            | ActionStatus::Completed
                    );
                    receipts.push(receipt);
                    if succeeded {
                        break;
                    }
                }
            }
            "sequence" => {
                for step in plan.steps.clone() {
                    let receipt = self.run_step(&plan, &step, source).await?;
                    receipts.push(receipt);
                }
            }
            // single / parallel / adaptive / redundant: the orchestrator already
            // chose the step set; run them concurrently.
            _ => {
                let mut handles = Vec::new();
                for step in plan.steps.clone() {
                    let plan_clone = plan.clone();
                    let this = self.clone_handle();
                    handles.push(tokio::spawn(async move {
                        this.run_step(&plan_clone, &step, source).await
                    }));
                }
                for handle in handles {
                    match handle.await {
                        Ok(Ok(receipt)) => receipts.push(receipt),
                        Ok(Err(e)) => tracing::warn!(error = %e, "step execution error"),
                        Err(e) => tracing::error!(error = %e, "step task panicked"),
                    }
                }
            }
        }

        plan.status = PlanStatus::Executed;
        self.store.upsert_plan(&plan)?;
        Ok(receipts)
    }

    /// Run one planned step end to end.
    pub(crate) async fn run_step(
        &self,
        plan: &Plan,
        step: &PlannedStep,
        source: ActionSource,
    ) -> DomainResult<ActionReceipt> {
        let now = Utc::now();
        let session = self.require_session().await?;
        let policy = self.policy().await;

        // Resolve the actuator; unavailable → blocked receipt with the reason.
        let actuator = match self.registry.actuator(&step.actuator_id).await {
            Ok(a) => a,
            Err(e) => {
                let receipt = refused_receipt(
                    plan,
                    step,
                    vec![PolicyDecision::Blocked {
                        rule: "registry.availability".into(),
                        reason: e.to_string(),
                    }],
                    now,
                );
                self.persist_receipt(&receipt, &step.channel).await?;
                self.emit_action_event(EventType::ActionFailed, &receipt, json!({"reason": e.to_string()}));
                return Ok(receipt);
            }
        };
        let manifest = actuator.manifest();
        // Health gate: offline drivers are not dispatched to.
        if !manifest.availability.is_available() || !actuator.status().await.is_usable() {
            let receipt = refused_receipt(
                plan,
                step,
                vec![PolicyDecision::Blocked {
                    rule: "actuator.offline".into(),
                    reason: format!("actuator {} is offline/unusable", manifest.id),
                }],
                now,
            );
            self.persist_receipt(&receipt, &step.channel).await?;
            self.emit_action_event(EventType::ActionFailed, &receipt, json!({"reason": "offline"}));
            return Ok(receipt);
        }

        let usage = self.usage_for(&session, &manifest).await?;
        let auth = self.authorize_step(
            &policy,
            &session,
            &manifest,
            step,
            &plan.intent.intent,
            source,
            &usage,
        );

        if auth.outcome != AuthorizationOutcome::Authorized {
            let receipt = refused_receipt(plan, step, auth.decisions, now);
            self.persist_receipt(&receipt, &step.channel).await?;
            self.events.emit(
                EventType::PlanBlocked,
                json!({
                    "planId": plan.plan_id.as_str(),
                    "actionId": receipt.action_id.as_str(),
                    "actuatorId": step.actuator_id.as_str(),
                    "outcome": auth.outcome,
                }),
            );
            self.store.audit(
                "action.blocked",
                "governor",
                &json!({"actionId": receipt.action_id.as_str(), "decisions": receipt.policy_decisions}),
            )?;
            return Ok(receipt);
        }

        // Build the immutable bounded action.
        let mut metadata = std::collections::BTreeMap::new();
        let mut decisions = auth.decisions.clone();
        if let Some(pattern_value) = step.requested.extra.as_ref().and_then(|e| e.get("pattern")) {
            if manifest.supports_pattern {
                let (pattern, pattern_decisions) =
                    self.bound_pattern_value(&policy, &manifest, pattern_value, &auth.effective)?;
                decisions.extend(pattern_decisions);
                metadata.insert("pattern".to_string(), serde_json::to_value(&pattern).unwrap_or_default());
            } else {
                decisions.push(PolicyDecision::Silenced {
                    rule: "pattern.unsupported".into(),
                    detail: format!("actuator {} does not support patterns", manifest.id),
                });
            }
        }
        let action = BoundedAction {
            action_id: ActionId::generate(),
            plan_id: plan.plan_id.clone(),
            session_id: session.session_id.clone(),
            actuator_id: step.actuator_id.clone(),
            intent: plan.intent.intent.clone(),
            risk_class: manifest.risk_class,
            requested: step.requested.clone(),
            effective: auth.effective.clone(),
            policy_decisions: decisions,
            expires_at: plan.expires_at,
            issued_at: now,
            correlation_id: plan.correlation_id.clone(),
            metadata,
            schema_version: interaction_core::SCHEMA_VERSION.to_string(),
        };

        // Authorized → Accepted (queued). Accepted is NOT completion.
        let mut receipt = ActionReceipt::for_action(&action, now);
        receipt
            .transition(ActionStatus::Accepted, Utc::now())
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        self.persist_receipt(&receipt, &step.channel).await?;
        self.emit_action_event(EventType::ActionAccepted, &receipt, json!({}));
        self.events.emit(
            EventType::PlanAuthorized,
            json!({"planId": plan.plan_id.as_str(), "actionId": receipt.action_id.as_str()}),
        );

        // Dispatch with timeout; a timeout means we DON'T KNOW → uncertain.
        let dispatch = tokio::time::timeout(
            std::time::Duration::from_millis(DISPATCH_TIMEOUT_MS),
            actuator.execute(action.clone()),
        )
        .await;
        let now2 = Utc::now();
        match dispatch {
            Err(_) => {
                receipt.push_error("dispatch_timeout", "driver did not answer in time", now2);
                let _ = receipt.transition(ActionStatus::Uncertain, now2);
                self.persist_receipt(&receipt, &step.channel).await?;
                self.emit_action_event(EventType::ActionUncertain, &receipt, json!({"reason": "dispatch timeout"}));
                return Ok(receipt);
            }
            Ok(Err(driver_err)) => {
                receipt.push_error("driver_error", &driver_err.to_string(), now2);
                let _ = receipt.transition(ActionStatus::Failed, now2);
                self.persist_receipt(&receipt, &step.channel).await?;
                self.emit_action_event(EventType::ActionFailed, &receipt, json!({"reason": driver_err.to_string()}));
                return Ok(receipt);
            }
            Ok(Ok(driver_receipt)) => {
                interaction_adapter_sdk::merge_driver_receipt(&mut receipt, &driver_receipt);
                self.persist_receipt(&receipt, &step.channel).await?;
                match receipt.current_status {
                    ActionStatus::Dispatched => {
                        self.emit_action_event(EventType::ActionDispatched, &receipt, json!({}))
                    }
                    ActionStatus::Acknowledged => {
                        self.emit_action_event(EventType::ActionAcknowledged, &receipt, json!({}))
                    }
                    ActionStatus::Failed => {
                        self.emit_action_event(EventType::ActionFailed, &receipt, json!({}))
                    }
                    _ => {}
                }
            }
        }

        // Verification.
        let strategy = plan
            .metadata
            .get("verification")
            .and_then(|v| v.as_str())
            .unwrap_or("best-effort")
            .to_string();
        let receipt = self.verify_receipt(receipt, &step.channel, &strategy).await?;

        // Budget bookkeeping on successful output.
        if matches!(
            receipt.current_status,
            ActionStatus::Acknowledged | ActionStatus::Observed | ActionStatus::Completed
        ) {
            self.track_session_usage(&session.session_id, &step.channel, &receipt).await;
        }
        Ok(receipt)
    }

    fn bound_pattern_value(
        &self,
        policy: &interaction_core::PolicyConfig,
        manifest: &ActuatorManifest,
        pattern_value: &serde_json::Value,
        effective: &interaction_core::ActionParameters,
    ) -> DomainResult<(PatternSpec, Vec<PolicyDecision>)> {
        // `repeat: "forever"` normalizes to a TTL-bounded lease, never an
        // unbounded loop.
        let mut value = pattern_value.clone();
        let mut decisions = Vec::new();
        if value.get("repeat").map(|r| r == "forever").unwrap_or(false) {
            let step_ms: u64 = value
                .get("steps")
                .and_then(|s| s.as_array())
                .map(|steps| {
                    steps
                        .iter()
                        .map(|st| {
                            st.get("durationMs").and_then(|d| d.as_u64()).unwrap_or(100)
                                + st.get("pauseMs").and_then(|d| d.as_u64()).unwrap_or(0)
                        })
                        .sum()
                })
                .unwrap_or(100)
                .max(1);
            let ttl = effective.duration_ms.unwrap_or(policy.default_ttl_ms);
            let bounded_repeat = (ttl / step_ms).clamp(1, 1000) as u32;
            decisions.push(PolicyDecision::Clamped {
                rule: "pattern.lease".into(),
                field: "repeat".into(),
                from: f64::INFINITY,
                to: bounded_repeat as f64,
            });
            value["repeat"] = json!(bounded_repeat);
        }
        let pattern: PatternSpec = serde_json::from_value(value)
            .map_err(|e| DomainError::Validation(format!("invalid pattern: {e}")))?;
        let max_mag = effective.magnitude.unwrap_or(1.0);
        let (bounded, more) = Governor::bound_pattern(policy, manifest, &pattern, max_mag);
        decisions.extend(more);
        Ok((bounded, decisions))
    }

    /// Verification engine: upgrade a receipt according to the strategy.
    pub(crate) async fn verify_receipt(
        &self,
        mut receipt: ActionReceipt,
        channel: &str,
        strategy: &str,
    ) -> DomainResult<ActionReceipt> {
        if receipt.is_terminal() {
            return Ok(receipt);
        }
        let now = Utc::now();
        match strategy {
            "none" => Ok(receipt),
            "observed" => {
                // Look for observations that reference the action id, reading
                // device-category receptors fresh first.
                let dispatched_at = receipt
                    .timestamps
                    .iter()
                    .find(|(s, _)| *s == ActionStatus::Dispatched)
                    .map(|(_, t)| *t)
                    .unwrap_or(now);
                for manifest in self.registry.receptor_manifests().await {
                    if manifest.category == "device" && manifest.availability.is_available() {
                        let _ = self.observe_fresh(&manifest.id).await;
                    }
                }
                let observations = self.store.query_observations(&interaction_core::ObservationQuery {
                    since: Some(dispatched_at - chrono::Duration::seconds(1)),
                    limit: Some(200),
                    ..Default::default()
                })?;
                let evidence: Vec<_> = observations
                    .iter()
                    .filter(|o| {
                        o.facts
                            .get("actionId")
                            .and_then(|v| v.as_str())
                            .map(|id| id == receipt.action_id.as_str())
                            .unwrap_or(false)
                    })
                    .collect();
                if !evidence.is_empty() {
                    if receipt.current_status == ActionStatus::Acknowledged {
                        let _ = receipt.transition(ActionStatus::Observed, now);
                        self.emit_action_event(EventType::ActionObserved, &receipt, json!({}));
                        let _ = receipt.transition(ActionStatus::Completed, Utc::now());
                    } else if receipt.current_status.can_transition_to(ActionStatus::Uncertain) {
                        // Dispatched-but-not-acked with evidence: mark observed
                        // is illegal from Dispatched; go through the legal path.
                        let _ = receipt.transition(ActionStatus::Uncertain, now);
                    }
                    receipt.verification = Some(VerificationEvidence {
                        observation_ids: evidence.iter().map(|o| o.observation_id.clone()).collect(),
                        verdict: VerificationVerdict::Observed,
                        detail: Some(format!("{} corroborating observation(s)", evidence.len())),
                        verified_at: now,
                    });
                    if receipt.current_status == ActionStatus::Completed {
                        self.emit_action_event(EventType::ActionCompleted, &receipt, json!({}));
                    }
                } else {
                    let age = now
                        .signed_duration_since(dispatched_at)
                        .num_milliseconds()
                        .max(0) as u64;
                    if age > DEFAULT_VERIFY_TIMEOUT_MS {
                        receipt.verification = Some(VerificationEvidence {
                            observation_ids: vec![],
                            verdict: VerificationVerdict::Uncertain,
                            detail: Some("no corroborating observation within timeout".into()),
                            verified_at: now,
                        });
                        let _ = receipt.transition(ActionStatus::Uncertain, now);
                        self.emit_action_event(EventType::ActionUncertain, &receipt, json!({}));
                    }
                    // else: leave as-is; caller may re-verify later.
                }
                self.persist_receipt(&receipt, channel).await?;
                Ok(receipt)
            }
            // best-effort (default): acknowledged is good enough to complete,
            // but the verdict honestly records that it was ack-only.
            _ => {
                if receipt.current_status == ActionStatus::Acknowledged {
                    receipt.verification = Some(VerificationEvidence {
                        observation_ids: vec![],
                        verdict: VerificationVerdict::AcknowledgedOnly,
                        detail: Some("driver acknowledged; no environmental confirmation".into()),
                        verified_at: now,
                    });
                    let _ = receipt.transition(ActionStatus::Completed, now);
                    self.emit_action_event(EventType::ActionCompleted, &receipt, json!({}));
                } else if receipt.current_status == ActionStatus::Dispatched {
                    let dispatched_at = receipt
                        .timestamps
                        .iter()
                        .find(|(s, _)| *s == ActionStatus::Dispatched)
                        .map(|(_, t)| *t)
                        .unwrap_or(now);
                    let age = now
                        .signed_duration_since(dispatched_at)
                        .num_milliseconds()
                        .max(0) as u64;
                    if age > DEFAULT_VERIFY_TIMEOUT_MS {
                        receipt.verification = Some(VerificationEvidence {
                            observation_ids: vec![],
                            verdict: VerificationVerdict::Uncertain,
                            detail: Some("dispatched but never acknowledged".into()),
                            verified_at: now,
                        });
                        let _ = receipt.transition(ActionStatus::Uncertain, now);
                        self.emit_action_event(EventType::ActionUncertain, &receipt, json!({}));
                    }
                }
                self.persist_receipt(&receipt, channel).await?;
                Ok(receipt)
            }
        }
    }
}
