//! Provider lifecycle in the runtime: builtin provider registration,
//! declarative adapter loading (config/adapters/*.yaml), pairing, revocation
//! and persistence. Pairing / install / enable / consent stay separate steps.

use crate::runtime::Runtime;
use interaction_core::{
    ActuatorId, DomainError, DomainResult, ProviderDescriptor, ProviderId, ProviderIdentity,
    ProviderKind, ProviderState, ReceptorId, ReceptorMode, Timestamp, TrustLevel,
};
use interaction_registry::providers::{discovered, provider_stopped, ProviderBlock};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// 「已測試」證據（spec §9.3）。掃描到 metadata、設定檔存在、甚至狀態變成
/// Available，都**不等於**測過：這筆記錄只在 runtime 真的觀察到一次成功／
/// 失敗時才寫入，並誠實保留是誰、用什麼方式測的。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTested {
    pub at: Timestamp,
    /// `handshake`＝宣告式裝置連線（serial/mqtt/ble）成功，代表 hello 身分
    /// 驗證＋pair-ok 已完成——但 pair-ok **不一定**代表配對碼被比對過：裝置
    /// 若在 hello 宣告 `pairing=false`（韌體的 PAIRING_CODE 是空的），它會對
    /// 任何碼都回 pair-ok。那種情況由 [`ProviderTested::pairing_unverified`]
    /// 標出來，`how` 的字串不變；`capability`＝該 provider 的受器讀成功／
    /// 動器回 ack；`human`＝使用者按下「測試裝置」。
    pub how: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 這次握手的配對碼從未被任何一方比對過（裝置說它不需要配對）。
    /// true ⇒ 身分證據只有「裝置自報的 deviceId」，證據等級必須誠實降級，
    /// 不得與真的比對過配對碼的裝置顯示成同一階。
    ///
    /// 舊的落地 JSON 沒有這個鍵：`default` 讓它讀回 false（缺席≠未驗證），
    /// `skip_serializing_if` 讓沒有問題的記錄維持原本的形狀（不新增噪音）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pairing_unverified: bool,
}

/// 記錄來源：哪一種能力提供了這次證據。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestedCapability {
    Receptor,
    Actuator,
}

// ---------------------------------------------------------------------------
// Provider 能力宣告（v0.6.0）
//
// 「哪些動器是角色自己的呈現面」「哪些受器是高風險」「這一類來源的人話名稱
// 叫什麼」，過去散在 character.rs／activity.rs／sensors.rs 裡以特定裝置的
// 能力 id 前綴判斷——等於 runtime 核心只理解一種特定裝置。現在改成
// **provider 註冊時自己宣告**：核心只查這張表，新增一種行動裝置不必再改
// 這幾個檔案的 if 分支。
//
// 有界：每個 `declaration_id` 一筆，宣告內容是純資料（沒有 I/O、沒有回呼）。
// ---------------------------------------------------------------------------

/// 能力 id 的比對方式：完全相同，或共用同一個前綴（一個 provider 的整組能力）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySelector {
    Exact(String),
    Prefix(String),
}

impl CapabilitySelector {
    pub fn exact(id: impl Into<String>) -> Self {
        CapabilitySelector::Exact(id.into())
    }

    pub fn prefix(prefix: impl Into<String>) -> Self {
        CapabilitySelector::Prefix(prefix.into())
    }

    pub fn matches(&self, capability_id: &str) -> bool {
        match self {
            CapabilitySelector::Exact(id) => capability_id == id,
            // 空前綴會match全部，那是宣告錯誤而不是「全部都算」：直接不match。
            CapabilitySelector::Prefix(prefix) => {
                !prefix.is_empty() && capability_id.starts_with(prefix.as_str())
            }
        }
    }
}

