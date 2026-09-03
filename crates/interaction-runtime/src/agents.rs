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
    check_delegation, validate_handoff, AgentContextBundleReceipt, AgentSessionId,
    AgentSessionRecord, AgentSessionState, CapabilityLease, DelegationEnvelope, DomainError,
    DomainResult, EventType, HandoffSummary, MailboxDirection, MailboxMessage, ProviderDescriptor,
    ProviderId, ProviderIdentity, ProviderKind, ProviderState, SessionBudget, TrustLevel,
};
use rand::RngCore;
use serde_json::{json, Value};

/// 誰在讀 agent session 的信箱。
///
/// 誠實階梯（dispatched ≠ acknowledged）在這裡有牙齒：只有 `Agent` 這一側
/// 的讀取算「送達」（蓋 deliveredAt、把委派 receipt 推到 acknowledged）。
/// 人類用 human token 的 GET、桌面 UI 的檢視一律是 `Human`——純觀看，
/// 不改任何狀態，也不發 `fetched` 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailboxReader {
    /// Agent 本人（agent／session token，或 agent host 程序）。
    Agent,
    /// 人類觀察者：唯讀。
    Human,
}
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAILBOX_CAP: usize = 200;
const MAX_BODY_BYTES: usize = 16 * 1024;
const CONTEXT_BUNDLE_RECEIPT_CAP: usize = 32;
/// 緊急停止時，單一 session 的「收尾」（狀態落地、consent 撤銷、provider
/// 關閉）上限。程序樹終止本身走鎖外路徑、不受這個期限影響；這個期限只是
/// 保證一個卡住的紀錄 I/O 不會拖住其他 session 的停止。
const ESTOP_SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Authorization carried by one memory-only Agent Session token. The token
/// itself is never stored; the map key is its SHA-256 digest.
#[derive(Debug, Clone)]
pub struct AgentSessionCapability {
    pub session_id: String,
    pub agent_id: String,
    pub tool_scope: BTreeSet<String>,
    pub domains: BTreeSet<String>,
    pub expires_at: chrono::DateTime<Utc>,
}

impl AgentSessionCapability {
    pub fn allows_tool(&self, canonical_name: &str) -> bool {
        let operation = canonical_name
            .strip_prefix("interaction.knowledge_")
            .map(|suffix| format!("knowledge.{}", suffix.replace('_', "-")))
            .unwrap_or_else(|| canonical_name.to_string());
        let is_read = matches!(
            canonical_name,
            "interaction.knowledge_search"
                | "interaction.knowledge_get"
                | "interaction.knowledge_get_source"
                | "interaction.knowledge_expand_graph"
        );
        let is_propose = canonical_name.starts_with("interaction.knowledge_propose_")
            || canonical_name == "interaction.knowledge_submit_review";
        self.tool_scope.contains(canonical_name)
            || self.tool_scope.contains(&operation)
            || self.tool_scope.contains("knowledge.*")
            || (is_read && self.tool_scope.contains("knowledge.read"))
            || (is_propose && self.tool_scope.contains("knowledge.propose"))
    }

    pub fn allows_domain(&self, domains: &[String]) -> bool {
        self.domains.contains("*")
            || (!domains.is_empty() && domains.iter().any(|domain| self.domains.contains(domain)))
    }
}

fn capability_digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

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
    /// Human-requested, lease-bounded workspace write access. It is accepted only
    /// for a gateway agent with an explicit workdir plus matching tool/consent scopes.
    #[serde(default)]
    pub allow_write: bool,
    #[serde(default)]
    pub max_cost: Option<f64>,
    #[serde(default)]
    pub max_messages: Option<u32>,
    #[serde(default)]
    pub delegation: Option<DelegationEnvelope>,
    /// Gateway agents（codex/claude-code）的工作目錄；預設唯讀模式。
    #[serde(default)]
    pub workdir: Option<String>,
    /// 續開既有 provider session（claude --resume / codex thread resume）。
    /// 只對 gateway agents 有意義；resume 不會放寬任何 scope——新 session
    /// 仍走完整的 lease/consent/sandbox 檢查。
    #[serde(default)]
    pub resume_provider_session_id: Option<String>,
}

impl Runtime {
    /// Mint/rotate a short-lived capability for one already-authorized Agent
    /// Session. Only trusted host integration code receives the plaintext;
    /// persistence, receipts, logs, and UI never do.
    pub async fn issue_agent_session_capability(&self, id: &str) -> DomainResult<String> {
        let record = self.get_agent_session(id).await?;
        if !record.state.is_open() || record.lease.is_expired(Utc::now()) {
            return Err(DomainError::Expired(format!("agent session {id}")));
        }
        let mut random = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut random);
        let token = format!(
            "iat-session-{}",
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let capability = AgentSessionCapability {
            session_id: id.to_string(),
            agent_id: record.agent_id,
            tool_scope: record.tool_scope.into_iter().collect(),
            domains: record
                .data_scope
                .into_iter()
                .filter_map(|scope| scope.strip_prefix("domain:").map(String::from))
                .collect(),
            expires_at: record.lease.expires_at,
        };
        let mut capabilities = self.agent_session_capabilities.write().await;
        capabilities
            .retain(|_, existing| existing.session_id != id && existing.expires_at > Utc::now());
        capabilities.insert(capability_digest(&token), capability);
        self.store.audit(
            "agent-session.capability-issued",
            "runtime",
            &json!({"agentSessionId": id, "expiresAt": record.lease.expires_at}),
        )?;
        Ok(token)
    }

