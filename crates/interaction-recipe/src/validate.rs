//! Recipe parsing + validation. YAML and JSON share the same domain model and
//! the same validator; errors carry field paths so UIs can highlight them.

use crate::{parse_duration_ms, FusionMode, Recipe};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub field: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RecipeParseError {
    #[error("failed to parse recipe: {0}")]
    Syntax(String),
    #[error("recipe validation failed: {}", format_issues(.0))]
    Invalid(Vec<ValidationIssue>),
}

fn format_issues(issues: &[ValidationIssue]) -> String {
    issues
        .iter()
        .map(|i| format!("{}: {}", i.field, i.message))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Parse from YAML or JSON (auto-detected) and validate.
pub fn parse_and_validate(input: &str) -> Result<Recipe, RecipeParseError> {
    let trimmed = input.trim_start();
    let recipe: Recipe = if trimmed.starts_with('{') {
        serde_json::from_str(input).map_err(|e| RecipeParseError::Syntax(e.to_string()))?
    } else {
        serde_yaml::from_str(input).map_err(|e| RecipeParseError::Syntax(e.to_string()))?
    };
    let issues = validate(&recipe);
    if issues.is_empty() {
        Ok(recipe)
    } else {
        Err(RecipeParseError::Invalid(issues))
    }
}

/// Structural validation beyond serde's type checks.
pub fn validate(recipe: &Recipe) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let push = |issues: &mut Vec<ValidationIssue>, field: &str, message: String| {
        issues.push(ValidationIssue {
            field: field.to_string(),
            message,
        });
    };

    if recipe.id.as_str().trim().is_empty() {
        push(&mut issues, "id", "must not be empty".into());
    }
    if recipe.name.trim().is_empty() {
        push(&mut issues, "name", "must not be empty".into());
    }
    if recipe.trigger.steps.is_empty() {
        push(
            &mut issues,
            "trigger.steps",
            "at least one step is required".into(),
        );
    }
    if recipe.trigger.mode == FusionMode::Single && recipe.trigger.steps.len() > 1 {
        push(
            &mut issues,
            "trigger.mode",
            "mode 'single' requires exactly one step".into(),
        );
    }
    if recipe.trigger.mode == FusionMode::Quorum {
        match recipe.trigger.quorum {
            None => push(
                &mut issues,
                "trigger.quorum",
                "required for mode 'quorum'".into(),
            ),
            Some(q) if q as usize > recipe.trigger.steps.len() => push(
                &mut issues,
                "trigger.quorum",
                format!(
                    "quorum {q} exceeds step count {}",
                    recipe.trigger.steps.len()
                ),
            ),
            _ => {}
        }
    }
    if recipe.trigger.mode == FusionMode::Weighted && recipe.trigger.threshold.is_none() {
        push(
            &mut issues,
            "trigger.threshold",
            "required for mode 'weighted'".into(),
        );
    }
    for (i, step) in recipe.trigger.steps.iter().enumerate() {
        if step.receptor.trim().is_empty() {
            push(
                &mut issues,
                &format!("trigger.steps[{i}].receptor"),
                "must not be empty".into(),
            );
        }
        if step.weight < 0.0 {
            push(
                &mut issues,
                &format!("trigger.steps[{i}].weight"),
                "must be >= 0".into(),
            );
        }
        if let Some(cond) = &step.condition {
            if let Err(e) = cond.validate() {
                push(&mut issues, &format!("trigger.steps[{i}].condition"), e);
            }
        }
    }
    for (field, value) in [
        ("trigger.within", &recipe.trigger.within),
        ("context.maxAge", &recipe.context.max_age),
        ("limits.cooldown", &recipe.limits.cooldown),
        ("limits.expiresAfter", &recipe.limits.expires_after),
        ("verification.timeout", &recipe.verification.timeout),
        ("actuation.jitter", &recipe.actuation.jitter),
    ] {
        if let Some(v) = value {
            if let Err(e) = parse_duration_ms(v) {
                push(&mut issues, field, e);
            }
        }
    }
    if recipe.actuation.candidates.is_empty() {
        push(
            &mut issues,
            "actuation.candidates",
            "at least one candidate actuator is required".into(),
        );
    }
    if recipe.actuation.min_channels > recipe.actuation.max_channels {
        push(
            &mut issues,
            "actuation.minChannels",
            "must be <= maxChannels".into(),
        );
    }
    if recipe.actuation.max_channels as usize > recipe.actuation.candidates.len().max(1) * 2 {
        // Not fatal; but max_channels beyond candidates is meaningless.
    }
    if !(0.0..=1.0).contains(&recipe.actuation.chance) {
        push(
            &mut issues,
            "actuation.chance",
            "must be within 0..1".into(),
        );
    }
    if let Some(mc) = recipe.context.min_confidence {
        if !(0.0..=1.0).contains(&mc) {
            push(
                &mut issues,
                "context.minConfidence",
                "must be within 0..1".into(),
            );
        }
    }
    if !recipe.decision.allow_no_action && recipe.actuation.min_channels == 0 {
        // minChannels 0 with allowNoAction=false is contradictory.
        push(
            &mut issues,
            "decision.allowNoAction",
            "allowNoAction=false requires actuation.minChannels >= 1".into(),
        );
    }
    for (i, scope) in recipe.consent.required.iter().enumerate() {
        let valid = scope.split_once(':').map(|(kind, id)| {
            !id.is_empty() && matches!(kind, "channel" | "actuator" | "receptor" | "tool")
        });
        if valid != Some(true) {
            push(
                &mut issues,
                &format!("consent.required[{i}]"),
                format!("{scope:?} must look like 'channel:<id>' / 'actuator:<id>' / 'receptor:<id>' / 'tool:<id>'"),
            );
        }
    }
    if let Some(ai) = &recipe.ai {
        if !(0.0..=1.0).contains(&ai.min_confidence) {
            push(
                &mut issues,
                "ai.minConfidence",
                "must be within 0..1".into(),
            );
        }
        if ai.max_wait_ms > 60_000 {
            push(
                &mut issues,
                "ai.maxWaitMs",
                "must be <= 60000 (an assist wait is not a background job)".into(),
            );
        }
        if ai.daily_call_cap == Some(0) {
            push(
                &mut issues,
                "ai.dailyCallCap",
                "0 disables AI entirely; use mode 'never' instead".into(),
            );
        }
    }
    issues
}

