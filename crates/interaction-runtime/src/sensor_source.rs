//! `SensorSource`：核心停止感測時唯一認得的介面（M2 §3.1）。
//!
//! 為什麼要有這個 port：在此之前，「停止所有感測」是本機麥克風的一段特例
//! 程式碼＋一段寫死的 iPhone 呼叫；每多一種會擷取的來源（宣告式裝置、未來的
//! 其他行動裝置），停止路徑就要再多一個 if 分支，而且緊急停止那條路徑還各自
//! 手刻了一份報告——兩條路徑因此可以對同一件事說出不同的話。
//!
//! 現在核心只問一個來源四件事：你是誰、你對應哪一筆能力宣告、你現在擷取什麼、
//! 請你在這個期限內停下來並誠實回報結果。
//!
//! 不變量：
//! - 誠實階梯：`request_stop` 回來 ≠ 停了。只有 [`SensorStopStatus::Stopped`]
//!   與 [`SensorStopStatus::AlreadyStopped`] 算「確認沒有在擷取」，其餘一律是
//!   「可能還在擷取」。
//! - 有界：`deadline` 是這次停止的**預算**。協調器另外再包一層逾時，來源自己
//!   不守約也不會把停止拖成無限等待。
//! - 感測不靜默：來源回報的 [`SensorUse`] 會直接進 `status.activeSensors`
//!   （tray／首頁／角色視窗都吃它）；已要求停止但沒確認的來源必須留在裡面。

use crate::runtime::{Runtime, RuntimeInner};
use crate::sensors::{SensorStopOutcome, SensorUse};
use interaction_core::{DomainError, DomainResult};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

/// 同時登記的感測來源上限。來源可能來自設定檔／區網（宣告式 adapter、配對的
/// 行動裝置），登記表不得無界成長；超過就誠實拒絕並留稽核，不靜默丟棄。
pub const MAX_SENSOR_SOURCES: usize = 32;

/// 來源不守自己的期限時，協調器最多再多等這麼久就當成「結果未知」。
const SOURCE_GRACE: Duration = Duration::from_millis(500);

/// 來源被移除、但移除當下它還在擷取：那一筆「可能還在擷取」要留在**即時**
/// 清單（`activeSensors`）多久。永遠留著等於永久誤報（我們確實不知道它停了
/// 沒有），一移除就消失則是靜默——取有界的中間值。
///
/// 誠實：過了這個窗**不是**「已經停了」。到期的記錄會轉進不受 TTL 影響的
/// [`UnresolvedStop`]（未解決停止摘要），只能被明確的確認或人為解除清掉。
pub const ORPHAN_CAPTURE_VISIBLE: Duration = Duration::from_secs(60);

/// 「未解決停止」摘要的上限。它不隨時間過期，所以必須自己有界；滿了丟最舊
/// 的一筆並留稽核（被丟掉的那一筆從來沒有被說成已停止）。
pub const MAX_UNRESOLVED_STOPS: usize = 32;

/// 一個感測來源登記的識別：`source_id` 加上**這一次登記**的世代。
///
/// 為什麼要世代：同一個 `source_id` 可以被重新登記（裝置重連、adapter 重新
/// 綁定）。舊那一次登記留下的「可能還在擷取」不會因為有了新來源就變成假的
/// ——新來源不知道舊連線那一頭發生過什麼事。沒有世代的話，重新登記會把上一
/// 次的未解決記錄無條件抹掉，而畫面上看起來「一切正常」。
pub type SourceKey = (String, u64);

/// 可注入的單調時鐘。
///
/// production 永遠是 `Instant::now()`；測試把它往前推，才不用真的等 60 秒
/// 去驗 TTL 行為（真等 60 秒的測試不會有人跑）。
#[derive(Default)]
pub struct SensorClock {
    /// 額外前進的毫秒數。production 永遠是 0。
    skew_ms: std::sync::atomic::AtomicU64,
}

/// 時鐘最多能被推多遠（避免 `Instant` 溢位；也讓誤用有上限）。
const MAX_CLOCK_SKEW: Duration = Duration::from_secs(365 * 24 * 3600);

impl SensorClock {
    pub fn now(&self) -> std::time::Instant {
        let skew = self
            .skew_ms
            .load(std::sync::atomic::Ordering::SeqCst)
            .min(MAX_CLOCK_SKEW.as_millis() as u64);
        std::time::Instant::now() + Duration::from_millis(skew)
    }

