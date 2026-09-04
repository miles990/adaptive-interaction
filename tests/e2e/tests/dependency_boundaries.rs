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

/// 直接依賴乾淨還不夠：遞移依賴也不能把 tokio 拉進來。
#[test]
fn pure_crates_do_not_pull_tokio_transitively() {
    let output = match Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(repo_root())
        .output()
    {
        Ok(out) => out,
        Err(e) => {
            eprintln!(
                "SKIPPED pure_crates_do_not_pull_tokio_transitively: cannot run `cargo metadata` ({e}); \
                 the direct-dependency assertion above still ran"
            );
            return;
        }
    };
    if !output.status.success() {
        eprintln!(
            "SKIPPED pure_crates_do_not_pull_tokio_transitively: `cargo metadata` exited with {:?}; \
             the direct-dependency assertion above still ran",
            output.status.code()
        );
        return;
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("cargo metadata is JSON");
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .expect("resolve.nodes");
    let packages = metadata["packages"].as_array().expect("packages");

    let id_of = |name: &str| -> String {
        packages
            .iter()
            .find(|p| p["name"] == name)
            .and_then(|p| p["id"].as_str())
            .unwrap_or_else(|| panic!("{name} is not a workspace member"))
            .to_string()
    };
    let name_of = |id: &str| -> String {
        packages
            .iter()
            .find(|p| p["id"] == id)
            .and_then(|p| p["name"].as_str())
            .unwrap_or("<unknown>")
            .to_string()
    };

    for crate_name in PURE_CRATES {
        let root = id_of(crate_name);
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<String> = VecDeque::from([root.clone()]);
        while let Some(id) = queue.pop_front() {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(node) = nodes.iter().find(|n| n["id"] == id.as_str()) else {
                continue;
            };
            for dep in node["deps"].as_array().cloned().unwrap_or_default() {
                // 只跟隨 normal 依賴：dev-dependencies 不會進到成品。
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
        assert!(
            !reached.contains("tokio"),
            "{crate_name} reaches tokio transitively; the pure domain layer must stay runtime-free \
             (docs/aip/architecture-boundaries.md §1)"
        );
    }
}
