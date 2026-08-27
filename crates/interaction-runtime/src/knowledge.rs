//! 知識服務（spec §11／§12）：CAS 素材、知識圖譜、FTS＋向量候選檢索、
//! Candidate 工作流。
//!
//! - 素材 blob：`<home>/state/assets/<hash[0..2]>/<hash>`，write-once。
//! - AI（agent actor）只能 propose（一律 Candidate）；activate 只屬於人類。
//! - 檢索：FTS5（bm25）＋可替換向量介面（v1 為誠實標示的詞彙備援索引，
//!   不是語意 embedding）；兩者都只產生**候選**，不是事實判斷。
//! - 刪素材前有影響預覽；引用它的 Active 知識不靜默級聯——標 disputed。

use crate::runtime::Runtime;
use chrono::Utc;
use interaction_core::{
    apply_knowledge_actor_rules, validate_edge, validate_node, AssetRecord, DomainError,
    DomainResult, KnowledgeEdge, KnowledgeEdgeId, KnowledgeNode, KnowledgeNodeId, KnowledgeReview,
    KnowledgeStatus, MediaType, MemoryActor, NodeType, RelationType, SourceRef, SCHEMA_VERSION,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024; // 64MB 本機素材上限
const MAX_INLINE_CONTENT: usize = 1024 * 1024;

/// 素材 hash 必須是 SHA-256 小寫 hex（64 位）。在 runtime 邊界擋掉
/// 萬用字元／畸形輸入——查詢層不得被 `%`／`_` 之類字串影響。
fn validate_asset_hash(hash: &str) -> DomainResult<()> {
    if hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(DomainError::Validation(
            "asset hash 必須是 64 位小寫 hex（SHA-256）".into(),
        ))
    }
}

/// 可替換向量索引介面（spec §11：Embedding 只負責找候選）。
pub trait VectorIndex: Send + Sync {
    fn upsert(&self, id: &str, text: &str);
    fn remove(&self, id: &str);
    /// 回傳 (id, 相似度 0..1)。
    fn query(&self, text: &str, k: usize) -> Vec<(String, f64)>;
    /// 誠實標示這個索引的性質。
    fn nature(&self) -> &'static str;
}

/// v1 備援：詞彙雜湊袋餘弦（**不是**語意 embedding；誠實標示）。
#[derive(Default)]
pub struct LexicalIndex {
    vectors:
        std::sync::Mutex<std::collections::HashMap<String, std::collections::HashMap<u32, f32>>>,
}

fn lex_vector(text: &str) -> std::collections::HashMap<u32, f32> {
    let mut v: std::collections::HashMap<u32, f32> = std::collections::HashMap::new();
    for token in text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        let mut h = Sha256::new();
        h.update(token.as_bytes());
        let d = h.finalize();
        let bucket = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) % 4096;
        *v.entry(bucket).or_insert(0.0) += 1.0;
    }
    // 中文等無空白語言：以雙字元 n-gram 補充。
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    for w in chars.windows(2) {
        let tok: String = w.iter().collect();
        let mut h = Sha256::new();
        h.update(tok.as_bytes());
        let d = h.finalize();
        let bucket = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) % 4096;
        *v.entry(bucket).or_insert(0.0) += 0.5;
    }
    v
}

fn cosine(a: &std::collections::HashMap<u32, f32>, b: &std::collections::HashMap<u32, f32>) -> f64 {
    let dot: f32 = a
        .iter()
        .filter_map(|(k, va)| b.get(k).map(|vb| va * vb))
        .sum();
    let na: f32 = a.values().map(|v| v * v).sum::<f32>().sqrt();
    let nb: f32 = b.values().map(|v| v * v).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        (dot / (na * nb)) as f64
    }
}

