//! 第三方 adapter 一致性測試（Character Presentation Protocol 1.0 §13）。
//!
//! 對每一份 manifest 跑同一組驗收：manifest 通過驗證 → 20 個 intent 全部有解析結果 →
//! 安全 intent 永不 `unsupported`（最差也只是 `system.text`）→ `claimed` 不會被角色端
//! 變成 `verified` → `emergency` 的 priority floor 仍是 100（角色不能把它調低）。
//!
//! 涵蓋範圍：
//! - `examples/character-adapters/*.manifest.json`（外部程序參考 adapter）
//! - `apps/interaction-desktop/public/characters/*/manifest.json`（內建角色）
//! - 環境變數 `CPP_CONFORMANCE_MANIFESTS`（以 `:` 分隔的額外路徑，可以是檔案或目錄）
//!
//! 第三方作者的用法見 `docs/character-protocol/adapter-authoring.md` §11。

mod common;

use common::t;
use interaction_character::*;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 收集要驗的 manifest 檔案路徑（確定性排序）。
fn manifest_paths() -> Vec<PathBuf> {
    let root = repo_root();
    let mut paths: Vec<PathBuf> = Vec::new();

    let examples = root.join("examples/character-adapters");
    push_dir_manifests(&examples, "manifest.json", &mut paths);

    let characters = root.join("apps/interaction-desktop/public/characters");
    if let Ok(entries) = std::fs::read_dir(&characters) {
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let file = dir.join("manifest.json");
            if file.is_file() {
                paths.push(file);
            }
        }
    }

    // 第三方：CPP_CONFORMANCE_MANIFESTS="/path/a.json:/path/to/dir"
    if let Ok(extra) = std::env::var("CPP_CONFORMANCE_MANIFESTS") {
        for item in extra.split(':').map(str::trim).filter(|s| !s.is_empty()) {
            let path = Path::new(item);
            if path.is_dir() {
                push_dir_manifests(path, ".json", &mut paths);
                let nested = path.join("manifest.json");
                if nested.is_file() {
                    paths.push(nested);
                }
            } else if path.is_file() {
                paths.push(path.to_path_buf());
            } else {
                panic!("CPP_CONFORMANCE_MANIFESTS 指到不存在的路徑：{item}");
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn push_dir_manifests(dir: &Path, suffix: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(suffix))
        })
        .collect();
    files.sort();
    out.append(&mut files);
}

fn load(path: &Path) -> CharacterManifest {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} 不是合法 JSON：{e}", path.display()));
    // 舊 Character Pack（`kind: "character-pack"`）先遷移成 CPP manifest 再驗。
    if value.get("kind").and_then(|v| v.as_str()) == Some("character-pack") {
        return migrate_legacy_pack(&value)
            .unwrap_or_else(|e| panic!("{} 無法從舊 pack 遷移：{e}", path.display()));
    }
    serde_json::from_value(value)
        .unwrap_or_else(|e| panic!("{} 不是合法的 CPP manifest：{e}", path.display()))
}

