//! §2.1／§2.2：host 注入 builtin 白名單、`PackMigrator` registry 依 pack kind 分派。
//!
//! v0.6.0 strangler：核心不再認識任何具名角色（小樞）。
//! - `ValidationLimits::default().builtin_whitelist` 是**空**的：host 必須注入自己的白名單。
//! - 舊 pack 遷移改由 `MigrationRegistry` 分派；核心只內建通用 sprite（`character-pack` 1.0／1.1）。

mod common;

use common::*;
use interaction_character::*;

/// 核心不預設任何 builtin id：預設白名單是空的，host 沒注入就沒有 builtin 角色可用。
#[test]
fn default_validation_limits_carry_an_empty_builtin_whitelist() {
    assert!(ValidationLimits::default().builtin_whitelist.is_empty());
}

/// 沒注入白名單 → builtin entrypoint 一律被拒（錯誤訊息不回顯輸入內容、不含路徑）。
#[test]
fn builtin_entrypoint_is_rejected_when_the_host_injects_nothing() {
    let manifest = reference_manifest();
    let bytes = serde_json::to_vec(&manifest).expect("serialize").len();
    let err = validate_manifest(bytes, &manifest, &ValidationLimits::default())
        .expect_err("no whitelist means no builtin");
    assert_eq!(err.code, ManifestErrorCode::Entrypoint);
    assert_eq!(err.path, "entrypoint.id");
    assert!(!err.message.contains('/'), "message must not echo paths");
}

/// host 注入白名單後同一份 manifest 就通過；白名單以外的 id 仍被拒。
#[test]
fn injected_whitelist_decides_which_builtin_ids_pass() {
    let manifest = reference_manifest();
    let entrypoint_id = match &manifest.entrypoint {
        Entrypoint::Builtin { id } => id.clone(),
        other => panic!("fixture must use a builtin entrypoint, got {other:?}"),
    };
    validate_with_whitelist(&manifest, &[entrypoint_id.as_str()]).expect("host injected the id");
    let err = validate_with_whitelist(&manifest, &["sprite"]).expect_err("id not in whitelist");
    assert_eq!(err.code, ManifestErrorCode::Entrypoint);
}

/// 核心 registry 只認識通用 sprite pack；具名角色的 rig pack 不在核心。
#[test]
fn core_registry_knows_sprite_only() {
    let registry = MigrationRegistry::with_core_migrators();
    assert_eq!(
        registry.supported_kinds(),
        vec![
            ("character-pack".to_string(), "1.0".to_string()),
            ("character-pack".to_string(), "1.1".to_string()),
        ]
    );
    let sprite = migrate_pack_to_manifest(&read_pack("shu-standard"), &registry)
        .expect("sprite pack migrates in the core");
    assert_eq!(
        sprite.entrypoint,
        Entrypoint::Builtin {
            id: "sprite".into()
        }
    );
    let err = migrate_pack_to_manifest(
        &serde_json::json!({"kind": "character-rig", "schemaVersion": "2.0", "id": "x"}),
        &registry,
    )
    .expect_err("rig packs need a host-registered migrator");
    assert_eq!(err.code, ManifestErrorCode::Legacy);
}

/// 空 registry 不會遷移任何東西（誠實拒絕，不猜）。
#[test]
fn empty_registry_migrates_nothing() {
    let err = migrate_pack_to_manifest(&read_pack("shu-standard"), &MigrationRegistry::new())
        .expect_err("empty registry");
    assert_eq!(err.code, ManifestErrorCode::Legacy);
}

/// 第三方 migrator 可註冊，並依 (kind, schemaVersion) 分派。
#[test]
fn registry_dispatches_by_kind_and_schema_version() {
    let mut registry = MigrationRegistry::with_core_migrators();
    registry
        .register(Box::new(DemoMigrator))
        .expect("register demo");
    let manifest = migrate_pack_to_manifest(
        &serde_json::json!({
            "kind": "demo-pack", "schemaVersion": "9.0",
            "id": "demo-one", "name": {"en": "Demo"}
        }),
        &registry,
    )
    .expect("demo migrates");
    assert_eq!(manifest.character_id, "demo-one");
    assert_eq!(
        manifest.entrypoint,
        Entrypoint::Builtin { id: "demo".into() }
    );
    // 核心 sprite 仍然可用（註冊不會蓋掉核心）。
    migrate_pack_to_manifest(&read_pack("shu-standard"), &registry).expect("sprite still works");
}

