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
        .contains("local-subword-embedding-v1"));
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

#[tokio::test]
async fn empty_or_note_only_source_refs_cannot_pass_the_evidence_gate() {
    let (_g, rt) = runtime().await;
    // evidence: [{}]——空 SourceRef 不構成 provenance。
    let err = rt
        .knowledge_propose_node(
            claim("空證據主張", vec![SourceRef::default()]),
            MemoryActor::Agent("codex".into()),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    // 只有 note 也不算證據。
    let err = rt
        .knowledge_propose_node(
            claim(
                "只有 note 的主張",
                vec![SourceRef {
                    note: Some("trust me".into()),
                    ..Default::default()
                }],
            ),
            MemoryActor::Agent("codex".into()),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
}

#[tokio::test]
async fn asset_audit_never_asserts_unverified_human_agency() {
    let (_g, rt) = runtime().await;
    let asset = rt
        .asset_import(
            None,
            Some("audit actor 測試素材"),
            None,
            "user-import",
            None,
        )
        .await
        .unwrap();
    rt.asset_delete(&asset.hash).await.unwrap();
    // store 層也可由非 HTTP 內部呼叫者使用——audit 不得斷言 "human"。
    let tail = rt.store.audit_tail(20).unwrap();
    for kind in ["asset.imported", "asset.deleted"] {
        let entry = tail.iter().find(|e| e["kind"] == kind).expect(kind);
        assert_eq!(
            entry["actor"], "unattributed-api-caller",
            "{kind} 的 audit actor 必須誠實標示未驗證身分"
        );
    }
}

#[tokio::test]
async fn dangling_evidence_is_annotated_and_blocks_approval() {
    let (_g, rt) = runtime().await;
    let asset = rt
        .asset_import(
            None,
            Some("candidate 引用的素材"),
            None,
            "user-import",
            None,
        )
        .await
        .unwrap();
    let node = rt
        .knowledge_propose_node(
            claim(
                "引用素材的候選",
                vec![SourceRef {
                    asset_hash: Some(asset.hash.clone()),
                    ..Default::default()
                }],
            ),
            MemoryActor::Agent("codex".into()),
        )
        .await
        .unwrap();
    assert_eq!(node.status, KnowledgeStatus::Candidate);
    rt.asset_delete(&asset.hash).await.unwrap();
    // Candidate 不變 disputed，但要留下懸空證據的可見註記。
    let after = rt.knowledge_get(node.node_id.as_str()).await.unwrap();
    assert_eq!(after.status, KnowledgeStatus::Candidate);
    assert!(
        after.reviews.iter().any(|r| r.note.contains("已刪除")),
        "複審者要看得見證據已懸空"
    );
    // approve 前重驗證據：引用已刪素材的候選不得升格。
    let err = rt
        .knowledge_review(node.node_id.as_str(), "approve", None, MemoryActor::Human)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    let still = rt.knowledge_get(node.node_id.as_str()).await.unwrap();
    assert_ne!(
        still.status,
        KnowledgeStatus::Active,
        "懸空證據不可成為 Active"
    );
}

#[tokio::test]
async fn review_cannot_resurrect_superseded_or_archived_versions() {
    let (_g, rt) = runtime().await;
    // v1 active → v2 取代 → v1 superseded。
    let v1 = rt
        .knowledge_propose_node(claim("v1 主張", url_evidence()), MemoryActor::Human)
        .await
        .unwrap();
    let mut v2 = claim("v2 主張", url_evidence());
    v2.supersedes = Some(v1.node_id.clone());
    let v2 = rt
        .knowledge_propose_node(v2, MemoryActor::Agent("codex".into()))
        .await
        .unwrap();
    rt.knowledge_review(v2.node_id.as_str(), "approve", None, MemoryActor::Human)
        .await
        .unwrap();
    assert_eq!(
        rt.knowledge_get(v1.node_id.as_str()).await.unwrap().status,
        KnowledgeStatus::Superseded
    );
    // superseded 不得經 approve 復活（否則同一主張兩個 Active 版本並存）。
    let err = rt
        .knowledge_review(v1.node_id.as_str(), "approve", None, MemoryActor::Human)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    assert_eq!(
        rt.knowledge_get(v1.node_id.as_str()).await.unwrap().status,
        KnowledgeStatus::Superseded,
        "superseded 是版本化終態"
    );
    // reject 過的候選 → archived；archived 也不得復活。
    let c = rt
        .knowledge_propose_node(
            claim("會被退回的候選", url_evidence()),
            MemoryActor::Agent("codex".into()),
        )
        .await
        .unwrap();
    rt.knowledge_review(c.node_id.as_str(), "reject", None, MemoryActor::Human)
        .await
        .unwrap();
    let err = rt
        .knowledge_review(c.node_id.as_str(), "approve", None, MemoryActor::Human)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    assert_eq!(
        rt.knowledge_get(c.node_id.as_str()).await.unwrap().status,
        KnowledgeStatus::Archived
    );
    // active 知識不可直接 reject（退場走 supersede）。
    let err = rt
        .knowledge_review(v2.node_id.as_str(), "reject", None, MemoryActor::Human)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    // comment 任何狀態皆可（agent 留言不受狀態機限制）。
    let after = rt
        .knowledge_review(
            v1.node_id.as_str(),
            "comment",
            Some("補充脈絡".into()),
            MemoryActor::Agent("codex".into()),
        )
        .await
        .unwrap();
    assert_eq!(after.status, KnowledgeStatus::Superseded);
}

#[tokio::test]
async fn asset_delete_cascade_and_dispute_are_not_truncated_by_caps() {
    let (_g, rt) = runtime().await;
    let asset = rt
        .asset_import(None, Some("被大量引用的素材"), None, "user-import", None)
        .await
        .unwrap();
    // 依附素材的衍生記憶「最舊」——舊實作只掃最近 1000 筆會漏掉它。
    let mut derived = new_memory_item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Inference,
        "最舊的衍生摘要",
        "從素材得到的摘要",
        MemoryActor::Runtime,
        Utc::now(),
    );
    derived.retention.delete_with_parent = Some(asset.hash.clone());
    let derived = rt.memory_create(derived).await.unwrap();
    // 1010 筆較新的無關記憶（超過舊的 recency 窗）。直接寫 store 以維持測試速度。
    for i in 0..1010 {
        let filler = new_memory_item(
            MemoryLayer::SessionContext,
            MemoryKind::Fact,
            format!("filler {i}"),
            "x",
            MemoryActor::Runtime,
            Utc::now(),
        );
        rt.store
            .save_memory(
                filler.memory_id.as_str(),
                "session-context",
                "fact",
                None,
                None,
                &serde_json::to_string(&filler).unwrap(),
            )
            .unwrap();
    }
    // 205 個引用素材的 Active 節點（超過舊的 200 上限）。
    let mut ref_ids = Vec::new();
    for i in 0..205 {
        let node = claim(
            &format!("引用素材的主張 {i}"),
            vec![SourceRef {
                asset_hash: Some(asset.hash.clone()),
                ..Default::default()
            }],
        );
        rt.store
            .save_knowledge_node(
                node.node_id.as_str(),
                "claim",
                "active",
                &node.title,
                &node.content,
                &serde_json::to_string(&node).unwrap(),
            )
            .unwrap();
        ref_ids.push(node.node_id.as_str().to_string());
    }
    // 內文順帶提到 hash 但 evidence 沒引用的節點——不得被誤標。
    let mut prose = claim("只是提到 hash 的主張", url_evidence());
    prose.content = format!("內文提到 {} 而已", asset.hash);
    rt.store
        .save_knowledge_node(
            prose.node_id.as_str(),
            "claim",
            "active",
            &prose.title,
            &prose.content,
            &serde_json::to_string(&prose).unwrap(),
        )
        .unwrap();

    // 預覽與級聯一致且完整。
    let impact = rt.asset_delete_impact(&asset.hash).await.unwrap();
    assert_eq!(
        impact["referencingKnowledgeNodes"]
            .as_array()
            .unwrap()
            .len(),
        205,
        "每個引用節點都要進預覽"
    );
    assert_eq!(
        impact["memoriesDeletedWithParent"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    rt.asset_delete(&asset.hash).await.unwrap();
    for id in &ref_ids {
        assert_eq!(
            rt.knowledge_get(id).await.unwrap().status,
            KnowledgeStatus::Disputed,
            "引用節點 {id} 必須標 disputed，不得因上限漏標"
        );
    }
    assert_eq!(
        rt.knowledge_get(prose.node_id.as_str())
            .await
            .unwrap()
            .status,
        KnowledgeStatus::Active,
        "子字串提及不算引用"
    );
    let err = rt.memory_get(derived.memory_id.as_str()).await.unwrap_err();
    assert!(
        matches!(err, DomainError::NotFound(_)),
        "衍生記憶必須級聯刪除"
    );
    // 萬用字元不可膨脹影響預覽（hash 格式在邊界就擋掉）。
    let err = rt.asset_delete_impact("%").await.unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
}

#[tokio::test]
async fn local_subword_embedding_retrieves_cross_language_domain_concepts_as_candidates() {
    let (_g, rt) = runtime().await;
    let mut node = claim("使用授權必須由人類確認", url_evidence());
    node.content = "權限不可由 AI 自行擴張".into();
    node.domains = vec!["privacy-consent".into()];
    let node = rt
        .knowledge_propose_node(node, MemoryActor::Human)
        .await
        .unwrap();

    let result = rt
        .knowledge_search("permission approval", 10)
        .await
        .unwrap();
    assert!(result["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate| candidate["nodeId"] == node.node_id.as_str()));
    assert!(result["retrievalNote"]
        .as_str()
        .unwrap()
        .contains("local-subword-embedding-v1"));
    assert!(result["retrievalNote"]
        .as_str()
        .unwrap()
        .contains("只產生候選"));
}

#[tokio::test]
async fn image_and_audio_derivatives_are_content_addressed_and_precisely_linked_to_sources() {
    let (home, rt) = runtime().await;
    // Two-pixel PPM: a real decodable raster without an opaque test fixture.
    let image_path = home.path().join("sample.ppm");
    std::fs::write(&image_path, b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00").unwrap();
    let image = rt
        .asset_import(
            Some(image_path.to_str().unwrap()),
            None,
            Some(MediaType::Image),
            "user-import",
            None,
        )
        .await
        .unwrap();
    let report = rt.asset_derive(&image.hash).await.unwrap();
    let thumbnail = report
        .derivatives
        .iter()
        .find(|item| item.kind == AssetDerivativeKind::Thumbnail)
        .expect("real thumbnail derivative");
    assert_eq!(thumbnail.status, AssetDerivativeStatus::Complete);
    assert_eq!(thumbnail.source.segment.as_deref(), Some("region=0,0,2,1"));
    let output_hash = thumbnail.output_hash.as_deref().unwrap();
    assert_ne!(output_hash, image.hash);
    assert!(!rt
        .asset_content(output_hash, 1024 * 1024)
        .await
        .unwrap()
        .is_empty());

    // Minimal mono PCM WAV with two samples. Feature extraction is local and
    // deterministic; it never records/transmits microphone input.
    let wav_path = home.path().join("sample.wav");
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&40u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8_000u32.to_le_bytes());
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&4u32.to_le_bytes());
    wav.extend_from_slice(&0i16.to_le_bytes());
    wav.extend_from_slice(&1000i16.to_le_bytes());
    std::fs::write(&wav_path, wav).unwrap();
    let audio = rt
        .asset_import(
            Some(wav_path.to_str().unwrap()),
            None,
            Some(MediaType::Audio),
            "user-import",
            None,
        )
        .await
        .unwrap();
    let report = rt.asset_derive(&audio.hash).await.unwrap();
    let features = report
        .derivatives
        .iter()
        .find(|item| item.kind == AssetDerivativeKind::AudioFeatures)
        .expect("audio features derivative");
    assert_eq!(features.status, AssetDerivativeStatus::Complete);
    assert!(features
        .source
        .segment
        .as_deref()
        .unwrap()
        .starts_with("t=0-"));
    assert!(report
        .derivatives
        .iter()
        .any(|item| item.kind == AssetDerivativeKind::Transcript));

    let listed = rt.asset_derivatives(&audio.hash).await.unwrap();
    assert_eq!(listed.len(), report.derivatives.len());
}

#[tokio::test]
async fn deleting_source_previews_and_removes_derived_assets_without_silent_orphans() {
    let (home, rt) = runtime().await;
    let image_path = home.path().join("cascade.ppm");
    std::fs::write(&image_path, b"P6\n1 1\n255\n\xff\x00\x00").unwrap();
    let image = rt
        .asset_import(
            Some(image_path.to_str().unwrap()),
            None,
            Some(MediaType::Image),
            "user-import",
            None,
        )
        .await
        .unwrap();
    let report = rt.asset_derive(&image.hash).await.unwrap();
    let output_hash = report
        .derivatives
        .iter()
        .find_map(|item| item.output_hash.clone())
        .expect("thumbnail output");

    let impact = rt.asset_delete_impact(&image.hash).await.unwrap();
    assert!(impact["derivedAssetsRemoved"]
        .as_array()
        .unwrap()
        .iter()
        .any(|hash| hash == &output_hash));
    assert!(impact["derivativesRemoved"].as_u64().unwrap() >= 1);

    rt.asset_delete(&image.hash).await.unwrap();
    assert!(matches!(
        rt.asset_get(&output_hash).await.unwrap_err(),
        DomainError::NotFound(_)
    ));
    assert!(rt.asset_derivatives(&image.hash).await.is_err());
}
