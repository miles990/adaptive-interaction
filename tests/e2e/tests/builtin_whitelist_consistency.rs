//! builtin adapter 白名單的跨語言一致性（`docs/aip/architecture-boundaries.md` §4）。
//!
//! 「有哪些 in-process builtin adapter」這件事在 repo 裡被寫了三次：
//!
//! | 位置 | 用途 |
//! |---|---|
//! | `crates/interaction-runtime/src/character.rs` `CHARACTER_BUILTIN_ENTRYPOINTS` | Rust 端 gate：`/v1/character/hello` 與 adapter 註冊的 manifest 驗證 |
//! | `apps/interaction-desktop/src/character/adapterRegistry.ts` `BUILTIN_ADAPTER_IDS` | TS 端 gate：`manifest.ts` 驗證與 `createBuiltinAdapter` |
//! | `apps/interaction-desktop/src/test/architecture-no-entrypoint-switch.test.ts` `ADAPTER_IDS` | 架構守門測試掃描的 id（可選：改成 import 就不需要這一份） |
//!
//! 三份各自手寫、順序不同、沒有任何測試比對。只在 TS 端加第五個 builtin adapter 時，
//! Runtime 會用 `ManifestErrorCode::Entrypoint` 拒絕那個角色的 manifest（hello 協商與
//! Tauri 匯入都失敗），而 TS 測試全綠——§4 想避免的「加角色要改 host」以另一種形式復發。
//!
//! 這個測試讀**兩邊的原始碼**做比對（repo 已有先例：`hitRegions.test.ts`、
//! `companion-gateway-wiring.test.ts` 讀 `src-tauri/src/*.rs`）。

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// 取出 `<name> ... = [ ... ]` 裡的所有雙引號／單引號字面字串。
/// 先跳到 `=`（Rust 的 `[&str; 4]` 型別標註也是中括號，不能直接抓第一個 `[`），
/// 再取它後面第一組中括號。找不到那個名字就回 `None`（讓呼叫端決定這是缺陷，
/// 還是「已經改成 import 了」）。
fn literal_ids_after(source: &str, name: &str) -> Option<Vec<String>> {
    let start = source.find(name)? + name.len();
    let rest = &source[start..];
    let equals = rest.find('=')?;
    let rest = &rest[equals..];
    let open = rest.find('[')?;
    let close = rest[open..].find(']')? + open;
    let body = &rest[open + 1..close];
    let mut ids = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '"' && c != '\'' {
            continue;
        }
        let quote = c;
        let mut value = String::new();
        for next in chars.by_ref() {
            if next == quote {
                break;
            }
            value.push(next);
        }
        ids.push(value);
    }
    Some(ids)
}

/// 取出 `<name> ... = "..."` 的字面字串（純量常數）。
fn literal_string_after(source: &str, name: &str) -> Option<String> {
    let start = source.find(name)? + name.len();
    let rest = &source[start..];
    let equals = rest.find('=')?;
    let rest = &rest[equals..];
    let open = rest.find('"')? + 1;
    let close = rest[open..].find('"')? + open;
    Some(rest[open..close].to_string())
}

/// Rust 端白名單。`shu-rig` 的字面值住在 `interaction-character-shu`
/// （Runtime 只引用 `ShuRigPack::ENTRYPOINT_ID`），所以那一項要去它的定義處取。
fn rust_entrypoints() -> BTreeSet<String> {
    let character = read("crates/interaction-runtime/src/character.rs");
    let listed = literal_ids_after(&character, "CHARACTER_BUILTIN_ENTRYPOINTS")
        .expect("CHARACTER_BUILTIN_ENTRYPOINTS 必須是一個字面陣列");
    let shu = read("crates/interaction-character-shu/src/lib.rs");
    // 用完整的宣告樣式定位，避免命中檔頭 doc comment 裡的同名字樣。
    let shu_id = literal_string_after(&shu, "ENTRYPOINT_ID: &'static str")
        .expect("ShuRigPack::ENTRYPOINT_ID 必須是一個字面字串");

    let mut ids = BTreeSet::new();
    for id in listed {
        ids.insert(id);
    }
    // 陣列裡以常數書寫的那一項（`ShuRigPack::ENTRYPOINT_ID`）不是字面值，補上它。
    assert!(
        character.contains("ShuRigPack::ENTRYPOINT_ID"),
        "Runtime 不得自己寫死 shu-rig 的字串；它應該引用 ShuRigPack::ENTRYPOINT_ID"
    );
    ids.insert(shu_id);
    ids
}

fn ts_builtin_ids() -> BTreeSet<String> {
    let source = read("apps/interaction-desktop/src/character/adapterRegistry.ts");
    literal_ids_after(&source, "BUILTIN_ADAPTER_IDS")
        .expect("BUILTIN_ADAPTER_IDS 必須是一個字面陣列")
        .into_iter()
        .collect()
}

/// 兩份白名單（Rust gate 與 TS gate）必須是同一個集合（順序無關）。
///
/// 不一致的後果不是測試變紅，而是**使用者看到角色載入失敗**：TS 端載得起來的
/// builtin adapter，Runtime 會在 manifest 驗證時以 `entrypoint` 錯誤碼拒絕。
#[test]
fn the_rust_and_typescript_builtin_whitelists_are_the_same_set() {
    let rust = rust_entrypoints();
    let ts = ts_builtin_ids();
    assert_eq!(
        rust, ts,
        "Rust（CHARACTER_BUILTIN_ENTRYPOINTS）與 TS（BUILTIN_ADAPTER_IDS）的 builtin \
         白名單不一致：只在一邊加 adapter，另一邊會拒絕該角色的 manifest"
    );
    assert!(!rust.is_empty(), "白名單不得是空的（那樣沒有角色載得起來）");
}

/// 架構守門測試掃描的 id 清單如果還是手寫的第三份，它也必須跟著一致。
/// 已經改成 `import { BUILTIN_ADAPTER_IDS }` 的話就沒有第三份可比，這裡放行。
#[test]
fn the_architecture_guard_test_scans_the_same_ids() {
    let source =
        read("apps/interaction-desktop/src/test/architecture-no-entrypoint-switch.test.ts");
    if source.contains("BUILTIN_ADAPTER_IDS") {
        // 直接 import 了唯一那一份：沒有第三份可以走樣。
        return;
    }
    let scanned: BTreeSet<String> = literal_ids_after(&source, "ADAPTER_IDS")
        .expect("架構守門測試要嘛 import BUILTIN_ADAPTER_IDS，要嘛留一份字面陣列")
        .into_iter()
        .collect();
    assert_eq!(
        scanned,
        ts_builtin_ids(),
        "架構守門測試掃描的 id 與 BUILTIN_ADAPTER_IDS 不一致：新加的 adapter \
         寫死分岔時不會被擋下"
    );
}
