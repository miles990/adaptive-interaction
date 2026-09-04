//! 小樞 rig pack：variants、能力集、`character-rig` 2.0 遷移，以及註冊進核心 registry。
//!
//! 這些檢查原本在 `interaction-character` 的核心測試裡（`legacy_rig_2_0_migrates_to_shu_rig_manifest`）；
//! v0.6.0 strangler 之後核心不再認識小樞，同語意的驗收搬到這個 crate。

use interaction_character::*;
use interaction_character_shu::{ShuRigPack, RIG_PACK_KIND, RIG_PACK_SCHEMA_VERSION};
use std::path::PathBuf;

fn packs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/interaction-desktop/public/packs")
}

fn read_pack(name: &str) -> serde_json::Value {
    let path = packs_dir().join(name).join("manifest.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// host 注入的白名單必須含小樞的 entrypoint id 才驗得過（核心預設是空的）。
fn host_limits() -> ValidationLimits {
    ValidationLimits {
        builtin_whitelist: vec![ShuRigPack::ENTRYPOINT_ID.to_string()],
        ..ValidationLimits::default()
    }
}

fn validate(manifest: &CharacterManifest) -> Result<ManifestReport, ManifestError> {
    let bytes = serde_json::to_vec(manifest).unwrap_or_default().len();
    validate_manifest(bytes, manifest, &host_limits())
}

/// host registry：核心的通用 sprite ＋ 小樞的 rig migrator。
fn host_registry() -> MigrationRegistry {
    let mut registry = MigrationRegistry::with_core_migrators();
    registry
        .register(ShuRigPack::migrator())
        .expect("rig migrator registers");
    registry
}

#[test]
fn entrypoint_id_and_variants_are_stable() {
    assert_eq!(ShuRigPack::ENTRYPOINT_ID, "shu-rig");
    assert_eq!(RIG_PACK_KIND, "character-rig");
    assert_eq!(RIG_PACK_SCHEMA_VERSION, "2.0");
    assert_eq!(
        ShuRigPack::VARIANTS
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<Vec<_>>(),
        vec!["maid-classic", "maid-dusk", "maid-sakura"]
    );
    assert!(ShuRigPack::is_variant("maid-dusk"));
    assert!(!ShuRigPack::is_variant("maid-unknown"));
    assert_eq!(ShuRigPack::DEFAULT_VARIANT, "maid-classic");
}

#[test]
fn capabilities_cover_the_full_rig_surface() {
    let caps = ShuRigPack::capabilities();
    for cap in [
        "visual.presence",
        "visual.pose",
        "visual.expression",
        "visual.gaze",
        "visual.locomotion",
        "visual.overlay",
        "visual.particles",
        "visual.prop",
        "visual.textBubble",
        "audio.speech",
        "audio.effect",
        "multiCharacter",
        "scene",
        "rollCall",
        "gameplay.toys",
        "gameplay.autonomy",
    ] {
        assert!(caps.contains_key(cap), "{cap}");
    }
    assert_eq!(caps.len(), 16);
    // expression 宣告 20 個 intent 變體；audio 需要音訊權限。
    assert_eq!(caps["visual.expression"].variants.len(), 20);
    assert!(caps["audio.speech"].requires_audio);
    let inputs = ShuRigPack::input_capabilities();
    assert_eq!(inputs.len(), 7);
    assert!(inputs.contains_key("input.click"));
}

/// 與核心舊測試 `legacy_rig_2_0_migrates_to_shu_rig_manifest` 同語意。
#[test]
fn legacy_rig_2_0_migrates_to_shu_rig_manifest() {
    let pack = read_pack("shu-maid-dusk");
    let m = migrate_pack_to_manifest(&pack, &host_registry()).expect("rig migrates");
    assert_eq!(m.character_id, "shu-maid-dusk");
    assert_eq!(
        m.entrypoint,
        Entrypoint::Builtin {
            id: "shu-rig".into()
        }
    );
    assert_eq!(
        m.variants.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
        vec!["maid-classic", "maid-dusk", "maid-sakura"]
    );
    assert_eq!(m.intents.len(), 20);
    for cap in [
        "visual.presence",
        "visual.pose",
        "visual.expression",
        "visual.gaze",
        "visual.locomotion",
        "visual.overlay",
        "visual.particles",
        "visual.prop",
        "visual.textBubble",
        "audio.speech",
        "audio.effect",
        "multiCharacter",
        "scene",
        "rollCall",
        "gameplay.toys",
        "gameplay.autonomy",
    ] {
        assert!(m.capabilities.contains_key(cap), "{cap}");
    }
    assert_eq!(m.input_capabilities.len(), 7);
    assert_eq!(m.channels.len(), 12);
    let prefs = m.preferences_schema.as_ref().expect("preferences");
    assert_eq!(
        prefs.properties["palette"].default,
        Some(serde_json::json!("maid-dusk"))
    );
    assert_eq!(m.extra["x-legacy"]["palette"], "maid-dusk");
    assert!(m.security_requirements.audio_output);
    assert!(!m.security_requirements.executable);
    validate(&m).expect("validates");
    // 三個 palette 都能遷移。
    for pack in ["shu-maid", "shu-maid-sakura"] {
        migrate_pack_to_manifest(&read_pack(pack), &host_registry())
            .unwrap_or_else(|e| panic!("{pack}: {e}"));
    }
}

#[test]
fn unknown_palette_is_refused_without_echoing_input() {
    let err = ShuRigPack::migrate(&serde_json::json!({
        "kind": "character-rig", "schemaVersion": "2.0",
        "id": "x", "name": {"en": "x"}, "palette": "maid-unknown"
    }))
    .expect_err("unknown palette");
    assert_eq!(err.code, ManifestErrorCode::Legacy);
    assert!(!err.message.contains("maid-unknown"));
}

#[test]
fn rig_pack_without_id_or_name_is_refused() {
    assert!(ShuRigPack::migrate(&serde_json::json!({"name": {"en": "x"}})).is_err());
    assert!(ShuRigPack::migrate(&serde_json::json!({"id": "x"})).is_err());
}

/// migrator 只認 `character-rig` 2.0；註冊後不影響核心的 sprite 路徑。
#[test]
fn migrator_registers_alongside_the_core_sprite_migrator() {
    let registry = host_registry();
    assert!(registry
        .supported_kinds()
        .contains(&("character-rig".to_string(), "2.0".to_string())));
    assert!(registry
        .supported_kinds()
        .contains(&("character-pack".to_string(), "1.0".to_string())));
    let sprite = migrate_pack_to_manifest(&read_pack("shu-standard"), &registry)
        .expect("sprite still works");
    assert_eq!(
        sprite.entrypoint,
        Entrypoint::Builtin {
            id: "sprite".into()
        }
    );
}
