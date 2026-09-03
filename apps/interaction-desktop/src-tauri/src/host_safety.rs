//! 可信 host 的安全狀態視圖（tray 與 overlay 視窗共用同一份推導）。
//!
//! 「感測不靜默」與「緊急停止必須看得見」不能交給角色 renderer：第三方角色、
//! 崩潰的 WebView、被隱藏的角色視窗都不可信（CPP README §9）。這裡由 Rust 從
//! Runtime `status` JSON 推導出固定文案需要的旗標；`refresh_tray` 用它更新 tray，
//! 並以 `emit_to("overlay", "host-safety", view)` 餵給 overlay 視窗。overlay 本身
//! 不讀任何 Runtime 狀態、不呼叫 `api.*`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 啟動寬限（秒）：app 剛啟動、Runtime 仍在 `Starting` 時，「離線」還不是警示
/// （否則每次開機都會閃一下「Runtime 離線」）。超過寬限仍未就緒就誠實顯示離線。
pub const STARTING_GRACE_SECS: u64 = 30;
// 寬限必須有界：太短會每次開機閃「離線」，太長會把真的故障藏起來。
const _: () = assert!(STARTING_GRACE_SECS >= 10 && STARTING_GRACE_SECS <= 60);

/// Host → overlay 的事件名稱。
pub const HOST_SAFETY_EVENT: &str = "host-safety";

/// 一個啟用中的感測器（對應 `status.activeSensors[]`，欄位同 Runtime `SensorUse`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorView {
    pub kind: String,
    #[serde(default)]
    pub started_by: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_stop_at: Option<String>,
    /// `active`／`stopping`（已要求停止、等來源確認）／`stop-unknown`
    /// （沒在有界時間內確認）。舊 daemon 沒有這個欄位＝視為 active。
    /// **停止中與結果未知仍然算感測中**：不得因為 state 不是 active 就不顯示。
    #[serde(default)]
    pub state: String,
}

/// 已要求停止但尚未確認。
pub const SENSOR_STATE_STOPPING: &str = "stopping";
/// 已要求停止、來源沒回覆：可能仍在擷取（誠實：未知 ≠ 已停）。
pub const SENSOR_STATE_STOP_UNKNOWN: &str = "stop-unknown";

/// Host 安全視圖：tray 與 overlay 的唯一資料來源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSafetyView {
    /// Runtime 是否可達（supervisor 就緒且 `status` 取得成功）。
    pub reachable: bool,
    /// 仍在啟動寬限內（不可達，但尚不算「離線」警示）。
    pub starting: bool,
    pub estop: bool,
    pub paused: bool,
    pub mic_active: bool,
    pub camera_active: bool,
    pub sensors: Vec<SensorView>,
    /// overlay 是否應顯示：estop ∨ 有感測 ∨（不可達 ∧ 非啟動寬限）。
    /// 由 Rust 算好帶過去，TS 端不必重複規則。
    pub active: bool,
    /// RFC3339。
    pub at: String,
}

/// 麥克風類感測：本機 `microphone` 與手機 `iphone.mic-level` 都算——手機的
/// 麥克風也是麥克風，不得只在手機上亮著、桌面卻一片安靜。
pub fn is_mic_kind(kind: &str) -> bool {
    kind == "microphone" || kind.contains("mic")
}

pub fn is_camera_kind(kind: &str) -> bool {
    kind.contains("camera")
}

impl HostSafetyView {
    /// 從 Runtime `status` JSON 推導。`status = None` 代表取不到狀態（不可達）。
    /// `starting` 由呼叫端依 supervisor 狀態與啟動時間決定。
    pub fn derive(
        supervisor_ready: bool,
        starting: bool,
        status: Option<&Value>,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        let reachable = supervisor_ready && status.is_some();
        let (mut estop, mut paused) = (false, false);
        let mut sensors = Vec::new();
        if let Some(s) = status {
            estop = s
                .get("emergencyStop")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            paused = s
                .pointer("/proactivePause/paused")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(list) = s.get("activeSensors").and_then(Value::as_array) {
                for item in list {
                    let Some(kind) = item.get("kind").and_then(Value::as_str) else {
                        continue;
                    };
                    let text = |key: &str| {
                        item.get(key)
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string()
                    };
                    sensors.push(SensorView {
                        kind: kind.to_string(),
                        started_by: text("startedBy"),
                        purpose: text("purpose"),
                        auto_stop_at: item
                            .get("autoStopAt")
                            .and_then(Value::as_str)
                            .map(String::from),
                        state: text("state"),
                    });
                }
            }
        }
        let mic_active = sensors.iter().any(|s| is_mic_kind(&s.kind));
        let camera_active = sensors.iter().any(|s| is_camera_kind(&s.kind));
        let starting = starting && !reachable;
        let active = estop || !sensors.is_empty() || (!reachable && !starting);
        HostSafetyView {
            reachable,
            starting,
            estop,
            paused,
            mic_active,
            camera_active,
            sensors,
            active,
            at: at.to_rfc3339(),
        }
    }

