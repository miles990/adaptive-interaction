//! §13：Manifest schema validation、malicious manifest、path traversal、legacy pack migration。

mod common;

use common::*;
use interaction_character::*;

fn err_code(result: Result<ManifestReport, ManifestError>) -> ManifestErrorCode {
    match result {
        Ok(report) => panic!("expected an error, got report {report:?}"),
        Err(e) => e.code,
    }
}

#[test]
fn valid_shu_maid_style_manifest_passes_with_report() {
    let manifest = shu_manifest();
    let report = validate(&manifest).expect("valid");
    assert!(!report.newer_minor);
    assert!(!report.executable);
    assert!(!report.needs_network);
    assert!(!report.external);
    assert_eq!(report.unknown_fields, vec!["futureField"]);
    assert!(manifest.extra.contains_key("x-vendor"));
    assert_eq!(
        report.custom_capabilities,
        vec![
            CustomCapabilityNote {
                id: "com.example.character.wings".into(),
                unknown: false
            },
            CustomCapabilityNote {
                id: "visual.wings".into(),
                unknown: true
            },
        ]
    );
    assert!(report.unknown_intents.is_empty());
    // 未知欄位保留、round-trip 不崩潰。
    let back = serde_json::to_value(&manifest).expect("serialize");
    assert_eq!(back["futureField"]["anything"], true);
    assert_eq!(back["entrypoint"]["kind"], "builtin");
    assert_eq!(back["pronouns"]["zh-TW"], "她");
}

#[test]
fn sprite_and_text_manifests_are_valid() {
    let sprite = sprite_manifest();
    let report = validate(&sprite).expect("sprite valid");
    assert!(report.custom_capabilities.is_empty());
    assert_eq!(
        sprite.entrypoint,
        Entrypoint::Builtin {
            id: "sprite".into()
        }
    );
    let text = text_manifest();
    validate(&text).expect("text valid");
    assert_eq!(text.capabilities.len(), 3);
}

#[test]
fn parse_manifest_from_bytes_enforces_size_and_json() {
    let limits = test_limits(&TEST_BUILTIN_WHITELIST);
    let bytes = SHU_MAID_JSON.as_bytes();
    let (manifest, _) = parse_manifest(bytes, &limits).expect("parses");
    assert_eq!(manifest.character_id, "shu-maid");
    let big = vec![b' '; MAX_MANIFEST_BYTES + 1];
    let err = parse_manifest(&big, &limits).expect_err("too large");
    assert_eq!(err.code, ManifestErrorCode::TooLarge);
    let err = parse_manifest(b"{\"characterId\": \"/Users/secret\"", &limits).expect_err("json");
    assert_eq!(err.code, ManifestErrorCode::Json);
    assert!(
        !err.message.contains("/Users"),
        "json errors must not echo input: {}",
        err.message
    );
}

#[test]
fn schema_version_major_and_newer_minor() {
    let mut m = shu_manifest();
    m.schema_version = "1.3".into();
    assert!(validate(&m).expect("newer minor ok").newer_minor);
    m.schema_version = "2.0".into();
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::SchemaVersion);
    m.schema_version = "1".into();
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::SchemaVersion);
    m.schema_version = "one.zero".into();
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::SchemaVersion);
}

#[test]
fn character_id_and_localized_text_bounds() {
    let mut m = shu_manifest();
    m.character_id = "Shu Maid".into();
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::CharacterId);
    let mut m = shu_manifest();
    m.display_name.clear();
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::LocalizedText);
    let mut m = shu_manifest();
    m.display_name.insert("en".into(), "x".repeat(49));
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::LocalizedText);
    let mut m = shu_manifest();
    m.description.insert("en".into(), "x".repeat(401));
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::LocalizedText);
    let mut m = shu_manifest();
    m.author = Some("x".repeat(121));
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::LocalizedText);
    let mut m = shu_manifest();
    m.variants[0]
        .display_name
        .insert("en".into(), "x".repeat(49));
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::LocalizedText);
}

