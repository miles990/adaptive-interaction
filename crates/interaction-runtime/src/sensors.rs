//! Sensor use indicators + the microphone listen flow.
//!
//! Invariants:
//! - A sensor is EITHER visibly in use (status/events/tray/companion all show
//!   it) or not capturing at all. There is no silent capture path.
//! - begin_listen is refused without: no estop, receptor enabled, AND an
//!   explicit session consent for the receptor (deterministic, in Rust).
//! - Emergency stop halts capture immediately.

use crate::mobile::MobileStopOutcome;
use crate::runtime::{Runtime, RuntimeInner};
use crate::sensor_source::{SensorSource, SensorStopReport, SensorStopStatus};
use interaction_core::{ConsentScope, DomainError, DomainResult, EventType, ReceptorId, Timestamp};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Weak;
use std::time::Duration;

/// 遠端感測來源對「停止所有感測」的誠實回報介面。
///
/// 這個檔案不認識任何具體裝置：要補「可能還在擷取」的事件時，它只問來源三件
/// 事——你是誰、你這次涵蓋哪些高風險受器（由 provider 自己宣告，核心不猜）、
/// 你確認停了沒有。新增一種行動裝置只要實作這個 trait，不必改這裡。
pub trait SensorStopOutcome {
    /// 來源識別碼（裝置 id 之類）。只進 payload，不進人話標題。
    fn source_id(&self) -> &str;
    /// 這次停止涵蓋的高風險受器 id（provider 宣告的清單）。
    fn sensor_ids(&self) -> Vec<String>;
    /// 結果字串（`stopped`／`unknown`／`unreachable` 這一類）。
    fn outcome_label(&self) -> &str;
    /// 實際等待毫秒（有界）。
    fn waited_ms(&self) -> u64;
    /// 來源**明確確認**已停止感測嗎？未確認一律當成「可能還在擷取」。
    fn confirmed_stopped(&self) -> bool;
}

/// 純函式：把「沒有確認停止」的來源攤成要補發的事件 payload。
///
/// 誠實階梯：確認停止的來源在連線 loop 已經發過 `sensor.stopped`，這裡只補
/// 「結果未知」——requested ≠ stopped，未確認不得被算成已停。
pub fn sensor_stop_uncertain_payloads<T: SensorStopOutcome>(outcomes: &[T]) -> Vec<Value> {
    let mut payloads = Vec::new();
    for outcome in outcomes {
        if outcome.confirmed_stopped() {
            continue;
        }
        for sensor in outcome.sensor_ids() {
            payloads.push(json!({
                "sensor": sensor,
                "deviceId": outcome.source_id(),
                "outcome": outcome.outcome_label(),
                "waitedMs": outcome.waited_ms(),
            }));
        }
    }
    payloads
}

/// 正在感測。
pub const SENSOR_STATE_ACTIVE: &str = "active";
/// 已要求停止，還在等來源確認（誠實：requested ≠ stopped）。
pub const SENSOR_STATE_STOPPING: &str = "stopping";
/// 已要求停止但沒有在有界時間內收到確認——來源可能仍在擷取。
pub const SENSOR_STATE_STOP_UNKNOWN: &str = "stop-unknown";

/// 停止結果未知的另一種成因：這個高風險受器由 provider 宣告，但**沒有任何來源**
/// 在停止掃描裡回報涵蓋它（沒有停止管道）。requested≠stopped，這裡連 requested
/// 都談不上。
pub const SENSOR_STOP_NO_PATH: &str = "no-stop-path";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorUse {
    pub kind: String,
    pub started_at: Timestamp,
    pub started_by: String,
    pub purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_stop_at: Option<Timestamp>,
    /// `active`／`stopping`／`stop-unknown`。停止中與結果未知**仍然是感測中**，
    /// 不得因此從 status／tray／UI 消失（感測不靜默）。
    pub state: String,
}

/// 本機擷取的停止結果。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStopReport {
    /// `stopped`＝本來在擷取、已停；`idle`＝本來就沒在擷取。
    pub microphone: String,
}