/// 對一份 manifest 跑完整驗收；回傳供彙總的協商結果。
fn conform(path: &Path, manifest: &CharacterManifest) -> Negotiated {
    let label = path.display().to_string();

    // 1. manifest 通過驗證（大小、能力 id、資產路徑、fallback…）。
    let bytes = serde_json::to_vec(manifest).unwrap_or_default().len();
    validate_manifest(bytes, manifest, &ValidationLimits::default())
        .unwrap_or_else(|e| panic!("{label}: manifest 驗證失敗：{e}"));

    // 2. 以「角色照 manifest 全數提供」協商：20 個 intent 全部有結果。
    let mut gw = Gateway::default();
    let id = gw.register_instance(manifest.clone(), CharacterRole::PrimaryCompanion);
    let (negotiated, _) = gw
        .on_negotiate(&id, Negotiate::from_manifest(manifest, 1), t(0))
        .unwrap_or_else(|e| panic!("{label}: 協商失敗：{e}"));
    assert_eq!(
        negotiated.resolutions.len(),
        20,
        "{label}: 20 個 intent 必須全部有解析結果"
    );

    for intent in CharacterIntent::ALL {
        let r = negotiated
            .resolutions
            .get(&intent)
            .unwrap_or_else(|| panic!("{label}: {intent} 沒有解析結果"));
        // 3. 安全 intent 永不遺失：最差也是 system.text。
        if intent.is_safety() {
            assert_ne!(
                r.resolution,
                Resolution::Unsupported,
                "{label}: 安全 intent {intent} 不得 unsupported"
            );
            assert!(
                r.via.is_some(),
                "{label}: 安全 intent {intent} 必須有承載能力或 system.text"
            );
        }
        // 4. claimed 不會變成 verified：intent fallback 不得把任何 intent 換成
        //    verified-success，變體名也不得借用 verified 的別名。
        assert_ne!(
            r.via_intent,
            Some(CharacterIntent::VerifiedSuccess),
            "{label}: {intent} 不得經由 verified-success 呈現"
        );
        if intent != CharacterIntent::VerifiedSuccess {
            if let Some(variant) = &r.variant {
                assert!(
                    !intent_variant_aliases(CharacterIntent::VerifiedSuccess)
                        .contains(&variant.as_str()),
                    "{label}: {intent} 挑到了 verified-success 的變體 {variant}"
                );
            }
        }
    }
    // ClaimCompleted 另外明確檢查一次（README §11：claimed ≠ verified）。
    let claimed = &negotiated.resolutions[&CharacterIntent::ClaimCompleted];
    assert_ne!(
        claimed.via_intent,
        Some(CharacterIntent::VerifiedSuccess),
        "{label}: claim-completed 不得退成 verified-success"
    );
    assert!(
        manifest
            .fallbacks
            .intents
            .get("claim-completed")
            .map(|target| target != CharacterIntent::VerifiedSuccess.as_str())
            .unwrap_or(true),
        "{label}: fallbacks.intents 不得把 claim-completed 換成 verified-success"
    );

    // 5. emergency 的 floor 仍是 100：角色端不能把安全 intent 調低。
    assert_eq!(
        CharacterIntent::Emergency.priority_floor(),
        100,
        "{label}: emergency floor 必須是 100"
    );
    let envelope = IntentEnvelope::from_runtime(
        "conformance-emergency",
        id.as_str(),
        Some("conformance".into()),
        CharacterIntent::Emergency,
        TruthState::Emergency,
        0, // 故意請求最低 priority
        t(1),
        t(61),
    );
    assert_eq!(
        envelope.priority, 100,
        "{label}: emergency 的 priority 必須被 floor 夾到 100"
    );
    let out = gw.dispatch(&id, envelope, t(1));
    let delivered = out.iter().any(|o| match o {
        GatewayOutput::Send {
            message: WireMessage::Intent { envelope },
            ..
        } => envelope.intent == CharacterIntent::Emergency && envelope.priority == 100,
        GatewayOutput::SystemText { intent, .. } => *intent == CharacterIntent::Emergency,
        _ => false,
    });
    assert!(
        delivered,
        "{label}: emergency 必須送到 adapter 或落到 system.text，不得遺失"
    );
    negotiated
}

#[test]
fn every_bundled_and_third_party_manifest_conforms() {
    let paths = manifest_paths();
    assert!(
        paths.len() >= 2,
        "至少要有參考 adapter 與內建角色的 manifest，實際找到 {}",
        paths.len()
    );
    for path in &paths {
        let manifest = load(path);
        conform(path, &manifest);
    }
    eprintln!("CPP conformance: {} 份 manifest 通過", paths.len());
}

#[test]
fn conformance_covers_the_reference_adapters() {
    let paths = manifest_paths();
    let names: Vec<String> = paths
        .iter()
        .map(|p| p.display().to_string().replace('\\', "/"))
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.ends_with("examples/character-adapters/text-adapter.manifest.json")),
        "外部程序參考 adapter 的 manifest 必須在驗收範圍內：{names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.contains("public/characters/") && n.ends_with("manifest.json")),
        "內建角色的 manifest 必須在驗收範圍內：{names:?}"
    );
}