    /// 把時鐘往前推（只給測試用；production code 不呼叫）。
    #[doc(hidden)]
    pub fn advance(&self, by: Duration) {
        let by = by.min(MAX_CLOCK_SKEW).as_millis() as u64;
        let _ = self.skew_ms.fetch_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |current| {
                Some(
                    current
                        .saturating_add(by)
                        .min(MAX_CLOCK_SKEW.as_millis() as u64),
                )
            },
        );
    }
}

/// 一次停止請求的結果（明確枚舉，不用字串猜）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SensorStopStatus {
    /// 來源**明確確認**已經停止擷取。
    Stopped,
    /// 本來就沒有在擷取（沒有東西要停）。重複停止時的正常結果。
    AlreadyStopped,
    /// 送出去了，但期限內沒有收到確認——來源可能還在擷取。
    Unknown,
    /// 根本送不出去（連線已斷／佇列滿）：沒有任何東西被停。
    Unreachable,
    /// 來源明確拒絕停止（它知道自己停不了）。誠實：這比逾時更確定它還在擷取。
    Refused,
}

impl SensorStopStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SensorStopStatus::Stopped => "stopped",
            SensorStopStatus::AlreadyStopped => "already-stopped",
            SensorStopStatus::Unknown => "unknown",
            SensorStopStatus::Unreachable => "unreachable",
            SensorStopStatus::Refused => "refused",
        }
    }

    /// 「確認沒有在擷取」。只有這兩種算數；其餘一律當成可能還在擷取。
    pub fn confirmed(&self) -> bool {
        matches!(
            self,
            SensorStopStatus::Stopped | SensorStopStatus::AlreadyStopped
        )
    }
}