#[test]
fn malicious_asset_paths_are_rejected_without_echo() {
    for bad in [
        "../secret.png",
        "img/../../secret.png",
        "/Users/victim/secret.png",
        "C:\\Windows\\system32\\x.png",
        "C:/x.png",
        "https://evil.example/x.png",
        "file:///etc/passwd",
        "img\\sheet.png",
        "~/x.png",
        "./sheet.png",
        "",
    ] {
        let mut m = shu_manifest();
        m.assets[0].path = bad.into();
        let err = validate(&m).expect_err(bad);
        assert_eq!(err.code, ManifestErrorCode::AssetPath, "{bad}");
        assert_eq!(err.path, "assets[0].path");
        if !bad.is_empty() {
            assert!(
                !err.message.contains(bad),
                "message echoes path: {}",
                err.message
            );
        }
        assert!(!err.message.contains("/Users"));
    }
    // entrypoint.module 走同一套路徑規則。
    let mut m = shu_manifest();
    m.adapter_kind = AdapterKind::Web;
    m.entrypoint = Entrypoint::Module {
        path: "../../evil.mjs".into(),
    };
    let err = validate(&m).expect_err("module traversal");
    assert_eq!(err.code, ManifestErrorCode::Entrypoint);
    assert!(!err.message.contains("evil"));
}

#[test]
fn oversize_limits() {
    let m = shu_manifest();
    let err = validate_manifest(
        MAX_MANIFEST_BYTES + 1,
        &m,
        &test_limits(&TEST_BUILTIN_WHITELIST),
    )
    .expect_err("too large");
    assert_eq!(err.code, ManifestErrorCode::TooLarge);

    let mut m = shu_manifest();
    for i in 0..65 {
        m.assets.push(AssetDecl {
            id: format!("a{i}"),
            path: format!("a{i}.png"),
            media_type: "image/png".into(),
            bytes: Some(1),
            sha256: None,
        });
    }
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Assets);

    let mut m = shu_manifest();
    m.assets[0].bytes = Some(8 * 1024 * 1024 + 1);
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::AssetBytes);

    let mut m = shu_manifest();
    m.resource_limits.max_asset_bytes = 33 * 1024 * 1024;
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::ResourceLimits);

    let mut m = shu_manifest();
    m.resource_limits.max_queue = 65;
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::ResourceLimits);

    let mut m = shu_manifest();
    if let Some(decl) = m.capabilities.get_mut("visual.expression") {
        decl.duration_range = Some(DurationRange {
            min_ms: 0,
            max_ms: 60_001,
        });
    }
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Capability);
}

#[test]
fn process_and_url_entrypoints_are_recorded_not_executed() {
    let mut m = shu_manifest();
    m.adapter_kind = AdapterKind::ExternalProcess;
    m.entrypoint = Entrypoint::Process {
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "echo pwned > /tmp/pwned".into(),
        ],
    };
    let report = validate(&m).expect("recorded");
    assert!(report.executable);
    assert!(report.external);
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("never auto-started")));
    assert!(m.entrypoint.is_executable());

    // entrypoint 種類必須與 adapterKind 一致（process 不能偽裝成 in-process）。
    let mut m = shu_manifest();
    m.entrypoint = Entrypoint::Process {
        command: vec!["evil".into()],
    };
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Entrypoint);

    let mut m = shu_manifest();
    m.entrypoint = Entrypoint::Builtin { id: "evil".into() };
    let err = validate(&m).expect_err("not whitelisted");
    assert_eq!(err.code, ManifestErrorCode::Entrypoint);
    // 錯誤訊息列出 host 注入的白名單（核心自己沒有預設值）。
    assert!(err.message.contains(&TEST_BUILTIN_WHITELIST.join(", ")));

    let mut m = shu_manifest();
    m.adapter_kind = AdapterKind::RemoteDevice;
    m.entrypoint = Entrypoint::Url {
        url: "http://evil.example/".into(),
    };
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Entrypoint);
    m.entrypoint = Entrypoint::Url {
        url: "ws://127.0.0.1:9000/character".into(),
    };
    let report = validate(&m).expect("url recorded");
    assert!(report.needs_network);
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("never auto-connected")));
}

