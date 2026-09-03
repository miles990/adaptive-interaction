//! Sensor use indicators + the microphone listen flow.
//!
//! Invariants:
//! - A sensor is EITHER visibly in use (status/events/tray/companion all show
//!   it) or not capturing at all. There is no silent capture path.
//! - begin_listen is refused without: no estop, receptor enabled, AND an
//!   explicit session consent for the receptor (deterministic, in Rust).
//! - Emergency stop halts capture immediately.

use crate::mobile::MobileStopOutcome;
use crate::runtime::Runtime;
use interaction_core::{ConsentScope, DomainError, DomainResult, EventType, Timestamp};
use serde_json::json;
use std::collections::BTreeMap;

/// 正在感測。
pub const SENSOR_STATE_ACTIVE: &str = "active";
/// 已要求停止，還在等來源確認（誠實：requested ≠ stopped）。
pub const SENSOR_STATE_STOPPING: &str = "stopping";
/// 已要求停止但沒有在有界時間內收到確認——來源可能仍在擷取。
pub const SENSOR_STATE_STOP_UNKNOWN: &str = "stop-unknown";

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

/// 「停止所有感測」的誠實報告（本機＋每一台已連線 iPhone）。
///
/// 誠實階梯：`stopped` 只有在**所有**來源都確認停止時才是 true；
/// 任何一台沒回覆就是 `uncertain`（手機可能還在錄音），不得謊稱成功。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopAllSensorsReport {
    pub stopped: bool,
    pub uncertain: bool,
    pub local: LocalStopReport,
    pub devices: Vec<MobileStopOutcome>,
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

    /// 目前所有「正在感測」的來源＝本機擷取＋已連線手機自報的高風險感測。
    /// `status.activeSensors` 用這一個（tray／首頁／角色視窗都吃它）——
    /// 手機的麥克風也是感測，不得只在手機上亮著、桌面卻一片安靜。
    pub async fn active_sensors_all(&self) -> Vec<SensorUse> {
        let mut all = self.active_sensors();
        all.extend(self.mobile_active_sensors().await);
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

    /// 依每台手機的結果補發事件：確認停止的那台在連線 loop 已發過
    /// `sensor.stopped`（以 mic_since 變化為準），這裡只補「結果未知」。
    pub(crate) fn emit_stop_sensor_events(&self, devices: &[MobileStopOutcome]) {
        for device in devices {
            if device.outcome == crate::mobile::StopOutcome::Stopped {
                continue;
            }
            self.events.emit(
                EventType::SensorStopUncertain,
                json!({
                    "sensor": "iphone.mic-level",
                    "deviceId": device.device_id,
                    "outcome": device.outcome.as_str(),
                    "waitedMs": device.waited_ms,
                }),
            );
        }
    }

    /// 立刻停止**所有**感測來源（使用者動作或 estop 路徑）：本機擷取先停，
    /// 再要求每一台已連線 iPhone 停止感測並有界等待確認。
    ///
    /// 誠實階梯：回傳的 `stopped` 只有在所有來源都確認時才是 true；手機沒
    /// 回覆＝`uncertain`（它可能還在錄音），絕不謊稱已停。
    pub async fn stop_all_sensors(&self, actor: &str) -> DomainResult<StopAllSensorsReport> {
        let local = self.stop_local_capture();
        let devices = self
            .mobile_stop_sensors(actor, "mobile.stop-sensors", "stop-all-sensors")
            .await;
        let report = StopAllSensorsReport {
            stopped: devices
                .iter()
                .all(|d| d.outcome == crate::mobile::StopOutcome::Stopped),
            uncertain: devices
                .iter()
                .any(|d| d.outcome != crate::mobile::StopOutcome::Stopped),
            local,
            devices,
        };
        self.emit_stop_sensor_events(&report.devices);
        self.store.audit(
            "sensor.stopped-all",
            actor,
            &serde_json::to_value(&report).unwrap_or_else(|_| json!({})),
        )?;
        Ok(report)
    }
}
