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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AssetDerivativeKind {
    Thumbnail,
    OcrText,
    Transcript,
    AudioFeatures,
    VideoMetadata,
    Keyframe,
    Subtitle,
    PdfText,
    CodeIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AssetDerivativeStatus {
    Complete,
    Unavailable,
    Failed,
}

/// A derived artifact never overwrites its parent. Complete rows point to a
/// second content-addressed asset and every row carries an exact parent
/// segment plus processor identity/version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssetDerivative {
    pub derivative_id: String,
    pub parent_hash: String,
    pub kind: AssetDerivativeKind,
    pub status: AssetDerivativeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_hash: Option<String>,
    pub source: SourceRef,
    pub processor: String,
    pub processor_version: String,
    pub detail: String,
    pub created_at: Timestamp,
    pub schema_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssetDerivationReport {
    pub asset_hash: String,
    pub derivatives: Vec<AssetDerivative>,
    pub completed_at: Timestamp,
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
    // 每筆 evidence 都要有可驗證的指向——空的 SourceRef 或只有 note 的
    // 條目不構成 provenance，否則證據門檻可被 `evidence: [{}]` 繞過。
    if node.node_type == NodeType::Claim {
        let has_ref = |s: &Option<String>| s.as_deref().is_some_and(|v| !v.trim().is_empty());
        let ok = !node.evidence.is_empty()
            && node
                .evidence
                .iter()
                .all(|e| has_ref(&e.asset_hash) || has_ref(&e.url));
        if !ok {
            return Err(
                "claim 的每筆 evidence 必須含 assetHash 或 url（素材 hash／URL＋片段；note 不足以作為證據）"
                    .into(),
            );
        }
    }
    Ok(())
}

/// 複審狀態機閘門（spec §15）：superseded／archived 是版本化終態——
/// 不得經 review 復活（否則同一主張會出現兩個 Active 版本）。
/// approve 允許自 candidate/stale/disputed 升格，對 active 是冪等的
/// 再確認；active 知識要退場走 supersede，不走 reject。
/// comment 任何狀態皆可（留言不改狀態）。
pub fn validate_review_transition(current: KnowledgeStatus, verdict: &str) -> Result<(), String> {
    if verdict != "approve" && verdict != "reject" {
        return Ok(());
    }
    match current {
        KnowledgeStatus::Candidate | KnowledgeStatus::Stale | KnowledgeStatus::Disputed => Ok(()),
        KnowledgeStatus::Active => {
            if verdict == "approve" {
                Ok(()) // 再確認（冪等）。
            } else {
                Err(
                    "active 知識不可直接 reject——請提出取代版本（supersede）或經衝突流程標記爭議"
                        .into(),
                )
            }
        }
        KnowledgeStatus::Superseded | KnowledgeStatus::Archived => Err(format!(
            "節點狀態為 {current:?}（版本化封存），不得經 review 復活；請提出取代版本（supersede）"
        )),
    }
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
    fn empty_or_note_only_source_refs_never_satisfy_the_evidence_gate() {
        // 空 SourceRef（全欄位 None）不算證據。
        let mut n = node(NodeType::Claim, MemoryActor::Human);
        n.evidence = vec![SourceRef::default()];
        assert!(validate_node(&n).is_err(), "evidence: [{{}}] 必須被拒");
        // 只有 note 不算證據。
        n.evidence = vec![SourceRef {
            note: Some("trust me".into()),
            ..Default::default()
        }];
        assert!(validate_node(&n).is_err(), "note-only 必須被拒");
        // 空白字串視同缺席。
        n.evidence = vec![SourceRef {
            url: Some("   ".into()),
            ..Default::default()
        }];
        assert!(validate_node(&n).is_err(), "空白 url 必須被拒");
        // 混入一筆空條目也不行（all 條目都要有指向）。
        n.evidence = vec![
            SourceRef {
                url: Some("https://example.com".into()),
                ..Default::default()
            },
            SourceRef::default(),
        ];
        assert!(validate_node(&n).is_err(), "夾帶空條目必須被拒");
        // assetHash 或 url 任一即可。
        n.evidence = vec![SourceRef {
            asset_hash: Some("a".repeat(64)),
            ..Default::default()
        }];
        assert!(validate_node(&n).is_ok());
        // Entity 不受此門檻影響。
        let mut e = node(NodeType::Entity, MemoryActor::Human);
        e.evidence = vec![SourceRef::default()];
        assert!(validate_node(&e).is_ok());
    }

    #[test]
    fn review_transitions_respect_the_state_machine() {
        use KnowledgeStatus as S;
        // 未定案狀態可 approve/reject。
        for s in [S::Candidate, S::Stale, S::Disputed] {
            assert!(validate_review_transition(s, "approve").is_ok(), "{s:?}");
            assert!(validate_review_transition(s, "reject").is_ok(), "{s:?}");
        }
        // 版本化終態不得復活。
        for s in [S::Superseded, S::Archived] {
            assert!(validate_review_transition(s, "approve").is_err(), "{s:?}");
            assert!(validate_review_transition(s, "reject").is_err(), "{s:?}");
        }
        // active：approve 是冪等再確認；reject 必須走 supersede。
        assert!(validate_review_transition(S::Active, "approve").is_ok());
        assert!(validate_review_transition(S::Active, "reject").is_err());
        // comment 任何狀態皆可。
        for s in [
            S::Candidate,
            S::Active,
            S::Stale,
            S::Disputed,
            S::Superseded,
            S::Archived,
        ] {
            assert!(validate_review_transition(s, "comment").is_ok(), "{s:?}");
        }
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
