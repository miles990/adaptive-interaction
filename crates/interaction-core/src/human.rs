//! Human-readable capability layer: optional, backward-compatible metadata
//! adapters can attach to manifests so UIs and CLIs can explain capabilities
//! to non-technical people.
//!
//! Hard rule: nothing in this module is a safety truth source. The formal
//! manifest fields (`risk_class`, `external_side_effect`, `requires_consent`,
//! `sensitivity`, …) and the Rust policy governor always win. When information
//! is missing here, consumers must degrade conservatively ("unknown"), never
//! optimistically.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Yes / no / unknown. Serialized as `true` / `false` / `"unknown"`.
///
/// The default is [`TriState::Unknown`]: absence of a declaration is never
/// treated as "no risk".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriState {
    Yes,
    No,
    #[default]
    Unknown,
}

impl TriState {
    pub fn from_bool(b: bool) -> Self {
        if b {
            TriState::Yes
        } else {
            TriState::No
        }
    }

    /// True when the value is affirmatively known to be `No`.
    /// `Unknown` is NOT a no — callers must treat it conservatively.
    pub fn is_known_no(self) -> bool {
        self == TriState::No
    }

    pub fn is_known_yes(self) -> bool {
        self == TriState::Yes
    }
}

impl Serialize for TriState {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            TriState::Yes => s.serialize_bool(true),
            TriState::No => s.serialize_bool(false),
            TriState::Unknown => s.serialize_str("unknown"),
        }
    }
}

impl<'de> Deserialize<'de> for TriState {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Bool(true) => Ok(TriState::Yes),
            Raw::Bool(false) => Ok(TriState::No),
            Raw::Text(t) if t == "unknown" => Ok(TriState::Unknown),
            Raw::Text(other) => Err(serde::de::Error::custom(format!(
                "expected true, false or \"unknown\", got {other:?}"
            ))),
        }
    }
}

impl schemars::JsonSchema for TriState {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TriState".into()
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "anyOf": [
                { "type": "boolean" },
                { "type": "string", "const": "unknown" }
            ]
        })
    }
}

/// Localized text keyed by BCP-47 tag (`zh-TW`, `en`, …).
///
/// Lookup falls back: exact tag → same primary language → `en` → any entry.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct LocalizedText(pub BTreeMap<String, String>);

impl LocalizedText {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, locale: impl Into<String>, text: impl Into<String>) -> Self {
        self.0.insert(locale.into(), text.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Resolve for a locale with deterministic fallback.
    pub fn get(&self, locale: &str) -> Option<&str> {
        self.get_strict(locale).or_else(|| self.get_fallback())
    }

    /// Resolve ONLY when this text actually has the requested language
    /// (exact tag or same primary language). No `en`/any fallback — used for
    /// field-level language-tier resolution so that a source that only has
    /// English can never shadow another source that has the user's language.
    pub fn get_strict(&self, locale: &str) -> Option<&str> {
        if let Some(t) = self.0.get(locale) {
            return Some(t);
        }
        let primary = locale.split(['-', '_']).next().unwrap_or(locale);
        self.0
            .iter()
            .find(|(k, _)| k.split(['-', '_']).next().unwrap_or(k) == primary)
            .map(|(_, t)| t.as_str())
    }

    /// The fallback tier: `en`, else any entry (deterministic: BTreeMap order).
    pub fn get_fallback(&self) -> Option<&str> {
        if let Some(t) = self.0.get("en") {
            return Some(t);
        }
        self.0.values().next().map(String::as_str)
    }
}

/// Structured human description an adapter attaches to a capability.
/// Purely presentational: names, examples, setup help. Never safety facts.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HumanPresentation {
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub name: LocalizedText,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub short_description: LocalizedText,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub long_description: LocalizedText,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<LocalizedText>,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub setup_instructions: LocalizedText,
    /// Icon name hint from the shared icon set (e.g. `bell`, `clock`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_hint: Option<String>,
    /// Presentation category (e.g. `notification`, `time`); may differ from
    /// the technical manifest category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Suggested for first-time users in onboarding.
    #[serde(default)]
    pub beginner_recommended: bool,
}

impl HumanPresentation {
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
            && self.short_description.is_empty()
            && self.long_description.is_empty()
            && self.examples.is_empty()
            && self.setup_instructions.is_empty()
            && self.icon_hint.is_none()
            && self.category.is_none()
            && !self.beginner_recommended
    }
}

/// Sensitivity of the data a receptor produces, in human terms.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DataSensitivityLevel {
    None,
    Low,
    Medium,
    High,
    #[default]
    Unknown,
}

/// Where observed data originates.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DataSource {
    Local,
    Device,
    ExternalService,
    #[default]
    Unknown,
}

/// How long observed data is retained.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DataRetention {
    None,
    Session,
    Persistent,
    #[default]
    Unknown,
}

/// What a receptor's data means for the person: categories, whether it is
/// personal, whether it leaves this machine.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataSemantics {
    /// Human-meaningful categories, e.g. `["task-status", "presence"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_categories: Vec<String>,
    #[serde(default)]
    pub personal_data: TriState,
    #[serde(default)]
    pub sensitivity: DataSensitivityLevel,
    #[serde(default)]
    pub source: DataSource,
    #[serde(default)]
    pub leaves_device: TriState,
    #[serde(default)]
    pub retention: DataRetention,
    /// Fact keys that are direct observations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fact_fields: Vec<String>,
    /// Fields that are model inferences (must be shown as guesses).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inference_fields: Vec<String>,
}

