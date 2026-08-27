//! 知識系統垂直閉環：CAS write-once、claim 要證據、agent 只能 Candidate、
//! 人類 approve 才 active、agent approve 降留言、類比≠因果、
//! supersede 版本化、FTS＋向量檢索、刪素材影響（disputed＋級聯）。

use chrono::Utc;
use interaction_core::*;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;

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

fn claim(title: &str, evidence: Vec<SourceRef>) -> KnowledgeNode {
    KnowledgeNode {
        node_id: KnowledgeNodeId::generate(),
        node_type: NodeType::Claim,
        title: title.into(),
        content: format!("{title} 的內容"),
        status: KnowledgeStatus::Active, // 服務層會依 actor 修正
        confidence: 0.7,
        created_by: MemoryActor::Human,
        evidence,
        domains: vec!["daw".into()],
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

fn url_evidence() -> Vec<SourceRef> {
    vec![SourceRef {
        url: Some("https://example.com/paper".into()),
        segment: Some("page=3".into()),
        ..Default::default()
    }]
}

#[tokio::test]
async fn assets_are_content_addressed_and_write_once() {
    let (_g, rt) = runtime().await;
    let a = rt
        .asset_import(None, Some("同一份文字內容"), None, "user-import", None)
        .await
        .unwrap();
    let b = rt
        .asset_import(
            None,
            Some("同一份文字內容"),
            None,
            "user-import",
            Some("重複".into()),
        )
        .await
        .unwrap();
    assert_eq!(a.hash, b.hash, "同內容同 hash");
    // write-once：第二次匯入不覆寫（description 保持第一次的 None）。
    assert_eq!(b.description, None, "中繼資料不可被覆寫");
    let listed = rt.asset_list(10).await.unwrap();
    assert_eq!(listed["count"], 1, "只有一筆");
    // 內容可讀回。
    let bytes = rt.asset_content(&a.hash, 1024).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&bytes), "同一份文字內容");
}

#[tokio::test]
async fn agent_proposals_are_candidates_until_a_human_approves() {
    let (_g, rt) = runtime().await;
    // agent 提案（即使填了 Active）→ Candidate。
    let node = rt
        .knowledge_propose_node(
            claim("EQ 增益疊加會削波", url_evidence()),
            MemoryActor::Agent("claude-code".into()),
        )
        .await
        .unwrap();
    assert_eq!(node.status, KnowledgeStatus::Candidate);
    assert!(!node.status.usable());

    // agent 想 approve → 降為留言，狀態不變。
    let after = rt
        .knowledge_review(
            node.node_id.as_str(),
            "approve",
            Some("我覺得沒問題".into()),
            MemoryActor::Agent("codex".into()),
        )
        .await
        .unwrap();
    assert_eq!(
        after.status,
        KnowledgeStatus::Candidate,
        "agent 不能自我核可"
    );
    assert_eq!(after.reviews[0].verdict, "comment");

    // 人類 approve → Active。
    let approved = rt
        .knowledge_review(node.node_id.as_str(), "approve", None, MemoryActor::Human)
        .await
        .unwrap();
    assert_eq!(approved.status, KnowledgeStatus::Active);
    assert!(approved.status.usable());
}

