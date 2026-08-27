//! 決策器＋經驗轉知識＋Receipt 閉環：
//! freshness sweep 標 stale、衝突標 disputed（雙方）、
//! 經驗候選升格閘門（反例＋適用範圍必填）、任務結束確定性收集、
//! receipt 誠實記錄（human_reviewed／conflict_check 三態）。

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

fn claim(title: &str) -> KnowledgeNode {
    KnowledgeNode {
        node_id: KnowledgeNodeId::generate(),
        node_type: NodeType::Claim,
        title: title.into(),
        content: "內容".into(),
        status: KnowledgeStatus::Active,
        confidence: 0.7,
        created_by: MemoryActor::Human,
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

#[tokio::test]
async fn freshness_sweep_marks_overdue_active_as_stale() {
    let (_g, rt) = runtime().await;
    let mut n = claim("會過期的知識");
    n.review_after = Some(Utc::now() - chrono::Duration::days(1));
    let n = rt
        .knowledge_propose_node(n, MemoryActor::Human)
        .await
        .unwrap();
    assert_eq!(n.status, KnowledgeStatus::Active);
    let marked = rt.knowledge_freshness_sweep().await;
    assert_eq!(marked, 1);
    let after = rt.knowledge_get(n.node_id.as_str()).await.unwrap();
    assert_eq!(after.status, KnowledgeStatus::Stale);
    // receipt 誠實記錄 staleMarked。
    let receipts = rt.knowledge_receipts(10).await.unwrap();
    assert!(receipts["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["changes"]["staleMarked"] == 1));
}

#[tokio::test]
async fn active_contradiction_disputes_both_sides() {
    let (_g, rt) = runtime().await;
    let a = rt
        .knowledge_propose_node(claim("A 為真"), MemoryActor::Human)
        .await
        .unwrap();
    let b = rt
        .knowledge_propose_node(claim("A 為假"), MemoryActor::Human)
        .await
        .unwrap();
    let edge = KnowledgeEdge {
        edge_id: KnowledgeEdgeId::generate(),
        from: a.node_id.clone(),
        to: b.node_id.clone(),
        relation: RelationType::Contradicts,
        origin: EdgeOrigin::UserConfirmed,
        status: KnowledgeStatus::Active,
        confidence: 0.9,
        created_by: MemoryActor::Human,
        rationale: None,
        created_at: Utc::now(),
        schema_version: SCHEMA_VERSION.into(),
    };
    rt.knowledge_propose_edge(edge, MemoryActor::Human)
        .await
        .unwrap();
    let out = rt
        .knowledge_conflict_check(a.node_id.as_str())
        .await
        .unwrap();
    assert_eq!(out["disputedWith"].as_array().unwrap().len(), 1);
    // 雙方都 disputed——系統不猜誰對。
    assert_eq!(
        rt.knowledge_get(a.node_id.as_str()).await.unwrap().status,
        KnowledgeStatus::Disputed
    );
    assert_eq!(
        rt.knowledge_get(b.node_id.as_str()).await.unwrap().status,
        KnowledgeStatus::Disputed
    );
}

#[tokio::test]
async fn experience_candidates_cannot_promote_without_counterexamples() {
    let (_g, rt) = runtime().await;
    // 任務失敗 → 確定性經驗收集＋Reflection Candidate。
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let record = rt
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: None,
            agent_id: "agent.x".into(),
            label: Some("會失敗的任務".into()),
            ttl_minutes: Some(10),
            data_scope: vec![],
            tool_scope: vec![],
            consent_scope: vec![],
            max_cost: None,
            max_messages: None,
            delegation: None,
            workdir: None,
        })
        .await
        .unwrap();
    let sid = record.session_id.as_str().to_string();
    rt.report_agent_session(&sid, "failed", json!({"error": "boom"}))
        .await
        .unwrap();
    rt.close_agent_session(&sid, None, "closed").await.unwrap();

    // TaskMemory 確定性收集（無 AI）。
    let tasks = rt.memory_list(Some("task-memory"), 10).await.unwrap();
    assert_eq!(tasks["count"], 1);
    assert_eq!(tasks["items"][0]["kind"], "fact", "runtime 觀測的執行事實");

    // Reflection Candidate 存在（失敗＝學習訊號）。
    let candidates = rt.knowledge_list(Some("candidate"), 10).await.unwrap();
    let reflection = candidates["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| {
            n["domains"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d == "learning-from-feedback")
        })
        .expect("reflection candidate created")
        .clone();
    let rid = reflection["nodeId"].as_str().unwrap();

    // 升格閘門：沒有反例＋適用範圍 → approve 被拒。
    let err = rt
        .knowledge_review(rid, "approve", None, MemoryActor::Human)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");

    // 補上反例與適用範圍後才可升格。
    let mut node = rt.knowledge_get(rid).await.unwrap();
    node.counterexamples = vec!["在 CI 環境不適用".into()];
    node.applicability = Some("只適用於本機互動式任務".into());
    // 直接持久化補欄位（模擬 UI 編輯）。
    let body = serde_json::to_value(&node).unwrap();
    let _ = body; // 經 service 寫回
    rt_persist(&rt, &node).await;
    let approved = rt
        .knowledge_review(rid, "approve", None, MemoryActor::Human)
        .await
        .unwrap();
    assert_eq!(approved.status, KnowledgeStatus::Active);
    // receipt：human_reviewed=true、claims published。
    let receipts = rt.knowledge_receipts(20).await.unwrap();
    assert!(receipts["receipts"].as_array().unwrap().iter().any(|r| {
        r["verification"]["humanReviewed"] == true && r["published"]["claims"] == true
    }));
}

async fn rt_persist(rt: &Runtime, node: &KnowledgeNode) {
    // 測試用：經公開 API 補欄位（propose 會重設狀態，這裡直接用 review 前置寫回）。
    // 服務層沒有 raw update；以 propose+supersede 過重——改走內部持久化語意：
    // 這裡簡單透過再次 propose 同 id 不可行，故用 review comment 附帶不變更，
    // 最終以 store 直寫模擬 UI 的編輯端點。
    let body = serde_json::to_string(node).unwrap();
    rt.store
        .save_knowledge_node(
            node.node_id.as_str(),
            "claim",
            "candidate",
            &node.title,
            &node.content,
            &body,
        )
        .unwrap();
}

#[tokio::test]
async fn update_decisions_are_deterministic_and_receipts_flow() {
    let (_g, rt) = runtime().await;
    let d = rt.knowledge_update_decision(interaction_runtime::curator::UpdateTrigger::RepoCommit);
    assert_eq!(d["needsAi"], false);
    let d = rt.knowledge_update_decision(
        interaction_runtime::curator::UpdateTrigger::LowConfidenceAnswer,
    );
    assert_eq!(d["requiresUserAsk"], true, "外部研究必須先問");

    // agent 提案 → receipt 記 candidatesCreated 且 humanReviewed=false。
    rt.knowledge_propose_node(claim("候選"), MemoryActor::Agent("codex".into()))
        .await
        .unwrap();
    let receipts = rt.knowledge_receipts(10).await.unwrap();
    let r = &receipts["receipts"][0];
    assert_eq!(r["changes"]["candidatesCreated"], 1);
    assert_eq!(r["verification"]["humanReviewed"], false);
    assert_eq!(r["published"]["claims"], false);
}
