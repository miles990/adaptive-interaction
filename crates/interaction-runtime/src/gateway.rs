//! Agent Gateway 與 runtime 的接線：真實子程序 agent（codex／claude-code）
//! 掛上 v0.3 的 agent session 模型。
//!
//! 誠實原則：
//! - Gateway 事件走既有 report_agent_session 路徑——agent 的 claim 永遠是
//!   claim（觀察 inference、actionId→claimActionId 防偽照舊）。
//! - 子程序絕不跨 runtime 重啟存活（restore 已把 open session 標 Expired）。
//! - estop／close／lease 到期 → 終止整棵子程序樹。
//! - approval 請求預設不核可：進 waiting-for-consent，由人類明確裁決；
//!   逾時自動拒絕（deny），絕不替人類同意。

use crate::runtime::Runtime;
use chrono::Utc;
use interaction_agent_gateway::{
    AgentConnector, AgentDiscovery, AgentKind, AgentSessionHandle, ApprovalDecision, GatewayEvent,
};
use interaction_core::{
    AgentSessionRecord, DomainError, DomainResult, MailboxDirection, MailboxMessage,
    ProviderDescriptor, ProviderId, ProviderIdentity, ProviderKind, ProviderState, Timestamp,
    TrustLevel,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// approval 無人裁決的自動拒絕時限。
pub const APPROVAL_TTL_SECS: i64 = 300;

pub fn agent_kind_for(agent_id: &str) -> Option<AgentKind> {
    match agent_id {
        "codex" => Some(AgentKind::Codex),
        "claude-code" => Some(AgentKind::ClaudeCode),
        _ => None,
    }
}

/// binary 可用 env 覆寫（測試 fixture 與非 PATH 安裝）：
/// INTERACT_AI_CLAUDE_BIN / INTERACT_AI_CODEX_BIN。
fn connectors() -> Vec<Box<dyn AgentConnector>> {
    let claude = std::env::var("INTERACT_AI_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
    let codex = std::env::var("INTERACT_AI_CODEX_BIN").unwrap_or_else(|_| "codex".into());
    vec![
        Box::new(interaction_agent_gateway::claude::ClaudeConnector::with_binary(claude)),
        Box::new(interaction_agent_gateway::codex::CodexConnector::with_binary(codex)),
    ]
}

struct PendingApproval {
    #[allow(dead_code)] // 進階詳情面板將顯示；先保留在資料模型
    summary: String,
    deadline: Timestamp,
}

struct ManagedSession {
    handle: Arc<tokio::sync::Mutex<Box<dyn AgentSessionHandle>>>,
    approvals: Arc<Mutex<HashMap<String, PendingApproval>>>,
}

#[derive(Default)]
pub struct GatewayManager {
    sessions: Mutex<HashMap<String, ManagedSession>>,
    /// 最近一次發現結果（背景更新；UI 讀這份）。
    discoveries: Mutex<Vec<AgentDiscovery>>,
}

impl GatewayManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn managed(&self, session_id: &str) -> Option<ManagedSessionRef> {
        let map = self.sessions.lock().expect("gateway sessions lock");
        map.get(session_id).map(|m| ManagedSessionRef {
            handle: m.handle.clone(),
            approvals: m.approvals.clone(),
        })
    }

    pub fn is_managed(&self, session_id: &str) -> bool {
        self.sessions
            .lock()
            .expect("gateway sessions lock")
            .contains_key(session_id)
    }

    pub fn managed_ids(&self) -> Vec<String> {
        self.sessions
            .lock()
            .expect("gateway sessions lock")
            .keys()
            .cloned()
            .collect()
    }

    fn remove(&self, session_id: &str) -> Option<ManagedSession> {
        self.sessions
            .lock()
            .expect("gateway sessions lock")
            .remove(session_id)
    }

    pub fn discoveries(&self) -> Vec<AgentDiscovery> {
        self.discoveries.lock().expect("discoveries lock").clone()
    }
}

struct ManagedSessionRef {
    handle: Arc<tokio::sync::Mutex<Box<dyn AgentSessionHandle>>>,
    approvals: Arc<Mutex<HashMap<String, PendingApproval>>>,
}

impl Runtime {
    /// 背景發現本機 agent 並註冊為 provider（不阻塞啟動、不啟動登入流程）。
    pub(crate) fn spawn_agent_discovery(&self) {
        let rt = self.clone();
        tokio::spawn(async move {
            rt.refresh_agent_providers().await;
        });
    }

    /// 重新發現（API 亦可觸發）。誠實：登入未知＝unknown，不裝可用。
    pub async fn refresh_agent_providers(&self) -> Vec<AgentDiscovery> {
        let mut results = Vec::new();
        for connector in connectors() {
            let discovery = connector.discover().await;
            let state = if discovery.usable() {
                ProviderState::Available
            } else if discovery.found {
                ProviderState::Degraded
            } else {
                ProviderState::Disconnected
            };
            let descriptor = ProviderDescriptor {
                identity: ProviderIdentity {
                    id: ProviderId::new(discovery.kind.provider_id()),
                    kind: ProviderKind::AiAgent,
                    display_name: match discovery.kind {
                        AgentKind::Codex => "Codex（本機 CLI）".into(),
                        AgentKind::ClaudeCode => "Claude Code（本機 CLI）".into(),
                    },
                    trust_level: TrustLevel::Discovered,
                    origin: "agent-gateway".into(),
                    version: discovery.version.clone().unwrap_or_default(),
                    fingerprint: None,
                    human: None,
                },
                state,
                receptors: vec![],
                actuators: vec![],
                tool_operations: vec![],
                paired_at: None,
                last_seen: Some(Utc::now()),
                detail: Some(discovery.detail.clone()),
            };
            let _ = self.providers.register(descriptor).await;
            results.push(discovery);
        }
        *self.gateway.discoveries.lock().expect("discoveries lock") = results.clone();
        results
    }

    /// 在 create_agent_session 成功後把 gateway agent 掛上子程序。
    /// 失敗＝建立失敗（誠實），不留半掛的 session。
    pub(crate) async fn gateway_attach(
        &self,
        kind: AgentKind,
        record: &AgentSessionRecord,
        workdir: Option<String>,
    ) -> DomainResult<Option<String>> {
        let connector: Box<dyn AgentConnector> = connectors()
            .into_iter()
            .find(|c| c.kind() == kind)
            .expect("connector exists for kind");
        let discovery = connector.discover().await;
        if !discovery.usable() {
            return Err(DomainError::Unavailable(format!(
                "{} 不可用：{}",
                kind.agent_id(),
                discovery.detail
            )));
        }
        let workdir = workdir
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.paths.home.clone());
        if !workdir.is_dir() {
            return Err(DomainError::Validation(format!(
                "workdir 不存在：{}",
                workdir.display()
            )));
        }
        let spec = interaction_agent_gateway::SessionSpec::read_only_in(workdir);
        let mut handle = connector
            .start_session(spec)
            .await
            .map_err(|e| DomainError::Unavailable(format!("啟動 {} 失敗：{e}", kind.agent_id())))?;
        let events = handle
            .take_events()
            .ok_or_else(|| DomainError::Internal("events already taken".into()))?;
        let provider_session_id = handle.provider_session_id();
        let approvals: Arc<Mutex<HashMap<String, PendingApproval>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let managed = ManagedSession {
            handle: Arc::new(tokio::sync::Mutex::new(handle)),
            approvals: approvals.clone(),
        };
        self.gateway
            .sessions
            .lock()
            .expect("gateway sessions lock")
            .insert(record.session_id.as_str().to_string(), managed);
        self.spawn_gateway_pump(record.session_id.as_str().to_string(), events, approvals);
        Ok(provider_session_id)
    }

    /// 事件泵：正規化事件 → 既有誠實回報路徑。
    fn spawn_gateway_pump(
        &self,
        session_id: String,
        mut events: tokio::sync::mpsc::Receiver<GatewayEvent>,
        approvals: Arc<Mutex<HashMap<String, PendingApproval>>>,
    ) {
        let rt = self.clone();
        tokio::spawn(async move {
            let mut claimed_or_failed = false;
            while let Some(ev) = events.recv().await {
                match ev {
                    GatewayEvent::SessionStarted {
                        provider_session_id,
                    } => {
                        rt.set_provider_session_id(&session_id, &provider_session_id)
                            .await;
                    }
                    GatewayEvent::TaskAccepted => {
                        let _ = rt
                            .report_agent_session(&session_id, "task-started", json!({}))
                            .await;
                    }
                    GatewayEvent::TaskProgress { text } => {
                        let _ = rt
                            .report_agent_session(&session_id, "progress", json!({"text": text}))
                            .await;
                    }
                    GatewayEvent::TaskWaitingForInput => {
                        let _ = rt
                            .report_agent_session(&session_id, "waiting-for-input", json!({}))
                            .await;
                    }
                    GatewayEvent::TaskWaitingForConsent {
                        request_id,
                        summary,
                    } => {
                        approvals.lock().expect("approvals lock").insert(
                            request_id.clone(),
                            PendingApproval {
                                summary: summary.clone(),
                                deadline: Utc::now() + chrono::Duration::seconds(APPROVAL_TTL_SECS),
                            },
                        );
                        let _ = rt
                            .report_agent_session(
                                &session_id,
                                "waiting-for-consent",
                                json!({"requestId": request_id, "summary": summary}),
                            )
                            .await;
                        let mut body = BTreeMap::new();
                        body.insert("requestId".to_string(), json!(request_id));
                        body.insert("summary".to_string(), json!(summary));
                        let _ = rt
                            .mailbox_send(
                                &session_id,
                                MailboxDirection::FromSession,
                                "approval-request",
                                body,
                                None,
                            )
                            .await;
                    }
                    GatewayEvent::ToolStarted { name } => {
                        let _ = rt
                            .report_agent_session(
                                &session_id,
                                "progress",
                                json!({"tool": {"name": name, "phase": "started"}}),
                            )
                            .await;
                    }
                    GatewayEvent::ToolCompleted { name } => {
                        let _ = rt
                            .report_agent_session(
                                &session_id,
                                "progress",
                                json!({"tool": {"name": name, "phase": "completed"}}),
                            )
                            .await;
                    }
                    GatewayEvent::ArtifactProduced { path } => {
                        let _ = rt
                            .report_agent_session(
                                &session_id,
                                "progress",
                                json!({"artifact": path}),
                            )
                            .await;
                    }
                    GatewayEvent::TaskClaimedCompleted {
                        summary,
                        cost_usd,
                        num_turns,
                    } => {
                        claimed_or_failed = true;
                        if let Some(cost) = cost_usd {
                            rt.add_agent_session_cost(&session_id, cost).await;
                        }
                        let mut body = BTreeMap::new();
                        body.insert(
                            "summary".to_string(),
                            json!(summary.clone().unwrap_or_default()),
                        );
                        if let Some(c) = cost_usd {
                            body.insert("costUsd".to_string(), json!(c));
                        }
                        let _ = rt
                            .mailbox_send(
                                &session_id,
                                MailboxDirection::FromSession,
                                "result",
                                body,
                                None,
                            )
                            .await;
                        let _ = rt
                            .report_agent_session(
                                &session_id,
                                "claimed-completed",
                                json!({
                                    "summary": summary,
                                    "costUsd": cost_usd,
                                    "numTurns": num_turns,
                                }),
                            )
                            .await;
                    }
                    GatewayEvent::TaskFailed { error } => {
                        claimed_or_failed = true;
                        let _ = rt
                            .report_agent_session(&session_id, "failed", json!({"error": error}))
                            .await;
                    }
                    GatewayEvent::TaskCancelled => {
                        claimed_or_failed = true;
                        let _ = rt
                            .report_agent_session(&session_id, "cancelled", json!({}))
                            .await;
                    }
                    GatewayEvent::SessionClosed { resumable, detail } => {
                        // 程序結束：若沒有任何結果聲稱且 session 還開著，
                        // 誠實回報 failed（絕不猜測成功）。
                        if !claimed_or_failed {
                            let _ = rt
                                .report_agent_session(
                                    &session_id,
                                    "failed",
                                    json!({
                                        "error": "agent 程序已結束而未回報結果",
                                        "resumable": resumable,
                                        "detail": detail,
                                    }),
                                )
                                .await;
                        }
                        break;
                    }
                    GatewayEvent::Unparsed { raw } => {
                        tracing::debug!(target: "interaction.gateway", session = %session_id, raw = %raw, "unparsed agent line");
                    }
                }
            }
            rt.gateway.remove(&session_id);
        });
    }

    async fn set_provider_session_id(&self, session_id: &str, provider_sid: &str) {
        let mut map = self.agent_sessions.write().await;
        if let Some(entry) = map.get_mut(session_id) {
            entry.record.provider_session_id = Some(provider_sid.to_string());
            self.persist_agent_session(&entry.record);
        }
    }

    async fn add_agent_session_cost(&self, session_id: &str, cost: f64) {
        if cost <= 0.0 {
            return;
        }
        let mut map = self.agent_sessions.write().await;
        if let Some(entry) = map.get_mut(session_id) {
            entry.record.budget.spent_cost += cost;
            self.persist_agent_session(&entry.record);
        }
    }

    /// mailbox ToSession 訊息送達真實 agent 子程序（送達＝delivered）。
    /// 回傳 true 表示已轉送；非 gateway session 回傳 false（v0.3 輪詢流程）。
    pub(crate) async fn gateway_deliver(&self, session_id: &str, message: &MailboxMessage) -> bool {
        let Some(managed) = self.gateway.managed(session_id) else {
            return false;
        };
        // 預算：超出成本上限就不再開新 turn（誠實拒絕）。
        let over_budget = {
            let map = self.agent_sessions.read().await;
            map.get(session_id)
                .map(|e| {
                    e.record.budget.max_cost > 0.0
                        && e.record.budget.spent_cost >= e.record.budget.max_cost
                })
                .unwrap_or(false)
        };
        if over_budget {
            let _ = self
                .report_agent_session(
                    session_id,
                    "failed",
                    json!({"error": "session 成本預算已用盡，不再開新 turn"}),
                )
                .await;
            return false;
        }
        let text = message
            .body
            .get("task")
            .or_else(|| message.body.get("text"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| serde_json::to_string(&message.body).unwrap_or_default());
        let ok = {
            let mut handle = managed.handle.lock().await;
            handle.send_user_message(&text).await.is_ok()
        };
        if ok {
            // 轉送即送達：補 delivered 戳記＋委派 receipt ack。
            let acked = {
                let mut map = self.agent_sessions.write().await;
                map.get_mut(session_id).and_then(|entry| {
                    entry
                        .mailbox
                        .iter_mut()
                        .find(|m| m.message_id == message.message_id)
                        .map(|m| {
                            m.delivered_at = Some(Utc::now());
                            m.action_id.clone()
                        })
                })
            };
            if let Some(Some(action_id)) = acked {
                let _ = self
                    .acknowledge_delegated_action_public(&action_id, &message.message_id)
                    .await;
            }
        } else {
            let _ = self
                .report_agent_session(
                    session_id,
                    "failed",
                    json!({"error": "無法把訊息送進 agent 子程序（可能已結束）"}),
                )
                .await;
        }
        ok
    }

    /// 人類裁決 agent 的 approval 請求。
    pub async fn gateway_resolve_approval(
        &self,
        session_id: &str,
        request_id: &str,
        approve: bool,
    ) -> DomainResult<Value> {
        let managed = self
            .gateway
            .managed(session_id)
            .ok_or_else(|| DomainError::NotFound(format!("gateway session {session_id}")))?;
        let known = managed
            .approvals
            .lock()
            .expect("approvals lock")
            .remove(request_id)
            .is_some();
        if !known {
            return Err(DomainError::NotFound(format!(
                "approval request {request_id}"
            )));
        }
        {
            let mut handle = managed.handle.lock().await;
            handle
                .resolve_approval(
                    request_id,
                    if approve {
                        ApprovalDecision::Approve
                    } else {
                        ApprovalDecision::Deny
                    },
                )
                .await
                .map_err(|e| DomainError::Unavailable(e.to_string()))?;
        }
        let _ = self
            .report_agent_session(
                session_id,
                "task-started",
                json!({"approvalResolved": request_id, "approved": approve}),
            )
            .await;
        self.store.audit(
            "agent.approval",
            "human",
            &json!({"sessionId": session_id, "requestId": request_id, "approved": approve}),
        )?;
        Ok(json!({"resolved": request_id, "approved": approve}))
    }

    /// 中斷目前 turn（不關 session）。
    pub async fn gateway_interrupt(&self, session_id: &str) -> DomainResult<Value> {
        let managed = self
            .gateway
            .managed(session_id)
            .ok_or_else(|| DomainError::NotFound(format!("gateway session {session_id}")))?;
        let mut handle = managed.handle.lock().await;
        handle
            .interrupt()
            .await
            .map_err(|e| DomainError::Unavailable(e.to_string()))?;
        Ok(json!({"interrupted": true}))
    }

    /// 終止子程序樹（estop／close／到期）。同步呼叫安全（內部 spawn）。
    pub(crate) fn gateway_spawn_kill(&self, session_id: &str, reason: &'static str) {
        let Some(managed) = self.gateway.remove(session_id) else {
            return;
        };
        let sid = session_id.to_string();
        tokio::spawn(async move {
            let mut handle = managed.handle.lock().await;
            let _ = handle.kill().await;
            tracing::info!(target: "interaction.gateway", session = %sid, reason, "agent subprocess killed");
        });
    }

    /// watchdog：逾時 approval 自動拒絕；已關閉 record 的殘留子程序清理。
    pub(crate) async fn gateway_sweep(&self) {
        let now = Utc::now();
        // 逾時 approval → 自動 deny（絕不自動同意）。
        let mut to_deny: Vec<(String, String)> = Vec::new();
        {
            let sessions = self.gateway.sessions.lock().expect("gateway sessions lock");
            for (sid, m) in sessions.iter() {
                let approvals = m.approvals.lock().expect("approvals lock");
                for (rid, p) in approvals.iter() {
                    if p.deadline <= now {
                        to_deny.push((sid.clone(), rid.clone()));
                    }
                }
            }
        }
        for (sid, rid) in to_deny {
            let _ = self.gateway_resolve_approval(&sid, &rid, false).await;
            let _ = self
                .report_agent_session(
                    &sid,
                    "progress",
                    json!({"approvalAutoDenied": rid, "reason": "逾時無人裁決，預設拒絕"}),
                )
                .await;
        }
        // record 已非 open（close／expire／estop 走過）但子程序還掛著 → 殺。
        let managed_ids = self.gateway.managed_ids();
        if !managed_ids.is_empty() {
            let map = self.agent_sessions.read().await;
            for sid in managed_ids {
                let open = map
                    .get(&sid)
                    .map(|e| e.record.state.is_open() && !e.record.lease.is_expired(now))
                    .unwrap_or(false);
                if !open {
                    drop_kill(self, &sid);
                }
            }
        }
    }
}

fn drop_kill(rt: &Runtime, sid: &str) {
    rt.gateway_spawn_kill(sid, "record-closed");
}

impl Runtime {
    /// 最近一次 agent 發現快照（背景更新）。
    pub fn agent_discoveries(&self) -> Vec<AgentDiscovery> {
        self.gateway.discoveries()
    }

    /// 確定性路由建議（spec §8.4）：建議，不強制；模糊任務列出兩者讓人選。
    /// 不用生成式 AI 做權限或路由決策。
    pub fn agent_route_suggestion(&self, kind: Option<&str>) -> Value {
        let discoveries = self.gateway.discoveries();
        let usable = |id: &str| {
            discoveries
                .iter()
                .find(|d| d.kind.agent_id() == id)
                .map(|d| d.usable())
                .unwrap_or(false)
        };
        let (primary, reason) = match kind.unwrap_or("") {
            "code" | "test" | "patch" | "repo-review" => (
                "codex",
                "程式實作／測試／Patch／Repository 審查：Codex 優先",
            ),
            "docs" | "concepts" | "content" | "planning" | "analysis" => (
                "claude-code",
                "長文件／概念歸納／內容／規劃／跨領域分析：Claude Code 優先",
            ),
            "review-second-opinion" => (
                "claude-code",
                "重要程式變更的第二雙眼睛：由另一個 agent 只讀複審",
            ),
            _ => ("", "模糊或跨領域任務：顯示兩個選項讓使用者選，不自動代選"),
        };
        json!({
            "kind": kind,
            "suggestion": if primary.is_empty() { Value::Null } else { json!(primary) },
            "reason": reason,
            "candidates": [
                {"agentId": "codex", "usable": usable("codex")},
                {"agentId": "claude-code", "usable": usable("claude-code")},
            ],
            "note": "建議僅供參考；建立 session 前會顯示資料範圍與成本預覽，且不會自動改送另一個 provider。",
        })
    }
}
