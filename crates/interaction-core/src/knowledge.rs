//! 知識系統領域模型（spec §11）：內容定址素材＋版本化知識圖譜。
//!
//! 不變量：
//! - 原始素材以 SHA-256 定址、write-once——AI 不可覆寫、不可刪除來源。
//! - 所有衍生內容（摘要/OCR/轉錄/主張）必須指回原始素材與精確片段。
//! - AI 只能建立 **Candidate**；發布（active）永遠是人類／可信審核的動作。
//! - 語意相似（embedding/類比）不可直接標成因果——validate 拒絕。
//! - 知識狀態機：candidate → active → stale → disputed → superseded →
//!   archived → deleted；superseded 版本化封存，不參與一般回答。

use crate::{KnowledgeEdgeId, KnowledgeNodeId, MemoryActor, Timestamp};
use serde::{Deserialize, Serialize};

/// 素材媒體類別（多模態）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MediaType {
    Text,
    Image,
    Audio,
    Video,
    Code,
    Data,
    Pdf,
    Other,
}

/// 內容定址的原始素材紀錄（blob 在檔案系統，中繼資料在此）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssetRecord {
    /// SHA-256 hex——素材身分＝內容雜湊，不可變。
    pub hash: String,
    pub media_type: MediaType,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    /// 來源描述（user-import／url:…／task-artifact:…）。
    pub source: String,
    pub added_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub schema_version: String,
}

/// 指回原始素材的精確引用（時間段／區域／行號）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_hash: Option<String>,
    /// 片段語法：`t=12.5-30.2`（秒）｜`region=x,y,w,h`｜`lines=10-42`｜`page=3`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 知識節點狀態機（spec §15）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum KnowledgeStatus {
    Candidate,
    Active,
    Stale,
    Disputed,
    Superseded,
    Archived,
}

