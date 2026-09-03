//! AI Provider → Agent Profile → Agent Session model.
//!
//! A SESSION is not an identity: it is a short-lived, leased, budgeted grant.
//! Everything a session can see or do is scoped (data/tool/consent scopes),
//! delegations carry an anti-loop envelope, and when the session ends its
//! consents, credentials and capabilities die with it. An agent's report is a
//! CLAIM (claimed-completed), never a receipt and never verification.

use crate::{AgentSessionId, ProviderId, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AgentSessionState {
    #[default]
    Created,
    Active,
    /// The agent is blocked on a human answer.
    ///
    /// HONEST GAP (v0.5): **no connector currently produces this state.**
    /// Claude Code `-p --output-format stream-json` has no "waiting for input"
    /// message (a finished turn is a `result`, i.e. a claim), and the pinned
    /// Codex app-server 0.149.1 schema has no equivalent notification either —
    /// its only blocking signal is an approval ServerRequest, which maps to
    /// `WaitingForConsent`. The state stays in the model because the runtime
    /// API/CLI can report it, but nothing infers it from provider traffic.
    WaitingForInput,
    WaitingForConsent,
    /// The agent SAYS it finished. Not verified, not a receipt.
    ClaimedCompleted,
    Failed,
    TimedOut,
    Cancelled,
    Expired,
    Closed,
    /// The work ended without any claim AND without an observed error: the
    /// outcome is genuinely **unknown**. It is not success and it is not a
    /// failure — claiming either would be a lie. A subprocess that exits
    /// without reporting a result lands here (only an observable error, e.g. a
    /// non-zero exit code, is honest enough to be `Failed`).
    Unknown,
}

impl AgentSessionState {
    pub fn is_open(self) -> bool {
        matches!(
            self,
            AgentSessionState::Created
                | AgentSessionState::Active
                | AgentSessionState::WaitingForInput
                | AgentSessionState::WaitingForConsent
                | AgentSessionState::ClaimedCompleted
        )
    }
}

/// Lease on the capabilities a session brings/uses. Expiry is checked lazily
/// on every access; renewal must be explicit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityLease {
    pub issued_at: Timestamp,
    pub expires_at: Timestamp,
    #[serde(default)]
    pub renewable: bool,
    #[serde(default = "default_true")]
    pub revoke_on_session_end: bool,
}

fn default_true() -> bool {
    true
}

impl CapabilityLease {
    pub fn is_expired(&self, now: Timestamp) -> bool {
        now >= self.expires_at
    }
}

/// Budgets: hard ceilings, decremented as the session works.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionBudget {
    /// Wall-clock ceiling for the whole session (0 = use lease expiry only).
    pub max_duration_ms: u64,
    /// Monetary ceiling in user-currency units (0 = no monetary budget).
    pub max_cost: f64,
    pub spent_cost: f64,
    /// Mailbox message ceiling (both directions), anti-runaway.
    pub max_messages: u32,
    pub spent_messages: u32,
}

impl SessionBudget {
    pub fn messages_left(&self) -> bool {
        self.max_messages == 0 || self.spent_messages < self.max_messages
    }
}

/// Anti-loop delegation envelope carried by every delegated task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DelegationEnvelope {
    pub root_task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<String>,
    pub delegation_id: String,
    #[serde(default)]
    pub origin_agent_id: String,
    pub hop_count: u32,
    pub max_hops: u32,
    #[serde(default)]
    pub visited_sessions: Vec<String>,
    /// Fraction of the root budget still available (1.0 at the root).
    pub budget_remaining: f64,
}

/// Deterministic delegation limits (policy-owned).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct DelegationLimits {
    pub max_depth: u32,
    pub max_sessions: u32,
    pub max_messages_per_session: u32,
    pub max_parallel: u32,
    /// Provider allowlist for delegation targets; empty = none allowed.
    pub provider_allowlist: Vec<String>,
}

impl Default for DelegationLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_sessions: 8,
            max_messages_per_session: 200,
            max_parallel: 4,
            provider_allowlist: Vec::new(),
        }
    }
}

