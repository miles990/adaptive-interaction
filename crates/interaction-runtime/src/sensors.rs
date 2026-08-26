//! Sensor use indicators + the microphone listen flow.
//!
//! Invariants:
//! - A sensor is EITHER visibly in use (status/events/tray/companion all show
//!   it) or not capturing at all. There is no silent capture path.
//! - begin_listen is refused without: no estop, receptor enabled, AND an
//!   explicit session consent for the receptor (deterministic, in Rust).
//! - Emergency stop halts capture immediately.

use crate::runtime::Runtime;
use interaction_core::{ConsentScope, DomainError, DomainResult, EventType, Timestamp};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorUse {
    pub kind: String,
    pub started_at: Timestamp,
    pub started_by: String,
    pub purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_stop_at: Option<Timestamp>,
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

    /// Stop ALL sensors immediately (user action or estop path).
    pub async fn stop_all_sensors(&self, actor: &str) -> DomainResult<()> {
        if let Some(mic) = self.mic_receptor.as_ref() {
            mic.stop_listen();
        }
        self.store.audit("sensor.stopped-all", actor, &json!({}))?;
        Ok(())
    }
}
