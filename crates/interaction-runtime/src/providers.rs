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

/// 唯讀投影用的比對方式描述：`exact`（單一能力）還是 `prefix`（整族能力）。
/// 兩者在維運上意義完全不同（一個前綴宣告會把之後新增的能力也算進去），
/// 所以投影必須看得出來，不能都攤平成一串字串。
fn selector_json(selector: &CapabilitySelector) -> Value {
    match selector {
        CapabilitySelector::Exact(id) => json!({"match": "exact", "value": id}),
        CapabilitySelector::Prefix(prefix) => json!({"match": "prefix", "value": prefix}),
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

/// 宣告表的**唯讀視角**：核心的讀者（character／activity／sensors）只能經由
/// 它查詢。
///
/// 為什麼要有這個 trait：`&ProviderCapabilityRegistry` 在型別上同時給了寫入
/// 能力——任何拿到它的投影路徑都能偷偷改寫「哪些動器是呈現面」「哪些受器是
/// 高風險」。能力語意的寫入權必須留在 provider 註冊路徑
/// （[`Runtime::declare_provider_capabilities`]／
/// [`Runtime::retract_provider_capabilities`]），讀者拿不到才叫確定性強制，
/// 不是靠慣例。
///
/// 這是同一份表的借用視角，不是第二份複本：沒有快照、沒有同步問題。
pub trait CapabilityDeclarationsView {
    /// 這個 actuator 是某個 provider 宣告的呈現面嗎？
    fn is_presentation_surface(&self, actuator_id: &str) -> bool;
    /// 提供這個受器的來源，它的人話種類名（沒有宣告＝None，呼叫端用中性字樣）。
    fn class_label_of_receptor(&self, receptor_id: &str) -> Option<String>;
    /// 所有 provider 宣告的高風險受器（去重、排序固定）。
    fn high_risk_receptors(&self) -> Vec<String>;
    /// 目前有哪些宣告（除錯／維運／測試用；順序固定）。
    fn declaration_ids(&self) -> Vec<String>;
    /// 取出一整筆宣告（唯讀投影用）。
    fn declaration(&self, declaration_id: &str) -> Option<ProviderCapabilityDeclaration>;
}

impl ProviderCapabilityRegistry {
    /// 登記（或覆寫）一個 provider 的宣告。整份覆寫：同一個 `declaration_id`
    /// 再宣告一次，舊的呈現面／高風險受器／種類名一律消失（宣告是那個
    /// provider 現在的完整事實，不是累加的補丁）。
    ///
    /// `pub(crate)`：唯一的對外入口是 [`Runtime::declare_provider_capabilities`]。
    pub(crate) fn declare(&self, declaration: ProviderCapabilityDeclaration) {
        self.write()
            .insert(declaration.declaration_id.clone(), declaration);
    }

    /// 移除一整筆宣告（provider 家族不再存在）。回傳是否真的移除了某一筆。
    ///
    /// `pub(crate)`：唯一的對外入口是 [`Runtime::retract_provider_capabilities`]。
    pub(crate) fn retract(&self, declaration_id: &str) -> bool {
        self.write().remove(declaration_id).is_some()
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
}

impl CapabilityDeclarationsView for ProviderCapabilityRegistry {
    fn is_presentation_surface(&self, actuator_id: &str) -> bool {
        self.read().values().any(|d| {
            d.presentation_surfaces
                .iter()
                .any(|s| s.matches(actuator_id))
        })
    }

    fn class_label_of_receptor(&self, receptor_id: &str) -> Option<String> {
        self.read()
            .values()
            .find(|d| d.receptors.iter().any(|r| r == receptor_id))
            .and_then(|d| d.class_label.clone())
    }

    fn high_risk_receptors(&self) -> Vec<String> {
        let set: std::collections::BTreeSet<String> = self
            .read()
            .values()
            .flat_map(|d| d.high_risk_receptors.iter().cloned())
            .collect();
        set.into_iter().collect()
    }

    fn declaration_ids(&self) -> Vec<String> {
        self.read().keys().cloned().collect()
    }

    fn declaration(&self, declaration_id: &str) -> Option<ProviderCapabilityDeclaration> {
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

    /// Provider 能力宣告表（同步可讀的**唯讀**視角）。character／activity／
    /// sensors 只查這張表，不比對任何具名裝置的能力字面值。
    ///
    /// 擁有者是 `RuntimeInner`（provider 層），不是 `CharacterHub`：角色呈現層
    /// 沒有能力語意的主權。
    pub fn capability_declarations(&self) -> &dyn CapabilityDeclarationsView {
        // 欄位存取（經 Deref 到 `RuntimeInner`）——同名方法與欄位分屬不同命名空間。
        &self.capability_declarations
    }

    /// 登記一個 provider 對自己能力的語意宣告（呈現面／高風險受器／種類名）。
    ///
    /// 生命週期：**register**（第一次宣告）與 **update**（同一個
    /// `declaration_id` 再宣告一次）走同一個入口，語意是整份覆寫——舊的呈現面
    /// 與高風險受器會消失，不會留下上一版的殘影。
    ///
    /// 刻意**不**與單台裝置的 enable／disable／revoke 連動：宣告說的是「這一族
    /// provider 的能力是什麼意思」，撤銷一支手機不會讓 `iphone.mic-level` 從此
    /// 不再是高風險受器。移除宣告是 [`Runtime::retract_provider_capabilities`]
    /// 的事（整族不再存在時才做）。
    pub fn declare_provider_capabilities(&self, declaration: ProviderCapabilityDeclaration) {
        self.capability_declarations.declare(declaration);
    }

    /// 移除一整族 provider 的能力宣告（**remove**）。回傳是否真的有一筆被移除。
    ///
    /// 用在整族來源不再存在的時候（例如宣告式 adapter 的 spec 被刪掉）。移除
    /// 之後，這一族宣告過的呈現面與高風險受器就不再出現在任何查詢裡——核心
    /// 不得繼續替一個已經不存在的來源保留語意。
    ///
    /// 這**不是**安全開關：它只改「這個能力 id 是什麼意思」，不改任何受器／
    /// 動器的啟用旗標，也不會把已經關掉的東西打開。
    pub fn retract_provider_capabilities(&self, declaration_id: &str) -> bool {
        self.capability_declarations.retract(declaration_id)
    }

    /// 宣告表的**唯讀投影**（維運／診斷用）。
    ///
    /// 為什麼需要它：這張表決定了核心怎麼理解每個能力 id（哪些是角色自己的呈
    /// 現面、哪些受器是高風險、感測事件標題用什麼種類名），但它過去只存在於
    /// 記憶體裡，沒有任何介面看得到。出問題時無從分辨「provider 根本沒宣告」
    /// 與「核心讀錯了」——那是一種可觀測性上的不誠實。
    ///
    /// 純唯讀：這條路徑不提供任何寫入或撤回的入口。
    pub fn capability_declarations_report(&self) -> Value {
        let declarations: Vec<Value> = self
            .capability_declarations
            .declaration_ids()
            .into_iter()
            .filter_map(|id| self.capability_declarations.declaration(&id))
            .map(|d| {
                json!({
                    "id": d.declaration_id,
                    "classLabel": d.class_label,
                    "presentationSurfaces": d
                        .presentation_surfaces
                        .iter()
                        .map(selector_json)
                        .collect::<Vec<_>>(),
                    "receptors": d.receptors,
                    "highRiskReceptors": d.high_risk_receptors,
                })
            })
            .collect();
        json!({ "declarations": declarations })
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

        // 0.5) 感測來源登記：停止感測只有**一個**協調器
        //      （`Runtime::stop_all_sensor_sources`），本機麥克風與行動裝置都
        //      只是登記進來的來源之一。本機不再是特例——特例會漂移（緊急停止
        //      那條路徑就曾經自己手刻一份報告）。
        let weak = self.weak_inner();
        let _ = self
            .register_sensor_source(std::sync::Arc::new(
                crate::sensors::LocalMicSensorSource::new(weak.clone()),
            ))
            .await;
        let _ = self
            .register_sensor_source(std::sync::Arc::new(crate::mobile::MobileSensorSource::new(
                weak,
            )))
            .await;

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
        let mut built = interaction_adapter_declarative::build(spec, Some(self.paths.home.clone()))
            .map_err(DomainError::Validation)?;
        // AIP 通道（每條裝置線一條）。型別抹除：核心不認得 serial／mqtt／ble，
        // 只看到「一條說得出身分、能收發 aip、能 stop-all 的通道」。
        let aip_channels = std::mem::take(&mut built.aip_channels);
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
        // 能力語意宣告（M2 §3.2）：這一族有哪些受器、人話種類名、以及**哪些是
        // 高風險受器**（spec 自己標 requiresConsent 的那些）。沒有這一步，停止
        // 感測的協調器對這台裝置一無所知，它的受器只會落成 `no-stop-path`。
        let (declaration, high_risk) = crate::declarative_session::declaration_for_spec(
            spec,
            provider_id.as_str(),
            &receptor_ids,
        );
        self.declare_provider_capabilities(declaration);
        // 感測停止路徑＋Character Session 綁定。停用／撤銷中的 provider 不開
        // 綁定 task（不得在背景重連握手），但仍要有人回答「它在擷取嗎」。
        self.bind_declarative_device(
            provider_id.as_str(),
            spec.display_name.as_deref().unwrap_or(&spec.id),
            aip_channels.clone(),
            high_risk,
            kept_off.is_some(),
        )
        .await;
        // 綁定成立：記住 spec 與這一次開出來的通道，之後「停用→啟用」才有
        // 東西可以重新綁定（不必重新啟動 daemon）。
        self.note_declarative_bound(provider_id.as_str(), spec, &aip_channels);
        if let Some(reason) = &kept_off {
            // 人類把它關掉了：綁定其實沒有成立（沒有連線 task）。誠實記成
            // 已拆掉，並保留原因——撤銷永遠不會被 rebind 復活。
            self.note_declarative_unbound(
                provider_id.as_str(),
                if reason == "revoked" {
                    crate::declarative_lifecycle::UnboundReason::Revoked
                } else {
                    crate::declarative_lifecycle::UnboundReason::Disabled
                },
            );
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
        // 對同一台 provider 的一次完整決定不可分割：兩個並行的停用會在 await
        // 點交錯，把同一台裝置下架兩次（重複稽核、重複翻旗標），而停用與重新
        // 綁定交錯時，剛建好的新連線甚至會被上一個決定漏掉而繼續活著。
        let _serialized = self.providers.lock_provider(id).await;
        // 狀態換了，設定檔警告還在：`transition` 會整個覆寫 detail，所以警告
        // 必須自己帶過去，否則按一次「啟用」就會把明文憑證的提醒洗掉。
        // 狀態註記（例如 re-arm 的說明）本來就只屬於前一個狀態，不帶。
        let previous = self.providers.get(id).await.ok();
        let previous_state = previous.as_ref().map(|d| d.state);
        let mut warnings = provider_detail_warnings(previous.and_then(|d| d.detail).as_deref());
        // 上一次重新綁定留下的提醒（進行中／失敗／不支援）屬於「上一個狀態」：
        // 人類再按一次啟用時不得把舊訊息一路帶著走——這一輪失敗時它會重新出現。
        warnings.retain(|w| !is_rebind_note(w));
        let desc = self
            .providers
            .transition(id, state, merge_provider_detail(None, None, &warnings))
            .await?;
        // v0.5.1 已知限制 #4：停下來的 provider 底下的受器／動器，enabled 旗標
        // 必須真的翻掉，不能只靠 `ProviderGate` 攔派工。旗標是「這個能力現在
        // 開著嗎」的單一事實：狀態列、能力清單、`stop_all_sensors` 的「仍然
        // 啟用」判斷都讀它。只攔不翻＝使用者停用了裝置，卻還看到一支「啟用中」
        // 的麥克風。比照 `revoke_provider` 的做法。
        if provider_stopped(state) {
            // X2：旗標翻掉還不夠——這個 provider 底下若有登記的感測來源，必須
            // 走同一條 request_stop 請它停止擷取（指名這一台），並把結果留痕。
            // 只靠背景 watcher 撞事件補送是競態，不是保證。
            //
            // 順序：**先問來源、再翻旗標**。旗標不是「裝置有沒有在擷取」的事實，
            // 只是「本機這一側收不收資料」；先翻掉旗標的話，來源會以為本來就沒有
            // 東西在擷取（回 already-stopped），於是既不會真的請裝置停下來，
            // 那一筆「可能還在擷取」也不會留下（＝感測靜默）。
            // 綁定的生命週期先寫成「已拆掉，原因是人類停用」：`retire()` 之後
            // 才寫的話，那條路徑只知道一個中性的 reason 字串，說不出是誰決定的。
            self.note_declarative_unbound(
                id.as_str(),
                crate::declarative_lifecycle::unbound_reason_for_state(state),
            );
            // 在翻旗標**之前**記下「哪些受器人類已經自己關了」。之後
            // `disable_provider_capabilities` 會把剩下的也關掉，那一刻起兩者
            // 就分不出來了——而免重啟重新綁定（第 7 步重新註冊整份 spec）會把
            // 不需 consent 的受器帶回預設的「開」。沒有這份紀錄，一次停用再
            // 啟用就等於替使用者按下了一個他沒有按的開關。
            // 只記「預設開著、但現在是關的」——需要 consent 的受器本來就是關的，
            // 把它算成人類的決定是替使用者編造一個他沒下過的決定。
            let mut human_disabled = Vec::new();
            for rid in &desc.receptors {
                if self.registry.receptor_flags(&ReceptorId::new(rid)).await == Some((false, true))
                {
                    human_disabled.push(rid.clone());
                }
            }
            self.note_declarative_human_disabled(id.as_str(), human_disabled);
            let sensor_stop = self
                .stop_provider_sensing(id.as_str(), crate::sensors::SENSOR_STOP_REASON_PROVIDER_OFF)
                .await;
            // 連線在**問完之後**才關：先關就等於親手拆掉唯一能問「你停了嗎」
            // 的那條線，然後永遠只能回答「未知」。停用仍然一定要真的把連線
            // 關掉——停用的 provider 不得繼續佔著埠／broker 連線做無盡重連。
            let closed_links = self.close_declarative_links(id, "disabled");
            self.disable_provider_capabilities(&desc, &format!("{state:?}").to_lowercase())
                .await;
            let _ = self.store.audit(
                "provider.transitioned",
                "user",
                &serde_json::json!({
                    "providerId": id.as_str(),
                    "state": format!("{state:?}").to_lowercase(),
                    "closedLinks": closed_links,
                    "sensorStop": sensor_stop,
                }),
            );
        }
        // 反方向刻意不做：回到 Available／Busy／… 時**不**自動把能力打開。
        // （宣告式裝置的重新綁定是另一回事：那是把整份 spec 重新註冊一次，
        //   能力回到「剛啟動時」的預設——需要 consent 的受器仍然是關的。）
        // registry 的旗標只有布林、沒有出處，runtime 分不出「是我剛才關的」還是
        // 「人類早就自己關掉的這一支」；自動打開會默默推翻人類的決定，對高風險
        // 受器更直接違反「高風險能力不自動恢復」的不變量（同
        // `mobile_disable_high_risk_receptors`：重連後要人類重新啟用）。
        // 重新啟用 provider 只是讓它「可以被啟用」，逐項啟用仍是人的動作。
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
        // 宣告式裝置：把 state 翻回一個「活著」的狀態只代表**允許再試一次**。
        // 能力宣告與實體連線由一條有界的背景 rebind 任務重新建立；在它握手
        // 成功之前，狀態誠實地留在 `Disconnected`（尚未連上），不是 Available。
        if !provider_stopped(state) {
            if let Some(generation) = self.begin_declarative_rebind(id.as_str()) {
                return self.enter_declarative_rebinding(id, generation).await;
            }
            // 宣告式裝置，但綁定表滿的時候它沒被登記：免重啟重新綁定對它不成立。
            // 停用時 `retire()` 已經把能力撤回了，所以這一刻回一個乾淨的
            // `Available` 等於說「可用」，而它一個能力都沒有。
            if previous_state.is_some_and(provider_stopped)
                && self.declarative_untracked(id.as_str())
            {
                return self.note_declarative_not_rebindable(id).await;
            }
        }
        Ok(desc)
    }

    /// 「可以再連上，但能力要等重新啟動才會回來」：狀態誠實地退到
    /// `Disconnected`（沒有連上、也沒有能力），detail 說得出原因，並留稽核。
    async fn note_declarative_not_rebindable(
        &self,
        id: &ProviderId,
    ) -> DomainResult<ProviderDescriptor> {
        let mut warnings = provider_detail_warnings(
            self.providers
                .get(id)
                .await
                .ok()
                .and_then(|d| d.detail)
                .as_deref(),
        );
        if !warnings.iter().any(|w| w == REBIND_UNTRACKED_WARNING) {
            warnings.push(REBIND_UNTRACKED_WARNING.to_string());
        }
        let others: Vec<String> = warnings
            .iter()
            .filter(|w| *w != REBIND_UNTRACKED_WARNING)
            .cloned()
            .collect();
        let note = if others.is_empty() {
            REBIND_UNTRACKED_NOTE.to_string()
        } else {
            format!("{} {REBIND_UNTRACKED_NOTE}", warning_summary(&others))
        };
        let _ = self.store.audit(
            "provider.rebind-not-tracked",
            "runtime",
            &serde_json::json!({
                "providerId": id.as_str(),
                "limit": crate::declarative_lifecycle::MAX_DECLARATIVE_BINDINGS,
                "reason": "this device was never recorded in the declarative binding table (it \
                           was full); re-enabling only allows it to connect again — its \
                           capabilities come back after a restart",
            }),
        );
        let desc = match self
            .providers
            .transition(
                id,
                ProviderState::Disconnected,
                merge_provider_detail(Some(&note), None, &warnings),
            )
            .await
        {
            Ok(desc) => desc,
            // 這個狀態到 `Disconnected` 不合法：那就維持現況，只留稽核。
            Err(_) => return self.providers.get(id).await,
        };
        self.persist_provider(id).await;
        self.character_project_provider(id, ProviderState::Disconnected);
        Ok(desc)
    }

    /// 進入「重新綁定中」：狀態誠實地退到 `Disconnected`，detail 說得出正在
    /// 做什麼，然後把八個步驟交給有界的背景任務。
    async fn enter_declarative_rebinding(
        &self,
        id: &ProviderId,
        generation: u64,
    ) -> DomainResult<ProviderDescriptor> {
        let mut warnings = provider_detail_warnings(
            self.providers
                .get(id)
                .await
                .ok()
                .and_then(|d| d.detail)
                .as_deref(),
        );
        if !warnings.iter().any(|w| w == REBINDING_WARNING) {
            warnings.push(REBINDING_WARNING.to_string());
        }
        let others: Vec<String> = warnings
            .iter()
            .filter(|w| *w != REBINDING_WARNING)
            .cloned()
            .collect();
        let note = if others.is_empty() {
            REBINDING_NOTE.to_string()
        } else {
            format!("{} {REBINDING_NOTE}", warning_summary(&others))
        };
        let desc = match self
            .providers
            .transition(
                id,
                ProviderState::Disconnected,
                merge_provider_detail(Some(&note), None, &warnings),
            )
            .await
        {
            Ok(desc) => desc,
            Err(error) => {
                // 這個狀態到 `Disconnected` 不合法（例如 provider 已經被別的決定
                // 帶到別處）：那就**不要**開始重新綁定——留著一個 `Rebinding`
                // 卻沒有人推進它，比誠實說「沒開始」更糟。
                self.abandon_declarative_rebind(id.as_str(), generation, &error.to_string())
                    .await;
                return self.providers.get(id).await;
            }
        };
        self.persist_provider(id).await;
        self.character_project_provider(id, ProviderState::Disconnected);
        self.spawn_declarative_rebind(id.as_str(), generation);
        Ok(desc)
    }

    /// 一條裝置線握上手了：**先**讓綁定成立，**再**把狀態從 `Disconnected`
    /// 收斂成 `Available`。
    ///
    /// 為什麼順序不能反：`DeviceBinding::on_aip` 的閘門要求生命週期是
    /// `Bound`。先翻狀態的話，中間那段窗口裡裝置送來的每一則 frame（含
    /// 「連上就自己送 join」那一族的 join）都會被確定性丟掉，而畫面上顯示
    /// 「可用、已連線」——狀態先於實際綁定，誠實階梯反向。
    ///
    /// 為什麼要拿 provider 的序列化鎖：這是一個 check-then-act，而且是由背景
    /// task 呼叫的。不持鎖的話，人類按下停用的同一刻若有一條線握上手，
    /// 這裡讀到的 `Disconnected` 已經過期，接著的 `Disabled → Available`
    /// 是一條合法邊——剛被停用的裝置會立刻被背景 task 翻回可用。
    ///
    /// 只認 `Disconnected` 這一個入口狀態。宣告式裝置平常停在 `Installed`
    /// （授權是逐能力的 enable，不是 provider 狀態），一條握手成功的連線
    /// **不得**因此把它升成 `Available`——那會讓「連上了」冒充「人類啟用了」。
    pub(crate) async fn converge_provider_after_link_ready(&self, id: &ProviderId) {
        use crate::declarative_lifecycle::LinkReadyOutcome;
        let _serialized = self.providers.lock_provider(id).await;
        // 撤銷／停用的決定跨重啟有效：store 說它是關的，就沒有任何背景握手
        // 可以把它打開。
        if let Some(reason) = self.provider_off_reason(id) {
            let _ = self.store.audit(
                "provider.link-ready-refused",
                "runtime",
                &serde_json::json!({
                    "providerId": id.as_str(),
                    "reason": format!("this device is still marked off ({reason})"),
                }),
            );
            return;
        }
        let outcome = self.note_declarative_link_ready(id.as_str());
        let (established, recovered) = match &outcome {
            LinkReadyOutcome::Refused { lifecycle } => {
                let _ = self.store.audit(
                    "provider.link-ready-refused",
                    "runtime",
                    &serde_json::json!({
                        "providerId": id.as_str(),
                        "lifecycle": lifecycle,
                        "reason": "a late handshake never revives a binding the human took down",
                    }),
                );
                return;
            }
            // 綁定表對它一無所知：維持既有行為（只收斂狀態），但也不假裝
            // 「綁定成立了」——失敗說明不得在這條路徑上被抹掉。
            LinkReadyOutcome::NotTracked => (false, false),
            LinkReadyOutcome::AlreadyBound | LinkReadyOutcome::Committed { .. } => (true, false),
            LinkReadyOutcome::RecoveredAfterFailure => (true, true),
        };
        self.converge_provider_state_locked(id, established, recovered)
            .await;
        if recovered {
            // 這一輪 rebind 的第 7 步已經把「人類先前手動關掉的受器」套用回去
            // 了（只是握手晚了一步）；紀錄留著的話，之後一次由**斷線**觸發的
            // 重新綁定會拿一份過期的名單去關掉人類早就重新打開的受器。
            self.clear_declarative_human_disabled(id.as_str());
        }
    }

    /// 把狀態從 `Disconnected` 收斂成 `Available`（**呼叫端必須持有這台
    /// provider 的序列化鎖**）。
    ///
    /// `established` ＝這一次收斂真的讓綁定成立了嗎。只有它為真時才可以把
    /// `rebind-failed: …` 那一句從 detail 拿掉——否則一條晚到的握手會把失敗
    /// 說明抹掉，畫面上看不出這台裝置其實一句話都送不進來。
    pub(crate) async fn converge_provider_state_locked(
        &self,
        id: &ProviderId,
        established: bool,
        recovered: bool,
    ) {
        let Ok(existing) = self.providers.get(id).await else {
            return;
        };
        if existing.state != ProviderState::Disconnected {
            return;
        }
        let previous = provider_detail_warnings(existing.detail.as_deref());
        let failure_note: Option<String> = previous
            .iter()
            .find(|w| w.starts_with(REBIND_FAILED_WARNING))
            .cloned();
        let warnings: Vec<String> = previous
            .into_iter()
            .filter(|w| {
                if w == REBINDING_WARNING {
                    // 「重新連線中」屬於上一個狀態，收斂完就不再成立。
                    return false;
                }
                if w.starts_with(REBIND_FAILED_WARNING) {
                    return !established;
                }
                true
            })
            .collect();
        if self
            .providers
            .transition(
                id,
                ProviderState::Available,
                merge_provider_detail(None, None, &warnings),
            )
            .await
            .is_ok()
        {
            self.persist_provider(id).await;
            self.character_project_provider(id, ProviderState::Available);
            if recovered {
                // 失敗是真的發生過：收斂把那一句從畫面上拿掉，稽核就必須把它
                // 接住，否則「那次重連怎麼了」在紀錄上是一段空白。
                let _ = self.store.audit(
                    "provider.rebind-recovered",
                    "runtime",
                    &serde_json::json!({
                        "providerId": id.as_str(),
                        "previousFailure": failure_note,
                        "note": "the device completed its handshake after the rebind had already \
                                 given up; the binding was re-established",
                    }),
                );
            }
        }
    }

    /// rebind 沒有成功：狀態留在 `Disconnected`，detail 換成誠實的失敗說明。
    pub(crate) async fn note_provider_rebind_failed_locked(&self, id: &ProviderId, reason: &str) {
        let Ok(existing) = self.providers.get(id).await else {
            return;
        };
        let mut warnings: Vec<String> = provider_detail_warnings(existing.detail.as_deref())
            .into_iter()
            .filter(|w| !is_rebind_note(w))
            .collect();
        warnings.push(format!("{REBIND_FAILED_WARNING}: {reason}"));
        let others: Vec<String> = warnings
            .iter()
            .filter(|w| !is_rebind_note(w))
            .cloned()
            .collect();
        let note = if others.is_empty() {
            REBIND_FAILED_NOTE.to_string()
        } else {
            format!("{} {REBIND_FAILED_NOTE}", warning_summary(&others))
        };
        if self
            .providers
            .transition(
                id,
                existing.state,
                merge_provider_detail(Some(&note), None, &warnings),
            )
            .await
            .is_ok()
        {
            self.persist_provider(id).await;
        }
    }

    /// 把一個已停下來的 provider 底下的受器／動器旗標全部關掉，並留下可追查的
    /// audit（哪些能力、因為哪個狀態被關）。
    ///
    /// 只往「關」的方向走：這個函式永遠不會把任何東西打開。
    ///
    /// audit 只記**這次真的從開變關**的能力：`registry.receptor()`／`actuator()`
    /// 反映的就是 enabled 旗標本身（provider 閘門不影響它），所以先問再關。
    /// 否則重複停用同一個 provider 會一直寫出「關掉了一堆東西」的假紀錄。
    async fn disable_provider_capabilities(&self, desc: &ProviderDescriptor, reason: &str) {
        // 家族共用能力（例如 `iphone.*`：每台已配對手機的 descriptor 都塞同一組
        // 字面值）不得由通用路徑關旗標——旗標只有一份，翻掉它等於把**別台**
        // 還在操作中的裝置一起關掉（違反「每個共享狀態只有一個 owner」與
        // 「感測不靜默」）。這一台自己的擷取由 `stop_provider_sensing` 指名停止。
        let shared = self.shared_capability_holders(desc).await;
        let mut receptors = Vec::new();
        let mut actuators = Vec::new();
        for rid in &desc.receptors {
            if shared.receptors.contains(rid) {
                continue;
            }
            let id = ReceptorId::new(rid);
            if self.registry.receptor(&id).await.is_err() {
                continue; // 已經是關的（或根本沒註冊）：沒有東西可關。
            }
            if self.registry.set_receptor_enabled(&id, false).await.is_ok() {
                receptors.push(rid.clone());
            }
        }
        for aid in &desc.actuators {
            if shared.actuators.contains(aid) {
                continue;
            }
            let id = ActuatorId::new(aid);
            if self.registry.actuator(&id).await.is_err() {
                continue; // 已經是關的（或根本沒註冊）。
            }
            if self.registry.set_actuator_enabled(&id, false).await.is_ok() {
                actuators.push(aid.clone());
            }
        }
        // 沒關的那一批要留痕：不解釋就變成「停用了卻還看得到一支啟用中的麥克風」。
        if !shared.is_empty() {
            self.store
                .audit(
                    "provider.capabilities-shared-kept",
                    "runtime",
                    &json!({
                        "providerId": desc.identity.id.as_str(),
                        "reason": reason,
                        "sharedWith": shared.holders,
                        "receptors": shared.receptors,
                        "actuators": shared.actuators,
                        "note": SHARED_CAPABILITY_NOTE,
                    }),
                )
                .ok();
        }
        if receptors.is_empty() && actuators.is_empty() {
            return;
        }
        self.store
            .audit(
                "provider.capabilities-disabled",
                "runtime",
                &json!({
                    "providerId": desc.identity.id.as_str(),
                    "reason": reason,
                    "receptors": receptors,
                    "actuators": actuators,
                }),
            )
            .ok();
    }

    /// `desc` 宣告的能力之中，哪些**還被其他仍在操作中的 provider 宣告**。
    ///
    /// 「仍在操作中」＝不是 [`provider_stopped`] 的狀態。registry 的 enabled 旗標
    /// 是全域單例（一個能力 id 一份），所以只要還有第二個持有者，這一份旗標就
    /// 不屬於被停用的這一台，通用路徑不得代它決定。
    pub(crate) async fn shared_capability_holders(
        &self,
        desc: &ProviderDescriptor,
    ) -> SharedCapabilities {
        let mut shared = SharedCapabilities::default();
        if desc.receptors.is_empty() && desc.actuators.is_empty() {
            return shared;
        }
        for other in self.providers.list().await {
            if other.identity.id == desc.identity.id || provider_stopped(other.state) {
                continue;
            }
            let mut holds = false;
            for rid in &desc.receptors {
                if other.receptors.iter().any(|r| r == rid) {
                    holds = true;
                    if !shared.receptors.iter().any(|r| r == rid) {
                        shared.receptors.push(rid.clone());
                    }
                }
            }
            for aid in &desc.actuators {
                if other.actuators.iter().any(|a| a == aid) {
                    holds = true;
                    if !shared.actuators.iter().any(|a| a == aid) {
                        shared.actuators.push(aid.clone());
                    }
                }
            }
            if holds {
                shared.holders.push(other.identity.id.as_str().to_string());
            }
        }
        shared
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
    pub(crate) fn provider_off_reason(&self, id: &ProviderId) -> Option<String> {
        self.store
            .get_meta(&provider_off_key(id))
            .ok()
            .flatten()
            .filter(|reason| !reason.is_empty())
    }

    /// 關閉某 provider 的宣告式 adapter 連線（若有）。回傳關掉的連線描述。
    pub(crate) fn close_declarative_links(&self, id: &ProviderId, reason: &str) -> Vec<String> {
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
        // 與 `transition_provider` 同一把鎖：撤銷不得與停用／重新綁定交錯。
        let _serialized = self.providers.lock_provider(id).await;
        let desc = self
            .providers
            .transition(id, ProviderState::Revoked, Some("revoked by user".into()))
            .await?;
        // 撤銷永遠不重新綁定：先寫死這個原因，之後任何一條 rebind 路徑都拒絕。
        self.note_declarative_unbound(
            id.as_str(),
            crate::declarative_lifecycle::UnboundReason::Revoked,
        );
        // X2：有登記感測來源的 provider（例如已配對的行動裝置）走同一條
        // request_stop＋release——撤銷一台正在擷取的裝置不得只翻旗標。
        // 順序：先問來源、再關連線、最後才翻旗標。先關連線就等於親手拆掉
        // 唯一能問「你停了嗎」的那條線，然後永遠只能回答「未知」。
        let sensor_stop = self
            .stop_provider_sensing(id.as_str(), crate::sensors::SENSOR_STOP_REASON_PROVIDER_OFF)
            .await;
        // 撤銷＝連線也要斷（不只是停止派工），而且必須跨重啟：重啟後 spec 重新
        // 載入時不得把連線開回來、也不得讓受器回到啟用。
        let closed_links = self.close_declarative_links(id, "revoked");
        // 撤銷也走同一條「家族共用能力不由通用路徑關旗標」的判斷：撤銷 A
        // 不得順手關掉 B 還在用的同一份旗標（B 沒有被撤銷）。
        self.disable_provider_capabilities(&desc, "revoked").await;
        self.mark_provider_off(id, "revoked");
        self.persist_provider(id).await;
        self.character_project_provider(id, ProviderState::Revoked);
        self.store.audit(
            "provider.revoked",
            "user",
            &serde_json::json!({
                "providerId": id.as_str(),
                "closedLinks": closed_links,
                "sensorStop": sensor_stop,
            }),
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

/// 重新綁定**進行中**的固定警告文字（原文給進階模式與 CLI；一般模式看到的
/// 是下面那一句人話）。誠實：這台裝置此刻**沒有**連上。
pub(crate) const REBINDING_WARNING: &str = "rebinding: this device's link and capability \
     declaration are being rebuilt; it is NOT connected yet and its state stays disconnected \
     until the handshake succeeds";

/// 同一件事的**人話**（一般模式看到的那一句）：不含能力 id、不含技術詞。
pub(crate) const REBINDING_NOTE: &str =
    "正在重新連上這台裝置，它的能力還沒有回來；連上之後才會顯示為可用。";

/// 重新綁定**失敗**的固定警告文字（後面會接上原因）。
pub(crate) const REBIND_FAILED_WARNING: &str = "rebind-failed";

/// 同一件事的人話。
pub(crate) const REBIND_FAILED_NOTE: &str =
    "這台裝置沒有重新連上，它的能力還沒有回來；請檢查裝置與接線後再啟用一次。";

/// 這台裝置**不支援**免重啟重新綁定的固定警告文字（綁定表滿時它沒被登記）。
pub(crate) const REBIND_UNTRACKED_WARNING: &str = "rebind-not-tracked: this device was never \
     recorded in the declarative binding table (it was full); re-enabling only allows it to \
     connect again — its capabilities come back after a restart";

/// 同一件事的人話。
pub(crate) const REBIND_UNTRACKED_NOTE: &str =
    "這台裝置目前不能免重新啟動重新連上，它的能力要等重新啟動後才會回來。";

/// 這一則警告是「重新綁定」自己留下的（進行中或失敗）嗎？
///
/// 只有一個判準：失敗那一則後面會接原因（`rebind-failed: …`），所以不能用
/// 相等比較——用相等比較的話，舊的失敗訊息會被人類的下一次啟用一路帶著走，
/// 於是一台已經連上的裝置畫面上還掛著上一次的失敗。
fn is_rebind_note(warning: &str) -> bool {
    warning == REBINDING_WARNING
        || warning == REBIND_UNTRACKED_WARNING
        || warning.starts_with(REBIND_FAILED_WARNING)
}

/// 稽核 `provider.capabilities-shared-kept` 的固定說明（不含路徑、不回顯輸入）。
const SHARED_CAPABILITY_NOTE: &str = "these capability flags are shared with other operational \
     providers and were left enabled; only this provider's own capture was stopped";

/// 一次停用裡「因為還有別人宣告而沒有關掉」的能力，以及還有誰宣告它們。
#[derive(Debug, Default)]
pub(crate) struct SharedCapabilities {
    pub(crate) receptors: Vec<String>,
    pub(crate) actuators: Vec<String>,
    pub(crate) holders: Vec<String>,
}

impl SharedCapabilities {
    fn is_empty(&self) -> bool {
        self.receptors.is_empty() && self.actuators.is_empty()
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