    /// Validate without revealing whether a token ever existed. Also checks
    /// the current live session, so close/expiry revocation is immediate.
    pub async fn agent_session_capability(&self, token: &str) -> Option<AgentSessionCapability> {
        let digest = capability_digest(token);
        let capability = self
            .agent_session_capabilities
            .read()
            .await
            .get(&digest)
            .cloned()?;
        if capability.expires_at <= Utc::now() {
            self.agent_session_capabilities
                .write()
                .await
                .remove(&digest);
            return None;
        }
        let record = self.get_agent_session(&capability.session_id).await.ok()?;
        if !record.state.is_open() || record.lease.is_expired(Utc::now()) {
            self.agent_session_capabilities
                .write()
                .await
                .remove(&digest);
            return None;
        }
        Some(capability)
    }

    async fn revoke_agent_session_capabilities(&self, id: &str) {
        self.agent_session_capabilities
            .write()
            .await
            .retain(|_, capability| capability.session_id != id);
    }

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
        // Serialize creation so the estop/count checks are not TOCTOU: two
        // racing creates cannot both pass a near-limit count, and a create
        // cannot slip in after an emergency stop's session sweep.
        let _create_guard = self.agent_create_lock.lock().await;
        if self.is_estopped() {
            return Err(DomainError::PolicyBlocked(
                "emergency stop engaged; no new agent sessions".into(),
            ));
        }
        if self
            .ui_preferences()
            .await
            .disabled_agents
            .iter()
            .any(|agent| agent == &input.agent_id)
        {
            return Err(DomainError::PolicyBlocked(format!(
                "agent connector {} is disabled by the user",
                input.agent_id
            )));
        }
        if input.allow_write {
            if crate::gateway::agent_kind_for(&input.agent_id).is_none() {
                return Err(DomainError::Validation(
                    "write-enabled session 只支援 Codex／Claude Code gateway agent".into(),
                ));
            }
            let workdir = input.workdir.as_deref().unwrap_or_default();
            if workdir.trim().is_empty() {
                return Err(DomainError::Validation(
                    "write-enabled session 必須明確指定 workdir".into(),
                ));
            }
            if !input.tool_scope.iter().any(|s| s == "workspace.write") {
                return Err(DomainError::ConsentRequired(
                    "write-enabled session 缺少 toolScope workspace.write".into(),
                ));
            }
            if !input
                .consent_scope
                .iter()
                .any(|s| s == "agent-session:workspace-write")
            {
                return Err(DomainError::ConsentRequired(
                    "write-enabled session 缺少 agent-session:workspace-write 使用授權".into(),
                ));
            }
        }
        let session_id = AgentSessionId::generate();
        let policy = self.policy().await;
        let open = self.open_agent_sessions().await;
        if let Some(envelope) = &input.delegation {
            check_delegation(envelope, &policy.delegation, session_id.as_str(), open)
                .map_err(DomainError::PolicyBlocked)?;
            // Deterministic fan-out cap that does NOT depend on caller-honest
            // hop_count: bound the number of open sessions sharing this
            // delegation tree's rootTaskId by max_parallel.
            let same_tree = self
                .agent_sessions
                .read()
                .await
                .values()
                .filter(|e| {
                    e.record.state.is_open()
                        && e.record
                            .delegation
                            .as_ref()
                            .map(|d| d.root_task_id == envelope.root_task_id)
                            .unwrap_or(false)
                })
                .count() as u32;
            if same_tree >= policy.delegation.max_parallel {
                return Err(DomainError::PolicyBlocked(format!(
                    "delegation tree {} already has {same_tree} open sessions (max_parallel {})",
                    envelope.root_task_id, policy.delegation.max_parallel
                )));
            }
        } else if open >= policy.delegation.max_sessions {
            return Err(DomainError::PolicyBlocked(format!(
                "too many open agent sessions ({open} ≥ {})",
                policy.delegation.max_sessions
            )));
        }

        let now = Utc::now();
        let ttl = input.ttl_minutes.unwrap_or(120).clamp(1, 24 * 60);
        let provider_id = ProviderId::new(input.provider_id.unwrap_or_else(|| {
            crate::gateway::agent_kind_for(&input.agent_id)
                .map(|kind| kind.provider_id().to_string())
                .unwrap_or_else(|| "provider.ai.unspecified".into())
        }));
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
            allow_write: input.allow_write,
            budget: SessionBudget {
                max_duration_ms: (ttl as u64) * 60_000,
                max_cost: input.max_cost.unwrap_or(0.0),
                spent_cost: 0.0,
                // effective limit = min(requested, policy). A caller-supplied
                // 0 (or a value above policy) can never widen the ceiling — 0
                // means "use the policy default", not "unlimited".
                max_messages: {
                    let policy_max = policy.delegation.max_messages_per_session;
                    match input.max_messages {
                        None | Some(0) => policy_max,
                        Some(n) => n.min(policy_max),
                    }
                },
                spent_messages: 0,
            },
            delegation: input.delegation,
            created_at: now,
            closed_at: None,
            detail: None,
            handoff: None,
            provider_session_id: None,
            claim_id: None,
            human_verified: None,
            context_bundles: vec![],
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
        // v0.5 角色 taxonomy：queued（session 已建立、任務尚未被取走）。
        self.emit_agent_session_state(session_id.as_str(), &record.agent_id, "created");

