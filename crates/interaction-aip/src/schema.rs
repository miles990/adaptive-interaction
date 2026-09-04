//! JSON Schema：由 Rust 型別產生單一文件（golden：`schemas/aip-1.0.schema.json`）。
//! TS／Swift 型別由 `scripts/aip-codegen.mjs` 從這份 schema 產生。

use schemars::JsonSchema;
use serde_json::{Map, Value};

pub const SCHEMA_TITLE: &str = "Adaptive Interaction Protocol 1.0";

/// 頂層根型別（固定順序）。
pub const ROOT_TYPES: [&str; 8] = [
    "Envelope",
    "Party",
    "ErrorPayload",
    "CapabilityAnnouncement",
    "NegotiatedCapabilities",
    "Outcome",
    "OfflinePolicy",
    "EvidenceClass",
];

fn add_root<T: JsonSchema>(name: &str, defs: &mut Map<String, Value>) {
    let root = schemars::schema_for!(T);
    let mut value = serde_json::to_value(root).unwrap_or_else(|_| Value::Object(Map::new()));
    if let Some(obj) = value.as_object_mut() {
        if let Some(Value::Object(nested)) = obj.remove("$defs") {
            for (k, v) in nested {
                defs.entry(k).or_insert(v);
            }
        }
        obj.remove("$schema");
        obj.entry("title".to_string())
            .or_insert_with(|| Value::String(name.to_string()));
    }
    defs.insert(name.to_string(), value);
}

/// 產生單一 JSON Schema 文件。
pub fn protocol_schema() -> Value {
    let mut defs = Map::new();
    add_root::<crate::Envelope>("Envelope", &mut defs);
    add_root::<crate::Party>("Party", &mut defs);
    add_root::<crate::ErrorPayload>("ErrorPayload", &mut defs);
    add_root::<crate::CapabilityAnnouncement>("CapabilityAnnouncement", &mut defs);
    add_root::<crate::NegotiatedCapabilities>("NegotiatedCapabilities", &mut defs);
    add_root::<crate::Outcome>("Outcome", &mut defs);
    add_root::<crate::OfflinePolicy>("OfflinePolicy", &mut defs);
    add_root::<crate::EvidenceClass>("EvidenceClass", &mut defs);
    let mut doc = Map::new();
    doc.insert(
        "$schema".into(),
        Value::String("https://json-schema.org/draft/2020-12/schema".into()),
    );
    doc.insert(
        "$id".into(),
        Value::String(
            "https://github.com/miles990/adaptive-interaction/schemas/aip-1.0.schema.json".into(),
        ),
    );
    doc.insert("title".into(), Value::String(SCHEMA_TITLE.into()));
    doc.insert(
        "description".into(),
        Value::String(
            "Adaptive Interaction Protocol (AIP): canonical JSON Schema generated from crates/interaction-aip. Roots are listed under $defs."
                .into(),
        ),
    );
    doc.insert(
        "specVersion".into(),
        Value::String(crate::SPEC_VERSION.into()),
    );
    doc.insert(
        "roots".into(),
        Value::Array(
            ROOT_TYPES
                .iter()
                .map(|s| Value::String((*s).into()))
                .collect(),
        ),
    );
    doc.insert(
        "messageTypes".into(),
        Value::Array(
            crate::MessageType::KNOWN
                .iter()
                .map(|m| Value::String(m.as_str().into()))
                .collect(),
        ),
    );
    doc.insert(
        "errorCodes".into(),
        Value::Array(
            crate::ErrorCode::KNOWN
                .iter()
                .map(|m| Value::String(m.as_str().into()))
                .collect(),
        ),
    );
    doc.insert(
        "limits".into(),
        serde_json::json!({
            "maxMessageBytes": crate::limits::MAX_MESSAGE_BYTES,
            "maxPayloadBytes": crate::limits::MAX_PAYLOAD_BYTES,
            "maxIdChars": crate::limits::MAX_ID_CHARS,
            "maxNameChars": crate::limits::MAX_NAME_CHARS,
            "maxStringChars": crate::limits::MAX_STRING_CHARS,
            "maxJsonDepth": crate::limits::MAX_JSON_DEPTH,
            "dedupeRing": crate::limits::DEDUPE_RING,
            "eventLogRing": crate::limits::EVENT_LOG_RING,
            "maxClockSkewMs": crate::limits::MAX_CLOCK_SKEW_MS,
            "defaultInteractionTtlMs": crate::limits::DEFAULT_INTERACTION_TTL_MS,
            "defaultIntentTtlMs": crate::limits::DEFAULT_INTENT_TTL_MS,
            "maxMembers": crate::limits::MAX_MEMBERS,
        }),
    );
    doc.insert("$defs".into(), Value::Object(defs));
    Value::Object(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_all_roots_and_is_stable() {
        let a = protocol_schema();
        let b = protocol_schema();
        assert_eq!(a, b);
        let defs = a["$defs"].as_object().expect("$defs");
        for root in ROOT_TYPES {
            assert!(defs.contains_key(root), "missing root {root}");
        }
        fn walk(v: &Value, defs: &Map<String, Value>) {
            match v {
                Value::Object(m) => {
                    if let Some(Value::String(r)) = m.get("$ref") {
                        let name = r.strip_prefix("#/$defs/").unwrap_or(r);
                        assert!(defs.contains_key(name), "dangling $ref {r}");
                    }
                    m.values().for_each(|x| walk(x, defs));
                }
                Value::Array(items) => items.iter().for_each(|x| walk(x, defs)),
                _ => {}
            }
        }
        walk(&a, defs);
        assert_eq!(a["messageTypes"].as_array().unwrap().len(), 12);
    }
}
