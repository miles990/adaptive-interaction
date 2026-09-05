//! **Canonical JSON 向量**（AIP §6）：鍵序、跳脫、數字字面的三端契約。
//!
//! 為什麼要另外一段：`stateHashes` 那組 fixture 是**真的 `SemanticState`**，鍵全是 ASCII
//! 欄位名（`mood`／`intensity`／`members`…）。也就是說，三端從來沒有在
//!
//!   * 非 ASCII 鍵、補充平面鍵（`Array.from` vs UTF-16 code unit 序）、
//!   * 需要跳脫的鍵與值（控制字元、引號、反斜線）與**不需要**跳脫但看起來很像要跳脫的
//!     字元（U+2028／U+2029／U+007F／`/`），
//!   * 整數與小數的字面（`1` vs `1.0`、`-0.0`，以及 ryu 的固定小數 ↔ 科學記號分界
//!     ——`1e-6` 與 `1e+16` 在 JS 的 `String()` 底下印成 `0.000001` 與 `10000000000000000`）
//!
//! 這三件事上交過答案。對抗審查 `hash-numeric-contract-017` 指出 TypeScript 端的鍵序曾經
//! 是 UTF-16 code unit 序（補充平面鍵會排到 U+F801..U+FFFF 的 BMP 鍵**之前**，與 Rust
//! 的 `keys.sort()`／Swift 的 UTF-8 位元組序相反）；那個 bug 修掉了（363c3d7），但當時沒有
//! 任何一筆 fixture 抓得住它——這一段就是補上那張網。
//!
//! 產生器兼驗證器：
//!
//! ```bash
//! cargo test -p interaction-aip --test canonical_vectors                    # 比對（平常）
//! AIP_UPDATE_FIXTURES=1 cargo test -p interaction-aip --test canonical_vectors  # 重生
//! ```
//!
//! 重生之後要在 `apps/interaction-desktop` 跑 `pnpm aip:codegen`（Swift 端把 manifest
//! 內嵌成字串）。消費端：
//!
//! | 端 | 測試 |
//! |---|---|
//! | TypeScript | `apps/interaction-desktop/src/test/canonical-vectors.test.ts` |
//! | Swift | `apps/interaction-ios/InteractionCompanionTests/CanonicalVectorsTests.swift` |
//!
//! **向量不遷就實作**：任何一端對不上，要修的是那一端的 canonical 實作，不是這裡的期望值。
//!
//! 邊界（刻意不涵蓋，見 `docs/aip/conformance.md` §3）：
//!
//!   * 整數字面只到 ±2^53。TypeScript 端是用 `JSON.parse` 讀 manifest 的，超過這個範圍的
//!     整數在那一步就已經失真，不是 canonical 實作能補救的；把它寫進向量只會逼人改壞向量。
//!   * `-0`（**整數**負零）：serde_json 讀成 `0`、寫成 `0`，而 JS 的 `JSON.parse("-0")` 是
//!     `-0`，`Object.is` 分得出來。同樣是解析層的差異，不是 canonical 層的。`-0.0`（f64）
//!     則有涵蓋，因為那是 host 真的寫得出來的字面。
//!   * `1E3`／`0.10` 這類**非正規**字面：Swift 端逐字保留原字面，Rust 會正規化成
//!     `1000.0`／`0.1`。manifest 裡的字面一律由 serde_json 寫出，所以三端看到的就是正規形。

use interaction_aip::{canonical_hash, canonical_json};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// 一筆向量。`input` 必須是 JSON **物件**（鍵序才有意義）。
struct Vector {
    id: &'static str,
    note: &'static str,
    input: Value,
}