/// How disruptive an actuator's output is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Interruptiveness {
    None,
    Low,
    Medium,
    High,
    #[default]
    Unknown,
}

/// The strongest delivery level an actuator can actually confirm.
/// `queued` NEVER implies `completed`; UIs must not display beyond this level.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ConfirmationLevel {
    Requested,
    Queued,
    Acknowledged,
    Delivered,
    Completed,
    Verified,
    #[default]
    Unknown,
}

/// What an actuator or write-operation affects.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EffectSemantics {
    /// Human-meaningful targets, e.g. `["screen", "external-calendar"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects: Vec<String>,
    #[serde(default)]
    pub external_side_effect: TriState,
    #[serde(default)]
    pub physical_effect: TriState,
    #[serde(default)]
    pub interruptiveness: Interruptiveness,
    #[serde(default)]
    pub reversible: TriState,
    /// The deepest confirmation the driver can honestly provide.
    #[serde(default)]
    pub confirmation_level: ConfirmationLevel,
}

/// When a granted consent expires.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ConsentExpiry {
    /// One action only.
    Action,
    #[default]
    Session,
    Duration,
    Persistent,
}

/// Consent guidance in human terms. The *requirement* itself stays in the
/// formal manifest (`requires_consent`) and the governor.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsentSemantics {
    #[serde(default)]
    pub required: TriState,
    /// Why consent is needed, in plain language.
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub reason: LocalizedText,
    /// Suggested consent scope string, e.g. `channel:haptic`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_scope: Option<String>,
    #[serde(default)]
    pub expires: ConsentExpiry,
}

/// The optional human layer bundle carried by manifests.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HumanMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<HumanPresentation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<DataSemantics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectSemantics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<ConsentSemantics>,
}

impl HumanMeta {
    pub fn is_empty(&self) -> bool {
        self.presentation.is_none()
            && self.data.is_none()
            && self.effect.is_none()
            && self.consent.is_none()
    }
}

/// Where a resolved display string came from. Ordered by trust for display
/// (not safety): user override > adapter > catalog > fallback > ai-assisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PresentationSource {
    User,
    Adapter,
    Catalog,
    Fallback,
    AiAssisted,
    /// Nothing usable existed; consumers must show the conservative
    /// "provider has not described this capability" message.
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tristate_serializes_as_bool_or_unknown() {
        assert_eq!(serde_json::to_string(&TriState::Yes).unwrap(), "true");
        assert_eq!(serde_json::to_string(&TriState::No).unwrap(), "false");
        assert_eq!(
            serde_json::to_string(&TriState::Unknown).unwrap(),
            "\"unknown\""
        );
        assert_eq!(
            serde_json::from_str::<TriState>("true").unwrap(),
            TriState::Yes
        );
        assert_eq!(
            serde_json::from_str::<TriState>("\"unknown\"").unwrap(),
            TriState::Unknown
        );
        assert!(serde_json::from_str::<TriState>("\"yes\"").is_err());
    }

    #[test]
    fn tristate_default_is_unknown_not_no() {
        // Absence of a declaration must never read as "safe".
        let d: DataSemantics = serde_json::from_str("{}").unwrap();
        assert_eq!(d.personal_data, TriState::Unknown);
        assert_eq!(d.leaves_device, TriState::Unknown);
        assert!(!d.personal_data.is_known_no());
        let e: EffectSemantics = serde_json::from_str("{}").unwrap();
        assert_eq!(e.physical_effect, TriState::Unknown);
        assert_eq!(e.confirmation_level, ConfirmationLevel::Unknown);
    }

    #[test]
    fn localized_text_fallback_chain() {
        let t = LocalizedText::new()
            .with("zh-TW", "桌面通知")
            .with("en", "Desktop notification");
        assert_eq!(t.get("zh-TW"), Some("桌面通知"));
        // Primary-language match.
        assert_eq!(t.get("zh"), Some("桌面通知"));
        assert_eq!(t.get("zh-CN"), Some("桌面通知"));
        // Unknown locale falls back to en.
        assert_eq!(t.get("fr"), Some("Desktop notification"));
        // en-only falls back to any.
        let only = LocalizedText::new().with("ja", "通知");
        assert_eq!(only.get("de"), Some("通知"));
        assert_eq!(LocalizedText::new().get("en"), None);
    }

    #[test]
    fn human_meta_roundtrips_yaml_and_json() {
        let meta = HumanMeta {
            presentation: Some(HumanPresentation {
                name: LocalizedText::new().with("zh-TW", "桌面通知"),
                beginner_recommended: true,
                ..Default::default()
            }),
            effect: Some(EffectSemantics {
                interruptiveness: Interruptiveness::Medium,
                confirmation_level: ConfirmationLevel::Delivered,
                external_side_effect: TriState::No,
                ..Default::default()
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: HumanMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, back);
        let yaml = serde_yaml::to_string(&meta).unwrap();
        let back: HumanMeta = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(meta, back);
    }
}
