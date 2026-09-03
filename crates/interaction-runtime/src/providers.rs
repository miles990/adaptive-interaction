//! Provider lifecycle in the runtime: builtin provider registration,
//! declarative adapter loading (config/adapters/*.yaml), pairing, revocation
//! and persistence. Pairing / install / enable / consent stay separate steps.

use crate::runtime::Runtime;
use interaction_core::{
    ActuatorId, DomainError, DomainResult, ProviderDescriptor, ProviderId, ProviderIdentity,
    ProviderKind, ProviderState, ReceptorId, ReceptorMode, Timestamp, TrustLevel,
};
use interaction_registry::providers::discovered;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// 「已測試」證據（spec §9.3）。掃描到 metadata、設定檔存在、甚至狀態變成
/// Available，都**不等於**測過：這筆記錄只在 runtime 真的觀察到一次成功／
/// 失敗時才寫入，並誠實保留是誰、用什麼方式測的。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTested {
    pub at: Timestamp,
    /// `handshake`＝宣告式裝置連線（serial/mqtt/ble）成功，代表 hello 身分
    /// 驗證＋pair-ok 已完成；`capability`＝該 provider 的受器讀成功／動器回
    /// ack；`human`＝使用者按下「測試裝置」。
    pub how: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 記錄來源：哪一種能力提供了這次證據。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestedCapability {
    Receptor,
    Actuator,
}

/// 自動記錄的節流窗：同樣的結果一分鐘內只寫一次，避免每次讀取都打 DB。
/// 人為測試不節流（使用者按了就要看到最新結果）。
const TESTED_AUTO_THROTTLE_SECS: i64 = 60;

/// 「已測試」證據的人話註記。UI（一般模式）會把它原樣顯示，所以這裡不用
/// 受器／動器／hello／pair-ok 這類技術詞：感知來源＝receptor、回應方式＝actuator；
/// 有實體連線（serial/mqtt/ble）的裝置多加一句「裝置報上身分並完成配對」。
/// 能力 id 一律保留（是可追查的事實），有人話名稱時放在前面。
/// 動器只證明「已回覆收到（acknowledged）」——誠實階梯：acknowledged ≠ completed。
pub fn tested_note(
    kind: TestedCapability,
    capability_id: &str,
    human_name: Option<&str>,
    linked: bool,
) -> String {
    let named = match human_name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) if name != capability_id => format!("「{name}」（{capability_id}）"),
        _ => capability_id.to_string(),
    };
    let what = match kind {
        TestedCapability::Receptor => format!("感知來源 {named} 讀取成功"),
        TestedCapability::Actuator => {
            format!("回應方式 {named} 已回覆收到（acknowledged，不代表已完成）")
        }
    };
    if linked {
        format!("裝置報上身分並完成配對：{what}")
    } else {
        what
    }
}

/// `detail` 既是人話註記、也是「已測試」證據與設定檔警告的載體：有證據或
/// 警告時寫成 `{"note": …, "tested": {…}, "warnings": [ … ]}`，
/// 都沒有時維持原本的純文字（向後相容）。
pub fn split_provider_detail(detail: Option<&str>) -> (Option<String>, Option<ProviderTested>) {
    let Some(text) = detail else {
        return (None, None);
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
        return (Some(text.to_string()), None);
    };
    // 只拆我們自己寫出來的形狀；其他 JSON 一律當純文字註記（不臆造證據）。
    if !map.contains_key("tested") && !map.contains_key("warnings") {
        return (Some(text.to_string()), None);
    }
    let tested = map
        .get("tested")
        .and_then(|v| serde_json::from_value::<ProviderTested>(v.clone()).ok());
    let note = map
        .get("note")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    (note, tested)
}

