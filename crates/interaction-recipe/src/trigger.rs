//! Trigger evaluation: pure, explainable decisions about whether a recipe
//! fires given a set of observations.

use crate::{parse_duration_ms, FusionMode, Recipe, TriggerStep};
use interaction_core::{Observation, Timestamp};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDecision {
    pub fired: bool,
    /// Step-by-step explanation of why the trigger did or didn't fire.
    pub explanation: Vec<String>,
}

impl TriggerDecision {
    fn no(reason: impl Into<String>) -> Self {
        Self {
            fired: false,
            explanation: vec![reason.into()],
        }
    }
}

/// Evaluate a recipe trigger against observations (any order accepted).
pub fn evaluate_trigger(
    recipe: &Recipe,
    observations: &[Observation],
    now: Timestamp,
) -> TriggerDecision {
    if !recipe.enabled {
        return TriggerDecision::no("recipe is disabled");
    }
    let trigger = &recipe.trigger;
    if trigger.steps.is_empty() {
        return TriggerDecision::no("trigger has no steps");
    }
    let min_confidence = recipe.context.min_confidence.unwrap_or(0.5);
    let window_ms = match trigger.within.as_deref().map(parse_duration_ms).transpose() {
        Ok(w) => w,
        Err(e) => return TriggerDecision::no(format!("invalid trigger window: {e}")),
    };

    let mut explanation = Vec::new();

    // Match each step to its most recent matching observation.
    let matches: Vec<Option<&Observation>> = trigger
        .steps
        .iter()
        .map(|step| best_match(step, observations, now, window_ms, min_confidence))
        .collect();

    for (i, (step, matched)) in trigger.steps.iter().zip(matches.iter()).enumerate() {
        match matched {
            Some(obs) => explanation.push(format!(
                "step {i} ({}) matched by observation {} at {}",
                step.receptor, obs.observation_id, obs.timestamp
            )),
            None => explanation.push(format!("step {i} ({}) not matched", step.receptor)),
        }
    }

    let matched_count = matches.iter().filter(|m| m.is_some()).count();
    let fired = match trigger.mode {
        FusionMode::Single => matched_count >= 1 && trigger.steps.len() == 1,
        FusionMode::Any => matched_count >= 1,
        FusionMode::All => matched_count == trigger.steps.len(),
        FusionMode::Quorum => {
            let need = trigger.quorum.unwrap_or(trigger.steps.len() as u32) as usize;
            explanation.push(format!("quorum: {matched_count}/{need}"));
            matched_count >= need
        }
        FusionMode::Weighted => {
            let threshold = trigger.threshold.unwrap_or(1.0);
            let sum: f64 = trigger
                .steps
                .iter()
                .zip(matches.iter())
                .filter(|(_, m)| m.is_some())
                .map(|(s, _)| s.weight)
                .sum();
            explanation.push(format!("weighted sum {sum:.2} vs threshold {threshold:.2}"));
            sum >= threshold
        }
        FusionMode::Sequence => {
            if matched_count != trigger.steps.len() {
                false
            } else {
                // Timestamps must be non-decreasing in step order.
                let stamps: Vec<Timestamp> = matches.iter().map(|m| m.unwrap().timestamp).collect();
                let ordered = stamps.windows(2).all(|w| w[0] <= w[1]);
                if !ordered {
                    explanation.push("sequence order violated".into());
                }
                let within_window = match window_ms {
                    Some(w) => {
                        let span = stamps
                            .last()
                            .unwrap()
                            .signed_duration_since(*stamps.first().unwrap())
                            .num_milliseconds();
                        let ok = span >= 0 && (span as u64) <= w;
                        if !ok {
                            explanation
                                .push(format!("sequence span {span}ms exceeds window {w}ms"));
                        }
                        ok
                    }
                    None => true,
                };
                ordered && within_window
            }
        }
    };

    explanation.push(if fired {
        format!(
            "trigger fired (mode {:?}, {matched_count}/{} steps)",
            trigger.mode,
            trigger.steps.len()
        )
    } else {
        format!(
            "trigger not fired (mode {:?}, {matched_count}/{} steps)",
            trigger.mode,
            trigger.steps.len()
        )
    });

    TriggerDecision { fired, explanation }
}