#[tokio::test]
async fn claims_need_evidence_and_analogies_cannot_be_causal() {
    let (_g, rt) = runtime().await;
    // 無證據 claim 拒絕。
    let err = rt
        .knowledge_propose_node(claim("沒有證據的主張", vec![]), MemoryActor::Human)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)));

    // 兩個節點＋類比因果邊 → 拒絕。
    let a = rt
        .knowledge_propose_node(claim("A", url_evidence()), MemoryActor::Human)
        .await
        .unwrap();
    let b = rt
        .knowledge_propose_node(claim("B", url_evidence()), MemoryActor::Human)
        .await
        .unwrap();
    let edge = KnowledgeEdge {
        edge_id: KnowledgeEdgeId::generate(),
        from: a.node_id.clone(),
        to: b.node_id.clone(),
        relation: RelationType::Causes,
        origin: EdgeOrigin::AiConjecture,
        status: KnowledgeStatus::Candidate,
        confidence: 0.9,
        created_by: MemoryActor::Human,
        rationale: None,
        created_at: Utc::now(),
        schema_version: SCHEMA_VERSION.into(),
    };
    assert!(rt
        .knowledge_propose_edge(edge.clone(), MemoryActor::Human)
        .await
        .is_err());
    // 同關係改 analogy → 允許（誠實標示為類比）。
    let mut ok_edge = edge;
    ok_edge.relation = RelationType::Analogy;
    rt.knowledge_propose_edge(ok_edge, MemoryActor::Human)
        .await
        .unwrap();
    // 圖譜展開看得到鄰居。
    let graph = rt.knowledge_graph(a.node_id.as_str(), 1).await.unwrap();
    assert_eq!(graph["neighbors"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn search_finds_via_fts_and_vector_candidates() {
    let (_g, rt) = runtime().await;
    let node = rt
        .knowledge_propose_node(
            claim("Limiter 的 lookahead 會引入延遲", url_evidence()),
            MemoryActor::Agent("claude-code".into()),
        )
        .await
        .unwrap();
    assert_eq!(node.status, KnowledgeStatus::Candidate);
    let results = rt.knowledge_search("lookahead 延遲", 10).await.unwrap();
    let hits = results["results"].as_array().unwrap();
    assert!(
        hits.iter().any(|h| h["nodeId"] == node.node_id.as_str()),
        "檢索找得到節點：{results}"
    );
    // 檢索誠實：說明 vector 是 lexical fallback、結果只是候選。
    assert!(results["retrievalNote"]
        .as_str()
        .unwrap()
        .contains("lexical-fallback"));
    // usable 誠實反映 candidate 狀態。
    let hit = hits
        .iter()
        .find(|h| h["nodeId"] == node.node_id.as_str())
        .unwrap();
    assert_eq!(hit["usable"], false, "candidate 不可用於一般回答");
    // 畸形查詢不 panic。
    let _ = rt.knowledge_search("\"unbalanced OR (", 5).await.unwrap();
}

#[tokio::test]
async fn supersede_archives_the_old_version() {
    let (_g, rt) = runtime().await;
    let v1 = rt
        .knowledge_propose_node(claim("舊版主張", url_evidence()), MemoryActor::Human)
        .await
        .unwrap();
    rt.knowledge_review(v1.node_id.as_str(), "approve", None, MemoryActor::Human)
        .await
        .unwrap();
    let mut v2 = claim("新版主張", url_evidence());
    v2.supersedes = Some(v1.node_id.clone());
    let v2 = rt
        .knowledge_propose_node(v2, MemoryActor::Agent("claude-code".into()))
        .await
        .unwrap();
    assert_eq!(v2.status, KnowledgeStatus::Candidate, "取代提案也是候選");
    // 人類核可新版 → 舊版 superseded（版本化封存，不參與一般回答）。
    rt.knowledge_review(v2.node_id.as_str(), "approve", None, MemoryActor::Human)
        .await
        .unwrap();
    let old = rt.knowledge_get(v1.node_id.as_str()).await.unwrap();
    assert_eq!(old.status, KnowledgeStatus::Superseded);
    assert!(!old.status.usable());
}

#[tokio::test]
async fn deleting_an_asset_disputes_knowledge_and_cascades_derivatives() {
    let (_g, rt) = runtime().await;
    let asset = rt
        .asset_import(None, Some("錄音檔的文字替身"), None, "user-import", None)
        .await
        .unwrap();
    // 引用素材的 active claim。
    let node = rt
        .knowledge_propose_node(
            claim(
                "錄音顯示節奏偏快",
                vec![SourceRef {
                    asset_hash: Some(asset.hash.clone()),
                    segment: Some("t=12.5-30.2".into()),
                    ..Default::default()
                }],
            ),
            MemoryActor::Human,
        )
        .await
        .unwrap();
    rt.knowledge_review(node.node_id.as_str(), "approve", None, MemoryActor::Human)
        .await
        .unwrap();
    // 隨父刪除的衍生記憶。
    let mut derived = new_memory_item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Inference,
        "衍生摘要",
        "從錄音得到的摘要",
        MemoryActor::Runtime,
        Utc::now(),
    );
    derived.retention.delete_with_parent = Some(asset.hash.clone());
    rt.memory_create(derived.clone()).await.unwrap();

    // 影響預覽誠實列出。
    let impact = rt.asset_delete_impact(&asset.hash).await.unwrap();
    assert!(impact["referencingKnowledgeNodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == node.node_id.as_str()));
    assert_eq!(
        impact["memoriesDeletedWithParent"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // 刪除：衍生記憶級聯刪；Active 知識 → disputed（不靜默消失）。
    rt.asset_delete(&asset.hash).await.unwrap();
    let after = rt.knowledge_get(node.node_id.as_str()).await.unwrap();
    assert_eq!(after.status, KnowledgeStatus::Disputed);
    assert!(after.reviews.iter().any(|r| r.note.contains("已刪除")));
    let mems = rt.memory_list(Some("domain-knowledge"), 100).await.unwrap();
    assert_eq!(mems["count"], 0, "隨父刪除生效");
    // evidence 指向不存在素材的新提案 → 拒絕。
    let err = rt
        .knowledge_propose_node(
            claim(
                "引用已刪素材",
                vec![SourceRef {
                    asset_hash: Some(asset.hash.clone()),
                    ..Default::default()
                }],
            ),
            MemoryActor::Human,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)));
    let _ = json!({});
}
