//! §2 `CharacterManifest`、§2.1 驗證規則、§2.2 舊 Character Pack 遷移。
//!
//! 驗證只讀 bytes 長度與已解析的結構，不碰檔案系統；`entrypoint` 的 process／url／module
//! 只記錄，永不執行、連線或下載。錯誤訊息不回顯超過 200 字、不含絕對路徑。

use crate::capability::{classify_capability, is_canonical_channel, CapabilityClass};
use crate::intent::CharacterIntent;
use crate::{char_len, parse_protocol_version, truncate_for_echo, PROTOCOL_MAJOR, PROTOCOL_MINOR};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// 語系 → 文案。每個值有長度上限（§2.1）。
pub type LocalizedText = BTreeMap<String, String>;

pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;
pub const MAX_ASSETS: usize = 64;
pub const MAX_ASSET_BYTES_CEILING: u64 = 32 * 1024 * 1024;
pub const DEFAULT_MAX_ASSET_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_DISPLAY_NAME_CHARS: usize = 48;
pub const MAX_AUTHOR_CHARS: usize = 120;
pub const MAX_DESCRIPTION_CHARS: usize = 400;
pub const MAX_PREFERENCE_PROPERTIES: usize = 32;
pub const MAX_PREFERENCE_ENUM: usize = 16;
pub const MAX_PREFERENCE_STRING_LENGTH: u64 = 200;
/// §9：`durationRange` 上限 60 s。
pub const MAX_DURATION_MS: u64 = 60_000;
pub const MAX_VARIANTS: usize = 64;
pub const MAX_LIST_ENTRIES: usize = 256;
pub const MAX_ID_CHARS: usize = 64;
pub const MAX_PATH_CHARS: usize = 512;
/// `MigrationRegistry` 可容納的 migrator 數量上限（有界集合）。
pub const MAX_MIGRATORS: usize = 32;
/// 單一 migrator 可宣告的 `schemaVersion` 數量上限。
pub const MAX_MIGRATOR_VERSIONS: usize = 8;
/// `x-` 開頭的頂層欄位視為 vendor extension：保留但不列入 `unknownFields`。
pub const VENDOR_EXTENSION_PREFIX: &str = "x-";

/// `adapterKind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterKind {
    #[default]
    InProcess,
    Web,
    ExternalProcess,
    RemoteDevice,
}

/// `entrypoint`（tagged `kind`）。只記錄，匯入不執行、不連線、不下載。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Entrypoint {
    /// in-process 內建 adapter；`id` 必須在 host 白名單。
    Builtin { id: String },
    /// web：相對路徑模組（匯入時不執行）。
    Module { path: String },
    /// external-process（永不自動啟動）。
    Process { command: Vec<String> },
    /// remote-device（永不自動連線）。
    Url { url: String },
}

impl Entrypoint {
    /// 是否含可執行內容（process）。
    pub fn is_executable(&self) -> bool {
        matches!(self, Entrypoint::Process { .. })
    }

    /// 這種 entrypoint 對應的 adapterKind。
    pub fn expected_adapter_kind(&self) -> AdapterKind {
        match self {
            Entrypoint::Builtin { .. } => AdapterKind::InProcess,
            Entrypoint::Module { .. } => AdapterKind::Web,
            Entrypoint::Process { .. } => AdapterKind::ExternalProcess,
            Entrypoint::Url { .. } => AdapterKind::RemoteDevice,
        }
    }
}

/// `assets[]`。`bytes`／`sha256` 未知時為 `None`（舊 pack 遷移不假造數字）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssetDecl {
    pub id: String,
    /// 相對路徑（§2.1 規則）。
    pub path: String,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum QualityLevel {
    #[default]
    Full,
    Reduced,
    Minimal,
}

/// Reduced Motion 下此能力的行為（§3.2／§3.4 步驟 4）。缺省視為 `unchanged`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ReducedMotionBehavior {
    Static,
    Reduced,
    #[default]
    Unchanged,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DurationRange {
    pub min_ms: u64,
    pub max_ms: u64,
}

fn default_true() -> bool {
    true
}

/// §3.2 `CapabilityDecl`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDecl {
    #[serde(default = "default_true")]
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
    #[serde(default = "default_true")]
    pub interruptible: bool,
    #[serde(default)]
    pub resumable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_range: Option<DurationRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_schema: Option<PreferencesSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_level: Option<QualityLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduced_motion_behavior: Option<ReducedMotionBehavior>,
    #[serde(default)]
    pub requires_foreground: bool,
    #[serde(default)]
    pub requires_audio: bool,
}

impl Default for CapabilityDecl {
    fn default() -> Self {
        CapabilityDecl {
            supported: true,
            version: None,
            variants: Vec::new(),
            max_concurrent: None,
            interruptible: true,
            resumable: false,
            duration_range: None,
            parameter_schema: None,
            quality_level: None,
            reduced_motion_behavior: None,
            requires_foreground: false,
            requires_audio: false,
        }
    }
}

impl CapabilityDecl {
    pub fn supported() -> Self {
        CapabilityDecl::default()
    }

    pub fn with_variants<I: IntoIterator<Item = S>, S: Into<String>>(mut self, v: I) -> Self {
        self.variants = v.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_reduced_motion(mut self, behavior: ReducedMotionBehavior) -> Self {
        self.reduced_motion_behavior = Some(behavior);
        self
    }

    pub fn non_interruptible(mut self) -> Self {
        self.interruptible = false;
        self
    }
}

/// §2.1 `preferencesSchema` 白名單子集：只接受 `type: object` 與有限的屬性型別。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesSchema {
    #[serde(rename = "type")]
    pub schema_type: String,
    #[serde(default)]
    pub properties: BTreeMap<String, PreferenceProperty>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    /// 其餘鍵（`$ref`／`patternProperties`／`additionalProperties`…）：驗證時拒絕。
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// 單一屬性：`boolean`／`number`（`minimum`／`maximum`）／`integer`／`string`（`maxLength` ≤ 200、`enum` ≤ 16）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct PreferenceProperty {
    #[serde(rename = "type")]
    pub property_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u64>,
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    /// 其餘鍵（`$ref`／`pattern`／`properties`／`items`…）：驗證時拒絕。
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// `securityRequirements.fileAccess`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FileAccess {
    #[default]
    None,
    /// 只讀角色資料夾。
    CharacterFolder,
    /// 使用者逐檔授權（file-drop grant）。
    UserGranted,
}

/// `securityRequirements`（全部預設關閉）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRequirements {
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub executable: bool,
    #[serde(default)]
    pub file_access: FileAccess,
    #[serde(default)]
    pub audio_output: bool,
    #[serde(default)]
    pub microphone: bool,
    #[serde(default)]
    pub camera: bool,
}

fn default_max_asset_bytes() -> u64 {
    DEFAULT_MAX_ASSET_BYTES
}
fn default_max_concurrent_commands() -> u32 {
    4
}
fn default_max_queue() -> u32 {
    32
}
fn default_max_fps() -> u32 {
    60
}

/// `resourceLimits`（含預設）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLimits {
    #[serde(default = "default_max_asset_bytes")]
    pub max_asset_bytes: u64,
    #[serde(default = "default_max_concurrent_commands")]
    pub max_concurrent_commands: u32,
    #[serde(default = "default_max_queue")]
    pub max_queue: u32,
    #[serde(default = "default_max_fps")]
    pub max_fps: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        ResourceLimits {
            max_asset_bytes: DEFAULT_MAX_ASSET_BYTES,
            max_concurrent_commands: 4,
            max_queue: 32,
            max_fps: 60,
        }
    }
}