/// Deterministic delegation check. Rejections are hard, with reasons.
pub fn check_delegation(
    envelope: &DelegationEnvelope,
    limits: &DelegationLimits,
    target_session: &str,
    open_sessions: u32,
) -> Result<(), String> {
    if envelope.hop_count >= envelope.max_hops.min(limits.max_depth) {
        return Err(format!(
            "delegation depth exhausted (hop {} of max {})",
            envelope.hop_count,
            envelope.max_hops.min(limits.max_depth)
        ));
    }
    if envelope
        .visited_sessions
        .iter()
        .any(|s| s == target_session)
    {
        return Err(format!(
            "delegation cycle: session {target_session} already in the chain"
        ));
    }
    if envelope.budget_remaining <= 0.0 {
        return Err("delegation budget exhausted".into());
    }
    if open_sessions >= limits.max_sessions {
        return Err(format!(
            "too many open agent sessions ({open_sessions} ≥ {})",
            limits.max_sessions
        ));
    }
    Ok(())
}

/// The persisted record of one agent session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionRecord {
    pub session_id: AgentSessionId,
    pub provider_id: ProviderId,
    pub agent_id: String,
    #[serde(default)]
    pub label: Option<String>,
    pub state: AgentSessionState,
    pub lease: CapabilityLease,
    /// What data the session may receive (human-meaningful categories).
    #[serde(default)]
    pub data_scope: Vec<String>,
    /// Tool operations the session may call / provide.
    #[serde(default)]
    pub tool_scope: Vec<String>,
    /// Consent scopes granted TO this session (die with it).
    #[serde(default)]
    pub consent_scope: Vec<String>,
    /// Whether this leased session may modify files inside its single workspace.
    /// Defaults false for backward compatibility; it cannot be upgraded after creation.
    #[serde(default)]
    pub allow_write: bool,
    pub budget: SessionBudget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<DelegationEnvelope>,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Minimal handoff stored at close (bounded); NEVER a full transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<HandoffSummary>,
    /// Provider 端 session/thread id（codex thread、claude session）。
    /// 供進階詳情與續開（resume）；不是 runtime 的 session 身分。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    /// 這個 session **實際**掛上子程序的工作目錄（正規化後的絕對路徑）。
    /// 只有 gateway agents（codex／claude-code）有值；純對話 session 永遠
    /// 是 None，升級前建立的舊記錄也是 None（新增欄位、舊 JSON 相容）。
    ///
    /// 安全用途：`dataScope` 裡的 `workspace:` 只是呼叫端自己附加的人話
    /// 標籤，不代表子程序真的被掛在哪裡。續開時只認這個欄位——沒有它就
    /// 無法證明「沒有換資料夾」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_workdir: Option<String>,
    /// 最新一次 claimed-completed 的識別（每一個新的聲稱都拿到新的 id）。
    /// 人工驗證只綁定這一個 claim：session 可多輪，第二輪的聲稱不得繼承
    /// 第一輪的綠勾。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_id: Option<String>,
    /// 人工驗證（human-only）：claim 經人類確認後才存在。沒有這個欄位，
    /// claimed-completed 永遠不是 verified——角色的綠色勾勾只認它。
    /// 任何更新的 agent 自我回報（新一輪工作、新的聲稱、失敗／未知）或
    /// 新任務送達都會清掉它：存在 ⇒ 驗證的就是 `claim_id` 這一個 claim。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_verified: Option<HumanVerification>,
    /// Exact deterministic Context Bundles actually attached to dispatched
    /// task messages. This is bounded evidence, not a transcript; it lets the
    /// human verify what memory/knowledge crossed the session boundary.
    #[serde(default)]
    pub context_bundles: Vec<AgentContextBundleReceipt>,
}

/// 人類對 agent claim 的獨立確認（記錄時間與可選備註）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HumanVerification {
    pub at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 被驗證的那個 claim（＝驗證當下 record 的 `claim_id`）。介面只在它
    /// 等於 record 目前的 `claim_id` 時顯示綠勾。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextBundleReceipt {
    pub bundle_id: String,
    pub message_id: String,
    pub generated_at: Timestamp,
    /// SHA-256 of the canonical serialized `bundle` below.
    pub content_hash: String,
    pub bundle: serde_json::Value,
}

