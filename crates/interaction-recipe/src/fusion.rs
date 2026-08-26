//! Multi-receptor fusion: turn raw observations into a structured, deduped,
//! freshness- and confidence-aware context. Explicit human input always beats
//! inference; stale data is dropped, contradictions are flagged.

use interaction_core::{Observation, ReceptorId, Timestamp};
use serde::Serialize;
use std::collections::BTreeMap;

/// Receptors whose facts represent explicit human input and therefore
/// override inferred state from any other source.
pub const EXPLICIT_INPUT_RECEPTORS: &[&str] = &["session.input", "manual.event"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FusedContext {
    /// Latest usable observation per receptor.
    pub latest: BTreeMap<String, Observation>,
    /// Fact key -> value after override/priority resolution.
    pub facts: BTreeMap<String, serde_json::Value>,
    /// Inference key -> (value, confidence) surviving the confidence floor.
    pub inferences: BTreeMap<String, (serde_json::Value, f64)>,
    /// Fact keys that had contradictory values across receptors.
    pub contradictions: Vec<String>,
    /// Receptors that were requested but had no usable observation.
    pub missing: Vec<String>,
    /// Observations dropped as stale.
    pub dropped_stale: u32,
}

/// Fuse observations for the given receptors.
///
/// * `max_age_ms` — observations older than this are dropped (stale data must
///   not masquerade as current state).
/// * `min_confidence` — inferences below this floor are dropped.
pub fn fuse(
    receptors: &[String],
    observations: &[Observation],
    now: Timestamp,
    max_age_ms: u64,
    min_confidence: f64,
) -> FusedContext {
    let mut latest: BTreeMap<String, Observation> = BTreeMap::new();
    let mut dropped_stale = 0u32;

    for obs in observations {
        if !receptors.is_empty() && !receptors.iter().any(|r| r == obs.receptor_id.as_str()) {
            continue;
        }
        if obs.is_stale(now, max_age_ms) {
            dropped_stale += 1;
            continue;
        }
        match latest.get(obs.receptor_id.as_str()) {
            Some(existing) if existing.timestamp >= obs.timestamp => {}
            _ => {
                latest.insert(obs.receptor_id.as_str().to_string(), obs.clone());
            }
        }
    }

    let mut facts: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut fact_source: BTreeMap<String, (String, f64)> = BTreeMap::new(); // key -> (receptor, quality)
    let mut contradictions: Vec<String> = Vec::new();

    // Deterministic order; explicit-input receptors applied last so they win.
    let mut ordered: Vec<&Observation> = latest.values().collect();
    ordered.sort_by_key(|o| {
        (EXPLICIT_INPUT_RECEPTORS.contains(&o.receptor_id.as_str()), o.timestamp)
    });

    for obs in &ordered {
        let is_explicit = EXPLICIT_INPUT_RECEPTORS.contains(&obs.receptor_id.as_str());
        for (key, value) in &obs.facts {
            match facts.get(key) {
                Some(existing) if existing != value => {
                    let prev = fact_source.get(key).cloned();
                    contradictions.push(key.clone());
                    // Explicit input or higher quality wins.
                    let should_replace = is_explicit
                        || prev.map(|(_, q)| obs.quality > q).unwrap_or(true);
                    if should_replace {
                        facts.insert(key.clone(), value.clone());
                        fact_source
                            .insert(key.clone(), (obs.receptor_id.as_str().into(), obs.quality));
                    }
                }
                _ => {
                    facts.insert(key.clone(), value.clone());
                    fact_source
                        .insert(key.clone(), (obs.receptor_id.as_str().into(), obs.quality));
                }
            }
        }
    }

    let mut inferences: BTreeMap<String, (serde_json::Value, f64)> = BTreeMap::new();
    for obs in &ordered {
        if obs.confidence < min_confidence {
            continue;
        }
        for (key, value) in &obs.inferences {
            // A fact with the same key always suppresses the inference.
            if facts.contains_key(key) {
                continue;
            }
            match inferences.get(key) {
                Some((_, existing_conf)) if *existing_conf >= obs.confidence => {}
                _ => {
                    inferences.insert(key.clone(), (value.clone(), obs.confidence));
                }
            }
        }
    }

    contradictions.sort();
    contradictions.dedup();

    let missing = receptors
        .iter()
        .filter(|r| !latest.contains_key(r.as_str()))
        .cloned()
        .collect();

    FusedContext { latest, facts, inferences, contradictions, missing, dropped_stale }
}

/// Latest observation for a receptor within the fused set.
pub fn latest_for<'a>(ctx: &'a FusedContext, receptor: &ReceptorId) -> Option<&'a Observation> {
    ctx.latest.get(receptor.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use interaction_core::ReceptorId;

    fn obs_at(receptor: &str, secs_ago: i64, now: Timestamp) -> Observation {
        Observation::now(ReceptorId::new(receptor), "test", now - chrono::Duration::seconds(secs_ago))
    }

    #[test]
    fn stale_observations_are_dropped() {
        let now = chrono::Utc::now();
        let fresh = obs_at("a", 1, now).with_fact("state", "present");
        let stale = obs_at("b", 600, now).with_fact("state", "away");
        let ctx = fuse(&["a".into(), "b".into()], &[fresh, stale], now, 60_000, 0.0);
        assert_eq!(ctx.dropped_stale, 1);
        assert_eq!(ctx.facts.get("state"), Some(&serde_json::json!("present")));
        assert_eq!(ctx.missing, vec!["b".to_string()]);
    }

    #[test]
    fn explicit_input_overrides_inference_and_facts() {
        let now = chrono::Utc::now();
        let camera = obs_at("camera.main", 2, now)
            .with_fact("state", "away")
            .with_inference("mood", "tired", 0.9);
        let user = obs_at("session.input", 1, now).with_fact("state", "present");
        let ctx = fuse(&[], &[camera, user], now, 60_000, 0.5);
        assert_eq!(ctx.facts.get("state"), Some(&serde_json::json!("present")));
        assert!(ctx.contradictions.contains(&"state".to_string()));
        // Inference survives (no competing fact for "mood").
        assert!(ctx.inferences.contains_key("mood"));
    }

    #[test]
    fn low_confidence_inferences_are_dropped() {
        let now = chrono::Utc::now();
        let cam = obs_at("camera.main", 1, now).with_inference("mood", "tired", 0.2);
        let ctx = fuse(&[], &[cam], now, 60_000, 0.5);
        assert!(ctx.inferences.is_empty());
    }

    #[test]
    fn latest_per_receptor_wins() {
        let now = chrono::Utc::now();
        let old = obs_at("a", 30, now).with_fact("v", 1);
        let new = obs_at("a", 1, now).with_fact("v", 2);
        let ctx = fuse(&[], &[old, new], now, 60_000, 0.0);
        assert_eq!(ctx.facts.get("v"), Some(&serde_json::json!(2)));
    }
}