/// 「停止所有感測」的誠實報告（本機＋每一台已連線的遠端來源）。
///
/// 誠實階梯：`stopped` 只有在**所有**來源都確認停止時才是 true；
/// 任何一台沒回覆就是 `uncertain`（它可能還在錄音），不得謊稱成功。
///
/// `devices` 仍是 `Vec<MobileStopOutcome>` 這個具體型別：它是 HTTP／CLI／桌面
/// 共用的回傳形狀（前端逐欄位讀），改形狀就是改 wire 契約。行為上這條路徑
/// 已經完全走 [`SensorSource`]——`devices` 只是把行動裝置那一族的結果投影回
/// 既有欄位；其他來源放在 `sources`（新欄位，舊介面看不到也不會壞）。
///
/// `stopped`／`uncertain` **同時**看兩個陣列＋「沒有停止管道」的高風險受器：
/// 任何一項沒確認就不是 stopped。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopAllSensorsReport {
    pub stopped: bool,
    pub uncertain: bool,
    pub local: LocalStopReport,
    pub devices: Vec<MobileStopOutcome>,
    /// 非行動裝置來源的逐筆結果（本機擷取投影在 `local`，不重複列）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SensorStopReport>,
}

/// 本機擷取來源的登記 id。`local` 欄位就是從它的報告投影出來的。
pub const LOCAL_SENSOR_SOURCE_ID: &str = "local.capture";
/// 本機擷取來源對應的能力宣告 id。
pub const LOCAL_SENSOR_DECLARATION_ID: &str = "provider.local.capture";
/// 本機麥克風受器 id（本機來源宣告它涵蓋這一個）。
pub const LOCAL_MIC_RECEPTOR_ID: &str = "microphone.listen";

/// 停止感測的 audit reason：使用者按了「停止所有感測」。
pub const SENSOR_STOP_REASON_USER: &str = "stop-all-sensors";
/// 停止感測的 audit reason：緊急停止。
pub const SENSOR_STOP_REASON_EMERGENCY: &str = "emergency-stop";
/// 停止感測的 audit reason：這個 provider 被撤銷／停用了（X2）。
/// **不是**緊急停止：來源（例如手機）要顯示「由桌面停止」而不是緊急停止那一句。
pub const SENSOR_STOP_REASON_PROVIDER_OFF: &str = "provider-stopped";
/// 停止感測的 audit reason：這台宣告式裝置正在**重新綁定**（AIP 1.0 澄清／
/// v0.7.0）。舊連線要被換掉，所以先請還在的來源停下來——不是使用者按的，
/// 也不是緊急停止。
pub const SENSOR_STOP_REASON_REBIND: &str = "provider-rebinding";

/// 一次「停止所有感測」掃描的原始結果（還沒投影成 wire 形狀）。
///
/// 誠實階梯集中在這裡：`stopped` 要求**每一份**報告都確認、而且沒有任何
/// 宣告過卻沒人涵蓋的高風險受器；只要有一項沒確認就是 `uncertain`。
/// 「停止所有感測」與緊急停止都用這一份，兩條路徑因此不可能各說各話。
#[derive(Debug, Clone)]
pub struct SensorStopSweep {
    /// 本機擷取的結果（從本機來源的報告投影）。
    pub local: LocalStopReport,
    /// 每一個來源的逐筆結果（含本機與行動裝置）。
    pub reports: Vec<SensorStopReport>,
    /// 宣告了、仍啟用、但這次掃描沒有任何來源涵蓋的高風險受器。
    pub unreported: Vec<String>,
}

impl SensorStopSweep {
    /// 全部確認停止（沒有任何「可能還在擷取」）。
    pub fn stopped(&self) -> bool {
        self.reports.iter().all(|r| r.confirmed()) && self.unreported.is_empty()
    }

    /// 有任何一項沒確認（來源沒回覆、拒絕、送不出去，或根本沒有停止管道）。
    pub fn uncertain(&self) -> bool {
        self.reports.iter().any(|r| !r.confirmed()) || !self.unreported.is_empty()
    }

    /// 投影成 HTTP／CLI／桌面共用的 wire 形狀。
    pub fn into_report(self) -> StopAllSensorsReport {
        let devices = crate::mobile::mobile_wire_outcomes(&self.reports);
        let sources: Vec<SensorStopReport> = self
            .reports
            .iter()
            .filter(|r| {
                r.declaration_id != crate::mobile::MOBILE_PROVIDER_DECLARATION_ID
                    && r.declaration_id != LOCAL_SENSOR_DECLARATION_ID
            })
            .cloned()
            .collect();
        StopAllSensorsReport {
            stopped: self.stopped(),
            uncertain: self.uncertain(),
            local: self.local,
            devices,
            sources,
        }
    }
}