#[test]
fn capability_id_rules() {
    let mut m = shu_manifest();
    m.capabilities
        .insert("wings".into(), CapabilityDecl::supported());
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Capability);

    let mut m = shu_manifest();
    m.capabilities
        .insert("system.text".into(), CapabilityDecl::supported());
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Capability);

    let mut m = shu_manifest();
    m.capabilities
        .insert("input.click".into(), CapabilityDecl::supported());
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Capability);

    let mut m = shu_manifest();
    m.input_capabilities
        .insert("visual.pose".into(), CapabilityDecl::supported());
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Capability);

    let mut m = shu_manifest();
    m.input_capabilities.insert(
        "com.example.input.gesture".into(),
        CapabilityDecl::supported(),
    );
    let report = validate(&m).expect("custom input ok");
    assert!(report
        .custom_capabilities
        .iter()
        .any(|c| c.id == "com.example.input.gesture" && !c.unknown));
}

#[test]
fn preferences_schema_subset_rules() {
    let good: PreferencesSchema = serde_json::from_str(
        r#"{"type":"object","properties":{"a":{"type":"boolean"},"b":{"type":"string","maxLength":200,"enum":["x"]},
            "c":{"type":"number","minimum":0,"maximum":1},"d":{"type":"integer"}},"required":["a"]}"#,
    )
    .expect("good schema");
    let mut m = shu_manifest();
    m.preferences_schema = Some(good);
    validate(&m).expect("subset ok");

    let bad_cases = [
        r#"{"type":"array"}"#,
        r##"{"type":"object","$ref":"#/x"}"##,
        r#"{"type":"object","properties":{"a":{"type":"string","pattern":"^x$"}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"object","properties":{}}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"array","items":{"type":"string"}}}}"#,
        r##"{"type":"object","properties":{"a":{"$ref":"#/defs/x","type":"string"}}}"##,
        r#"{"type":"object","properties":{"a":{"type":"string","maxLength":201}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"number","enum":["1"]}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"boolean","minimum":1}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"number","minimum":2,"maximum":1}}}"#,
        r#"{"type":"object","properties":{"a":{"type":"string","default":1}}}"#,
        r#"{"type":"object","properties":{},"required":["missing"]}"#,
    ];
    for bad in bad_cases {
        let schema: PreferencesSchema =
            serde_json::from_str(bad).unwrap_or_else(|e| panic!("{bad}: {e}"));
        let mut m = shu_manifest();
        m.preferences_schema = Some(schema);
        assert_eq!(
            err_code(validate(&m)),
            ManifestErrorCode::PreferencesSchema,
            "{bad}"
        );
    }
    let mut many = String::from(r#"{"type":"object","properties":{"#);
    for i in 0..33 {
        if i > 0 {
            many.push(',');
        }
        many.push_str(&format!(r#""p{i}":{{"type":"boolean"}}"#));
    }
    many.push_str("}}");
    let schema: PreferencesSchema = serde_json::from_str(&many).expect("33 props parse");
    let mut m = shu_manifest();
    m.preferences_schema = Some(schema);
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::PreferencesSchema);
    let enum17 = format!(
        r#"{{"type":"object","properties":{{"a":{{"type":"string","enum":[{}]}}}}}}"#,
        (0..17)
            .map(|i| format!("\"v{i}\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    let schema: PreferencesSchema = serde_json::from_str(&enum17).expect("enum17");
    let mut m = shu_manifest();
    m.preferences_schema = Some(schema);
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::PreferencesSchema);
}

#[test]
fn error_messages_never_echo_more_than_200_chars() {
    let mut m = shu_manifest();
    let long_key = format!("com.{}.x.y", "a".repeat(2000));
    m.capabilities.insert(long_key, CapabilityDecl::supported());
    // 名稱合法（namespaced），改用 variants 太長觸發錯誤。
    let long_variant = "v".repeat(70);
    if let Some(decl) = m.capabilities.get_mut("visual.pose") {
        decl.variants = vec![long_variant];
    }
    let err = validate(&m).expect_err("variant too long");
    assert!(
        err.path.chars().count() <= 260,
        "path {} chars",
        err.path.chars().count()
    );
    assert!(err.message.chars().count() <= 201);
    let long_field = "f".repeat(1000);
    let mut m = shu_manifest();
    m.extra.insert(long_field, serde_json::json!(1));
    let report = validate(&m).expect("unknown field ok");
    assert!(report
        .unknown_fields
        .iter()
        .all(|f| f.chars().count() <= 201));
}

#[test]
fn compatibility_and_fallbacks_are_checked() {
    let mut m = shu_manifest();
    m.compatibility = Some(Compatibility {
        protocol: "2.x".into(),
        runtime: None,
    });
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Compatibility);
    let mut m = shu_manifest();
    m.fallbacks
        .capabilities
        .insert("visual.pose".into(), vec!["visual.pose".into()]);
    assert_eq!(err_code(validate(&m)), ManifestErrorCode::Fallbacks);
    let mut m = shu_manifest();
    m.fallbacks.intents.insert("work".into(), "dance".into());
    let report = validate(&m).expect("unknown fallback intent is a warning");
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("fallbacks.intents.work")));
    let mut m = shu_manifest();
    m.intents.push("dance".into());
    let report = validate(&m).expect("unknown intent kept");
    assert_eq!(report.unknown_intents, vec!["dance"]);
}

