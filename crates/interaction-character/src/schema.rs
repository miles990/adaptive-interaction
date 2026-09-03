//! §10 JSON Schema：由 Rust 型別產生單一文件（golden：`schemas/character-protocol.schema.json`）。
//!
//! 頂層 `$defs` 收錄 `CharacterManifest`、`IntentEnvelope`、`CommandReceipt`、`CharacterInputEvent`、
//! `WireMessage`、`Hello`、`Negotiate`、`Negotiated` 以及它們引用的所有子型別；鍵序穩定
//! （serde_json 的 map 依鍵排序）。

use crate::input::CharacterInputEvent;
use crate::intent::IntentEnvelope;
use crate::manifest::CharacterManifest;
use crate::receipt::CommandReceipt;
use crate::wire::{Hello, Negotiate, Negotiated, WireMessage};
use crate::PROTOCOL_VERSION;
use schemars::JsonSchema;
use serde_json::{Map, Value};

/// 協定 schema 的 `title`。
pub const SCHEMA_TITLE: &str = "Character Presentation Protocol 1.0";

/// 頂層根型別名稱（固定順序）。
pub const ROOT_TYPES: [&str; 8] = [
    "CharacterManifest",
    "IntentEnvelope",
    "CommandReceipt",
    "CharacterInputEvent",
    "WireMessage",
    "Hello",
    "Negotiate",
    "Negotiated",
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
    add_root::<CharacterManifest>("CharacterManifest", &mut defs);
    add_root::<IntentEnvelope>("IntentEnvelope", &mut defs);
    add_root::<CommandReceipt>("CommandReceipt", &mut defs);
    add_root::<CharacterInputEvent>("CharacterInputEvent", &mut defs);
    add_root::<WireMessage>("WireMessage", &mut defs);
    add_root::<Hello>("Hello", &mut defs);
    add_root::<Negotiate>("Negotiate", &mut defs);
    add_root::<Negotiated>("Negotiated", &mut defs);
    let mut doc = Map::new();
    doc.insert(
        "$schema".into(),
        Value::String("https://json-schema.org/draft/2020-12/schema".into()),
    );
    doc.insert(
        "$id".into(),
        Value::String(
            "https://github.com/miles990/adaptive-interaction/schemas/character-protocol.schema.json"
                .into(),
        ),
    );
    doc.insert("title".into(), Value::String(SCHEMA_TITLE.into()));
    doc.insert(
        "description".into(),
        Value::String(
            "Character Presentation Protocol (CPP): canonical JSON Schema generated from crates/interaction-character. Roots are listed under $defs."
                .into(),
        ),
    );
    doc.insert(
        "protocolVersion".into(),
        Value::String(PROTOCOL_VERSION.into()),
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
        assert_eq!(a["title"], SCHEMA_TITLE);
        let defs = a["$defs"].as_object().expect("$defs");
        for root in ROOT_TYPES {
            assert!(defs.contains_key(root), "missing root {root}");
        }
        // 每個 $ref 都指向存在的 $defs。
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
        // adapter → runtime 型別不得含 truthState。
        for name in ["CommandReceipt", "CharacterInputEvent", "Negotiate"] {
            let props = defs[name]["properties"].as_object().expect("properties");
            assert!(
                !props.contains_key("truthState"),
                "{name} must not carry truthState"
            );
            assert!(
                !props.contains_key("verified"),
                "{name} must not carry verified"
            );
        }
        assert!(defs["IntentEnvelope"]["properties"]
            .as_object()
            .is_some_and(|p| p.contains_key("truthState")));
    }
}
