//! Adaptive orchestrator: turns a semantic intent plus the current capability
//! snapshot into a plan — the *minimal effective action set* — or an explicit
//! no-action plan. Scoring is a deterministic, explainable heuristic:
//!
//! ```text
//! utility = expected benefit - interruption - risk - cost - repetition fatigue
//! ```
//!
//! This is a design heuristic, not a scientific model; every number lands in
//! the plan's rationale so humans and AIs can audit the choice.

use crate::text::TextSelector;
use interaction_core::{
    new_plan, ActionParameters, ActuatorManifest, CapabilitySnapshot, MessageStrategy, Plan,
    PlanStatus, PlannedStep, RejectedCandidate, SemanticIntent, SessionId, Timestamp,
};
use std::collections::BTreeMap;

/// Extra per-actuator context the orchestrator uses for fatigue scoring.
#[derive(Debug, Clone, Default)]
pub struct ActuatorUsageHint {
    pub fired_last_hour: u32,
}

pub struct PlanRequest<'a> {
    pub session_id: SessionId,
    pub intent: SemanticIntent,
    pub snapshot: &'a CapabilitySnapshot,
    /// Restrict candidates to these actuator ids (recipe candidates). Empty = all.
    pub candidates: Vec<String>,
    pub min_channels: u32,
    pub max_channels: u32,
    pub allow_no_action: bool,
    pub message_strategy: MessageStrategy,
    pub usage: BTreeMap<String, ActuatorUsageHint>,
    pub now: Timestamp,
    pub default_ttl_ms: u64,
}

/// Channels that can carry a text message.
fn is_text_channel(channel: &str) -> bool {
    matches!(
        channel,
        "conversation" | "web-ui" | "notification" | "log" | "webhook"
    )
}

fn interruption_cost(channel: &str) -> f64 {
    match channel {
        "log" => 0.01,
        "web-ui" => 0.05,
        "conversation" => 0.10,
        "webhook" => 0.15,
        "visual" | "desktop-pet" => 0.20,
        "light" => 0.25,
        "notification" => 0.30,
        "haptic" => 0.35,
        "audio" => 0.40,
        _ => 0.20,
    }
}

fn risk_penalty(m: &ActuatorManifest) -> f64 {
    (m.risk_class as u8 as f64) * 0.10 + if m.external_side_effect { 0.10 } else { 0.0 }
}

const SCORE_FLOOR: f64 = 0.15;

