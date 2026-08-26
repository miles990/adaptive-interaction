//! Human display resolver: turns a technical manifest + catalog + user
//! preference + (optional) AI-assisted text into one deterministic
//! human-readable "card".
//!
//! Resolution priority (per the product spec):
//! - display name: user override → adapter presentation → catalog → fallback
//!   from the technical id
//! - description: adapter → catalog → deterministic schema fallback → AI note
//! - icon/category: adapter → catalog → kind default
//! - safety/impact facts: formal manifest fields merged conservatively with
//!   adapter-declared semantics; catalog hints stay in a separate `typical`
//!   block and can never masquerade as facts; AI text never changes facts.

use crate::catalog::{CapabilityKind, Catalog, TypicalSemantics};
use interaction_core::{
    ActuatorManifest, Availability, ConfirmationLevel, DataRetention, DataSensitivityLevel,
    DataSource, HumanMeta, Interruptiveness, PresentationSource, ReceptorManifest, RiskClass,
    Sensitivity, ToolOperationManifest, TriState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One badge shown on a capability card. `label` is localized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Badge {
    pub key: String,
    pub label: String,
    /// `info` | `ok` | `warn` | `danger`
    pub tone: String,
}

/// Resolved data-flow facts for a receptor-like capability.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedData {
    pub personal_data: TriState,
    pub sensitivity: DataSensitivityLevel,
    pub source: DataSource,
    pub leaves_device: TriState,
    pub retention: DataRetention,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fact_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inference_fields: Vec<String>,
}

/// Resolved impact facts for an actuator-like capability.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedEffect {
    pub external_side_effect: TriState,
    pub physical_effect: TriState,
    pub interruptiveness: Interruptiveness,
    pub reversible: TriState,
    /// Deepest delivery level that can honestly be confirmed.
    pub confirmation_level: ConfirmationLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedConsent {
    pub required: TriState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_scope: Option<String>,
}

/// The deterministic human view of one capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HumanCard {
    pub id: String,
    pub kind: CapabilityKind,
    pub display_name: String,
    pub name_source: PresentationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    pub description_source: PresentationSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_instructions: Option<String>,
    pub icon: String,
    pub color_role: String,
    pub category: String,
    pub beginner_recommended: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_id: Option<String>,
    pub badges: Vec<Badge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ResolvedData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<ResolvedEffect>,
    pub consent: ResolvedConsent,
    /// Catalog hints; presentation only, labelled "typical" in UIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typical: Option<TypicalSemantics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_note: Option<String>,
    /// AI-assisted supplement; never a fact source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_description: Option<String>,
    /// True when neither adapter nor catalog described this capability.
    pub undescribed: bool,
    /// Localized conservative message shown when `undescribed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conservative_notice: Option<String>,
    // Technical passthrough (advanced mode / actions).
    pub availability: Availability,
    pub requires_consent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_class: Option<RiskClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
}

/// Per-request context for resolution.
#[derive(Debug, Clone, Default)]
pub struct ResolveContext<'a> {
    pub locale: &'a str,
    /// User's custom display name (presentation only).
    pub user_name: Option<&'a str>,
    /// AI-assisted description, already validated against the manifest hash.
    pub ai_description: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// Localized fixed strings (deterministic, no AI involved).
// ---------------------------------------------------------------------------

fn is_zh(locale: &str) -> bool {
    locale.split(['-', '_']).next().unwrap_or("") == "zh"
}

fn t(locale: &str, zh: &'static str, en: &'static str) -> String {
    if is_zh(locale) { zh } else { en }.to_string()
}

fn conservative_notice(locale: &str) -> String {
    t(
        locale,
        "提供者尚未提供完整的資料與影響說明。在你確認前，系統不會自動使用這項能力。",
        "The provider has not fully described this capability's data and impact. \
         The system will not use it automatically before you confirm.",
    )
}

// ---------------------------------------------------------------------------
// Conservative merges.
// ---------------------------------------------------------------------------