/// `detail` 裡的設定檔警告（明文憑證…）。純文字或沒有警告時回空陣列。
/// 這些字串會點名能力與欄位，是**進階模式／CLI** 的內容，
/// 一般模式看到的是 [`warning_summary`] 那一句。
pub fn provider_detail_warnings(detail: Option<&str>) -> Vec<String> {
    let Some(text) = detail else {
        return vec![];
    };
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
        return vec![];
    };
    map.get("warnings")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 警告的人話摘要（一般模式那一句）：只說有幾項與性質，不外洩能力 id 或欄位名
/// （原文在 `warnings` 陣列裡，給進階模式與 `interact-ai providers show`）。
fn warning_summary(warnings: &[String]) -> String {
    format!(
        "設定檔有 {} 項安全提醒：連線密碼／配對碼是明文寫在設定檔裡，建議改成外部保管。",
        warnings.len()
    )
}

fn merge_provider_detail(
    note: Option<&str>,
    tested: Option<&ProviderTested>,
    warnings: &[String],
) -> Option<String> {
    if tested.is_none() && warnings.is_empty() {
        return note.map(|s| s.to_string());
    }
    let mut obj = serde_json::Map::new();
    // 警告存在時 `note` 一定要有內容：舊介面看不懂新鍵時會退回整串 JSON 當註記，
    // 那樣就會把技術字串印到一般模式畫面上。
    let note = match (note, warnings.is_empty()) {
        (Some(note), _) if !note.is_empty() => note.to_string(),
        (_, false) => warning_summary(warnings),
        _ => String::new(),
    };
    if !note.is_empty() {
        obj.insert("note".into(), Value::String(note));
    }
    if let Some(record) = tested {
        obj.insert(
            "tested".into(),
            serde_json::to_value(record).unwrap_or(Value::Null),
        );
    }
    if !warnings.is_empty() {
        obj.insert(
            "warnings".into(),
            Value::Array(warnings.iter().cloned().map(Value::String).collect()),
        );
    }
    Some(Value::Object(obj).to_string())
}