/// Serialize a recipe back to YAML, preserving unknown fields captured in
/// the `extra` maps. This is the inverse of [`parse_and_validate`].
pub fn to_yaml(recipe: &Recipe) -> Result<String, String> {
    serde_yaml::to_string(recipe).map_err(|e| e.to_string())
}

/// Serialize a recipe to pretty JSON (same model, same fidelity as YAML).
pub fn to_json_pretty(recipe: &Recipe) -> Result<String, String> {
    serde_json::to_string_pretty(recipe).map_err(|e| e.to_string())
}

/// JSON Schema for recipes, generated from the domain model itself so UIs and
/// external validators agree with the Rust validator's shape checks.
pub fn recipe_json_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(Recipe);
    serde_json::to_value(schema).unwrap_or_else(|_| serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
id: adaptive-task-completion
name: Adaptive task completion
enabled: true
trigger:
  mode: sequence
  within: 10m
  steps:
    - receptor: task.lifecycle
      condition:
        event: task.completed
    - receptor: user.presence
      condition:
        state: present
context:
  receptors: [system.time]
decision:
  objective: celebrate-without-interrupting
  allowNoAction: true
message:
  mode: adaptive
  intents: [celebration]
  templates: ["完成了，所有檢查都已通過。"]
  allowSilence: true
actuation:
  mode: adaptive
  candidates: [conversation, web-ui]
  minChannels: 0
  maxChannels: 3
verification:
  strategy: best-effort
  timeout: 10s
limits:
  cooldown: 15m
  expiresAfter: 30s
"#;

    #[test]
    fn good_recipe_parses() {
        let recipe = parse_and_validate(GOOD).expect("should parse");
        assert_eq!(recipe.id.as_str(), "adaptive-task-completion");
        assert_eq!(recipe.trigger.steps.len(), 2);
    }

    #[test]
    fn json_and_yaml_share_the_model() {
        let recipe = parse_and_validate(GOOD).unwrap();
        let json = serde_json::to_string(&recipe).unwrap();
        let reparsed = parse_and_validate(&json).expect("json roundtrip");
        assert_eq!(recipe, reparsed);
    }

    #[test]
    fn invalid_recipe_reports_field_paths() {
        let bad = r#"
id: ""
name: x
trigger:
  mode: quorum
  steps:
    - receptor: a
      condition: "no operator"
decision:
  objective: y
  allowNoAction: false
actuation:
  candidates: []
  minChannels: 0
  chance: 3.0
"#;
        let err = parse_and_validate(bad).unwrap_err();
        let RecipeParseError::Invalid(issues) = err else {
            panic!("expected validation failure")
        };
        let fields: Vec<&str> = issues.iter().map(|i| i.field.as_str()).collect();
        assert!(fields.contains(&"id"));
        assert!(fields.contains(&"trigger.quorum"));
        assert!(fields.contains(&"trigger.steps[0].condition"));
        assert!(fields.contains(&"actuation.candidates"));
        assert!(fields.contains(&"actuation.chance"));
        assert!(fields.contains(&"decision.allowNoAction"));
    }

    #[test]
    fn syntax_error_does_not_panic() {
        assert!(matches!(
            parse_and_validate("{{{{ not yaml"),
            Err(RecipeParseError::Syntax(_))
        ));
    }

    #[test]
    fn schema_is_generated() {
        let schema = recipe_json_schema();
        assert!(schema.get("properties").is_some());
    }

    #[test]
    fn unknown_fields_survive_yaml_roundtrip() {
        // A future/other tool wrote fields this version doesn't know about,
        // at the top level and inside sub-specs. They must not be dropped.
        let input = r#"
id: rt
name: Round trip
futureTopLevelSetting:
  nested: true
  list: [1, 2]
trigger:
  mode: single
  vendorTriggerHint: fast-path
  steps:
    - receptor: task.lifecycle
      vendorStepNote: keep-me
decision:
  objective: test
actuation:
  candidates: [conversation]
  vendorActuationTweak: 7
"#;
        let recipe = parse_and_validate(input).expect("parses with unknown fields");
        assert_eq!(
            recipe.extra.get("futureTopLevelSetting").unwrap()["nested"],
            serde_json::json!(true)
        );
        assert_eq!(
            recipe.trigger.extra.get("vendorTriggerHint").unwrap(),
            &serde_json::json!("fast-path")
        );
        let yaml = to_yaml(&recipe).unwrap();
        let back = parse_and_validate(&yaml).expect("roundtrip parses");
        assert_eq!(recipe, back, "YAML roundtrip must be lossless");
        assert!(yaml.contains("vendorStepNote"));
        assert!(yaml.contains("vendorActuationTweak"));
        // JSON path preserves the same data.
        let json = to_json_pretty(&recipe).unwrap();
        let back_json = parse_and_validate(&json).expect("json roundtrip parses");
        assert_eq!(recipe, back_json);
    }

    #[test]
    fn ai_spec_parses_and_validates() {
        let input = r#"
id: ai-recipe
name: With AI gate
trigger:
  mode: single
  steps:
    - receptor: user.presence
decision:
  objective: assist
actuation:
  candidates: [conversation]
ai:
  mode: when-uncertain
  minConfidence: 0.7
  onUnavailable: no-action
"#;
        let recipe = parse_and_validate(input).unwrap();
        let ai = recipe.ai.as_ref().unwrap();
        assert_eq!(ai.mode, crate::AiAssistMode::WhenUncertain);
        assert_eq!(ai.on_unavailable, crate::AiUnavailableBehavior::NoAction);
        assert!(
            !ai.allow_sensitive_egress,
            "sensitive egress must default off"
        );

        let bad = input.replace("minConfidence: 0.7", "minConfidence: 3.0");
        let err = parse_and_validate(&bad).unwrap_err();
        let RecipeParseError::Invalid(issues) = err else {
            panic!("expected validation failure")
        };
        assert!(issues.iter().any(|i| i.field == "ai.minConfidence"));
    }

    #[test]
    fn recipe_without_ai_field_defaults_to_never() {
        let recipe = parse_and_validate(GOOD).unwrap();
        assert!(recipe.ai.is_none(), "legacy recipes stay AI-free");
    }
}
