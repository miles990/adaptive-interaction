//! interaction-aip：Adaptive Interaction Protocol（AIP）1.0 的**權威實作**。
//!
//! 對應規格：`docs/aip/README.md`。純邏輯 crate：沒有 tokio、沒有 I/O，時間由呼叫端以
//! [`Timestamp`] 或 millis 注入，所有規則都可確定性測試。JSON Schema（golden
//! `schemas/aip-1.0.schema.json`）由 [`schema::protocol_schema`] 從這裡的型別產生；TS／Swift 型別由
//! `scripts/aip-codegen.mjs` 從同一份 schema 產生，不得手寫分歧。
//!
//! | 模組 | 規格章節 |
//! |---|---|
//! | [`envelope`] | §1 Envelope、§2.2 profiles、§7 deadline／dedupe |
//! | [`message`] | §2 十二種 message type、§2.3 name 命名空間 |
//! | [`outcome`] | §3 Outcome 階梯 |
//! | [`error`] | §12 穩定錯誤碼與 error payload |
//! | [`version`] | §4 版本與協商 |
//! | [`capability`] | §4.2 capability 宣告與協商 |
//! | [`offline`] | §8 離線事件政策 |
//! | [`evidence`] | §10 證據分類 |
//! | [`limits`] | §11 上限 |
//! | [`schema`] | JSON Schema 產生 |
//!
//! 不變量：`source` 只是宣稱（§5）；未知 message type 不執行；`verified` 只有 Runtime 可產生；
//! 所有集合有界。

pub mod capability;
pub mod envelope;
pub mod error;
pub mod evidence;
pub mod limits;
pub mod message;
pub mod offline;
pub mod outcome;
pub mod schema;
pub mod version;

pub use capability::*;
pub use envelope::*;
pub use error::*;
pub use evidence::*;
pub use message::*;
pub use offline::*;
pub use outcome::*;
pub use version::*;

/// UTC 時間戳（RFC 3339 序列化）。
pub type Timestamp = chrono::DateTime<chrono::Utc>;

/// 本實作的 spec 版本字串。
pub const SPEC_VERSION: &str = "aip/1.0";
/// 本實作的 major。不同 major 一律拒絕。
pub const SPEC_MAJOR: u32 = 1;
/// 本實作的 minor。對方較新時取 min 並標 `newerMinor`。
pub const SPEC_MINOR: u32 = 0;

/// Canonical JSON（鍵排序、無空白）→ SHA-256 hex。state hash 與 golden 比對都用它。
pub fn canonical_hash(value: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let canonical = canonical_json(value);
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}

/// 鍵排序、無空白的 JSON 文字（serde_json 的 Map 預設是 BTreeMap 排序；這裡再遞迴確保）。
pub fn canonical_json(value: &serde_json::Value) -> String {
    fn write(v: &serde_json::Value, out: &mut String) {
        match v {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(k).unwrap_or_default());
                    out.push(':');
                    write(&map[*k], out);
                }
                out.push('}');
            }
            serde_json::Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_sorts_keys_recursively_and_is_stable() {
        let a = json!({"b": {"y": 1, "x": [3, {"q": 1, "p": 2}]}, "a": "z"});
        let b = json!({"a": "z", "b": {"x": [3, {"p": 2, "q": 1}], "y": 1}});
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
        assert_eq!(
            canonical_json(&a),
            r#"{"a":"z","b":{"x":[3,{"p":2,"q":1}],"y":1}}"#
        );
    }
}