impl Runtime {
    /// Called once at startup: builtin provider + persisted providers +
    /// declarative adapters from `config/adapters/*.yaml`.
    pub(crate) async fn init_providers(&self) {
        // 1) Builtin local provider (trust: builtin, always available).
        let builtin = ProviderDescriptor {
            identity: ProviderIdentity {
                id: ProviderId::new("provider.local.builtin"),
                kind: ProviderKind::Local,
                display_name: "內建本機能力".into(),
                trust_level: TrustLevel::Builtin,
                origin: "builtin".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                fingerprint: None,
                human: None,
            },
            state: ProviderState::Available,
            receptors: self
                .registry
                .receptor_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| !id.starts_with("companion."))
                .collect(),
            actuators: self
                .registry
                .actuator_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| !id.starts_with("companion."))
                .collect(),
            tool_operations: self
                .registry
                .tool_operations()
                .await
                .into_iter()
                .map(|m| m.name)
                .collect(),
            paired_at: None,
            last_seen: Some(chrono::Utc::now()),
            detail: None,
        };
        let _ = self.providers.register(builtin).await;

        // 1.5) Presentation Provider：桌面角色是一級 provider，能力逐項宣告。
        //      信任層級 Builtin（本應用自帶的表面），但可用性誠實跟隨視窗
        //      presence（receptor/actuator 健康由 bridge 決定）。顯示名不寫死
        //      任何角色：由 `/v1/character/hello` 協商到的 manifest displayName
        //      決定（見 `with_character_label`），未連線前誠實標示。
        let companion = ProviderDescriptor {
            identity: ProviderIdentity {
                id: ProviderId::new(crate::character::COMPANION_PROVIDER_ID),
                kind: ProviderKind::Companion,
                display_name: self.companion_provider_display_name(),
                trust_level: TrustLevel::Builtin,
                origin: "builtin.presentation".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                fingerprint: None,
                human: None,
            },
            state: ProviderState::Available,
            receptors: self
                .registry
                .receptor_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| id.starts_with("companion."))
                .collect(),
            actuators: self
                .registry
                .actuator_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| id.starts_with("companion."))
                .collect(),
            tool_operations: vec![],
            paired_at: None,
            last_seen: Some(chrono::Utc::now()),
            detail: Some(self.companion_provider_detail()),
        };
        let _ = self.providers.register(companion).await;

        // 2) Persisted provider records (paired devices etc.).
        //    Crash/restart must NOT auto-recover an operational device: pairing
        //    survives, but any provider left Available/Busy/Degraded comes back
        //    Disabled so a physical device never re-arms itself on its own.
        if let Ok(bodies) = self.store.all_providers() {
            for body in bodies {
                if let Ok(mut desc) = serde_json::from_str::<ProviderDescriptor>(&body) {
                    // 「已測試」證據跨重啟保留（它記的是過去確實發生過的事），
                    // 但它只是歷史，不會讓裝置自己回到可用狀態。registry 裡
                    // 只留人話註記，證據放回 runtime 的表。
                    let (note, tested) = split_provider_detail(desc.detail.as_deref());
                    desc.detail = note;
                    if let Some(tested) = tested {
                        if let Ok(mut map) = self.provider_tested.lock() {
                            map.insert(desc.identity.id.as_str().to_string(), tested);
                        }
                    }
                    let downgraded = desc.state.is_operational();
                    if downgraded {
                        desc.state = ProviderState::Disabled;
                        desc.detail = Some("re-armed on restart requires explicit enable".into());
                    }
                    let id = desc.identity.id.clone();
                    let _ = self.providers.register(desc).await;
                    if downgraded {
                        self.persist_provider(&id).await;
                    }
                }
            }
        }

        // 3) Declarative adapters (File=Truth; human-owned specs).
        let dir = self.paths.home.join("config").join("adapters");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_yaml = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e, "yaml" | "yml" | "json"))
                .unwrap_or(false);
            if !is_yaml {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "adapter spec unreadable");
                    continue;
                }
            };
            match interaction_adapter_declarative::parse_spec(&text) {
                Ok(spec) => {
                    if let Err(e) = self.register_declarative_spec(&spec).await {
                        tracing::warn!(path = %path.display(), error = %e, "adapter spec rejected");
                    }
                }
                Err(e) => {
                    // Invalid spec never crashes the runtime; it is surfaced.
                    tracing::warn!(path = %path.display(), error = %e, "adapter spec invalid");
                }
            }
        }
    }

    /// Register one declarative spec's capabilities + provider record.
    pub async fn register_declarative_spec(
        &self,
        spec: &interaction_adapter_declarative::DeclarativeSpec,
    ) -> DomainResult<()> {
        let built = interaction_adapter_declarative::build(spec, Some(self.paths.home.clone()))
            .map_err(DomainError::Validation)?;
        let identity = spec.provider.clone().unwrap_or(ProviderIdentity {
            id: ProviderId::new(format!("provider.adapter.{}", spec.id)),
            kind: ProviderKind::Device,
            display_name: spec.display_name.clone().unwrap_or_else(|| spec.id.clone()),
            trust_level: TrustLevel::Untrusted,
            origin: "config/adapters".into(),
            version: String::new(),
            fingerprint: None,
            human: None,
        });
        let provider_id = identity.id.clone();
        // 記住這個 provider 開出來的實體連線（serial/mqtt/ble），
        // disable／revoke 時才關得掉——停用的 provider 不得繼續佔著埠或
        // broker 連線做無盡重連。
        interaction_adapter_declarative::register_provider_links(
            provider_id.as_str(),
            &built.links,
        );
        // 有實體連線（serial/mqtt/ble）的 provider：之後任何一次成功的讀取或
        // 命令都必然通過 hello 身分＋pair-ok 握手，證據等級記為 handshake。
        if let Ok(mut links) = self.device_link_providers.lock() {
            if built.links.is_empty() {
                links.remove(provider_id.as_str());
            } else {
                links.insert(provider_id.as_str().to_string());
            }
        }

        let mut receptor_ids = Vec::new();
        let mut actuator_ids = Vec::new();
        for receptor in built.receptors {
            receptor_ids.push(receptor.manifest().id.as_str().to_string());
            self.registry.register_receptor(receptor).await?;
        }
        for actuator in built.actuators {
            actuator_ids.push(actuator.manifest().id.as_str().to_string());
            self.registry.register_actuator(actuator).await?;
        }

        // A persisted record (e.g. already paired) wins over a fresh one.
        if let Ok(existing) = self.providers.get(&provider_id).await {
            self.providers
                .attach_capabilities(&provider_id, receptor_ids, actuator_ids, vec![])
                .await?;
            // 明文憑證警告是「這份 spec 現在的事實」，不是狀態註記：重啟後
            // （persisted 記錄勝出）與 re-arm 的「requires explicit enable」
            // 都不得把它吃掉，所以每次註冊都重新掛回去。狀態不變（自轉移合法），
            // 只換 detail。
            let (note, _) = split_provider_detail(existing.detail.as_deref());
            let detail = merge_provider_detail(note.as_deref(), None, &built.warnings);
            if detail != existing.detail {
                self.providers
                    .transition(&provider_id, existing.state, detail)
                    .await?;
                self.persist_provider(&provider_id).await;
            }
            return Ok(());
        }
        let mut desc = discovered(identity);
        // Declared in human-owned config = installed; still DISABLED and
        // consent-gated until the human enables each capability.
        desc.state = ProviderState::Installed;
        desc.receptors = receptor_ids;
        desc.actuators = actuator_ids;
        // 建置時發現的問題（明文憑證…）跟著 provider 走，一般模式看得到摘要、
        // 進階模式與 CLI 看得到原文。靜靜吞掉等於幫使用者隱瞞。
        desc.detail = merge_provider_detail(None, None, &built.warnings);
        self.providers.register(desc.clone()).await?;
        self.persist_provider(&provider_id).await;
        Ok(())
    }

    /// 對外的 provider 清單：一律附上「已測試」證據（沒測過就是沒有，不
    /// 用狀態假裝）。
    pub async fn list_providers(&self) -> Vec<ProviderDescriptor> {
        self.providers
            .list()
            .await
            .into_iter()
            .map(|desc| self.with_character_label(self.with_tested(desc)))
            .collect()
    }

    pub async fn get_provider(&self, id: &ProviderId) -> DomainResult<ProviderDescriptor> {
        Ok(self.with_character_label(self.with_tested(self.providers.get(id).await?)))
    }

    /// 桌面角色 provider 的顯示名／註記跟著目前協商到的角色走（例如
    /// 「桌面角色：小樞（Presentation）」），尚未 hello 前為「桌面角色（尚未連線）」。
    fn with_character_label(&self, mut desc: ProviderDescriptor) -> ProviderDescriptor {
        if desc.identity.id.as_str() == crate::character::COMPANION_PROVIDER_ID {
            desc.identity.display_name = self.companion_provider_display_name();
            let (_, tested) = split_provider_detail(desc.detail.as_deref());
            desc.detail = merge_provider_detail(
                Some(&self.companion_provider_detail()),
                tested.as_ref(),
                &[],
            );
        }
        desc
    }

    /// registry 裡的 detail 是人話註記＋設定檔警告；對外輸出時才把證據併進去。
    fn with_tested(&self, mut desc: ProviderDescriptor) -> ProviderDescriptor {
        let tested = self.provider_tested_record(desc.identity.id.as_str());
        let warnings = provider_detail_warnings(desc.detail.as_deref());
        let (note, _) = split_provider_detail(desc.detail.as_deref());
        desc.detail = merge_provider_detail(note.as_deref(), tested.as_ref(), &warnings);
        desc
    }

    pub fn provider_tested_record(&self, provider_id: &str) -> Option<ProviderTested> {
        self.provider_tested.lock().ok()?.get(provider_id).cloned()
    }

    // ------------------------------------------------------------------
    // 「已測試」證據（spec §9.3）：只發現 ≠ 已配對 ≠ 已測試 ≠ 已啟用
    // ------------------------------------------------------------------

    /// 記一次證據。`how != "human"` 時同結果一分鐘內只寫一次（節流），
    /// 人為測試永遠即時覆蓋。回傳實際生效的紀錄。
    pub(crate) async fn record_provider_tested(
        &self,
        id: &ProviderId,
        how: &str,
        ok: bool,
        note: impl Into<String>,
    ) -> ProviderTested {
        let record = ProviderTested {
            at: chrono::Utc::now(),
            how: how.to_string(),
            ok,
            note: Some(note.into()),
        };
        {
            let Ok(mut map) = self.provider_tested.lock() else {
                return record;
            };
            if how != "human" {
                if let Some(previous) = map.get(id.as_str()) {
                    let same = previous.ok == ok && previous.how == record.how;
                    let fresh = (record.at - previous.at).num_seconds() < TESTED_AUTO_THROTTLE_SECS;
                    if same && fresh {
                        return previous.clone();
                    }
                }
            }
            map.insert(id.as_str().to_string(), record.clone());
        }
        self.persist_provider(id).await;
        record
    }

    /// 某個能力剛剛真的成功了 → 把證據記到它的 provider 上。
    /// 讀不到 provider（例如能力不屬於任何 provider）就什麼都不做。
    pub(crate) async fn note_capability_tested(&self, kind: TestedCapability, capability_id: &str) {
        self.note_capability_tested_on(kind, capability_id, None)
            .await
    }

    /// 同上，但由呼叫端指名「真正執行的那一台裝置」（driver 回報的 deviceId）。
    /// 同一個能力 id 可能同時屬於多台手機（`provider.mobile.<deviceId>`），
    /// 這時只有 driver 說的那一台才是事實——指名了卻找不到對應的 provider 時
    /// 什麼都不記，絕不把證據掛到別台身上。
    pub(crate) async fn note_capability_tested_on(
        &self,
        kind: TestedCapability,
        capability_id: &str,
        device_id: Option<&str>,
    ) {
        let Some(descriptor) = self
            .provider_of_capability(kind, capability_id, device_id)
            .await
        else {
            return;
        };
        let id = descriptor.identity.id;
        let linked = self
            .device_link_providers
            .lock()
            .map(|links| links.contains(id.as_str()))
            .unwrap_or(false);
        // 人話名稱來自能力 manifest（沒有就只用 id）；UI 會把這段 note 原樣顯示。
        let human_name = match kind {
            TestedCapability::Receptor => self
                .registry
                .receptor_manifests()
                .await
                .into_iter()
                .find(|m| m.id.as_str() == capability_id)
                .map(|m| m.name),
            TestedCapability::Actuator => self
                .registry
                .actuator_manifests()
                .await
                .into_iter()
                .find(|m| m.id.as_str() == capability_id)
                .map(|m| m.name),
        };
        let how = if linked { "handshake" } else { "capability" };
        let note = tested_note(kind, capability_id, human_name.as_deref(), linked);
        self.record_provider_tested(&id, how, true, note).await;
    }

    async fn provider_of_capability(
        &self,
        kind: TestedCapability,
        capability_id: &str,
        device_id: Option<&str>,
    ) -> Option<ProviderDescriptor> {
        let lists = |p: &ProviderDescriptor| {
            let list = match kind {
                TestedCapability::Receptor => &p.receptors,
                TestedCapability::Actuator => &p.actuators,
            };
            list.iter().any(|id| id == capability_id)
        };
        // 指名了裝置：只認那一台的 provider（手機是 `provider.mobile.<deviceId>`）。
        // 找不到、或那台 provider 根本沒有這個能力 → 什麼都不記；把證據記到
        // 另一台身上比沒有證據更糟（使用者會以為那台測過了）。
        if let Some(device_id) = device_id.map(str::trim).filter(|id| !id.is_empty()) {
            let pid = ProviderId::new(format!("provider.mobile.{device_id}"));
            return self.providers.get(&pid).await.ok().filter(lists);
        }
        self.providers.list().await.into_iter().find(lists)
    }

    /// 人類按下「測試裝置」：對這個 provider 的第一個**現在真的開著、且能
    /// 主動讀取**的受器做一次讀取。只讀不動——不會觸發任何動器，也不會替
    /// 使用者打開任何被停用的感測器（停用的受器直接誠實回報讀不到）。
    pub async fn test_provider(&self, id: &ProviderId) -> DomainResult<Value> {
        let descriptor = self.providers.get(id).await?;
        let (chosen, blocked) = self.pick_testable_receptor(&descriptor).await;
        let Some(receptor_id) = chosen else {
            let reason = match (descriptor.receptors.is_empty(), blocked) {
                (true, _) => "這個提供者沒有可讀的受器，無法在不觸發動器的情況下測試".to_string(),
                (false, Some(why)) => format!("這個提供者的受器現在都讀不到：{why}"),
                (false, None) => "這個提供者的受器現在都讀不到".to_string(),
            };
            let tested = self
                .record_provider_tested(id, "human", false, reason.clone())
                .await;
            self.audit_provider_test(id, None, false, &reason);
            return Ok(json!({
                "providerId": id.as_str(),
                "ok": false,
                "receptorId": Value::Null,
                "reason": reason,
                "tested": tested,
            }));
        };
        match self.observe_fresh(&receptor_id).await {
            Ok(observation) => {
                let note = format!("人為測試：讀取受器 {receptor_id} 成功");
                let tested = self.record_provider_tested(id, "human", true, note).await;
                self.audit_provider_test(id, Some(receptor_id.as_str()), true, "read ok");
                Ok(json!({
                    "providerId": id.as_str(),
                    "ok": true,
                    "receptorId": receptor_id.as_str(),
                    "observation": observation,
                    "tested": tested,
                }))
            }
            Err(e) => {
                let reason = e.to_string();
                let tested = self
                    .record_provider_tested(
                        id,
                        "human",
                        false,
                        format!("人為測試：讀取受器 {receptor_id} 失敗：{reason}"),
                    )
                    .await;
                self.audit_provider_test(id, Some(receptor_id.as_str()), false, &reason);
                Ok(json!({
                    "providerId": id.as_str(),
                    "ok": false,
                    "receptorId": receptor_id.as_str(),
                    "reason": reason,
                    "tested": tested,
                }))
            }
        }
    }

    /// 挑一個可以主動讀的受器：優先 Poll/Stream（event 受器沒有新事件時本來
    /// 就讀不到，拿它當測試結果會誤導）。回傳 (選中的受器, 第一個擋住的原因)。
    async fn pick_testable_receptor(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> (Option<ReceptorId>, Option<String>) {
        let mut fallback: Option<ReceptorId> = None;
        let mut blocked: Option<String> = None;
        for raw in &descriptor.receptors {
            let receptor_id = ReceptorId::new(raw);
            match self.registry.receptor(&receptor_id).await {
                Ok(instance) => {
                    if instance.manifest().mode != ReceptorMode::Event {
                        return (Some(receptor_id), blocked);
                    }
                    if fallback.is_none() {
                        fallback = Some(receptor_id);
                    }
                }
                Err(e) => {
                    if blocked.is_none() {
                        blocked = Some(e.to_string());
                    }
                }
            }
        }
        (fallback, blocked)
    }

    fn audit_provider_test(&self, id: &ProviderId, receptor: Option<&str>, ok: bool, note: &str) {
        let _ = self.store.audit(
            "provider.tested",
            "user",
            &json!({
                "providerId": id.as_str(),
                "receptorId": receptor,
                "ok": ok,
                "note": note,
            }),
        );
    }

    /// Pairing ceremony (shared-code): the human enters the code the device
    /// shows. The stored fingerprint mixes a fresh random per-pairing secret so
    /// it is NOT a pure function of the public provider id + code (which anyone
    /// could recompute); an IP address is never an identity. Transitions →
    /// Paired.
    ///
    /// Honest scope note: this records that a human completed a pairing with a
    /// secret. Enforcing device identity on every request (rejecting an
    /// imposter at the same address) needs device-side crypto that HTTP/mock
    /// devices in this build do not present — see docs/ARCHITECTURE.md.
    pub async fn pair_provider(
        &self,
        id: &ProviderId,
        pairing_code: &str,
    ) -> DomainResult<ProviderDescriptor> {
        if pairing_code.trim().len() < 4 {
            return Err(DomainError::Validation(
                "pairing code must be at least 4 characters".into(),
            ));
        }
        // Random per-pairing secret: makes the fingerprint unforgeable from the
        // public (id, code) pair.
        let salt = uuid::Uuid::new_v4();
        let mut hasher = Sha256::new();
        hasher.update(id.as_str().as_bytes());
        hasher.update(b":");
        hasher.update(pairing_code.trim().as_bytes());
        hasher.update(b":");
        hasher.update(salt.as_bytes());
        let fingerprint = hex_lower(&hasher.finalize());

        // discovered/unpaired → paired (the registry refuses shortcuts).
        let desc = self.providers.get(id).await?;
        if desc.state == ProviderState::Discovered {
            self.providers
                .transition(id, ProviderState::Unpaired, None)
                .await?;
        }
        let mut updated = self
            .providers
            .transition(id, ProviderState::Paired, Some("paired via code".into()))
            .await?;
        updated.identity.fingerprint = Some(fingerprint.clone());
        updated.identity.trust_level = TrustLevel::Paired;
        // Write the fingerprint back into the registry record.
        self.providers.remove(id).await.ok();
        self.providers.register(updated.clone()).await?;
        self.persist_provider(id).await;
        // Character Protocol §11：paired → greet（device-online）。
        self.character_project_provider(id, ProviderState::Paired);
        self.store.audit(
            "provider.paired",
            "user",
            &serde_json::json!({"providerId": id.as_str(), "fingerprint": fingerprint}),
        )?;
        Ok(updated)
    }

    /// Explicit lifecycle transition (install/enable/disable…), persisted.
    ///
    /// 停用類的狀態（disabled/closed/expired）必須真的把裝置連線關掉：
    /// 「停用」不能只是不派工，還在背景重連的連線＝使用者以為關了但沒關。
    pub async fn transition_provider(
        &self,
        id: &ProviderId,
        state: ProviderState,
    ) -> DomainResult<ProviderDescriptor> {
        // 狀態換了，設定檔警告還在：`transition` 會整個覆寫 detail，所以警告
        // 必須自己帶過去，否則按一次「啟用」就會把明文憑證的提醒洗掉。
        // 狀態註記（例如 re-arm 的說明）本來就只屬於前一個狀態，不帶。
        let warnings = provider_detail_warnings(
            self.providers
                .get(id)
                .await
                .ok()
                .and_then(|d| d.detail)
                .as_deref(),
        );
        let desc = self
            .providers
            .transition(id, state, merge_provider_detail(None, None, &warnings))
            .await?;
        if matches!(
            state,
            ProviderState::Disabled | ProviderState::Closed | ProviderState::Expired
        ) {
            self.close_declarative_links(id, "disabled");
        }
        self.persist_provider(id).await;
        // Character Protocol §11：available → greet、disconnected → notice。
        self.character_project_provider(id, state);
        Ok(desc)
    }

    /// 關閉某 provider 的宣告式 adapter 連線（若有）。回傳關掉的連線描述。
    fn close_declarative_links(&self, id: &ProviderId, reason: &str) -> Vec<String> {
        let closed = interaction_adapter_declarative::shutdown_provider_links(id.as_str());
        if !closed.is_empty() {
            tracing::info!(
                provider = %id.as_str(),
                reason,
                links = ?closed,
                "closed declarative device links"
            );
        }
        closed
    }

    /// Revoke: capabilities disabled immediately, state sticks at Revoked.
    pub async fn revoke_provider(&self, id: &ProviderId) -> DomainResult<ProviderDescriptor> {
        let desc = self
            .providers
            .transition(id, ProviderState::Revoked, Some("revoked by user".into()))
            .await?;
        for rid in &desc.receptors {
            let _ = self
                .registry
                .set_receptor_enabled(&ReceptorId::new(rid), false)
                .await;
        }
        for aid in &desc.actuators {
            let _ = self
                .registry
                .set_actuator_enabled(&ActuatorId::new(aid), false)
                .await;
        }
        // 撤銷＝連線也要斷（不只是停止派工）。
        let closed_links = self.close_declarative_links(id, "revoked");
        self.persist_provider(id).await;
        self.character_project_provider(id, ProviderState::Revoked);
        self.store.audit(
            "provider.revoked",
            "user",
            &serde_json::json!({"providerId": id.as_str(), "closedLinks": closed_links}),
        )?;
        Ok(desc)
    }

    async fn persist_provider(&self, id: &ProviderId) {
        if let Ok(desc) = self.providers.get(id).await {
            if desc.identity.trust_level == TrustLevel::Builtin {
                return; // builtin is reconstructed each start
            }
            let desc = self.with_tested(desc);
            if let Ok(body) = serde_json::to_string(&desc) {
                let _ = self.store.save_provider(id.as_str(), &body);
            }
        } else {
            let _ = self.store.delete_provider(id.as_str());
        }
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一般模式會把 note 原樣顯示：不得出現受器／動器／hello／pair-ok 這類技術詞，
    /// 但能力 id 必須保留（可追查），有人話名稱時一起顯示。
    #[test]
    fn tested_note_speaks_human_and_keeps_the_capability_id() {
        let note = tested_note(
            TestedCapability::Receptor,
            "desk-light.status",
            Some("書桌燈狀態"),
            true,
        );
        assert_eq!(
            note,
            "裝置報上身分並完成配對：感知來源 「書桌燈狀態」（desk-light.status） 讀取成功"
        );
        for jargon in ["受器", "動器", "hello", "pair-ok", "握手"] {
            assert!(!note.contains(jargon), "{jargon} leaked into {note}");
        }

        // 沒有人話名稱／名稱等於 id／空白名稱：只寫 id，不寫「」（）。
        let plain = tested_note(TestedCapability::Receptor, "desk-light.status", None, false);
        assert_eq!(plain, "感知來源 desk-light.status 讀取成功");
        assert_eq!(
            tested_note(
                TestedCapability::Receptor,
                "desk-light.status",
                Some("desk-light.status"),
                false
            ),
            plain
        );
        assert_eq!(
            tested_note(
                TestedCapability::Receptor,
                "desk-light.status",
                Some("  "),
                false
            ),
            plain
        );
    }

    /// 動器只證明 acknowledged（誠實階梯：acknowledged ≠ completed），note 必須說清楚。
    #[test]
    fn tested_note_for_actuators_never_claims_completion() {
        let note = tested_note(
            TestedCapability::Actuator,
            "desk-light.set",
            Some("書桌燈"),
            false,
        );
        assert_eq!(
            note,
            "回應方式 「書桌燈」（desk-light.set） 已回覆收到（acknowledged，不代表已完成）"
        );
        assert!(note.contains("acknowledged"));
        assert!(!note.contains("完成。") && note.contains("不代表已完成"));
        let linked = tested_note(TestedCapability::Actuator, "desk-light.set", None, true);
        assert!(linked.starts_with("裝置報上身分並完成配對："));
        assert!(
            linked.ends_with("回應方式 desk-light.set 已回覆收到（acknowledged，不代表已完成）")
        );
    }
}