/// 一個來源（或它底下的一台裝置）對一次停止請求的誠實回報。
///
/// 欄位一律是中性的：`source_id` 可以是一台裝置、一條連線或整個來源本身，
/// 核心不假設它是什麼。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorStopReport {
    /// 誰回報的（裝置 id／連線 id／來源 id）。只進 payload，不進人話標題。
    pub source_id: String,
    /// 對應哪一筆能力宣告（`ProviderCapabilityDeclaration::declaration_id`）。
    pub declaration_id: String,
    /// 這台來源的人話名稱（有就給，沒有時介面退回 `source_id`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
    /// 這次停止涵蓋哪些受器（由來源自己宣告，核心不猜）。
    pub sensors: Vec<String>,
    pub outcome: SensorStopStatus,
    /// 實際等待毫秒（有界）。
    pub waited_ms: u64,
    /// 確認來源（例如 `ack`／`status`）。沒確認就沒有值。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed_via: Option<String>,
    /// 給人看的補充（為什麼 refused／unreachable）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl SensorStopReport {
    pub fn new(
        source_id: impl Into<String>,
        declaration_id: impl Into<String>,
        sensors: Vec<String>,
        outcome: SensorStopStatus,
        waited_ms: u64,
    ) -> Self {
        SensorStopReport {
            source_id: source_id.into(),
            declaration_id: declaration_id.into(),
            source_label: None,
            sensors,
            outcome,
            waited_ms,
            confirmed_via: None,
            detail: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.source_label = Some(label.into());
        self
    }

    pub fn with_via(mut self, via: Option<impl Into<String>>) -> Self {
        self.confirmed_via = via.map(Into::into);
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// 確認沒有在擷取嗎？（[`SensorStopStatus::confirmed`] 的轉呼叫）
    pub fn confirmed(&self) -> bool {
        self.outcome.confirmed()
    }

    /// 這一筆足以清掉**舊世代**留下的未解決停止嗎？
    ///
    /// 誠實界線：`Stopped` 是「我真的把它停下來／收到它停了的回覆」，那是一次
    /// 主動的確認；`AlreadyStopped` 卻同時是兩件事——「本機這一側本來就沒有
    /// 東西要停」（一個旗標）與「這個來源對該受器有權威，而它確認沒在擷取」
    /// （本機麥克風）。前者沒有跟任何裝置往返過，拿它替**上一次登記**留下的
    /// 未解決停止作證，就是用一台新裝置替一台舊裝置簽名（見 [`SourceKey`]）。
    ///
    /// 所以 `AlreadyStopped` 只有帶著 [`Self::confirmed_via`]（說得出證據來自
    /// 哪裡：裝置 ack、來源本身的權威）時才算數。
    pub fn resolves_unresolved_stops(&self) -> bool {
        match self.outcome {
            SensorStopStatus::Stopped => true,
            SensorStopStatus::AlreadyStopped => self.confirmed_via.is_some(),
            _ => false,
        }
    }
}

/// 既有的「補發未確認事件」介面：新的報告型別直接實作它，`sensors.rs` 的純
/// 函式（`sensor_stop_uncertain_payloads`）因此完全不必改。
impl SensorStopOutcome for SensorStopReport {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    fn sensor_ids(&self) -> Vec<String> {
        self.sensors.clone()
    }

    fn outcome_label(&self) -> &str {
        self.outcome.as_str()
    }

    fn waited_ms(&self) -> u64 {
        self.waited_ms
    }

    fn confirmed_stopped(&self) -> bool {
        self.outcome.confirmed()
    }
}

/// 一個會擷取的來源。核心停止感測時只認得這個介面。
#[async_trait::async_trait]
pub trait SensorSource: Send + Sync {
    /// 來源識別碼。登記表的鍵；也是「哪一個 provider 底下」的前綴比對依據
    /// （`provider.mobile` 之於 `provider.mobile.<裝置>`）。
    fn source_id(&self) -> String;

    /// 對應的能力宣告 id。來源逾時到連受器清單都拿不到時，協調器靠它從宣告表
    /// 查出「這一族宣告了哪些高風險受器」，才說得出是哪一個受器可能還在擷取。
    fn declaration_id(&self) -> String;

    /// 停止是本機同步動作、沒有遠端等待嗎？true 的來源由協調器**先**跑完，
    /// 不排在別人的有界等待後面（本機麥克風不該等一台沒回應的裝置）。
    fn stops_immediately(&self) -> bool {
        false
    }

    /// 這個來源自己就會發 `sensor.stopped`（例如連線 loop 以感測起訖為準）嗎？
    /// true 時協調器不重發，避免同一次停止在事件流上出現兩則。
    fn reports_own_stop_events(&self) -> bool {
        false
    }

    /// 目前正在擷取什麼（空＝沒有）。已要求停止但沒確認的必須留在這裡並標
    /// `stopping`／`stop-unknown`——消失等於宣稱它停了。
    async fn active_captures(&self) -> Vec<SensorUse>;

    /// 請這個來源停止感測。`target=Some(id)` 只針對它底下的一台（撤銷單一裝置
    /// 走這條），`None` ＝整個來源。`deadline` 是這次的等待預算。
    ///
    /// 回傳是**誠實回報**，不是「送出去了」：沒確認一律 Unknown/Unreachable/Refused。
    async fn request_stop(
        &self,
        target: Option<&str>,
        deadline: Duration,
        reason: &str,
    ) -> Vec<SensorStopReport>;

    /// provider 被撤銷／停用後，這個來源要不要順便放掉那台裝置的連線？
    /// 預設不做（多數來源沒有連線可放）。回傳給稽核的摘要。
    async fn release(&self, _target: Option<&str>, _reason: &str) -> Option<serde_json::Value> {
        None
    }
}

/// 被移除、但移除當下還在擷取的來源留下的「可能還在擷取」記錄。
///
/// 生命週期只有兩段，而且兩段都不是「消失」：
/// 1. [`ORPHAN_CAPTURE_VISIBLE`] 之內在 `activeSensors` 上以 `stop-unknown` 現身；
/// 2. 之後轉進 [`UnresolvedStop`]——不再佔著即時清單，但仍然是一筆沒有結論的
///    停止，要由確認或人為解除才會消失。
#[derive(Debug, Clone)]
pub(crate) struct OrphanedCaptures {
    pub(crate) captures: Vec<SensorUse>,
    pub(crate) at: std::time::Instant,
    /// 這一筆屬於哪一次登記（見 [`SourceKey`]）。
    pub(crate) since: chrono::DateTime<chrono::Utc>,
    /// 移除**當下**問到的人話名稱（見 [`Runtime::sensor_source_label`]）。
    ///
    /// 為什麼在這裡定格、而不是讀取時再查：來源已經被移除了，它的 provider
    /// 記錄與能力宣告隨時可能跟著被撤掉——之後再查只會查到空的，畫面上那一筆
    /// 就會從「客廳的 ESP32」退化成「某個裝置」。名字是移除那一刻的事實。
    pub(crate) label: Option<String>,
}

/// 一筆「已經不在即時清單上、但仍然沒有結論」的停止。
///
/// 誠實：它**不是**歷史。歷史在稽核裡；這張表回答的是「現在還有哪些東西，
/// 我們不知道它停了沒有」。所以它不隨時間過期，只能由
/// 「同 id 的新來源對那個受器確認停止」或人類明確解除清掉。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedStop {
    pub source_id: String,
    /// 哪一次登記（同 id 的新來源不會蓋掉舊世代的這一筆）。
    pub generation: u64,
    /// 這一筆涵蓋哪些受器。
    pub sensors: Vec<String>,
    /// 來源被移除、這筆變成未解決的時間。
    pub since: chrono::DateTime<chrono::Utc>,
    /// 最後看到的擷取狀態（含 `state`／`purpose`）。不猜、不改寫。
    pub last_known: Vec<SensorUse>,
    /// 人話名稱（選填）。`source_id`／`declaration_id` 是內部識別，一般模式
    /// 不得顯示；不知道名字時整個欄位省略，呼叫端自己用中性字樣（「某個裝置」），
    /// **不得**拿 id 冒充人話。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
}

