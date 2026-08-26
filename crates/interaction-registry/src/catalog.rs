//! Common Capability Catalog: a small, versioned, localizable directory of
//! *concepts* (not brands or devices). It supplies consistent names, icons,
//! categories and typical-usage hints for well-known capabilities.
//!
//! The catalog is NOT a safety truth source. `typical` hints may only inform
//! presentation; they never override an adapter's formal manifest or policy.

use interaction_core::{
    ConfirmationLevel, DataSensitivityLevel, DataSource, Interruptiveness, LocalizedText, TriState,
};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Which registry a capability lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityKind {
    Receptor,
    Actuator,
    ToolOperation,
}

/// Presentation-only hints about how this concept typically behaves.
/// Every field is optional; absence means "no hint".
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypicalSemantics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interruptiveness: Option<Interruptiveness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_effect: Option<TriState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_level: Option<ConfirmationLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_data: Option<TriState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<DataSensitivityLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DataSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaves_device: Option<TriState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub canonical_id: String,
    pub kind: CapabilityKind,
    pub category: String,
    /// Technical-id patterns. Glob: `*` = one segment, `**` = any segments.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Driver-id patterns matched the same way.
    #[serde(default)]
    pub driver_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_role: Option<String>,
    #[serde(default)]
    pub beginner_recommended: bool,
    #[serde(default)]
    pub name: LocalizedText,
    #[serde(default)]
    pub short_description: LocalizedText,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub long_description: LocalizedText,
    #[serde(default)]
    pub typical: TypicalSemantics,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub risk_note: LocalizedText,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema_version: String,
    pub version: u64,
    pub entries: Vec<CatalogEntry>,
}

const BUILTIN_CATALOG: &str = include_str!("../../../schemas/catalog/common-capabilities.yaml");

impl Catalog {
    /// The embedded builtin catalog. Parsed once; a parse failure is a build
    /// defect and is covered by tests, so the fallback is an empty catalog
    /// rather than a panic in production paths.
    pub fn builtin() -> &'static Catalog {
        static CATALOG: OnceLock<Catalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            Catalog::parse(BUILTIN_CATALOG).unwrap_or_else(|e| {
                tracing::error!(error = %e, "builtin capability catalog failed to parse");
                Catalog {
                    schema_version: "1.0".into(),
                    version: 0,
                    entries: Vec::new(),
                }
            })
        })
    }

    pub fn parse(yaml: &str) -> Result<Catalog, String> {
        serde_yaml::from_str(yaml).map_err(|e| e.to_string())
    }

    /// Look up the catalog entry for a technical id (and optionally driver).
    ///
    /// Deterministic order: exact canonical-id / alias match first (file
    /// order), then driver match, then glob alias match (file order).
    pub fn lookup(
        &self,
        kind: CapabilityKind,
        technical_id: &str,
        driver: Option<&str>,
    ) -> Option<&CatalogEntry> {
        let of_kind = || self.entries.iter().filter(move |e| e.kind == kind);
        // Pass 1: exact.
        if let Some(hit) = of_kind()
            .find(|e| e.canonical_id == technical_id || e.aliases.iter().any(|a| a == technical_id))
        {
            return Some(hit);
        }
        // Pass 2: driver exact/glob.
        if let Some(driver) = driver {
            if let Some(hit) = of_kind().find(|e| {
                e.driver_aliases
                    .iter()
                    .any(|a| a == driver || glob_match(a, driver))
            }) {
                return Some(hit);
            }
        }
        // Pass 3: glob aliases.
        of_kind().find(|e| e.aliases.iter().any(|a| glob_match(a, technical_id)))
    }
}

/// Segment-wise glob: `*` matches exactly one segment, `**` matches zero or
/// more segments. Segments are separated by `.`.
pub fn glob_match(pattern: &str, id: &str) -> bool {
    let pat: Vec<&str> = pattern.split('.').collect();
    let seg: Vec<&str> = id.split('.').collect();
    glob_rec(&pat, &seg)
}

fn glob_rec(pat: &[&str], seg: &[&str]) -> bool {
    match pat.first() {
        None => seg.is_empty(),
        Some(&"**") => {
            // `**` swallows zero or more segments.
            (0..=seg.len()).any(|skip| glob_rec(&pat[1..], &seg[skip..]))
        }
        Some(&"*") => !seg.is_empty() && glob_rec(&pat[1..], &seg[1..]),
        Some(literal) => {
            !seg.is_empty()
                && seg[0].eq_ignore_ascii_case(literal)
                && glob_rec(&pat[1..], &seg[1..])
        }
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    #[test]
    fn builtin_catalog_parses_and_is_nonempty() {
        let c = Catalog::builtin();
        assert!(c.version >= 1);
        assert!(
            c.entries.len() >= 30 && c.entries.len() <= 80,
            "catalog should stay small and curated, got {}",
            c.entries.len()
        );
        // Every entry must have zh-TW and en names + short descriptions.
        for e in &c.entries {
            assert!(
                e.name.get("zh-TW").is_some() && e.name.get("en").is_some(),
                "{} missing localized name",
                e.canonical_id
            );
            assert!(
                e.short_description.get("zh-TW").is_some(),
                "{} missing zh-TW shortDescription",
                e.canonical_id
            );
            assert!(e.icon.is_some(), "{} missing icon", e.canonical_id);
        }
        // Canonical ids must be unique.
        let mut ids: Vec<_> = c.entries.iter().map(|e| &e.canonical_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), c.entries.len());
    }

    #[test]
    fn glob_semantics() {
        assert!(glob_match("**.device-status", "dev.device.device-status"));
        assert!(glob_match("**.device-status", "device-status"));
        assert!(glob_match(
            "smart_light.**",
            "smart_light.bedroom.set_brightness"
        ));
        assert!(glob_match("camera.**", "camera.main"));
        assert!(!glob_match("camera.**", "webcamera.main"));
        assert!(glob_match("*.presence", "user.presence"));
        assert!(!glob_match("*.presence", "a.b.presence"));
    }

    #[test]
    fn lookup_priority_exact_then_driver_then_glob() {
        let c = Catalog::builtin();
        // Exact alias.
        let hit = c
            .lookup(CapabilityKind::Actuator, "local-notification", None)
            .expect("desktop notification");
        assert_eq!(hit.canonical_id, "common.notification.desktop");
        // Driver alias beats glob.
        let hit = c
            .lookup(
                CapabilityKind::Actuator,
                "dev.device",
                Some("builtin.mock-actuator"),
            )
            .expect("mock actuator via driver");
        assert_eq!(hit.canonical_id, "common.actuator.mock");
        // Glob: paired device-status receptors.
        let hit = c
            .lookup(CapabilityKind::Receptor, "dev.device.device-status", None)
            .expect("device status via glob");
        assert_eq!(hit.canonical_id, "common.device.status");
        // Unknown id: no entry.
        assert!(c
            .lookup(CapabilityKind::Actuator, "totally.unknown.thing", None)
            .is_none());
    }

    #[test]
    fn camera_is_marked_sensitive_in_catalog() {
        let c = Catalog::builtin();
        let cam = c
            .lookup(CapabilityKind::Receptor, "camera.main", None)
            .expect("camera glob");
        assert_eq!(cam.canonical_id, "common.video.camera");
        assert_eq!(cam.typical.sensitivity, Some(DataSensitivityLevel::High));
        assert!(!cam.risk_note.is_empty());
    }
}
