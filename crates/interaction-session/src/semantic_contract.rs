//! Consumer contract for a complete semantic state. The authoritative host's
//! restore policy remains stricter: a renderer retains future fields, whereas
//! a host must not re-author unknown state from an untrusted snapshot.
use interaction_aip::limits;
use serde_json::{json, Value};
use std::sync::OnceLock;

pub const SEMANTIC_STATE_PROFILE: &str = "semantic-state/1.0";

/// Derived from the domain type, without introducing an AIP -> Session edge.
/// Vocabulary strings are open to consumers; unknown values have no behavior.
pub fn semantic_state_schema() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(crate::SemanticState))
        .expect("SemanticState schema is serializable");
    normalize(&mut schema);
    schema["$id"] = json!(
        "https://github.com/miles990/adaptive-interaction/schemas/semantic-state-1.0.schema.json"
    );
    schema["profile"] = json!(SEMANTIC_STATE_PROFILE);
    schema["x-limits"] = json!({"maxBytes": limits::MAX_PAYLOAD_BYTES, "maxDepth": limits::MAX_JSON_DEPTH - 1, "maxStringChars": limits::MAX_STRING_CHARS, "noNull": true});
    schema["properties"]["members"]["maxItems"] = json!(limits::MAX_MEMBERS);
    schema["$defs"]["Mood"]["properties"]["intensity"]["minimum"] = json!(0.0);
    schema["$defs"]["Mood"]["properties"]["intensity"]["maximum"] = json!(1.0);
    schema["$defs"]["Mood"]["properties"]["intensity"]["x-nonNegativeZero"] = json!(true);
    if let Some(branches) = schema["$defs"]["Attention"]["oneOf"].as_array_mut() {
        branches.push(json!({"type":"object", "properties":{"kind":{"type":"string", "not":{"enum":["none","member","task"]}}}, "required":["kind"]}));
    }
    schema
}

fn normalize(node: &mut Value) {
    match node {
        Value::Object(map) => {
            // Optional None is absent, never a present null in a state.
            if let Some(Value::Array(types)) = map.get_mut("type") {
                types.retain(|t| t != "null");
                if types.len() == 1 {
                    let only = types[0].clone();
                    map.insert("type".into(), only);
                }
            }
            if let Some(Value::Array(branches)) = map.get_mut("anyOf") {
                branches.retain(|b| b.get("type") != Some(&json!("null")));
                if branches.len() == 1 {
                    let remaining = branches[0].as_object().cloned().unwrap_or_default();
                    map.remove("anyOf");
                    map.extend(remaining);
                } else if branches
                    .iter()
                    .all(|b| b.get("const").is_some_and(Value::is_string))
                {
                    let known: Vec<Value> = branches.iter().map(|b| b["const"].clone()).collect();
                    map.remove("anyOf");
                    map.insert("type".into(), json!("string"));
                    map.insert("x-knownValues".into(), json!(known));
                }
            }
            if let Some(values) = map.remove("enum") {
                map.insert("x-knownValues".into(), values);
            }
            if map.get("additionalProperties") == Some(&Value::Bool(false)) {
                map.insert("additionalProperties".into(), Value::Bool(true));
            }
            if map.get("default") == Some(&Value::Null) {
                map.remove("default");
            }
            for value in map.values_mut() {
                normalize(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize(value);
            }
        }
        _ => {}
    }
}

/// A checked consumer state borrows the complete wire JSON. It never rebuilds
/// the hash input by serializing a DTO or a renderer projection.
#[derive(Debug, Clone, Copy)]
pub struct ValidatedSemanticState<'a> {
    raw: &'a Value,
}
impl<'a> ValidatedSemanticState<'a> {
    pub fn raw(&self) -> &'a Value {
        self.raw
    }
}

pub fn validate_semantic_state(value: &Value) -> Option<ValidatedSemanticState<'_>> {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    let schema = SCHEMA.get_or_init(semantic_state_schema);
    if !bounded(value, 1)
        || interaction_aip::canonical_json(value).len() > limits::MAX_PAYLOAD_BYTES
        || !matches_schema(value, schema, schema)
    {
        return None;
    }
    Some(ValidatedSemanticState { raw: value })
}

fn bounded(value: &Value, depth: usize) -> bool {
    if depth > limits::MAX_JSON_DEPTH - 1 {
        return false;
    }
    match value {
        Value::Null => false,
        Value::String(s) => s.chars().count() <= limits::MAX_STRING_CHARS,
        Value::Array(a) => a.iter().all(|v| bounded(v, depth + 1)),
        Value::Object(o) => o
            .iter()
            .all(|(k, v)| k.chars().count() <= limits::MAX_STRING_CHARS && bounded(v, depth + 1)),
        Value::Number(n) => n.as_f64().is_some_and(f64::is_finite),
        Value::Bool(_) => true,
    }
}

fn matches_schema(value: &Value, node: &Value, root: &Value) -> bool {
    if let Some(reference) = node["$ref"].as_str() {
        return root
            .pointer(reference.trim_start_matches('#'))
            .is_some_and(|s| matches_schema(value, s, root));
    }
    if let Some(branches) = node["oneOf"].as_array() {
        if branches
            .iter()
            .filter(|s| matches_schema(value, s, root))
            .count()
            != 1
        {
            return false;
        }
    }
    if let Some(branches) = node["anyOf"].as_array() {
        if !branches.iter().any(|s| matches_schema(value, s, root)) {
            return false;
        }
    }
    if node
        .get("not")
        .is_some_and(|s| matches_schema(value, s, root))
    {
        return false;
    }
    if node.get("const").is_some_and(|expected| value != expected) {
        return false;
    }
    if node["enum"]
        .as_array()
        .is_some_and(|values| !values.contains(value))
    {
        return false;
    }
    match node["type"].as_str() {
        Some("object") => {
            let Some(object) = value.as_object() else {
                return false;
            };
            if node["required"].as_array().is_some_and(|keys| {
                keys.iter()
                    .any(|key| !object.contains_key(key.as_str().unwrap_or("")))
            }) {
                return false;
            }
            if let Some(properties) = node["properties"].as_object() {
                for (key, schema) in properties {
                    if object
                        .get(key)
                        .is_some_and(|value| !matches_schema(value, schema, root))
                    {
                        return false;
                    }
                }
            }
        }
        Some("array") => {
            let Some(array) = value.as_array() else {
                return false;
            };
            if node["maxItems"]
                .as_u64()
                .is_some_and(|max| array.len() as u64 > max)
            {
                return false;
            }
            if let Some(items) = node.get("items") {
                if !array.iter().all(|value| matches_schema(value, items, root)) {
                    return false;
                }
            }
        }
        Some("string") => {
            let Some(text) = value.as_str() else {
                return false;
            };
            if node["format"] == "date-time" && chrono::DateTime::parse_from_rfc3339(text).is_err()
            {
                return false;
            }
        }
        Some("boolean") if !value.is_boolean() => return false,
        Some("number" | "integer") => {
            let Some(number) = value.as_f64() else {
                return false;
            };
            if !number.is_finite()
                || (node["type"] == "integer" && number.fract() != 0.0)
                || node["minimum"].as_f64().is_some_and(|min| number < min)
                || node["maximum"].as_f64().is_some_and(|max| number > max)
                || (node["x-nonNegativeZero"] == true && number.is_sign_negative())
            {
                return false;
            }
        }
        _ => {}
    }
    true
}

/// Inspect JSON structure before serde can turn a present null into Option::None.
pub(crate) fn contains_null(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(values) => values.iter().any(contains_null),
        Value::Object(values) => values.values().any(contains_null),
        _ => false,
    }
}
