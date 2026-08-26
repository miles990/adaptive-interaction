//! Observations: what receptors report. Facts and inferences are kept apart,
//! and every inference carries confidence so the orchestrator can refuse to
//! act on stale or low-confidence guesses.

use crate::{CorrelationId, ObservationId, ReceptorId, SessionId, Timestamp, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ObservationSource {
    /// Driver that produced the observation.
    pub driver: String,
    /// Quality of the source 0..1 (sensor calibration, API reliability).
    #[serde(default = "default_quality")]
    pub quality: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn default_quality() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub observation_id: ObservationId,
    pub receptor_id: ReceptorId,
    /// When the underlying event happened.
    pub timestamp: Timestamp,
    /// When the runtime received it.
    pub received_at: Timestamp,
    /// Directly observable facts. Keys are receptor-defined.
    #[serde(default)]
    pub facts: BTreeMap<String, Value>,
    /// Model/driver inferences. MUST NOT be treated as facts.
    #[serde(default)]
    pub inferences: BTreeMap<String, Value>,
    /// Confidence of the inferences, 0..1. Facts are assumed confidence 1.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    /// Age of the data when received, in milliseconds.
    #[serde(default)]
    pub freshness_ms: u64,
    /// Source quality 0..1.
    #[serde(default = "default_quality")]
    pub quality: f64,
    pub source: ObservationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
    pub schema_version: String,
}

fn default_confidence() -> f64 {
    1.0
}

impl Observation {
    pub fn now(receptor_id: ReceptorId, driver: impl Into<String>, ts: Timestamp) -> Self {
        Self {
            observation_id: ObservationId::generate(),
            receptor_id,
            timestamp: ts,
            received_at: ts,
            facts: BTreeMap::new(),
            inferences: BTreeMap::new(),
            confidence: 1.0,
            freshness_ms: 0,
            quality: 1.0,
            source: ObservationSource {
                driver: driver.into(),
                quality: 1.0,
                detail: None,
            },
            session_id: None,
            correlation_id: None,
            schema_version: SCHEMA_VERSION.to_string(),
        }
    }

    pub fn with_fact(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.facts.insert(key.into(), value.into());
        self
    }

    pub fn with_inference(
        mut self,
        key: impl Into<String>,
        value: impl Into<Value>,
        confidence: f64,
    ) -> Self {
        self.inferences.insert(key.into(), value.into());
        self.confidence = self.confidence.min(confidence.clamp(0.0, 1.0));
        self
    }

    /// True when the observation is older than `max_age_ms` relative to `now`.
    pub fn is_stale(&self, now: Timestamp, max_age_ms: u64) -> bool {
        let age = now.signed_duration_since(self.timestamp);
        age.num_milliseconds() < 0 || age.num_milliseconds() as u128 > max_age_ms as u128
    }
}

/// Query for stored observations.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ObservationQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receptor_id: Option<ReceptorId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<Timestamp>,
    /// Reject observations older than this many milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_ms: Option<u64>,
    /// Minimum inference confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_and_inferences_are_separate() {
        let ts = chrono::Utc::now();
        let obs = Observation::now(ReceptorId::new("camera.main"), "test", ts)
            .with_fact("personVisible", true)
            .with_inference("possibleState", "focused", 0.67);
        assert_eq!(
            obs.facts.get("personVisible"),
            Some(&serde_json::json!(true))
        );
        assert!(!obs.facts.contains_key("possibleState"));
        assert!((obs.confidence - 0.67).abs() < f64::EPSILON);
    }

    #[test]
    fn staleness() {
        let ts = chrono::Utc::now();
        let obs = Observation::now(
            ReceptorId::new("x"),
            "test",
            ts - chrono::Duration::seconds(10),
        );
        assert!(obs.is_stale(ts, 5_000));
        assert!(!obs.is_stale(ts, 20_000));
    }
}