/// `fallbacks`：能力鏈與 intent 替換（§3.4 步驟 2／3）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct Fallbacks {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub intents: BTreeMap<String, String>,
}

/// `compatibility`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Compatibility {
    /// `1.x` 或 `1.N`。
    pub protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
}

/// `variants[]`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VariantDecl {
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub display_name: LocalizedText,
}

/// §2 `CharacterManifest`。未知頂層欄位保留在 `extra`（同 major 相容）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CharacterManifest {
    pub schema_version: String,
    pub character_id: String,
    pub display_name: LocalizedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub description: LocalizedText,
    pub version: String,
    #[serde(default)]
    pub adapter_kind: AdapterKind,
    pub entrypoint: Entrypoint,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<AssetDecl>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<String, CapabilityDecl>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input_capabilities: BTreeMap<String, CapabilityDecl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intents: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<VariantDecl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locales: Vec<String>,
    /// 省略時 UI 用中立文案。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<LocalizedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferences_schema: Option<PreferencesSchema>,
    #[serde(default)]
    pub security_requirements: SecurityRequirements,
    #[serde(default)]
    pub resource_limits: ResourceLimits,
    #[serde(default)]
    pub fallbacks: Fallbacks,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<Compatibility>,
    /// 未知頂層欄位（保留、不崩潰）；`x-` 開頭為 vendor extension。
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// 驗證限制（host 可調，但不得高於協定上限）。
///
/// `builtin_whitelist` 沒有預設值：核心不知道有哪些 in-process adapter，
/// host（runtime／Tauri／桌面 TS registry）必須注入自己的 adapter registry keys；
/// 空白名單代表「這個 host 不提供任何 builtin 角色」，所有 builtin entrypoint 都會被拒。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidationLimits {
    pub max_manifest_bytes: usize,
    pub max_assets: usize,
    pub max_asset_bytes_ceiling: u64,
    pub builtin_whitelist: Vec<String>,
    /// 本實作的 schema minor；manifest minor 較新 → `newerMinor: true`。
    pub implemented_minor: u32,
}

impl Default for ValidationLimits {
    fn default() -> Self {
        ValidationLimits {
            max_manifest_bytes: MAX_MANIFEST_BYTES,
            max_assets: MAX_ASSETS,
            max_asset_bytes_ceiling: MAX_ASSET_BYTES_CEILING,
            // 核心不認識任何具名 builtin adapter：白名單一律由 host 注入
            // （runtime／Tauri／TS registry 各自提供），預設是空的。
            builtin_whitelist: Vec::new(),
            implemented_minor: PROTOCOL_MINOR,
        }
    }
}

/// custom 能力註記。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CustomCapabilityNote {
    pub id: String,
    /// 已知 canonical 前綴但未收錄（§2.1 `unknown: true`）。
    pub unknown: bool,
}

/// 驗證通過後的報告（UI 依此標示第三方／外部／需要網路／有可執行程式）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManifestReport {
    pub newer_minor: bool,
    pub custom_capabilities: Vec<CustomCapabilityNote>,
    pub unknown_fields: Vec<String>,
    /// manifest 中不是 §4.1 詞彙的 intent（同 major 內保留、協商時 `unsupported`）。
    pub unknown_intents: Vec<String>,
    /// entrypoint 為 process 或 `securityRequirements.executable`：只記錄，不執行。
    pub executable: bool,
    pub needs_network: bool,
    /// `adapterKind ≠ in-process`。
    pub external: bool,
    pub warnings: Vec<String>,
}

/// 錯誤代碼。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestErrorCode {
    TooLarge,
    Json,
    SchemaVersion,
    CharacterId,
    LocalizedText,
    Version,
    Assets,
    AssetPath,
    AssetBytes,
    Entrypoint,
    Capability,
    PreferencesSchema,
    Fallbacks,
    Variants,
    Channels,
    Compatibility,
    ResourceLimits,
    Legacy,
}

impl ManifestErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ManifestErrorCode::TooLarge => "too-large",
            ManifestErrorCode::Json => "json",
            ManifestErrorCode::SchemaVersion => "schema-version",
            ManifestErrorCode::CharacterId => "character-id",
            ManifestErrorCode::LocalizedText => "localized-text",
            ManifestErrorCode::Version => "version",
            ManifestErrorCode::Assets => "assets",
            ManifestErrorCode::AssetPath => "asset-path",
            ManifestErrorCode::AssetBytes => "asset-bytes",
            ManifestErrorCode::Entrypoint => "entrypoint",
            ManifestErrorCode::Capability => "capability",
            ManifestErrorCode::PreferencesSchema => "preferences-schema",
            ManifestErrorCode::Fallbacks => "fallbacks",
            ManifestErrorCode::Variants => "variants",
            ManifestErrorCode::Channels => "channels",
            ManifestErrorCode::Compatibility => "compatibility",
            ManifestErrorCode::ResourceLimits => "resource-limits",
            ManifestErrorCode::Legacy => "legacy",
        }
    }
}

impl std::fmt::Display for ManifestErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Manifest 驗證錯誤。`message` 不回顯超過 200 字、不含絕對路徑。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code} at {path}: {message}")]
pub struct ManifestError {
    pub code: ManifestErrorCode,
    /// JSON 路徑（例如 `assets[2].path`）。
    pub path: String,
    pub message: String,
}

impl ManifestError {
    fn new(code: ManifestErrorCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        let message: String = message.into();
        ManifestError {
            code,
            path: path.into(),
            message: truncate_for_echo(&message),
        }
    }
}

/// 是否為 `^[a-z0-9][a-z0-9._-]{0,63}$`。
pub fn is_valid_character_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    let rest: Vec<char> = chars.collect();
    if rest.len() > 63 {
        return false;
    }
    rest.iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// §2.1 資產路徑規則。回傳的原因**不含**路徑本身。
pub fn check_relative_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("path is empty");
    }
    if char_len(path) > MAX_PATH_CHARS {
        return Err("path exceeds 512 chars");
    }
    if path.contains('\\') {
        return Err("path contains a backslash");
    }
    if path.chars().any(|c| c.is_control()) {
        return Err("path contains control characters");
    }
    if path.starts_with('/') {
        return Err("path is absolute (leading slash)");
    }
    if path.starts_with('~') {
        return Err("path is home-relative");
    }
    if path.contains(':') {
        return Err("path looks like a URL or a drive letter");
    }
    for segment in path.split('/') {
        match segment {
            "" => return Err("path contains an empty segment"),
            "." => return Err("path contains a '.' segment"),
            ".." => return Err("path contains a '..' segment (traversal)"),
            _ => {}
        }
    }
    Ok(())
}

