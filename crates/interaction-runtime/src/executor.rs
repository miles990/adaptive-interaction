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
        let channel_used = self
            .store
            .channel_usage_ms(&session.session_id, &manifest.channel)?;
        let scheduled = self.store.scheduled_action_count()?;
        Ok(UsageContext {
            actuator_fired_last_hour: fired,
            actuator_last_fired_at: last,
            channel_budget_used_ms: channel_used,
            monetary_spent: session.monetary_spent,
            scheduled_actions: scheduled,
        })
    }

    #[allow(clippy::too_many_arguments)]
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
    pub async fn simulate_plan(
        &self,
        plan_id: &interaction_core::PlanId,
    ) -> DomainResult<SimulationReport> {
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
        Ok(SimulationReport {
            plan_id: plan.plan_id.as_str().into(),
            steps,
            would_execute,
        })
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
                    handles.push((
                        step.clone(),
                        tokio::spawn(
                            async move { this.run_step(&plan_clone, &step, source).await },
                        ),
                    ));
                }
                for (step, handle) in handles {
                    match handle.await {
                        Ok(Ok(receipt)) => receipts.push(receipt),
                        // Step errors are never swallowed: they become failed
                        // receipts so callers see exactly what happened.
                        Ok(Err(e)) => {
                            tracing::warn!(error = %e, "step execution error");
                            receipts
                                .push(self.record_step_failure(&plan, &step, &e.to_string()).await);
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "step task panicked");
                            receipts.push(
                                self.record_step_failure(&plan, &step, "step task panicked")
                                    .await,
                            );
                        }
                    }
                }
            }
        }

        plan.status = PlanStatus::Executed;
        self.store.upsert_plan(&plan)?;
        Ok(receipts)
    }

    /// A failed-step receipt for errors that would otherwise be swallowed.
    async fn record_step_failure(
        &self,
        plan: &Plan,
        step: &PlannedStep,
        reason: &str,
    ) -> ActionReceipt {
        let now = Utc::now();
        let mut receipt = refused_receipt(plan, step, Vec::new(), now);
        // Rewrite the terminal state to Failed (refused_receipt yields Blocked).
        receipt.current_status = ActionStatus::Failed;
        if let Some(last) = receipt.timestamps.last_mut() {
            last.0 = ActionStatus::Failed;
        }
        receipt.push_error("step_error", reason, now);
        let _ = self.persist_receipt(&receipt, &step.channel).await;
        self.emit_action_event(EventType::ActionFailed, &receipt, json!({"reason": reason}));
        receipt
    }

    /// Last-instant gate evaluated immediately before driver dispatch. Closes
    /// the authorize→dispatch race window for emergency stop, consent
    /// revocation, session death and runtime shutdown.
    async fn pre_dispatch_gate(
        &self,
        manifest: &ActuatorManifest,
    ) -> Result<(), (ActionStatus, PolicyDecision)> {
        if self.is_estopped() {
            return Err((
                ActionStatus::Stopped,
                PolicyDecision::Blocked {
                    rule: "emergency-stop.pre-dispatch".into(),
                    reason: "emergency stop engaged between authorization and dispatch".into(),
                },
            ));
        }
        if self.shutdown_token.is_cancelled() {
            return Err((
                ActionStatus::Cancelled,
                PolicyDecision::Blocked {
                    rule: "shutdown.pre-dispatch".into(),
                    reason: "runtime is shutting down".into(),
                },
            ));
        }
        // Re-read the CURRENT session (not the clone authorization used):
        // consent may have been revoked while this step was in flight.
        let now = Utc::now();
        let session = self.current_session().await;
        let session_ok = session.as_ref().map(|s| s.is_active(now)).unwrap_or(false);
        if !session_ok {
            return Err((
                ActionStatus::Cancelled,
                PolicyDecision::Blocked {
                    rule: "session.pre-dispatch".into(),
                    reason: "session ended between authorization and dispatch".into(),
                },
            ));
        }
        if manifest.requires_consent {
            let session = session.expect("checked above");
            let by_actuator = session.has_consent(
                &interaction_core::ConsentScope::Actuator(manifest.id.as_str().to_string()),
                now,
            );
            let by_channel = session.has_consent(
                &interaction_core::ConsentScope::Channel(manifest.channel.clone()),
                now,
            );
            if !by_actuator && !by_channel {
                return Err((
                    ActionStatus::Cancelled,
                    PolicyDecision::Blocked {
                        rule: "consent.pre-dispatch".into(),
                        reason: "consent revoked between authorization and dispatch".into(),
                    },
                ));
            }
        }
        Ok(())
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
                self.emit_action_event(
                    EventType::ActionFailed,
                    &receipt,
                    json!({"reason": e.to_string()}),
                );
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
            self.emit_action_event(
                EventType::ActionFailed,
                &receipt,
                json!({"reason": "offline"}),
            );
            return Ok(receipt);
        }

        let usage = self.usage_for(&session, &manifest).await?;
        // 主動式對話政策：只約束「自主來源＋對話頻道＋宣告為主動對話」的
        // 說話動作（metadata 帶 proactiveClass——生成式主動對話一定帶；
        // 使用者自訂配方可選擇加入）。明確請求與未宣告的既有自動化不受限
        // （向後相容：配方有自己的預算與冷卻）。
        if source == ActionSource::Autonomous
            && crate::proactive::is_dialogue_channel(&step.channel)
            && plan.metadata.contains_key("proactiveClass")
        {
            let class = crate::proactive::class_from_metadata(&plan.metadata);
            // 去重鍵由觸發方（recipe／gateway）明確宣告；未宣告＝不去重，
            // 只受頻率限制（intent 名稱不能當事件身分）。
            let dedup_key = plan
                .metadata
                .get("dedupKey")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let crate::proactive::ProactiveDecision::Suppressed { reason } =
                self.proactive_dialogue_gate(class, &dedup_key).await
            {
                let receipt = refused_receipt(
                    plan,
                    step,
                    vec![PolicyDecision::Silenced {
                        rule: "proactive-dialogue".into(),
                        detail: reason,
                    }],
                    now,
                );
                self.persist_receipt(&receipt, &step.channel).await?;
                self.events.emit(
                    EventType::PlanBlocked,
                    json!({
                        "planId": plan.plan_id.as_str(),
                        "actionId": receipt.action_id.as_str(),
                        "actuatorId": step.actuator_id.as_str(),
                        "outcome": "silenced-proactive-dialogue",
                    }),
                );
                return Ok(receipt);
            }
        }

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
                metadata.insert(
                    "pattern".to_string(),
                    serde_json::to_value(&pattern).unwrap_or_default(),
                );
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

        // Last-instant gate: emergency stop / shutdown / consent revocation may
        // have happened between authorization and this point. Never dispatch
        // through that window.
        if let Err((terminal, decision)) = self.pre_dispatch_gate(&manifest).await {
            receipt.policy_decisions.push(decision.clone());
            receipt.push_error("pre_dispatch_gate", format!("{decision:?}"), Utc::now());
            let _ = receipt.transition(terminal, Utc::now());
            self.persist_receipt(&receipt, &step.channel).await?;
            self.emit_action_event(EventType::ActionCancelled, &receipt, json!({"gate": true}));
            return Ok(receipt);
        }

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
                self.emit_action_event(
                    EventType::ActionUncertain,
                    &receipt,
                    json!({"reason": "dispatch timeout"}),
                );
                return Ok(receipt);
            }
            Ok(Err(driver_err)) => {
                receipt.push_error("driver_error", driver_err.to_string(), now2);
                let _ = receipt.transition(ActionStatus::Failed, now2);
                self.persist_receipt(&receipt, &step.channel).await?;
                self.emit_action_event(
                    EventType::ActionFailed,
                    &receipt,
                    json!({"reason": driver_err.to_string()}),
                );
                return Ok(receipt);
            }
            Ok(Ok(driver_receipt)) => {
                interaction_adapter_sdk::merge_driver_receipt(&mut receipt, &driver_receipt);
                let applied = self.persist_receipt(&receipt, &step.channel).await?;
                if !applied {
                    // A concurrent e-stop sweep / watchdog already terminalized
                    // this receipt; the stored copy is the truth.
                    return self.store.receipt(&receipt.action_id);
                }
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

        // If the emergency stop fired while the driver was executing, do not
        // proceed to verification/completion: the store sweep already marked
        // this receipt stopped (sticky), so align the local copy honestly.
        if self.is_estopped() && !receipt.is_terminal() {
            receipt.push_error(
                "emergency_stop",
                "emergency stop engaged during dispatch",
                Utc::now(),
            );
            let _ = receipt.transition(ActionStatus::Stopped, Utc::now());
            self.persist_receipt(&receipt, &step.channel).await?;
            return Ok(receipt);
        }

        // Monetary budget accounting: the invocation cost is charged as soon
        // as the command actually left the driver (dispatched or beyond) —
        // the governor reads this back on the next authorization.
        if manifest.cost.monetary_per_invocation > 0.0
            && matches!(
                receipt.current_status,
                ActionStatus::Dispatched
                    | ActionStatus::Acknowledged
                    | ActionStatus::Observed
                    | ActionStatus::Completed
            )
        {
            self.charge_session_cost(&session.session_id, manifest.cost.monetary_per_invocation)
                .await;
        }

        // Verification. Unknown strategy strings fall back to the STRICTEST
        // mode (observed), never silently to best-effort.
        let strategy = match plan.metadata.get("verification").and_then(|v| v.as_str()) {
            None => "best-effort".to_string(),
            Some(s @ ("best-effort" | "observed" | "none")) => s.to_string(),
            Some(unknown) => {
                tracing::warn!(
                    strategy = unknown,
                    "unknown verification strategy; falling back to strict 'observed'"
                );
                "observed".to_string()
            }
        };
        let receipt = self
            .verify_receipt(receipt, &step.channel, &strategy)
            .await?;

        // Budget bookkeeping on successful output.
        if matches!(
            receipt.current_status,
            ActionStatus::Acknowledged | ActionStatus::Observed | ActionStatus::Completed
        ) {
            self.track_session_usage(&session.session_id, &step.channel, &receipt)
                .await;
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

    /// The actuator's formally declared deepest-honest confirmation level.
    /// Missing declarations resolve to Unknown — conservative, never upgraded.
    pub(crate) async fn actuator_confirmation(
        &self,
        actuator_id: &interaction_core::ActuatorId,
    ) -> interaction_core::ConfirmationLevel {
        match self.registry.actuator_any(actuator_id).await {
            Ok(actuator) => actuator
                .manifest()
                .human
                .as_ref()
                .and_then(|h| h.effect.as_ref())
                .map(|e| e.confirmation_level)
                .unwrap_or_default(),
            Err(_) => interaction_core::ConfirmationLevel::default(),
        }
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
                let observations =
                    self.store
                        .query_observations(&interaction_core::ObservationQuery {
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
                    } else if receipt
                        .current_status
                        .can_transition_to(ActionStatus::Uncertain)
                    {
                        // Dispatched-but-not-acked with evidence: mark observed
                        // is illegal from Dispatched; go through the legal path.
                        let _ = receipt.transition(ActionStatus::Uncertain, now);
                    }
                    receipt.verification = Some(VerificationEvidence {
                        observation_ids: evidence
                            .iter()
                            .map(|o| o.observation_id.clone())
                            .collect(),
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
                if !self.persist_receipt(&receipt, channel).await? {
                    return self.store.receipt(&receipt.action_id);
                }
                Ok(receipt)
            }
            // best-effort (default): an ack may complete the action ONLY when
            // the actuator formally declares that its acknowledgement means
            // delivery (local surfaces: conversation/web-ui/log declare
            // Delivered/Completed). Device/external actuators that can only
            // honestly confirm "acknowledged" STAY acknowledged — completion
            // requires observation (spec: acknowledged ≠ completed).
            _ => {
                if receipt.current_status == ActionStatus::Acknowledged {
                    let confirmation = self.actuator_confirmation(&receipt.actuator_id).await;
                    let ack_means_delivered = matches!(
                        confirmation,
                        interaction_core::ConfirmationLevel::Delivered
                            | interaction_core::ConfirmationLevel::Completed
                            | interaction_core::ConfirmationLevel::Verified
                    );
                    if ack_means_delivered {
                        receipt.verification = Some(VerificationEvidence {
                            observation_ids: vec![],
                            verdict: VerificationVerdict::AcknowledgedOnly,
                            detail: Some(
                                "driver acknowledged; no environmental confirmation".into(),
                            ),
                            verified_at: now,
                        });
                        let _ = receipt.transition(ActionStatus::Completed, now);
                        self.emit_action_event(EventType::ActionCompleted, &receipt, json!({}));
                    } else {
                        receipt.verification = Some(VerificationEvidence {
                            observation_ids: vec![],
                            verdict: VerificationVerdict::AcknowledgedOnly,
                            detail: Some(
                                "device acknowledged the request; completion not confirmed — \
                                 re-verify against observations"
                                    .into(),
                            ),
                            verified_at: now,
                        });
                    }
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
                if !self.persist_receipt(&receipt, channel).await? {
                    return self.store.receipt(&receipt.action_id);
                }
                Ok(receipt)
            }
        }
    }
}