impl KnowledgeStatus {
    /// 一般回答／bundle 可用的狀態（superseded/archived 不參與）。
    pub fn usable(&self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum NodeType {
    Entity,
    Claim,
    Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeNode {
    pub node_id: KnowledgeNodeId,
    pub node_type: NodeType,
    pub title: String,
    pub content: String,
    pub status: KnowledgeStatus,
    pub confidence: f64,
    pub created_by: MemoryActor,
    /// 證據：指回素材／URL 的精確引用。Claim 必須至少一個。
    #[serde(default)]
    pub evidence: Vec<SourceRef>,
    #[serde(default)]
    pub domains: Vec<String>,
    /// 反例與適用範圍（升格 know-how 必填的誠實欄位）。
    #[serde(default)]
    pub counterexamples: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applicability: Option<String>,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<KnowledgeNodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_after: Option<Timestamp>,
    /// 複審紀錄（agent 可 submit-review 留言；只有人類 approve 才 activate）。
    #[serde(default)]
    pub reviews: Vec<KnowledgeReview>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub schema_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeReview {
    pub reviewer: MemoryActor,
    /// comment｜approve｜reject（approve 只有人類有效——服務層強制）。
    pub verdict: String,
    pub note: String,
    pub at: Timestamp,
}

/// 關係類型（spec §11 清單）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RelationType {
    Supports,
    Contradicts,
    Causes,
    Influences,
    AppliesTo,
    ImplementedBy,
    SimilarTo,
    Analogy,
    DesignTransfer,
    DerivedFrom,
    Supersedes,
    ConflictsWith,
}

/// 連結性質（跨領域連結必須標記；spec §11）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeOrigin {
    /// 研究支持。
    ResearchSupported,
    /// 工程類比。
    EngineeringAnalogy,
    /// 創作啟發。
    CreativeInspiration,
    /// AI 推測。
    AiConjecture,
    /// 使用者確認的設計原則。
    UserConfirmed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEdge {
    pub edge_id: KnowledgeEdgeId,
    pub from: KnowledgeNodeId,
    pub to: KnowledgeNodeId,
    pub relation: RelationType,
    pub origin: EdgeOrigin,
    pub status: KnowledgeStatus,
    pub confidence: f64,
    pub created_by: MemoryActor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    pub created_at: Timestamp,
    pub schema_version: String,
}

pub const MAX_KNOWLEDGE_CONTENT_BYTES: usize = 16 * 1024;
pub const MAX_KNOWLEDGE_TITLE_CHARS: usize = 160;

pub fn validate_node(node: &KnowledgeNode) -> Result<(), String> {
    if node.title.trim().is_empty() || node.title.chars().count() > MAX_KNOWLEDGE_TITLE_CHARS {
        return Err(format!("title 必須為 1..{MAX_KNOWLEDGE_TITLE_CHARS} 字"));
    }
    if node.content.len() > MAX_KNOWLEDGE_CONTENT_BYTES {
        return Err(format!("content 超過 {MAX_KNOWLEDGE_CONTENT_BYTES} bytes"));
    }
    if !(0.0..=1.0).contains(&node.confidence) {
        return Err("confidence 必須在 0..1".into());
    }
    // Claim 必須有證據指引（可稽核；沒有證據的主張不能進圖譜）。
    if node.node_type == NodeType::Claim && node.evidence.is_empty() {
        return Err("claim 必須附至少一個 evidence（素材 hash／URL＋片段）".into());
    }
    Ok(())
}

/// 邊驗證：類比／啟發／AI 推測不可宣稱因果（spec：語意相似≠因果）。
pub fn validate_edge(edge: &KnowledgeEdge) -> Result<(), String> {
    if !(0.0..=1.0).contains(&edge.confidence) {
        return Err("confidence 必須在 0..1".into());
    }
    if edge.from == edge.to {
        return Err("邊不可指向自己".into());
    }
    let causal = matches!(
        edge.relation,
        RelationType::Causes | RelationType::Influences
    );
    let weak_origin = matches!(
        edge.origin,
        EdgeOrigin::EngineeringAnalogy | EdgeOrigin::CreativeInspiration | EdgeOrigin::AiConjecture
    );
    if causal && weak_origin {
        return Err(
            "類比／啟發／AI 推測不可直接標成因果（causes/influences 需要 research-supported 或 user-confirmed）"
                .into(),
        );
    }
    Ok(())
}

/// actor 規則：agent 建立的節點／邊一律 Candidate（永不直接 active）。
pub fn apply_knowledge_actor_rules(status: &mut KnowledgeStatus, actor: &MemoryActor) {
    if matches!(actor, MemoryActor::Agent(_)) {
        *status = KnowledgeStatus::Candidate;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SCHEMA_VERSION;
    use chrono::Utc;

    fn node(node_type: NodeType, actor: MemoryActor) -> KnowledgeNode {
        KnowledgeNode {
            node_id: KnowledgeNodeId::generate(),
            node_type,
            title: "測試節點".into(),
            content: "內容".into(),
            status: KnowledgeStatus::Active,
            confidence: 0.8,
            created_by: actor,
            evidence: vec![SourceRef {
                url: Some("https://example.com".into()),
                ..Default::default()
            }],
            domains: vec![],
            counterexamples: vec![],
            applicability: None,
            version: 1,
            supersedes: None,
            review_after: None,
            reviews: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            schema_version: SCHEMA_VERSION.into(),
        }
    }

    #[test]
    fn claims_require_evidence() {
        let mut n = node(NodeType::Claim, MemoryActor::Human);
        assert!(validate_node(&n).is_ok());
        n.evidence.clear();
        assert!(validate_node(&n).is_err());
        // Entity 不強制證據。
        let mut e = node(NodeType::Entity, MemoryActor::Human);
        e.evidence.clear();
        assert!(validate_node(&e).is_ok());
    }

    #[test]
    fn analogies_can_never_claim_causality() {
        let mk = |relation, origin| KnowledgeEdge {
            edge_id: KnowledgeEdgeId::generate(),
            from: KnowledgeNodeId::new("kn-a"),
            to: KnowledgeNodeId::new("kn-b"),
            relation,
            origin,
            status: KnowledgeStatus::Candidate,
            confidence: 0.5,
            created_by: MemoryActor::Human,
            rationale: None,
            created_at: Utc::now(),
            schema_version: SCHEMA_VERSION.into(),
        };
        assert!(validate_edge(&mk(RelationType::Causes, EdgeOrigin::AiConjecture)).is_err());
        assert!(validate_edge(&mk(RelationType::Causes, EdgeOrigin::EngineeringAnalogy)).is_err());
        assert!(validate_edge(&mk(RelationType::Causes, EdgeOrigin::ResearchSupported)).is_ok());
        assert!(validate_edge(&mk(RelationType::Analogy, EdgeOrigin::AiConjecture)).is_ok());
        assert!(validate_edge(&mk(
            RelationType::Influences,
            EdgeOrigin::CreativeInspiration
        ))
        .is_err());
    }

    #[test]
    fn agents_only_ever_create_candidates() {
        let mut status = KnowledgeStatus::Active;
        apply_knowledge_actor_rules(&mut status, &MemoryActor::Agent("codex".into()));
        assert_eq!(status, KnowledgeStatus::Candidate);
        let mut status = KnowledgeStatus::Active;
        apply_knowledge_actor_rules(&mut status, &MemoryActor::Human);
        assert_eq!(status, KnowledgeStatus::Active);
    }

    #[test]
    fn superseded_is_not_usable_in_answers() {
        assert!(KnowledgeStatus::Active.usable());
        for s in [
            KnowledgeStatus::Candidate,
            KnowledgeStatus::Stale,
            KnowledgeStatus::Disputed,
            KnowledgeStatus::Superseded,
            KnowledgeStatus::Archived,
        ] {
            assert!(!s.usable(), "{s:?} 不可參與一般回答");
        }
    }
}
