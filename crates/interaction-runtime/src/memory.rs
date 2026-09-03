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
/// 單次掃描的來源上限（storage 單頁上限）。掃到上限代表「還有沒看過的記憶」，
/// 必須誠實回報，不得讓呼叫端以為已經看過全部。
pub const BUNDLE_SCAN_LIMIT: u32 = 1000;
/// 單次匯出的上限（storage 單頁上限）；達到上限時必須誠實回報，
/// 不得讓使用者以為手上的備份是完整的。
pub const EXPORT_MAX_ITEMS: u32 = 1000;

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
        // API 已將 human/agent token 分權；此處仍重套 actor 規則作為
        // 非 HTTP 呼叫者的 defense-in-depth。kind／retention 補丁只降不升，
        // 不得成為解除候選降權的側門。
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

    /// 匯出記憶（使用者資料主權）。過期項仍可匯出（資料仍屬使用者），
    /// 但每筆附衍生 status——過期／stale 不得無標記地冒充有效資料。
    ///
    /// 誠實範圍：這裡**只有記憶**，不含知識節點、素材與衍生資料、角色互動記憶；
    /// 而且單次上限 EXPORT_MAX_ITEMS 筆（依 updated_at 由新到舊）。達到上限時
    /// `limitReached` 為 true——靜默丟掉最舊的一段，會讓使用者以為備份是全部。
    pub async fn memory_export(&self) -> DomainResult<Value> {
        let now = Utc::now();
        let bodies = self.store.list_memory(None, EXPORT_MAX_ITEMS)?;
        let limit_reached = bodies.len() as u32 >= EXPORT_MAX_ITEMS;
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
            "scope": "memory-items-only",
            "notIncluded": ["knowledge", "assets", "character-interaction-memory"],
            "limit": EXPORT_MAX_ITEMS,
            "limitReached": limit_reached,
            "note": if limit_reached {
                "已達單次匯出上限：只匯出最近更新的記憶，較舊的沒有匯出；本檔不含知識、素材與角色互動記憶。"
            } else {
                "本檔只含記憶，不含知識、素材與角色互動記憶。"
            },
            "items": items,
        }))
    }

    /// 確定性 Context Bundle（spec §12）：最小必要，附排除說明。
    /// 永不包含：stale/expired、對此 agent 不可見、敏感 tag、超出上限。
    ///
    /// 上限造成的遺漏也是排除：`excluded.overCapacity` 計數＋`truncated` 旗標。
    /// 靜默截斷會讓 UI 說「擋下來的：沒有」、讓 agent 以為上下文完整。
    pub async fn memory_context_bundle(
        &self,
        task: &str,
        domains: &[String],
        agent_id: &str,
    ) -> DomainResult<Value> {
        let now = Utc::now();
        let bodies = self.store.list_memory(None, BUNDLE_SCAN_LIMIT)?;
        // 掃到單頁上限＝還有沒看過的記憶：不知道有沒有漏，就得說不知道。
        let scan_limit_reached = bodies.len() as u32 >= BUNDLE_SCAN_LIMIT;
        let mut included: Vec<Value> = Vec::new();
        let mut needs_review: Vec<String> = Vec::new();
        let mut excluded_not_visible = 0u32;
        let mut excluded_sensitive = 0u32;
        // 排除原因都要回報（介面承諾列出「未複審候選」與 domain 過濾）：
        // 兩者只回計數，不把被排除的 id 交給 agent 當線索。
        let mut excluded_unreviewed_candidates = 0u32;
        let mut excluded_outside_domains = 0u32;
        // 通過所有過濾、只是撞到條數／bytes 上限而沒放進來的：也要回報。
        let mut excluded_over_capacity = 0u32;
        let mut bytes = 0usize;

        // Built-in Domain Packs are installed local reference data. They are
        // included only when the Session explicitly names the exact domain;
        // an empty scope remains fail-closed and means none.
        // 撞到上限就不再收（維持「遇到第一個放不下的就停」的確定性選擇），
        // 但被擋下來的每一筆都要計數——break 掉的東西才是最容易被忘記的謊。
        let mut packs_capped = false;
        for entry in self.domain_pack_context_entries(domains)? {
            let size = serde_json::to_vec(&entry).map_or(BUNDLE_MAX_BYTES + 1, |value| value.len());
            if packs_capped || included.len() >= BUNDLE_MAX_ITEMS || bytes + size > BUNDLE_MAX_BYTES
            {
                packs_capped = true;
                excluded_over_capacity += 1;
                continue;
            }
            bytes += size;
            included.push(entry);
        }

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

        let mut items_capped = false;
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
                excluded_unreviewed_candidates += 1;
                continue;
            }
            // Domain 過濾：知識類必須命中 Session 明確授權的 domain。
            // 空集合不是「全部」；它代表 Session 沒有 Domain Knowledge
            // grant，因此 fail closed，避免新 Session 靜默取得整個知識庫。
            let domain_layer = matches!(
                item.layer,
                MemoryLayer::DomainKnowledge | MemoryLayer::DomainKnowHow | MemoryLayer::Skill
            );
            if domain_layer
                && (domains.is_empty() || !item.tags.iter().any(|t| domains.contains(t)))
            {
                excluded_outside_domains += 1;
                continue;
            }
            let size = item.content.len() + item.title.len();
            // 同上：停止收錄，但把「本來可以給、只是放不下」的筆數如實回報。
            if items_capped || included.len() >= BUNDLE_MAX_ITEMS || bytes + size > BUNDLE_MAX_BYTES
            {
                items_capped = true;
                excluded_over_capacity += 1;
                continue;
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
        let truncated = excluded_over_capacity > 0 || scan_limit_reached;
        let mut note = String::from(
            "確定性選擇；stale 需重新確認、candidate 未經複審不提供、敏感標籤排除、知識類只給 Session 明確授權的 domain。",
        );
        if excluded_over_capacity > 0 {
            note.push_str(&format!(
                "本次已達份量上限，另有 {excluded_over_capacity} 筆可提供的內容沒有放進來（excluded.overCapacity）；這份上下文不完整。"
            ));
        }
        if scan_limit_reached {
            note.push_str(&format!(
                "記憶總量已達單次掃描上限 {BUNDLE_SCAN_LIMIT} 筆，更舊的記憶這次沒有被檢視（limits.scanLimitReached）。"
            ));
        }
        Ok(json!({
            "task": task,
            "agentId": agent_id,
            "domains": domains,
            "generatedAt": now,
            "includes": included,
            "excluded": {
                "needsReview": needs_review,
                "notVisibleToAgent": excluded_not_visible,
                "sensitive": excluded_sensitive,
                "unreviewedCandidates": excluded_unreviewed_candidates,
                "outsideGrantedDomains": excluded_outside_domains,
                "overCapacity": excluded_over_capacity,
            },
            "truncated": truncated,
            "limits": {
                "maxItems": BUNDLE_MAX_ITEMS,
                "maxBytes": BUNDLE_MAX_BYTES,
                "scanLimit": BUNDLE_SCAN_LIMIT,
                "scanLimitReached": scan_limit_reached,
            },
            "note": note,
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