fn check_localized(
    path: &str,
    text: &LocalizedText,
    max_chars: usize,
    require_one: bool,
) -> Result<(), ManifestError> {
    if require_one && text.is_empty() {
        return Err(ManifestError::new(
            ManifestErrorCode::LocalizedText,
            path,
            "at least one locale is required",
        ));
    }
    for (locale, value) in text {
        if locale.is_empty() || char_len(locale) > 35 {
            return Err(ManifestError::new(
                ManifestErrorCode::LocalizedText,
                format!("{path}.<locale>"),
                "locale key must be 1..=35 chars",
            ));
        }
        if char_len(value) > max_chars {
            return Err(ManifestError::new(
                ManifestErrorCode::LocalizedText,
                format!("{path}.{locale}"),
                format!("value exceeds {max_chars} chars"),
            ));
        }
    }
    Ok(())
}

fn is_media_type(value: &str) -> bool {
    let Some((kind, sub)) = value.split_once('/') else {
        return false;
    };
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '+' | '-')
            })
    };
    ok(kind) && ok(sub)
}

fn check_ident_list(
    path: &str,
    values: &[String],
    max_entries: usize,
    code: ManifestErrorCode,
) -> Result<(), ManifestError> {
    if values.len() > max_entries {
        return Err(ManifestError::new(
            code,
            path,
            format!("more than {max_entries} entries"),
        ));
    }
    for (i, v) in values.iter().enumerate() {
        if v.is_empty() || char_len(v) > MAX_ID_CHARS {
            return Err(ManifestError::new(
                code,
                format!("{path}[{i}]"),
                format!("entry must be 1..={MAX_ID_CHARS} chars"),
            ));
        }
        if v.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(ManifestError::new(
                code,
                format!("{path}[{i}]"),
                "entry contains whitespace or control characters",
            ));
        }
    }
    Ok(())
}

const ALLOWED_PROPERTY_TYPES: [&str; 4] = ["boolean", "number", "integer", "string"];

fn check_preferences_schema(path: &str, schema: &PreferencesSchema) -> Result<(), ManifestError> {
    let code = ManifestErrorCode::PreferencesSchema;
    if schema.schema_type != "object" {
        return Err(ManifestError::new(
            code,
            format!("{path}.type"),
            "only type: object is accepted",
        ));
    }
    if let Some(key) = schema.extra.keys().next() {
        return Err(ManifestError::new(
            code,
            format!("{path}.{}", truncate_for_echo(key)),
            "keyword is not allowed in preferencesSchema",
        ));
    }
    if schema.properties.len() > MAX_PREFERENCE_PROPERTIES {
        return Err(ManifestError::new(
            code,
            format!("{path}.properties"),
            format!("more than {MAX_PREFERENCE_PROPERTIES} properties"),
        ));
    }
    for name in &schema.required {
        if !schema.properties.contains_key(name) {
            return Err(ManifestError::new(
                code,
                format!("{path}.required"),
                "required names a property that does not exist",
            ));
        }
    }
    for (name, prop) in &schema.properties {
        let ppath = format!("{path}.properties.{}", truncate_for_echo(name));
        let name_ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && char_len(name) <= MAX_ID_CHARS;
        if !name_ok {
            return Err(ManifestError::new(
                code,
                ppath,
                "property name must match ^[a-zA-Z][a-zA-Z0-9_]{0,63}$",
            ));
        }
        if let Some(key) = prop.extra.keys().next() {
            return Err(ManifestError::new(
                code,
                format!("{ppath}.{}", truncate_for_echo(key)),
                "keyword is not allowed ($ref, pattern, nested objects and arrays are rejected)",
            ));
        }
        if !ALLOWED_PROPERTY_TYPES.contains(&prop.property_type.as_str()) {
            return Err(ManifestError::new(
                code,
                format!("{ppath}.type"),
                "type must be boolean, number, integer or string",
            ));
        }
        let is_numeric = matches!(prop.property_type.as_str(), "number" | "integer");
        let is_string = prop.property_type == "string";
        if (prop.minimum.is_some() || prop.maximum.is_some()) && !is_numeric {
            return Err(ManifestError::new(
                code,
                ppath,
                "minimum/maximum only allowed on number/integer",
            ));
        }
        if let (Some(min), Some(max)) = (prop.minimum, prop.maximum) {
            if min > max {
                return Err(ManifestError::new(code, ppath, "minimum exceeds maximum"));
            }
        }
        if prop.max_length.is_some() && !is_string {
            return Err(ManifestError::new(
                code,
                ppath,
                "maxLength only allowed on string",
            ));
        }
        if let Some(max_length) = prop.max_length {
            if max_length > MAX_PREFERENCE_STRING_LENGTH {
                return Err(ManifestError::new(
                    code,
                    format!("{ppath}.maxLength"),
                    format!("maxLength exceeds {MAX_PREFERENCE_STRING_LENGTH}"),
                ));
            }
        }
        if let Some(values) = &prop.enum_values {
            if !is_string {
                return Err(ManifestError::new(
                    code,
                    ppath,
                    "enum only allowed on string",
                ));
            }
            if values.len() > MAX_PREFERENCE_ENUM {
                return Err(ManifestError::new(
                    code,
                    format!("{ppath}.enum"),
                    format!("enum exceeds {MAX_PREFERENCE_ENUM} entries"),
                ));
            }
            if values
                .iter()
                .any(|v| char_len(v) > MAX_PREFERENCE_STRING_LENGTH as usize)
            {
                return Err(ManifestError::new(
                    code,
                    format!("{ppath}.enum"),
                    format!("enum entry exceeds {MAX_PREFERENCE_STRING_LENGTH} chars"),
                ));
            }
        }
        for (field, text) in [("title", &prop.title), ("description", &prop.description)] {
            if let Some(t) = text {
                if char_len(t) > MAX_DESCRIPTION_CHARS {
                    return Err(ManifestError::new(
                        code,
                        format!("{ppath}.{field}"),
                        format!("exceeds {MAX_DESCRIPTION_CHARS} chars"),
                    ));
                }
            }
        }
        if let Some(default) = &prop.default {
            let type_ok = match prop.property_type.as_str() {
                "boolean" => default.is_boolean(),
                "number" => default.is_number(),
                "integer" => default.is_i64() || default.is_u64(),
                "string" => default.is_string(),
                _ => false,
            };
            if !type_ok {
                return Err(ManifestError::new(
                    code,
                    format!("{ppath}.default"),
                    "default does not match the declared type",
                ));
            }
        }
    }
    Ok(())
}