/// 同一個 (kind, version) 不得註冊兩次：後註冊者不得悄悄覆蓋前者。
#[test]
fn registry_refuses_duplicate_kind_and_version() {
    let mut registry = MigrationRegistry::with_core_migrators();
    registry.register(Box::new(DemoMigrator)).expect("first");
    let err = registry
        .register(Box::new(DemoMigrator))
        .expect_err("duplicate");
    assert_eq!(err.code, ManifestErrorCode::Legacy);
    // 註冊失敗不得留下半套狀態：仍是 sprite ＋ demo 兩個 migrator、三組 (kind, version)。
    assert_eq!(registry.len(), 2);
    assert_eq!(registry.supported_kinds().len(), 3);
}

/// registry 有界：超過 `MAX_MIGRATORS` 一律拒絕（不是無界集合）。
#[test]
fn registry_is_bounded() {
    let mut registry = MigrationRegistry::new();
    for i in 0..MAX_MIGRATORS {
        registry
            .register(Box::new(CountedMigrator(i)))
            .unwrap_or_else(|e| panic!("register {i}: {e}"));
    }
    assert_eq!(registry.len(), MAX_MIGRATORS);
    let err = registry
        .register(Box::new(CountedMigrator(MAX_MIGRATORS)))
        .expect_err("bounded");
    assert_eq!(err.code, ManifestErrorCode::Legacy);
}

/// 未知 kind／version 的錯誤訊息有界且不回顯完整輸入。
#[test]
fn unknown_pack_kind_is_refused_without_echoing_long_input() {
    let long_kind = "k".repeat(4_000);
    let err = migrate_pack_to_manifest(
        &serde_json::json!({"kind": long_kind, "schemaVersion": "1.0", "id": "x"}),
        &MigrationRegistry::with_core_migrators(),
    )
    .expect_err("unknown kind");
    assert_eq!(err.code, ManifestErrorCode::Legacy);
    assert!(
        err.message.chars().count() <= 512,
        "message must stay bounded, got {} chars",
        err.message.chars().count()
    );
}

/// migrator 用得到的公開建構 helper：核心不必知道角色細節也能讓第三方組出合法 manifest。
#[test]
fn legacy_helpers_are_public_for_third_party_migrators() {
    let json =
        serde_json::json!({"id": "demo-two", "name": {"en": "Demo Two"}, "version": "2.1.0"});
    let manifest = legacy_base_manifest(&json, "demo").expect("base manifest");
    assert_eq!(manifest.character_id, "demo-two");
    assert_eq!(manifest.version, "2.1.0");
    assert_eq!(legacy_json_str(&json, "id").as_deref(), Some("demo-two"));
    assert_eq!(legacy_json_localized(json.get("name"))["en"], "Demo Two");
    assert_eq!(
        legacy_migration_error("bad pack").code,
        ManifestErrorCode::Legacy
    );
    // id／name 缺一不可。
    assert!(legacy_base_manifest(&serde_json::json!({"name": {"en": "x"}}), "demo").is_err());
    assert!(legacy_base_manifest(&serde_json::json!({"id": "x"}), "demo").is_err());
}

struct DemoMigrator;

impl PackMigrator for DemoMigrator {
    fn kind(&self) -> &str {
        "demo-pack"
    }
    fn schema_versions(&self) -> &[&str] {
        &["9.0"]
    }
    fn migrate(&self, json: &serde_json::Value) -> Result<CharacterManifest, ManifestError> {
        legacy_base_manifest(json, "demo")
    }
}

struct CountedMigrator(usize);

impl PackMigrator for CountedMigrator {
    fn kind(&self) -> &str {
        // kind 相同、version 不同也算不同項目；這裡用固定 kind 搭配不同 version。
        "counted-pack"
    }
    fn schema_versions(&self) -> &[&str] {
        // 借用 'static 表：每個 index 一個唯一版本字串。
        VERSIONS
            .get(self.0)
            .map(std::slice::from_ref)
            .unwrap_or(&[])
    }
    fn migrate(&self, json: &serde_json::Value) -> Result<CharacterManifest, ManifestError> {
        legacy_base_manifest(json, "demo")
    }
}

/// `MAX_MIGRATORS + 1` 個唯一版本字串（測試用固定表）。
static VERSIONS: [&str; 33] = [
    "0.0", "0.1", "0.2", "0.3", "0.4", "0.5", "0.6", "0.7", "0.8", "0.9", "1.0", "1.1", "1.2",
    "1.3", "1.4", "1.5", "1.6", "1.7", "1.8", "1.9", "2.0", "2.1", "2.2", "2.3", "2.4", "2.5",
    "2.6", "2.7", "2.8", "2.9", "3.0", "3.1", "3.2",
];
