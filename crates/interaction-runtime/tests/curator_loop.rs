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
async fn candidate_edges_never_demote_active_knowledge() {
    let (_g, rt) = runtime().await;
    // 人類建立的 Active 知識。
    let a = rt
        .knowledge_propose_node(claim("A 為真"), MemoryActor::Human)
        .await
        .unwrap();
    assert_eq!(a.status, KnowledgeStatus::Active);
    // agent 提案的候選節點＋未審核的 contradicts 邊（agent 一律 Candidate）。
    let b = rt
        .knowledge_propose_node(claim("A 為假"), MemoryActor::Agent("codex".into()))
        .await
        .unwrap();
    assert_eq!(b.status, KnowledgeStatus::Candidate);
    let edge = KnowledgeEdge {
        edge_id: KnowledgeEdgeId::generate(),
        from: b.node_id.clone(),
        to: a.node_id.clone(),
        relation: RelationType::Contradicts,
        origin: EdgeOrigin::AiConjecture,
        status: KnowledgeStatus::Active, // 服務層會依 actor 降為 Candidate
        confidence: 0.9,
        created_by: MemoryActor::Agent("codex".into()),
        rationale: None,
        created_at: Utc::now(),
        schema_version: SCHEMA_VERSION.into(),
    };
    let edge = rt
        .knowledge_propose_edge(edge, MemoryActor::Agent("codex".into()))
        .await
        .unwrap();
    assert_eq!(edge.status, KnowledgeStatus::Candidate);
    // 人類核可 B（approve 觸發衝突檢查）：未審核的 AI 推測邊
    // 不得把任何一方拉成 Disputed。
    rt.knowledge_review(b.node_id.as_str(), "approve", None, MemoryActor::Human)
        .await
        .unwrap();
    assert_eq!(
        rt.knowledge_get(a.node_id.as_str()).await.unwrap().status,
        KnowledgeStatus::Active,
        "人類核可的知識不因 AI 推測邊失去 usable"
    );
    assert_eq!(
        rt.knowledge_get(b.node_id.as_str()).await.unwrap().status,
        KnowledgeStatus::Active
    );
    // 衝突檢查誠實回報候選衝突供人裁決——但不改狀態。
    let out = rt
        .knowledge_conflict_check(a.node_id.as_str())
        .await
        .unwrap();
    assert!(out["disputedWith"].as_array().unwrap().is_empty());
    assert_eq!(
        out["candidateConflicts"].as_array().unwrap().len(),
        1,
        "candidate 邊要列給人看：{out}"
    );
    assert_eq!(
        rt.knowledge_get(a.node_id.as_str()).await.unwrap().status,
        KnowledgeStatus::Active
    );
}

#[tokio::test]
async fn freshness_sweep_scans_beyond_a_single_page() {
    let (_g, rt) = runtime().await;
    // 過期節點「最舊」寫入——舊實作只取最近 1000 筆會漏掉它。
    let mut overdue = claim("超出窗口的過期知識");
    overdue.review_after = Some(Utc::now() - chrono::Duration::days(1));
    rt.store
        .save_knowledge_node(
            overdue.node_id.as_str(),
            "claim",
            "active",
            &overdue.title,
            &overdue.content,
            &serde_json::to_string(&overdue).unwrap(),
        )
        .unwrap();
    // 確保 updated_at 嚴格早於後續 filler（毫秒精度）。
    std::thread::sleep(std::time::Duration::from_millis(10));
    for i in 0..1010 {
        let n = claim(&format!("新鮮知識 {i}"));
        rt.store
            .save_knowledge_node(
                n.node_id.as_str(),
                "claim",
                "active",
                &n.title,
                &n.content,
                &serde_json::to_string(&n).unwrap(),
            )
            .unwrap();
    }
    let marked = rt.knowledge_freshness_sweep().await;
    assert_eq!(marked, 1, "掃描必須涵蓋全部 Active 節點，不得截斷");
    assert_eq!(
        rt.knowledge_get(overdue.node_id.as_str())
            .await
            .unwrap()
            .status,
        KnowledgeStatus::Stale
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
            resume_provider_session_id: None,
            allow_write: false,
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

#[tokio::test]
async fn user_correction_is_deletable_memory_and_unpublished_candidate() {
    let (_g, rt) = runtime().await;
    let out = rt
        .record_user_correction(interaction_runtime::curator::UserCorrectionInput {
            original_assumption: Some("所有專案都使用同一套規則".into()),
            correction: "只在 adaptive-interaction 專案套用這項規則".into(),
            scope: Some("adaptive-interaction repository".into()),
        })
        .await
        .unwrap();

    assert_eq!(out["memory"]["layer"], "user-memory");
    assert_eq!(out["memory"]["kind"], "preference");
    assert!(out["memory"]["retention"]["reviewAfter"].is_string());
    assert_eq!(out["candidate"]["status"], "candidate");
    assert_eq!(out["candidate"]["createdBy"]["kind"], "human");
    assert_eq!(out["decision"]["needsAi"], true);
    assert_eq!(
        out["knowledgeReceipt"]["verification"]["humanReviewed"],
        false
    );
    assert_eq!(out["knowledgeReceipt"]["published"]["claims"], false);

    let memories = rt.memory_list(Some("user-memory"), 10).await.unwrap();
    assert_eq!(memories["count"], 1);
    let candidates = rt.knowledge_list(Some("candidate"), 10).await.unwrap();
    assert_eq!(candidates["count"], 1);
}