/// Merge two declarations of a risk-increasing fact (`Yes` = riskier).
/// Any `Yes` wins, else any known `No`, else `Unknown`.
fn merge_risky(a: TriState, b: TriState) -> TriState {
    use TriState::*;
    match (a, b) {
        (Yes, _) | (_, Yes) => Yes,
        (No, _) | (_, No) => No,
        _ => Unknown,
    }
}

/// Merge reversibility (`No` = riskier).
fn merge_reversible(a: TriState, b: TriState) -> TriState {
    use TriState::*;
    match (a, b) {
        (No, _) | (_, No) => No,
        (Yes, _) | (_, Yes) => Yes,
        _ => Unknown,
    }
}

fn sensitivity_to_level(s: Sensitivity) -> DataSensitivityLevel {
    match s {
        Sensitivity::Public => DataSensitivityLevel::None,
        Sensitivity::Internal => DataSensitivityLevel::Low,
        Sensitivity::Personal => DataSensitivityLevel::Medium,
        Sensitivity::Intimate => DataSensitivityLevel::High,
    }
}

fn sensitivity_to_personal(s: Sensitivity) -> TriState {
    match s {
        Sensitivity::Public => TriState::No,
        Sensitivity::Internal => TriState::Unknown,
        Sensitivity::Personal | Sensitivity::Intimate => TriState::Yes,
    }
}

