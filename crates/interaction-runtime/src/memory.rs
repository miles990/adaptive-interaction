//! 記憶服務（spec §10／§15／§16）：CRUD、保存期限、Context Bundle。
//!
//! - actor 規則在寫入前套用（agent 的 fact 降 inference、長期使用者記憶
//!   降 candidate）——呼叫端聲稱的身分由 API 層決定，不信 payload。
//! - Context Bundle 是**確定性**選擇：不用生成式 AI 決定給誰看什麼。
//! - 到期清除掛在 watchdog，但讀取端不等 sweep：expired 在 get／update
//!   一律 NotFound，export 附衍生 status；stale 不入 bundle、只列 needsReview。

use crate::runtime::Runtime;
use chrono::Utc;
use interaction_core::{
    apply_actor_rules, default_retention, validate_memory_item, DomainError, DomainResult,
    MemoryActor, MemoryItem, MemoryKind, MemoryLayer, MemoryStatus,
};
use serde_json::{json, Value};

/// Bundle 上限：條數與總 bytes（提供最小必要，不是全部倒給 agent）。
pub const BUNDLE_MAX_ITEMS: usize = 24;
pub const BUNDLE_MAX_BYTES: usize = 48 * 1024;

fn layer_str(layer: MemoryLayer) -> String {
    serde_json::to_value(layer)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

impl Runtime {
    /// 建立記憶（actor 由 API 層依認證面決定）。
    pub async fn memory_create(&self, mut item: MemoryItem) -> DomainResult<MemoryItem> {
        let now = Utc::now();
        item.created_at = now;
        item.updated_at = now;
        apply_actor_rules(&mut item, now);
        validate_memory_item(&item).map_err(DomainError::Validation)?;
        self.persist_memory(&item)?;
        self.store.audit(
            "memory.created",
            match &item.created_by {
                MemoryActor::Human => "human",
                MemoryActor::Agent(_) => "agent",
                MemoryActor::Runtime => "runtime",
            },
            &json!({"memoryId": item.memory_id.as_str(), "layer": item.layer, "kind": item.kind}),
        )?;
        Ok(item)
    }

    /// 部分更新（title/content/tags/retention/visibility）。
    pub async fn memory_update(&self, id: &str, patch: Value) -> DomainResult<MemoryItem> {
        let mut item = self.memory_get(id).await?;
        let mut v = serde_json::to_value(&item).unwrap_or_default();
        if let (Some(obj), Some(p)) = (v.as_object_mut(), patch.as_object()) {
            for key in [
                "title",
                "content",
                "tags",
                "retention",
                "agentVisibility",
                "agentDenylist",
                "kind",
                "confidence",
            ] {
                if let Some(val) = p.get(key) {
                    obj.insert(key.to_string(), val.clone());
                }
            }
        }
        let mut updated: MemoryItem =
            serde_json::from_value(v).map_err(|e| DomainError::Validation(e.to_string()))?;
        let patched_at = Utc::now();
        updated.updated_at = patched_at;
        // flat token 無法辨識呼叫者：agent 建立的項目在 PATCH 時重套 actor
        // 規則（只降不升），kind／retention 補丁不得成為解除候選降權的側門。
        apply_actor_rules(&mut updated, patched_at);
        validate_memory_item(&updated).map_err(DomainError::Validation)?;
        item = updated;
        self.persist_memory(&item)?;
        Ok(item)
    }

    pub async fn memory_get(&self, id: &str) -> DomainResult<MemoryItem> {
        let body = self
            .store
            .get_memory(id)?
            .ok_or_else(|| DomainError::NotFound(format!("memory {id}")))?;
        let item: MemoryItem =
            serde_json::from_str(&body).map_err(|e| DomainError::Internal(e.to_string()))?;
        // expiresAt 到期＝停止使用：sweep 只是延遲的實體刪除，讀取端行為
        // 必須與刪除後一致（NotFound）——否則過期資料在 sweep 前仍被當有效
        // 供應，PATCH 也能讓它復活。過期後唯一合法操作是刪除（不走此路徑）。
        if item.status(Utc::now()) == MemoryStatus::Expired {
            return Err(DomainError::NotFound(format!("memory {id} (expired)")));
        }
        Ok(item)
    }

    pub async fn memory_delete(&self, id: &str) -> DomainResult<bool> {
        let deleted = self.store.delete_memory(id)?;
        if deleted {
            self.store
                .audit("memory.deleted", "human", &json!({"memoryId": id}))?;
        }
        Ok(deleted)
    }

    /// 列出（layer 可選）＋衍生狀態（active/stale/expired）。
    pub async fn memory_list(&self, layer: Option<&str>, limit: u32) -> DomainResult<Value> {
        let now = Utc::now();
        let bodies = self.store.list_memory(layer, limit)?;
        let mut items = Vec::new();
        for body in bodies {
            if let Ok(item) = serde_json::from_str::<MemoryItem>(&body) {
                let status = item.status(now);
                let mut v = serde_json::to_value(&item).unwrap_or_default();
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("status".into(), json!(status));
                }
                items.push(v);
            }
        }
        Ok(json!({"items": items, "count": items.len()}))
    }

    /// 清除 session 暫存（session 結束／使用者「清除短期記憶」）。
    /// storage 單次列表上限 1000：迴圈清到空為止，不得靜默留殘量——
    /// 使用者主動的隱私清除必須完整，清不完就誠實回報。
    pub async fn memory_clear_session_context(&self) -> DomainResult<u32> {
        // 輪數上限只防病態情況（損壞列刪不掉、並行狂寫）；正常一輪至少
        // 刪掉一整頁，遠不會觸頂。
        const MAX_ROUNDS: u32 = 1000;
        let mut n: u32 = 0;
        for _ in 0..MAX_ROUNDS {
            let bodies = self.store.list_memory(Some("session-context"), 1000)?;
            if bodies.is_empty() {
                return Ok(n);
            }
            let mut deleted_this_round: u32 = 0;
            for body in bodies {
                if let Ok(item) = serde_json::from_str::<MemoryItem>(&body) {
                    if self.store.delete_memory(item.memory_id.as_str())? {
                        n += 1;
                        deleted_this_round += 1;
                    }
                }
            }
            // 一輪一筆都刪不掉（如 body 解析失敗取不到 id）：迴圈不可能
            // 收斂，停下來走誠實回報。
            if deleted_this_round == 0 {
                break;
            }
        }
        let remaining = self.store.list_memory(Some("session-context"), 1000)?.len();
        if remaining == 0 {
            Ok(n)
        } else {
            Err(DomainError::Internal(format!(
                "session-context 清除未完成：已刪 {n} 筆，仍殘留至少 {remaining} 筆無法清除"
            )))
        }
    }

    /// 匯出全部記憶（使用者資料主權）。過期項仍可匯出（資料仍屬使用者），
    /// 但每筆附衍生 status——過期／stale 不得無標記地冒充有效資料。
    pub async fn memory_export(&self) -> DomainResult<Value> {
        let now = Utc::now();
        let bodies = self.store.list_memory(None, 1000)?;
        let mut items: Vec<Value> = Vec::new();
        for body in bodies {
            let Ok(mut v) = serde_json::from_str::<Value>(&body) else {
                continue;
            };
            // schema 不符的舊資料照樣匯出（主權），但狀態未知標 uncertain。
            let status = serde_json::from_str::<MemoryItem>(&body)
                .map(|item| json!(item.status(now)))
                .unwrap_or_else(|_| json!("uncertain"));
            if let Some(obj) = v.as_object_mut() {
                obj.insert("status".into(), status);
            }
            items.push(v);
        }
        Ok(json!({
            "exportedAt": now,
            "count": items.len(),
            "items": items,
        }))
    }

    /// 確定性 Context Bundle（spec §12）：最小必要，附排除說明。
    /// 永不包含：stale/expired、對此 agent 不可見、敏感 tag、超出上限。
    pub async fn memory_context_bundle(
        &self,
        task: &str,
        domains: &[String],
        agent_id: &str,
    ) -> DomainResult<Value> {
        let now = Utc::now();
        let bodies = self.store.list_memory(None, 1000)?;
        let mut included: Vec<Value> = Vec::new();
        let mut needs_review: Vec<String> = Vec::new();
        let mut excluded_not_visible = 0u32;
        let mut excluded_sensitive = 0u32;
        let mut bytes = 0usize;

        let mut items: Vec<MemoryItem> = bodies
            .iter()
            .filter_map(|b| serde_json::from_str(b).ok())
            .collect();
        // 確定性排序：層級優先序 → 最近使用 → id。
        let layer_rank = |l: MemoryLayer| match l {
            MemoryLayer::DomainKnowHow => 0,
            MemoryLayer::Skill => 1,
            MemoryLayer::DomainKnowledge => 2,
            MemoryLayer::TaskMemory => 3,
            MemoryLayer::AgentHandoff => 4,
            MemoryLayer::WorldKnowledge => 5,
            _ => 9,
        };
        items.sort_by(|a, b| {
            layer_rank(a.layer)
                .cmp(&layer_rank(b.layer))
                .then(b.updated_at.cmp(&a.updated_at))
                .then(a.memory_id.as_str().cmp(b.memory_id.as_str()))
        });

        for item in items {
            match item.status(now) {
                MemoryStatus::Expired => continue,
                MemoryStatus::Stale => {
                    needs_review.push(item.memory_id.as_str().to_string());
                    continue;
                }
                MemoryStatus::Active => {}
            }
            if item.tags.iter().any(|t| t == "sensitive") {
                excluded_sensitive += 1;
                continue;
            }
            if !item.visible_to_agent(agent_id) {
                excluded_not_visible += 1;
                continue;
            }
            // Candidate 不入 bundle（未經複審的內容不給 agent 當上下文）。
            if item.kind == MemoryKind::Candidate {
                continue;
            }
            // Domain 過濾：知識類必須命中請求的 domain tags（空=全部）。
            let domain_layer = matches!(
                item.layer,
                MemoryLayer::DomainKnowledge | MemoryLayer::DomainKnowHow | MemoryLayer::Skill
            );
            if domain_layer && !domains.is_empty() && !item.tags.iter().any(|t| domains.contains(t))
            {
                continue;
            }
            let size = item.content.len() + item.title.len();
            if included.len() >= BUNDLE_MAX_ITEMS || bytes + size > BUNDLE_MAX_BYTES {
                break;
            }
            bytes += size;
            included.push(json!({
                "memoryId": item.memory_id.as_str(),
                "layer": item.layer,
                "kind": item.kind,
                "title": item.title,
                "content": item.content,
                "confidence": item.confidence,
            }));
        }
        Ok(json!({
            "task": task,
            "agentId": agent_id,
            "generatedAt": now,
            "includes": included,
            "excluded": {
                "needsReview": needs_review,
                "notVisibleToAgent": excluded_not_visible,
                "sensitive": excluded_sensitive,
            },
            "note": "確定性選擇；stale 需重新確認、candidate 未經複審不提供、敏感標籤排除。",
        }))
    }

    /// watchdog：到期記憶清除。
    pub async fn sweep_memory(&self) {
        if let Ok(n) = self.store.prune_expired_memory(Utc::now()) {
            if n > 0 {
                tracing::info!(target: "interaction.memory", pruned = n, "expired memories removed");
            }
        }
    }

    fn persist_memory(&self, item: &MemoryItem) -> DomainResult<()> {
        let body = serde_json::to_string(item).map_err(|e| DomainError::Internal(e.to_string()))?;
        self.store.save_memory(
            item.memory_id.as_str(),
            &layer_str(item.layer),
            &serde_json::to_value(item.kind)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            item.retention
                .expires_at
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                .as_deref(),
            item.retention
                .review_after
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                .as_deref(),
            &body,
        )
    }
}