impl VectorIndex for LexicalIndex {
    fn upsert(&self, id: &str, text: &str) {
        self.vectors
            .lock()
            .expect("lex lock")
            .insert(id.to_string(), lex_vector(text));
    }
    fn remove(&self, id: &str) {
        self.vectors.lock().expect("lex lock").remove(id);
    }
    fn query(&self, text: &str, k: usize) -> Vec<(String, f64)> {
        let q = lex_vector(text);
        let map = self.vectors.lock().expect("lex lock");
        let mut scored: Vec<(String, f64)> = map
            .iter()
            .map(|(id, v)| (id.clone(), cosine(&q, v)))
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }
    fn nature(&self) -> &'static str {
        "lexical-fallback（詞彙雜湊袋餘弦；非語意 embedding）"
    }
}

impl Runtime {
    fn assets_dir(&self) -> PathBuf {
        self.paths.home.join("state").join("assets")
    }

    fn asset_blob_path(&self, hash: &str) -> PathBuf {
        self.assets_dir().join(&hash[..2]).join(hash)
    }

    /// 匯入素材：本機路徑或行內文字。write-once（同 hash 冪等）。
    pub async fn asset_import(
        &self,
        path: Option<&str>,
        inline_text: Option<&str>,
        media_type: Option<MediaType>,
        source: &str,
        description: Option<String>,
    ) -> DomainResult<AssetRecord> {
        let (bytes, name): (Vec<u8>, Option<String>) = match (path, inline_text) {
            (Some(p), _) => {
                let pb = PathBuf::from(p);
                let meta = std::fs::metadata(&pb)
                    .map_err(|e| DomainError::Validation(format!("讀不到 {p}：{e}")))?;
                if meta.len() > MAX_ASSET_BYTES {
                    return Err(DomainError::Validation(format!(
                        "素材超過上限 {MAX_ASSET_BYTES} bytes"
                    )));
                }
                (
                    std::fs::read(&pb).map_err(|e| DomainError::Validation(e.to_string()))?,
                    pb.file_name().map(|n| n.to_string_lossy().to_string()),
                )
            }
            (None, Some(t)) => {
                if t.len() > MAX_INLINE_CONTENT {
                    return Err(DomainError::Validation("inline 內容過大".into()));
                }
                (t.as_bytes().to_vec(), None)
            }
            (None, None) => {
                return Err(DomainError::Validation("需要 path 或 content".into()));
            }
        };
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = format!("{:x}", hasher.finalize());
        let media_type = media_type.unwrap_or_else(|| guess_media_type(name.as_deref()));

        // blob write-once：已存在就不重寫（內容定址 ⇒ 相同 hash 相同內容）。
        let blob = self.asset_blob_path(&hash);
        if !blob.exists() {
            if let Some(parent) = blob.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| DomainError::Internal(e.to_string()))?;
            }
            std::fs::write(&blob, &bytes).map_err(|e| DomainError::Internal(e.to_string()))?;
        }
        let record = AssetRecord {
            hash: hash.clone(),
            media_type,
            size_bytes: bytes.len() as u64,
            original_name: name,
            source: source.to_string(),
            added_at: Utc::now(),
            description,
            schema_version: SCHEMA_VERSION.into(),
        };
        let body =
            serde_json::to_string(&record).map_err(|e| DomainError::Internal(e.to_string()))?;
        let inserted = self.store.insert_asset(
            &hash,
            &serde_json::to_value(media_type)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            record.size_bytes,
            &body,
        )?;
        if inserted {
            // 誠實階梯：flat token 無法辨識呼叫者是人或 AI host——audit
            // 不得斷言 "human"。真實 actor 需 API 層帶入（已知限制）。
            self.store.audit(
                "asset.imported",
                "unattributed-api-caller",
                &json!({"hash": hash, "source": source}),
            )?;
        }
        // 已存在 → 回存的紀錄（write-once：不覆寫）。
        if !inserted {
            if let Some(existing) = self.store.get_asset(&hash)? {
                return serde_json::from_str(&existing)
                    .map_err(|e| DomainError::Internal(e.to_string()));
            }
        }
        Ok(record)
    }

    pub async fn asset_get(&self, hash: &str) -> DomainResult<AssetRecord> {
        validate_asset_hash(hash)?;
        let body = self
            .store
            .get_asset(hash)?
            .ok_or_else(|| DomainError::NotFound(format!("asset {hash}")))?;
        serde_json::from_str(&body).map_err(|e| DomainError::Internal(e.to_string()))
    }

    pub async fn asset_list(&self, limit: u32) -> DomainResult<Value> {
        let bodies = self.store.list_assets(limit)?;
        let items: Vec<Value> = bodies
            .iter()
            .filter_map(|b| serde_json::from_str(b).ok())
            .collect();
        Ok(json!({"assets": items, "count": items.len()}))
    }

    /// 刪除影響預覽：哪些知識節點引用它、哪些記憶隨父刪除。
    /// 兩個查詢都是全量精確比對——預覽必須跟實際級聯一致，
    /// recency 窗或列數上限會讓預覽低報、級聯漏刪。
    pub async fn asset_delete_impact(&self, hash: &str) -> DomainResult<Value> {
        validate_asset_hash(hash)?;
        let nodes = self.store.nodes_referencing_asset(hash)?;
        let dependent_memories = self.store.list_memory_ids_by_delete_parent(hash)?;
        Ok(json!({
            "hash": hash,
            "referencingKnowledgeNodes": nodes,
            "memoriesDeletedWithParent": dependent_memories,
            "note": "引用中的 Active 知識不會被靜默刪除——會標記 disputed（失去來源），需人工處理。",
        }))
    }

    /// 刪除素材（設計上屬人類動作；flat token 尚無法強制辨識呼叫者，
    /// 見已知限制）：級聯刪 delete_with_parent 記憶；引用它的 Active
    /// 知識標 disputed（不靜默消失）。
    pub async fn asset_delete(&self, hash: &str) -> DomainResult<Value> {
        let impact = self.asset_delete_impact(hash).await?;
        let deleted = self.store.delete_asset(hash)?;
        if !deleted {
            return Err(DomainError::NotFound(format!("asset {hash}")));
        }
        let blob = self.asset_blob_path(hash);
        let _ = std::fs::remove_file(blob);
        // 級聯：隨父刪除的衍生記憶。
        if let Some(ids) = impact["memoriesDeletedWithParent"].as_array() {
            for id in ids {
                if let Some(id) = id.as_str() {
                    let _ = self.store.delete_memory(id)?;
                }
            }
        }
        // 失去來源的知識：Active → disputed；Candidate/Stale 不改狀態，
        // 但留下可見註記讓複審者看見證據已懸空（approve 時另有硬性
        // 重驗擋下，見 knowledge_review）。
        if let Some(ids) = impact["referencingKnowledgeNodes"].as_array() {
            for id in ids {
                if let Some(id) = id.as_str() {
                    if let Ok(mut node) = self.knowledge_get(id).await {
                        match node.status {
                            KnowledgeStatus::Active => {
                                node.status = KnowledgeStatus::Disputed;
                            }
                            KnowledgeStatus::Candidate | KnowledgeStatus::Stale => {}
                            _ => continue,
                        }
                        node.reviews.push(KnowledgeReview {
                            reviewer: MemoryActor::Runtime,
                            verdict: "comment".into(),
                            note: format!("來源素材 {hash} 已刪除，知識失去支持"),
                            at: Utc::now(),
                        });
                        node.updated_at = Utc::now();
                        let _ = self.persist_knowledge_node(&node);
                    }
                }
            }
        }
        // 誠實階梯：呼叫者身分無法驗證，audit 不得斷言 "human"。
        self.store.audit(
            "asset.deleted",
            "unattributed-api-caller",
            &json!({"hash": hash, "impact": impact}),
        )?;
        Ok(json!({"deleted": true, "impact": impact}))
    }

    /// 讀 blob（上限保護；文字類回傳 UTF-8）。
    pub async fn asset_content(&self, hash: &str, max_bytes: usize) -> DomainResult<Vec<u8>> {
        let _ = self.asset_get(hash).await?; // 必須有中繼資料
        let blob = self.asset_blob_path(hash);
        let bytes = std::fs::read(&blob).map_err(|e| DomainError::NotFound(e.to_string()))?;
        if bytes.len() > max_bytes {
            return Err(DomainError::Validation(format!(
                "素材 {} bytes 超過此端點上限 {max_bytes}",
                bytes.len()
            )));
        }
        Ok(bytes)
    }

    // -------------------------------------------------------------------
    // 知識圖譜。
    // -------------------------------------------------------------------

    /// 建立節點（actor 由 API 層決定；agent 一律 Candidate）。
    pub async fn knowledge_propose_node(
        &self,
        mut node: KnowledgeNode,
        actor: MemoryActor,
    ) -> DomainResult<KnowledgeNode> {
        node.created_by = actor.clone();
        apply_knowledge_actor_rules(&mut node.status, &actor);
        let now = Utc::now();
        node.created_at = now;
        node.updated_at = now;
        validate_node(&node).map_err(DomainError::Validation)?;
        // 證據中的素材 hash 必須存在（衍生內容必須指回真素材）。
        for e in &node.evidence {
            if let Some(h) = &e.asset_hash {
                if self.store.get_asset(h)?.is_none() {
                    return Err(DomainError::Validation(format!(
                        "evidence 指向不存在的素材 {h}"
                    )));
                }
            }
        }
        self.persist_knowledge_node(&node)?;
        let is_candidate = node.status == KnowledgeStatus::Candidate;
        self.emit_knowledge_receipt(crate::curator::KnowledgeReceipt {
            update_id: format!("kr-{}", uuid::Uuid::new_v4()),
            triggered_by: match &node.created_by {
                MemoryActor::Agent(a) => format!("agent:{a}"),
                MemoryActor::Human => "human".into(),
                MemoryActor::Runtime => "runtime".into(),
            },
            agent_sessions: vec![],
            sources: node.evidence.iter().filter_map(|e| e.url.clone()).collect(),
            source_hashes: node
                .evidence
                .iter()
                .filter_map(|e| e.asset_hash.clone())
                .collect(),
            changes: crate::curator::ReceiptChanges {
                added_claims: if is_candidate { 0 } else { 1 },
                candidates_created: if is_candidate { 1 } else { 0 },
                ..Default::default()
            },
            verification: crate::curator::ReceiptVerification {
                schema_passed: true,
                source_hashes_verified: node
                    .evidence
                    .iter()
                    .all(|e| e.asset_hash.is_some() || e.url.is_some())
                    && !node.evidence.is_empty(),
                conflict_check: "unknown".into(),
                human_reviewed: !is_candidate,
            },
            published: crate::curator::ReceiptPublished {
                metadata: true,
                claims: !is_candidate,
            },
            created_at: Utc::now(),
            schema_version: SCHEMA_VERSION.into(),
        });
        Ok(node)
    }

    /// 建立邊（同 actor 規則＋因果驗證）。
    pub async fn knowledge_propose_edge(
        &self,
        mut edge: KnowledgeEdge,
        actor: MemoryActor,
    ) -> DomainResult<KnowledgeEdge> {
        edge.created_by = actor.clone();
        apply_knowledge_actor_rules(&mut edge.status, &actor);
        edge.created_at = Utc::now();
        validate_edge(&edge).map_err(DomainError::Validation)?;
        for id in [&edge.from, &edge.to] {
            if self.store.get_knowledge_node(id.as_str())?.is_none() {
                return Err(DomainError::NotFound(format!("節點 {id} 不存在")));
            }
        }
        self.store.save_knowledge_edge(
            edge.edge_id.as_str(),
            edge.from.as_str(),
            edge.to.as_str(),
            &serde_json::to_value(edge.relation)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            &status_str(edge.status),
            &serde_json::to_string(&edge).map_err(|e| DomainError::Internal(e.to_string()))?,
        )?;
        Ok(edge)
    }

    pub async fn knowledge_get(&self, id: &str) -> DomainResult<KnowledgeNode> {
        let body = self
            .store
            .get_knowledge_node(id)?
            .ok_or_else(|| DomainError::NotFound(format!("knowledge node {id}")))?;
        serde_json::from_str(&body).map_err(|e| DomainError::Internal(e.to_string()))
    }

    pub async fn knowledge_list(&self, status: Option<&str>, limit: u32) -> DomainResult<Value> {
        let bodies = self.store.list_knowledge_nodes(status, limit)?;
        let items: Vec<Value> = bodies
            .iter()
            .filter_map(|b| serde_json::from_str(b).ok())
            .collect();
        Ok(json!({"nodes": items, "count": items.len()}))
    }

    /// 複審（spec §12）：agent 只能留言；approve/reject 生效需人類。
    /// approve → active；reject → archived；supersede 由 approve 帶 supersedes 處理。
    pub async fn knowledge_review(
        &self,
        id: &str,
        verdict: &str,
        note: Option<String>,
        actor: MemoryActor,
    ) -> DomainResult<KnowledgeNode> {
        let mut node = self.knowledge_get(id).await?;
        let is_human = matches!(actor, MemoryActor::Human);
        let effective_verdict = match verdict {
            "approve" | "reject" if !is_human => {
                // agent 的裁決降為留言——絕不能自我核可。
                "comment"
            }
            v => v,
        };
        // 狀態機閘門（spec §15）：approve/reject 只對未定案節點有效；
        // superseded/archived 是版本化終態，不得經 review 復活。
        // agent 的裁決已降為 comment，不受此限。
        interaction_core::validate_review_transition(node.status, effective_verdict)
            .map_err(DomainError::Validation)?;
        node.reviews.push(KnowledgeReview {
            reviewer: actor,
            verdict: effective_verdict.to_string(),
            note: note.unwrap_or_default(),
            at: Utc::now(),
        });
        match effective_verdict {
            "approve" => {
                // 升格閘門（spec §14/§18）：經驗類 know-how 候選必須先補
                // 反例與適用範圍（證據在 propose 已強制）——結構性防止
                // 單次偶發被普遍化。
                if node.domains.iter().any(|d| d == "learning-from-feedback")
                    && (node.counterexamples.is_empty() || node.applicability.is_none())
                {
                    return Err(DomainError::Validation(
                        "經驗候選升格需要 counterexamples 與 applicability（反例與適用範圍必填）"
                            .into(),
                    ));
                }
                // 升格前重新驗證證據來源仍存在：candidate 期間素材可能
                // 已被刪除——引用懸空 hash 的節點不得成為 Active。
                for e in &node.evidence {
                    if let Some(h) = &e.asset_hash {
                        if self.store.get_asset(h)?.is_none() {
                            return Err(DomainError::Validation(format!(
                                "evidence 指向已刪除的素材 {h}，不可升格；請先更新證據或提出取代版本"
                            )));
                        }
                    }
                }
                node.status = KnowledgeStatus::Active;
                // 若此節點取代舊版：舊版 → superseded（版本化封存）。
                if let Some(old_id) = node.supersedes.clone() {
                    if let Ok(mut old) = self.knowledge_get(old_id.as_str()).await {
                        old.status = KnowledgeStatus::Superseded;
                        old.updated_at = Utc::now();
                        let _ = self.persist_knowledge_node(&old);
                    }
                }
            }
            "reject" => {
                node.status = KnowledgeStatus::Archived;
            }
            _ => {}
        }
        node.updated_at = Utc::now();
        self.persist_knowledge_node(&node)?;
        // 發布 receipt＋（approve 時）確定性衝突檢查。
        if effective_verdict == "approve" || effective_verdict == "reject" {
            let approved = effective_verdict == "approve";
            let conflict = if approved {
                let out = self.knowledge_conflict_check(node.node_id.as_str()).await?;
                if out["disputedWith"]
                    .as_array()
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
                {
                    "conflicts-found"
                } else {
                    "passed"
                }
            } else {
                "unknown"
            };
            // 衝突檢查可能把節點改成 disputed——回讀最新狀態。
            node = self.knowledge_get(node.node_id.as_str()).await?;
            self.emit_knowledge_receipt(crate::curator::KnowledgeReceipt {
                update_id: format!("kr-{}", uuid::Uuid::new_v4()),
                triggered_by: "human-review".into(),
                agent_sessions: vec![],
                sources: vec![],
                source_hashes: vec![],
                changes: crate::curator::ReceiptChanges {
                    added_claims: if approved { 1 } else { 0 },
                    superseded_claims: if approved && node.supersedes.is_some() {
                        1
                    } else {
                        0
                    },
                    ..Default::default()
                },
                verification: crate::curator::ReceiptVerification {
                    schema_passed: true,
                    source_hashes_verified: false,
                    conflict_check: conflict.into(),
                    human_reviewed: true,
                },
                published: crate::curator::ReceiptPublished {
                    metadata: true,
                    claims: approved && node.status == KnowledgeStatus::Active,
                },
                created_at: Utc::now(),
                schema_version: SCHEMA_VERSION.into(),
            });
        }
        Ok(node)
    }

    /// 檢索：FTS（bm25）＋向量候選（誠實標示 lexical-fallback）。
    /// 兩者都只是候選——不是事實判斷。
    pub async fn knowledge_search(&self, query: &str, k: u32) -> DomainResult<Value> {
        let fts = self
            .store
            .search_knowledge(&fts_sanitize(query), k)
            .unwrap_or_default();
        let vector = self.vector_index.query(query, k as usize);
        let mut seen = std::collections::BTreeSet::new();
        let mut results = Vec::new();
        for (id, score) in fts
            .iter()
            .map(|(id, s)| (id.clone(), json!({"fts": s})))
            .chain(
                vector
                    .iter()
                    .map(|(id, s)| (id.clone(), json!({"vector": s}))),
            )
        {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Ok(node) = self.knowledge_get(&id).await {
                results.push(json!({
                    "nodeId": id,
                    "title": node.title,
                    "status": node.status,
                    "nodeType": node.node_type,
                    "confidence": node.confidence,
                    "usable": node.status.usable(),
                    "retrieval": score,
                }));
            }
        }
        Ok(json!({
            "query": query,
            "results": results,
            "retrievalNote": format!("FTS=bm25；vector={}；檢索只產生候選，不代表可信", self.vector_index.nature()),
        }))
    }

    /// 圖譜展開（進階詳情）：節點＋相鄰邊。
    pub async fn knowledge_graph(&self, root: &str, _depth: u32) -> DomainResult<Value> {
        let node = self.knowledge_get(root).await?;
        let edges = self.store.edges_touching(root, 200)?;
        let edges: Vec<Value> = edges
            .iter()
            .filter_map(|b| serde_json::from_str(b).ok())
            .collect();
        let mut neighbor_ids = std::collections::BTreeSet::new();
        for e in &edges {
            for key in ["from", "to"] {
                if let Some(id) = e.get(key).and_then(|v| v.as_str()) {
                    if id != root {
                        neighbor_ids.insert(id.to_string());
                    }
                }
            }
        }
        let mut neighbors = Vec::new();
        for id in neighbor_ids {
            if let Ok(n) = self.knowledge_get(&id).await {
                neighbors.push(json!({"nodeId": id, "title": n.title, "status": n.status}));
            }
        }
        Ok(json!({
            "root": serde_json::to_value(&node).unwrap_or_default(),
            "edges": edges,
            "neighbors": neighbors,
        }))
    }

    pub(crate) fn persist_knowledge_node(&self, node: &KnowledgeNode) -> DomainResult<()> {
        let body = serde_json::to_string(node).map_err(|e| DomainError::Internal(e.to_string()))?;
        self.store.save_knowledge_node(
            node.node_id.as_str(),
            &serde_json::to_value(node.node_type)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            &status_str(node.status),
            &node.title,
            &node.content,
            &body,
        )?;
        self.vector_index.upsert(
            node.node_id.as_str(),
            &format!("{} {}", node.title, node.content),
        );
        Ok(())
    }

    /// 啟動時重建向量索引（記憶體內）。keyset 分頁掃完全部節點——
    /// 單頁上限不得靜默截斷（超過 1000 節點的圖譜也要完整進候選索引）。
    pub(crate) fn rebuild_vector_index(&self) {
        const PAGE: u32 = 500;
        let mut after: Option<String> = None;
        loop {
            let Ok(page) = self
                .store
                .list_knowledge_nodes_page(None, after.as_deref(), PAGE)
            else {
                // 啟動路徑不硬失敗；索引本就標示為候選層，缺頁只影響召回。
                break;
            };
            let Some((last_id, _)) = page.last() else {
                break;
            };
            after = Some(last_id.clone());
            let full_page = page.len() as u32 == PAGE;
            for (_, body) in page {
                if let Ok(node) = serde_json::from_str::<KnowledgeNode>(&body) {
                    self.vector_index.upsert(
                        node.node_id.as_str(),
                        &format!("{} {}", node.title, node.content),
                    );
                }
            }
            if !full_page {
                break;
            }
        }
    }
}

