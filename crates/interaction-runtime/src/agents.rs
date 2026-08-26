//! Agent sessions in the runtime: creation (delegation-checked), the
//! session mailbox, honest task delivery states, lease expiry, closure with
//! bounded handoff, and emergency-stop propagation.
//!
//! Honesty ladder for delegated work:
//!   dispatched  = task placed in the session's mailbox
//!   acknowledged = the session actually FETCHED the task
//!   claimed-completed = the agent SAYS it finished (an inference, never
//!                       verification — the observation stores the claim
//!                       under `inferences`, so the verifier can never treat
//!                       an agent claim as observed evidence)

use crate::runtime::Runtime;
use chrono::Utc;
use interaction_core::{
    check_delegation, validate_handoff, AgentSessionId, AgentSessionRecord, AgentSessionState,
    CapabilityLease, DelegationEnvelope, DomainError, DomainResult, EventType, HandoffSummary,
    MailboxDirection, MailboxMessage, ProviderDescriptor, ProviderId, ProviderIdentity,
    ProviderKind, ProviderState, SessionBudget, TrustLevel,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};

const MAILBOX_CAP: usize = 200;
const MAX_BODY_BYTES: usize = 16 * 1024;

pub struct AgentSessionEntry {
    pub record: AgentSessionRecord,
    pub mailbox: VecDeque<MailboxMessage>,
    next_message: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentSession {
    pub provider_id: Option<String>,
    pub agent_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub ttl_minutes: Option<u32>,
    #[serde(default)]
    pub data_scope: Vec<String>,
    #[serde(default)]
    pub tool_scope: Vec<String>,
    #[serde(default)]
    pub consent_scope: Vec<String>,
    #[serde(default)]
    pub max_cost: Option<f64>,
    #[serde(default)]
    pub max_messages: Option<u32>,
    #[serde(default)]
    pub delegation: Option<DelegationEnvelope>,
}

impl Runtime {
    /// Number of open (leased, unexpired) agent sessions.
    pub async fn open_agent_sessions(&self) -> u32 {
        let now = Utc::now();
        self.agent_sessions
            .read()
            .await
            .values()
            .filter(|e| e.record.state.is_open() && !e.record.lease.is_expired(now))
            .count() as u32
    }

    /// Create a leased agent session. Delegated creations carry an envelope
    /// that is checked deterministically (depth / cycle / budget / count).
    pub async fn create_agent_session(
        &self,
        input: CreateAgentSession,
    ) -> DomainResult<AgentSessionRecord> {
        if self.is_estopped() {
            return Err(DomainError::PolicyBlocked(
                "emergency stop engaged; no new agent sessions".into(),
            ));
        }
        let session_id = AgentSessionId::generate();
        let policy = self.policy().await;
        let open = self.open_agent_sessions().await;
        if let Some(envelope) = &input.delegation {
            check_delegation(envelope, &policy.delegation, session_id.as_str(), open)
                .map_err(DomainError::PolicyBlocked)?;
        } else if open >= policy.delegation.max_sessions {
            return Err(DomainError::PolicyBlocked(format!(
                "too many open agent sessions ({open} ≥ {})",
                policy.delegation.max_sessions
            )));
        }

        let now = Utc::now();
        let ttl = input.ttl_minutes.unwrap_or(120).clamp(1, 24 * 60);
        let provider_id = ProviderId::new(
            input
                .provider_id
                .unwrap_or_else(|| "provider.ai.unspecified".into()),
        );
        let record = AgentSessionRecord {
            session_id: session_id.clone(),
            provider_id: provider_id.clone(),
            agent_id: input.agent_id.clone(),
            label: input.label,
            state: AgentSessionState::Created,
            lease: CapabilityLease {
                issued_at: now,
                expires_at: now + chrono::Duration::minutes(ttl as i64),
                renewable: true,
                revoke_on_session_end: true,
            },
            data_scope: input.data_scope,
            tool_scope: input.tool_scope,
            consent_scope: input.consent_scope,
            budget: SessionBudget {
                max_duration_ms: (ttl as u64) * 60_000,
                max_cost: input.max_cost.unwrap_or(0.0),
                spent_cost: 0.0,
                max_messages: input
                    .max_messages
                    .unwrap_or(policy.delegation.max_messages_per_session),
                spent_messages: 0,
            },
            delegation: input.delegation,
            created_at: now,
            closed_at: None,
            detail: None,
            handoff: None,
        };

        // A session is also a provider (uniform surface for the UI).
        let descriptor = ProviderDescriptor {
            identity: ProviderIdentity {
                id: ProviderId::new(format!("provider.ai-session.{}", session_id.as_str())),
                kind: ProviderKind::AiSession,
                display_name: record
                    .label
                    .clone()
                    .unwrap_or_else(|| input.agent_id.clone()),
                trust_level: TrustLevel::Untrusted,
                origin: provider_id.as_str().to_string(),
                version: String::new(),
                fingerprint: None,
                human: None,
            },
            state: ProviderState::Available,
            receptors: vec!["agent.session".into()],
            actuators: vec!["agent.delegate".into()],
            tool_operations: record.tool_scope.clone(),
            paired_at: None,
            last_seen: Some(now),
            detail: None,
        };
        let _ = self.providers.register(descriptor).await;

        self.persist_agent_session(&record);
        self.agent_sessions.write().await.insert(
            session_id.as_str().to_string(),
            AgentSessionEntry {
                record: record.clone(),
                mailbox: VecDeque::new(),
                next_message: 1,
            },
        );
        self.events.emit(
            EventType::SessionStarted,
            json!({"agentSessionId": session_id.as_str(), "agentId": record.agent_id}),
        );
        Ok(record)
    }