/// 一個 provider 對自己能力的語意宣告（純資料）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderCapabilityDeclaration {
    /// 宣告來源（provider id 或 provider 家族 id）；同一個 id 再宣告一次＝覆寫。
    pub declaration_id: String,
    /// 這一類來源的人話「種類名」（由 provider 自己給）。注意它不是某一台
    /// 裝置的暱稱：介面只知道「是哪一種來源」而不知道是哪一台時才用它，
    /// 沒有宣告就由呼叫端退回中性字樣。
    pub class_label: Option<String>,
    /// 呈現面 actuator（角色自己）：它們的收據不投影成 `action.*` intent，
    /// 否則角色會對自己的呈現動作再演一次。
    pub presentation_surfaces: Vec<CapabilitySelector>,
    /// 這個 provider 提供的受器 id（種類名查表用）。
    pub receptors: Vec<String>,
    /// 高風險受器：停止結果未知時要誠實補「可能還在擷取」的事件。
    pub high_risk_receptors: Vec<String>,
}

impl ProviderCapabilityDeclaration {
    pub fn new(declaration_id: impl Into<String>) -> Self {
        ProviderCapabilityDeclaration {
            declaration_id: declaration_id.into(),
            ..Default::default()
        }
    }

    pub fn with_class_label(mut self, label: impl Into<String>) -> Self {
        self.class_label = Some(label.into());
        self
    }

    pub fn with_presentation_surface(mut self, selector: CapabilitySelector) -> Self {
        self.presentation_surfaces.push(selector);
        self
    }

    pub fn with_receptor(mut self, receptor_id: impl Into<String>) -> Self {
        self.receptors.push(receptor_id.into());
        self
    }

    pub fn with_receptors<I, S>(mut self, receptor_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.receptors
            .extend(receptor_ids.into_iter().map(Into::into));
        self
    }

    pub fn with_high_risk_receptor(mut self, receptor_id: impl Into<String>) -> Self {
        self.high_risk_receptors.push(receptor_id.into());
        self
    }

    pub fn with_high_risk_receptors<I, S>(mut self, receptor_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.high_risk_receptors
            .extend(receptor_ids.into_iter().map(Into::into));
        self
    }
}

/// 宣告表：同步可讀（投影路徑沒有 await 點可用），寫入只發生在 provider
/// 註冊時。鎖中毒時退回既有內容繼續用（`poisoned.into_inner()`，與本 crate
/// 其他鎖同一個慣例）——純資料表，沒有半寫入的不變量，讓整個 Runtime 崩潰
/// 不會比較誠實；把表當成空的更不誠實（呈現面會被當成一般動器、高風險受器
/// 清單會憑空變空）。
#[derive(Default)]
pub struct ProviderCapabilityRegistry {
    inner: std::sync::RwLock<BTreeMap<String, ProviderCapabilityDeclaration>>,
}

impl std::fmt::Debug for ProviderCapabilityRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderCapabilityRegistry")
            .field("declarations", &self.declaration_ids())
            .finish()
    }
}

impl ProviderCapabilityRegistry {
    /// 登記（或覆寫）一個 provider 的宣告。
    pub fn declare(&self, declaration: ProviderCapabilityDeclaration) {
        self.write()
            .insert(declaration.declaration_id.clone(), declaration);
    }