/// What survives a session: a bounded, structured summary. No chat logs, no
/// system prompts, no hidden reasoning, no secrets.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct HandoffSummary {
    pub task: String,
    pub confirmed_facts: Vec<String>,
    pub inferences: Vec<HandoffInference>,
    pub decisions: Vec<String>,
    pub artifacts: Vec<HandoffArtifact>,
    pub permissions: Vec<String>,
    pub remaining_work: Vec<String>,
    pub risks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HandoffInference {
    pub text: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HandoffArtifact {
    pub uri: String,
    #[serde(default)]
    pub hash: Option<String>,
}

/// Bound the handoff: refuse transcript-sized payloads.
pub fn validate_handoff(h: &HandoffSummary) -> Result<(), String> {
    const MAX_ITEMS: usize = 50;
    const MAX_TEXT: usize = 2_000;
    if h.task.len() > MAX_TEXT {
        return Err("handoff task text too long (max 2000 chars)".into());
    }
    let lists = [
        ("confirmedFacts", h.confirmed_facts.len()),
        ("decisions", h.decisions.len()),
        ("artifacts", h.artifacts.len()),
        ("permissions", h.permissions.len()),
        ("remainingWork", h.remaining_work.len()),
        ("risks", h.risks.len()),
        ("inferences", h.inferences.len()),
    ];
    for (name, len) in lists {
        if len > MAX_ITEMS {
            return Err(format!(
                "handoff {name} has {len} items (max {MAX_ITEMS}) — a handoff is a summary, \
                 not a transcript"
            ));
        }
    }
    for f in h
        .confirmed_facts
        .iter()
        .chain(h.decisions.iter())
        .chain(h.remaining_work.iter())
        .chain(h.risks.iter())
        .chain(h.permissions.iter())
    {
        if f.len() > MAX_TEXT {
            return Err("handoff item too long (max 2000 chars each)".into());
        }
    }
    for i in &h.inferences {
        if !(0.0..=1.0).contains(&i.confidence) {
            return Err("handoff inference confidence must be 0..1".into());
        }
    }
    Ok(())
}

/// One mailbox message. Sessions communicate ONLY through the runtime
/// mailbox — they never read each other's transcripts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailboxMessage {
    pub message_id: String,
    pub session_id: AgentSessionId,
    /// `to-session` (tasks/questions for the agent) or `from-session`.
    pub direction: MailboxDirection,
    pub kind: String,
    /// Bounded structured body.
    pub body: BTreeMap<String, serde_json::Value>,
    pub created_at: Timestamp,
    /// Set when the session actually FETCHED the message (delivery proof).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MailboxDirection {
    ToSession,
    FromSession,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(hops: u32, max: u32, visited: &[&str]) -> DelegationEnvelope {
        DelegationEnvelope {
            root_task_id: "root".into(),
            parent_task_id: None,
            delegation_id: "d1".into(),
            origin_agent_id: "agent.a".into(),
            hop_count: hops,
            max_hops: max,
            visited_sessions: visited.iter().map(|s| s.to_string()).collect(),
            budget_remaining: 1.0,
        }
    }

    #[test]
    fn delegation_depth_cycle_and_budget_are_enforced() {
        let limits = DelegationLimits::default();
        assert!(check_delegation(&envelope(0, 3, &[]), &limits, "s2", 0).is_ok());
        // Depth: policy max_depth caps even a generous envelope max_hops.
        assert!(check_delegation(&envelope(3, 10, &[]), &limits, "s2", 0)
            .unwrap_err()
            .contains("depth"));
        // Cycle: a visited session may not be delegated to again.
        assert!(check_delegation(&envelope(1, 3, &["s2"]), &limits, "s2", 0)
            .unwrap_err()
            .contains("cycle"));
        // Budget exhaustion is terminal.
        let mut e = envelope(1, 3, &[]);
        e.budget_remaining = 0.0;
        assert!(check_delegation(&e, &limits, "s2", 0)
            .unwrap_err()
            .contains("budget"));
        // Session-count ceiling.
        assert!(check_delegation(&envelope(0, 3, &[]), &limits, "s2", 8)
            .unwrap_err()
            .contains("open agent sessions"));
    }

    #[test]
    fn handoff_refuses_transcript_sized_payloads() {
        let ok = HandoffSummary {
            task: "整理 API 文件".into(),
            confirmed_facts: vec!["routes.rs 有 60 個 endpoint".into()],
            inferences: vec![HandoffInference {
                text: "文件可能已過期".into(),
                confidence: 0.6,
            }],
            ..Default::default()
        };
        assert!(validate_handoff(&ok).is_ok());
        let too_many = HandoffSummary {
            confirmed_facts: (0..51).map(|i| format!("fact {i}")).collect(),
            ..Default::default()
        };
        assert!(validate_handoff(&too_many)
            .unwrap_err()
            .contains("not a transcript"));
        let bad_conf = HandoffSummary {
            inferences: vec![HandoffInference {
                text: "x".into(),
                confidence: 1.5,
            }],
            ..Default::default()
        };
        assert!(validate_handoff(&bad_conf).is_err());
    }

    #[test]
    fn claimed_completed_is_open_not_terminal() {
        // The agent's claim keeps the session OPEN — verification and closure
        // are separate, human/runtime-owned steps.
        assert!(AgentSessionState::ClaimedCompleted.is_open());
        assert!(!AgentSessionState::Closed.is_open());
        assert!(!AgentSessionState::Expired.is_open());
    }

    /// regression（誠實階梯）：程序結束而沒有結果聲稱曾被記成 `failed`。
    /// `resolvedWorkdir` 是**純加法**的新欄位：升級前存下來的舊 JSON 沒有
    /// 這個鍵，反序列化必須落到 None 而不是報錯；None 也不得寫進 JSON
    /// （`skip_serializing_if`），舊版讀回去才不會看到多餘的鍵。
    #[test]
    fn resolved_workdir_is_additive_and_round_trips_old_records() {
        let old_json = serde_json::json!({
            "sessionId": "asession-1",
            "providerId": "provider.ai-agent.claude-code",
            "agentId": "claude-code",
            "state": "created",
            "lease": {
                "issuedAt": "2026-01-01T00:00:00Z",
                "expiresAt": "2026-01-01T00:30:00Z",
                "renewable": true,
                "revokeOnSessionEnd": true
            },
            "budget": {
                "maxDurationMs": 1_800_000,
                "maxCost": 0.5,
                "spentCost": 0.0,
                "maxMessages": 10,
                "spentMessages": 0
            },
            "createdAt": "2026-01-01T00:00:00Z",
            "providerSessionId": "thread-abc"
        });
        let record: AgentSessionRecord =
            serde_json::from_value(old_json).expect("舊記錄仍要讀得回來");
        assert_eq!(record.resolved_workdir, None, "舊記錄沒有這個欄位＝未知");

        // None 不寫進 JSON。
        let encoded = serde_json::to_value(&record).unwrap();
        assert!(
            encoded.get("resolvedWorkdir").is_none(),
            "None 不得序列化成 null：{encoded}"
        );

        // 有值時原樣往返。
        let mut with_dir = record.clone();
        with_dir.resolved_workdir = Some("/private/tmp/workspace".into());
        let encoded = serde_json::to_value(&with_dir).unwrap();
        assert_eq!(
            encoded["resolvedWorkdir"],
            serde_json::json!("/private/tmp/workspace")
        );
        let decoded: AgentSessionRecord = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, with_dir);
    }

    /// 未知就是未知——terminal、非 open、序列化為 kebab-case `unknown`，
    /// 而且**不等於**任何一種成功或失敗狀態。
    #[test]
    fn unknown_is_a_terminal_state_of_its_own_and_never_a_failure() {
        assert!(!AgentSessionState::Unknown.is_open());
        assert_ne!(AgentSessionState::Unknown, AgentSessionState::Failed);
        assert_ne!(
            AgentSessionState::Unknown,
            AgentSessionState::ClaimedCompleted
        );
        assert_eq!(
            serde_json::to_value(AgentSessionState::Unknown).unwrap(),
            serde_json::json!("unknown")
        );
        assert_eq!(
            serde_json::from_value::<AgentSessionState>(serde_json::json!("unknown")).unwrap(),
            AgentSessionState::Unknown
        );
    }
}