    /// Lazy lease expiry: expired-but-open sessions flip to Expired and lose
    /// their provider surface. Called on every access.
    async fn expire_if_needed(&self, entry: &mut AgentSessionEntry) {
        let now = Utc::now();
        if entry.record.state.is_open() && entry.record.lease.is_expired(now) {
            entry.record.state = AgentSessionState::Expired;
            entry.record.closed_at = Some(now);
            entry.record.detail = Some("lease expired".into());
            self.persist_agent_session(&entry.record);
            let pid = ProviderId::new(format!(
                "provider.ai-session.{}",
                entry.record.session_id.as_str()
            ));
            let _ = self
                .providers
                .transition(&pid, ProviderState::Expired, Some("lease expired".into()))
                .await;
            self.events.emit(
                EventType::SessionStopped,
                json!({"agentSessionId": entry.record.session_id.as_str(), "reason": "expired"}),
            );
        }
    }

    pub async fn list_agent_sessions(&self) -> Vec<AgentSessionRecord> {
        let mut map = self.agent_sessions.write().await;
        let mut out = Vec::new();
        for entry in map.values_mut() {
            self.expire_if_needed(entry).await;
            out.push(entry.record.clone());
        }
        out
    }

    pub async fn get_agent_session(&self, id: &str) -> DomainResult<AgentSessionRecord> {
        let mut map = self.agent_sessions.write().await;
        let entry = map
            .get_mut(id)
            .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
        self.expire_if_needed(entry).await;
        Ok(entry.record.clone())
    }

    /// Renew a renewable lease BEFORE it expires (expired = gone for good).
    pub async fn renew_agent_session(
        &self,
        id: &str,
        extra_minutes: u32,
    ) -> DomainResult<AgentSessionRecord> {
        let mut map = self.agent_sessions.write().await;
        let entry = map
            .get_mut(id)
            .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
        self.expire_if_needed(entry).await;
        if !entry.record.state.is_open() {
            return Err(DomainError::Conflict(format!(
                "agent session {id} is {:?}; expired/closed sessions cannot be renewed",
                entry.record.state
            )));
        }
        if !entry.record.lease.renewable {
            return Err(DomainError::PolicyBlocked("lease is not renewable".into()));
        }
        entry.record.lease.expires_at += chrono::Duration::minutes(extra_minutes.min(240) as i64);
        self.persist_agent_session(&entry.record);
        Ok(entry.record.clone())
    }