#[test]
fn magic_bytes_reject_spoofed_extension() {
    // 檔名／MIME 說是 PNG，內容是 GIF：不信任。
    assert!(!asset_magic_matches("image/png", b"GIF89a\x00\x00"));
    assert!(asset_magic_matches("image/gif", b"GIF89a\x00\x00"));
    // SVG 裡藏 HTML 不算 SVG。
    assert!(!asset_magic_matches(
        "image/svg+xml",
        b"<html><script>alert(1)</script></html>"
    ));
    // 未知 MIME 一律不信任。
    assert!(!asset_magic_matches("application/x-sh", b"#!/bin/sh"));
}

#[test]
fn legacy_pack_v1_migrates_to_sprite_manifest() {
    let pack = read_pack("shu-standard");
    let m = migrate_pack_to_manifest(&pack, &core_registry()).expect("v1.0 migrates");
    assert_eq!(m.character_id, "shu-standard");
    assert_eq!(m.adapter_kind, AdapterKind::InProcess);
    assert_eq!(
        m.entrypoint,
        Entrypoint::Builtin {
            id: "sprite".into()
        }
    );
    assert_eq!(m.display_name["zh-TW"], "小樞・標準型");
    assert_eq!(
        m.assets.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        vec!["sheet", "preview"]
    );
    assert_eq!(m.assets[0].path, "sheet.png");
    assert_eq!(
        m.assets[0].bytes, None,
        "unknown bytes stay None (no fake numbers)"
    );
    assert!(m.capabilities.contains_key("visual.presence"));
    let expr = &m.capabilities["visual.expression"];
    assert_eq!(expr.variants.len(), 18);
    assert!(expr.variants.iter().any(|v| v == "thinking"));
    assert!(
        !m.capabilities.contains_key("visual.gaze"),
        "v1.0 has no anchors"
    );
    assert_eq!(m.input_capabilities.len(), 5);
    for id in [
        "input.click",
        "input.drag",
        "input.drop",
        "input.text",
        "input.fileDrop",
    ] {
        assert!(m.input_capabilities.contains_key(id), "{id}");
    }
    for native in [
        "idle",
        "notice",
        "think",
        "work",
        "wait",
        "ask",
        "blocked",
        "unknown",
        "verified-success",
        "offline",
        "emergency",
        "rest",
        "sleep",
    ] {
        assert!(
            m.intents.iter().any(|i| i == native),
            "missing native intent {native}"
        );
    }
    assert!(
        !m.intents.iter().any(|i| i == "failed"),
        "v1.0 has no failed animation"
    );
    assert_eq!(m.fallbacks.intents["failed"], "blocked");
    assert_eq!(m.fallbacks.intents["play"], "notice");
    // 舊 renderer 的 emergency → paused（sleep）會把安全語意換成日常演出：遷移時丟掉，
    // 讓 emergency 改走能力鏈／system.text（呈現層沒有權限主權）。
    assert!(
        !m.fallbacks.intents.contains_key("emergency"),
        "emergency 不得被遷移成非安全 intent"
    );
    for (from, to) in &m.fallbacks.intents {
        if let (Some(from), Some(to)) = (CharacterIntent::parse(from), CharacterIntent::parse(to)) {
            assert!(
                !from.is_safety() || to.is_safety(),
                "遷移產生的 {from} → {to} 改寫了安全語意"
            );
        }
    }
    assert_eq!(m.extra["x-legacy"]["hasAnchors"], false);
    assert_eq!(m.extra["x-legacy"]["columns"], 8);
    assert_eq!(m.states.len(), 18);
    let report = validate(&m).expect("migrated manifest validates");
    assert!(
        report.unknown_fields.is_empty(),
        "x-legacy is a vendor extension"
    );
}

