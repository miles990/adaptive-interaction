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

/// 來源被移除、但移除當下它還在擷取：那一筆「可能還在擷取」要留在
/// `activeSensors` 多久。永遠留著等於永久誤報（我們確實不知道它停了沒有），
/// 一移除就消失則是靜默——取有界的中間值，並且一定留稽核。
pub const ORPHAN_CAPTURE_VISIBLE: Duration = Duration::from_secs(60);

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

/// 被移除、但移除當下還在擷取的來源留下的「可能還在擷取」記錄（有界可見）。
#[derive(Debug, Clone)]
pub(crate) struct OrphanedCaptures {
    pub(crate) captures: Vec<SensorUse>,
    pub(crate) at: std::time::Instant,
}

/// 來源登記表：有界、以 `source_id` 為鍵。
pub(crate) type SensorSourceMap = BTreeMap<String, Arc<dyn SensorSource>>;

impl Runtime {
    /// 登記一個感測來源。同一個 `source_id` 再登記一次＝取代（來源自己就是
    /// 那件事的完整事實）。超過 [`MAX_SENSOR_SOURCES`] 時誠實拒絕並留稽核。
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
        map.insert(id.clone(), source);
        drop(map);
        // 來源被取代／新登記時，舊的「可能還在擷取」記錄不再適用。
        self.orphan_captures.write().await.remove(&id);
        Ok(())
    }

    /// 移除一個感測來源。移除當下它還在擷取的話，那一筆擷取**不得靜默消失**：
    /// 記成有界可見的「停止結果未知」、補發事件、並留永久稽核。
    pub async fn unregister_sensor_source(&self, source_id: &str) -> bool {
        let removed = self.sensor_sources.write().await.remove(source_id);
        let Some(source) = removed else {
            return false;
        };
        let captures = source.active_captures().await;
        if captures.is_empty() {
            self.orphan_captures.write().await.remove(source_id);
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
        let now = std::time::Instant::now();
        let mut orphans = self.orphan_captures.write().await;
        // 有界：先清掉過期的；還是滿的話丟最舊的一筆（並留痕）。來源反覆
        // 登記／移除不得讓這張表與 `activeSensors` 無界成長。
        orphans.retain(|_, entry| now.duration_since(entry.at) < ORPHAN_CAPTURE_VISIBLE);
        let mut dropped = None;
        if orphans.len() >= MAX_SENSOR_SOURCES && !orphans.contains_key(source_id) {
            if let Some(oldest) = orphans
                .iter()
                .min_by_key(|(_, entry)| entry.at)
                .map(|(id, _)| id.clone())
            {
                orphans.remove(&oldest);
                dropped = Some(oldest);
            }
        }
        orphans.insert(
            source_id.to_string(),
            OrphanedCaptures {
                captures: stale,
                at: now,
            },
        );
        drop(orphans);
        if let Some(dropped) = dropped {
            // 被丟掉的那一筆**從來沒有**被說成已停止：稽核是它最後的痕跡。
            let _ = self.store.audit(
                "sensor.removed-capture-record-dropped",
                "runtime",
                &serde_json::json!({
                    "sourceId": dropped,
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
            &serde_json::json!({"sourceId": source_id, "sensors": sensors}),
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
        self.sensor_sources.read().await.values().cloned().collect()
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
        if let Some(source) = sources.get(provider_id) {
            return Some((source.clone(), None));
        }
        for (id, source) in sources.iter() {
            if let Some(rest) = provider_id.strip_prefix(&format!("{id}.")) {
                if !rest.is_empty() {
                    return Some((source.clone(), Some(rest.to_string())));
                }
            }
        }
        None
    }

    /// 「來源被移除時還在擷取」的有界可見記錄（過期的順手清掉）。
    pub(crate) async fn orphaned_captures(&self) -> Vec<SensorUse> {
        let now = std::time::Instant::now();
        let mut map = self.orphan_captures.write().await;
        map.retain(|_, entry| now.duration_since(entry.at) < ORPHAN_CAPTURE_VISIBLE);
        map.values()
            .flat_map(|entry| entry.captures.iter().cloned())
            .collect()
    }

    /// 這個來源之後確認停止了：清掉它的「可能還在擷取」記錄（誠實：已經知道
    /// 停了就不該繼續嚇人）。
    pub(crate) async fn clear_orphaned_captures(&self, source_id: &str) {
        self.orphan_captures.write().await.remove(source_id);
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
        if !reports.is_empty() && reports.iter().all(|r| r.confirmed()) {
            self.clear_orphaned_captures(&source.source_id()).await;
        }
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

/// `Weak<RuntimeInner>` → `Runtime`：來源是被 Runtime 持有的，反向只能持弱參考
/// （否則 Runtime 永遠不會被釋放）。
pub(crate) fn upgrade(weak: &Weak<RuntimeInner>) -> Option<Runtime> {
    weak.upgrade().map(Runtime::from_inner)
}
