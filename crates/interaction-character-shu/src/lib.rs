//! interaction-character-shu：小樞（`shu-rig`）專屬的 Character Pack 資料與舊 pack 遷移。
//!
//! v0.6.0 strangler（`docs/aip/architecture-boundaries.md` §4）：Character Presentation Protocol
//! 的核心 crate（`interaction-character`）**不再認識任何具名角色**。小樞的 entrypoint id、
//! rig variants、能力集與 `character-rig` 2.0 遷移全部住在這裡；host（Runtime／Tauri／桌面前端）
//! 把 [`ShuRigPack::ENTRYPOINT_ID`] 放進 builtin 白名單、把 [`ShuRigPack::migrator`] 註冊進
//! [`MigrationRegistry`]，核心才會認得它。
//!
//! 這個 crate 是純資料＋純函式：不讀檔、不執行 entrypoint、不碰網路，時間由呼叫端提供。
//! 小樞只是 Reference Adapter：這裡的內容不是協定的一部分，換掉它不影響任何其他角色。

use interaction_character::{
    legacy_base_manifest, legacy_json_str, legacy_migration_error, CapabilityDecl, CharacterIntent,
    CharacterManifest, ManifestError, PackMigrator, PreferenceProperty, PreferencesSchema,
    ReducedMotionBehavior, VariantDecl, CANONICAL_CHANNELS,
};
use std::collections::BTreeMap;

/// 小樞舊 pack 的 `kind`。
pub const RIG_PACK_KIND: &str = "character-rig";
/// 小樞舊 pack 支援的 `schemaVersion`。
pub const RIG_PACK_SCHEMA_VERSION: &str = "2.0";

/// 小樞 rig pack：entrypoint id、配色 variants、能力集與遷移。
///
/// 純命名空間（沒有狀態）：host 只需要 [`ShuRigPack::ENTRYPOINT_ID`] 與
/// [`ShuRigPack::migrator`]。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShuRigPack;

impl ShuRigPack {
    /// in-process builtin adapter id（host 白名單用）。
    pub const ENTRYPOINT_ID: &'static str = "shu-rig";