fn check_capability_map(
    path: &str,
    map: &BTreeMap<String, CapabilityDecl>,
    input_only: bool,
    report: &mut ManifestReport,
) -> Result<(), ManifestError> {
    let code = ManifestErrorCode::Capability;
    for (id, decl) in map {
        let cpath = format!("{path}.{}", truncate_for_echo(id));
        match classify_capability(id) {
            CapabilityClass::Canonical => {
                if input_only && !id.starts_with("input.") {
                    return Err(ManifestError::new(
                        code,
                        cpath,
                        "inputCapabilities may only contain input.* or namespaced custom ids",
                    ));
                }
                if !input_only && id.starts_with("input.") {
                    return Err(ManifestError::new(
                        code,
                        cpath,
                        "input.* ids belong in inputCapabilities",
                    ));
                }
                if id == crate::capability::SYSTEM_TEXT {
                    return Err(ManifestError::new(
                        code,
                        cpath,
                        "system.text is provided by the runtime and may not be declared",
                    ));
                }
            }
            CapabilityClass::Custom => report.custom_capabilities.push(CustomCapabilityNote {
                id: id.clone(),
                unknown: false,
            }),
            CapabilityClass::UnknownCanonical => {
                report.custom_capabilities.push(CustomCapabilityNote {
                    id: id.clone(),
                    unknown: true,
                })
            }
            CapabilityClass::Invalid => return Err(ManifestError::new(
                code,
                cpath,
                "capability id must be canonical or namespaced custom (>= 3 lowercase segments)",
            )),
        }
        check_ident_list(
            &format!("{cpath}.variants"),
            &decl.variants,
            MAX_VARIANTS,
            code,
        )?;
        if let Some(range) = decl.duration_range {
            if range.min_ms > range.max_ms || range.max_ms > MAX_DURATION_MS {
                return Err(ManifestError::new(
                    code,
                    format!("{cpath}.durationRange"),
                    format!("minMs must be <= maxMs and maxMs <= {MAX_DURATION_MS}"),
                ));
            }
        }
        if decl.max_concurrent == Some(0) {
            return Err(ManifestError::new(
                code,
                format!("{cpath}.maxConcurrent"),
                "maxConcurrent must be >= 1",
            ));
        }
        if let Some(version) = &decl.version {
            if version.is_empty() || char_len(version) > 32 {
                return Err(ManifestError::new(
                    code,
                    format!("{cpath}.version"),
                    "version must be 1..=32 chars",
                ));
            }
        }
        if let Some(schema) = &decl.parameter_schema {
            check_preferences_schema(&format!("{cpath}.parameterSchema"), schema)?;
        }
    }
    Ok(())
}