/// 一次登記中的感測來源＋它的世代。
#[derive(Clone)]
pub(crate) struct RegisteredSource {
    pub(crate) source: Arc<dyn SensorSource>,
    pub(crate) generation: u64,
}

/// 來源登記表：有界、以 `source_id` 為鍵。
pub(crate) type SensorSourceMap = BTreeMap<String, RegisteredSource>;

impl Runtime {
    /// 登記一個感測來源。同一個 `source_id` 再登記一次＝取代（來源自己就是
    /// 那件事的完整事實）。超過 [`MAX_SENSOR_SOURCES`] 時誠實拒絕並留稽核。
    ///
    /// **不會**清掉上一次登記留下的「可能還在擷取」記錄：新來源不知道舊連線
    /// 那一頭發生過什麼事，抹掉等於用一台新裝置替一台舊裝置作證。舊記錄只能
    /// 由「這個新來源對那個受器確認停止」或人類明確解除清掉。
    pub async fn register_sensor_source(&self, source: Arc<dyn SensorSource>) -> DomainResult<()> {
        let id = source.source_id();
        if id.trim().is_empty() {
            return Err(DomainError::Validation("sensor source id is empty".into()));
        }
        let mut map = self.sensor_sources.write().await;
        if map.len() >= MAX_SENSOR_SOURCES && !map.contains_key(&id) {
            drop(map);
            let _ = self.store.audit(
                "sensor.source-rejected",
                "runtime",
                &serde_json::json!({
                    "sourceId": id,
                    "limit": MAX_SENSOR_SOURCES,
                    "reason": "sensor source registry is full",
                }),
            );
            return Err(DomainError::PolicyBlocked(format!(
                "sensor source registry is full ({MAX_SENSOR_SOURCES}); {id} was not registered"
            )));
        }
        let generation = self
            .sensor_source_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            .saturating_add(1);
        map.insert(id.clone(), RegisteredSource { source, generation });
        Ok(())
    }

    /// 這個來源目前這一次登記的世代（沒有登記＝None）。
    pub async fn sensor_source_generation(&self, source_id: &str) -> Option<u64> {
        self.sensor_sources
            .read()
            .await
            .get(source_id)
            .map(|entry| entry.generation)
    }

