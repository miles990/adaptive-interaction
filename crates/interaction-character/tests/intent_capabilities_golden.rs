//! §3.4 的 intent→能力表是**確定性演算法**的資料層，Rust 為權威、TypeScript 為鏡射
//! （`docs/character-protocol/README.md` 開頭）。兩邊各自維護時會靜默漂移：同一份 manifest
//! 在 Runtime gateway 與視窗 gateway 得到不同的 resolution／via。
//!
//! 這裡把權威表凍結成一份 golden JSON，Rust 與 TypeScript（`apps/interaction-desktop`
//! 的 `character-protocol.test.ts`）各自對它斷言，任一邊改動就會失敗。
//!
//! 重生：`UPDATE_CPP_GOLDEN=1 cargo test -p interaction-character --test intent_capabilities_golden`

use interaction_character::{intent_capabilities, CharacterIntent};
use std::path::PathBuf;

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/intent-capabilities.json")
}

fn table() -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for intent in CharacterIntent::ALL {
        map.insert(
            intent.as_str().to_string(),
            serde_json::Value::Array(
                intent_capabilities(intent)
                    .iter()
                    .map(|c| serde_json::Value::String((*c).to_string()))
                    .collect(),
            ),
        );
    }
    serde_json::Value::Object(map)
}

#[test]
fn intent_capability_table_matches_the_cross_language_golden() {
    let path = golden_path();
    let actual = table();
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(&actual).expect("golden serializes")
    );
    if std::env::var("UPDATE_CPP_GOLDEN").is_ok() {
        std::fs::write(&path, &rendered).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let expected: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"));
    assert_eq!(
        actual, expected,
        "intent→能力表與 golden 不符。若這是刻意的協定變更，請同時更新 \
         apps/interaction-desktop/src/character/protocol.ts 的 INTENT_CAPABILITIES，\
         再以 UPDATE_CPP_GOLDEN=1 重生這份 golden。"
    );
    assert_eq!(
        actual.as_object().map(|o| o.len()),
        Some(20),
        "20 個 intent 必須全部在表內"
    );
}