/// §2.1 驗證。`bytes_len` 是原始檔案大小（呼叫端提供），這裡不讀檔。
pub fn validate_manifest(
    bytes_len: usize,
    manifest: &CharacterManifest,
    limits: &ValidationLimits,
) -> Result<ManifestReport, ManifestError> {
    let mut report = ManifestReport::default();
    if bytes_len > limits.max_manifest_bytes.min(MAX_MANIFEST_BYTES) {
        return Err(ManifestError::new(
            ManifestErrorCode::TooLarge,
            "",
            format!(
                "manifest is {bytes_len} bytes (max {})",
                limits.max_manifest_bytes.min(MAX_MANIFEST_BYTES)
            ),
        ));
    }

    match parse_protocol_version(&manifest.schema_version) {
        Some((major, minor)) if major == PROTOCOL_MAJOR => {
            report.newer_minor = minor > limits.implemented_minor;
        }
        Some((major, _)) => {
            return Err(ManifestError::new(
                ManifestErrorCode::SchemaVersion,
                "schemaVersion",
                format!("major {major} is not supported (expected {PROTOCOL_MAJOR})"),
            ))
        }
        None => {
            return Err(ManifestError::new(
                ManifestErrorCode::SchemaVersion,
                "schemaVersion",
                "must be major.minor",
            ))
        }
    }

    if !is_valid_character_id(&manifest.character_id) {
        return Err(ManifestError::new(
            ManifestErrorCode::CharacterId,
            "characterId",
            "must match ^[a-z0-9][a-z0-9._-]{0,63}$",
        ));
    }
    check_localized(
        "displayName",
        &manifest.display_name,
        MAX_DISPLAY_NAME_CHARS,
        true,
    )?;
    check_localized(
        "description",
        &manifest.description,
        MAX_DESCRIPTION_CHARS,
        false,
    )?;
    if let Some(pronouns) = &manifest.pronouns {
        check_localized("pronouns", pronouns, MAX_DISPLAY_NAME_CHARS, false)?;
    }
    if let Some(author) = &manifest.author {
        if char_len(author) > MAX_AUTHOR_CHARS {
            return Err(ManifestError::new(
                ManifestErrorCode::LocalizedText,
                "author",
                format!("exceeds {MAX_AUTHOR_CHARS} chars"),
            ));
        }
    }
    if manifest.version.is_empty() || char_len(&manifest.version) > 64 {
        return Err(ManifestError::new(
            ManifestErrorCode::Version,
            "version",
            "must be 1..=64 chars",
        ));
    }

    // resourceLimits 先檢查，資產上限要用。
    let rl = &manifest.resource_limits;
    let ceiling = limits.max_asset_bytes_ceiling.min(MAX_ASSET_BYTES_CEILING);
    if rl.max_asset_bytes > ceiling {
        return Err(ManifestError::new(
            ManifestErrorCode::ResourceLimits,
            "resourceLimits.maxAssetBytes",
            format!("exceeds ceiling {ceiling}"),
        ));
    }
    if rl.max_concurrent_commands == 0 || rl.max_concurrent_commands > 64 {
        return Err(ManifestError::new(
            ManifestErrorCode::ResourceLimits,
            "resourceLimits.maxConcurrentCommands",
            "must be 1..=64",
        ));
    }
    if rl.max_queue == 0 || rl.max_queue > crate::wire::Limits::MAX_PENDING as u32 {
        return Err(ManifestError::new(
            ManifestErrorCode::ResourceLimits,
            "resourceLimits.maxQueue",
            format!("must be 1..={}", crate::wire::Limits::MAX_PENDING),
        ));
    }
    if rl.max_fps == 0 || rl.max_fps > 120 {
        return Err(ManifestError::new(
            ManifestErrorCode::ResourceLimits,
            "resourceLimits.maxFps",
            "must be 1..=120",
        ));
    }

    // assets
    if manifest.assets.len() > limits.max_assets.min(MAX_ASSETS) {
        return Err(ManifestError::new(
            ManifestErrorCode::Assets,
            "assets",
            format!("more than {} assets", limits.max_assets.min(MAX_ASSETS)),
        ));
    }
    let mut asset_ids = BTreeSet::new();
    for (i, asset) in manifest.assets.iter().enumerate() {
        let apath = format!("assets[{i}]");
        if asset.id.is_empty() || char_len(&asset.id) > MAX_ID_CHARS || !asset_ids.insert(&asset.id)
        {
            return Err(ManifestError::new(
                ManifestErrorCode::Assets,
                format!("{apath}.id"),
                "asset id must be unique and 1..=64 chars",
            ));
        }
        if let Err(reason) = check_relative_path(&asset.path) {
            return Err(ManifestError::new(
                ManifestErrorCode::AssetPath,
                format!("{apath}.path"),
                reason,
            ));
        }
        if !is_media_type(&asset.media_type) {
            return Err(ManifestError::new(
                ManifestErrorCode::Assets,
                format!("{apath}.mediaType"),
                "must be a lowercase type/subtype",
            ));
        }
        if let Some(bytes) = asset.bytes {
            if bytes > rl.max_asset_bytes {
                return Err(ManifestError::new(
                    ManifestErrorCode::AssetBytes,
                    format!("{apath}.bytes"),
                    format!("{bytes} exceeds maxAssetBytes {}", rl.max_asset_bytes),
                ));
            }
        }
        if let Some(sha) = &asset.sha256 {
            if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(ManifestError::new(
                    ManifestErrorCode::Assets,
                    format!("{apath}.sha256"),
                    "must be 64 hex chars",
                ));
            }
        }
    }

    // entrypoint（只記錄，不執行）。
    let expected_kind = manifest.entrypoint.expected_adapter_kind();
    if expected_kind != manifest.adapter_kind {
        return Err(ManifestError::new(
            ManifestErrorCode::Entrypoint,
            "entrypoint",
            "entrypoint kind does not match adapterKind",
        ));
    }
    match &manifest.entrypoint {
        Entrypoint::Builtin { id } => {
            if !limits.builtin_whitelist.iter().any(|w| w == id) {
                return Err(ManifestError::new(
                    ManifestErrorCode::Entrypoint,
                    "entrypoint.id",
                    format!(
                        "builtin id is not in the host whitelist ({})",
                        limits.builtin_whitelist.join(", ")
                    ),
                ));
            }
        }
        Entrypoint::Module { path } => {
            if let Err(reason) = check_relative_path(path) {
                return Err(ManifestError::new(
                    ManifestErrorCode::Entrypoint,
                    "entrypoint.path",
                    reason,
                ));
            }
        }
        Entrypoint::Process { command } => {
            if command.is_empty() || command.iter().any(|c| c.is_empty()) {
                return Err(ManifestError::new(
                    ManifestErrorCode::Entrypoint,
                    "entrypoint.command",
                    "command must be a non-empty argv",
                ));
            }
            report.executable = true;
            report
                .warnings
                .push("entrypoint.process recorded only; never auto-started".into());
        }
        Entrypoint::Url { url } => {
            if !(url.starts_with("ws://") || url.starts_with("wss://")) {
                return Err(ManifestError::new(
                    ManifestErrorCode::Entrypoint,
                    "entrypoint.url",
                    "url must use ws:// or wss://",
                ));
            }
            report
                .warnings
                .push("entrypoint.url recorded only; never auto-connected".into());
        }
    }

    check_capability_map("capabilities", &manifest.capabilities, false, &mut report)?;
    check_capability_map(
        "inputCapabilities",
        &manifest.input_capabilities,
        true,
        &mut report,
    )?;

    check_ident_list(
        "channels",
        &manifest.channels,
        MAX_LIST_ENTRIES,
        ManifestErrorCode::Channels,
    )?;
    for (i, channel) in manifest.channels.iter().enumerate() {
        if !is_canonical_channel(channel) && !crate::capability::is_namespaced_custom(channel) {
            report.warnings.push(format!(
                "channels[{i}] is neither canonical nor namespaced; it will be ignored"
            ));
        }
    }
    check_ident_list(
        "states",
        &manifest.states,
        MAX_LIST_ENTRIES,
        ManifestErrorCode::Channels,
    )?;
    check_ident_list(
        "intents",
        &manifest.intents,
        MAX_LIST_ENTRIES,
        ManifestErrorCode::Channels,
    )?;
    for intent in &manifest.intents {
        if CharacterIntent::parse(intent).is_none() {
            report.unknown_intents.push(intent.clone());
        }
    }
    check_ident_list(
        "locales",
        &manifest.locales,
        MAX_LIST_ENTRIES,
        ManifestErrorCode::Channels,
    )?;

    if manifest.variants.len() > MAX_VARIANTS {
        return Err(ManifestError::new(
            ManifestErrorCode::Variants,
            "variants",
            format!("more than {MAX_VARIANTS} variants"),
        ));
    }
    let mut variant_ids = BTreeSet::new();
    for (i, v) in manifest.variants.iter().enumerate() {
        if v.id.is_empty() || char_len(&v.id) > MAX_ID_CHARS || !variant_ids.insert(&v.id) {
            return Err(ManifestError::new(
                ManifestErrorCode::Variants,
                format!("variants[{i}].id"),
                "variant id must be unique and 1..=64 chars",
            ));
        }
        check_localized(
            &format!("variants[{i}].displayName"),
            &v.display_name,
            MAX_DISPLAY_NAME_CHARS,
            false,
        )?;
    }

    if let Some(schema) = &manifest.preferences_schema {
        check_preferences_schema("preferencesSchema", schema)?;
    }

    for (cap, chain) in &manifest.fallbacks.capabilities {
        let fpath = format!("fallbacks.capabilities.{}", truncate_for_echo(cap));
        if classify_capability(cap) == CapabilityClass::Invalid {
            return Err(ManifestError::new(
                ManifestErrorCode::Fallbacks,
                fpath,
                "key is not a valid capability id",
            ));
        }
        check_ident_list(&fpath, chain, 16, ManifestErrorCode::Fallbacks)?;
        for target in chain {
            if target == cap || classify_capability(target) == CapabilityClass::Invalid {
                return Err(ManifestError::new(
                    ManifestErrorCode::Fallbacks,
                    fpath,
                    "chain entries must be valid capability ids other than the key",
                ));
            }
        }
    }
    for (intent, target) in &manifest.fallbacks.intents {
        let fpath = format!("fallbacks.intents.{}", truncate_for_echo(intent));
        if intent == target
            || char_len(intent) > MAX_ID_CHARS
            || target.is_empty()
            || char_len(target) > MAX_ID_CHARS
        {
            return Err(ManifestError::new(
                ManifestErrorCode::Fallbacks,
                fpath,
                "intent fallback must name a different intent (1..=64 chars)",
            ));
        }
        match (
            CharacterIntent::parse(intent),
            CharacterIntent::parse(target),
        ) {
            // 安全 intent 只能退到另一個安全 intent：呈現層不得用 fallbacks.intents
            // 把「需要同意／被阻擋／失敗／離線」換成 greet／play 之類的日常演出。
            (Some(from), Some(to)) if from.is_safety() && !to.is_safety() => {
                return Err(ManifestError::new(
                    ManifestErrorCode::Fallbacks,
                    fpath,
                    "a safety intent may only fall back to another safety intent",
                ));
            }
            (_, None) => report.warnings.push(format!(
                "{fpath} targets an unknown intent; it will never match"
            )),
            _ => {}
        }
    }

    if let Some(compat) = &manifest.compatibility {
        let ok = match compat.protocol.split_once('.') {
            Some((major, minor)) => {
                major.parse::<u32>().ok() == Some(PROTOCOL_MAJOR)
                    && (minor == "x" || minor.parse::<u32>().is_ok())
            }
            None => false,
        };
        if !ok {
            return Err(ManifestError::new(
                ManifestErrorCode::Compatibility,
                "compatibility.protocol",
                format!("must be {PROTOCOL_MAJOR}.x or {PROTOCOL_MAJOR}.N"),
            ));
        }
    }

    for key in manifest.extra.keys() {
        if !key.starts_with(VENDOR_EXTENSION_PREFIX) {
            report.unknown_fields.push(truncate_for_echo(key));
        }
    }
    report.needs_network = manifest.security_requirements.network
        || matches!(manifest.adapter_kind, AdapterKind::RemoteDevice);
    report.executable |= manifest.security_requirements.executable;
    report.external = manifest.adapter_kind != AdapterKind::InProcess;
    Ok(report)
}

/// 解析 bytes → manifest 並驗證（大小、JSON、§2.1）。不執行任何 entrypoint。
pub fn parse_manifest(
    bytes: &[u8],
    limits: &ValidationLimits,
) -> Result<(CharacterManifest, ManifestReport), ManifestError> {
    if bytes.len() > limits.max_manifest_bytes.min(MAX_MANIFEST_BYTES) {
        return Err(ManifestError::new(
            ManifestErrorCode::TooLarge,
            "",
            format!("manifest is {} bytes", bytes.len()),
        ));
    }
    let manifest: CharacterManifest = serde_json::from_slice(bytes).map_err(|e| {
        ManifestError::new(
            ManifestErrorCode::Json,
            "",
            format!(
                "line {} column {}: {}",
                e.line(),
                e.column(),
                e.classify_message()
            ),
        )
    })?;
    let report = validate_manifest(bytes.len(), &manifest, limits)?;
    Ok((manifest, report))
}