    /// 這個感測來源的**人話**名稱（不知道就是 `None`）。
    ///
    /// 為什麼核心要管這件事：`sourceId` 是內部識別（`provider.mobile.7f3a…`），
    /// 一般模式不得把它丟到畫面上。但「哪一台可能還在擷取」這句話沒有名字就
    /// 等於沒說，所以名字要由**認得那台裝置的那一層**（provider 登記表／能力
    /// 宣告）提供，不是由呈現層去猜或去拼 id。
    ///
    /// 兩個來源，由具體到一般：
    /// 1. 同 id 的 provider 顯示名——「那一台」的名字（使用者自己取的暱稱）；
    /// 2. 能力宣告的 `class_label`——「那一類」的名字（provider 家族自己給的
    ///    種類名），只在不知道是哪一台時才夠用。
    ///
    /// 誠實：兩邊都沒有就是 `None`。**不得**退回 `source_id`——那不是人話，
    /// 而且會把內部識別洩到一般模式的畫面上。
    pub(crate) async fn sensor_source_label(
        &self,
        source_id: &str,
        declaration_id: &str,
    ) -> Option<String> {
        let trimmed = |value: String| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        };
        let by_provider = self
            .providers
            .get(&interaction_core::ProviderId::new(source_id))
            .await
            .ok()
            .and_then(|desc| trimmed(desc.identity.display_name));
        if by_provider.is_some() {
            return by_provider;
        }
        self.capability_declarations()
            .declaration(declaration_id)
            .and_then(|d| d.class_label)
            .and_then(trimmed)
    }

    /// 移除一個感測來源。移除當下它還在擷取的話，那一筆擷取**不得靜默消失**：
    /// 記成有界可見的「停止結果未知」、補發事件、並留永久稽核。
    pub async fn unregister_sensor_source(&self, source_id: &str) -> bool {
        let removed = self.sensor_sources.write().await.remove(source_id);
        let Some(entry) = removed else {
            return false;
        };
        let generation = entry.generation;
        let captures = entry.source.active_captures().await;
        if captures.is_empty() {
            return true;
        }
        let sensors: Vec<String> = captures.iter().map(|c| c.kind.clone()).collect();
        let stale: Vec<SensorUse> = captures
            .into_iter()
            .map(|mut c| {
                c.state = crate::sensors::SENSOR_STATE_STOP_UNKNOWN.to_string();
                c.purpose = format!("{}（來源已移除，停止結果未知）", c.purpose);
                c
            })
            .collect();
        let label = self
            .sensor_source_label(source_id, &entry.source.declaration_id())
            .await;
        let now = self.sensor_clock.now();
        let mut orphans = self.orphan_captures.write().await;
        // 有界：先把過期的搬進「未解決停止」（不是丟掉），還是滿的話丟最舊的
        // 一筆並留痕。來源反覆登記／移除不得讓這張表與 `activeSensors` 無界成長。
        let expired = drain_expired(&mut orphans, now);
        let mut dropped = None;
        if orphans.len() >= MAX_SENSOR_SOURCES {
            if let Some(oldest) = orphans
                .iter()
                .min_by_key(|(_, entry)| entry.at)
                .map(|(key, _)| key.clone())
            {
                orphans.remove(&oldest);
                dropped = Some(oldest);
            }
        }
        orphans.insert(
            (source_id.to_string(), generation),
            OrphanedCaptures {
                captures: stale,
                at: now,
                since: chrono::Utc::now(),
                label,
            },
        );
        drop(orphans);
        self.record_unresolved_stops(expired).await;
        if let Some((dropped_id, dropped_generation)) = dropped {
            // 被丟掉的那一筆**從來沒有**被說成已停止：稽核是它最後的痕跡。
            let _ = self.store.audit(
                "sensor.removed-capture-record-dropped",
                "runtime",
                &serde_json::json!({
                    "sourceId": dropped_id,
                    "generation": dropped_generation,
                    "limit": MAX_SENSOR_SOURCES,
                    "reason": "the removed-while-capturing record is bounded; the oldest entry was dropped without ever being confirmed stopped",
                }),
            );
        }
        for sensor in &sensors {
            self.events.emit(
                interaction_core::EventType::SensorStopUncertain,
                serde_json::json!({
                    "sensor": sensor,
                    "deviceId": source_id,
                    "outcome": SensorStopStatus::Unknown.as_str(),
                    "waitedMs": 0,
                    "reason": "sensor source removed while capturing",
                }),
            );
        }
        let _ = self.store.audit(
            "sensor.source-removed-while-capturing",
            "runtime",
            &serde_json::json!({
                "sourceId": source_id,
                "generation": generation,
                "sensors": sensors,
            }),
        );
        true
    }

    /// 目前登記的來源（順序固定）。
    pub async fn sensor_source_ids(&self) -> Vec<String> {
        self.sensor_sources.read().await.keys().cloned().collect()
    }

    /// 快照登記中的來源。停止掃描一律先快照再跑：掃描進行中有人移除來源，
    /// 那一份還在飛的停止結果仍然要回得來（不得被吞掉）。
    pub(crate) async fn sensor_sources_snapshot(&self) -> Vec<Arc<dyn SensorSource>> {
        self.sensor_sources
            .read()
            .await
            .values()
            .map(|entry| entry.source.clone())
            .collect()
    }

    /// 哪一個來源涵蓋這個 provider？回傳（來源, 要指名的 target）。
    ///
    /// 對應規則只有兩條，而且不認得任何具體裝置：id 完全相同＝整個來源；
    /// `<source_id>.<其餘>` ＝這個來源底下的那一台（`target` 就是「其餘」）。
    pub(crate) async fn sensor_source_for_provider(
        &self,
        provider_id: &str,
    ) -> Option<(Arc<dyn SensorSource>, Option<String>)> {
        let sources = self.sensor_sources.read().await;
        if let Some(entry) = sources.get(provider_id) {
            return Some((entry.source.clone(), None));
        }
        for (id, entry) in sources.iter() {
            if let Some(rest) = provider_id.strip_prefix(&format!("{id}.")) {
                if !rest.is_empty() {
                    return Some((entry.source.clone(), Some(rest.to_string())));
                }
            }
        }
        None
    }

    /// 「來源被移除時還在擷取」的**即時**可見記錄。到期的不會消失，而是轉進
    /// 「未解決停止」摘要——過期只代表「不再佔著即時清單」，不代表停了。
    pub(crate) async fn orphaned_captures(&self) -> Vec<SensorUse> {
        self.settle_expired_orphans().await;
        self.orphan_captures
            .read()
            .await
            .values()
            .flat_map(|entry| entry.captures.iter().cloned())
            .collect()
    }

    /// 過了即時可見窗的孤兒記錄轉進「未解決停止」。
    ///
    /// 刻意是**惰性**的（讀的時候才結算），而不是一個背景計時器：多一個
    /// 定時 task 就多一個要收掉的東西，而且沒有人在看的時候，這件事本來就
    /// 不需要發生。兩個讀取面（即時清單與未解決摘要）都會先呼叫它，所以
    /// 不管從哪一邊看，看到的都是同一份結算過的事實。
    async fn settle_expired_orphans(&self) {
        let now = self.sensor_clock.now();
        let expired = {
            let mut map = self.orphan_captures.write().await;
            drain_expired(&mut map, now)
        };
        self.record_unresolved_stops(expired).await;
    }

    /// 把到期的孤兒記錄轉成「未解決停止」（有界；滿了丟最舊的一筆並留痕）。
    async fn record_unresolved_stops(&self, expired: Vec<(SourceKey, OrphanedCaptures)>) {
        if expired.is_empty() {
            return;
        }
        let mut recorded = Vec::new();
        let mut dropped = Vec::new();
        {
            let mut map = self.unresolved_stops.write().await;
            for ((source_id, generation), entry) in expired {
                if map.len() >= MAX_UNRESOLVED_STOPS
                    && !map.contains_key(&(source_id.clone(), generation))
                {
                    if let Some(oldest) = map
                        .iter()
                        .min_by_key(|(_, value)| value.since)
                        .map(|(key, _)| key.clone())
                    {
                        map.remove(&oldest);
                        dropped.push(oldest);
                    }
                }
                let record = UnresolvedStop {
                    source_id: source_id.clone(),
                    generation,
                    sensors: entry.captures.iter().map(|c| c.kind.clone()).collect(),
                    since: entry.since,
                    last_known: entry.captures,
                    source_label: entry.label,
                };
                recorded.push(serde_json::json!({
                    "sourceId": record.source_id,
                    "generation": record.generation,
                    "sourceLabel": record.source_label,
                    "sensors": record.sensors,
                    "since": record.since,
                }));
                map.insert((source_id, generation), record);
            }
        }
        for (source_id, generation) in dropped {
            let _ = self.store.audit(
                "sensor.unresolved-stop-dropped",
                "runtime",
                &serde_json::json!({
                    "sourceId": source_id,
                    "generation": generation,
                    "limit": MAX_UNRESOLVED_STOPS,
                    "reason": "the unresolved-stop summary is bounded; the oldest entry was dropped without ever being confirmed stopped",
                }),
            );
        }
        let _ = self.store.audit(
            "sensor.unresolved-stop-recorded",
            "runtime",
            &serde_json::json!({
                "entries": recorded,
                "visibleForSeconds": ORPHAN_CAPTURE_VISIBLE.as_secs(),
                "reason": "the removed source never confirmed it stopped; it left the live list but the stop is still unresolved",
            }),
        );
    }

    /// 目前所有「未解決停止」（順序固定；空的話不序列化到 status）。
    pub async fn unresolved_stops(&self) -> Vec<UnresolvedStop> {
        self.settle_expired_orphans().await;
        self.unresolved_stops
            .read()
            .await
            .values()
            .cloned()
            .collect()
    }

    /// 一個**登記中**的來源確認了某些受器已經停止：把同 id 舊世代留下來的
    /// 未解決記錄清掉（即時清單與未解決摘要都清）。
    ///
    /// 誠實界線：只有**帶得出證據**的確認算數
    /// （[`SensorStopReport::resolves_unresolved_stops`]：主動停下來，或
    /// already-stopped 而且說得出 `confirmed_via`），而且只有現在還登記著的
    /// 來源說了才算——一台已經不在的裝置不能替自己作證。
    pub(crate) async fn resolve_stops_for(&self, source_id: &str, confirmed: &[String]) {
        if confirmed.is_empty() {
            return;
        }
        let mut cleared: Vec<serde_json::Value> = Vec::new();
        {
            let mut map = self.orphan_captures.write().await;
            let keys: Vec<SourceKey> = map
                .keys()
                .filter(|(id, _)| id == source_id)
                .cloned()
                .collect();
            for key in keys {
                if let Some(entry) = map.get_mut(&key) {
                    entry.captures.retain(|c| !confirmed.contains(&c.kind));
                    if entry.captures.is_empty() {
                        map.remove(&key);
                    }
                }
            }
        }
        {
            let mut map = self.unresolved_stops.write().await;
            let keys: Vec<SourceKey> = map
                .keys()
                .filter(|(id, _)| id == source_id)
                .cloned()
                .collect();
            for key in keys {
                let remove = match map.get_mut(&key) {
                    Some(entry) => {
                        entry.sensors.retain(|s| !confirmed.contains(s));
                        entry.last_known.retain(|c| !confirmed.contains(&c.kind));
                        entry.sensors.is_empty()
                    }
                    None => false,
                };
                if remove {
                    map.remove(&key);
                    cleared.push(serde_json::json!({"sourceId": key.0, "generation": key.1}));
                }
            }
        }
        if !cleared.is_empty() {
            let _ = self.store.audit(
                "sensor.unresolved-stop-resolved",
                "runtime",
                &serde_json::json!({
                    "clearedBy": source_id,
                    "sensors": confirmed,
                    "entries": cleared,
                }),
            );
        }
    }

    /// 人為解除一筆「未解決停止」。
    ///
    /// 誠實：這**不是**「它停了」，而是「人類看過了，不用再提醒」。所以它一定
    /// 要指名世代（不會誤消掉別的一筆），一定要留稽核，而且只有人可以做——
    /// AI 不得替使用者宣告一件沒有人確認過的事。
    pub async fn dismiss_unresolved_stop(
        &self,
        source_id: &str,
        generation: u64,
        actor: &str,
    ) -> DomainResult<serde_json::Value> {
        let key = (source_id.to_string(), generation);
        let removed = self.unresolved_stops.write().await.remove(&key);
        let Some(record) = removed else {
            return Err(DomainError::NotFound(format!(
                "no unresolved stop for {source_id} (generation {generation})"
            )));
        };
        let _ = self.store.audit(
            "sensor.unresolved-stop-dismissed",
            actor,
            &serde_json::json!({
                "sourceId": record.source_id,
                "generation": record.generation,
                "sensors": record.sensors,
                "since": record.since,
                "note": "dismissed by a human; this does NOT mean the source confirmed it stopped",
            }),
        );
        Ok(serde_json::json!({
            "dismissed": true,
            "sourceId": record.source_id,
            "generation": record.generation,
            "sensors": record.sensors,
            "since": record.since,
            "confirmedStopped": false,
        }))
    }

    /// 問一個來源停止，並在來源自己的期限外再包一層逾時：來源不守約也不得
    /// 把停止拖成無限等待。逾時時用能力宣告表補出「這一族宣告了哪些高風險
    /// 受器」，才說得出是哪一個受器可能還在擷取。
    pub(crate) async fn request_source_stop(
        &self,
        source: &Arc<dyn SensorSource>,
        target: Option<&str>,
        deadline: Duration,
        reason: &str,
    ) -> Vec<SensorStopReport> {
        let started = std::time::Instant::now();
        match tokio::time::timeout(
            deadline + SOURCE_GRACE,
            source.request_stop(target, deadline, reason),
        )
        .await
        {
            Ok(reports) => reports,
            Err(_) => {
                let declaration_id = source.declaration_id();
                let sensors = self
                    .capability_declarations()
                    .declaration(&declaration_id)
                    .map(|d| d.high_risk_receptors)
                    .unwrap_or_default();
                vec![SensorStopReport::new(
                    target.unwrap_or(&source.source_id()).to_string(),
                    declaration_id,
                    sensors,
                    SensorStopStatus::Unknown,
                    started.elapsed().as_millis() as u64,
                )
                .with_detail("the source did not answer the stop request within its deadline")]
            }
        }
    }
}