    /// Put a message in a session's mailbox (either direction). Budgeted,
    /// bounded, honest: placing a task = dispatched, nothing more.
    pub async fn mailbox_send(
        &self,
        id: &str,
        direction: MailboxDirection,
        kind: &str,
        body: BTreeMap<String, Value>,
        action_id: Option<String>,
    ) -> DomainResult<MailboxMessage> {
        if serde_json::to_vec(&body).map(|v| v.len()).unwrap_or(0) > MAX_BODY_BYTES {
            return Err(DomainError::Validation(format!(
                "mailbox body too large (max {MAX_BODY_BYTES} bytes)"
            )));
        }
        let mut map = self.agent_sessions.write().await;
        let entry = map
            .get_mut(id)
            .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
        self.expire_if_needed(entry).await;
        if !entry.record.state.is_open() {
            return Err(DomainError::Conflict(format!(
                "agent session {id} is {:?}; mailbox closed",
                entry.record.state
            )));
        }
        if !entry.record.budget.messages_left() {
            return Err(DomainError::PolicyBlocked(format!(
                "agent session {id}: message budget exhausted ({})",
                entry.record.budget.max_messages
            )));
        }
        entry.record.budget.spent_messages += 1;
        if entry.mailbox.len() >= MAILBOX_CAP {
            entry.mailbox.pop_front();
        }
        let message = MailboxMessage {
            message_id: format!("msg-{}-{}", id, entry.next_message),
            session_id: entry.record.session_id.clone(),
            direction,
            kind: kind.to_string(),
            body,
            created_at: Utc::now(),
            delivered_at: None,
            action_id,
        };
        entry.next_message += 1;
        entry.mailbox.push_back(message.clone());
        self.persist_agent_session(&entry.record);
        Ok(message)
    }

    /// Fetch messages. When the SESSION fetches its tasks (`to-session`),
    /// delivery becomes real: delivered_at is stamped and any linked action
    /// receipt moves dispatched → acknowledged.
    pub async fn mailbox_fetch(
        &self,
        id: &str,
        direction: MailboxDirection,
    ) -> DomainResult<Vec<MailboxMessage>> {
        let mut acked: Vec<(String, String)> = Vec::new();
        let out = {
            let mut map = self.agent_sessions.write().await;
            let entry = map
                .get_mut(id)
                .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
            self.expire_if_needed(entry).await;
            let now = Utc::now();
            let mut out = Vec::new();
            for m in entry.mailbox.iter_mut() {
                if m.direction == direction {
                    if direction == MailboxDirection::ToSession && m.delivered_at.is_none() {
                        m.delivered_at = Some(now);
                        if let Some(aid) = &m.action_id {
                            acked.push((aid.clone(), m.message_id.clone()));
                        }
                    }
                    out.push(m.clone());
                }
            }
            out
        };
        for (action_id, message_id) in acked {
            let _ = self
                .acknowledge_delegated_action(&action_id, &message_id)
                .await;
        }
        Ok(out)
    }

    async fn acknowledge_delegated_action(
        &self,
        action_id: &str,
        message_id: &str,
    ) -> DomainResult<()> {
        let mut receipt = self
            .store
            .receipt(&interaction_core::ActionId::new(action_id))?;
        if receipt.current_status == interaction_core::ActionStatus::Dispatched {
            let _ = receipt.transition(interaction_core::ActionStatus::Acknowledged, Utc::now());
            receipt
                .driver_response
                .insert("deliveredMessage".into(), json!(message_id));
            self.store.upsert_receipt(&receipt, "agent")?;
            self.emit_action_event(EventType::ActionAcknowledged, &receipt, json!({}));
        }
        Ok(())
    }