/// 向量本體。每一筆都針對一個具體的分歧點，不是「多測幾個例子」。
fn vectors() -> Vec<Vector> {
    vec![
        Vector {
            id: "ascii-keys-unsorted",
            note: "基準線：純 ASCII 鍵、輸入順序與排序後不同；巢狀物件與陣列內的物件都要遞迴排序",
            input: json!({
                "zulu": true,
                "alpha": null,
                "Mike": "M",
                "_underscore": 1,
                "0-digit": 2,
                "nested": { "b": [ { "y": 1, "x": 2 } ], "a": "A" }
            }),
        },
        Vector {
            id: "bmp-non-ascii-keys",
            note: "BMP 非 ASCII 鍵，含 U+FFFD 與 U+F801..U+FFFF 區間（私用區、noncharacter）；值是該鍵排序後的 0-based 位置",
            input: json!({
                "zz": 0,
                "\u{00e9}": 1,
                "\u{03c0}": 2,
                "\u{4e2d}": 3,
                "\u{fb00}": 4,
                "\u{f801}": 5,
                "\u{fffd}": 6,
                "\u{ffff}": 7
            }),
        },
        Vector {
            id: "supplementary-plane-keys",
            note: "補充平面鍵（U+10000、音樂符號、emoji、U+10FFFF）；值是排序後的位置",
            input: json!({
                "\u{10000}": 0,
                "\u{1d11e}": 1,
                "\u{1f642}": 2,
                "\u{10ffff}": 3
            }),
        },
        Vector {
            id: "code-point-order-not-utf16",
            note: "證明 code point 序 ≠ UTF-16 code unit 序：補充平面鍵在 code point 序**在後**，在 code unit 序卻因為代理對開頭 0xD800 排到 U+F801 之前。值是 code point 序的位置",
            input: json!({
                "A": 0,
                "\u{f801}": 1,
                "\u{fffd}": 2,
                "\u{ffff}": 3,
                "\u{10000}": 4,
                "\u{1f642}": 5
            }),
        },
        Vector {
            id: "escaped-keys-and-values",
            note: "必須跳脫的鍵與值：引號、反斜線、\\b \\t \\n \\f \\r 短寫，以及沒有短寫的控制字元（U+0000、U+001F）寫成小寫 \\u00xx",
            input: json!({
                "quote\"key": "quote\"value",
                "back\\slash": "back\\slash",
                "tab\there": "tab\there",
                "newline\nhere": "newline\nhere",
                "return\rhere": "return\rhere",
                "backspace\u{0008}here": "backspace\u{0008}here",
                "formfeed\u{000c}here": "formfeed\u{000c}here",
                "nul\u{0000}here": "nul\u{0000}here",
                "unit\u{001f}here": "all of them: \"\\\u{0008}\u{000c}\n\r\t\u{0000}\u{001f}"
            }),
        },
        Vector {
            id: "unescaped-passthrough",
            note: "看起來像要跳脫、但 serde_json **不**跳脫的：U+2028／U+2029（JS 舊版原始碼的行終結符）、U+007F（DEL）、U+00A0 與 `/`。三端都必須原樣輸出",
            input: json!({
                "solidus/key": "a/b",
                "del\u{007f}here": "del\u{007f}here",
                "nbsp\u{00a0}here": "nbsp\u{00a0}here",
                "ls\u{2028}here": "line separator: \u{2028}",
                "ps\u{2029}here": "paragraph separator: \u{2029}"
            }),
        },
        Vector {
            id: "combining-marks-not-unicode-collation",
            note: "組合附加符號：`e` + U+0301 在 UTF-8 位元組序（＝ code point 序）排在 `f`／`z` **之前**，但 Swift 的 `String` `<` 先做 NFC（變成 U+00E9）再比，會把它排到最後。值是 code point 序的位置",
            input: json!({
                "e": 0,
                "e\u{0301}": 1,
                "f": 2,
                "z": 3
            }),
        },
        Vector {
            id: "nested-key-order-recursion",
            note: "鍵序必須遞迴：陣列裡的物件、物件裡的物件、空字串鍵與非 ASCII 鍵混排，五層深",
            input: json!({
                "z": {
                    "\u{4e2d}": {
                        "b": [
                            { "y": 1, "a": 2 },
                            { "\u{10000}": 3, "~": 4, "/": 5 }
                        ],
                        "a": { "": { "\u{03b2}": 6, "\u{03b1}": 7 } }
                    },
                    "A": 8
                },
                "a": [ [ { "b": 9, "a": 10 } ] ]
            }),
        },
        Vector {
            id: "numbers-integers",
            note: "整數字面原樣輸出（沒有小數點）。上下界刻意停在 ±2^53：TypeScript 端用 JSON.parse 讀 manifest，超過就已經在解析層失真",
            input: json!({
                "zero": 0,
                "one": 1,
                "minus-one": -1,
                "max-safe": 9_007_199_254_740_992i64,
                "min-safe": -9_007_199_254_740_992i64,
                "wide": 1_234_567_890_123i64,
                "in-array": [0, -1, 42]
            }),
        },
        Vector {
            id: "numbers-doubles",
            note: "f64 字面：整數值也要帶小數點（1.0／-0.0／2.0），其餘是最短 round-trip 十進位。鍵 `~tilde/slash` 同時釘住 doublePaths 的 RFC 6901 跳脫",
            input: json!({
                "half": 0.5,
                "neg-quarter": -0.25,
                "unit": 1.0,
                "neg-zero": -0.0,
                "pi": std::f64::consts::PI,
                "tiny": 1e-7,
                "huge": 1e21,
                "~tilde/slash": 2.0,
                "in-array": [0.5, 1.0]
            }),
        },
        Vector {
            id: "numbers-exponent-forms",
            note: "f64 的固定小數 ↔ 科學記號分界：serde_json（ryu）在第一位數的十進位指數 k ∈ [-5, 16) 用固定小數，其餘用 `1e+16`／`1e-6` 這種帶正負號、不補零的指數形。JS 的 `String()` 分界是 (-7, 21)，兩者在 1e-6 與 1e16..1e21 之間完全不同",
            input: json!({
                "fixed-low-edge": 1e-5,
                "sci-low-edge": 1e-6,
                "fixed-high-edge": 1e15,
                "sci-high-edge": 1e16,
                "js-prints-fixed": 1e20,
                "js-prints-sci": 1e21,
                "two-pow-53": 9_007_199_254_740_992.0,
                "many-digits": 123_456_789_012_345_680_000.0,
                "smallest-subnormal": 5e-324,
                "largest-finite": 1.797_693_134_862_315_7e308,
                "negative": -1e-6
            }),
        },
        Vector {
            id: "empty-containers",
            note: "空物件與空陣列（含空字串鍵、陣列裡的空容器）：`{}`／`[]` 不得消失、不得變成 null",
            input: json!({
                "": {},
                "a": [],
                "b": [[], {}, [{}, []]],
                "c": { "": { "": [] } }
            }),
        },
    ]
}