/// 本機擷取（麥克風）作為一個一般的 [`SensorSource`]。
///
/// 為什麼要包起來：本機以前是停止路徑裡唯一的特例（協調器直接呼叫
/// `stop_local_capture`）。特例會漂移——緊急停止那條路徑就曾經自己手刻一份
/// 報告。現在本機只是「停止是同步的」那一種來源，其餘一視同仁。
pub(crate) struct LocalMicSensorSource {
    runtime: Weak<RuntimeInner>,
}

impl LocalMicSensorSource {
    pub(crate) fn new(runtime: Weak<RuntimeInner>) -> Self {
        LocalMicSensorSource { runtime }
    }
}

#[async_trait::async_trait]
impl SensorSource for LocalMicSensorSource {
    fn source_id(&self) -> String {
        LOCAL_SENSOR_SOURCE_ID.to_string()
    }

    fn declaration_id(&self) -> String {
        LOCAL_SENSOR_DECLARATION_ID.to_string()
    }

    /// 本機停止沒有任何 await：協調器先跑它，不排在遠端等待後面。
    fn stops_immediately(&self) -> bool {
        true
    }

    /// `stop_listen` 會同步回呼 `sensor_state_changed` → 已經發過
    /// `sensor.stopped`，協調器不重發。
    fn reports_own_stop_events(&self) -> bool {
        true
    }

    async fn active_captures(&self) -> Vec<SensorUse> {
        match crate::sensor_source::upgrade(&self.runtime) {
            Some(rt) => rt.active_sensors(),
            None => vec![],
        }
    }

    async fn request_stop(
        &self,
        _target: Option<&str>,
        _deadline: Duration,
        _reason: &str,
    ) -> Vec<SensorStopReport> {
        let Some(rt) = crate::sensor_source::upgrade(&self.runtime) else {
            return vec![];
        };
        let local = rt.stop_local_capture();
        let outcome = if local.microphone == "stopped" {
            SensorStopStatus::Stopped
        } else {
            SensorStopStatus::AlreadyStopped
        };
        vec![SensorStopReport::new(
            LOCAL_SENSOR_SOURCE_ID,
            LOCAL_SENSOR_DECLARATION_ID,
            vec![LOCAL_MIC_RECEPTOR_ID.to_string()],
            outcome,
            0,
        )]
    }
}

impl Runtime {
    /// Currently-capturing sensors (empty = nothing is listening/watching).
    pub fn active_sensors(&self) -> Vec<SensorUse> {
        self.sensors
            .lock()
            .expect("sensors lock")
            .values()
            .cloned()
            .collect()
    }

    /// 目前所有「正在感測」的來源＝每一個登記中的 [`SensorSource`] 自報的擷取
    /// ＋被移除時還在擷取、還沒確認停止的那些（有界可見）。
    /// `status.activeSensors` 用這一個（tray／首頁／角色視窗都吃它）——
    /// 手機的麥克風也是感測，不得只在手機上亮著、桌面卻一片安靜。
    pub async fn active_sensors_all(&self) -> Vec<SensorUse> {
        let sources = self.sensor_sources_snapshot().await;
        let mut all = Vec::new();
        // 本機來源沒有登記時（理論上不會發生）也不得讓本機擷取消失。
        if !sources
            .iter()
            .any(|s| s.source_id() == LOCAL_SENSOR_SOURCE_ID)
        {
            all.extend(self.active_sensors());
        }
        for source in sources {
            all.extend(source.active_captures().await);
        }
        all.extend(self.orphaned_captures().await);
        all
    }

    pub(crate) fn sensor_state_changed(&self, kind: &str, active: bool) {
        {
            let mut map = self.sensors.lock().expect("sensors lock");
            if active {
                map.entry(kind.to_string()).or_insert(SensorUse {
                    kind: kind.to_string(),
                    started_at: chrono::Utc::now(),
                    started_by: "user".into(),
                    purpose: "click-to-listen".into(),
                    auto_stop_at: None,
                    state: SENSOR_STATE_ACTIVE.to_string(),
                });
            } else {
                map.remove(kind);
            }
        }
        self.events.emit(
            if active {
                EventType::SensorStarted
            } else {
                EventType::SensorStopped
            },
            json!({"sensor": kind}),
        );
    }

