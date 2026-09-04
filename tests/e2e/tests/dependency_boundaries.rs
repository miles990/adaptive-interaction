//! 依賴邊界：`docs/aip/architecture-boundaries.md` §1「依賴只能朝向核心」的可執行版本。
//!
//! `interaction-aip` 與 `interaction-session` 是純領域 crate。它們一旦長出 runtime／transport
//! 依賴（tokio、axum、Tauri、WebSocket／BLE／MQTT／Serial／HTTP client），架構邊界就已經破了，
//! 之後任何「核心不碰 I/O」的說法都不再成立。這個測試把那條線釘在 CI 裡。

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

/// 純領域 crate 不得（直接或遞移）依賴的套件。
const FORBIDDEN: [&str; 9] = [
    "tokio",
    "axum",
    "tauri",
    "tungstenite",
    "rumqttc",
    "serialport",
    "btleplug",
    "reqwest",
    "hyper",
];

/// 受這條邊界保護的 crate。
const PURE_CRATES: [&str; 2] = ["interaction-aip", "interaction-session"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 只取 `[dependencies]`／`[build-dependencies]` 區段的套件名（`[dev-dependencies]` 允許測試工具）。
fn declared_dependencies(manifest: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut in_scope = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_scope = matches!(
                line,
                "[dependencies]" | "[build-dependencies]" | "[target.'cfg(unix)'.dependencies]"
            ) || (line.starts_with("[dependencies.")
                || line.starts_with("[build-dependencies."));
            if line.starts_with("[dependencies.") || line.starts_with("[build-dependencies.") {
                if let Some(name) = line
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split('.')
                    .nth(1)
                {
                    names.insert(name.to_string());
                }
            }
            continue;
        }
        if !in_scope || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            let name = name.trim().trim_matches('"');
            if !name.is_empty() {
                names.insert(name.to_string());
            }
        }
    }
    names
}

#[test]
fn pure_crates_declare_no_transport_or_runtime_dependencies() {
    for crate_name in PURE_CRATES {
        let path = repo_root().join(format!("crates/{crate_name}/Cargo.toml"));
        let manifest = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {crate_name}/Cargo.toml: {e}"));
        let declared = declared_dependencies(&manifest);
        assert!(
            !declared.is_empty(),
            "{crate_name}: parsed zero dependencies, the manifest parser is broken"
        );
        for banned in FORBIDDEN {
            assert!(
                !declared.contains(banned),
                "{crate_name} declares `{banned}`; pure domain crates must stay off the runtime/transport layer \
                 (docs/aip/architecture-boundaries.md §1)"
            );
        }
    }
}

/// 執行一次 `cargo metadata --locked` 並回傳解析後的 JSON。
///
/// **不會 skip**：`cargo metadata` 跑不起來時直接 panic。「沒辦法驗證」與「驗證通過」
/// 必須在退出碼上分得出來——`cargo test` 預設吃掉通過測試的 stderr，一句
/// `eprintln!("SKIPPED …")` 在 CI／發布驗證裡看起來就是綠燈，那正是誠實階梯要擋的
/// 「未驗證卻宣稱通過」。
fn cargo_metadata() -> Value {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "dependency boundary is UNVERIFIED: cannot run `cargo metadata` ({e}). \
                 A boundary that cannot be checked must fail, not pass \
                 (docs/aip/architecture-boundaries.md §1)"
            )
        });
    assert!(
        output.status.success(),
        "dependency boundary is UNVERIFIED: `cargo metadata --locked` exited with {:?}. \
         A stale Cargo.lock or a broken workspace must fail this test, not silently skip it: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(400)
            .collect::<String>()
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata is JSON")
}

/// 從 `crate_name` 出發走完 normal 依賴圖，回傳能走到的所有套件名。
///
/// dev-dependencies 不跟隨：測試工具不會進到成品。
fn reachable_packages(metadata: &Value, crate_name: &str) -> BTreeSet<String> {
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .expect("resolve.nodes");
    let packages = metadata["packages"].as_array().expect("packages");

    let root = packages
        .iter()
        .find(|p| p["name"] == crate_name)
        .and_then(|p| p["id"].as_str())
        .unwrap_or_else(|| panic!("{crate_name} is not a workspace member"))
        .to_string();
    let name_of = |id: &str| -> String {
        packages
            .iter()
            .find(|p| p["id"] == id)
            .and_then(|p| p["name"].as_str())
            .unwrap_or("<unknown>")
            .to_string()
    };

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::from([root]);
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(node) = nodes.iter().find(|n| n["id"] == id.as_str()) else {
            continue;
        };
        for dep in node["deps"].as_array().cloned().unwrap_or_default() {
            let normal = dep["dep_kinds"]
                .as_array()
                .map(|kinds| kinds.iter().any(|k| k["kind"].is_null()))
                .unwrap_or(true);
            if !normal {
                continue;
            }
            if let Some(pkg) = dep["pkg"].as_str() {
                queue.push_back(pkg.to_string());
            }
        }
    }
    let reached: BTreeSet<String> = seen.iter().map(|id| name_of(id)).collect();
    assert!(
        reached.contains(crate_name),
        "{crate_name}: the dependency walk found nothing, the metadata parser is broken"
    );
    reached
}

/// 走到的套件裡有哪些是禁用的。
fn forbidden_reached(reached: &BTreeSet<String>) -> Vec<&'static str> {
    FORBIDDEN
        .iter()
        .copied()
        .filter(|banned| reached.contains(*banned))
        .collect()
}

/// 直接依賴乾淨還不夠：遞移依賴也不能把**任何一個** FORBIDDEN 套件拉進來。
///
/// 純 crate 之間是 path 依賴（`interaction-session` → `interaction-character`），
/// 只釘 tokio 的話，下游改成引入 axum／reqwest／tungstenite／rumqttc／serialport／
/// btleplug／hyper 其中任何一個，這裡都還是綠的。
#[test]
fn pure_crates_do_not_pull_transport_or_runtime_crates_transitively() {
    let metadata = cargo_metadata();
    for crate_name in PURE_CRATES {
        let reached = reachable_packages(&metadata, crate_name);
        let banned = forbidden_reached(&reached);
        assert!(
            banned.is_empty(),
            "{crate_name} reaches {banned:?} transitively; the pure domain layer must stay off the \
             runtime/transport layer (docs/aip/architecture-boundaries.md §1)"
        );
    }
}

/// 反向控制組：同一套走圖＋比對邏輯，用在**本來就坐在傳輸層**的 crate 上必須抓得到東西。
///
/// 沒有這一條的話，上面那個測試綠燈有兩種可能——邊界真的乾淨，或者走圖／比對根本沒在
/// 運作（例如 `cargo metadata` 的 id 格式換了、`name_of` 全部回 `<unknown>`）。
/// 兩種都會印同一行 `ok`，而只有一種是真的。
#[test]
fn the_transitive_check_actually_detects_a_banned_crate() {
    let metadata = cargo_metadata();
    let reached = reachable_packages(&metadata, "interaction-api");
    let banned = forbidden_reached(&reached);
    for expected in ["tokio", "axum", "hyper"] {
        assert!(
            banned.contains(&expected),
            "the transitive walk failed to see `{expected}` under interaction-api (it declares it \
             directly); a green boundary test would therefore prove nothing. reached {} packages, \
             banned {banned:?}",
            reached.len()
        );
    }
}
