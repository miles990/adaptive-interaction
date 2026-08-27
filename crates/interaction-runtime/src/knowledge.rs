//! 知識服務（spec §11／§12）：CAS 素材、知識圖譜、FTS＋向量候選檢索、
//! Candidate 工作流。
//!
//! - 素材 blob：`<home>/state/assets/<hash[0..2]>/<hash>`，write-once。
//! - AI（agent actor）只能 propose（一律 Candidate）；activate 只屬於人類。
//! - 檢索：FTS5（bm25）＋可替換向量介面（v1 為本機、非生成式的
//!   subword/concept feature embedding）；兩者都只產生**候選**，不是事實判斷。
//! - 刪素材前有影響預覽；引用它的 Active 知識不靜默級聯——標 disputed。

use crate::runtime::Runtime;
use base64::Engine;
use chrono::Utc;
use interaction_core::{
    apply_knowledge_actor_rules, validate_edge, validate_node, AssetDerivationReport,
    AssetDerivative, AssetDerivativeKind, AssetDerivativeStatus, AssetRecord, DomainError,
    DomainResult, KnowledgeEdge, KnowledgeEdgeId, KnowledgeNode, KnowledgeNodeId, KnowledgeReview,
    KnowledgeStatus, MediaType, MemoryActor, NodeType, RelationType, SourceRef, SCHEMA_VERSION,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

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

/// v1 本機 embedding：詞彙＋字元 subword＋小型跨語言領域概念特徵。
/// 它是確定性、離線、可替換的 sparse feature embedding；不宣稱是神經
/// 模型，也不把相似度升格成關係或因果。
#[derive(Default)]
pub struct LocalSubwordEmbeddingIndex {
    vectors:
        std::sync::Mutex<std::collections::HashMap<String, std::collections::HashMap<u32, f32>>>,
}

fn add_feature(v: &mut std::collections::HashMap<u32, f32>, feature: &str, weight: f32) {
    let mut h = Sha256::new();
    h.update(feature.as_bytes());
    let d = h.finalize();
    let bucket = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) % 8192;
    *v.entry(bucket).or_insert(0.0) += weight;
}

