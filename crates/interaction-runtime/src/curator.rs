//! 知識更新決策器＋經驗轉知識＋Knowledge Receipt（spec §13／§14／§17）。
//!
//! 兩個獨立決策（純函式、確定性）：「是否需要更新知識？」「是否需要呼叫 AI？」
//! 發布政策三級：可自動發布（metadata 類）／只能建 Candidate（AI 內容）／
//! 必須人類確認（敏感、人格、醫法財安、擴大範圍）。
//! 每次知識變化產生機器可讀 knowledgeReceipt（誠實 verification 欄位）。

use crate::runtime::Runtime;
use chrono::Utc;
use interaction_core::{
    AgentSessionRecord, AgentSessionState, DomainResult, EventType, KnowledgeStatus, MemoryActor,
    RuntimeEvent, SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// §13 更新觸發與決策（純函式）。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateTrigger {
    /// 使用者加入素材。
    UserAddedAsset,
    /// 已核准來源內容改變（ETag/Last-Modified）。
    SourceChanged,
    /// Repository 新 commit。
    RepoCommit,
    /// 任務產生新 artifact。
    TaskArtifact,
    /// 使用者糾正小樞。
    UserCorrection,
    /// 發現知識衝突。
    ConflictDetected,
    /// 知識超過 reviewAfter。
    ReviewOverdue,
    /// 回答時資料不足／過舊／低信心。
    LowConfidenceAnswer,
    /// 定期健檢（預設只做低成本來源/版本檢查）。
    PeriodicHealthCheck,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDecision {
    pub trigger: UpdateTrigger,
    pub needs_update: bool,
    /// 只有語意工作才需要 AI；hash/metadata/索引/狀態檢查不需要。
    pub needs_ai: bool,
    /// 外部研究／新 session／擴大範圍／產生成本 → 必須先問或依明確設定。
    pub requires_user_ask: bool,
    pub deterministic_steps: Vec<&'static str>,
    pub ai_steps: Vec<&'static str>,
    pub reason: &'static str,
}

/// 確定性決策表（spec §13.1–13.3）。
pub fn classify_update(trigger: UpdateTrigger) -> UpdateDecision {
    use UpdateTrigger as T;
    match trigger {
        T::UserAddedAsset => UpdateDecision {
            trigger,
            needs_update: true,
            needs_ai: true,
            requires_user_ask: false,
            deterministic_steps: vec!["hash", "metadata", "index", "schema-validate"],
            ai_steps: vec!["extract-concepts-claims(candidate)"],
            reason: "新素材：確定性入庫；語意抽取需要 AI 且只能產 Candidate",
        },
        T::SourceChanged => UpdateDecision {
            trigger,
            needs_update: true,
            needs_ai: true,
            requires_user_ask: false,
            deterministic_steps: vec!["etag", "hash", "source-status"],
            ai_steps: vec!["semantic-diff(candidate)"],
            reason: "來源變更：ETag/hash 確定性；新舊語意比較需要 AI",
        },
        T::RepoCommit => UpdateDecision {
            trigger,
            needs_update: true,
            needs_ai: false,
            requires_user_ask: false,
            deterministic_steps: vec!["commit-metadata", "index"],
            ai_steps: vec![],
            reason: "commit 是確定性版本資訊；深入分析等待明確要求",
        },
        T::TaskArtifact => UpdateDecision {
            trigger,
            needs_update: true,
            needs_ai: false,
            requires_user_ask: false,
            deterministic_steps: vec!["artifact-hash", "task-memory"],
            ai_steps: vec![],
            reason: "artifact 確定性登記；經驗回顧另由學習訊號觸發",
        },
        T::UserCorrection => UpdateDecision {
            trigger,
            needs_update: true,
            needs_ai: true,
            requires_user_ask: false,
            deterministic_steps: vec!["record-correction"],
            ai_steps: vec![
                "conflict-analysis(candidate)",
                "knowhow-extraction(candidate)",
            ],
            reason: "使用者糾正：高學習價值；分析結果仍是 Candidate",
        },
        T::ConflictDetected => UpdateDecision {
            trigger,
            needs_update: true,
            needs_ai: true,
            requires_user_ask: false,
            deterministic_steps: vec!["mark-disputed"],
            ai_steps: vec!["conflict-adjudication-proposal(candidate)"],
            reason: "衝突：確定性標 disputed；裁決建議需要 AI 且需人核",
        },
        T::ReviewOverdue => UpdateDecision {
            trigger,
            needs_update: true,
            needs_ai: false,
            requires_user_ask: false,
            deterministic_steps: vec!["mark-stale"],
            ai_steps: vec![],
            reason: "過期：確定性標 stale；重新驗證等待人或明確設定",
        },
        T::LowConfidenceAnswer => UpdateDecision {
            trigger,
            needs_update: true,
            needs_ai: true,
            requires_user_ask: true,
            deterministic_steps: vec!["gap-record"],
            ai_steps: vec!["external-research(needs-consent)"],
            reason: "資料不足：外部研究會產生成本——必須先問或依明確設定",
        },
        T::PeriodicHealthCheck => UpdateDecision {
            trigger,
            needs_update: true,
            needs_ai: false,
            requires_user_ask: false,
            deterministic_steps: vec!["source-availability", "version-check", "stale-sweep"],
            ai_steps: vec![],
            reason: "健檢預設只做低成本確定性檢查，不在背景全面研究",
        },
    }
}

/// §13.4 發布政策。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeKind {
    Metadata,
    IndexUpdate,
    SourceStatus,
    DeterministicVersion,
    AiSummary,
    NewClaim,
    CausalRelation,
    CrossDomainLink,
    NewKnowHow,
    SupersedeKnowledge,
    MediaIntentInference,
    AiExperienceSummary,
    UserLongTermMemory,
    SensitiveData,
    PersonaCoreChange,
    ImportantKnowHowReplacement,
    MedicalLegalFinancialSafety,
    NewAutoUpdateSource,
    ExpandAgentReadableScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PublishClass {
    /// 可自動發布。
    Auto,
    /// 只能建立 Candidate。
    CandidateOnly,
    /// 必須使用者／可信審核確認。
    RequiresConfirmation,
}

pub fn publish_class(kind: ChangeKind) -> PublishClass {
    use ChangeKind as C;
    match kind {
        C::Metadata | C::IndexUpdate | C::SourceStatus | C::DeterministicVersion => {
            PublishClass::Auto
        }
        C::AiSummary
        | C::NewClaim
        | C::CausalRelation
        | C::CrossDomainLink
        | C::NewKnowHow
        | C::SupersedeKnowledge
        | C::MediaIntentInference
        | C::AiExperienceSummary => PublishClass::CandidateOnly,
        C::UserLongTermMemory
        | C::SensitiveData
        | C::PersonaCoreChange
        | C::ImportantKnowHowReplacement
        | C::MedicalLegalFinancialSafety
        | C::NewAutoUpdateSource
        | C::ExpandAgentReadableScope => PublishClass::RequiresConfirmation,
    }
}

// ---------------------------------------------------------------------------
// §17 Knowledge Receipt。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeReceipt {
    pub update_id: String,
    pub triggered_by: String,
    #[serde(default)]
    pub agent_sessions: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub source_hashes: Vec<String>,
    pub changes: ReceiptChanges,
    pub verification: ReceiptVerification,
    pub published: ReceiptPublished,
    pub created_at: interaction_core::Timestamp,
    pub schema_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ReceiptChanges {
    pub added_claims: u32,
    pub updated_relations: u32,
    pub superseded_claims: u32,
    pub candidates_created: u32,
    pub disputed: u32,
    pub stale_marked: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptVerification {
    pub schema_passed: bool,
    pub source_hashes_verified: bool,
    /// "passed" | "conflicts-found" | "unknown"（誠實三態）。
    pub conflict_check: String,
    pub human_reviewed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ReceiptPublished {
    pub metadata: bool,
    pub claims: bool,
}

impl Runtime {
    /// 寫入 receipt＋發 knowledge.updated 事件。
    pub(crate) fn emit_knowledge_receipt(&self, receipt: KnowledgeReceipt) {
        if let Ok(body) = serde_json::to_string(&receipt) {
            let _ = self.store.save_knowledge_receipt(&receipt.update_id, &body);
        }
        self.events.publish(RuntimeEvent::new(
            EventType::KnowledgeUpdated,
            Utc::now(),
            serde_json::to_value(&receipt).unwrap_or_default(),
        ));
    }

    pub async fn knowledge_receipts(&self, limit: u32) -> DomainResult<Value> {
        let bodies = self.store.list_knowledge_receipts(limit)?;
        let items: Vec<Value> = bodies
            .iter()
            .filter_map(|b| serde_json::from_str(b).ok())
            .collect();
        Ok(json!({"receipts": items, "count": items.len()}))
    }

    /// 更新決策（advisory API：host／UI 先問決策器再動手）。
    pub fn knowledge_update_decision(&self, trigger: UpdateTrigger) -> Value {
        serde_json::to_value(classify_update(trigger)).unwrap_or_default()
    }

    // -------------------------------------------------------------------
    // 確定性維護 sweep（watchdog）。
    // -------------------------------------------------------------------

    /// 過了 reviewAfter 的 Active 知識 → Stale（確定性、無 AI）。
    pub async fn knowledge_freshness_sweep(&self) -> u32 {
        let now = Utc::now();
        let mut marked = 0u32;
        if let Ok(bodies) = self.store.list_knowledge_nodes(Some("active"), 1000) {
            for body in bodies {
                if let Ok(mut node) = serde_json::from_str::<interaction_core::KnowledgeNode>(&body)
                {
                    if node.review_after.map(|t| now >= t).unwrap_or(false) {
                        node.status = KnowledgeStatus::Stale;
                        node.updated_at = now;
                        if self.persist_knowledge_node(&node).is_ok() {
                            marked += 1;
                        }
                    }
                }
            }
        }
        if marked > 0 {
            self.emit_knowledge_receipt(KnowledgeReceipt {
                update_id: format!("kr-{}", uuid::Uuid::new_v4()),
                triggered_by: "review-overdue".into(),
                agent_sessions: vec![],
                sources: vec![],
                source_hashes: vec![],
                changes: ReceiptChanges {
                    stale_marked: marked,
                    ..Default::default()
                },
                verification: ReceiptVerification {
                    schema_passed: true,
                    source_hashes_verified: false,
                    conflict_check: "unknown".into(),
                    human_reviewed: false,
                },
                published: ReceiptPublished {
                    metadata: true,
                    claims: false,
                },
                created_at: Utc::now(),
                schema_version: SCHEMA_VERSION.into(),
            });
        }
        marked
    }

    /// 確定性衝突檢查：兩個 Active 節點之間有 active Contradicts 邊 →
    /// 雙方標 Disputed（不猜對錯——裁決屬於人類／後續 AI 候選提案）。
    pub async fn knowledge_conflict_check(&self, node_id: &str) -> DomainResult<Value> {
        let node = self.knowledge_get(node_id).await?;
        let mut disputed = Vec::new();
        if node.status == KnowledgeStatus::Active {
            let edges = self.store.edges_touching(node_id, 200)?;
            for body in edges {
                let Ok(edge) = serde_json::from_str::<interaction_core::KnowledgeEdge>(&body)
                else {
                    continue;
                };
                if !matches!(
                    edge.relation,
                    interaction_core::RelationType::Contradicts
                        | interaction_core::RelationType::ConflictsWith
                ) {
                    continue;
                }
                let other_id = if edge.from.as_str() == node_id {
                    edge.to.clone()
                } else {
                    edge.from.clone()
                };
                if let Ok(mut other) = self.knowledge_get(other_id.as_str()).await {
                    if other.status == KnowledgeStatus::Active {
                        other.status = KnowledgeStatus::Disputed;
                        other.updated_at = Utc::now();
                        let _ = self.persist_knowledge_node(&other);
                        let mut me = self.knowledge_get(node_id).await?;
                        me.status = KnowledgeStatus::Disputed;
                        me.updated_at = Utc::now();
                        let _ = self.persist_knowledge_node(&me);
                        disputed.push(other_id.as_str().to_string());
                    }
                }
            }
        }
        if !disputed.is_empty() {
            self.emit_knowledge_receipt(KnowledgeReceipt {
                update_id: format!("kr-{}", uuid::Uuid::new_v4()),
                triggered_by: "conflict-detected".into(),
                agent_sessions: vec![],
                sources: vec![],
                source_hashes: vec![],
                changes: ReceiptChanges {
                    disputed: (disputed.len() + 1) as u32,
                    ..Default::default()
                },
                verification: ReceiptVerification {
                    schema_passed: true,
                    source_hashes_verified: false,
                    conflict_check: "conflicts-found".into(),
                    human_reviewed: false,
                },
                published: ReceiptPublished::default(),
                created_at: Utc::now(),
                schema_version: SCHEMA_VERSION.into(),
            });
        }
        Ok(json!({"nodeId": node_id, "disputedWith": disputed}))
    }

    // -------------------------------------------------------------------
    // §14 經驗轉知識。
    // -------------------------------------------------------------------

    /// 任務結束後的確定性收集：目標／工具／結果／成本／重試 → TaskMemory。
    /// 不用 AI；學習訊號另行判斷是否建立 Reflection Candidate。
    pub(crate) fn record_task_experience(
        &self,
        record: &AgentSessionRecord,
        prior_state: AgentSessionState,
    ) {
        let now = Utc::now();
        let outcome = format!("{prior_state:?}");
        let content = json!({
            "goal": record.label,
            "agentId": record.agent_id,
            "outcome": outcome,
            "costUsd": record.budget.spent_cost,
            "messages": record.budget.spent_messages,
            "providerSessionId": record.provider_session_id,
            "closedAt": record.closed_at,
        })
        .to_string();
        let mut item = interaction_core::new_memory_item(
            interaction_core::MemoryLayer::TaskMemory,
            interaction_core::MemoryKind::Fact, // runtime 觀測的執行事實
            format!(
                "任務：{}",
                record
                    .label
                    .clone()
                    .unwrap_or_else(|| record.agent_id.clone())
            ),
            content,
            MemoryActor::Runtime,
            now,
        );
        item.provenance = vec![format!("agent-session:{}", record.session_id.as_str())];
        let _ = self.memory_create_internal(&item);

        // 學習訊號（§14）：非預期失敗／取消後重試等——只有有訊號才建 Reflection。
        let signals = learning_signals(record, prior_state);
        if !signals.is_empty() {
            let node = interaction_core::KnowledgeNode {
                node_id: interaction_core::KnowledgeNodeId::generate(),
                node_type: interaction_core::NodeType::Claim,
                title: format!(
                    "經驗候選：{}",
                    record
                        .label
                        .clone()
                        .unwrap_or_else(|| record.agent_id.clone())
                ),
                content: json!({
                    "signals": signals,
                    "outcome": outcome,
                    "什麼": "此任務值得回顧",
                    "升格條件": "需要證據、反例與適用範圍，並經人類複審",
                })
                .to_string(),
                status: KnowledgeStatus::Candidate,
                confidence: 0.4,
                created_by: MemoryActor::Runtime,
                evidence: vec![interaction_core::SourceRef {
                    note: Some(format!("agent-session:{}", record.session_id.as_str())),
                    ..Default::default()
                }],
                domains: vec!["learning-from-feedback".into()],
                counterexamples: vec![],
                applicability: None,
                version: 1,
                supersedes: None,
                review_after: Some(now + chrono::Duration::days(60)),
                reviews: vec![],
                created_at: now,
                updated_at: now,
                schema_version: SCHEMA_VERSION.into(),
            };
            if self.persist_knowledge_node(&node).is_ok() {
                self.emit_knowledge_receipt(KnowledgeReceipt {
                    update_id: format!("kr-{}", uuid::Uuid::new_v4()),
                    triggered_by: "task-experience".into(),
                    agent_sessions: vec![record.session_id.as_str().to_string()],
                    sources: vec![],
                    source_hashes: vec![],
                    changes: ReceiptChanges {
                        candidates_created: 1,
                        ..Default::default()
                    },
                    verification: ReceiptVerification {
                        schema_passed: true,
                        source_hashes_verified: false,
                        conflict_check: "unknown".into(),
                        human_reviewed: false,
                    },
                    published: ReceiptPublished::default(),
                    created_at: now,
                    schema_version: SCHEMA_VERSION.into(),
                });
            }
        }
    }
}

/// 學習價值訊號（§14）：純函式（以 close 前的實際結局判斷）。
pub fn learning_signals(
    record: &AgentSessionRecord,
    prior_state: AgentSessionState,
) -> Vec<&'static str> {
    let mut signals = Vec::new();
    match prior_state {
        AgentSessionState::Failed => signals.push("unexpected-failure"),
        AgentSessionState::TimedOut => signals.push("timeout"),
        _ => {}
    }
    if record.budget.max_cost > 0.0 && record.budget.spent_cost > record.budget.max_cost * 0.8 {
        signals.push("near-budget-exhaustion");
    }
    signals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_updates_never_need_ai() {
        for t in [
            UpdateTrigger::RepoCommit,
            UpdateTrigger::TaskArtifact,
            UpdateTrigger::ReviewOverdue,
            UpdateTrigger::PeriodicHealthCheck,
        ] {
            let d = classify_update(t);
            assert!(!d.needs_ai, "{t:?} 不應呼叫 AI");
            assert!(d.ai_steps.is_empty());
        }
    }

    #[test]
    fn semantic_work_needs_ai_and_external_research_needs_consent() {
        assert!(classify_update(UpdateTrigger::UserAddedAsset).needs_ai);
        assert!(classify_update(UpdateTrigger::UserCorrection).needs_ai);
        let research = classify_update(UpdateTrigger::LowConfidenceAnswer);
        assert!(research.requires_user_ask, "外部研究必須先問");
        // 健檢不做背景全面研究。
        assert!(!classify_update(UpdateTrigger::PeriodicHealthCheck).requires_user_ask);
    }

    #[test]
    fn publish_policy_matches_the_spec_table() {
        assert_eq!(publish_class(ChangeKind::Metadata), PublishClass::Auto);
        assert_eq!(publish_class(ChangeKind::SourceStatus), PublishClass::Auto);
        for k in [
            ChangeKind::AiSummary,
            ChangeKind::NewClaim,
            ChangeKind::CausalRelation,
            ChangeKind::CrossDomainLink,
            ChangeKind::NewKnowHow,
            ChangeKind::SupersedeKnowledge,
            ChangeKind::MediaIntentInference,
            ChangeKind::AiExperienceSummary,
        ] {
            assert_eq!(publish_class(k), PublishClass::CandidateOnly, "{k:?}");
        }
        for k in [
            ChangeKind::UserLongTermMemory,
            ChangeKind::SensitiveData,
            ChangeKind::PersonaCoreChange,
            ChangeKind::ImportantKnowHowReplacement,
            ChangeKind::MedicalLegalFinancialSafety,
            ChangeKind::NewAutoUpdateSource,
            ChangeKind::ExpandAgentReadableScope,
        ] {
            assert_eq!(
                publish_class(k),
                PublishClass::RequiresConfirmation,
                "{k:?}"
            );
        }
    }
}