impl Runtime {
    /// provider 被撤銷／停用時的感測收斂（X2）。
    ///
    /// 通用入口（`revoke_provider`／`transition_provider`）過去對「有連線、正在
    /// 擷取」的來源幾乎什麼都沒做：受器旗標翻掉，但沒有人請那台裝置停止感測，
    /// 只能等背景 watcher 撞上事件才補一則停止請求——那是競態，不是保證。
    ///
    /// 現在走**同一條** [`SensorSource::request_stop`]（target 指名那一台），
    /// 未確認的補「可能還在擷取」事件，最後請來源放掉連線
    /// （[`SensorSource::release`]）。回傳值進 provider 的稽核。
    pub(crate) async fn stop_provider_sensing(
        &self,
        provider_id: &str,
        reason: &str,
    ) -> Option<serde_json::Value> {
        let (source, target) = self.sensor_source_for_provider(provider_id).await?;
        let reports = self
            .request_source_stop(
                &source,
                target.as_deref(),
                crate::mobile::STOP_SENSORS_WAIT,
                reason,
            )
            .await;
        self.emit_stop_sensor_events(&reports);
        // 只有明確確認的受器才清掉舊世代留下的未解決記錄——而且必須是這個
        // **還登記著**的來源自己說的（誠實：新裝置不能替舊裝置作證）。
        let confirmed: Vec<String> = reports
            .iter()
            .filter(|r| r.resolves_unresolved_stops())
            .flat_map(|r| r.sensors.clone())
            .collect();
        self.resolve_stops_for(&source.source_id(), &confirmed)
            .await;
        let released = source.release(target.as_deref(), reason).await;
        Some(serde_json::json!({
            "sourceId": source.source_id(),
            "target": target,
            "reason": reason,
            "reports": reports,
            "released": released,
        }))
    }
}

/// 把過期的孤兒記錄從即時表裡取出（**取出**，不是丟掉：呼叫端要把它們轉進
/// 「未解決停止」摘要）。
fn drain_expired(
    map: &mut BTreeMap<SourceKey, OrphanedCaptures>,
    now: std::time::Instant,
) -> Vec<(SourceKey, OrphanedCaptures)> {
    let expired: Vec<SourceKey> = map
        .iter()
        .filter(|(_, entry)| now.duration_since(entry.at) >= ORPHAN_CAPTURE_VISIBLE)
        .map(|(key, _)| key.clone())
        .collect();
    expired
        .into_iter()
        .filter_map(|key| map.remove(&key).map(|entry| (key, entry)))
        .collect()
}

/// `Weak<RuntimeInner>` → `Runtime`：來源是被 Runtime 持有的，反向只能持弱參考
/// （否則 Runtime 永遠不會被釋放）。
pub(crate) fn upgrade(weak: &Weak<RuntimeInner>) -> Option<Runtime> {
    weak.upgrade().map(Runtime::from_inner)
}
