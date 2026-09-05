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
            // 這一項不是 wire 上的長度上限，而是**協商結果**必須有界的要求：
            // `unsupportedInputs` 來自對方宣告的 `inputs`（外部輸入、本身無界），而協商回覆
            // 是一則要送上線的訊息。三端各自實作協商，所以截斷點必須是同一個數字——它進
            // schema 是為了讓 TS／Swift 從 codegen 讀到權威值，不是為了描述 wire 格式。
            "maxUnsupportedInputs": crate::limits::MAX_UNSUPPORTED_INPUTS,
            // 接收端規則的兩個上界（AIP 1.0 接收端澄清）：resume 回覆最多幾則 patch、
            // 連續 realign 幾次就是 unrecoverable。三端各自實作接收端狀態機，數字必須同一個。
            "maxResumePatches": crate::limits::MAX_RESUME_PATCHES,
            "maxRealignAttempts": crate::limits::MAX_REALIGN_ATTEMPTS,
        }),
    );
    doc.insert("$defs".into(), Value::Object(defs));
    Value::Object(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SCREAMING_SNAKE` → `camelCase`（`MAX_JSON_DEPTH` → `maxJsonDepth`）。
    fn camel(name: &str) -> String {
        let mut out = String::new();
        for (i, part) in name.split('_').enumerate() {
            let lower = part.to_lowercase();
            if i == 0 {
                out.push_str(&lower);
            } else {
                let mut chars = lower.chars();
                if let Some(first) = chars.next() {
                    out.extend(first.to_uppercase());
                    out.push_str(chars.as_str());
                }
            }
        }
        out
    }

    /// **漂移 gate**：`limits.rs` 裡的每一個上限都必須出現在 schema 的 `limits` 表。
    ///
    /// schema 是 TS／Swift codegen 的唯一來源（`scripts/aip-codegen.mjs` → `AIP_LIMITS`／
    /// `AIPLimits`）。漏一個常數，就會有一端只能手寫一份同值的字面量——兩份數字各自演化，
    /// 而且沒有任何測試會發現（v0.6.0 的 `MAX_UNSUPPORTED_INPUTS` 就是這樣：Rust 32、
    /// TypeScript 另外手寫 32）。這裡直接讀 `limits.rs` 的原始碼列舉常數名，
    /// 新增常數而忘了進 schema 就紅燈。
    #[test]
    fn every_limit_constant_is_published_in_the_schema() {
        let source = include_str!("limits.rs");
        let declared: Vec<String> = source
            .lines()
            .filter_map(|line| line.trim().strip_prefix("pub const "))
            .filter_map(|rest| rest.split(':').next())
            .map(|name| camel(name.trim()))
            .collect();
        assert!(
            declared.len() >= 13,
            "limits.rs 的常數解析失敗（只找到 {declared:?}）"
        );
        let schema = protocol_schema();
        let published = schema["limits"].as_object().expect("limits object");
        for key in &declared {
            assert!(
                published.contains_key(key),
                "limits.rs 有 `{key}` 但 schema 的 limits 表沒有：TS／Swift 會被迫手寫同一個數字"
            );
        }
        let mut extra: Vec<&String> = published.keys().filter(|k| !declared.contains(k)).collect();
        extra.sort();
        assert!(
            extra.is_empty(),
            "schema 的 limits 表有 limits.rs 沒有的鍵：{extra:?}"
        );
    }

    /// 協商結果的 `unsupportedInputs` 上限必須以權威值出現在 schema（TS 端讀它，不再手寫）。
    #[test]
    fn max_unsupported_inputs_is_published_with_the_authoritative_value() {
        let schema = protocol_schema();
        assert_eq!(
            schema["limits"]["maxUnsupportedInputs"],
            serde_json::json!(crate::limits::MAX_UNSUPPORTED_INPUTS)
        );
    }

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