    /// The session reports its own state. Claims stay claims: the observation
    /// carries the payload under `inferences`, and any linked action id is
    /// stored as `claimActionId` (NOT `actionId`) so verification can never
    /// mistake an agent claim for observed evidence.
    pub async fn report_agent_session(
        &self,
        id: &str,
        event: &str,
        payload: Value,
    ) -> DomainResult<AgentSessionRecord> {
        let next_state = match event {
            "task-started" | "progress" => AgentSessionState::Active,
            "waiting-for-input" => AgentSessionState::WaitingForInput,
            "waiting-for-consent" => AgentSessionState::WaitingForConsent,
            "claimed-completed" => AgentSessionState::ClaimedCompleted,
            "failed" => AgentSessionState::Failed,
            "timed-out" => AgentSessionState::TimedOut,
            "cancelled" => AgentSessionState::Cancelled,
            other => {
                return Err(DomainError::Validation(format!(
                    "unknown session event {other:?}"
                )))
            }
        };
        let record = {
            let mut map = self.agent_sessions.write().await;
            let entry = map
                .get_mut(id)
                .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
            self.expire_if_needed(entry).await;
            if !entry.record.state.is_open() {
                return Err(DomainError::Conflict(format!(
                    "agent session {id} is {:?}; reports are closed",
                    entry.record.state
                )));
            }
            entry.record.state = next_state;
            self.persist_agent_session(&entry.record);
            entry.record.clone()
        };

        // Session-as-receptor: the report becomes an observation. Facts are
        // only what actually happened (a report arrived); the content is an
        // inference (the agent's own claim).
        let mut facts = BTreeMap::new();
        facts.insert("sessionId".to_string(), json!(id));
        facts.insert("event".to_string(), json!(event));
        let mut inferences = BTreeMap::new();
        if !payload.is_null() {
            let mut claim = payload;
            // Defensive: an agent must not smuggle an `actionId` fact.
            if let Some(obj) = claim.as_object_mut() {
                if let Some(aid) = obj.remove("actionId") {
                    obj.insert("claimActionId".into(), aid);
                }
            }
            inferences.insert("report".to_string(), claim);
        }
        self.ingest("agent.session", facts, inferences, 1.0).await?;
        Ok(record)
    }

    /// Close a session: consents die, provider surface closes, undelivered
    /// tasks are cancelled, and ONLY a bounded handoff survives.
    pub async fn close_agent_session(
        &self,
        id: &str,
        handoff: Option<HandoffSummary>,
        reason: &str,
    ) -> DomainResult<AgentSessionRecord> {
        if let Some(h) = &handoff {
            validate_handoff(h).map_err(DomainError::Validation)?;
        }
        let record = {
            let mut map = self.agent_sessions.write().await;
            let entry = map
                .get_mut(id)
                .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
            let now = Utc::now();
            entry.record.state = match reason {
                "cancelled" => AgentSessionState::Cancelled,
                _ => AgentSessionState::Closed,
            };
            entry.record.closed_at = Some(now);
            entry.record.consent_scope.clear(); // consents die with the session
            entry.record.handoff = handoff;
            entry.record.detail = Some(reason.to_string());
            // Undelivered tasks are dead, honestly.
            entry.mailbox.retain(|m| m.delivered_at.is_some());
            self.persist_agent_session(&entry.record);
            entry.record.clone()
        };
        let pid = ProviderId::new(format!("provider.ai-session.{id}"));
        let _ = self
            .providers
            .transition(&pid, ProviderState::Closed, Some(reason.to_string()))
            .await;
        self.store.audit(
            "agent-session.closed",
            "user",
            &json!({"agentSessionId": id, "reason": reason}),
        )?;
        self.events.emit(
            EventType::SessionStopped,
            json!({"agentSessionId": id, "reason": reason}),
        );
        Ok(record)
    }

    /// Emergency stop propagation: every open session is cancelled and gets a
    /// cancel message; nothing resumes automatically.
    pub(crate) async fn estop_agent_sessions(&self) {
        let ids: Vec<String> = {
            let map = self.agent_sessions.read().await;
            map.values()
                .filter(|e| e.record.state.is_open())
                .map(|e| e.record.session_id.as_str().to_string())
                .collect()
        };
        for id in ids {
            let _ = self
                .mailbox_send(
                    &id,
                    MailboxDirection::ToSession,
                    "cancel",
                    BTreeMap::from([(
                        "reason".to_string(),
                        json!("emergency stop — stop all work now"),
                    )]),
                    None,
                )
                .await;
            let _ = self.close_agent_session(&id, None, "cancelled").await;
        }
    }

    pub(crate) fn persist_agent_session(&self, record: &AgentSessionRecord) {
        if let Ok(body) = serde_json::to_string(record) {
            let _ = self
                .store
                .save_agent_session(record.session_id.as_str(), &body);
        }
    }