    /// Begin one bounded microphone listen window. Deterministic gates:
    /// estop → refused; receptor disabled → refused; no explicit session
    /// consent for this receptor → refused.
    pub async fn begin_mic_listen(
        &self,
        duration_ms: u64,
        actor: &str,
    ) -> DomainResult<BTreeMap<String, serde_json::Value>> {
        if self.is_estopped() {
            return Err(DomainError::PolicyBlocked(
                "emergency stop engaged; sensors stay off".into(),
            ));
        }
        let id = interaction_core::ReceptorId::new("microphone.listen");
        // Enabled check: the registry refuses disabled/unregistered receptors.
        let manifests = self.registry.receptor_manifests().await;
        let manifest = manifests
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| DomainError::NotFound("microphone.listen".into()))?;
        if !manifest.availability.is_available() {
            return Err(DomainError::PolicyBlocked(format!(
                "microphone.listen is {:?} (enable it first)",
                manifest.availability
            )));
        }
        // Explicit consent for THIS receptor in the current session.
        let session = self
            .current_session()
            .await
            .ok_or_else(|| DomainError::SessionInactive("no active session".into()))?;
        let scope = ConsentScope::Receptor("microphone.listen".into());
        if !session.has_consent(&scope, chrono::Utc::now()) {
            return Err(DomainError::ConsentRequired(
                "microphone.listen needs an explicit session consent \
                 (receptor:microphone.listen)"
                    .into(),
            ));
        }
        let mic = self
            .mic_receptor
            .as_ref()
            .ok_or_else(|| DomainError::NotFound("microphone receptor not present".into()))?;
        mic.begin_listen(duration_ms)
            .map_err(|e| DomainError::PolicyBlocked(e.to_string()))?;
        // Re-check estop AFTER opening: an emergency stop that engaged during
        // the await points above (registry/session reads) may have already run
        // its stop_all_sensors sweep before this window existed. Close it now
        // so a concurrent estop can never leave the mic capturing.
        if self.is_estopped() {
            mic.stop_listen();
            return Err(DomainError::PolicyBlocked(
                "emergency stop engaged during start; capture aborted".into(),
            ));
        }
        {
            let mut map = self.sensors.lock().expect("sensors lock");
            if let Some(u) = map.get_mut("microphone") {
                u.started_by = actor.to_string();
                u.auto_stop_at = Some(
                    chrono::Utc::now()
                        + chrono::Duration::milliseconds(
                            duration_ms.clamp(500, adapters_media::MAX_LISTEN_MS) as i64,
                        ),
                );
            }
        }
        self.store.audit(
            "sensor.microphone.started",
            actor,
            &json!({"durationMs": duration_ms}),
        )?;
        Ok(BTreeMap::from([("listening".to_string(), json!(true))]))
    }

    /// 本機擷取立刻停（同步、沒有任何 await）。任何「要等手機確認」的路徑都
    /// 必須先做這一步——本機麥克風不得排在遠端等待後面。
    /// 回傳 `stopped`（本來在擷取）或 `idle`（本來就沒有）。
    pub(crate) fn stop_local_capture(&self) -> LocalStopReport {
        let was_listening = self
            .sensors
            .lock()
            .expect("sensors lock")
            .contains_key("microphone");
        if let Some(mic) = self.mic_receptor.as_ref() {
            // stop_listen 會同步回呼 sensor_state_changed → 發 sensor.stopped。
            mic.stop_listen();
        }
        LocalStopReport {
            microphone: if was_listening { "stopped" } else { "idle" }.into(),
        }
    }

    /// 依每個來源的結果補發事件：確認停止的那台在連線 loop 已發過
    /// `sensor.stopped`（以感測起訖變化為準），這裡只補「結果未知」。
    /// 受器 id 由來源自己宣告（[`SensorStopOutcome::sensor_ids`]）。
    pub(crate) fn emit_stop_sensor_events<T: SensorStopOutcome>(&self, devices: &[T]) {
        for payload in sensor_stop_uncertain_payloads(devices) {
            self.events.emit(EventType::SensorStopUncertain, payload);
        }
    }

    /// 立刻停止**所有**感測來源（使用者動作或 estop 路徑）：本機擷取先停，
    /// 再要求每一個登記中的來源停止感測並有界等待確認。
    ///
    /// 誠實階梯：回傳的 `stopped` 只有在所有來源都確認時才是 true；來源沒
    /// 回覆＝`uncertain`（它可能還在錄音），絕不謊稱已停。
    pub async fn stop_all_sensors(&self, actor: &str) -> DomainResult<StopAllSensorsReport> {
        let (report, audited) = self
            .stop_all_sensors_with_reason(actor, SENSOR_STOP_REASON_USER)
            .await;
        audited?;
        Ok(report)
    }

    /// 「停止所有感測」與緊急停止**共用**的那一段（X1）：同一個協調器、同一份
    /// 報告、同一組事件與稽核。緊急停止不得比使用者按的那顆按鈕更樂觀，也不得
    /// 自己手刻一份報告。
    ///
    /// 稽核寫入失敗不影響「已經停了」這件事實，所以報告與稽核結果分開回傳：
    /// 使用者路徑照舊把它變成 `Err`，緊急停止路徑只記錄、不因此中斷。
    pub(crate) async fn stop_all_sensors_with_reason(
        &self,
        actor: &str,
        reason: &str,
    ) -> (StopAllSensorsReport, DomainResult<()>) {
        let sweep = self
            .stop_all_sensor_sources(actor, reason, crate::mobile::STOP_SENSORS_WAIT)
            .await;
        let unreported = sweep.unreported.clone();
        let report = sweep.into_report();
        let mut audited = self.store.audit(
            "sensor.stopped-all",
            actor,
            &serde_json::to_value(&report).unwrap_or_else(|_| json!({})),
        );
        if !unreported.is_empty() {
            // 報告本身的形狀是 wire 契約（前端逐欄位讀）：沒被問到的受器記在
            // 稽核裡（可追查、不改契約），事件面則已經補過 stop-uncertain。
            let not_requested = self.store.audit(
                "sensor.stop-not-requested",
                actor,
                &json!({
                    "receptors": unreported,
                    "reason": SENSOR_STOP_NO_PATH,
                }),
            );
            audited = audited.and(not_requested);
        }
        (report, audited)
    }

    /// 停止感測的**唯一**協調器（M2 §3.1）。
    ///
    /// 步驟固定：(1) 本機（同步）來源先停；(2) 其餘來源並行、各自有界等待；
    /// (3) 掃一遍「宣告過但沒有任何來源涵蓋」的高風險受器。
    ///
    /// 有界：每個來源都被包在 `deadline` 的逾時裡（[`Runtime::request_source_stop`]），
    /// 來源自己不守約也不會把停止拖成無限等待；並行是為了不讓一個沒回應的來源
    /// 把其他來源排在後面。
    pub async fn stop_all_sensor_sources(
        &self,
        actor: &str,
        reason: &str,
        deadline: Duration,
    ) -> SensorStopSweep {
        let sources = self.sensor_sources_snapshot().await;
        // 誰要求的、什麼時候、預算多少：先留痕再動手（掃描中途程序死掉時，
        // 至少看得出「有人要求停止」這件事發生過）。
        let _ = self.store.audit(
            "sensor.stop-requested",
            actor,
            &json!({
                "reason": reason,
                "deadlineMs": deadline.as_millis() as u64,
                "sources": sources.iter().map(|s| s.source_id()).collect::<Vec<_>>(),
            }),
        );
        let mut collected: Vec<(usize, Vec<SensorStopReport>)> = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            if source.stops_immediately() {
                collected.push((
                    index,
                    self.request_source_stop(source, None, deadline, reason)
                        .await,
                ));
            }
        }
        let remote: Vec<(usize, &std::sync::Arc<dyn SensorSource>)> = sources
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.stops_immediately())
            .collect();
        let results = futures_util::future::join_all(
            remote
                .iter()
                .map(|(_, source)| self.request_source_stop(source, None, deadline, reason)),
        )
        .await;
        for ((index, _), reports) in remote.iter().zip(results.into_iter()) {
            collected.push((*index, reports));
        }

        let mut reports = Vec::new();
        for (index, source_reports) in collected {
            let source = &sources[index];
            // 確認停止的來源：若它自己的連線 loop 不發事件，這裡補一則
            // `sensor.stopped`（停止也不靜默）。`already-stopped` 本來就沒在
            // 擷取，沒有「停止」這件事可報。
            if !source.reports_own_stop_events() {
                for report in source_reports
                    .iter()
                    .filter(|r| r.outcome == SensorStopStatus::Stopped)
                {
                    for sensor in &report.sensors {
                        self.events.emit(
                            EventType::SensorStopped,
                            json!({
                                "sensor": sensor,
                                "sourceId": report.source_id,
                                "outcome": report.outcome.as_str(),
                            }),
                        );
                    }
                }
            }
            // 這個**還登記著**的來源明確確認停止的受器：把同 id 舊世代留下的
            // 未解決記錄清掉（誠實：只清它確認過的那幾個受器，不整筆抹掉）。
            let confirmed: Vec<String> = source_reports
                .iter()
                .filter(|r| r.confirmed())
                .flat_map(|r| r.sensors.clone())
                .collect();
            self.resolve_stops_for(&source.source_id(), &confirmed)
                .await;
            reports.extend(source_reports);
        }
        // 未確認的一律補「可能還在擷取」（requested ≠ stopped）。
        self.emit_stop_sensor_events(&reports);
        let local = local_stop_report(&reports);
        let unreported = self.unreported_high_risk_receptors(&reports).await;
        self.emit_unreported_high_risk_events(&unreported);
        SensorStopSweep {
            local,
            reports,
            unreported,
        }
    }

    /// 移除一個受器。高風險受器要**先**請涵蓋它的來源停止感測（有界），再移除
    /// 記錄——否則受器記錄消失、來源還在擷取，畫面卻一片安靜（S4 競態）。
    ///
    /// 「哪個來源涵蓋它」一律查能力宣告表，核心不比對任何具名裝置。
    pub async fn unregister_receptor(&self, id: &ReceptorId) -> DomainResult<()> {
        let receptor_id = id.as_str().to_string();
        let high_risk = self
            .capability_declarations()
            .high_risk_receptors()
            .iter()
            .any(|r| r == &receptor_id);
        if high_risk {
            let sources = self.sensor_sources_snapshot().await;
            let mut reports = Vec::new();
            for source in sources.iter().filter(|s| {
                self.capability_declarations()
                    .declaration(&s.declaration_id())
                    .is_some_and(|d| {
                        d.receptors.iter().any(|r| r == &receptor_id)
                            || d.high_risk_receptors.iter().any(|r| r == &receptor_id)
                    })
            }) {
                reports.extend(
                    self.request_source_stop(
                        source,
                        None,
                        crate::mobile::STOP_SENSORS_WAIT,
                        SENSOR_STOP_REASON_USER,
                    )
                    .await,
                );
            }
            if !reports.is_empty() {
                self.emit_stop_sensor_events(&reports);
                let _ = self.store.audit(
                    "sensor.stop-before-receptor-removed",
                    "user",
                    &json!({"receptorId": receptor_id, "reports": reports}),
                );
            }
        }
        self.registry.unregister_receptor(id).await
    }

    /// provider 宣告過、但這次停止掃描沒有任何來源回報涵蓋、而且**目前仍啟用**
    /// 的高風險受器。
    ///
    /// - 涵蓋與否由來源自己說（[`SensorStopOutcome::sensor_ids`]），核心不比對
    ///   任何具名裝置的字面值。
    /// - 宣告表只讀不寫：拿到的是 `&dyn CapabilityDeclarationsView`（唯讀視角），
    ///   停止路徑在型別上就改不了「哪些受器是高風險」。
    /// - 已停用／未註冊的受器不算：registry 對它們回 `Err`，沒有東西能經由它
    ///   流進來，硬報「可能還在擷取」是另一種不誠實。
    async fn unreported_high_risk_receptors<T: SensorStopOutcome>(
        &self,
        reported: &[T],
    ) -> Vec<String> {
        let covered: std::collections::BTreeSet<String> = reported
            .iter()
            .flat_map(|outcome| outcome.sensor_ids())
            .collect();
        let mut unreported = Vec::new();
        for id in self.capability_declarations().high_risk_receptors() {
            if covered.contains(&id) {
                continue;
            }
            if self
                .registry
                .receptor(&interaction_core::ReceptorId::new(&id))
                .await
                .is_ok()
            {
                unreported.push(id);
            }
        }
        unreported
    }

    /// 誠實階梯：沒有停止管道＝結果未知，不是已停。與逾時未確認共用同一種事件，
    /// 讓收件匣、tray、status 都看得見。
    fn emit_unreported_high_risk_events(&self, unreported: &[String]) {
        for sensor in unreported {
            self.events.emit(
                EventType::SensorStopUncertain,
                json!({
                    "sensor": sensor,
                    "outcome": SENSOR_STOP_NO_PATH,
                    "waitedMs": 0,
                }),
            );
        }
    }
}

/// 本機來源的報告 → wire 上的 `local` 欄位。沒有本機來源時誠實回 `idle`
/// （本機擷取只可能由本機來源啟動，沒有它就沒有本機擷取在跑）。
fn local_stop_report(reports: &[SensorStopReport]) -> LocalStopReport {
    let microphone = reports
        .iter()
        .find(|r| r.source_id == LOCAL_SENSOR_SOURCE_ID)
        .map(|r| {
            if r.outcome == SensorStopStatus::Stopped {
                "stopped"
            } else {
                "idle"
            }
        })
        .unwrap_or("idle");
    LocalStopReport {
        microphone: microphone.to_string(),
    }
}
