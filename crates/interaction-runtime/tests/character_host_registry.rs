//! Runtime 端的 host 注入：builtin adapter 白名單與舊 pack migrator registry。
//!
//! CPP 核心（`interaction-character`）v0.6.0 起不認識任何具名角色：白名單預設是空的、
//! 遷移要由 host 註冊 migrator。這裡釘住桌面 Runtime 實際注入了什麼。

use interaction_character::{
    migrate_pack_to_manifest, validate_manifest, CharacterManifest, Entrypoint, ManifestErrorCode,
    ValidationLimits,
};
use interaction_character_shu::ShuRigPack;
use interaction_runtime::character::{character_host_registry, CHARACTER_BUILTIN_ENTRYPOINTS};

fn manifest_with_builtin(id: &str) -> CharacterManifest {
    interaction_character::minimal_manifest("host-check", id)
}

#[test]
fn host_whitelist_covers_every_builtin_adapter_the_desktop_ships() {
    assert_eq!(
        CHARACTER_BUILTIN_ENTRYPOINTS.to_vec(),
        vec![ShuRigPack::ENTRYPOINT_ID, "shape", "sprite", "text"]
    );
    let limits = character_host_registry().validation_limits();
    assert_eq!(limits.builtin_whitelist.len(), 4);
    for id in CHARACTER_BUILTIN_ENTRYPOINTS {
        let manifest = manifest_with_builtin(id);
        let bytes = serde_json::to_vec(&manifest).unwrap_or_default().len();
        validate_manifest(bytes, &manifest, &limits).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

#[test]
fn builtin_ids_outside_the_host_registry_are_rejected() {
    let limits = character_host_registry().validation_limits();
    let manifest = manifest_with_builtin("evil");
    let bytes = serde_json::to_vec(&manifest).unwrap_or_default().len();
    let err = validate_manifest(bytes, &manifest, &limits).expect_err("not a host adapter");
    assert_eq!(err.code, ManifestErrorCode::Entrypoint);
}

/// 核心預設仍然是空白名單：host 沒注入就沒有任何 builtin 角色（不得靠核心的預設值過關）。
#[test]
fn the_core_default_still_grants_nothing() {
    let manifest = manifest_with_builtin(ShuRigPack::ENTRYPOINT_ID);
    let bytes = serde_json::to_vec(&manifest).unwrap_or_default().len();
    assert!(validate_manifest(bytes, &manifest, &ValidationLimits::default()).is_err());
}

#[test]
fn host_migration_registry_carries_sprite_and_the_shu_rig_migrator() {
    let registry = character_host_registry();
    let kinds = registry.migrations().supported_kinds();
    assert!(kinds.contains(&("character-pack".to_string(), "1.0".to_string())));
    assert!(kinds.contains(&("character-pack".to_string(), "1.1".to_string())));
    assert!(kinds.contains(&("character-rig".to_string(), "2.0".to_string())));

    let rig = migrate_pack_to_manifest(
        &serde_json::json!({
            "kind": "character-rig", "schemaVersion": "2.0",
            "id": "host-rig", "name": {"zh-TW": "測試"}, "palette": "maid-dusk"
        }),
        registry.migrations(),
    )
    .expect("host registry migrates rig packs");
    assert_eq!(
        rig.entrypoint,
        Entrypoint::Builtin {
            id: ShuRigPack::ENTRYPOINT_ID.into()
        }
    );
}