    /// Restore persisted session records (closed history + expire leftovers).
    pub(crate) async fn restore_agent_sessions(&self) {
        let Ok(bodies) = self.store.all_agent_sessions() else {
            return;
        };
        let mut map = self.agent_sessions.write().await;
        for body in bodies {
            if let Ok(mut record) = serde_json::from_str::<AgentSessionRecord>(&body) {
                // Open sessions do NOT survive a restart: the lease's host
                // context is gone. Mark them expired, honestly.
                if record.state.is_open() {
                    record.state = AgentSessionState::Expired;
                    record.closed_at = Some(Utc::now());
                    record.detail = Some("runtime restarted".into());
                    if let Ok(body) = serde_json::to_string(&record) {
                        let _ = self
                            .store
                            .save_agent_session(record.session_id.as_str(), &body);
                    }
                }
                map.insert(
                    record.session_id.as_str().to_string(),
                    AgentSessionEntry {
                        record,
                        mailbox: VecDeque::new(),
                        next_message: 1,
                    },
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// agent.delegate actuator: delegation goes through the SAME plan → governor →
// receipt pipeline as every other side effect. The receipt is honest:
// dispatched = in the mailbox; acknowledged only when the session fetches.
// ---------------------------------------------------------------------------

pub struct DelegateActuator {
    inner: std::sync::Weak<crate::runtime::RuntimeInner>,
}

impl DelegateActuator {
    pub fn new(inner: std::sync::Weak<crate::runtime::RuntimeInner>) -> Self {
        Self { inner }
    }

    fn runtime(&self) -> Option<Runtime> {
        self.inner.upgrade().map(Runtime::from_inner)
    }
}

#[async_trait::async_trait]
impl interaction_core::Actuator for DelegateActuator {
    fn manifest(&self) -> interaction_core::ActuatorManifest {
        use interaction_core::{ConfirmationLevel, EffectSemantics, HumanMeta, TriState};
        interaction_adapter_sdk::ActuatorManifestBuilder::new(
            "agent.delegate",
            "Delegate to agent session",
            "agent",
            "builtin.agent",
        )
        .description("Places a task in an agent session's mailbox. Dispatched = queued; acknowledged = the session fetched it; completion claims are never auto-verified.")
        .risk(interaction_core::RiskClass::BoundedSideEffect)
        .requires_consent(true)
        .human(HumanMeta {
            effect: Some(EffectSemantics {
                affects: vec!["agent-session".into()],
                external_side_effect: TriState::Unknown,
                physical_effect: TriState::No,
                reversible: TriState::Unknown,
                confirmation_level: ConfirmationLevel::Acknowledged,
                ..Default::default()
            }),
            ..Default::default()
        })
        .build()
    }

    async fn execute(
        &self,
        action: interaction_core::BoundedAction,
    ) -> Result<interaction_core::ActionReceipt, interaction_core::ActuatorError> {
        use interaction_core::ActuatorError;
        let runtime = self
            .runtime()
            .ok_or_else(|| ActuatorError::Unavailable("runtime shutting down".into()))?;
        let extra = action.effective.extra.clone().unwrap_or(Value::Null);
        let session_id = extra
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ActuatorError::Rejected(
                    "agent.delegate needs payload.sessionId (target agent session)".into(),
                )
            })?
            .to_string();
        let task = extra
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or(&action.intent)
            .to_string();
        let mut body = BTreeMap::new();
        body.insert("task".to_string(), json!(task));
        if let Some(env) = extra.get("delegation") {
            body.insert("delegation".to_string(), env.clone());
        }
        match runtime
            .mailbox_send(
                &session_id,
                MailboxDirection::ToSession,
                "task",
                body,
                Some(action.action_id.as_str().to_string()),
            )
            .await
        {
            Ok(message) => Ok(
                interaction_adapter_sdk::DriverReceipt::start(&action, Utc::now())
                    .dispatched()
                    .note("messageId", json!(message.message_id))
                    .note("agentSessionId", json!(session_id))
                    .finish(),
            ),
            Err(e) => Ok(
                interaction_adapter_sdk::DriverReceipt::start(&action, Utc::now())
                    .failed("delegation-refused", &e.to_string())
                    .finish(),
            ),
        }
    }

    async fn status(&self) -> interaction_core::ComponentHealth {
        interaction_core::ComponentHealth::healthy().at(Utc::now())
    }

    async fn cancel(
        &self,
        action_id: &interaction_core::ActionId,
    ) -> Result<interaction_core::ActionReceipt, interaction_core::ActuatorError> {
        Err(interaction_core::ActuatorError::NotFound(format!(
            "{action_id}: delegated tasks are cancelled by closing the session"
        )))
    }

    async fn emergency_stop(&self) -> Result<(), interaction_core::ActuatorError> {
        // Session cancellation is propagated by the runtime's estop path.
        Ok(())
    }
}