/// 便利建構（API 層用）。
pub fn memory_from_input(input: Value, actor: MemoryActor) -> Result<MemoryItem, String> {
    let now = Utc::now();
    let layer: MemoryLayer =
        serde_json::from_value(input.get("layer").cloned().unwrap_or(json!("user-memory")))
            .map_err(|e| format!("layer: {e}"))?;
    let kind: MemoryKind =
        serde_json::from_value(input.get("kind").cloned().unwrap_or(json!("fact")))
            .map_err(|e| format!("kind: {e}"))?;
    let mut item = interaction_core::new_memory_item(
        layer,
        kind,
        input.get("title").and_then(|v| v.as_str()).unwrap_or(""),
        input.get("content").and_then(|v| v.as_str()).unwrap_or(""),
        actor,
        now,
    );
    if let Some(tags) = input.get("tags").and_then(|v| v.as_array()) {
        item.tags = tags
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect();
    }
    if let Some(conf) = input.get("confidence").and_then(|v| v.as_f64()) {
        item.confidence = conf;
    }
    if let Some(prov) = input.get("provenance").and_then(|v| v.as_array()) {
        item.provenance = prov
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect();
    }
    if let Some(r) = input.get("retention") {
        // 明確給 {} = until-deleted；不給 = 層級預設。
        item.retention =
            serde_json::from_value(r.clone()).map_err(|e| format!("retention: {e}"))?;
    } else {
        item.retention = default_retention(layer, now);
    }
    if let Some(vis) = input.get("agentVisibility").and_then(|v| v.as_array()) {
        item.agent_visibility = vis
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect();
    }
    if let Some(deny) = input.get("agentDenylist").and_then(|v| v.as_array()) {
        item.agent_denylist = deny
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect();
    }
    Ok(item)
}

impl Runtime {
    /// 同步內部建立（runtime 自身寫入，如 handoff 落地）。
    pub(crate) fn memory_create_internal(&self, item: &MemoryItem) -> DomainResult<()> {
        validate_memory_item(item).map_err(DomainError::Validation)?;
        let body = serde_json::to_string(item).map_err(|e| DomainError::Internal(e.to_string()))?;
        self.store.save_memory(
            item.memory_id.as_str(),
            &layer_str(item.layer),
            &serde_json::to_value(item.kind)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            item.retention
                .expires_at
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                .as_deref(),
            item.retention
                .review_after
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                .as_deref(),
            &body,
        )
    }
}