#[test]
fn legacy_pack_v1_1_with_anchors_gains_gaze_and_failed() {
    let pack = read_pack("shu-lively");
    let m = migrate_pack_to_manifest(&pack, &core_registry()).expect("v1.1 migrates");
    assert_eq!(m.character_id, "shu-lively");
    assert!(m.capabilities.contains_key("visual.gaze"));
    assert_eq!(
        m.capabilities["visual.gaze"].reduced_motion_behavior,
        Some(ReducedMotionBehavior::Disabled)
    );
    assert!(m.channels.iter().any(|c| c == "gaze"));
    assert!(m.intents.iter().any(|i| i == "failed"));
    assert_eq!(m.extra["x-legacy"]["hasAnchors"], true);
    assert_eq!(m.extra["x-legacy"]["schemaVersion"], "1.1");
    validate(&m).expect("validates");
}

/// `character-rig` 2.0（某個角色專屬的 rig pack）不在核心：核心 registry 沒有它的
/// migrator，遷移必須誠實失敗，由該角色自己的 crate 註冊（見 `interaction-character-shu`）。
#[test]
fn rig_packs_are_not_migrated_by_the_core_registry() {
    let err = migrate_pack_to_manifest(&read_pack("shu-maid-dusk"), &core_registry())
        .expect_err("core has no rig migrator");
    assert_eq!(err.code, ManifestErrorCode::Legacy);
}

#[test]
fn legacy_unknown_formats_are_refused() {
    let err = migrate_pack_to_manifest(
        &serde_json::json!({"kind": "persona-pack", "schemaVersion": "1.0", "id": "x"}),
        &core_registry(),
    )
    .expect_err("persona packs are not character manifests");
    assert_eq!(err.code, ManifestErrorCode::Legacy);
    let err = migrate_pack_to_manifest(
        &serde_json::json!({"kind": "character-pack", "schemaVersion": "3.0", "id": "x"}),
        &core_registry(),
    )
    .expect_err("unknown version");
    assert_eq!(err.code, ManifestErrorCode::Legacy);
    let err = migrate_pack_to_manifest(
        &serde_json::json!({"kind": "character-pack", "schemaVersion": "1.0",
            "id": "x", "name": {"en": "x"}, "sheet": "../../../etc/passwd", "animations": {"idle": {}}}),
        &core_registry(),
    )
    .expect_err("traversal in legacy sheet");
    assert_eq!(err.code, ManifestErrorCode::AssetPath);
    assert!(!err.message.contains("passwd"));
}

/// `fallbacks.intents` 不得把安全 intent 映射到非安全 intent：驗證階段就要擋，
/// 而不是讓協商／呈現層去改寫安全語意。
#[test]
fn safety_intent_fallbacks_must_target_another_safety_intent() {
    for (from, to) in [
        ("request-consent", "greet"),
        ("blocked", "play"),
        ("emergency", "sleep"),
        ("offline", "idle"),
        ("wait", "think"),
    ] {
        let mut m = shu_manifest();
        m.fallbacks.intents.insert(from.into(), to.into());
        assert_eq!(
            err_code(validate(&m)),
            ManifestErrorCode::Fallbacks,
            "{from} → {to} 必須被拒絕"
        );
    }
    // 安全 → 安全（規格範例 failed → blocked）與非安全 → 非安全都仍然合法。
    for (from, to) in [
        ("failed", "blocked"),
        ("request-consent", "ask"),
        ("play", "notice"),
        ("sleep", "rest"),
    ] {
        let mut m = shu_manifest();
        m.fallbacks.intents.insert(from.into(), to.into());
        validate(&m).unwrap_or_else(|e| panic!("{from} → {to} 應該合法，卻被拒：{e}"));
    }
}