fn status_str(status: KnowledgeStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

/// FTS5 查詢字串消毒：以雙引號包裹避免語法錯誤（畸形查詢不 panic）。
fn fts_sanitize(query: &str) -> String {
    format!("\"{}\"", query.replace('"', " "))
}

fn guess_media_type(name: Option<&str>) -> MediaType {
    let Some(name) = name else {
        return MediaType::Text;
    };
    let lower = name.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => MediaType::Image,
        "mp3" | "wav" | "flac" | "m4a" | "ogg" => MediaType::Audio,
        "mp4" | "mov" | "webm" | "mkv" => MediaType::Video,
        "rs" | "ts" | "tsx" | "js" | "py" | "go" | "c" | "cpp" | "java" => MediaType::Code,
        "csv" | "json" | "yaml" | "yml" | "parquet" => MediaType::Data,
        "pdf" => MediaType::Pdf,
        "txt" | "md" | "html" => MediaType::Text,
        _ => MediaType::Other,
    }
}

/// API 層輸入 → 節點。
pub fn node_from_input(input: &Value) -> Result<KnowledgeNode, String> {
    let now = Utc::now();
    let node_type: NodeType =
        serde_json::from_value(input.get("nodeType").cloned().unwrap_or(json!("claim")))
            .map_err(|e| format!("nodeType: {e}"))?;
    let evidence: Vec<SourceRef> = input
        .get("evidence")
        .cloned()
        .map(|v| serde_json::from_value(v).map_err(|e| format!("evidence: {e}")))
        .transpose()?
        .unwrap_or_default();
    Ok(KnowledgeNode {
        node_id: KnowledgeNodeId::generate(),
        node_type,
        title: input
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        content: input
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        status: KnowledgeStatus::Candidate,
        confidence: input
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5),
        created_by: MemoryActor::Human, // 由服務層覆寫
        evidence,
        domains: input
            .get("domains")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        counterexamples: input
            .get("counterexamples")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        applicability: input
            .get("applicability")
            .and_then(|v| v.as_str())
            .map(String::from),
        version: 1,
        supersedes: input
            .get("supersedes")
            .and_then(|v| v.as_str())
            .map(KnowledgeNodeId::new),
        review_after: None,
        reviews: vec![],
        created_at: now,
        updated_at: now,
        schema_version: SCHEMA_VERSION.into(),
    })
}

/// API 層輸入 → 邊。
pub fn edge_from_input(input: &Value) -> Result<KnowledgeEdge, String> {
    let relation: RelationType = serde_json::from_value(
        input
            .get("relation")
            .cloned()
            .unwrap_or(json!("similar-to")),
    )
    .map_err(|e| format!("relation: {e}"))?;
    let origin: interaction_core::EdgeOrigin = serde_json::from_value(
        input
            .get("origin")
            .cloned()
            .unwrap_or(json!("ai-conjecture")),
    )
    .map_err(|e| format!("origin: {e}"))?;
    Ok(KnowledgeEdge {
        edge_id: KnowledgeEdgeId::generate(),
        from: KnowledgeNodeId::new(input.get("from").and_then(|v| v.as_str()).unwrap_or("")),
        to: KnowledgeNodeId::new(input.get("to").and_then(|v| v.as_str()).unwrap_or("")),
        relation,
        origin,
        status: KnowledgeStatus::Candidate,
        confidence: input
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5),
        created_by: MemoryActor::Human,
        rationale: input
            .get("rationale")
            .and_then(|v| v.as_str())
            .map(String::from),
        created_at: Utc::now(),
        schema_version: SCHEMA_VERSION.into(),
    })
}