trait ClassifyMessage {
    fn classify_message(&self) -> String;
}

impl ClassifyMessage for serde_json::Error {
    /// serde 的錯誤訊息可能回顯輸入；只保留分類，不帶內容。
    fn classify_message(&self) -> String {
        match self.classify() {
            serde_json::error::Category::Io => "io error".into(),
            serde_json::error::Category::Syntax => "syntax error".into(),
            serde_json::error::Category::Data => "data does not match the manifest schema".into(),
            serde_json::error::Category::Eof => "unexpected end of input".into(),
        }
    }
}

/// 以 magic bytes 核對媒體型別（MIME／副檔名不可作唯一信任依據）。未知型別一律 `false`。
pub fn asset_magic_matches(media_type: &str, bytes: &[u8]) -> bool {
    let starts = |prefix: &[u8]| bytes.len() >= prefix.len() && &bytes[..prefix.len()] == prefix;
    let riff_with = |tag: &[u8]| bytes.len() >= 12 && starts(b"RIFF") && &bytes[8..12] == tag;
    let text_head = || {
        let head = &bytes[..bytes.len().min(1024)];
        let mut s = String::from_utf8_lossy(head).to_string();
        if let Some(stripped) = s.strip_prefix('\u{feff}') {
            s = stripped.to_string();
        }
        s.trim_start().to_string()
    };
    match media_type {
        "image/png" => starts(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "image/jpeg" | "image/jpg" => starts(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => starts(b"GIF87a") || starts(b"GIF89a"),
        "image/webp" => riff_with(b"WEBP"),
        "image/svg+xml" => {
            let head = text_head();
            (head.starts_with("<svg") || head.starts_with("<?xml") || head.starts_with("<!--"))
                && head.contains("<svg")
        }
        "application/json" => {
            let head = text_head();
            head.starts_with('{') || head.starts_with('[')
        }
        "audio/mpeg" | "audio/mp3" => {
            starts(b"ID3") || (bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
        }
        "audio/wav" | "audio/wave" | "audio/x-wav" => riff_with(b"WAVE"),
        "audio/ogg" | "application/ogg" => starts(b"OggS"),
        "video/webm" | "audio/webm" => starts(&[0x1A, 0x45, 0xDF, 0xA3]),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// §2.2 Migration
// ---------------------------------------------------------------------------

/// 舊 sprite pack：intent → 原生動畫名。
const SPRITE_INTENT_ANIMATIONS: &[(CharacterIntent, &str)] = &[
    (CharacterIntent::Idle, "idle"),
    (CharacterIntent::Notice, "notice"),
    (CharacterIntent::Think, "thinking"),
    (CharacterIntent::Work, "act"),
    (CharacterIntent::Wait, "waiting"),
    (CharacterIntent::Ask, "ask"),
    (CharacterIntent::Blocked, "blocked"),
    (CharacterIntent::Unknown, "unknown"),
    (CharacterIntent::VerifiedSuccess, "success"),
    (CharacterIntent::Failed, "failed"),
    (CharacterIntent::Offline, "offline"),
    (CharacterIntent::Emergency, "emergency"),
    (CharacterIntent::Rest, "quiet"),
    (CharacterIntent::Sleep, "paused"),
];

/// 舊 renderer `FALLBACKS` 鏈（動畫 → 較平靜的動畫）轉成 intent → intent。
///
/// 只保留「非安全 → 任意」與「安全 → 安全」：舊 renderer 的 `emergency → paused`、
/// `blocked → paused`、`ask → notice` 這類鏈會把安全語意換成日常演出，遷移時一律丟掉
/// （那些 intent 改走能力鏈，最差落到 system.text）。安全狀態永不退到 success／慶祝。
const SPRITE_INTENT_FALLBACKS: &[(CharacterIntent, CharacterIntent)] = &[
    (CharacterIntent::Failed, CharacterIntent::Blocked),
    (CharacterIntent::RequestConsent, CharacterIntent::Ask),
    (CharacterIntent::Acknowledge, CharacterIntent::Notice),
    (CharacterIntent::Greet, CharacterIntent::Notice),
    (CharacterIntent::Play, CharacterIntent::Notice),
    (CharacterIntent::Think, CharacterIntent::Idle),
    (CharacterIntent::Work, CharacterIntent::Idle),
    (CharacterIntent::Rest, CharacterIntent::Idle),
    (CharacterIntent::Sleep, CharacterIntent::Rest),
];

/// 舊 pack 的 localized 欄位（`{"zh-TW": "…"}`）→ [`LocalizedText`]。第三方 migrator 可用。
pub fn legacy_json_localized(value: Option<&serde_json::Value>) -> LocalizedText {
    value
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// 舊 pack 的字串欄位。第三方 migrator 可用。
pub fn legacy_json_str(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// 遷移錯誤（`ManifestErrorCode::Legacy`）。第三方 migrator 可用。
pub fn legacy_migration_error(message: impl Into<String>) -> ManifestError {
    ManifestError::new(ManifestErrorCode::Legacy, "", message)
}

/// 舊 pack 的共同骨架（id／名字／版本／in-process builtin entrypoint）→ 最小 manifest。
///
/// 第三方 migrator（例如某個角色自己的 crate）用它組出 manifest 再補自己的能力，
/// 這樣核心就不必知道任何具名角色的欄位。
pub fn legacy_base_manifest(
    json: &serde_json::Value,
    entrypoint_id: &str,
) -> Result<CharacterManifest, ManifestError> {
    let id = legacy_json_str(json, "id")
        .ok_or_else(|| legacy_migration_error("legacy pack has no id"))?;
    let name = legacy_json_localized(json.get("name"));
    if name.is_empty() {
        return Err(legacy_migration_error("legacy pack has no name"));
    }
    Ok(CharacterManifest {
        schema_version: crate::PROTOCOL_VERSION.to_string(),
        character_id: id,
        locales: name.keys().cloned().collect(),
        display_name: name,
        author: legacy_json_str(json, "author"),
        description: legacy_json_localized(json.get("description")),
        version: legacy_json_str(json, "version").unwrap_or_else(|| "0.0.0".to_string()),
        adapter_kind: AdapterKind::InProcess,
        entrypoint: Entrypoint::Builtin {
            id: entrypoint_id.to_string(),
        },
        assets: Vec::new(),
        capabilities: BTreeMap::new(),
        input_capabilities: BTreeMap::new(),
        channels: Vec::new(),
        states: Vec::new(),
        intents: Vec::new(),
        variants: Vec::new(),
        pronouns: None,
        preferences_schema: None,
        security_requirements: SecurityRequirements::default(),
        resource_limits: ResourceLimits::default(),
        fallbacks: Fallbacks::default(),
        compatibility: Some(Compatibility {
            protocol: "1.x".to_string(),
            runtime: Some(">=0.5.0".to_string()),
        }),
        extra: BTreeMap::new(),
    })
}

fn migrate_sprite_pack(
    json: &serde_json::Value,
    schema_version: &str,
) -> Result<CharacterManifest, ManifestError> {
    let mut manifest = legacy_base_manifest(json, "sprite")?;
    let sheet = legacy_json_str(json, "sheet")
        .ok_or_else(|| legacy_migration_error("character-pack has no sheet"))?;
    check_relative_path(&sheet)
        .map_err(|reason| ManifestError::new(ManifestErrorCode::AssetPath, "sheet", reason))?;
    manifest.assets.push(AssetDecl {
        id: "sheet".to_string(),
        path: sheet.clone(),
        media_type: "image/png".to_string(),
        bytes: None,
        sha256: None,
    });
    if let Some(preview) = legacy_json_str(json, "preview") {
        if check_relative_path(&preview).is_ok() {
            manifest.assets.push(AssetDecl {
                id: "preview".to_string(),
                path: preview,
                media_type: "image/png".to_string(),
                bytes: None,
                sha256: None,
            });
        }
    }
    let animations: Vec<String> = json
        .get("animations")
        .and_then(|a| a.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    if animations.is_empty() {
        return Err(legacy_migration_error("character-pack has no animations"));
    }
    let has_anchors = json
        .get("anchors")
        .and_then(|a| a.get("idle"))
        .and_then(|a| a.as_array())
        .is_some_and(|a| !a.is_empty());

    manifest.capabilities.insert(
        "visual.presence".into(),
        CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Static),
    );
    manifest.capabilities.insert(
        "visual.expression".into(),
        CapabilityDecl::supported()
            .with_variants(animations.iter().cloned())
            .with_reduced_motion(ReducedMotionBehavior::Static),
    );
    if has_anchors {
        manifest.capabilities.insert(
            "visual.gaze".into(),
            CapabilityDecl::supported().with_reduced_motion(ReducedMotionBehavior::Disabled),
        );
    }
    for input in [
        "input.click",
        "input.drag",
        "input.drop",
        "input.text",
        "input.fileDrop",
    ] {
        manifest
            .input_capabilities
            .insert(input.into(), CapabilityDecl::supported());
    }
    manifest.channels = vec!["transform".into(), "expression".into()];
    if has_anchors {
        manifest.channels.push("gaze".into());
    }
    manifest.states = animations.clone();
    for (intent, animation) in SPRITE_INTENT_ANIMATIONS {
        if animations.iter().any(|a| a == animation) {
            manifest.intents.push(intent.as_str().to_string());
        }
    }
    for (intent, target) in SPRITE_INTENT_FALLBACKS {
        // 守衛（表格已排除，這裡再擋一次）：安全 intent 只能退到安全 intent。
        if intent.is_safety() && !target.is_safety() {
            continue;
        }
        manifest
            .fallbacks
            .intents
            .insert(intent.as_str().to_string(), target.as_str().to_string());
    }
    manifest
        .fallbacks
        .capabilities
        .insert("visual.expression".into(), vec!["visual.presence".into()]);
    manifest.fallbacks.capabilities.insert(
        "visual.pose".into(),
        vec!["visual.expression".into(), "visual.presence".into()],
    );
    manifest
        .fallbacks
        .capabilities
        .insert("visual.textBubble".into(), vec!["visual.expression".into()]);
    manifest
        .fallbacks
        .capabilities
        .insert("visual.gaze".into(), vec!["visual.expression".into()]);
    manifest.extra.insert(
        "x-legacy".into(),
        serde_json::json!({
            "kind": "character-pack",
            "schemaVersion": schema_version,
            "sheet": sheet,
            "frameSize": json.get("frameSize").cloned().unwrap_or(serde_json::Value::Null),
            "anchor": json.get("anchor").cloned().unwrap_or(serde_json::Value::Null),
            "columns": json.get("columns").cloned().unwrap_or(serde_json::Value::Null),
            "animations": json.get("animations").cloned().unwrap_or(serde_json::Value::Null),
            "hasAnchors": has_anchors,
        }),
    );
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// §2.2 舊 pack 遷移：`PackMigrator` registry（核心只內建通用 sprite）
// ---------------------------------------------------------------------------

/// 錯誤訊息裡回顯 kind／schemaVersion 的長度上限（識別字本來就短；不回顯完整輸入）。
const MAX_KIND_ECHO_CHARS: usize = 64;

fn truncate_identifier(value: &str) -> String {
    if char_len(value) <= MAX_KIND_ECHO_CHARS {
        return value.to_string();
    }
    let mut out: String = value.chars().take(MAX_KIND_ECHO_CHARS).collect();
    out.push('…');
    out
}

/// 一種舊 pack 格式的遷移器。核心只內建通用 sprite（`character-pack` 1.0／1.1）；
/// 任何具名角色（例如某個 rig）的遷移由它自己的 crate 實作並由 host 註冊。
///
/// 純函式：不讀檔、不執行 entrypoint、不改寫使用者設定。
pub trait PackMigrator: Send + Sync {
    /// 這個 migrator 負責的 pack `kind`（例如 `character-pack`）。
    fn kind(&self) -> &str;
    /// 支援的 `schemaVersion`（≤ [`MAX_MIGRATOR_VERSIONS`] 個）。
    fn schema_versions(&self) -> &[&str];
    /// 遷移；輸入不合格時回 `ManifestErrorCode::Legacy`（訊息不回顯輸入內容）。
    fn migrate(&self, json: &serde_json::Value) -> Result<CharacterManifest, ManifestError>;
}

/// 通用 sprite pack（`character-pack` 1.0／1.1）遷移器。與任何具名角色無關。
#[derive(Debug, Default, Clone, Copy)]
pub struct SpritePackMigrator;

impl PackMigrator for SpritePackMigrator {
    fn kind(&self) -> &str {
        "character-pack"
    }
    fn schema_versions(&self) -> &[&str] {
        &["1.0", "1.1"]
    }
    fn migrate(&self, json: &serde_json::Value) -> Result<CharacterManifest, ManifestError> {
        let schema_version = legacy_json_str(json, "schemaVersion").unwrap_or_default();
        migrate_sprite_pack(json, &schema_version)
    }
}

/// 依 (kind, schemaVersion) 分派的遷移器登錄表。**有界**（≤ [`MAX_MIGRATORS`]）、
/// 不允許重複註冊同一組 (kind, version)：後註冊者不得悄悄覆蓋前者。
#[derive(Default)]
pub struct MigrationRegistry {
    migrators: Vec<Box<dyn PackMigrator>>,
}

impl std::fmt::Debug for MigrationRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigrationRegistry")
            .field("kinds", &self.supported_kinds())
            .finish()
    }
}

impl MigrationRegistry {
    /// 空的 registry：什麼都不遷移。
    pub fn new() -> Self {
        MigrationRegistry {
            migrators: Vec::new(),
        }
    }

    /// 核心內建的 registry：只有通用 sprite。
    pub fn with_core_migrators() -> Self {
        let mut registry = MigrationRegistry::new();
        // 核心自己註冊，數量在上限內，不可能失敗。
        let _ = registry.register(Box::new(SpritePackMigrator));
        registry
    }

    /// 註冊一個 migrator。超過上限、宣告版本過多或 (kind, version) 重複一律拒絕。
    pub fn register(&mut self, migrator: Box<dyn PackMigrator>) -> Result<(), ManifestError> {
        if self.migrators.len() >= MAX_MIGRATORS {
            return Err(legacy_migration_error(format!(
                "migration registry is full (max {MAX_MIGRATORS})"
            )));
        }
        let kind = migrator.kind().to_string();
        let versions = migrator.schema_versions();
        if versions.is_empty() || versions.len() > MAX_MIGRATOR_VERSIONS {
            return Err(legacy_migration_error(format!(
                "migrator must declare 1..={MAX_MIGRATOR_VERSIONS} schema versions"
            )));
        }
        for version in versions {
            if self.find(&kind, version).is_some() {
                return Err(legacy_migration_error(format!(
                    "a migrator for '{}' {} is already registered",
                    truncate_identifier(&kind),
                    truncate_identifier(version)
                )));
            }
        }
        self.migrators.push(migrator);
        Ok(())
    }

    /// 已註冊的 migrator 數量。
    pub fn len(&self) -> usize {
        self.migrators.len()
    }

    pub fn is_empty(&self) -> bool {
        self.migrators.is_empty()
    }

    /// 支援的 (kind, schemaVersion)，依註冊順序。
    pub fn supported_kinds(&self) -> Vec<(String, String)> {
        self.migrators
            .iter()
            .flat_map(|m| {
                m.schema_versions()
                    .iter()
                    .map(|v| (m.kind().to_string(), (*v).to_string()))
            })
            .collect()
    }

    fn find(&self, kind: &str, schema_version: &str) -> Option<&dyn PackMigrator> {
        self.migrators
            .iter()
            .find(|m| m.kind() == kind && m.schema_versions().contains(&schema_version))
            .map(|m| m.as_ref())
    }
}

/// §2.2 舊 pack JSON → manifest，依 `registry` 註冊的 (kind, schemaVersion) 分派。
/// 未註冊的格式一律拒絕（不猜、不執行）；不改寫使用者設定、不讀檔。
pub fn migrate_pack_to_manifest(
    json: &serde_json::Value,
    registry: &MigrationRegistry,
) -> Result<CharacterManifest, ManifestError> {
    let kind = legacy_json_str(json, "kind").unwrap_or_default();
    let schema_version = legacy_json_str(json, "schemaVersion").unwrap_or_default();
    match registry.find(&kind, &schema_version) {
        Some(migrator) => migrator.migrate(json),
        None => Err(legacy_migration_error(format!(
            "unsupported legacy pack kind/schemaVersion: {}/{}",
            truncate_identifier(&kind),
            truncate_identifier(&schema_version)
        ))),
    }
}

/// §2.2 舊 pack JSON → manifest（只有核心內建的通用 sprite）。
///
/// 保留給既有呼叫端；核心不依賴任何角色 crate，所以這條路徑**只**能遷移 sprite pack。
/// 需要其他格式（例如某個角色的 rig pack）請改用 [`migrate_pack_to_manifest`] 並註冊 migrator。
#[deprecated(
    since = "0.6.0",
    note = "use migrate_pack_to_manifest with a host MigrationRegistry; this path only migrates sprite packs"
)]
pub fn migrate_legacy_pack(json: &serde_json::Value) -> Result<CharacterManifest, ManifestError> {
    migrate_pack_to_manifest(json, &MigrationRegistry::with_core_migrators())
}

/// 建立最小合法 manifest（測試與 reference adapter 用）。
pub fn minimal_manifest(character_id: &str, builtin: &str) -> CharacterManifest {
    CharacterManifest {
        schema_version: crate::PROTOCOL_VERSION.to_string(),
        character_id: character_id.to_string(),
        display_name: [("en".to_string(), character_id.to_string())]
            .into_iter()
            .collect(),
        author: None,
        description: BTreeMap::new(),
        version: "0.1.0".to_string(),
        adapter_kind: AdapterKind::InProcess,
        entrypoint: Entrypoint::Builtin {
            id: builtin.to_string(),
        },
        assets: Vec::new(),
        capabilities: BTreeMap::new(),
        input_capabilities: BTreeMap::new(),
        channels: Vec::new(),
        states: Vec::new(),
        intents: Vec::new(),
        variants: Vec::new(),
        locales: vec!["en".to_string()],
        pronouns: None,
        preferences_schema: None,
        security_requirements: SecurityRequirements::default(),
        resource_limits: ResourceLimits::default(),
        fallbacks: Fallbacks::default(),
        compatibility: None,
        extra: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_id_regex() {
        assert!(is_valid_character_id("demo-character"));
        assert!(is_valid_character_id("a"));
        assert!(is_valid_character_id("0abc.def_g-h"));
        assert!(!is_valid_character_id(""));
        assert!(!is_valid_character_id("-abc"));
        assert!(!is_valid_character_id("Abc"));
        assert!(!is_valid_character_id(&"a".repeat(65)));
        assert!(is_valid_character_id(&"a".repeat(64)));
    }

    #[test]
    fn relative_path_rules() {
        assert!(check_relative_path("sheet.png").is_ok());
        assert!(check_relative_path("img/sheet.png").is_ok());
        assert!(check_relative_path("../x.png").is_err());
        assert!(check_relative_path("a/../x.png").is_err());
        assert!(check_relative_path("/etc/passwd").is_err());
        assert!(check_relative_path("C:/x.png").is_err());
        assert!(check_relative_path("a\\b.png").is_err());
        assert!(check_relative_path("https://x/y.png").is_err());
        assert!(check_relative_path("~/x.png").is_err());
        assert!(check_relative_path("./x.png").is_err());
        assert!(check_relative_path("a//b").is_err());
        assert!(check_relative_path("").is_err());
    }

    #[test]
    fn magic_bytes() {
        assert!(asset_magic_matches(
            "image/png",
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]
        ));
        assert!(!asset_magic_matches("image/png", b"GIF89a"));
        assert!(asset_magic_matches("image/gif", b"GIF89a..."));
        assert!(asset_magic_matches("image/jpeg", &[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(asset_magic_matches(
            "image/webp",
            b"RIFF\x00\x00\x00\x00WEBPVP8 "
        ));
        assert!(!asset_magic_matches(
            "image/webp",
            b"RIFF\x00\x00\x00\x00WAVEfmt "
        ));
        assert!(asset_magic_matches(
            "audio/wav",
            b"RIFF\x00\x00\x00\x00WAVEfmt "
        ));
        assert!(asset_magic_matches(
            "image/svg+xml",
            b"  <?xml version=\"1.0\"?><svg/>"
        ));
        assert!(asset_magic_matches(
            "image/svg+xml",
            b"\xEF\xBB\xBF<svg xmlns=\"x\"/>"
        ));
        assert!(!asset_magic_matches("image/svg+xml", b"<html><svg/>"));
        assert!(asset_magic_matches("application/json", b" {\"a\":1}"));
        assert!(!asset_magic_matches("application/json", b"<svg/>"));
        assert!(asset_magic_matches("audio/mpeg", b"ID3\x03"));
        assert!(asset_magic_matches("audio/mpeg", &[0xFF, 0xFB, 0x90]));
        assert!(asset_magic_matches("audio/ogg", b"OggS\x00"));
        assert!(asset_magic_matches(
            "video/webm",
            &[0x1A, 0x45, 0xDF, 0xA3, 0]
        ));
        assert!(!asset_magic_matches("font/woff2", b"wOF2"));
        assert!(!asset_magic_matches("image/png", b""));
    }
}