    /// 感測文字（tray／overlay 共用；`None` = 沒有感測在用）。永遠是文字，
    /// 不靠顏色或 glyph 單獨表達。
    pub fn sensor_text(&self) -> Option<String> {
        if self.sensors.is_empty() {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        match (self.mic_active, self.camera_active) {
            (true, true) => parts.push("麥克風＋攝影機使用中".into()),
            (true, false) => parts.push("麥克風使用中".into()),
            (false, true) => parts.push("攝影機使用中".into()),
            (false, false) => {}
        }
        let others: Vec<&str> = self
            .sensors
            .iter()
            .filter(|s| !is_mic_kind(&s.kind) && !is_camera_kind(&s.kind))
            .map(|s| s.kind.as_str())
            .collect();
        if !others.is_empty() {
            parts.push(format!("感測使用中（{}）", others.join("、")));
        }
        // 停止中／結果未知也要說出來——「已要求停止」不等於「已經停了」。
        if self
            .sensors
            .iter()
            .any(|s| s.state == SENSOR_STATE_STOP_UNKNOWN)
        {
            parts.push("停止結果未知（來源未回覆）".into());
        } else if self
            .sensors
            .iter()
            .any(|s| s.state == SENSOR_STATE_STOPPING)
        {
            parts.push("停止中（等待確認）".into());
        }
        Some(parts.join("、"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    #[test]
    fn derives_from_status_including_mobile_microphone() {
        // 本機麥克風＋手機麥克風音量（kind 不是 "microphone"）都要算麥克風。
        let status = json!({
            "emergencyStop": false,
            "proactivePause": {"paused": false},
            "agentSessions": 1,
            "activeSensors": [
                {"kind": "microphone", "startedAt": "2026-09-02T00:00:00Z",
                 "startedBy": "user", "purpose": "click-to-listen",
                 "autoStopAt": "2026-09-02T00:00:10Z"},
                {"kind": "iphone.mic-level", "startedAt": "2026-09-02T00:00:00Z",
                 "startedBy": "iphone:abc", "purpose": "iPhone 麥克風音量（僅音量值）"}
            ]
        });
        let v = HostSafetyView::derive(true, false, Some(&status), now());
        assert!(v.reachable);
        assert!(!v.starting);
        assert!(!v.estop);
        assert!(v.mic_active);
        assert!(!v.camera_active);
        assert_eq!(v.sensors.len(), 2);
        assert_eq!(
            v.sensors[0].auto_stop_at.as_deref(),
            Some("2026-09-02T00:00:10Z")
        );
        assert_eq!(v.sensors[1].started_by, "iphone:abc");
        assert!(v.active, "an active sensor must show the overlay");
        assert_eq!(v.sensor_text().as_deref(), Some("麥克風使用中"));

        // 只有手機麥克風時也要亮。
        let status = json!({"activeSensors": [{"kind": "iphone.mic-level"}]});
        let v = HostSafetyView::derive(true, false, Some(&status), now());
        assert!(v.mic_active && v.active);
    }

    #[test]
    fn camera_and_other_sensor_kinds_are_never_silent() {
        let status = json!({"activeSensors": [
            {"kind": "camera", "startedBy": "user", "purpose": "test"},
            {"kind": "lidar", "startedBy": "user", "purpose": "test"}
        ]});
        let v = HostSafetyView::derive(true, false, Some(&status), now());
        assert!(v.camera_active && !v.mic_active && v.active);
        let text = v.sensor_text().expect("sensor text");
        assert!(text.contains("攝影機使用中"));
        assert!(
            text.contains("lidar"),
            "unknown kinds still get text: {text}"
        );

        let both = json!({"activeSensors": [{"kind": "microphone"}, {"kind": "camera"}]});
        let v = HostSafetyView::derive(true, false, Some(&both), now());
        assert_eq!(v.sensor_text().as_deref(), Some("麥克風＋攝影機使用中"));
    }

    /// 「停止中／結果未知」仍然是感測中：不得從 tray／overlay 消失，
    /// 而且文字要說出結果未知（誠實：已要求停止 ≠ 已經停了）。
    #[test]
    fn a_sensor_being_stopped_is_never_hidden_and_says_so() {
        let stopping = json!({"activeSensors": [
            {"kind": "iphone.mic-level", "startedBy": "iphone:abc",
             "purpose": "iPhone 麥克風音量：停止中（等待 iPhone 確認）",
             "state": "stopping"}
        ]});
        let v = HostSafetyView::derive(true, false, Some(&stopping), now());
        assert!(v.mic_active && v.active, "停止中仍要亮");
        let text = v.sensor_text().expect("sensor text");
        assert!(text.contains("麥克風使用中"), "{text}");
        assert!(text.contains("停止中"), "{text}");

        let unknown = json!({"activeSensors": [
            {"kind": "iphone.mic-level", "startedBy": "iphone:abc",
             "purpose": "停止結果未知", "state": "stop-unknown"}
        ]});
        let v = HostSafetyView::derive(true, false, Some(&unknown), now());
        assert!(v.mic_active && v.active, "結果未知一定不能消失");
        let text = v.sensor_text().expect("sensor text");
        assert!(text.contains("結果未知"), "{text}");

        // 舊 daemon 沒有 state 欄位 → 視為 active（相容，不多話）。
        let legacy = json!({"activeSensors": [{"kind": "microphone"}]});
        let v = HostSafetyView::derive(true, false, Some(&legacy), now());
        assert_eq!(v.sensors[0].state, "");
        assert_eq!(v.sensor_text().as_deref(), Some("麥克風使用中"));
    }

    #[test]
    fn estop_shows_even_with_nothing_else() {
        let status = json!({"emergencyStop": true, "activeSensors": []});
        let v = HostSafetyView::derive(true, false, Some(&status), now());
        assert!(v.estop && v.active);
        assert!(v.sensor_text().is_none());
    }

    #[test]
    fn paused_alone_is_informational_not_an_alert() {
        let status = json!({"proactivePause": {"paused": true}, "activeSensors": []});
        let v = HostSafetyView::derive(true, false, Some(&status), now());
        assert!(v.paused);
        assert!(!v.active, "pause must not pop the safety overlay");
    }

    #[test]
    fn offline_shows_unless_within_starting_grace() {
        // 取不到 status → 不可達 → 離線警示。
        let v = HostSafetyView::derive(true, false, None, now());
        assert!(!v.reachable && v.active);
        // supervisor 未就緒亦同（即使拿到了 JSON）。
        let s = json!({});
        let v = HostSafetyView::derive(false, false, Some(&s), now());
        assert!(!v.reachable && v.active);
        // 啟動寬限內：不可達但不警示。
        let v = HostSafetyView::derive(false, true, None, now());
        assert!(!v.reachable && v.starting && !v.active);
        // 可達時 starting 一律清掉。
        let v = HostSafetyView::derive(true, true, Some(&s), now());
        assert!(v.reachable && !v.starting && !v.active);
    }

    #[test]
    fn serializes_camel_case_for_the_overlay() {
        let status = json!({"emergencyStop": true, "activeSensors": [
            {"kind": "microphone", "startedBy": "user", "purpose": "p", "autoStopAt": "x"}
        ]});
        let v = HostSafetyView::derive(true, false, Some(&status), now());
        let json = serde_json::to_value(&v).expect("serialize");
        for key in [
            "reachable",
            "starting",
            "estop",
            "paused",
            "micActive",
            "cameraActive",
            "sensors",
            "active",
            "at",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
        assert_eq!(json["sensors"][0]["startedBy"], "user");
        assert_eq!(json["sensors"][0]["autoStopAt"], "x");
    }
}