        // Gateway agents（codex/claude-code）：掛真實子程序。失敗＝建立失敗
        // （誠實），不留下看似可用其實沒有 agent 的 session。
        let record = if let Some(kind) = crate::gateway::agent_kind_for(&record.agent_id) {
            let session_capability_token = self
                .issue_agent_session_capability(session_id.as_str())
                .await?;
            match self
                .gateway_attach(
                    kind,
                    &record,
                    input.workdir.clone(),
                    session_capability_token,
                    input.resume_provider_session_id.clone(),
                )
                .await
            {
                Ok(provider_sid) => {
                    let updated = {
                        let mut map = self.agent_sessions.write().await;
                        match map.get_mut(session_id.as_str()) {
                            Some(entry) => {
                                // 事件泵可能已用 init 事件回填 provider session
                                // id——attach 的回傳值（claude 是 None）不得
                                // 把它蓋掉。
                                entry.record.provider_session_id =
                                    provider_sid.or(entry.record.provider_session_id.take());
                                self.persist_agent_session(&entry.record);
                                entry.record.clone()
                            }
                            None => record,
                        }
                    };
                    // 子程序的 pgid 立刻落地（meta）：daemon 崩潰或非正常結束
                    // 時，下次啟動的 restore 才找得回這棵孤兒程序樹。
                    self.record_gateway_pgid(session_id.as_str()).await;
                    updated
                }
                Err(e) => {
                    let _ = self
                        .close_agent_session(session_id.as_str(), None, "connector-failed")
                        .await;
                    return Err(e);
                }
            }
        } else {
            record
        };
        // estop 旗標的寫入不經 agent_create_lock，可能在本次建立途中亮起；
        // estop 的 session 快照會等本 create 釋放鎖後補收這個 session，但
        // 呼叫端不該拿到一個註定被清掉的 session——回傳前再檢查一次，
        // 命中就回滾：殺子程序、關 session、誠實拒絕。
        if self.is_estopped() {
            self.gateway_spawn_kill(session_id.as_str(), "estop-during-create");
            let _ = self
                .close_agent_session(session_id.as_str(), None, "cancelled")
                .await;
            return Err(DomainError::PolicyBlocked(
                "emergency stop engaged; no new agent sessions".into(),
            ));
        }
        Ok(record)
    }

    /// 委派 receipt 的 ack（gateway 轉送時使用）。
    pub(crate) async fn acknowledge_delegated_action_public(
        &self,
        action_id: &str,
        message_id: &str,
    ) -> DomainResult<()> {
        self.acknowledge_delegated_action(action_id, message_id)
            .await
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
            // 角色 taxonomy：租約到期是「時間到了，工作沒收尾」——必須跟
            // session.stopped 一起發出，否則小樞會停在最後一個假象狀態
            // （例如永遠的「工作中」），感測／狀態就靜默了。
            self.emit_agent_session_state(
                entry.record.session_id.as_str(),
                &entry.record.agent_id,
                "timed-out",
            );
            self.revoke_agent_session_capabilities(entry.record.session_id.as_str())
                .await;
        }
    }

    /// 立刻讓租約到期（人類／營運端的縮短動作）。只會縮短、永不延長，
    /// 走的是與惰性到期完全相同的路徑，所以 timed-out taxonomy 事件、
    /// provider 收攤與 capability 撤銷都與真實到期一模一樣。
    pub async fn expire_agent_session_lease(&self, id: &str) -> DomainResult<AgentSessionRecord> {
        let mut map = self.agent_sessions.write().await;
        let entry = map
            .get_mut(id)
            .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
        if entry.record.state.is_open() {
            entry.record.lease.expires_at = Utc::now();
        }
        self.expire_if_needed(entry).await;
        Ok(entry.record.clone())
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
        mut body: BTreeMap<String, Value>,
        action_id: Option<String>,
    ) -> DomainResult<MailboxMessage> {
        // A task's actual Context Bundle is computed at dispatch time from the
        // immutable session lease. Caller-supplied `contextBundle` is replaced,
        // so an agent cannot forge the "what was provided" evidence.
        let context_bundle = if direction == MailboxDirection::ToSession && kind == "task" {
            let (agent_id, domains) = {
                let map = self.agent_sessions.read().await;
                let entry = map
                    .get(id)
                    .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
                let domains = entry
                    .record
                    .data_scope
                    .iter()
                    .filter_map(|scope| scope.strip_prefix("domain:").map(String::from))
                    .collect::<Vec<_>>();
                (entry.record.agent_id.clone(), domains)
            };
            let task = body
                .get("task")
                .or_else(|| body.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let bundle = self
                .memory_context_bundle(task, &domains, &agent_id)
                .await?;
            body.insert("contextBundle".into(), bundle.clone());
            Some(bundle)
        } else {
            None
        };
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
        if let Some(bundle) = context_bundle {
            let canonical = serde_json::to_vec(&bundle)
                .map_err(|e| DomainError::Internal(format!("serialize context bundle: {e}")))?;
            let evidence = AgentContextBundleReceipt {
                bundle_id: format!("bundle-{}", uuid::Uuid::new_v4()),
                message_id: message.message_id.clone(),
                generated_at: Utc::now(),
                content_hash: format!("{:x}", Sha256::digest(&canonical)),
                bundle,
            };
            if entry.record.context_bundles.len() >= CONTEXT_BUNDLE_RECEIPT_CAP {
                entry.record.context_bundles.remove(0);
            }
            entry.record.context_bundles.push(evidence);
        }
        self.persist_agent_session(&entry.record);
        drop(map);
        // Gateway session：ToSession 任務即時送進真實 agent 子程序
        // （送達成功才補 delivered 戳記；非 gateway session 維持輪詢流程）。
        // 沒送到（上一輪還在跑、stdin 阻塞、預算用盡、子程序已不在）就把
        // 錯誤交給呼叫端：訊息留在信箱、沒有 delivered 戳記，session 狀態
        // 不因「沒送到」而改寫——「未送達」不是「任務失敗」。
        if direction == MailboxDirection::ToSession {
            self.gateway_deliver(id, &message).await?;
        }
        Ok(message)
    }

    /// 誰在讀這個信箱。送達（delivered）語意**只屬於 agent**：
    /// 人類「看過信箱」不是「agent 收到任務」，把 GET 當成送達會讓
    /// dispatched 直接跳成 acknowledged，等於用觀看偽造送達證據。
    pub async fn mailbox_read(
        &self,
        id: &str,
        direction: MailboxDirection,
        reader: MailboxReader,
    ) -> DomainResult<Vec<MailboxMessage>> {
        match reader {
            MailboxReader::Human => self.mailbox_peek(id, direction).await,
            MailboxReader::Agent => self.mailbox_fetch(id, direction).await,
        }
    }

    /// 讀取信箱但**不**標記送達（UI 檢視／測試用；送達語意只屬於 fetch）。
    pub async fn mailbox_peek(
        &self,
        id: &str,
        direction: MailboxDirection,
    ) -> DomainResult<Vec<MailboxMessage>> {
        let map = self.agent_sessions.read().await;
        let entry = map
            .get(id)
            .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
        Ok(entry
            .mailbox
            .iter()
            .filter(|m| m.direction == direction)
            .cloned()
            .collect())
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
            // Gateway session（codex/claude-code）的送達語意只屬於
            // gateway_deliver——子程序真的收到訊息才算 delivered。這裡的
            // fetch 對 gateway session 是純觀看（GET messages 任何持 token
            // 的觀察者都能呼叫，子程序從不輪詢這個端點）：蓋 delivered／
            // 推進 receipt 會把「有人看過信箱」偽裝成「agent 已收到任務」，
            // 違反誠實階梯（dispatched≠acknowledged）。
            let observer_only = crate::gateway::agent_kind_for(&entry.record.agent_id).is_some();
            let now = Utc::now();
            let mut out = Vec::new();
            let mut newly_delivered = false;
            for m in entry.mailbox.iter_mut() {
                if m.direction == direction {
                    if direction == MailboxDirection::ToSession
                        && !observer_only
                        && m.delivered_at.is_none()
                    {
                        m.delivered_at = Some(now);
                        newly_delivered = true;
                        if let Some(aid) = &m.action_id {
                            acked.push((aid.clone(), m.message_id.clone()));
                        }
                    }
                    out.push(m.clone());
                }
            }
            // 新任務送達 ⇒ 舊的人工驗證只屬於上一個 claim，不再顯示為
            // 「目前這個」已確認。
            if newly_delivered && entry.record.human_verified.take().is_some() {
                self.persist_agent_session(&entry.record);
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
    /// v0.5 角色 taxonomy 事件：created/fetched/working/waiting-input/
    /// waiting-consent/claimed-completed/verified/failed/unknown/timed-out/
    /// cancelled/closed。小樞依這些「真實事件」演出；`verified` 只會由
    /// verify_agent_session（human-only）發出，`unknown` 表示結果未知
    /// （既不演成功也不演失敗）。
    pub(crate) fn emit_agent_session_state(&self, id: &str, agent_id: &str, state: &str) {
        self.events.emit(
            EventType::AgentSessionState,
            json!({"agentSessionId": id, "agentId": agent_id, "state": state}),
        );
        // Character Protocol §11：同一批真實事件投影成 Character Intent
        // （correlationId = agentSessionId；verified 只會從這條人工驗證路徑來）。
        self.character_project_session(id, state);
    }

    /// 人工驗證 agent 的 claimed-completed（human token 專屬路由）。
    /// claim ≠ verified：只有這裡能把 session 升級為 verified，
    /// 角色也只在收到 `verified` 事件後才播放綠勾演出。
    pub async fn verify_agent_session(
        &self,
        id: &str,
        note: Option<String>,
    ) -> DomainResult<AgentSessionRecord> {
        if let Some(n) = &note {
            if n.chars().count() > 500 {
                return Err(DomainError::Validation(
                    "verification note is too long".into(),
                ));
            }
        }
        let record = {
            let mut map = self.agent_sessions.write().await;
            let entry = map
                .get_mut(id)
                .ok_or_else(|| DomainError::NotFound(format!("agent session {id}")))?;
            if entry.record.state != AgentSessionState::ClaimedCompleted {
                return Err(DomainError::Conflict(format!(
                    "agent session {id} is {:?}; only a claimed-completed session can be verified",
                    entry.record.state
                )));
            }
            if entry.record.human_verified.is_some() {
                return Err(DomainError::Conflict(format!(
                    "agent session {id} is already verified"
                )));
            }
            // 驗證綁定「當下這個 claim」：新任務送達／新一輪工作／新的聲稱
            // 都會清掉它，所以第二輪的聲稱永遠得重新驗證。
            entry.record.human_verified = Some(interaction_core::HumanVerification {
                at: Utc::now(),
                note,
                claim_id: entry.record.claim_id.clone(),
            });
            self.persist_agent_session(&entry.record);
            entry.record.clone()
        };
        self.store.audit(
            "agent-session.verified",
            "user",
            &json!({"agentSessionId": id}),
        )?;
        self.emit_agent_session_state(id, &record.agent_id, "verified");
        // 手機的綠勾只能從這裡出發：human verify（不經 plan／policy／AI 路徑，
        // `map_wire_params` 對 `verified-success` 一律拒絕）。背景直送，
        // 沒有手機連線就誠實留在 debug log，不影響驗證本身。
        {
            let runtime = self.clone();
            let session_id = id.to_string();
            tokio::spawn(async move {
                if let Err(e) = runtime.mobile_present_verified(&session_id).await {
                    tracing::debug!(
                        error = %e,
                        agent_session = %session_id,
                        "verified state was not shown on a paired iPhone"
                    );
                }
            });
        }
        Ok(record)
    }

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
            // 結果未知：工作結束了，但既沒有聲稱也沒有可觀察的錯誤。
            // 不是成功、不是失敗——誠實階梯不容許在這裡二選一。
            "unknown" => AgentSessionState::Unknown,
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
            // 每一個新的聲稱都是新的 claim（拿新 id）；而**任何**更新的
            // agent 自我回報——新一輪工作、新的聲稱、失敗／未知——都讓
            // 舊的人工驗證失效：人類確認的是上一個 claim，不是這一個。
            if next_state == AgentSessionState::ClaimedCompleted {
                entry.record.claim_id = Some(format!("claim-{}", uuid::Uuid::new_v4()));
            }
            entry.record.human_verified = None;
            self.persist_agent_session(&entry.record);
            entry.record.clone()
        };

        // 角色 taxonomy 事件（agent 的自我回報照實轉譯；claim 不升級）。
        //
        // 狀態已經落地，事件就**必須**發得出去——所以先發事件，再做 receptor
        // ingest。regression（agent-honesty）：舊版把 `ingest(...).await?` 排在
        // 前面，`agent.session` push receptor 被停用／不存在時（例如中斷的同時
        // 感測層被關掉）`?` 會提早返回，cancelled／failed／unknown／timed-out
        // 因此**完全靜默**：SSE 從中斷前的序號重放只會停在 working，每一個即時
        // 畫面都停在舊狀態直到重新載入。觀察管線失敗不得吃掉狀態真相。
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
        let taxonomy = match event {
            "task-started" | "progress" => "working",
            "waiting-for-input" => "waiting-input",
            "waiting-for-consent" => "waiting-consent",
            other => other, // claimed-completed / failed / unknown / timed-out / cancelled
        };
        // 狀態真相已落地：事件在觀察管線之前發出，receptor 停用／缺席時
        // 只影響下面的 ingest（誠實回 Err 給回報者），不會吞掉狀態事件。
        self.emit_agent_session_state(id, &record.agent_id, taxonomy);
        // The FACT (a report arrived) is certain; the agent's CLAIM inside it is
        // not. Confidence describes the inferences, so a self-report carries a
        // deliberately moderate 0.5 — never 1.0 — so fusion/uncertainty gates
        // never treat an unverified claim as unambiguous evidence.
        self.ingest("agent.session", facts, inferences, 0.5).await?;
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
            // 先做惰性過期再擋重複關閉：租約已過期的 session 誠實地以
            // Expired 收場（expire_if_needed 已持久化＋發事件），不得被改寫
            // 成 Closed/Cancelled。session 生命週期的終局以 closed_at 為準
            // ——只有 close／expiry／restore 會設它；report 造成的 Failed/
            // TimedOut/Cancelled 是「任務結局」，仍需要 close 來收尾（清
            // consent、關 provider、記經驗）。已有 closed_at 的 session 不得
            // 再關：terminal 狀態不可翻轉，handoff 與經驗記錄不得被第二次
            // 關閉抹掉或重複產生。
            self.expire_if_needed(entry).await;
            if entry.record.closed_at.is_some() {
                return Err(DomainError::Conflict(format!(
                    "agent session {id} is {:?}; already closed",
                    entry.record.state
                )));
            }
            let now = Utc::now();
            let prior_state = entry.record.state;
            // 任務結局（Failed／Unknown／TimedOut／Cancelled）是誠實階梯上的
            // 事實，關閉不得把它改寫成「已關閉」——否則失敗／未知這個結局
            // 從主要狀態消失，只剩 detail 裡一行 `(was Failed)`。close 只負責
            // 收尾（closed_at、consent、provider、經驗記錄）。
            entry.record.state = match (reason, prior_state) {
                (
                    _,
                    AgentSessionState::Failed
                    | AgentSessionState::Unknown
                    | AgentSessionState::TimedOut
                    | AgentSessionState::Cancelled,
                ) => prior_state,
                ("cancelled", _) => AgentSessionState::Cancelled,
                _ => AgentSessionState::Closed,
            };
            entry.record.detail = Some(format!("{reason} (was {prior_state:?})"));
            entry.record.closed_at = Some(now);
            // Consents die with the session unless the lease explicitly opts
            // out (revoke_on_session_end). Default is true, so this honors the
            // lease flag rather than leaving it a dead knob.
            if entry.record.lease.revoke_on_session_end {
                entry.record.consent_scope.clear();
            }
            entry.record.handoff = handoff;
            // Undelivered tasks are dead, honestly.
            entry.mailbox.retain(|m| m.delivered_at.is_some());
            self.persist_agent_session(&entry.record);
            // Gateway session：關閉即終止子程序樹（絕不留孤兒）。
            self.gateway_spawn_kill(id, "session-closed");
            (entry.record.clone(), prior_state)
        };
        let (record, prior_state) = record;
        self.revoke_agent_session_capabilities(id).await;
        // kill 已排入（SIGTERM→寬限→SIGKILL）；忘掉 pgid 記錄，重啟時不再
        // 對這個（屆時可能已被重用的）pid 送訊號。殘餘風險：daemon 在 kill
        // 寬限期內崩潰，這棵子程序樹可能存活且不再被 restore 找回。
        self.forget_gateway_pgid(id);
        // Handoff 摘要落入記憶層（AgentHandoff，30 天保存；bounded 已驗證）。
        if let Some(h) = &record.handoff {
            let content = serde_json::to_string_pretty(h).unwrap_or_default();
            let item = interaction_core::new_memory_item(
                interaction_core::MemoryLayer::AgentHandoff,
                interaction_core::MemoryKind::Inference,
                format!(
                    "Handoff：{}",
                    record
                        .label
                        .clone()
                        .unwrap_or_else(|| record.agent_id.clone())
                ),
                content,
                interaction_core::MemoryActor::Runtime,
                Utc::now(),
            );
            let mut item = item;
            item.provenance = vec![format!("agent-session:{}", record.session_id.as_str())];
            item.confidence = 0.5; // handoff 內容是 agent 聲稱的摘要
            let _ = self.memory_create_internal(&item);
        }
        // §14 確定性經驗收集（無 AI；學習訊號才會另建 Reflection Candidate）。
        self.record_task_experience(&record, prior_state);
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
        self.emit_agent_session_state(
            id,
            &record.agent_id,
            if record.state == AgentSessionState::Cancelled {
                "cancelled"
            } else {
                "closed"
            },
        );
        Ok(record)
    }

    /// Emergency stop propagation: every open session is cancelled, its
    /// subprocess tree is terminated, and nothing resumes automatically.
    ///
    /// 緊急停止**不得**變成一次新的模型回合。舊版先送一則 `cancel` 進信箱，
    /// 而 ToSession 的信箱訊息一律會被轉送進 agent 子程序的 stdin（codex 是
    /// `turn/start`、claude 是新的 user message、codex exec 甚至會重新 spawn
    /// 一個程序）——等於「停止」這個動作自己對外開了一輪計費呼叫、發出
    /// `fetched`（角色因此演成「工作中」）、吃掉一格訊息預算，而且得等
    /// stdin 逾時之後才輪得到殺程序；多個卡死的 session 還會把終止排成
    /// 一列。現在改成：只留 runtime 自己寫的稽核註記（不是使用者回合），
    /// 直接關閉 session 並終止程序樹，且每個 session 的收尾有界又彼此併行。
    pub(crate) async fn estop_agent_sessions(&self) {
        let ids: Vec<String> = {
            // 與 create_agent_session 互斥（agent_create_lock）：進行中的
            // 建立完成後才快照，該 session 必然入列；之後的建立則看到
            // estop 旗標（在本函式之前已寫入）而被拒絕——estop 與 create
            // 之間不再有 TOCTOU 縫隙。鎖只罩快照，不罩後面的信箱／關閉 I/O。
            let _create_guard = self.agent_create_lock.lock().await;
            let map = self.agent_sessions.read().await;
            map.values()
                .filter(|e| e.record.state.is_open())
                .map(|e| e.record.session_id.as_str().to_string())
                .collect()
        };
        let stops = ids.iter().map(|id| async move {
            // 收尾有界：紀錄 I/O 卡住不得擋住「停止」本身，也不得讓下一個
            // session 排隊等待。逾時就直接走鎖外的程序樹終止路徑。
            if tokio::time::timeout(
                ESTOP_SESSION_TIMEOUT,
                self.close_agent_session(id, None, "cancelled"),
            )
            .await
            .is_err()
            {
                self.gateway_spawn_kill(id, "emergency-stop");
                tracing::warn!(
                    target: "interaction.agents",
                    agent_session = %id,
                    "緊急停止：session 收尾逾時，仍已終止其子程序樹"
                );
            }
            // 稽核註記要在關閉**之後**才寫：close 會把未送達的信件清掉。
            self.estop_mailbox_note(id).await;
        });
        futures_util::future::join_all(stops).await;
    }

    /// 緊急停止留在信箱裡的稽核註記：**runtime 自己寫**的系統紀錄，不是
    /// 使用者回合——不經 gateway 轉送、不會寫進 agent 子程序的 stdin、不
    /// 佔訊息預算、也不發 `fetched`。放在 `from-session`（人類讀的那一側，
    /// 與 `approval-resolved` 一致），並明說沒有送給 agent。
    async fn estop_mailbox_note(&self, id: &str) {
        {
            let mut map = self.agent_sessions.write().await;
            let Some(entry) = map.get_mut(id) else {
                return;
            };
            if entry.mailbox.len() >= MAILBOX_CAP {
                entry.mailbox.pop_front();
            }
            let message = MailboxMessage {
                message_id: format!("msg-{}-{}", id, entry.next_message),
                session_id: entry.record.session_id.clone(),
                direction: MailboxDirection::FromSession,
                kind: "emergency-stop".to_string(),
                body: BTreeMap::from([
                    ("by".to_string(), json!("runtime")),
                    (
                        "reason".to_string(),
                        json!("緊急停止：這個工作已被取消，並已對它的子程序樹送出終止（先禮貌終止，寬限後強制）；不會自動恢復。"),
                    ),
                    ("deliveredToAgent".to_string(), json!(false)),
                ]),
                created_at: Utc::now(),
                delivered_at: None,
                action_id: None,
            };
            entry.next_message += 1;
            entry.mailbox.push_back(message);
        }
        let _ = self.store.audit(
            "agent-session.emergency-stop",
            "runtime",
            &json!({"agentSessionId": id, "deliveredToAgent": false}),
        );
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
        // 上一輪 daemon 可能沒走完 shutdown（崩潰／SIGKILL／斷電）：標
        // Expired 之前，先依已落地的 pgid 記錄終結還活著的孤兒子程序樹。
        let reaped = self.reap_recorded_gateway_pgids("restore").await;
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
                    record.detail = Some(match reaped.get(record.session_id.as_str()) {
                        Some(pgid) => format!(
                            "runtime restarted; orphan subprocess group reaped (pgid {pgid})"
                        ),
                        None => "runtime restarted".into(),
                    });
                    if let Ok(body) = serde_json::to_string(&record) {
                        let _ = self
                            .store
                            .save_agent_session(record.session_id.as_str(), &body);
                    }
                    // 角色 taxonomy：上一輪 daemon 沒走完，這些工作最後
                    // 到底成了沒有——沒有人知道。誠實發 unknown，不讓
                    // 重啟後的 UI 停在重啟前的假象（例如「工作中」）。
                    self.emit_agent_session_state(
                        record.session_id.as_str(),
                        &record.agent_id,
                        "unknown",
                    );
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

    // ------------------------------------------------------------------
    // Gateway 孤兒子程序記錄（meta:gateway_pgids）。spawn_grouped 讓每個
    // gateway 子程序自成 process group（pgid == 直接子程序 pid），但 handle
    // 不跨 daemon 重啟存活——pgid 記錄是 shutdown／崩潰重啟後唯一找得回
    // 整棵孤兒程序樹的線索（「子程序絕不跨 runtime 重啟存活」）。
    // ------------------------------------------------------------------

    fn load_gateway_pgids(&self) -> BTreeMap<String, GatewayPgidRecord> {
        self.store
            .get_meta(GATEWAY_PGID_META_KEY)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save_gateway_pgids(&self, map: &BTreeMap<String, GatewayPgidRecord>) {
        if let Ok(body) = serde_json::to_string(map) {
            let _ = self.store.set_meta(GATEWAY_PGID_META_KEY, &body);
        }
    }

    /// attach 成功後立刻記下子程序的 pgid（best-effort）。handle 不暴露
    /// pid，這裡以 OS 快照鎖定「本 daemon 的直接子程序、自為 group leader
    /// （spawn_grouped 的簽名）、且未被記錄過」的行程；create 已被
    /// agent_create_lock 序列化，同一時刻至多一個新 gateway 子程序。候選
    /// 不唯一（前一個 session 的殘影還沒死透）或已消失（極早退出）就誠實
    /// 放棄記錄——該 session 若遇上 daemon 崩潰將無法被 restore 找回，
    /// 屬已知殘餘風險。
    pub(crate) async fn record_gateway_pgid(&self, session_id: &str) {
        #[cfg(unix)]
        {
            let mut known = self.load_gateway_pgids();
            let recorded: std::collections::BTreeSet<u32> =
                known.values().map(|r| r.pgid).collect();
            let me = std::process::id();
            let candidates: Vec<ProcRow> = snapshot_processes()
                .await
                .into_iter()
                .filter(|p| p.ppid == me && p.pid == p.pgid && !recorded.contains(&p.pgid))
                .collect();
            match candidates.as_slice() {
                [only] => {
                    known.insert(
                        session_id.to_string(),
                        GatewayPgidRecord {
                            pgid: only.pgid,
                            cmd: only.cmd.clone(),
                        },
                    );
                    self.save_gateway_pgids(&known);
                }
                other => {
                    tracing::warn!(
                        target: "interaction.gateway",
                        session = %session_id,
                        candidates = other.len(),
                        "無法確定 gateway 子程序 pgid；daemon 崩潰時這個 session 的孤兒清理會漏掉它"
                    );
                }
            }
        }
        #[cfg(not(unix))]
        let _ = session_id;
    }

    /// 移除一個 session 的 pgid 記錄（正常關閉路徑；kill 已由 handle 負責）。
    pub(crate) fn forget_gateway_pgid(&self, session_id: &str) {
        let mut known = self.load_gateway_pgids();
        if known.remove(session_id).is_some() {
            self.save_gateway_pgids(&known);
        }
    }

    /// 終結所有已記錄的 gateway 子程序群組（shutdown 與 restore 共用）。
    /// SIGTERM → 有界輪詢（≤1s）→ 無條件補 SIGKILL 給整組：group leader
    /// 先退出時，忽略 SIGTERM 的孫程序仍會被終結。PID 重用防護：只有
    /// 「還活著、仍自為 group leader、command line 與記錄完全相同」的行程
    /// 才送訊號，其餘放過並丟棄記錄——寧可留下孤兒（誠實記為已知限制），
    /// 不可誤殺無關行程；同 pid 同 command 的極端重用仍可能誤殺（殘餘風險）。
    pub(crate) async fn reap_recorded_gateway_pgids(
        &self,
        reason: &'static str,
    ) -> BTreeMap<String, u32> {
        let known = self.load_gateway_pgids();
        let mut reaped: BTreeMap<String, u32> = BTreeMap::new();
        if known.is_empty() {
            return reaped;
        }
        #[cfg(unix)]
        {
            let procs = snapshot_processes().await;
            for (sid, rec) in &known {
                let verified = procs
                    .iter()
                    .any(|p| p.pid == rec.pgid && p.pgid == rec.pgid && p.cmd == rec.cmd);
                if !verified {
                    continue;
                }
                signal_process_group(rec.pgid, "TERM").await;
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let leader_alive = snapshot_processes()
                        .await
                        .iter()
                        .any(|p| p.pid == rec.pgid && p.pgid == rec.pgid);
                    if !leader_alive {
                        break;
                    }
                }
                signal_process_group(rec.pgid, "KILL").await;
                tracing::info!(
                    target: "interaction.gateway",
                    session = %sid,
                    pgid = rec.pgid,
                    reason,
                    "orphan agent subprocess group reaped"
                );
                reaped.insert(sid.clone(), rec.pgid);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = reason;
            // 非 unix 平台沒有 process-group 訊號語意；誠實不假裝清理過。
            tracing::warn!(
                target: "interaction.gateway",
                "gateway 孤兒子程序清理在此平台不支援"
            );
        }
        self.save_gateway_pgids(&BTreeMap::new());
        reaped
    }
}

pub(crate) const GATEWAY_PGID_META_KEY: &str = "gateway_pgids";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct GatewayPgidRecord {
    pgid: u32,
    /// 記錄當下的完整 command line（ps 快照）：重啟後的 kill 驗證依賴
    /// 「command 完全相同」這一條，作為 best-effort 的 PID 重用防護。
    cmd: String,
}

#[cfg(unix)]
#[derive(Debug, Clone)]
struct ProcRow {
    pid: u32,
    pgid: u32,
    ppid: u32,
    cmd: String,
}

/// OS 行程快照（pid/pgid/ppid/command；zombie 除外——殺不得也不該當
/// attribution 候選）。best-effort：`ps` 失敗或輸出解析不了就回空，
/// 寧可少記／少殺，不可猜。
#[cfg(unix)]
async fn snapshot_processes() -> Vec<ProcRow> {
    let out = match tokio::process::Command::new("ps")
        .args(["-ax", "-ww", "-o", "pid=,pgid=,ppid=,stat=,command="])
        .output()
        .await
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let pid: u32 = it.next()?.parse().ok()?;
            let pgid: u32 = it.next()?.parse().ok()?;
            let ppid: u32 = it.next()?.parse().ok()?;
            let stat = it.next()?;
            if stat.starts_with('Z') {
                return None;
            }
            let cmd = it.collect::<Vec<_>>().join(" ");
            Some(ProcRow {
                pid,
                pgid,
                ppid,
                cmd,
            })
        })
        .collect()
}

/// 對整個 process group（負 pid）送訊號。runtime crate 不直接依賴 libc，
/// 走 POSIX sh 內建 kill；pgid 由 u32 格式化，無注入面。
#[cfg(unix)]
async fn signal_process_group(pgid: u32, signal: &str) {
    let _ = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("kill -s {signal} -- -{pgid}"))
        .output()
        .await;
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
            // Advance the envelope on the way through instead of forwarding it
            // verbatim: increment the hop count and append the target session
            // so downstream depth/cycle checks see a real chain, not a static
            // hop_count:0 the caller can reset every hop.
            if let Ok(mut envelope) =
                serde_json::from_value::<interaction_core::DelegationEnvelope>(env.clone())
            {
                envelope.hop_count = envelope.hop_count.saturating_add(1);
                envelope.parent_task_id = Some(envelope.delegation_id.clone());
                if !envelope.visited_sessions.iter().any(|s| s == &session_id) {
                    envelope.visited_sessions.push(session_id.clone());
                }
                body.insert(
                    "delegation".to_string(),
                    serde_json::to_value(envelope).unwrap_or_else(|_| env.clone()),
                );
            } else {
                body.insert("delegation".to_string(), env.clone());
            }
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
