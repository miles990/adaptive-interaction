//! Conditions over observation facts.
//!
//! Two syntaxes are accepted (both appear in the spec's examples):
//! - a map of exact fact matches: `{ event: task.completed }`
//! - an expression string: `"event == task.completed"`, `"count > 3"`

use interaction_core::Observation;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ConditionSpec {
    /// `"event == task.completed"` style expression.
    Expression(String),
    /// `{ event: task.completed, state: present }` — all pairs must match.
    Equals(BTreeMap<String, Value>),
}

#[derive(Debug, Clone, PartialEq)]
enum Op {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
}

impl ConditionSpec {
    /// Evaluate against an observation. Only **facts** are consulted by
    /// default; inference keys can be referenced with an `inferred.` prefix
    /// and are only matched when confidence passes `min_confidence`.
    pub fn matches(&self, obs: &Observation, min_confidence: f64) -> bool {
        match self {
            ConditionSpec::Equals(pairs) => pairs.iter().all(|(key, expected)| {
                lookup(obs, key, min_confidence)
                    .map(|actual| loose_eq(&actual, expected))
                    .unwrap_or(false)
            }),
            ConditionSpec::Expression(expr) => eval_expression(expr, obs, min_confidence),
        }
    }

    /// Validate the syntax; returns an error message on malformed expressions.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            ConditionSpec::Equals(pairs) => {
                if pairs.is_empty() {
                    return Err("condition map must not be empty".into());
                }
                Ok(())
            }
            ConditionSpec::Expression(expr) => {
                parse_expression(expr).map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }
}

fn lookup(obs: &Observation, key: &str, min_confidence: f64) -> Option<Value> {
    if let Some(inferred_key) = key.strip_prefix("inferred.") {
        if obs.confidence < min_confidence {
            return None;
        }
        return obs.inferences.get(inferred_key).cloned();
    }
    obs.facts.get(key).cloned()
}

/// Compare loosely: strings match unquoted scalars, numbers match numerically.
fn loose_eq(actual: &Value, expected: &Value) -> bool {
    if actual == expected {
        return true;
    }
    match (actual, expected) {
        (Value::String(a), b) => a == &scalar_to_string(b),
        (a, Value::String(b)) => &scalar_to_string(a) == b,
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => false,
    }
}

fn scalar_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn parse_expression(expr: &str) -> Result<(String, Op, String), String> {
    // Longest operators first so `>=` doesn't parse as `>`.
    for (token, op) in [
        ("==", Op::Eq),
        ("!=", Op::Ne),
        (">=", Op::Ge),
        ("<=", Op::Le),
        (">", Op::Gt),
        ("<", Op::Lt),
    ] {
        if let Some(idx) = expr.find(token) {
            let key = expr[..idx].trim().to_string();
            let value = expr[idx + token.len()..].trim().to_string();
            if key.is_empty() || value.is_empty() {
                return Err(format!("expression {expr:?} is missing a side"));
            }
            return Ok((key, op, value));
        }
    }
    Err(format!("expression {expr:?} has no operator (==, !=, >, <, >=, <=)"))
}

fn eval_expression(expr: &str, obs: &Observation, min_confidence: f64) -> bool {
    let Ok((key, op, raw)) = parse_expression(expr) else {
        return false;
    };
    let Some(actual) = lookup(obs, &key, min_confidence) else {
        return false;
    };
    let expected = raw.trim_matches(|c| c == '"' || c == '\'');
    match op {
        Op::Eq => loose_eq(&actual, &Value::String(expected.to_string())),
        Op::Ne => !loose_eq(&actual, &Value::String(expected.to_string())),
        Op::Gt | Op::Lt | Op::Ge | Op::Le => {
            let (Some(a), Ok(b)) = (as_f64(&actual), expected.parse::<f64>()) else {
                return false;
            };
            match op {
                Op::Gt => a > b,
                Op::Lt => a < b,
                Op::Ge => a >= b,
                Op::Le => a <= b,
                _ => unreachable!(),
            }
        }
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interaction_core::ReceptorId;

    fn obs() -> Observation {
        Observation::now(ReceptorId::new("t"), "test", chrono::Utc::now())
            .with_fact("event", "task.completed")
            .with_fact("count", 5)
            .with_inference("possibleState", "focused", 0.4)
    }

    #[test]
    fn map_condition() {
        let c: ConditionSpec =
            serde_yaml::from_str("event: task.completed").unwrap();
        assert!(c.matches(&obs(), 0.0));
    }

    #[test]
    fn expression_conditions() {
        let eq: ConditionSpec = serde_yaml::from_str("\"event == task.completed\"").unwrap();
        assert!(eq.matches(&obs(), 0.0));
        let gt: ConditionSpec = serde_yaml::from_str("\"count > 3\"").unwrap();
        assert!(gt.matches(&obs(), 0.0));
        let lt: ConditionSpec = serde_yaml::from_str("\"count < 3\"").unwrap();
        assert!(!lt.matches(&obs(), 0.0));
    }

    #[test]
    fn low_confidence_inference_does_not_match() {
        let c: ConditionSpec =
            serde_yaml::from_str("\"inferred.possibleState == focused\"").unwrap();
        // Confidence 0.4 < required 0.6 → no match.
        assert!(!c.matches(&obs(), 0.6));
        // Relaxed requirement → matches.
        assert!(c.matches(&obs(), 0.3));
    }

    #[test]
    fn facts_do_not_leak_inferences() {
        let c: ConditionSpec = serde_yaml::from_str("possibleState: focused").unwrap();
        assert!(!c.matches(&obs(), 0.0), "inference must not match as a fact");
    }

    #[test]
    fn validation_rejects_garbage() {
        let bad = ConditionSpec::Expression("no operator here".into());
        assert!(bad.validate().is_err());
        let empty = ConditionSpec::Equals(Default::default());
        assert!(empty.validate().is_err());
    }
}