fn embedding_vector(text: &str) -> std::collections::HashMap<u32, f32> {
    let mut v: std::collections::HashMap<u32, f32> = std::collections::HashMap::new();
    let lower = text.to_lowercase();
    for token in lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        add_feature(&mut v, &format!("token:{token}"), 1.0);
    }
    // 中文等無空白語言與拼字變形：2/3 字元 subword。
    let chars: Vec<char> = lower.chars().filter(|c| !c.is_whitespace()).collect();
    for width in [2usize, 3] {
        for window in chars.windows(width) {
            add_feature(
                &mut v,
                &format!("subword:{}", window.iter().collect::<String>()),
                if width == 2 { 0.45 } else { 0.3 },
            );
        }
    }
    // 內建 Domain Pack 的跨語言概念 anchors。這是明示、版本化的
    // vocabulary transfer，不是研究支持的因果關係。
    const CONCEPTS: &[(&str, &[&str])] = &[
        (
            "consent",
            &[
                "consent",
                "permission",
                "approval",
                "authorization",
                "授權",
                "同意",
                "權限",
            ],
        ),
        (
            "verification",
            &[
                "verify",
                "verification",
                "validation",
                "test",
                "驗證",
                "確認",
                "測試",
            ],
        ),
        (
            "privacy",
            &[
                "privacy",
                "personal data",
                "sensitive",
                "隱私",
                "個資",
                "敏感",
            ],
        ),
        (
            "delegation",
            &["delegate", "delegation", "handoff", "委派", "轉交"],
        ),
        (
            "knowledge",
            &["knowledge", "know-how", "memory", "知識", "方法", "記憶"],
        ),
        (
            "failure",
            &["failure", "failed", "error", "錯誤", "失敗", "異常"],
        ),
    ];
    for (concept, aliases) in CONCEPTS {
        if aliases.iter().any(|alias| lower.contains(alias)) {
            add_feature(&mut v, &format!("concept:{concept}"), 3.0);
        }
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

impl VectorIndex for LocalSubwordEmbeddingIndex {
    fn upsert(&self, id: &str, text: &str) {
        self.vectors
            .lock()
            .expect("lex lock")
            .insert(id.to_string(), embedding_vector(text));
    }
    fn remove(&self, id: &str) {
        self.vectors.lock().expect("lex lock").remove(id);
    }
    fn query(&self, text: &str, k: usize) -> Vec<(String, f64)> {
        let q = embedding_vector(text);
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
        "local-subword-embedding-v1（離線 sparse feature embedding；非神經模型）"
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
            // 誠實階梯：store 層也可由非 HTTP 內部呼叫者使用，audit
            // 因此不斷言 "human"。HTTP 邊界另以 human/agent token 分權。
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
        let derivatives = self
            .store
            .list_asset_derivatives(hash)?
            .into_iter()
            .filter_map(|body| serde_json::from_str::<AssetDerivative>(&body).ok())
            .collect::<Vec<_>>();
        let mut derived_assets_removed = Vec::new();
        let mut derived_assets_retained_shared = Vec::new();
        for output_hash in derivatives
            .iter()
            .filter_map(|derivative| derivative.output_hash.as_deref())
        {
            if self
                .store
                .count_asset_derivative_output_references(output_hash)?
                <= 1
            {
                derived_assets_removed.push(output_hash.to_string());
            } else {
                derived_assets_retained_shared.push(output_hash.to_string());
            }
        }
        derived_assets_removed.sort();
        derived_assets_removed.dedup();
        derived_assets_retained_shared.sort();
        derived_assets_retained_shared.dedup();

        let mut affected_hashes = vec![hash.to_string()];
        affected_hashes.extend(derived_assets_removed.iter().cloned());
        let mut nodes = Vec::new();
        let mut dependent_memories = Vec::new();
        for affected in &affected_hashes {
            nodes.extend(self.store.nodes_referencing_asset(affected)?);
            dependent_memories.extend(self.store.list_memory_ids_by_delete_parent(affected)?);
        }
        nodes.sort();
        nodes.dedup();
        dependent_memories.sort();
        dependent_memories.dedup();
        Ok(json!({
            "hash": hash,
            "referencingKnowledgeNodes": nodes,
            "memoriesDeletedWithParent": dependent_memories,
            "derivativesRemoved": derivatives.len(),
            "derivedAssetsRemoved": derived_assets_removed,
            "derivedAssetsRetainedShared": derived_assets_retained_shared,
            "note": "引用中的 Active 知識不會被靜默刪除——會標記 disputed（失去來源），需人工處理。",
        }))
    }

    /// 刪除素材（設計上屬人類動作；HTTP 邊界拒絕 agent token，
    /// 內部呼叫者仍須只在人類控制面使用）：級聯刪 delete_with_parent 記憶；引用它的 Active
    /// 知識標 disputed（不靜默消失）。
    pub async fn asset_delete(&self, hash: &str) -> DomainResult<Value> {
        let impact = self.asset_delete_impact(hash).await?;
        let deleted = self.store.delete_asset(hash)?;
        if !deleted {
            return Err(DomainError::NotFound(format!("asset {hash}")));
        }
        let blob = self.asset_blob_path(hash);
        let _ = std::fs::remove_file(blob);
        // 衍生列跟隨父素材刪除；只有沒有被其他父素材引用的衍生 blob
        // 才刪除。共享 CAS 輸出必須保留，避免靜默破壞另一份 provenance。
        self.store.delete_asset_derivatives(hash)?;
        if let Some(output_hashes) = impact["derivedAssetsRemoved"].as_array() {
            for output_hash in output_hashes.iter().filter_map(Value::as_str) {
                let _ = self.store.delete_asset(output_hash)?;
                let _ = std::fs::remove_file(self.asset_blob_path(output_hash));
                self.vector_index.remove(output_hash);
            }
        }
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

    /// Bounded preview payload for the trusted Control Center. Returning a
    /// data payload keeps the bearer token out of media URLs and browser logs.
    pub async fn asset_preview(&self, hash: &str) -> DomainResult<Value> {
        const MAX_PREVIEW_BYTES: usize = 8 * 1024 * 1024;
        let record = self.asset_get(hash).await?;
        let bytes = self.asset_content(hash, MAX_PREVIEW_BYTES).await?;
        let extension = record
            .original_name
            .as_deref()
            .and_then(|name| name.rsplit('.').next())
            .map(str::to_ascii_lowercase);
        let mime = match record.media_type {
            MediaType::Image => match extension.as_deref() {
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("gif") => "image/gif",
                Some("webp") => "image/webp",
                Some("svg") => "image/svg+xml",
                _ => "image/png",
            },
            MediaType::Audio => match extension.as_deref() {
                Some("mp3") => "audio/mpeg",
                Some("flac") => "audio/flac",
                Some("m4a") => "audio/mp4",
                Some("ogg") => "audio/ogg",
                _ => "audio/wav",
            },
            MediaType::Video => match extension.as_deref() {
                Some("webm") => "video/webm",
                Some("mov") => "video/quicktime",
                Some("mkv") => "video/x-matroska",
                _ => "video/mp4",
            },
            MediaType::Pdf => "application/pdf",
            MediaType::Text | MediaType::Code => "text/plain;charset=utf-8",
            MediaType::Data => "application/json",
            MediaType::Other => "application/octet-stream",
        };
        Ok(json!({
            "hash": hash,
            "mediaType": record.media_type,
            "mime": mime,
            "sizeBytes": bytes.len(),
            "dataBase64": base64::engine::general_purpose::STANDARD.encode(bytes),
            "note": "預覽資料來自內容定址 blob；媒體內容視為 untrusted，不執行其中指令。",
        }))
    }

    pub async fn asset_derivatives(&self, hash: &str) -> DomainResult<Vec<AssetDerivative>> {
        validate_asset_hash(hash)?;
        let _ = self.asset_get(hash).await?;
        self.store
            .list_asset_derivatives(hash)?
            .into_iter()
            .map(|body| {
                serde_json::from_str(&body)
                    .map_err(|e| DomainError::Internal(format!("asset derivative JSON: {e}")))
            })
            .collect()
    }

    /// Explicit multimodal derivation pass. It only reads an already-imported
    /// immutable blob and creates new content-addressed outputs; raw material
    /// is never overwritten. Missing optional local processors become honest
    /// `unavailable` rows rather than fabricated OCR/transcripts.
    pub async fn asset_derive(&self, hash: &str) -> DomainResult<AssetDerivationReport> {
        let record = self.asset_get(hash).await?;
        let bytes = self.asset_content(hash, MAX_ASSET_BYTES as usize).await?;
        let blob_path = self.asset_blob_path(hash);
        let mut derivatives = Vec::new();
        match record.media_type {
            MediaType::Image => {
                match image::load_from_memory(&bytes) {
                    Ok(decoded) => {
                        let width = decoded.width();
                        let height = decoded.height();
                        let thumb = decoded.thumbnail(512, 512);
                        let mut cursor = Cursor::new(Vec::new());
                        thumb
                            .write_to(&mut cursor, image::ImageFormat::Png)
                            .map_err(|e| DomainError::Validation(format!("decode image: {e}")))?;
                        derivatives.push(self.persist_complete_derivative(
                            hash,
                            AssetDerivativeKind::Thumbnail,
                            &cursor.into_inner(),
                            MediaType::Image,
                            Some("thumbnail.png".into()),
                            format!("region=0,0,{width},{height}"),
                            "image-rs",
                            env!("CARGO_PKG_VERSION"),
                            format!("{}×{} PNG thumbnail", thumb.width(), thumb.height()),
                        )?);
                    }
                    Err(error) => derivatives.push(self.persist_derivative_status(
                        hash,
                        AssetDerivativeKind::Thumbnail,
                        AssetDerivativeStatus::Failed,
                        None,
                        "region=unknown".into(),
                        "image-rs",
                        env!("CARGO_PKG_VERSION"),
                        format!("image decoder rejected material: {error}"),
                    )?),
                }
                let path = blob_path.to_string_lossy().into_owned();
                match run_bounded_command(
                    "tesseract",
                    &[&path, "stdout", "--psm", "6"],
                    20,
                    4 * 1024 * 1024,
                )
                .await
                {
                    Ok(text) if !text.iter().all(|byte| byte.is_ascii_whitespace()) => {
                        derivatives.push(self.persist_complete_derivative(
                            hash,
                            AssetDerivativeKind::OcrText,
                            &text,
                            MediaType::Text,
                            Some("ocr.txt".into()),
                            "region=full".into(),
                            "tesseract",
                            "cli",
                            "本機 OCR；輸出視為 untrusted candidate，不自動發布".into(),
                        )?);
                    }
                    Ok(_) => derivatives.push(self.persist_derivative_status(
                        hash,
                        AssetDerivativeKind::OcrText,
                        AssetDerivativeStatus::Complete,
                        None,
                        "region=full".into(),
                        "tesseract",
                        "cli",
                        "OCR 完成但沒有辨識文字".into(),
                    )?),
                    Err(error) => derivatives.push(self.persist_derivative_status(
                        hash,
                        AssetDerivativeKind::OcrText,
                        AssetDerivativeStatus::Unavailable,
                        None,
                        "region=full".into(),
                        "tesseract",
                        "cli",
                        format!("本機 OCR 不可用；未產生文字：{error}"),
                    )?),
                }
            }
            MediaType::Audio => {
                match wav_features(&bytes) {
                    Ok((features, duration)) => {
                        let encoded = serde_json::to_vec_pretty(&features)
                            .map_err(|e| DomainError::Internal(e.to_string()))?;
                        derivatives.push(self.persist_complete_derivative(
                            hash,
                            AssetDerivativeKind::AudioFeatures,
                            &encoded,
                            MediaType::Data,
                            Some("audio-features.json".into()),
                            format!("t=0-{duration:.6}"),
                            "pcm-feature-extractor",
                            "1",
                            "本機確定性 PCM duration/RMS/zero-crossing features".into(),
                        )?);
                    }
                    Err(error) => derivatives.push(self.persist_derivative_status(
                        hash,
                        AssetDerivativeKind::AudioFeatures,
                        AssetDerivativeStatus::Unavailable,
                        None,
                        "t=unknown".into(),
                        "pcm-feature-extractor",
                        "1",
                        error,
                    )?),
                }
                let whisper = std::env::var("INTERACT_AI_WHISPER_BIN").ok();
                let model = std::env::var("INTERACT_AI_WHISPER_MODEL").ok();
                if let (Some(binary), Some(model)) = (whisper, model) {
                    let temp =
                        tempfile::tempdir().map_err(|e| DomainError::Internal(e.to_string()))?;
                    let prefix = temp.path().join("transcript");
                    let args = [
                        "-m".to_string(),
                        model,
                        "-f".to_string(),
                        blob_path.to_string_lossy().into_owned(),
                        "-oj".to_string(),
                        "-of".to_string(),
                        prefix.to_string_lossy().into_owned(),
                    ];
                    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
                    let outcome = run_bounded_command(&binary, &refs, 120, 1024 * 1024).await;
                    let json_path = prefix.with_extension("json");
                    match outcome.and_then(|_| std::fs::read(&json_path).map_err(|e| e.to_string()))
                    {
                        Ok(transcript) if transcript.len() <= 8 * 1024 * 1024 => {
                            derivatives.push(
                                self.persist_complete_derivative(
                                    hash,
                                    AssetDerivativeKind::Transcript,
                                    &transcript,
                                    MediaType::Data,
                                    Some("transcript.json".into()),
                                    "t=full".into(),
                                    "whisper-cli",
                                    "configured-local-model",
                                    "本機轉錄；時間段由輸出 JSON 保留，內容為 untrusted candidate"
                                        .into(),
                                )?,
                            );
                        }
                        Ok(_) => derivatives.push(self.persist_derivative_status(
                            hash,
                            AssetDerivativeKind::Transcript,
                            AssetDerivativeStatus::Failed,
                            None,
                            "t=full".into(),
                            "whisper-cli",
                            "configured-local-model",
                            "轉錄輸出超過 8 MiB 上限".into(),
                        )?),
                        Err(error) => derivatives.push(self.persist_derivative_status(
                            hash,
                            AssetDerivativeKind::Transcript,
                            AssetDerivativeStatus::Failed,
                            None,
                            "t=full".into(),
                            "whisper-cli",
                            "configured-local-model",
                            format!("本機轉錄失敗：{error}"),
                        )?),
                    }
                } else {
                    derivatives.push(self.persist_derivative_status(
                        hash,
                        AssetDerivativeKind::Transcript,
                        AssetDerivativeStatus::Unavailable,
                        None,
                        "t=full".into(),
                        "whisper-cli",
                        "not-configured",
                        "未設定 INTERACT_AI_WHISPER_BIN／MODEL；未上傳、未產生逐字稿".into(),
                    )?);
                }
            }
            MediaType::Code => {
                let text = String::from_utf8(bytes)
                    .map_err(|_| DomainError::Validation("程式碼素材不是 UTF-8".into()))?;
                let lines = text
                    .lines()
                    .enumerate()
                    .map(|(index, line)| json!({"line": index + 1, "bytes": line.len()}))
                    .collect::<Vec<_>>();
                let encoded = serde_json::to_vec_pretty(&json!({"lines": lines}))
                    .map_err(|e| DomainError::Internal(e.to_string()))?;
                derivatives.push(self.persist_complete_derivative(
                    hash,
                    AssetDerivativeKind::CodeIndex,
                    &encoded,
                    MediaType::Data,
                    Some("code-index.json".into()),
                    format!("lines=1-{}", text.lines().count().max(1)),
                    "utf8-line-index",
                    "1",
                    "UTF-8 line index".into(),
                )?);
            }
            MediaType::Video => {
                let path = blob_path.to_string_lossy().into_owned();
                match run_bounded_command(
                    "ffprobe",
                    &[
                        "-v",
                        "error",
                        "-show_format",
                        "-show_streams",
                        "-of",
                        "json",
                        &path,
                    ],
                    20,
                    4 * 1024 * 1024,
                )
                .await
                {
                    Ok(metadata) => derivatives.push(self.persist_complete_derivative(
                        hash,
                        AssetDerivativeKind::VideoMetadata,
                        &metadata,
                        MediaType::Data,
                        Some("video-metadata.json".into()),
                        "t=full".into(),
                        "ffprobe",
                        "cli",
                        "container/stream metadata only".into(),
                    )?),
                    Err(error) => derivatives.push(self.persist_derivative_status(
                        hash,
                        AssetDerivativeKind::VideoMetadata,
                        AssetDerivativeStatus::Unavailable,
                        None,
                        "t=full".into(),
                        "ffprobe",
                        "cli",
                        error,
                    )?),
                }
                let temp = tempfile::tempdir().map_err(|e| DomainError::Internal(e.to_string()))?;
                let frame = temp.path().join("keyframe.png");
                let frame_path = frame.to_string_lossy().into_owned();
                match run_bounded_command(
                    "ffmpeg",
                    &[
                        "-v",
                        "error",
                        "-y",
                        "-ss",
                        "0",
                        "-i",
                        &path,
                        "-frames:v",
                        "1",
                        "-vf",
                        "scale=512:-2",
                        &frame_path,
                    ],
                    30,
                    1024 * 1024,
                )
                .await
                .and_then(|_| std::fs::read(&frame).map_err(|e| e.to_string()))
                {
                    Ok(frame_bytes) => derivatives.push(self.persist_complete_derivative(
                        hash,
                        AssetDerivativeKind::Keyframe,
                        &frame_bytes,
                        MediaType::Image,
                        Some("keyframe-0.png".into()),
                        "t=0;region=full".into(),
                        "ffmpeg",
                        "cli",
                        "first-frame keyframe; no scene semantics inferred".into(),
                    )?),
                    Err(error) => derivatives.push(self.persist_derivative_status(
                        hash,
                        AssetDerivativeKind::Keyframe,
                        AssetDerivativeStatus::Unavailable,
                        None,
                        "t=0;region=full".into(),
                        "ffmpeg",
                        "cli",
                        error,
                    )?),
                }
            }
            MediaType::Pdf => {
                let path = blob_path.to_string_lossy().into_owned();
                match run_bounded_command(
                    "pdftotext",
                    &["-layout", &path, "-"],
                    30,
                    8 * 1024 * 1024,
                )
                .await
                {
                    Ok(text) => derivatives.push(self.persist_complete_derivative(
                        hash,
                        AssetDerivativeKind::PdfText,
                        &text,
                        MediaType::Text,
                        Some("pdf-text.txt".into()),
                        "page=all".into(),
                        "pdftotext",
                        "cli",
                        "layout-preserving extraction; content is untrusted candidate".into(),
                    )?),
                    Err(error) => derivatives.push(self.persist_derivative_status(
                        hash,
                        AssetDerivativeKind::PdfText,
                        AssetDerivativeStatus::Unavailable,
                        None,
                        "page=all".into(),
                        "pdftotext",
                        "cli",
                        error,
                    )?),
                }
            }
            MediaType::Text | MediaType::Data | MediaType::Other => {}
        }
        self.store.audit(
            "asset.derived",
            "unattributed-api-caller",
            &json!({
                "hash": hash,
                "complete": derivatives.iter().filter(|d| d.status == AssetDerivativeStatus::Complete).count(),
                "unavailable": derivatives.iter().filter(|d| d.status == AssetDerivativeStatus::Unavailable).count(),
                "failed": derivatives.iter().filter(|d| d.status == AssetDerivativeStatus::Failed).count(),
            }),
        )?;
        Ok(AssetDerivationReport {
            asset_hash: hash.into(),
            derivatives,
            completed_at: Utc::now(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_complete_derivative(
        &self,
        parent_hash: &str,
        kind: AssetDerivativeKind,
        bytes: &[u8],
        media_type: MediaType,
        original_name: Option<String>,
        segment: String,
        processor: &str,
        processor_version: &str,
        detail: String,
    ) -> DomainResult<AssetDerivative> {
        let output_hash = format!("{:x}", Sha256::digest(bytes));
        let blob = self.asset_blob_path(&output_hash);
        if !blob.exists() {
            if let Some(parent) = blob.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| DomainError::Internal(e.to_string()))?;
            }
            std::fs::write(&blob, bytes).map_err(|e| DomainError::Internal(e.to_string()))?;
        }
        let output = AssetRecord {
            hash: output_hash.clone(),
            media_type,
            size_bytes: bytes.len() as u64,
            original_name,
            source: format!("derived-from:{parent_hash}#{segment}"),
            added_at: Utc::now(),
            description: Some(format!("{kind:?} derivative; untrusted until reviewed")),
            schema_version: SCHEMA_VERSION.into(),
        };
        self.store.insert_asset(
            &output_hash,
            &serde_json::to_value(media_type)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            output.size_bytes,
            &serde_json::to_string(&output).map_err(|e| DomainError::Internal(e.to_string()))?,
        )?;
        self.persist_derivative_status(
            parent_hash,
            kind,
            AssetDerivativeStatus::Complete,
            Some(output_hash),
            segment,
            processor,
            processor_version,
            detail,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_derivative_status(
        &self,
        parent_hash: &str,
        kind: AssetDerivativeKind,
        status: AssetDerivativeStatus,
        output_hash: Option<String>,
        segment: String,
        processor: &str,
        processor_version: &str,
        detail: String,
    ) -> DomainResult<AssetDerivative> {
        let kind_text = serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "unknown".into());
        let status_text = serde_json::to_value(status)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "failed".into());
        let id_seed =
            format!("{parent_hash}\0{kind_text}\0{processor}\0{processor_version}\0{segment}");
        let derivative = AssetDerivative {
            derivative_id: format!("derivative-{:x}", Sha256::digest(id_seed.as_bytes())),
            parent_hash: parent_hash.into(),
            kind,
            status,
            output_hash,
            source: SourceRef {
                asset_hash: Some(parent_hash.into()),
                segment: Some(segment),
                ..Default::default()
            },
            processor: processor.into(),
            processor_version: processor_version.into(),
            detail,
            created_at: Utc::now(),
            schema_version: SCHEMA_VERSION.into(),
        };
        self.store.save_asset_derivative(
            &derivative.derivative_id,
            parent_hash,
            &kind_text,
            &status_text,
            &serde_json::to_string(&derivative)
                .map_err(|e| DomainError::Internal(e.to_string()))?,
        )?;
        Ok(derivative)
    }

    // -------------------------------------------------------------------
    // 知識圖譜。
    // -------------------------------------------------------------------

    /// 建立節點（actor 由 API 層決定；agent 一律 Candidate）。
    pub async fn knowledge_propose_node(
        &self,
        node: KnowledgeNode,
        actor: MemoryActor,
    ) -> DomainResult<KnowledgeNode> {
        self.knowledge_propose_node_with_session(node, actor, None)
            .await
    }

    pub async fn knowledge_propose_node_for_session(
        &self,
        node: KnowledgeNode,
        actor: MemoryActor,
        agent_session_id: String,
    ) -> DomainResult<KnowledgeNode> {
        self.knowledge_propose_node_with_session(node, actor, Some(agent_session_id))
            .await
    }

    async fn knowledge_propose_node_with_session(
        &self,
        mut node: KnowledgeNode,
        actor: MemoryActor,
        agent_session_id: Option<String>,
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
            agent_sessions: agent_session_id.into_iter().collect(),
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
        edge: KnowledgeEdge,
        actor: MemoryActor,
    ) -> DomainResult<KnowledgeEdge> {
        self.knowledge_propose_edge_with_session(edge, actor, None)
            .await
    }

    pub async fn knowledge_propose_edge_for_session(
        &self,
        edge: KnowledgeEdge,
        actor: MemoryActor,
        agent_session_id: String,
    ) -> DomainResult<KnowledgeEdge> {
        self.knowledge_propose_edge_with_session(edge, actor, Some(agent_session_id))
            .await
    }

    async fn knowledge_propose_edge_with_session(
        &self,
        mut edge: KnowledgeEdge,
        actor: MemoryActor,
        agent_session_id: Option<String>,
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
        self.emit_knowledge_receipt(crate::curator::KnowledgeReceipt {
            update_id: format!("kr-{}", uuid::Uuid::new_v4()),
            triggered_by: match &edge.created_by {
                MemoryActor::Agent(agent) => format!("agent:{agent}"),
                MemoryActor::Human => "human".into(),
                MemoryActor::Runtime => "runtime".into(),
            },
            agent_sessions: agent_session_id.into_iter().collect(),
            sources: vec![],
            source_hashes: vec![],
            changes: crate::curator::ReceiptChanges {
                updated_relations: 1,
                candidates_created: u32::from(edge.status == KnowledgeStatus::Candidate),
                ..Default::default()
            },
            verification: crate::curator::ReceiptVerification {
                schema_passed: true,
                source_hashes_verified: false,
                conflict_check: "unknown".into(),
                human_reviewed: !matches!(edge.created_by, MemoryActor::Agent(_)),
            },
            published: crate::curator::ReceiptPublished {
                metadata: true,
                claims: false,
            },
            created_at: Utc::now(),
            schema_version: SCHEMA_VERSION.into(),
        });
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

    /// 檢索：FTS（bm25）＋本機 subword/concept embedding 候選。
    /// 兩者都只是候選——不是事實判斷。
    pub async fn knowledge_search(&self, query: &str, k: u32) -> DomainResult<Value> {
        self.knowledge_search_in_domains(query, k, None).await
    }

    pub async fn knowledge_search_scoped(
        &self,
        query: &str,
        k: u32,
        domains: &std::collections::BTreeSet<String>,
    ) -> DomainResult<Value> {
        self.knowledge_search_in_domains(query, k, Some(domains))
            .await
    }

    async fn knowledge_search_in_domains(
        &self,
        query: &str,
        k: u32,
        domains: Option<&std::collections::BTreeSet<String>>,
    ) -> DomainResult<Value> {
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
                if domains.is_some_and(|allowed| {
                    !allowed.contains("*")
                        && (node.domains.is_empty()
                            || !node.domains.iter().any(|domain| allowed.contains(domain)))
                }) {
                    continue;
                }
                results.push(json!({
                    "nodeId": id,
                    "title": node.title,
                    "status": node.status,
                    "nodeType": node.node_type,
                    "confidence": node.confidence,
                    "domains": node.domains,
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

    pub async fn asset_accessible_in_domains(
        &self,
        hash: &str,
        domains: &std::collections::BTreeSet<String>,
    ) -> DomainResult<bool> {
        validate_asset_hash(hash)?;
        if domains.contains("*") {
            return Ok(true);
        }
        for body in self.store.nodes_referencing_asset(hash)? {
            if let Ok(node) = serde_json::from_str::<KnowledgeNode>(&body) {
                if !node.domains.is_empty()
                    && node.domains.iter().any(|domain| domains.contains(domain))
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
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

    pub async fn knowledge_graph_scoped(
        &self,
        root: &str,
        domains: &std::collections::BTreeSet<String>,
    ) -> DomainResult<Value> {
        let root_node = self.knowledge_get(root).await?;
        if !domains.contains("*")
            && (root_node.domains.is_empty()
                || !root_node
                    .domains
                    .iter()
                    .any(|domain| domains.contains(domain)))
        {
            return Err(DomainError::PolicyBlocked(
                "knowledge node 不在此 Agent Session 的 Domain scope".into(),
            ));
        }
        let graph = self.knowledge_graph(root, 1).await?;
        let mut allowed_ids = std::collections::BTreeSet::from([root.to_string()]);
        if let Some(neighbors) = graph["neighbors"].as_array() {
            for neighbor in neighbors {
                let Some(id) = neighbor["nodeId"].as_str() else {
                    continue;
                };
                if let Ok(node) = self.knowledge_get(id).await {
                    if domains.contains("*")
                        || (!node.domains.is_empty()
                            && node.domains.iter().any(|domain| domains.contains(domain)))
                    {
                        allowed_ids.insert(id.to_string());
                    }
                }
            }
        }
        let neighbors = graph["neighbors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|neighbor| {
                neighbor["nodeId"]
                    .as_str()
                    .is_some_and(|id| allowed_ids.contains(id))
            })
            .cloned()
            .collect::<Vec<_>>();
        let edges = graph["edges"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|edge| {
                edge["from"]
                    .as_str()
                    .zip(edge["to"].as_str())
                    .is_some_and(|(from, to)| {
                        allowed_ids.contains(from) && allowed_ids.contains(to)
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        Ok(json!({"root": graph["root"], "edges": edges, "neighbors": neighbors}))
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

/// Execute an optional local media processor without a shell. Both streams,
/// wall-clock time, and in-memory output are bounded. `kill_on_drop` means a
/// timeout cannot leave a child behind.
async fn run_bounded_command(
    binary: &str,
    args: &[&str],
    timeout_seconds: u64,
    max_stdout_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut child = tokio::process::Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("{binary} 無法啟動：{error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{binary} stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{binary} stderr unavailable"))?;

    let operation = async {
        let stdout_task = async {
            let mut bytes = Vec::new();
            stdout
                .take(max_stdout_bytes.saturating_add(1) as u64)
                .read_to_end(&mut bytes)
                .await
                .map_err(|error| format!("{binary} stdout：{error}"))?;
            Ok::<Vec<u8>, String>(bytes)
        };
        let stderr_task = async {
            let mut bytes = Vec::new();
            stderr
                .take(16 * 1024)
                .read_to_end(&mut bytes)
                .await
                .map_err(|error| format!("{binary} stderr：{error}"))?;
            Ok::<Vec<u8>, String>(bytes)
        };
        let (stdout_result, stderr_result, status_result) =
            tokio::join!(stdout_task, stderr_task, child.wait());
        let stdout = stdout_result?;
        let stderr = stderr_result?;
        let status = status_result.map_err(|error| format!("{binary} wait：{error}"))?;
        if stdout.len() > max_stdout_bytes {
            return Err(format!(
                "{binary} stdout 超過 {max_stdout_bytes} bytes 上限"
            ));
        }
        if !status.success() {
            let detail = String::from_utf8_lossy(&stderr);
            return Err(format!(
                "{binary} 結束碼 {}：{}",
                status
                    .code()
                    .map_or_else(|| "signal".into(), |code| code.to_string()),
                detail.trim()
            ));
        }
        Ok(stdout)
    };

    tokio::time::timeout(Duration::from_secs(timeout_seconds), operation)
        .await
        .map_err(|_| format!("{binary} 超過 {timeout_seconds} 秒，已終止"))?
}

fn wav_features(bytes: &[u8]) -> Result<(Value, f64), String> {
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("目前內建音訊特徵解析器只支援 PCM WAV；其他格式需本機 ffprobe adapter".into());
    }
    let mut offset = 12usize;
    let mut channels = None;
    let mut sample_rate = None;
    let mut bits = None;
    let mut pcm_format = None;
    let mut data: Option<&[u8]> = None;
    while offset + 8 <= bytes.len() {
        let kind = &bytes[offset..offset + 4];
        let size =
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap_or([0; 4])) as usize;
        let start = offset + 8;
        let end = start.saturating_add(size).min(bytes.len());
        if kind == b"fmt " && end >= start + 16 {
            pcm_format = Some(u16::from_le_bytes([bytes[start], bytes[start + 1]]));
            channels = Some(u16::from_le_bytes([bytes[start + 2], bytes[start + 3]]));
            sample_rate = Some(u32::from_le_bytes([
                bytes[start + 4],
                bytes[start + 5],
                bytes[start + 6],
                bytes[start + 7],
            ]));
            bits = Some(u16::from_le_bytes([bytes[start + 14], bytes[start + 15]]));
        } else if kind == b"data" {
            data = Some(&bytes[start..end]);
        }
        offset = end + (size % 2);
    }
    let (channels, rate, bits, data) = (
        channels.ok_or("WAV missing channel metadata")?,
        sample_rate.ok_or("WAV missing sample rate")?,
        bits.ok_or("WAV missing bit depth")?,
        data.ok_or("WAV missing data chunk")?,
    );
    if pcm_format != Some(1) || bits != 16 || channels == 0 || rate == 0 {
        return Err("內建音訊特徵解析器只支援 16-bit PCM WAV".into());
    }
    let samples = data
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) as f64 / i16::MAX as f64)
        .collect::<Vec<_>>();
    let frames = samples.len() as f64 / channels as f64;
    let duration = frames / rate as f64;
    let rms = if samples.is_empty() {
        0.0
    } else {
        (samples.iter().map(|sample| sample * sample).sum::<f64>() / samples.len() as f64).sqrt()
    };
    let zero_crossings = samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0 && pair[1] >= 0.0) || (pair[0] >= 0.0 && pair[1] < 0.0))
        .count();
    Ok((
        json!({
            "format": "pcm-wav",
            "channels": channels,
            "sampleRateHz": rate,
            "bitsPerSample": bits,
            "durationSeconds": duration,
            "rms": rms,
            "zeroCrossings": zero_crossings,
            "tempoBpm": Value::Null,
            "tempoNote": "短片段或無可靠 beat 時不推測 BPM",
        }),
        duration,
    ))
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