// ------------------------------------------------------------------ 產生器

/// 一筆向量在 manifest 裡的樣子。
fn entry(v: &Vector) -> Value {
    let mut map = Map::new();
    map.insert("id".into(), json!(v.id));
    map.insert("note".into(), json!(v.note));
    map.insert("doublePaths".into(), json!(double_paths(&v.input)));
    map.insert("input".into(), v.input.clone());
    map.insert("canonical".into(), json!(canonical_json(&v.input)));
    map.insert("sha256".into(), json!(canonical_hash(&v.input)));
    Value::Object(map)
}

fn entries() -> Vec<Value> {
    vectors().iter().map(entry).collect()
}

/// 這筆向量裡所有 **f64** 值的 RFC 6901 pointer。
///
/// TypeScript 端唯一需要的「型別知識」：JS 的 `number` 分不出 `1` 與 `1.0`，但知道哪些
/// 路徑是 f64 之後就能把整數值印回 `1.0`。與 `SEMANTIC_STATE_DOUBLE_PATHS` 是同一個機制，
/// 只是這裡的來源是向量本身而不是 schema。
fn double_paths(input: &Value) -> Vec<String> {
    fn walk(value: &Value, pointer: &str, out: &mut Vec<String>) {
        match value {
            Value::Number(n) if n.is_f64() => out.push(pointer.to_string()),
            Value::Object(map) => {
                for (key, child) in map {
                    walk(child, &format!("{pointer}/{}", escape_pointer(key)), out);
                }
            }
            Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    walk(child, &format!("{pointer}/{i}"), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(input, "", &mut out);
    out.sort();
    out
}

/// RFC 6901：`~` → `~0`、`/` → `~1`（順序不可反）。
fn escape_pointer(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn section_text(list: &[Value]) -> String {
    let rendered: Vec<String> = list
        .iter()
        .map(|entry| {
            serde_json::to_string_pretty(entry)
                .expect("entry serializes")
                .lines()
                .map(|line| format!("    {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect();
    // U+2028／U+2029 在 JSON 字串裡合法，serde_json 也原樣寫出——但 manifest 會被
    // `scripts/aip-codegen.mjs` 內嵌進 Swift 原始碼的 raw string，而 Swift 的 lexer 把
    // U+2028 當換行、對裸的 DEL 會抱怨，裸的組合附加符號則會黏到前一個原始碼字元上。寫成 ` ` 是**同一個 JSON 值**（三端解析回來都是那個字元），
    // 只是讓檔案本身留在「原始碼安全」的字元集裡。
    let body = rendered
        .join(",\n")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
        .replace('\u{007f}', "\\u007f")
        .replace('\u{0301}', "\\u0301");
    format!("\"canonicalVectors\": [\n{body}\n  ]")
}

fn splice_manifest(text: &str, section: &str) -> String {
    const KEY: &str = "\"canonicalVectors\": [";
    if let Some(start) = text.find(KEY) {
        let close = text[start..]
            .find("\n  ]")
            .map(|i| start + i + "\n  ]".len())
            .expect("canonicalVectors 段以兩格縮排的 `]` 結束");
        format!("{}{}{}", &text[..start], section, &text[close..])
    } else {
        let end = text
            .trim_end()
            .strip_suffix('}')
            .expect("manifest ends with `}`")
            .trim_end()
            .len();
        format!("{},\n  {}\n}}\n", &text[..end], section)
    }
}

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("manifest.json")
}

fn update_requested() -> bool {
    std::env::var("AIP_UPDATE_FIXTURES").is_ok_and(|v| v == "1")
}

// ------------------------------------------------------------------ 測試

#[test]
fn canonical_vectors_match_the_authoritative_implementation() {
    let list = entries();
    let path = manifest_path();
    let mut text = std::fs::read_to_string(&path).expect("manifest.json readable");

    if update_requested() {
        text = splice_manifest(&text, &section_text(&list));
        std::fs::write(&path, &text).expect("manifest written");
    }

    let manifest: Value = serde_json::from_str(&text).expect("manifest.json is JSON");
    let on_disk = manifest
        .get("canonicalVectors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            panic!("manifest.json 缺 `canonicalVectors` 段：用 AIP_UPDATE_FIXTURES=1 重生")
        });

    assert_eq!(
        on_disk, list,
        "manifest.json 的 canonicalVectors 與產生器不一致：AIP_UPDATE_FIXTURES=1 重生"
    );
}

#[test]
fn the_vector_set_covers_every_divergence_it_claims_to() {
    let list = vectors();
    assert!(
        list.len() >= 8,
        "canonical 向量至少要 8 筆（鍵序／跳脫／數字字面三類都要有），現在只有 {}",
        list.len()
    );

    let ids: BTreeSet<&str> = list.iter().map(|v| v.id).collect();
    assert_eq!(ids.len(), list.len(), "向量 id 必須唯一");

    for v in &list {
        assert!(
            v.input.is_object(),
            "{}：input 必須是 JSON 物件（鍵序才有東西可證明）",
            v.id
        );
    }

    // 每一筆的 hash 都不同：碰撞代表兩筆向量其實在測同一件事。
    let hashes: BTreeSet<String> = list.iter().map(|v| canonical_hash(&v.input)).collect();
    assert_eq!(hashes.len(), list.len(), "每一筆向量的 sha256 都必須不同");

    // canonical 文字本身是合法 JSON，而且再 canonical 一次是不動點。
    for v in &list {
        let text = canonical_json(&v.input);
        let reparsed: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{}：canonical 不是合法 JSON（{e}）", v.id));
        assert_eq!(
            canonical_json(&reparsed),
            text,
            "{}：canonical 不是不動點（再跑一次就變了）",
            v.id
        );
    }

    // 至少一筆有非 ASCII 鍵、一筆有補充平面鍵、一筆有控制字元、一筆有 f64、一筆有空容器。
    let all_keys: Vec<String> = list.iter().flat_map(|v| collect_keys(&v.input)).collect();
    assert!(
        all_keys
            .iter()
            .any(|k| k.chars().any(|c| c as u32 > 0x7f && (c as u32) < 0x1_0000)),
        "沒有任何 BMP 非 ASCII 鍵"
    );
    assert!(
        all_keys
            .iter()
            .any(|k| k.chars().any(|c| c as u32 >= 0x1_0000)),
        "沒有任何補充平面鍵"
    );
    assert!(
        all_keys
            .iter()
            .any(|k| k.chars().any(|c| (c as u32) < 0x20)),
        "沒有任何含控制字元的鍵"
    );
    assert!(
        list.iter().any(|v| !double_paths(&v.input).is_empty()),
        "沒有任何 f64 值"
    );
}

/// U+2028／U+2029／U+007F／`/` 在 serde_json 是**不跳脫**的；這條在文件裡寫過，但沒有
/// 測試釘住過。跳脫了會是「三端各自跳脫成不同形式」的開端。
#[test]
fn passthrough_characters_are_not_escaped() {
    let vector = vectors()
        .into_iter()
        .find(|v| v.id == "unescaped-passthrough")
        .expect("unescaped-passthrough 向量存在");
    let text = canonical_json(&vector.input);
    for (name, ch) in [
        ("U+2028", '\u{2028}'),
        ("U+2029", '\u{2029}'),
        ("U+007F", '\u{007f}'),
        ("U+00A0", '\u{00a0}'),
        ("solidus", '/'),
    ] {
        assert!(
            text.contains(ch),
            "{name} 必須原樣出現在 canonical 文字裡（被跳脫了）"
        );
    }
    assert!(
        !text.contains("\\u2028") && !text.contains("\\/"),
        "canonical 文字不得含 \\u2028 或 \\/ 這種跳脫形式"
    );
}

/// 指數向量的存在理由：它必須**同時**含有 ryu 印成固定小數、與 ryu 印成科學記號的值，
/// 而且至少一個值是「JS 的 `String()` 會印成另一種形式」的。少了任何一邊，這筆向量就
/// 抓不到 TypeScript 端把 `1e+16` 印成 `10000000000000000.0` 那一類的分歧。
#[test]
fn the_exponent_vector_straddles_the_fixed_to_scientific_boundary() {
    let vector = vectors()
        .into_iter()
        .find(|v| v.id == "numbers-exponent-forms")
        .expect("numbers-exponent-forms 向量存在");
    let text = canonical_json(&vector.input);
    assert!(
        text.contains("0.00001"),
        "缺少 ryu 印成固定小數的值（k = -5 那一格）"
    );
    assert!(
        text.contains("1000000000000000.0"),
        "缺少 ryu 印成固定小數的值（k = 15 那一格）"
    );
    assert!(text.contains("1e-6"), "缺少 k = -6 的科學記號");
    assert!(text.contains("1e+16"), "缺少 k = 16 的科學記號");
    assert!(
        text.contains("1e+20"),
        "缺少 k = 20——JS 的 String() 在這裡印固定小數，ryu 印科學記號"
    );
}

/// 組合附加符號向量的存在理由：它必須讓「UTF-8 位元組序」與「先正規化再比」分岔。
///
/// Swift 的 `String` `<` 會先做 NFC（`e` + U+0301 → U+00E9）再比 scalar，於是那個鍵會排到
/// `f`／`z` **之後**；位元組序則是排在**之前**（`e` 是共同前綴）。`CharacterSemantic.swift`
/// 明確用 `Array(key.utf8).lexicographicallyPrecedes` 就是為了這件事——在這筆向量出現以前，
/// 把它換成 `map.keys.sorted()` 三端照樣全綠。
#[test]
fn the_combining_mark_vector_separates_byte_order_from_normalized_order() {
    let vector = vectors()
        .into_iter()
        .find(|v| v.id == "combining-marks-not-unicode-collation")
        .expect("combining-marks-not-unicode-collation 向量存在");
    let keys: Vec<String> = collect_top_level_keys(&vector.input);
    let combining = keys
        .iter()
        .find(|k| k.chars().any(|c| c == '\u{0301}'))
        .expect("向量裡要有一個帶組合附加符號的鍵");
    assert!(
        keys.iter()
            .any(|k| k.as_str() > combining.as_str() && k.is_ascii()),
        "位元組序必須把組合附加符號的鍵排在某個純 ASCII 鍵之前，否則分不開兩種排序"
    );
    // NFC 之後那個鍵是 U+00E9，比任何 ASCII 都大——這正是 Swift `<` 會給的相反答案。
    assert!(
        combining.replace('\u{0301}', "") == "e",
        "這筆向量假設組合附加符號掛在 `e` 上（NFC 之後是 U+00E9）"
    );
}

/// 這一筆向量的存在理由：code point 序與 UTF-16 code unit 序必須給出**不同**的答案。
/// 哪天有人把補充平面鍵拿掉，這條會紅——向量就不再證明任何事了。
#[test]
fn the_ordering_vector_actually_separates_code_point_from_utf16_order() {
    let vector = vectors()
        .into_iter()
        .find(|v| v.id == "code-point-order-not-utf16")
        .expect("code-point-order-not-utf16 向量存在");
    let keys: Vec<String> = collect_top_level_keys(&vector.input);

    let mut by_code_point = keys.clone();
    by_code_point.sort(); // Rust String Ord = UTF-8 位元組序 = code point 序

    let mut by_utf16 = keys.clone();
    by_utf16.sort_by(|a, b| {
        let left: Vec<u16> = a.encode_utf16().collect();
        let right: Vec<u16> = b.encode_utf16().collect();
        left.cmp(&right)
    });

    assert_ne!(
        by_code_point, by_utf16,
        "這筆向量沒有分開兩種排序，等於證明不了 hash-numeric-contract-017 修好了"
    );

    // 值就是 code point 序的位置：canonical 文字裡的值必須是 0,1,2,…
    let ordered: Vec<i64> = by_code_point
        .iter()
        .map(|k| vector.input[k].as_i64().expect("值是整數"))
        .collect();
    assert_eq!(
        ordered,
        (0..ordered.len() as i64).collect::<Vec<_>>(),
        "向量的值必須標出 code point 序的位置，否則錯誤訊息讀不出哪裡排錯"
    );
}

fn collect_top_level_keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

fn collect_keys(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    out.push(key.clone());
                    walk(child, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    walk(value, &mut out);
    out
}