fn best_match<'a>(
    step: &TriggerStep,
    observations: &'a [Observation],
    now: Timestamp,
    window_ms: Option<u64>,
    min_confidence: f64,
) -> Option<&'a Observation> {
    observations
        .iter()
        .filter(|o| o.receptor_id.as_str() == step.receptor)
        .filter(|o| window_ms.map(|w| !o.is_stale(now, w)).unwrap_or(true))
        .filter(|o| {
            step.condition
                .as_ref()
                .map(|c| c.matches(o, min_confidence))
                .unwrap_or(true)
        })
        .max_by_key(|o| o.timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use interaction_core::ReceptorId;

    fn recipe_yaml(mode: &str, extra: &str) -> Recipe {
        let yaml = format!(
            r#"
id: r1
name: test
trigger:
  mode: {mode}
  within: 10m
  {extra}
  steps:
    - receptor: task.lifecycle
      condition:
        event: task.completed
    - receptor: user.presence
      condition:
        state: present
decision:
  objective: test
  allowNoAction: true
actuation:
  mode: adaptive
  candidates: [conversation]
"#
        );
        serde_yaml::from_str(&yaml).expect("valid recipe yaml")
    }

    fn obs(receptor: &str, key: &str, value: &str, secs_ago: i64, now: Timestamp) -> Observation {
        Observation::now(
            ReceptorId::new(receptor),
            "test",
            now - chrono::Duration::seconds(secs_ago),
        )
        .with_fact(key, value)
    }

    #[test]
    fn all_mode_needs_every_step() {
        let recipe = recipe_yaml("all", "");
        let now = chrono::Utc::now();
        let partial = [obs("task.lifecycle", "event", "task.completed", 5, now)];
        assert!(!evaluate_trigger(&recipe, &partial, now).fired);
        let full = [
            obs("task.lifecycle", "event", "task.completed", 5, now),
            obs("user.presence", "state", "present", 3, now),
        ];
        let d = evaluate_trigger(&recipe, &full, now);
        assert!(d.fired, "{:?}", d.explanation);
    }

    #[test]
    fn any_mode_fires_on_one() {
        let recipe = recipe_yaml("any", "");
        let now = chrono::Utc::now();
        let partial = [obs("user.presence", "state", "present", 3, now)];
        assert!(evaluate_trigger(&recipe, &partial, now).fired);
    }

    #[test]
    fn sequence_requires_order_within_window() {
        let recipe = recipe_yaml("sequence", "");
        let now = chrono::Utc::now();
        // Correct order: task completed first, presence after.
        let good = [
            obs("task.lifecycle", "event", "task.completed", 60, now),
            obs("user.presence", "state", "present", 5, now),
        ];
        assert!(evaluate_trigger(&recipe, &good, now).fired);
        // Wrong order.
        let bad = [
            obs("task.lifecycle", "event", "task.completed", 5, now),
            obs("user.presence", "state", "present", 60, now),
        ];
        assert!(!evaluate_trigger(&recipe, &bad, now).fired);
        // Outside window (within: 10m).
        let stale = [
            obs("task.lifecycle", "event", "task.completed", 3600, now),
            obs("user.presence", "state", "present", 5, now),
        ];
        assert!(!evaluate_trigger(&recipe, &stale, now).fired);
    }

    #[test]
    fn quorum_mode() {
        let recipe = recipe_yaml("quorum", "quorum: 1");
        let now = chrono::Utc::now();
        let one = [obs("task.lifecycle", "event", "task.completed", 5, now)];
        assert!(evaluate_trigger(&recipe, &one, now).fired);
    }

    #[test]
    fn weighted_mode() {
        let mut recipe = recipe_yaml("weighted", "threshold: 1.5");
        recipe.trigger.steps[0].weight = 1.0;
        recipe.trigger.steps[1].weight = 1.0;
        let now = chrono::Utc::now();
        let one = [obs("task.lifecycle", "event", "task.completed", 5, now)];
        assert!(!evaluate_trigger(&recipe, &one, now).fired);
        let both = [
            obs("task.lifecycle", "event", "task.completed", 5, now),
            obs("user.presence", "state", "present", 3, now),
        ];
        assert!(evaluate_trigger(&recipe, &both, now).fired);
    }

    #[test]
    fn disabled_recipe_never_fires() {
        let mut recipe = recipe_yaml("any", "");
        recipe.enabled = false;
        let now = chrono::Utc::now();
        let full = [obs("task.lifecycle", "event", "task.completed", 5, now)];
        let d = evaluate_trigger(&recipe, &full, now);
        assert!(!d.fired);
        assert!(d.explanation[0].contains("disabled"));
    }
}