    /// 中毒＝上一個持鎖者 panic 了，但這張表是純資料（沒有跨欄位不變量會因此半寫入）：
    /// 退回既有內容繼續用，不讓一個無關的 panic 把整個能力語意投影變成空白。
    fn read(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, ProviderCapabilityDeclaration>> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, BTreeMap<String, ProviderCapabilityDeclaration>> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 這個 actuator 是某個 provider 宣告的呈現面嗎？
    pub fn is_presentation_surface(&self, actuator_id: &str) -> bool {
        self.read().values().any(|d| {
            d.presentation_surfaces
                .iter()
                .any(|s| s.matches(actuator_id))
        })
    }

    /// 提供這個受器的來源，它的人話種類名（沒有宣告＝None，呼叫端用中性字樣）。
    pub fn class_label_of_receptor(&self, receptor_id: &str) -> Option<String> {
        self.read()
            .values()
            .find(|d| d.receptors.iter().any(|r| r == receptor_id))
            .and_then(|d| d.class_label.clone())
    }

    /// 所有 provider 宣告的高風險受器（去重、排序固定）。
    pub fn high_risk_receptors(&self) -> Vec<String> {
        let set: std::collections::BTreeSet<String> = self
            .read()
            .values()
            .flat_map(|d| d.high_risk_receptors.iter().cloned())
            .collect();
        set.into_iter().collect()
    }

    /// 目前有哪些宣告（除錯／測試用；順序固定）。
    pub fn declaration_ids(&self) -> Vec<String> {
        self.read().keys().cloned().collect()
    }

    pub fn declaration(&self, declaration_id: &str) -> Option<ProviderCapabilityDeclaration> {
        self.read().get(declaration_id).cloned()
    }
}

/// 桌面角色（Presentation Provider）能力 id 的共同前綴。
pub const COMPANION_CAPABILITY_PREFIX: &str = "companion.";

/// 這個能力 id 屬於桌面角色（呈現面）嗎？
///
/// 前綴的**唯一判斷點**：宣告表（`companion_capability_declaration`）與 provider
/// 能力歸屬（builtin／companion 的切分）都走這裡。各自寫死字面值的話，前綴一改
/// 兩邊就瞬間不一致——能力同時被算進 builtin、又不被視為呈現面。
pub fn is_companion_capability(id: &str) -> bool {
    id.starts_with(COMPANION_CAPABILITY_PREFIX)
}

/// 桌面角色 provider 的宣告：它整組能力都是角色自己的呈現面。
pub fn companion_capability_declaration() -> ProviderCapabilityDeclaration {
    ProviderCapabilityDeclaration::new(crate::character::COMPANION_PROVIDER_ID)
        .with_presentation_surface(CapabilitySelector::prefix(COMPANION_CAPABILITY_PREFIX))
}

/// 自動記錄的節流窗：同樣的結果一分鐘內只寫一次，避免每次讀取都打 DB。
/// 人為測試不節流（使用者按了就要看到最新結果）。
const TESTED_AUTO_THROTTLE_SECS: i64 = 60;

/// 「人類把這個 provider 關掉了」的落地鍵前綴（`provider-off:<providerId>`）。
///
/// 為什麼不直接看 provider 記錄的狀態：重啟時**系統自己**會把 operational 的
/// provider 降成 Disabled（`re-armed on restart requires explicit enable`），
/// 那是系統的降級，不是人類的決定；只看狀態會把兩者混為一談，讓每次重啟都變成
/// 永久停用。這個鍵只在人類真的按下停用／撤銷時寫入，重新啟用時清掉。
const PROVIDER_OFF_META_PREFIX: &str = "provider-off:";

/// 重啟降級（系統做的，不是人類的決定）留下的註記。用來把它跟「人類真的
/// 按了停用」分開，見升級邊界的判斷。
const RE_ARM_NOTE: &str = "re-armed on restart requires explicit enable";

fn provider_off_key(id: &ProviderId) -> String {
    format!("{PROVIDER_OFF_META_PREFIX}{}", id.as_str())
}

/// 「已測試」證據的人話註記。UI（一般模式）會把它原樣顯示，所以這裡不用
/// 受器／動器／hello／pair-ok 這類技術詞：感知來源＝receptor、回應方式＝actuator；
/// 有實體連線（serial/mqtt/ble）的裝置多加一句「裝置報上身分並完成配對」。
/// 能力 id 一律保留（是可追查的事實），有人話名稱時放在前面。
/// 動器只證明「已回覆收到（acknowledged）」——誠實階梯：acknowledged ≠ completed。
/// `pairing_unverified`＝這次握手的配對碼從未被比對過（裝置說它不需要配對）：
/// 那一句「完成配對」不得出現，改說身分證據只有裝置自報的 deviceId。
pub fn tested_note(
    kind: TestedCapability,
    capability_id: &str,
    human_name: Option<&str>,
    linked: bool,
    pairing_unverified: bool,
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
    if pairing_unverified {
        // 「裝置宣稱不需配對」≠「配對已驗證」：只有 DeviceLink 真的比對過碼
        // 才算。這句話取代「完成配對」，不論這個 provider 有沒有實體連線。
        format!(
            "裝置報上身分，但這次握手無法證明配對碼被比對過（裝置說它不需要配對），身分證據僅為裝置自報的 deviceId：{what}"
        )
    } else if linked {
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
    /// 執行期閘門：擁有這個受器的 provider 是否已被停下來（停用／撤銷／到期／
    /// 關閉）。沒有 provider 記錄的能力（內建、動態註冊）回 `None`。
    pub(crate) fn receptor_provider_block(&self, receptor_id: &str) -> Option<ProviderBlock> {
        self.providers.gate().receptor_block(receptor_id)
    }

    /// 動器版本的同一個閘門。
    pub(crate) fn actuator_provider_block(&self, actuator_id: &str) -> Option<ProviderBlock> {
        self.providers.gate().actuator_block(actuator_id)
    }

    /// 觀察路徑用：provider 被停下來時回 `Unavailable`，形狀比照 registry 的
    /// 「這個受器被停用了」。
    pub(crate) fn receptor_provider_gate(&self, receptor_id: &str) -> DomainResult<()> {
        match self.receptor_provider_block(receptor_id) {
            Some(block) => Err(DomainError::Unavailable(
                block.reason(&format!("receptor {receptor_id}")),
            )),
            None => Ok(()),
        }
    }

    /// Provider 能力宣告表（同步可讀）。character／activity／sensors 只查這張
    /// 表，不比對任何具名裝置的能力字面值。
    ///
    /// 存放位置說明：目前掛在 `CharacterHub` 上，因為它是本工作流可動的檔案
    /// 裡唯一同步可達的 runtime 級容器（投影路徑沒有 await 點可用）。語意上
    /// 它屬於 provider 層，日後 `RuntimeInner` 進入可改範圍時應搬過去。
    pub fn capability_declarations(&self) -> &ProviderCapabilityRegistry {
        self.character.capability_declarations()
    }

    /// 登記一個 provider 對自己能力的語意宣告（呈現面／高風險受器／種類名）。
    pub fn declare_provider_capabilities(&self, declaration: ProviderCapabilityDeclaration) {
        self.capability_declarations().declare(declaration);
    }

    pub(crate) async fn init_providers(&self) {
        // 能力清單的 availability 投影要看得到 provider 狀態：被停用／撤銷的
        // provider 底下的能力不得繼續宣稱 Available。
        self.registry.attach_provider_gate(self.providers.gate());

        // 0) 能力語意宣告：呈現面、高風險受器、人話種類名一律由 provider 自己
        //    說明。這一步跟「伺服器有沒有起來」「有沒有配對過裝置」無關——
        //    核心對能力 id 的理解不能依賴某個 provider 剛好在線上。
        self.declare_provider_capabilities(companion_capability_declaration());
        self.declare_provider_capabilities(crate::mobile::mobile_capability_declaration());

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
                .filter(|id| !is_companion_capability(id))
                .collect(),
            actuators: self
                .registry
                .actuator_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| !is_companion_capability(id))
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
                .filter(|id| is_companion_capability(id))
                .collect(),
            actuators: self
                .registry
                .actuator_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| is_companion_capability(id))
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
                        desc.detail = Some(RE_ARM_NOTE.into());
                    }
                    let id = desc.identity.id.clone();
                    // 升級邊界：舊版寫下的「已停用／已撤銷」記錄還沒有
                    // provider-off 記號（那個記號是後來才加的）。第一次重啟
                    // 一律採安全預設——不重開連線、能力維持關閉——並留一則
                    // audit 請使用者確認，而不是默默把裝置打開。
                    // 系統自己在重啟時做的降級（上面那一段）不算人類的決定，
                    // 用它留下的註記辨識，避免每次重啟都變成永久停用。
                    if !downgraded
                        && provider_stopped(desc.state)
                        && desc.detail.as_deref() != Some(RE_ARM_NOTE)
                        && self.provider_off_reason(&id).is_none()
                    {
                        let label = format!("{:?}", desc.state).to_lowercase();
                        let reason = format!("legacy-{label}");
                        self.mark_provider_off(&id, &reason);
                        self.store
                            .audit(
                                "provider.legacy-off-assumed",
                                "runtime",
                                &json!({
                                    "providerId": id.as_str(),
                                    "state": desc.state,
                                    "reason": "no provider-off marker from an older version",
                                    "action": "kept off until a human confirms",
                                }),
                            )
                            .ok();
                    }
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
        // 人類按下的「停用／撤銷」必須跨重啟：spec 每次啟動都會重新載入，
        // 但被停用／撤銷的裝置不得因此把實體連線重新開回來、也不得讓受器
        // 回到啟用（重啟不是重新授權）。決定記在 store，重啟後仍在。
        let kept_off = self.provider_off_reason(&provider_id);
        let built = interaction_adapter_declarative::build(spec, Some(self.paths.home.clone()))
            .map_err(DomainError::Validation)?;
        // 記住這個 provider 開出來的實體連線（serial/mqtt/ble），
        // disable／revoke 時才關得掉——停用的 provider 不得繼續佔著埠或
        // broker 連線做無盡重連。
        interaction_adapter_declarative::register_provider_links(
            provider_id.as_str(),
            &built.links,
        );
        if let Some(reason) = &kept_off {
            // build() 會 spawn serial/mqtt/ble 連線：在註冊能力之前就立刻關掉，
            // 停用中的裝置不得在背景重連（重啟不得把人類關掉的東西打開）。
            let closed = self.close_declarative_links(&provider_id, reason);
            tracing::info!(
                provider = %provider_id.as_str(),
                reason,
                links = ?closed,
                "declarative device stays off across restart (a human disabled/revoked it)"
            );
        }
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
        if let Some(reason) = &kept_off {
            // 能力重新註冊時 registry 會用 manifest 的預設值決定啟用與否
            // （不需 consent 的受器預設啟用）——被停用／撤銷的裝置必須立刻
            // 回到 disabled，否則「重啟」就成了繞過人類決定的後門。
            for rid in &receptor_ids {
                let _ = self
                    .registry
                    .set_receptor_enabled(&ReceptorId::new(rid), false)
                    .await;
            }
            for aid in &actuator_ids {
                let _ = self
                    .registry
                    .set_actuator_enabled(&ActuatorId::new(aid), false)
                    .await;
            }
            self.store
                .audit(
                    "provider.kept-off-at-start",
                    "runtime",
                    &json!({
                        "providerId": provider_id.as_str(),
                        "reason": reason,
                        "receptors": receptor_ids,
                        "actuators": actuator_ids,
                    }),
                )
                .ok();
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
    ///
    /// `pairing_unverified`：`Some(_)`＝這次的證據來源知道配對驗證狀態
    /// （裝置握手）；`None`＝這個來源根本不知道（人為測試、受器讀取）。
    /// 不知道的來源**不得**把先前握手記下的「配對碼未經比對」洗成乾淨記錄
    /// ——沉默的降級跟謊稱已驗證是同一件事。
    pub(crate) async fn record_provider_tested(
        &self,
        id: &ProviderId,
        how: &str,
        ok: bool,
        note: impl Into<String>,
        pairing_unverified: Option<bool>,
    ) -> ProviderTested {
        let mut record = ProviderTested {
            at: chrono::Utc::now(),
            how: how.to_string(),
            ok,
            note: Some(note.into()),
            pairing_unverified: pairing_unverified.unwrap_or(false),
        };
        {
            let Ok(mut map) = self.provider_tested.lock() else {
                return record;
            };
            let previous = map.get(id.as_str()).cloned();
            if pairing_unverified.is_none() {
                record.pairing_unverified = previous.as_ref().is_some_and(|p| p.pairing_unverified);
            }
            if how != "human" {
                if let Some(previous) = previous.as_ref() {
                    // 配對驗證狀態變了就不算「同樣的結果」：節流不得把
                    // 「這次的碼沒被比對過」壓成上一分鐘的乾淨記錄（反之亦然）。
                    let same = previous.ok == ok
                        && previous.how == record.how
                        && previous.pairing_unverified == record.pairing_unverified;
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
        self.note_capability_tested_on(kind, capability_id, None, None)
            .await
    }

    /// 同上，但由呼叫端指名「真正執行的那一台裝置」（driver 回報的 deviceId）。
    /// 同一個能力 id 可能同時屬於多台裝置（每台各有自己的 provider 記錄），
    /// 這時只有 driver 說的那一台才是事實——指名了卻找不到對應的 provider 時
    /// 什麼都不記，絕不把證據掛到別台身上。
    ///
    /// `pairing_unverified` 來自 driver 收據（`driver_response.pairingUnverified`）：
    /// 裝置在 hello 說它不需要配對時，spec 配的那組碼從未被任何一方比對過。
    /// 這件事只有單次握手知道，必須跟著證據一起記下來，否則 provider 的
    /// 證據等級會把它演成與真配對無法區分的「已測試」。
    pub(crate) async fn note_capability_tested_on(
        &self,
        kind: TestedCapability,
        capability_id: &str,
        device_id: Option<&str>,
        pairing_unverified: Option<bool>,
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
        // 這次的來源不知道配對驗證狀態（受器讀取）就沿用既有記錄：人話註記
        // 與旗標必須一致，才不會出現「文案說完成配對、旗標說沒驗證」。
        let pairing_unverified = pairing_unverified.unwrap_or_else(|| {
            self.provider_tested_record(id.as_str())
                .is_some_and(|previous| previous.pairing_unverified)
        });
        let note = tested_note(
            kind,
            capability_id,
            human_name.as_deref(),
            linked,
            pairing_unverified,
        );
        self.record_provider_tested(&id, how, true, note, Some(pairing_unverified))
            .await;
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
        // 指名了裝置：只認那一台的 provider。id 的組法由 provider 自己提供
        // （`crate::mobile::mobile_provider_id`），這裡不重寫命名規則。
        // 找不到、或那台 provider 根本沒有這個能力 → 什麼都不記；把證據記到
        // 另一台身上比沒有證據更糟（使用者會以為那台測過了）。
        if let Some(device_id) = device_id.map(str::trim).filter(|id| !id.is_empty()) {
            let pid = crate::mobile::mobile_provider_id(device_id);
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
                .record_provider_tested(id, "human", false, reason.clone(), None)
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
                let tested = self
                    .record_provider_tested(id, "human", true, note, None)
                    .await;
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
                        None,
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
        // 停用類的決定是人類做的，必須跨重啟：否則重開 daemon 就會把 serial 埠／
        // broker 連線重新開回來、受器也回到啟用（＝停用只在這次執行有效）。
        // 重新啟用（或其他狀態）就把記號清掉，裝置下一次啟動才回得來。
        if matches!(
            state,
            ProviderState::Disabled
                | ProviderState::Closed
                | ProviderState::Expired
                | ProviderState::Revoked
        ) {
            self.mark_provider_off(id, &format!("{state:?}").to_lowercase());
        } else {
            self.clear_provider_off(id);
        }
        self.persist_provider(id).await;
        // Character Protocol §11：available → greet、disconnected → notice。
        self.character_project_provider(id, state);
        Ok(desc)
    }

    /// 記下「人類把這個 provider 關掉了」（跨重啟有效）。落地失敗只記警告：
    /// 這一次的停用仍然生效（連線已關），但要誠實留下痕跡說重啟後可能復活。
    fn mark_provider_off(&self, id: &ProviderId, reason: &str) {
        if let Err(e) = self.store.set_meta(&provider_off_key(id), reason) {
            tracing::warn!(
                provider = %id.as_str(),
                error = %e,
                "could not persist the disable decision — it may not survive a restart"
            );
            self.store
                .audit(
                    "provider.off-not-persisted",
                    "runtime",
                    &json!({"providerId": id.as_str(), "reason": reason, "error": e.to_string()}),
                )
                .ok();
        }
    }

    /// 人類重新啟用 → 清掉「關掉了」的記號（空字串＝沒有記號）。
    fn clear_provider_off(&self, id: &ProviderId) {
        let _ = self.store.set_meta(&provider_off_key(id), "");
    }

    /// 這個 provider 是否被人類停用／撤銷過（重啟後仍要遵守）。
    fn provider_off_reason(&self, id: &ProviderId) -> Option<String> {
        self.store
            .get_meta(&provider_off_key(id))
            .ok()
            .flatten()
            .filter(|reason| !reason.is_empty())
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
        // 撤銷＝連線也要斷（不只是停止派工），而且必須跨重啟：重啟後 spec 重新
        // 載入時不得把連線開回來、也不得讓受器回到啟用。
        let closed_links = self.close_declarative_links(id, "revoked");
        self.mark_provider_off(id, "revoked");
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

    /// 呈現面前綴只能有**一個**產生點。`COMPANION_CAPABILITY_PREFIX` 之外還有
    /// 同樣的字面前綴另外被寫死一次時，前綴一改，宣告表（呈現面選擇器）與 provider
    /// 能力歸屬（builtin／companion 切分）就會瞬間不一致，而且沒有任何測試會擋。
    ///
    /// 這裡直接掃自己的原始碼：常數定義那一行是唯一允許出現的字面值。
    #[test]
    fn the_companion_prefix_has_exactly_one_literal_in_this_file() {
        let source = include_str!("providers.rs");
        let literal = format!("\"{COMPANION_CAPABILITY_PREFIX}\"");
        let hits = source.matches(literal.as_str()).count();
        assert_eq!(
            hits, 1,
            "呈現面前綴的字面值只能出現在常數定義那一行（找到 {hits} 處）；\
             其他地方請改用 is_companion_capability()／COMPANION_CAPABILITY_PREFIX"
        );
    }

    /// 鎖中毒時**退回既有內容繼續用**（宣告表的註解說的就是這件事）。
    /// 舊實作在中毒時 `read().ok()` 回 `None`、`write().ok()` 直接丟棄寫入：
    /// 呈現面會被當成一般動器（角色對自己的動作再演一次）、高風險受器清單變空
    /// （停止結果未知時不再補「可能還在擷取」）、種類名退回中性字樣。註解與實作
    /// 不一致比兩者都錯更危險，因為沒有人會去查。
    #[test]
    fn a_poisoned_registry_still_serves_and_accepts_declarations() {
        let registry = std::sync::Arc::new(ProviderCapabilityRegistry::default());
        registry.declare(
            ProviderCapabilityDeclaration::new("provider.test")
                .with_class_label("測試來源")
                .with_receptor("test.receptor")
                .with_high_risk_receptor("test.mic")
                .with_presentation_surface(CapabilitySelector::prefix("test.")),
        );

        // 持鎖時 panic → 鎖中毒（panic 訊息由測試框架印出，是預期的）。
        let poisoner = registry.clone();
        let joined = std::thread::spawn(move || {
            let _guard = poisoner.inner.write().expect("write lock");
            panic!("deliberate poison");
        })
        .join();
        assert!(joined.is_err(), "製造中毒的執行緒必須真的 panic");
        assert!(registry.inner.is_poisoned(), "鎖必須真的中毒");

        assert_eq!(
            registry.declaration_ids(),
            vec!["provider.test".to_string()]
        );
        assert_eq!(
            registry.class_label_of_receptor("test.receptor").as_deref(),
            Some("測試來源")
        );
        assert_eq!(registry.high_risk_receptors(), vec!["test.mic".to_string()]);
        assert!(registry.is_presentation_surface("test.overlay"));
        assert!(registry.declaration("provider.test").is_some());

        // 中毒之後的寫入也不得被靜默丟棄。
        registry.declare(ProviderCapabilityDeclaration::new("provider.later"));
        assert!(registry.declaration("provider.later").is_some());
    }

    /// 一般模式會把 note 原樣顯示：不得出現受器／動器／hello／pair-ok 這類技術詞，
    /// 但能力 id 必須保留（可追查），有人話名稱時一起顯示。
    #[test]
    fn tested_note_speaks_human_and_keeps_the_capability_id() {
        let note = tested_note(
            TestedCapability::Receptor,
            "desk-light.status",
            Some("書桌燈狀態"),
            true,
            false,
        );
        assert_eq!(
            note,
            "裝置報上身分並完成配對：感知來源 「書桌燈狀態」（desk-light.status） 讀取成功"
        );
        for jargon in ["受器", "動器", "hello", "pair-ok", "握手"] {
            assert!(!note.contains(jargon), "{jargon} leaked into {note}");
        }

        // 沒有人話名稱／名稱等於 id／空白名稱：只寫 id，不寫「」（）。
        let plain = tested_note(
            TestedCapability::Receptor,
            "desk-light.status",
            None,
            false,
            false,
        );
        assert_eq!(plain, "感知來源 desk-light.status 讀取成功");
        assert_eq!(
            tested_note(
                TestedCapability::Receptor,
                "desk-light.status",
                Some("desk-light.status"),
                false,
                false
            ),
            plain
        );
        assert_eq!(
            tested_note(
                TestedCapability::Receptor,
                "desk-light.status",
                Some("  "),
                false,
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
            false,
        );
        assert_eq!(
            note,
            "回應方式 「書桌燈」（desk-light.set） 已回覆收到（acknowledged，不代表已完成）"
        );
        assert!(note.contains("acknowledged"));
        assert!(!note.contains("完成。") && note.contains("不代表已完成"));
        let linked = tested_note(
            TestedCapability::Actuator,
            "desk-light.set",
            None,
            true,
            false,
        );
        assert!(linked.starts_with("裝置報上身分並完成配對："));
        assert!(
            linked.ends_with("回應方式 desk-light.set 已回覆收到（acknowledged，不代表已完成）")
        );
    }

    /// protocol-conformance-030：裝置說它不需要配對時，spec 配的那組碼從未被
    /// 任何一方比對過。人話註記不得沿用「完成配對」——那是這個缺陷最會誤導
    /// 人的一句（有實體連線的裝置本來就會走到 `linked = true`）。
    #[test]
    fn tested_note_never_claims_a_pairing_that_was_never_compared() {
        let unverified = tested_note(
            TestedCapability::Actuator,
            "esp32-desk.vibe",
            None,
            true,
            true,
        );
        assert!(
            unverified
                .starts_with("裝置報上身分，但這次握手無法證明配對碼被比對過（裝置說它不需要配對），身分證據僅為裝置自報的 deviceId："),
            "{unverified}"
        );
        assert!(!unverified.contains("完成配對"), "{unverified}");
        assert!(
            unverified
                .ends_with("回應方式 esp32-desk.vibe 已回覆收到（acknowledged，不代表已完成）"),
            "{unverified}"
        );
        // 沒有實體連線登記（linked=false）時同樣要說出來：旗標是單次握手的
        // 事實，不是 provider 層級的 device_link 集合說了算。
        assert_eq!(
            tested_note(
                TestedCapability::Actuator,
                "esp32-desk.vibe",
                None,
                false,
                true
            ),
            unverified
        );
    }
}