    /// rig 2.0 的三個 palette（固定順序：id、zh-TW、en）。
    pub const VARIANTS: [(&'static str, &'static str, &'static str); 3] = [
        ("maid-classic", "經典", "Classic"),
        ("maid-dusk", "暮色", "Dusk"),
        ("maid-sakura", "櫻", "Sakura"),
    ];

    /// 舊 pack 沒宣告 palette 時的預設值。
    pub const DEFAULT_VARIANT: &'static str = "maid-classic";

    /// `value` 是不是已知 palette（未知名稱不猜、不接受）。
    pub fn is_variant(value: &str) -> bool {
        Self::VARIANTS.iter().any(|(id, _, _)| *id == value)
    }

    /// CPP §12 `shu-rig` 完整能力集。
    pub fn capabilities() -> BTreeMap<String, CapabilityDecl> {
        let mut caps = BTreeMap::new();
        let all_intents: Vec<String> = CharacterIntent::ALL
            .iter()
            .map(|i| i.as_str().to_string())
            .collect();
        caps.insert(
            "visual.presence".into(),
            CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Static),
        );
        for cap in [
            "visual.pose",
            "visual.expression",
            "visual.gaze",
            "visual.locomotion",
            "visual.overlay",
            "visual.prop",
            "visual.textBubble",
        ] {
            let mut decl =
                CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Reduced);
            if cap == "visual.expression" {
                decl = decl.with_variants(all_intents.iter().cloned());
            }
            caps.insert(cap.into(), decl);
        }
        caps.insert(
            "visual.particles".into(),
            CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Disabled),
        );
        let mut speech = CapabilityDecl::supported();
        speech.requires_audio = true;
        caps.insert("audio.speech".into(), speech);
        let mut effect = CapabilityDecl::supported();
        effect.requires_audio = true;
        caps.insert("audio.effect".into(), effect);
        for cap in ["multiCharacter", "scene", "rollCall"] {
            caps.insert(cap.into(), CapabilityDecl::supported());
        }
        caps.insert(
            "gameplay.toys".into(),
            CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Reduced),
        );
        caps.insert(
            "gameplay.autonomy".into(),
            CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Disabled),
        );
        caps
    }

    /// CPP §12 `shu-rig` 輸入能力集。
    pub fn input_capabilities() -> BTreeMap<String, CapabilityDecl> {
        [
            "input.click",
            "input.hover",
            "input.drag",
            "input.drop",
            "input.pointerProximity",
            "input.text",
            "input.fileDrop",
        ]
        .iter()
        .map(|id| ((*id).to_string(), CapabilityDecl::supported()))
        .collect()
    }

    /// `character-rig` 2.0 → CPP manifest。不改寫使用者設定、不讀檔。
    pub fn migrate(json: &serde_json::Value) -> Result<CharacterManifest, ManifestError> {
        let mut manifest = legacy_base_manifest(json, Self::ENTRYPOINT_ID)?;
        let palette =
            legacy_json_str(json, "palette").unwrap_or_else(|| Self::DEFAULT_VARIANT.to_string());
        if !Self::is_variant(&palette) {
            return Err(legacy_migration_error(
                "character-rig palette is not one of the known palettes",
            ));
        }
        manifest.capabilities = Self::capabilities();
        manifest.input_capabilities = Self::input_capabilities();
        manifest.channels = CANONICAL_CHANNELS.iter().map(|s| s.to_string()).collect();
        manifest.intents = CharacterIntent::ALL
            .iter()
            .map(|i| i.as_str().to_string())
            .collect();
        manifest.variants = Self::VARIANTS
            .iter()
            .map(|(id, zh, en)| VariantDecl {
                id: (*id).to_string(),
                display_name: [
                    ("zh-TW".to_string(), (*zh).to_string()),
                    ("en".to_string(), (*en).to_string()),
                ]
                .into_iter()
                .collect(),
            })
            .collect();
        manifest.security_requirements.audio_output = true;
        manifest.preferences_schema = Some(PreferencesSchema {
            schema_type: "object".into(),
            properties: [(
                "palette".to_string(),
                PreferenceProperty {
                    property_type: "string".into(),
                    enum_values: Some(
                        Self::VARIANTS
                            .iter()
                            .map(|(id, _, _)| (*id).to_string())
                            .collect(),
                    ),
                    default: Some(serde_json::Value::String(palette.clone())),
                    ..PreferenceProperty::default()
                },
            )]
            .into_iter()
            .collect(),
            required: Vec::new(),
            extra: BTreeMap::new(),
        });
        manifest
            .fallbacks
            .capabilities
            .insert("visual.particles".into(), vec!["visual.expression".into()]);
        manifest.fallbacks.capabilities.insert(
            "visual.locomotion".into(),
            vec!["visual.pose".into(), "visual.presence".into()],
        );
        manifest
            .fallbacks
            .capabilities
            .insert("gameplay.toys".into(), vec!["visual.pose".into()]);
        manifest.extra.insert(
            "x-legacy".into(),
            serde_json::json!({
                "kind": RIG_PACK_KIND,
                "schemaVersion": RIG_PACK_SCHEMA_VERSION,
                "palette": palette,
            }),
        );
        Ok(manifest)
    }

    /// 給 host 註冊進 [`MigrationRegistry`] 的 migrator。
    pub fn migrator() -> Box<dyn PackMigrator> {
        Box::new(RigPackMigrator)
    }
}

/// `character-rig` 2.0 的 [`PackMigrator`] 實作。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RigPackMigrator;

impl PackMigrator for RigPackMigrator {
    fn kind(&self) -> &str {
        RIG_PACK_KIND
    }

    fn schema_versions(&self) -> &[&str] {
        &[RIG_PACK_SCHEMA_VERSION]
    }

    fn migrate(&self, json: &serde_json::Value) -> Result<CharacterManifest, ManifestError> {
        ShuRigPack::migrate(json)
    }
}