fn max_sensitivity(a: DataSensitivityLevel, b: DataSensitivityLevel) -> DataSensitivityLevel {
    use DataSensitivityLevel::*;
    let rank = |s: DataSensitivityLevel| match s {
        None => 0u8,
        Low => 1,
        Medium => 2,
        Unknown => 3, // unknown is treated as more concerning than "medium"
        High => 4,
    };
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

/// Deterministic physical-effect hint from the formal channel name.
fn channel_physical(channel: &str) -> TriState {
    match channel {
        "haptic" | "light" => TriState::Yes,
        "conversation" | "web-ui" | "notification" | "log" | "visual" | "audio" | "webhook"
        | "desktop-pet" => TriState::No,
        _ => TriState::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Deterministic fallbacks.
// ---------------------------------------------------------------------------

/// `smart_light.bedroom.set_brightness` → `Smart Light · Bedroom · Set Brightness`
pub fn fallback_name_from_id(id: &str) -> String {
    id.split('.')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            segment
                .split(['_', '-'])
                .filter(|w| !w.is_empty())
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Pull a human string out of a JSON Schema (`description` then `title`).
fn schema_text(schema: Option<&Value>) -> Option<String> {
    let schema = schema?;
    for key in ["description", "title"] {
        if let Some(s) = schema.get(key).and_then(Value::as_str) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Core resolution.
// ---------------------------------------------------------------------------

struct Common<'a> {
    id: String,
    kind: CapabilityKind,
    tech_name: &'a str,
    tech_description: &'a str,
    tech_category: String,
    driver: Option<&'a str>,
    schema: Option<&'a Value>,
    human: Option<&'a HumanMeta>,
    availability: Availability,
    requires_consent: bool,
    risk_class: Option<RiskClass>,
    channel: Option<&'a str>,
}

fn resolve_common(c: Common<'_>, catalog: &Catalog, ctx: &ResolveContext<'_>) -> HumanCard {
    let locale = if ctx.locale.is_empty() {
        "zh-TW"
    } else {
        ctx.locale
    };
    let entry = catalog.lookup(c.kind, &c.id, c.driver);
    let presentation = c.human.and_then(|h| h.presentation.as_ref());

    // Display name.
    let (display_name, name_source) =
        if let Some(u) = ctx.user_name.filter(|u| !u.trim().is_empty()) {
            (u.trim().to_string(), PresentationSource::User)
        } else if let Some(n) = presentation.and_then(|p| p.name.get(locale)) {
            (n.to_string(), PresentationSource::Adapter)
        } else if let Some(n) = entry.and_then(|e| e.name.get(locale)) {
            (n.to_string(), PresentationSource::Catalog)
        } else if !c.tech_name.trim().is_empty() && c.tech_name != c.id {
            (c.tech_name.to_string(), PresentationSource::Adapter)
        } else {
            (fallback_name_from_id(&c.id), PresentationSource::Fallback)
        };

    // Short description.
    let (short_description, description_source) =
        if let Some(d) = presentation.and_then(|p| p.short_description.get(locale)) {
            (Some(d.to_string()), PresentationSource::Adapter)
        } else if !c.tech_description.trim().is_empty() {
            (
                Some(c.tech_description.trim().to_string()),
                PresentationSource::Adapter,
            )
        } else if let Some(d) = entry.and_then(|e| e.short_description.get(locale)) {
            (Some(d.to_string()), PresentationSource::Catalog)
        } else if let Some(d) = schema_text(c.schema) {
            (Some(d), PresentationSource::Fallback)
        } else {
            (None, PresentationSource::None)
        };

    let long_description = presentation
        .and_then(|p| p.long_description.get(locale).map(str::to_string))
        .or_else(|| entry.and_then(|e| e.long_description.get(locale).map(str::to_string)));

    let examples = presentation
        .map(|p| {
            p.examples
                .iter()
                .filter_map(|e| e.get(locale).map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let setup_instructions =
        presentation.and_then(|p| p.setup_instructions.get(locale).map(str::to_string));

    // Icon / color role / category.
    let kind_default_icon = match c.kind {
        CapabilityKind::Receptor => "scan-eye",
        CapabilityKind::Actuator => "send",
        CapabilityKind::ToolOperation => "wrench",
    };
    let icon = presentation
        .and_then(|p| p.icon_hint.clone())
        .or_else(|| entry.and_then(|e| e.icon.clone()))
        .unwrap_or_else(|| kind_default_icon.to_string());
    let color_role = entry
        .and_then(|e| e.color_role.clone())
        .unwrap_or_else(|| match c.kind {
            CapabilityKind::Receptor => "input".to_string(),
            _ => "output".to_string(),
        });
    let category = presentation
        .and_then(|p| p.category.clone())
        .or_else(|| entry.map(|e| e.category.clone()))
        .unwrap_or(c.tech_category);

    let beginner_recommended = presentation
        .map(|p| p.beginner_recommended)
        .unwrap_or(false)
        || entry.map(|e| e.beginner_recommended).unwrap_or(false);

    let undescribed = matches!(description_source, PresentationSource::None);

    // Consent: the formal flag always wins; human text supplements.
    let declared_consent = c.human.and_then(|h| h.consent.as_ref());
    let consent = ResolvedConsent {
        required: merge_risky(
            TriState::from_bool(c.requires_consent),
            declared_consent.map(|d| d.required).unwrap_or_default(),
        ),
        reason: declared_consent
            .and_then(|d| d.reason.get(locale).map(str::to_string))
            .or_else(|| entry.and_then(|e| e.risk_note.get(locale).map(str::to_string))),
        suggested_scope: declared_consent.and_then(|d| d.suggested_scope.clone()),
    };

    let risk_note = entry.and_then(|e| e.risk_note.get(locale).map(str::to_string));
    let typical = entry
        .map(|e| e.typical.clone())
        .filter(|t| *t != TypicalSemantics::default());

    HumanCard {
        id: c.id.clone(),
        kind: c.kind,
        display_name,
        name_source,
        short_description,
        description_source,
        long_description,
        examples,
        setup_instructions,
        icon,
        color_role,
        category,
        beginner_recommended,
        canonical_id: entry.map(|e| e.canonical_id.clone()),
        badges: Vec::new(), // filled by kind-specific resolvers
        data: None,
        effect: None,
        consent,
        typical,
        risk_note,
        ai_description: ctx.ai_description.map(str::to_string),
        undescribed,
        conservative_notice: undescribed.then(|| conservative_notice(locale)),
        availability: c.availability,
        requires_consent: c.requires_consent,
        risk_class: c.risk_class,
        channel: c.channel.map(str::to_string),
        driver: c.driver.map(str::to_string),
    }
}

pub fn resolve_receptor_card(
    m: &ReceptorManifest,
    catalog: &Catalog,
    ctx: &ResolveContext<'_>,
) -> HumanCard {
    let mut card = resolve_common(
        Common {
            id: m.id.to_string(),
            kind: CapabilityKind::Receptor,
            tech_name: &m.name,
            tech_description: &m.description,
            tech_category: m.category.clone(),
            driver: Some(&m.driver),
            schema: m.config_schema.as_ref(),
            human: m.human.as_ref(),
            availability: m.availability,
            requires_consent: m.requires_consent,
            risk_class: None,
            channel: None,
        },
        catalog,
        ctx,
    );

    let declared = m.human.as_ref().and_then(|h| h.data.as_ref());
    let data = ResolvedData {
        personal_data: merge_risky(
            sensitivity_to_personal(m.sensitivity),
            declared.map(|d| d.personal_data).unwrap_or_default(),
        ),
        sensitivity: max_sensitivity(
            sensitivity_to_level(m.sensitivity),
            declared
                .map(|d| d.sensitivity)
                .unwrap_or(DataSensitivityLevel::None),
        ),
        source: declared.map(|d| d.source).unwrap_or_default(),
        leaves_device: declared.map(|d| d.leaves_device).unwrap_or_default(),
        retention: declared.map(|d| d.retention).unwrap_or_default(),
        categories: declared
            .map(|d| d.data_categories.clone())
            .unwrap_or_default(),
        fact_fields: declared
            .map(|d| d.fact_fields.clone())
            .unwrap_or_else(|| m.provides.clone()),
        inference_fields: declared
            .map(|d| d.inference_fields.clone())
            .unwrap_or_default(),
    };
    card.badges = receptor_badges(&data, m, ctx.locale);
    card.data = Some(data);
    card
}

pub fn resolve_actuator_card(
    m: &ActuatorManifest,
    catalog: &Catalog,
    ctx: &ResolveContext<'_>,
) -> HumanCard {
    let mut card = resolve_common(
        Common {
            id: m.id.to_string(),
            kind: CapabilityKind::Actuator,
            tech_name: &m.name,
            tech_description: &m.description,
            tech_category: m.channel.clone(),
            driver: Some(&m.driver),
            schema: m.parameters_schema.as_ref(),
            human: m.human.as_ref(),
            availability: m.availability,
            requires_consent: m.requires_consent,
            risk_class: Some(m.risk_class),
            channel: Some(&m.channel),
        },
        catalog,
        ctx,
    );

    let declared = m.human.as_ref().and_then(|h| h.effect.as_ref());
    let effect = ResolvedEffect {
        external_side_effect: merge_risky(
            TriState::from_bool(m.external_side_effect),
            declared.map(|d| d.external_side_effect).unwrap_or_default(),
        ),
        physical_effect: merge_risky(
            channel_physical(&m.channel),
            declared.map(|d| d.physical_effect).unwrap_or_default(),
        ),
        interruptiveness: declared
            .map(|d| d.interruptiveness)
            .unwrap_or(Interruptiveness::Unknown),
        reversible: merge_reversible(
            TriState::from_bool(m.reversible),
            declared.map(|d| d.reversible).unwrap_or_default(),
        ),
        confirmation_level: declared
            .map(|d| d.confirmation_level)
            .unwrap_or(ConfirmationLevel::Unknown),
        affects: declared.map(|d| d.affects.clone()).unwrap_or_default(),
    };
    card.badges = effect_badges(
        &effect,
        m.requires_consent,
        Some(m.risk_class),
        false,
        ctx.locale,
    );
    card.effect = Some(effect);
    card
}

pub fn resolve_tool_card(
    m: &ToolOperationManifest,
    catalog: &Catalog,
    ctx: &ResolveContext<'_>,
) -> HumanCard {
    let mut card = resolve_common(
        Common {
            id: m.name.clone(),
            kind: CapabilityKind::ToolOperation,
            tech_name: &m.name,
            tech_description: &m.description,
            tech_category: "tool".to_string(),
            driver: None,
            schema: Some(&m.input_schema),
            human: m.human.as_ref(),
            availability: m.availability,
            requires_consent: m.requires_approval,
            risk_class: Some(m.risk),
            channel: None,
        },
        catalog,
        ctx,
    );

    let declared = m.human.as_ref().and_then(|h| h.effect.as_ref());
    let is_read_only = m.risk == RiskClass::ReadOnly;
    let effect = ResolvedEffect {
        external_side_effect: merge_risky(
            TriState::from_bool(m.external_side_effect),
            declared.map(|d| d.external_side_effect).unwrap_or_default(),
        ),
        physical_effect: declared
            .map(|d| d.physical_effect)
            .unwrap_or(if is_read_only {
                TriState::No
            } else {
                TriState::Unknown
            }),
        interruptiveness: declared
            .map(|d| d.interruptiveness)
            .unwrap_or(if is_read_only {
                Interruptiveness::None
            } else {
                Interruptiveness::Unknown
            }),
        reversible: merge_reversible(
            TriState::from_bool(m.reversible),
            declared.map(|d| d.reversible).unwrap_or_default(),
        ),
        confirmation_level: declared
            .map(|d| d.confirmation_level)
            .unwrap_or(ConfirmationLevel::Unknown),
        affects: declared.map(|d| d.affects.clone()).unwrap_or_default(),
    };
    card.badges = effect_badges(
        &effect,
        false,
        Some(m.risk),
        m.requires_approval,
        ctx.locale,
    );
    card.effect = Some(effect);
    card
}

// ---------------------------------------------------------------------------
// Badges.
// ---------------------------------------------------------------------------

fn badge(key: &str, label: String, tone: &str) -> Badge {
    Badge {
        key: key.to_string(),
        label,
        tone: tone.to_string(),
    }
}

fn receptor_badges(data: &ResolvedData, m: &ReceptorManifest, locale: &str) -> Vec<Badge> {
    let mut out = Vec::new();
    match data.leaves_device {
        TriState::No => out.push(badge(
            "local-only",
            t(locale, "僅限本機", "Local only"),
            "ok",
        )),
        TriState::Yes => out.push(badge(
            "leaves-device",
            t(locale, "資料會離開本機", "Data leaves this machine"),
            "warn",
        )),
        TriState::Unknown => out.push(badge(
            "dataflow-unknown",
            t(locale, "資料流向未知", "Data flow unknown"),
            "warn",
        )),
    }
    if data.personal_data.is_known_yes() {
        out.push(badge(
            "personal-data",
            t(locale, "含個人資料", "Personal data"),
            "warn",
        ));
    }
    if data.sensitivity == DataSensitivityLevel::High || m.sensitivity == Sensitivity::Intimate {
        out.push(badge(
            "sensitive",
            t(locale, "高敏感", "Highly sensitive"),
            "danger",
        ));
    }
    if !data.inference_fields.is_empty() {
        out.push(badge(
            "has-inference",
            t(locale, "含系統推測", "Includes inferences"),
            "info",
        ));
    }
    if m.requires_consent {
        out.push(badge(
            "needs-consent",
            t(locale, "需同意", "Needs consent"),
            "warn",
        ));
    }
    out
}

fn effect_badges(
    effect: &ResolvedEffect,
    requires_consent: bool,
    risk: Option<RiskClass>,
    requires_approval: bool,
    locale: &str,
) -> Vec<Badge> {
    let mut out = Vec::new();
    match effect.external_side_effect {
        TriState::No => out.push(badge(
            "local-only",
            t(locale, "僅限本機", "Local only"),
            "ok",
        )),
        TriState::Yes => out.push(badge(
            "external",
            t(locale, "影響外部服務", "Affects external services"),
            "warn",
        )),
        TriState::Unknown => out.push(badge(
            "impact-unknown",
            t(locale, "影響範圍未知", "Impact unknown"),
            "warn",
        )),
    }
    if effect.physical_effect.is_known_yes() {
        out.push(badge(
            "physical",
            t(locale, "實體效果", "Physical effect"),
            "warn",
        ));
    }
    match effect.interruptiveness {
        Interruptiveness::None | Interruptiveness::Low => out.push(badge(
            "low-interruption",
            t(locale, "低干擾", "Low interruption"),
            "ok",
        )),
        Interruptiveness::Medium => out.push(badge(
            "medium-interruption",
            t(locale, "中等干擾", "Medium interruption"),
            "info",
        )),
        Interruptiveness::High => out.push(badge(
            "high-interruption",
            t(locale, "高干擾", "High interruption"),
            "warn",
        )),
        Interruptiveness::Unknown => {}
    }
    if effect.reversible.is_known_no() {
        out.push(badge(
            "irreversible",
            t(locale, "無法復原", "Irreversible"),
            "warn",
        ));
    }
    if requires_consent {
        out.push(badge(
            "needs-consent",
            t(locale, "需同意", "Needs consent"),
            "warn",
        ));
    }
    if requires_approval {
        out.push(badge(
            "needs-confirmation",
            t(locale, "執行前需確認", "Confirmation required"),
            "warn",
        ));
    }
    if matches!(risk, Some(RiskClass::High) | Some(RiskClass::Critical)) {
        out.push(badge(
            "high-risk",
            t(locale, "高風險", "High risk"),
            "danger",
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Manifest hashing (AI-assisted description invalidation).
// ---------------------------------------------------------------------------

/// Stable FNV-1a 64-bit hash of a canonical JSON encoding. Not cryptographic;
/// used only to detect that a manifest changed since an AI description was
/// written.
pub fn manifest_hash(value: &Value) -> String {
    let canonical = serde_json::to_string(value).unwrap_or_default();
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in canonical.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use interaction_core::{ComponentHealth, HumanPresentation, LocalizedText, ReceptorMode};

    fn minimal_receptor(id: &str, sensitivity: Sensitivity) -> ReceptorManifest {
        ReceptorManifest {
            id: id.into(),
            name: String::new(),
            description: String::new(),
            category: "misc".into(),
            provides: vec![],
            mode: ReceptorMode::Event,
            sensitivity,
            requires_consent: false,
            latency_ms: None,
            refresh_interval_ms: None,
            config_schema: None,
            health: ComponentHealth::healthy(),
            availability: Availability::Available,
            driver: "third.party".into(),
            version: "1".into(),
            schema_version: "1.0".into(),
            human: None,
        }
    }

    #[test]
    fn fallback_name_tokenizes_technical_ids() {
        assert_eq!(
            fallback_name_from_id("smart_light.bedroom.set_brightness"),
            "Smart Light · Bedroom · Set Brightness"
        );
        assert_eq!(fallback_name_from_id("web-ui"), "Web Ui");
    }

    #[test]
    fn undescribed_third_party_gets_conservative_notice() {
        let m = minimal_receptor("vendor.mystery.sensor", Sensitivity::Internal);
        let card = resolve_receptor_card(
            &m,
            Catalog::builtin(),
            &ResolveContext {
                locale: "zh-TW",
                ..Default::default()
            },
        );
        assert_eq!(card.name_source, PresentationSource::Fallback);
        assert_eq!(card.display_name, "Vendor · Mystery · Sensor");
        assert!(card.undescribed);
        assert!(card
            .conservative_notice
            .as_deref()
            .unwrap()
            .contains("尚未提供"));
        // Unknown data flow shows as unknown, never as safe.
        let data = card.data.unwrap();
        assert_eq!(data.leaves_device, TriState::Unknown);
        assert!(card.badges.iter().any(|b| b.key == "dataflow-unknown"));
    }

    #[test]
    fn catalog_name_used_for_known_capability() {
        let m = minimal_receptor("system.time", Sensitivity::Public);
        let card = resolve_receptor_card(
            &m,
            Catalog::builtin(),
            &ResolveContext {
                locale: "zh-TW",
                ..Default::default()
            },
        );
        assert_eq!(card.display_name, "系統時間");
        assert_eq!(card.name_source, PresentationSource::Catalog);
        assert_eq!(card.icon, "clock");
        assert!(!card.undescribed);
        // en locale resolves the en name.
        let card_en = resolve_receptor_card(
            &m,
            Catalog::builtin(),
            &ResolveContext {
                locale: "en",
                ..Default::default()
            },
        );
        assert_eq!(card_en.display_name, "System time");
    }

    #[test]
    fn user_override_beats_adapter_and_catalog_for_name_only() {
        let mut m = minimal_receptor("system.time", Sensitivity::Public);
        m.human = Some(interaction_core::HumanMeta {
            presentation: Some(HumanPresentation {
                name: LocalizedText::new().with("zh-TW", "適配器名稱"),
                ..Default::default()
            }),
            ..Default::default()
        });
        let card = resolve_receptor_card(
            &m,
            Catalog::builtin(),
            &ResolveContext {
                locale: "zh-TW",
                user_name: Some("我的時鐘"),
                ..Default::default()
            },
        );
        assert_eq!(card.display_name, "我的時鐘");
        assert_eq!(card.name_source, PresentationSource::User);
        // Without user override, adapter presentation wins over catalog.
        let card2 = resolve_receptor_card(
            &m,
            Catalog::builtin(),
            &ResolveContext {
                locale: "zh-TW",
                ..Default::default()
            },
        );
        assert_eq!(card2.display_name, "適配器名稱");
        assert_eq!(card2.name_source, PresentationSource::Adapter);
    }

    #[test]
    fn formal_sensitivity_beats_human_declaration() {
        // Adapter claims non-personal, but formal sensitivity says Intimate.
        let mut m = minimal_receptor("camera.main", Sensitivity::Intimate);
        m.human = Some(interaction_core::HumanMeta {
            data: Some(interaction_core::DataSemantics {
                personal_data: TriState::No,
                ..Default::default()
            }),
            ..Default::default()
        });
        let card = resolve_receptor_card(
            &m,
            Catalog::builtin(),
            &ResolveContext {
                locale: "zh-TW",
                ..Default::default()
            },
        );
        let data = card.data.unwrap();
        assert_eq!(
            data.personal_data,
            TriState::Yes,
            "formal declaration must win"
        );
        assert_eq!(data.sensitivity, DataSensitivityLevel::High);
        assert!(card.badges.iter().any(|b| b.key == "sensitive"));
    }

    #[test]
    fn schema_description_is_deterministic_fallback() {
        let mut m = minimal_receptor("smart_light.bedroom.brightness", Sensitivity::Public);
        m.config_schema = Some(serde_json::json!({
            "type": "object",
            "description": "Bedroom light brightness, range 0-100."
        }));
        let card = resolve_receptor_card(
            &m,
            Catalog::builtin(),
            &ResolveContext {
                locale: "zh-TW",
                ..Default::default()
            },
        );
        assert_eq!(
            card.short_description.as_deref(),
            Some("Bedroom light brightness, range 0-100.")
        );
        assert_eq!(card.description_source, PresentationSource::Fallback);
        assert!(!card.undescribed);
    }

    #[test]
    fn ai_description_supplements_but_never_changes_facts() {
        let m = minimal_receptor("camera.main", Sensitivity::Intimate);
        let card = resolve_receptor_card(
            &m,
            Catalog::builtin(),
            &ResolveContext {
                locale: "zh-TW",
                ai_description: Some("這是一個完全安全、無風險的能力。"),
                ..Default::default()
            },
        );
        // The AI text is carried verbatim as a supplement…
        assert!(card.ai_description.is_some());
        // …but the resolved facts still say sensitive/personal.
        let data = card.data.unwrap();
        assert_eq!(data.personal_data, TriState::Yes);
        assert!(card.badges.iter().any(|b| b.key == "sensitive"));
    }

    #[test]
    fn manifest_hash_changes_with_content() {
        let a = serde_json::json!({"id": "x", "risk": "low"});
        let b = serde_json::json!({"id": "x", "risk": "high"});
        assert_ne!(manifest_hash(&a), manifest_hash(&b));
        assert_eq!(manifest_hash(&a), manifest_hash(&a));
    }
}