pub fn build_plan(req: PlanRequest<'_>, texts: &TextSelector) -> Plan {
    let mut plan = new_plan(
        req.session_id.clone(),
        req.intent.clone(),
        req.now,
        req.default_ttl_ms,
    );

    // Candidate pool: available actuators, optionally restricted by the caller.
    let mut scored: Vec<(f64, &ActuatorManifest, String)> = Vec::new();
    for manifest in &req.snapshot.actuators {
        if !req.candidates.is_empty() && !req.candidates.iter().any(|c| c == manifest.id.as_str()) {
            continue;
        }
        if !manifest.availability.is_available() {
            plan.rejected.push(RejectedCandidate {
                actuator_id: manifest.id.clone(),
                reason: format!("unavailable ({:?})", manifest.availability),
            });
            continue;
        }

        let preferred = &req.intent.preferred_channels;
        let benefit = if preferred.is_empty() {
            0.8
        } else if let Some(idx) = preferred.iter().position(|c| c == &manifest.channel) {
            1.0 - (idx as f64) * 0.1
        } else {
            0.45
        };
        let interruption = interruption_cost(&manifest.channel);
        let risk = risk_penalty(manifest);
        let cost = manifest.cost.monetary_per_invocation + manifest.cost.resource * 0.2;
        let fatigue = req
            .usage
            .get(manifest.id.as_str())
            .map(|u| u.fired_last_hour as f64 * 0.15)
            .unwrap_or(0.0);
        let score = benefit - interruption - risk - cost - fatigue;
        let rationale = format!(
            "benefit {benefit:.2} - interruption {interruption:.2} - risk {risk:.2} \
             - cost {cost:.2} - fatigue {fatigue:.2} = {score:.2}"
        );
        scored.push((score, manifest, rationale));
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Minimal effective set: highest-utility actuator per channel, floor-gated,
    // capped by max_channels.
    let text = texts.select(
        &req.message_strategy,
        &req.intent.intent,
        req.intent.message.as_deref(),
    );
    let mut used_channels: Vec<String> = Vec::new();
    for (score, manifest, rationale) in &scored {
        if plan.steps.len() >= req.max_channels as usize {
            plan.rejected.push(RejectedCandidate {
                actuator_id: manifest.id.clone(),
                reason: format!("maxChannels {} reached", req.max_channels),
            });
            continue;
        }
        if used_channels.contains(&manifest.channel) {
            plan.rejected.push(RejectedCandidate {
                actuator_id: manifest.id.clone(),
                reason: format!("channel {} already covered", manifest.channel),
            });
            continue;
        }
        // Below the utility floor: only take it if we still owe min_channels.
        if *score < SCORE_FLOOR && plan.steps.len() >= req.min_channels as usize {
            plan.rejected.push(RejectedCandidate {
                actuator_id: manifest.id.clone(),
                reason: format!("utility {score:.2} below floor {SCORE_FLOOR}"),
            });
            continue;
        }
        let message = if is_text_channel(&manifest.channel) {
            text.clone()
        } else {
            None
        };
        plan.steps.push(PlannedStep {
            actuator_id: manifest.id.clone(),
            channel: manifest.channel.clone(),
            requested: ActionParameters {
                magnitude: req.intent.magnitude,
                duration_ms: req.intent.duration_ms,
                message,
                extra: req.intent.payload.clone(),
            },
            score: *score,
            rationale: rationale.clone(),
        });
        used_channels.push(manifest.channel.clone());
    }

    if plan.steps.is_empty() || (plan.steps.len() < req.min_channels as usize) {
        if req.allow_no_action {
            plan.steps.clear();
            plan.status = PlanStatus::NoAction;
            plan.metadata.insert(
                "noActionReason".into(),
                serde_json::json!(
                    "no candidate met the utility floor / minChannels; not intervening is the chosen action"
                ),
            );
        } else {
            plan.status = PlanStatus::Blocked;
            plan.metadata.insert(
                "blockedReason".into(),
                serde_json::json!(format!(
                    "needed {} channels but only {} viable",
                    req.min_channels,
                    plan.steps.len()
                )),
            );
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use interaction_adapter_sdk::ActuatorManifestBuilder;
    use interaction_core::*;

    fn snapshot(actuators: Vec<ActuatorManifest>) -> CapabilitySnapshot {
        CapabilitySnapshot {
            receptors: vec![],
            actuators,
            tool_operations: vec![],
            constraints: vec![],
            session_policy: PolicyConfig::default(),
            generated_at: chrono::Utc::now(),
            version: 1,
            schema_version: SCHEMA_VERSION.into(),
        }
    }

    fn manifest(id: &str, channel: &str) -> ActuatorManifest {
        ActuatorManifestBuilder::new(id, id, channel, "test")
            .risk(RiskClass::Low)
            .build()
    }

    fn request<'a>(
        snapshot: &'a CapabilitySnapshot,
        intent: SemanticIntent,
        max_channels: u32,
    ) -> PlanRequest<'a> {
        PlanRequest {
            session_id: SessionId::generate(),
            intent,
            snapshot,
            candidates: vec![],
            min_channels: 0,
            max_channels,
            allow_no_action: true,
            message_strategy: MessageStrategy::default(),
            usage: BTreeMap::new(),
            now: chrono::Utc::now(),
            default_ttl_ms: 30_000,
        }
    }

    #[test]
    fn picks_minimal_effective_set_with_distinct_channels() {
        let snap = snapshot(vec![
            manifest("conversation", "conversation"),
            manifest("web-ui", "web-ui"),
            manifest("conversation2", "conversation"),
        ]);
        let texts = TextSelector::default();
        let mut intent = SemanticIntent::new("celebration");
        intent.preferred_channels = vec!["conversation".into()];
        let plan = build_plan(request(&snap, intent, 2), &texts);
        assert_eq!(plan.steps.len(), 2);
        let channels: Vec<&str> = plan.steps.iter().map(|s| s.channel.as_str()).collect();
        assert!(channels.contains(&"conversation"));
        assert!(channels.contains(&"web-ui"));
        // One conversation actuator was rejected as duplicate channel.
        assert!(plan
            .rejected
            .iter()
            .any(|r| r.reason.contains("already covered")));
        // Text was attached to text channels.
        assert!(plan.steps.iter().all(|s| s.requested.message.is_some()));
    }

    #[test]
    fn no_action_when_nothing_viable() {
        let snap = snapshot(vec![]);
        let texts = TextSelector::default();
        let plan = build_plan(request(&snap, SemanticIntent::new("presence"), 2), &texts);
        assert_eq!(plan.status, PlanStatus::NoAction);
        assert!(plan.steps.is_empty());
    }

    #[test]
    fn fatigue_pushes_repeated_actuator_below_floor() {
        let snap = snapshot(vec![manifest("web-ui", "web-ui")]);
        let texts = TextSelector::default();
        let mut req = request(&snap, SemanticIntent::new("progress"), 2);
        req.usage.insert(
            "web-ui".into(),
            ActuatorUsageHint {
                fired_last_hour: 10,
            },
        );
        let plan = build_plan(req, &texts);
        assert_eq!(
            plan.status,
            PlanStatus::NoAction,
            "fatigued channel should be skipped"
        );
    }

    #[test]
    fn unavailable_actuators_are_rejected_with_reason() {
        let mut m = manifest("audio", "audio");
        m.availability = Availability::Offline;
        let snap = snapshot(vec![m, manifest("conversation", "conversation")]);
        let texts = TextSelector::default();
        let plan = build_plan(request(&snap, SemanticIntent::new("success"), 2), &texts);
        assert!(plan
            .rejected
            .iter()
            .any(|r| r.actuator_id.as_str() == "audio"));
        assert_eq!(plan.steps.len(), 1);
    }

    #[test]
    fn min_channels_unmet_blocks_when_no_action_disallowed() {
        let snap = snapshot(vec![]);
        let texts = TextSelector::default();
        let mut req = request(&snap, SemanticIntent::new("warning"), 2);
        req.allow_no_action = false;
        req.min_channels = 1;
        let plan = build_plan(req, &texts);
        assert_eq!(plan.status, PlanStatus::Blocked);
    }
}
