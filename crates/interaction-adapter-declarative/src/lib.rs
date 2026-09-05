//! Declarative adapter engine.
//!
//! A human-owned YAML/JSON spec (File=Truth, lives in the config directory)
//! describes how to reach an external device/service over HTTP or SSE; the
//! engine turns each declared capability into a real [`Receptor`] /
//! [`Actuator`] instance. No Rust required for common integrations.
//!
//! Safety properties:
//! - Drivers only ever see `BoundedAction.effective` (policy-bounded values);
//!   template substitution has no access to anything else.
//! - Secrets are referenced (`secret://name`), resolved at send time from the
//!   OS environment / the runtime's secret store — never written to specs.
//! - SSRF guard: only http/https, and the cloud-metadata range 169.254.0.0/16
//!   is hard-blocked. Specs are human-owned config: the URL itself is the
//!   human's explicit allowlist entry.
//! - v0.5 real-hardware transports: `serial`（USB CDC，行分隔 JSON）、
//!   `mqtt`（QoS1 topic pair）、`ble`（GATT command/state characteristics，
//!   僅 macOS/Windows）。三者共用 `protocol.rs` 的誠實核心：hello 身分驗證
//!   （埠/IP/topic 不是身分）、配對碼握手、cmd nonce＋裝置端 dedupe、
//!   ack 逾時＝結果未知且絕不自動重送、斷線退避重連＋重新握手。
//! - Transports still beyond this build (websocket, webhook 及 Linux 上的
//!   ble) parse but are HONESTLY refused at build time with a clear error —
//!   nothing pretends to work.

pub mod fragment;
pub mod protocol;

#[cfg(all(
    feature = "transport-ble",
    any(target_os = "macos", target_os = "windows")
))]
pub mod ble;
#[cfg(any(feature = "transport-serial", feature = "transport-mqtt"))]
pub mod link_caps;
#[cfg(feature = "transport-mqtt")]
pub mod mqtt;
#[cfg(feature = "transport-serial")]
pub mod serial;

use async_trait::async_trait;
use chrono::Utc;
use interaction_adapter_sdk::{ActuatorManifestBuilder, DriverReceipt, ReceptorManifestBuilder};
use interaction_core::{
    ActionId, ActionReceipt, Actuator, ActuatorError, BoundedAction, ComponentHealth, HumanMeta,
    Observation, Receptor, ReceptorError, ReceptorId, ReceptorMode, RiskClass, Sensitivity,
    SessionContext,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Spec model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeclarativeSpec {
    pub schema_version: String,
    /// Adapter id; capability ids are prefixed with it when not absolute.
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub provider: Option<interaction_core::ProviderIdentity>,
    pub capabilities: Vec<CapabilitySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySpec {
    pub kind: CapabilityKindSpec,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Actuator channel (light/haptic/…); receptors use `category`.
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    pub transport: Transport,
    /// http/sse：請求描述（link 傳輸不用）。
    #[serde(default)]
    pub request: Option<RequestSpec>,
    /// serial/mqtt/ble actuator：裝置命令（name＋params 模板）。
    #[serde(default)]
    pub command: Option<CommandSpec>,
    /// serial 傳輸設定（同一 adapter 內的所有 serial capability 必須一致）。
    #[serde(default)]
    pub serial: Option<SerialSpec>,
    /// mqtt 傳輸設定。
    #[serde(default)]
    pub mqtt: Option<MqttSpec>,
    /// ble 傳輸設定。
    #[serde(default)]
    pub ble: Option<BleSpec>,
    /// Receptor: poll interval.
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
    /// Receptor: JSON-pointer fact mapping (`fact name` → `/json/pointer`).
    #[serde(default)]
    pub facts: BTreeMap<String, String>,
    /// Actuator: deepest honest confirmation (`requested` | `acknowledged`).
    #[serde(default)]
    pub confirmation: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub retry: Option<RetrySpec>,
    #[serde(default)]
    pub risk: Option<RiskClass>,
    #[serde(default)]
    pub requires_consent: bool,
    #[serde(default)]
    pub external_side_effect: bool,
    /// Optional emergency-stop request sent on estop.
    #[serde(default)]
    pub stop_request: Option<RequestSpec>,
    /// Actuator：**裝置安全上限**（maxMagnitude / maxDurationMs / maxPerHour …）。
    /// 直接進 manifest，讓 Policy Governor 的
    /// min(AI 請求, 使用者偏好, session 限制, 裝置安全上限, 剩餘預算)
    /// 真的有「裝置」那一項——host 端先夾，不能只靠韌體自己 clamp
    /// （非參考韌體可能根本不 clamp）。
    #[serde(default)]
    pub limits: interaction_core::ActuatorLimits,
    #[serde(default)]
    pub human: Option<HumanMeta>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityKindSpec {
    Receptor,
    Actuator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    Http,
    Sse,
    Webhook,
    Websocket,
    Mqtt,
    Serial,
    Ble,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestSpec {
    #[serde(default = "default_method")]
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// JSON body template; string values may contain `{{magnitude}}`,
    /// `{{durationMs}}`, `{{message}}`, `{{intent}}`, `{{actionId}}`.
    #[serde(default)]
    pub body: Option<Value>,
}

fn default_method() -> String {
    "GET".into()
}

/// link 傳輸的裝置命令：`{"type":"cmd","name":…,"params":…}` 的 name 與
/// params 模板（模板僅能引用 policy-bounded effective 值，與 http body 同）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    pub name: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// USB Serial 傳輸設定。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SerialSpec {
    /// 例：/dev/cu.usbmodem14101（macOS）、/dev/ttyUSB0（Linux）。
    pub port: String,
    #[serde(default = "default_baud")]
    pub baud: u32,
    /// 裝置身分：hello.deviceId 必須等於此值（埠路徑不是身分）。
    pub expected_device_id: String,
    /// 配對碼（建議 secret://）：連線後先 pair，失敗即拒。
    #[serde(default)]
    pub pairing_code: Option<String>,
}

fn default_baud() -> u32 {
    115_200
}

/// MQTT 傳輸設定。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MqttSpec {
    pub broker_host: String,
    #[serde(default = "default_mqtt_port")]
    pub broker_port: u16,
    /// 裝置 topic 前綴：host 發佈 `<prefix>/to-device`、訂閱 `<prefix>/from-device`。
    pub topic_prefix: String,
    pub expected_device_id: String,
    #[serde(default)]
    pub pairing_code: Option<String>,
    /// broker 認證（值可用 secret://）。
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    /// 多久沒聽到裝置就算失聯（毫秒；預設 15000＝參考韌體 state 推播週期的
    /// 3 倍）。broker 連著不代表 ESP32 還活著，健康度要看這個。
    #[serde(default)]
    pub liveness_timeout_ms: Option<u64>,
}

fn default_mqtt_port() -> u16 {
    1883
}

/// BLE 傳輸設定（GATT）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BleSpec {
    /// 廣播名稱（掃描用；不是身分——身分仍靠 hello.deviceId＋配對碼）。
    pub device_name: String,
    pub service_uuid: String,
    pub command_char_uuid: String,
    pub state_char_uuid: String,
    pub expected_device_id: String,
    #[serde(default)]
    pub pairing_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetrySpec {
    #[serde(default = "default_attempts")]
    pub attempts: u32,
    #[serde(default = "default_backoff")]
    pub backoff_ms: u64,
}

fn default_attempts() -> u32 {
    2
}
fn default_backoff() -> u64 {
    250
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

pub fn parse_spec(text: &str) -> Result<DeclarativeSpec, String> {
    let spec: DeclarativeSpec =
        serde_yaml::from_str(text).map_err(|e| format!("spec parse error: {e}"))?;
    validate_spec(&spec)?;
    Ok(spec)
}

pub fn validate_spec(spec: &DeclarativeSpec) -> Result<(), String> {
    if spec.capabilities.is_empty() {
        return Err("spec has no capabilities".into());
    }
    if spec.capabilities.len() > 64 {
        return Err("spec has too many capabilities (max 64)".into());
    }
    for cap in &spec.capabilities {
        validate_limits(cap)?;
        match cap.transport {
            Transport::Http | Transport::Sse => {
                let Some(request) = &cap.request else {
                    return Err(format!("{}: http/sse capability needs `request`", cap.id));
                };
                validate_url(&request.url)?;
                if let Some(stop) = &cap.stop_request {
                    validate_url(&stop.url)?;
                }
                for value in request.headers.values() {
                    if value.len() > 4096 {
                        return Err(format!("{}: header value too long", cap.id));
                    }
                }
                if cap.kind == CapabilityKindSpec::Receptor
                    && cap.transport == Transport::Http
                    && cap.facts.is_empty()
                {
                    return Err(format!(
                        "{}: http receptor needs a `facts` json-pointer mapping",
                        cap.id
                    ));
                }
            }
            Transport::Serial => {
                validate_link_cap(cap, cap.serial.is_some(), "serial")?;
                if !cfg!(feature = "transport-serial") {
                    return Err(honest_refusal(cap, "serial"));
                }
            }
            Transport::Mqtt => {
                validate_link_cap(cap, cap.mqtt.is_some(), "mqtt")?;
                if !cfg!(feature = "transport-mqtt") {
                    return Err(honest_refusal(cap, "mqtt"));
                }
            }
            Transport::Ble => {
                validate_link_cap(cap, cap.ble.is_some(), "ble")?;
                if !cfg!(all(
                    feature = "transport-ble",
                    any(target_os = "macos", target_os = "windows")
                )) {
                    return Err(format!(
                        "{}: transport Ble is declared but NOT supported in this build on this \
                         platform (BLE needs macOS/Windows with the transport-ble feature). \
                         The capability was not created.",
                        cap.id
                    ));
                }
                if let Some(ble) = &cap.ble {
                    for (label, raw) in [
                        ("serviceUuid", &ble.service_uuid),
                        ("commandCharUuid", &ble.command_char_uuid),
                        ("stateCharUuid", &ble.state_char_uuid),
                    ] {
                        if raw.parse::<u128>().is_err() && parse_uuid(raw).is_none() {
                            return Err(format!("{}: {label} {raw:?} is not a valid UUID", cap.id));
                        }
                    }
                }
            }
            other => {
                return Err(format!(
                    "{}: transport {other:?} is declared but NOT supported in this build \
                     (supported: http, sse, serial, mqtt, ble). The capability was not created.",
                    cap.id
                ));
            }
        }
    }
    Ok(())
}

/// 裝置安全上限的健全性檢查。壞的上限比沒有上限更危險：NaN 會讓
/// `min()` 的比較行為無法預期，0 會把整個動器變成永遠拒絕（看起來像壞掉，
/// 不像「被限制」）——兩者都誠實拒絕，不默默吞掉。
fn validate_limits(cap: &CapabilitySpec) -> Result<(), String> {
    let limits = &cap.limits;
    if let Some(m) = limits.max_magnitude {
        if !m.is_finite() || !(0.0..=1.0).contains(&m) {
            return Err(format!(
                "{}: limits.maxMagnitude must be a finite number in 0.0..=1.0 (got {m})",
                cap.id
            ));
        }
    }
    if limits.max_duration_ms == Some(0) {
        return Err(format!(
            "{}: limits.maxDurationMs 0 would block every action; remove it or give a real ceiling",
            cap.id
        ));
    }
    if limits.max_per_hour == Some(0) {
        return Err(format!(
            "{}: limits.maxPerHour 0 would block every action; remove it or give a real ceiling",
            cap.id
        ));
    }
    if limits.max_pattern_steps == Some(0) {
        return Err(format!(
            "{}: limits.maxPatternSteps 0 would block every pattern; remove it or give a real ceiling",
            cap.id
        ));
    }
    if cap.kind == CapabilityKindSpec::Receptor
        && limits != &interaction_core::ActuatorLimits::default()
    {
        return Err(format!(
            "{}: `limits` is an actuator-only safety ceiling (receptors have none)",
            cap.id
        ));
    }
    Ok(())
}

/// link 傳輸共通驗證：需要傳輸設定；actuator 需要 command；expectedDeviceId
/// 不可為空（身分不可省略）。
fn validate_link_cap(
    cap: &CapabilitySpec,
    has_transport_cfg: bool,
    label: &str,
) -> Result<(), String> {
    if !has_transport_cfg {
        return Err(format!(
            "{}: {label} capability needs a `{label}` config block",
            cap.id
        ));
    }
    if cap.kind == CapabilityKindSpec::Actuator && cap.command.is_none() {
        return Err(format!(
            "{}: {label} actuator needs a `command` (name + params template)",
            cap.id
        ));
    }
    if cap.kind == CapabilityKindSpec::Receptor && cap.facts.is_empty() {
        return Err(format!(
            "{}: {label} receptor needs a `facts` json-pointer mapping (e.g. /facts/lux)",
            cap.id
        ));
    }
    let expected = match (&cap.serial, &cap.mqtt, &cap.ble) {
        (Some(s), _, _) => &s.expected_device_id,
        (_, Some(m), _) => &m.expected_device_id,
        (_, _, Some(b)) => &b.expected_device_id,
        _ => return Ok(()),
    };
    if expected.trim().is_empty() {
        return Err(format!(
            "{}: expectedDeviceId must not be empty — a port/IP/topic is never an identity",
            cap.id
        ));
    }
    Ok(())
}

fn honest_refusal(cap: &CapabilitySpec, label: &str) -> String {
    format!(
        "{}: transport {label} is declared but NOT supported in this build \
         (the transport-{label} feature is disabled). The capability was not created.",
        cap.id
    )
}

/// 簡易 UUID 檢查（8-4-4-4-12 hex）。
fn parse_uuid(raw: &str) -> Option<[u8; 16]> {
    let clean: String = raw.chars().filter(|c| *c != '-').collect();
    if clean.len() != 32 || !clean.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, chunk) in clean.as_bytes().chunks(2).enumerate() {
        let hex = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(hex, 16).ok()?;
    }
    Some(out)
}

/// SSRF guard: http/https only; cloud-metadata endpoints hard-blocked in every
/// address encoding (IPv4, IPv4-mapped/compat IPv6, and known metadata hosts).
/// RFC1918 / loopback IPv4 stay allowed — the platform's job is talking to LAN
/// devices — but link-local (169.254/fe80) is always blocked because that is
/// where cloud metadata lives.
pub fn validate_url(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(raw).map_err(|e| format!("invalid url {raw:?}: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "url scheme {other:?} not allowed (http/https only)"
            ))
        }
    }
    let Some(host) = url.host_str() else {
        return Err("url has no host".into());
    };
    if is_blocked_host(host) {
        return Err(format!(
            "url host {host:?} is blocked (metadata/link-local range)"
        ));
    }
    Ok(())
}

/// True for hosts we must never fetch, resolving every IP encoding first.
pub fn is_blocked_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    if lower == "metadata.google.internal" || lower == "metadata" {
        return true;
    }
    // `host_str()` wraps IPv6 in brackets; strip them before parsing.
    let bare = lower.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        return ip_is_blocked(ip);
    }
    // Not an IP literal → block only the obvious metadata hostname above.
    false
}

fn ip_is_blocked(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    // Native IPv6 loopback / link-local first — no v6 LAN device is a supported
    // target and these reach host-local/metadata services. (Checked before the
    // v4-mapped canonicalization so `::1` is not mis-read as 0.0.0.1.)
    if let IpAddr::V6(v6) = ip {
        if v6.is_loopback() {
            return true;
        }
        let seg0 = v6.segments()[0];
        if (0xfe80..0xfec0).contains(&seg0) {
            return true;
        }
    }
    // Canonicalize the real IPv4-mapped form (`::ffff:a.b.c.d`) to v4 so
    // `[::ffff:169.254.169.254]` cannot smuggle a link-local metadata target.
    let v4 = match ip {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(v6) => v6.to_ipv4_mapped(),
    };
    if let Some(v4) = v4 {
        let o = v4.octets();
        // AWS/GCP/Azure/OpenStack link-local metadata (169.254.0.0/16).
        if o[0] == 169 && o[1] == 254 {
            return true;
        }
        // Alibaba Cloud metadata.
        if o == [100, 100, 100, 200] {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Secret references
// ---------------------------------------------------------------------------

/// Resolve `secret://name` header values. Sources, in order:
/// 1. `INTERACT_AI_SECRET_<NAME>` environment variable (uppercased, - → _)
/// 2. the runtime secret file `<home>/state/secrets.json` (0600, human-owned)
///
/// Secrets never appear in specs, logs or receipts.
pub fn resolve_secret(reference: &str, home: Option<&std::path::Path>) -> Result<String, String> {
    let Some(name) = reference.strip_prefix("secret://") else {
        return Ok(reference.to_string());
    };
    let env_key = format!(
        "INTERACT_AI_SECRET_{}",
        name.to_ascii_uppercase().replace('-', "_")
    );
    if let Ok(v) = std::env::var(&env_key) {
        return Ok(v);
    }
    if let Some(home) = home {
        let path = home.join("state").join("secrets.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(&text) {
                if let Some(v) = map.get(name) {
                    return Ok(v.clone());
                }
            }
        }
    }
    Err(format!(
        "secret {name:?} not found (set {env_key} or add it to state/secrets.json)"
    ))
}

// ---------------------------------------------------------------------------
// Receptor honesty: a reply that resolves no declared fact is not an observation
// ---------------------------------------------------------------------------

/// 「裝置有回應，但宣告的 fact 一個都對不上」的誠實說明。
///
/// 為什麼要點名兩邊：這種失敗幾乎都是 pointer 打錯或韌體欄位改名，只講
/// 「讀不到」會讓人去查線路。detail 列出 spec 宣告的 pointer 與裝置實際
/// 回了哪些鍵（有界：最多 12 個鍵），一眼就能對出差在哪。
pub(crate) fn unresolved_facts_detail(
    source: &str,
    declared: &BTreeMap<String, String>,
    payload: &Value,
) -> String {
    const MAX_KEYS: usize = 12;
    let pointers: Vec<&str> = declared.values().map(String::as_str).collect();
    let root = payload.pointer("/facts").unwrap_or(payload);
    let mut seen: Vec<String> = match root {
        Value::Object(map) => map.keys().take(MAX_KEYS).cloned().collect(),
        _ => Vec::new(),
    };
    if matches!(root, Value::Object(map) if map.len() > MAX_KEYS) {
        seen.push("…".into());
    }
    let seen = if seen.is_empty() {
        "<none>".to_string()
    } else {
        seen.join(", ")
    };
    format!(
        "{source} 裝置有回應，但宣告的 {} 個 fact 一個都對不上（pointer: {}；裝置回的鍵: {seen}）\
         ——這不算一次成功的觀察",
        declared.len(),
        pointers.join(", ")
    )
}

// ---------------------------------------------------------------------------
// Template substitution (bounded values ONLY)
// ---------------------------------------------------------------------------

/// Substitute placeholders from the policy-bounded effective parameters.
/// A string that is EXACTLY one numeric placeholder becomes a JSON number.
pub fn substitute(template: &Value, action: &BoundedAction) -> Value {
    let e = &action.effective;
    let scalars: BTreeMap<&str, Value> = BTreeMap::from([
        ("magnitude", json!(e.magnitude.unwrap_or(0.0))),
        ("durationMs", json!(e.duration_ms.unwrap_or(0))),
        ("message", json!(e.message.clone().unwrap_or_default())),
        ("intent", json!(action.intent.clone())),
        ("actionId", json!(action.action_id.as_str())),
    ]);
    fn walk(v: &Value, scalars: &BTreeMap<&str, Value>) -> Value {
        match v {
            Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
                    let key = trimmed[2..trimmed.len() - 2].trim();
                    if let Some(val) = scalars.get(key) {
                        return val.clone();
                    }
                }
                let mut out = s.clone();
                for (k, val) in scalars {
                    let needle = format!("{{{{{k}}}}}");
                    if out.contains(&needle) {
                        let text = match val {
                            Value::String(t) => t.clone(),
                            other => other.to_string(),
                        };
                        out = out.replace(&needle, &text);
                    }
                }
                Value::String(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(|i| walk(i, scalars)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), walk(v, scalars)))
                    .collect::<Map<_, _>>(),
            ),
            other => other.clone(),
        }
    }
    walk(template, &scalars)
}

// ---------------------------------------------------------------------------
// HTTP receptor
// ---------------------------------------------------------------------------

pub struct DeclarativeHttpReceptor {
    spec: CapabilitySpec,
    request: RequestSpec,
    adapter_id: String,
    client: reqwest::Client,
    home: Option<std::path::PathBuf>,
    endpoint: EndpointHealth,
}

/// HTTP/SSE 端點的誠實健康度。這種傳輸沒有常駐連線，唯一知道裝置在不在
/// 的方法就是「最近一次請求的結果」——所以：
/// - 還沒通訊過＝**未驗證**（degraded），不是 healthy（硬編 healthy 等於
///   對使用者宣稱一台從未回應過的裝置是好的）；
/// - 最近一次成功＝healthy；
/// - 最近一次失敗＝見 `receptor_health` / `actuator_health`。
#[derive(Default)]
pub struct EndpointHealth {
    /// 0＝尚未通訊；1＝最近一次成功；2＝最近一次失敗。
    state: std::sync::atomic::AtomicU8,
    detail: std::sync::Mutex<String>,
}

// 0＝尚未通訊（AtomicU8 的預設值，也是「未驗證」的語意）。
const ENDPOINT_OK: u8 = 1;
const ENDPOINT_FAILED: u8 = 2;

impl EndpointHealth {
    fn record_ok(&self) {
        self.state
            .store(ENDPOINT_OK, std::sync::atomic::Ordering::SeqCst);
    }

    fn record_failure(&self, detail: impl Into<String>) {
        let text = detail.into();
        match self.detail.lock() {
            Ok(mut guard) => *guard = text,
            Err(poisoned) => *poisoned.into_inner() = text,
        }
        self.state
            .store(ENDPOINT_FAILED, std::sync::atomic::Ordering::SeqCst);
    }

    fn failure_detail(&self) -> String {
        match self.detail.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Receptor 版：失敗＝offline。讀取本身不受健康度把關，所以下一次
    /// 觀察就能自行恢復。
    fn receptor_health(&self) -> ComponentHealth {
        match self.state.load(std::sync::atomic::Ordering::SeqCst) {
            ENDPOINT_OK => ComponentHealth::healthy(),
            ENDPOINT_FAILED => {
                ComponentHealth::offline(format!("最近一次讀取失敗：{}", self.failure_detail()))
            }
            _ => ComponentHealth::degraded(
                "尚未與裝置通訊過（HTTP 沒有常駐連線；健康度以最近一次請求為準）".to_string(),
            ),
        }
    }

    /// Actuator 版：失敗＝degraded 而不是 offline。executor 會用 status()
    /// 擋下派工，一旦標成 offline 就再也沒有請求能證明它恢復了（只有派工
    /// 會發請求）——那不是誠實，是自鎖。失敗原因照實寫進訊息。
    fn actuator_health(&self) -> ComponentHealth {
        match self.state.load(std::sync::atomic::Ordering::SeqCst) {
            ENDPOINT_OK => ComponentHealth::healthy(),
            ENDPOINT_FAILED => ComponentHealth::degraded(format!(
                "最近一次請求失敗：{}（HTTP 沒有常駐連線，要到下一次派工才知道是否恢復）",
                self.failure_detail()
            )),
            _ => ComponentHealth::degraded(
                "尚未與裝置通訊過（HTTP 沒有常駐連線；健康度以最近一次請求為準）".to_string(),
            ),
        }
    }
}

impl DeclarativeHttpReceptor {
    fn full_id(&self) -> String {
        qualified_id(&self.adapter_id, &self.spec.id)
    }
}

/// 這份 spec 的**受器** id ↔「是否需要 consent」。
///
/// 為什麼是這裡：受器 id 的組法（`qualified_id`）與 `requiresConsent` 都是這個
/// 引擎的事實。呼叫端（Runtime 的能力宣告）自己再拼一次字串，spec 的 id 規則
/// 一改，兩邊就會靜默不一致——那正是「哪些受器算高風險」這種安全語意最不能
/// 出現的漂移。
pub fn receptor_consent_map(spec: &DeclarativeSpec) -> Vec<(String, bool)> {
    spec.capabilities
        .iter()
        .filter(|cap| cap.kind == CapabilityKindSpec::Receptor)
        .map(|cap| (qualified_id(&spec.id, &cap.id), cap.requires_consent))
        .collect()
}

pub(crate) fn qualified_id(adapter: &str, id: &str) -> String {
    if id.contains('.') {
        id.to_string()
    } else {
        format!("{adapter}.{id}")
    }
}

/// 一次 HTTP 請求失敗時，「送出去了沒有」這件事本身。
///
/// 誠實階梯要靠它分開兩種完全不同的處境：
/// - `NotSent`：請求**確定沒離開這台機器**（URL/方法不合法、secret 解不出來、
///   連線建立就失敗）——沒有實體效果，重試是安全的。
/// - `OutcomeUnknown`：連線已建立、請求已寫出，之後才逾時／中斷——裝置**可能
///   已經執行了**。絕不重送（會重複實體效果），也絕不記成 failed。
#[derive(Debug, Clone)]
pub(crate) enum SendError {
    NotSent(String),
    OutcomeUnknown(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::NotSent(s) | SendError::OutcomeUnknown(s) => write!(f, "{s}"),
        }
    }
}

/// reqwest 錯誤 → 「送出去了沒有」。連線階段的錯誤（DNS、拒絕連線、
/// connect timeout）確定沒送出；其餘（送出後逾時、連線被重置、回應讀不完）
/// 一律當成結果未知——寧可標未知，也不冒充「沒發生」。
pub(crate) fn classify_send_error(e: &reqwest::Error) -> SendError {
    if e.is_connect() || e.is_builder() {
        SendError::NotSent(e.to_string())
    } else {
        SendError::OutcomeUnknown(e.to_string())
    }
}

async fn send_request(
    client: &reqwest::Client,
    req: &RequestSpec,
    body: Option<Value>,
    timeout_ms: u64,
    idempotency_key: Option<&str>,
    home: Option<&std::path::Path>,
) -> Result<(u16, Value), SendError> {
    validate_url(&req.url).map_err(SendError::NotSent)?;
    let method = reqwest::Method::from_bytes(req.method.as_bytes())
        .map_err(|_| SendError::NotSent(format!("bad method {:?}", req.method)))?;
    let mut builder = client
        .request(method, &req.url)
        .timeout(Duration::from_millis(timeout_ms.clamp(100, 60_000)));
    for (k, v) in &req.headers {
        let resolved = resolve_secret(v, home).map_err(SendError::NotSent)?;
        builder = builder.header(k, resolved);
    }
    if let Some(key) = idempotency_key {
        builder = builder.header("Idempotency-Key", key);
    }
    if let Some(body) = body {
        builder = builder.json(&body);
    }
    let resp = builder.send().await.map_err(|e| classify_send_error(&e))?;
    let status = resp.status().as_u16();
    let value = resp.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, value))
}

#[async_trait]
impl Receptor for DeclarativeHttpReceptor {
    fn manifest(&self) -> interaction_core::ReceptorManifest {
        let mut b = ReceptorManifestBuilder::new(
            &self.full_id(),
            self.spec.name.as_deref().unwrap_or(&self.spec.id),
            &format!("declarative.{}", self.adapter_id),
        )
        .description(self.spec.description.as_deref().unwrap_or(""))
        .category(self.spec.category.as_deref().unwrap_or("device"))
        .mode(ReceptorMode::Poll)
        .sensitivity(Sensitivity::Internal, self.spec.requires_consent)
        .refresh_interval_ms(self.spec.poll_interval_ms.unwrap_or(30_000));
        if let Some(h) = &self.spec.human {
            b = b.human(h.clone());
        }
        let keys: Vec<&str> = self.spec.facts.keys().map(String::as_str).collect();
        b.provides(&keys).build()
    }

    async fn start(&self, _context: SessionContext) -> Result<(), ReceptorError> {
        Ok(())
    }

    async fn read(&self) -> Result<Observation, ReceptorError> {
        let (status, value) = send_request(
            &self.client,
            &self.request,
            None,
            self.spec.timeout_ms.unwrap_or(5_000),
            None,
            self.home.as_deref(),
        )
        .await
        .map_err(|e| {
            // 受器沒有實體副作用：兩種失敗對它都只是「這次讀不到」。
            let detail = e.to_string();
            self.endpoint.record_failure(&detail);
            ReceptorError::Unavailable(detail)
        })?;
        if !(200..300).contains(&status) {
            self.endpoint
                .record_failure(format!("device returned HTTP {status}"));
            return Err(ReceptorError::Unavailable(format!(
                "device returned HTTP {status}"
            )));
        }
        self.endpoint.record_ok();
        let mut obs = Observation::now(
            ReceptorId::new(self.full_id()),
            format!("declarative.{}", self.adapter_id),
            Utc::now(),
        );
        for (fact, pointer) in &self.spec.facts {
            if let Some(v) = value.pointer(pointer) {
                obs.facts.insert(fact.clone(), v.clone());
            }
        }
        // 一個 fact 都沒解出來＝這次沒有讀到任何資料（pointer 打錯、對方
        // 的欄位改名、回了空物件）。回 Ok 會讓 runtime 把它算成一次成功的
        // 觀察並把 provider 記成「已測試」——那是拿 metadata 冒充資料。
        if obs.facts.is_empty() {
            let detail = unresolved_facts_detail("http", &self.spec.facts, &value);
            self.endpoint.record_failure(&detail);
            return Err(ReceptorError::Unavailable(detail));
        }
        Ok(obs)
    }

    async fn health(&self) -> ComponentHealth {
        self.endpoint.receptor_health().at(Utc::now())
    }

    async fn stop(&self) -> Result<(), ReceptorError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HTTP actuator
// ---------------------------------------------------------------------------

pub struct DeclarativeHttpActuator {
    spec: CapabilitySpec,
    request: RequestSpec,
    adapter_id: String,
    client: reqwest::Client,
    home: Option<std::path::PathBuf>,
    endpoint: EndpointHealth,
}

#[async_trait]
impl Actuator for DeclarativeHttpActuator {
    fn manifest(&self) -> interaction_core::ActuatorManifest {
        let mut b = ActuatorManifestBuilder::new(
            &qualified_id(&self.adapter_id, &self.spec.id),
            self.spec.name.as_deref().unwrap_or(&self.spec.id),
            self.spec.channel.as_deref().unwrap_or("device"),
            &format!("declarative.{}", self.adapter_id),
        )
        .description(self.spec.description.as_deref().unwrap_or(""))
        .risk(self.spec.risk.unwrap_or(RiskClass::BoundedSideEffect))
        .external(self.spec.external_side_effect)
        .requires_consent(true) // external device output is consent-gated by default
        // 裝置安全上限（spec 的 `limits:`）→ Policy Governor 的 min() 鏈。
        .limits(self.spec.limits.clone());
        if let Some(h) = &self.spec.human {
            b = b.human(h.clone());
        } else {
            // Formal declaration synthesized from the spec: the deepest
            // confirmation this transport can honestly provide. Everything
            // else stays Unknown (conservative).
            use interaction_core::{ConfirmationLevel, EffectSemantics, TriState};
            b = b.human(HumanMeta {
                effect: Some(EffectSemantics {
                    confirmation_level: if self.spec.confirmation.as_deref() == Some("acknowledged")
                    {
                        ConfirmationLevel::Acknowledged
                    } else {
                        ConfirmationLevel::Requested
                    },
                    external_side_effect: TriState::Unknown,
                    physical_effect: TriState::Unknown,
                    reversible: TriState::Unknown,
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        b.build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        if action.expires_at <= Utc::now() {
            return Err(ActuatorError::Rejected("action expired".into()));
        }
        let body = self
            .request
            .body
            .as_ref()
            .map(|template| substitute(template, &action));
        let timeout = self.spec.timeout_ms.unwrap_or(5_000);
        let retry = self.spec.retry.clone().unwrap_or(RetrySpec {
            attempts: 1,
            backoff_ms: 0,
        });
        let mut last_err = String::new();
        for attempt in 0..retry.attempts.max(1) {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(retry.backoff_ms)).await;
            }
            // 真正的送出時刻（收據的 Dispatched 要用它，不是回來之後的 now）。
            let sent_at = Utc::now();
            match send_request(
                &self.client,
                &self.request,
                body.clone(),
                timeout,
                Some(action.action_id.as_str()),
                self.home.as_deref(),
            )
            .await
            {
                Ok((status, _response)) if (200..300).contains(&status) => {
                    self.endpoint.record_ok();
                    // Only the status is recorded — never the device's response
                    // body. A malicious/echo device could reflect the resolved
                    // secret:// credential, and receipts are persisted and
                    // readable via the local API, so the body must not land
                    // there (the receipt is not the place for device payloads).
                    let receipt = DriverReceipt::start(&action, sent_at)
                        .dispatched_at(sent_at)
                        .note("httpStatus", json!(status));
                    // Honesty: 2xx means the DEVICE ACCEPTED the request. That
                    // is "acknowledged" at most — never completed/verified.
                    let receipt = if self.spec.confirmation.as_deref() == Some("acknowledged") {
                        receipt.acknowledged()
                    } else {
                        receipt
                    };
                    return Ok(receipt.finish());
                }
                Ok((status, _)) => {
                    // 裝置回了 HTTP 錯誤＝它收到了並拒絕：確定沒有「未知」，
                    // 可以重試（下一輪可能成功）。
                    last_err = format!("device returned HTTP {status}");
                }
                Err(SendError::OutcomeUnknown(detail)) => {
                    // 連線已建立、請求已寫出，之後才逾時／中斷：裝置**可能已經
                    // 執行了**。絕不重送（重複的實體效果比誠實的未知更糟），
                    // 也絕不記成 failed——收據停在 dispatched＋outcomeUnknown，
                    // runtime 據此標成 uncertain（link 傳輸同一套規則）。
                    self.endpoint.record_failure(&detail);
                    return Ok(DriverReceipt::start(&action, sent_at)
                        .dispatched_at(sent_at)
                        .note("transport", json!("http"))
                        .note("httpTimeout", json!(true))
                        .note("sendOutcomeUnknown", json!(true))
                        .note("outcomeUnknown", json!(true))
                        .note("detail", json!(detail))
                        .finish());
                }
                Err(SendError::NotSent(detail)) => {
                    // 確定沒送出（連不上／URL 不合法／secret 解不出來）：
                    // 沒有實體效果，重試是安全的。
                    last_err = detail;
                }
            }
        }
        // 每一次嘗試都失敗：健康度必須反映它（不得繼續宣稱 healthy）。
        self.endpoint.record_failure(&last_err);
        Ok(DriverReceipt::start(&action, Utc::now())
            .failed("device-unreachable", &last_err)
            .finish())
    }

    async fn status(&self) -> ComponentHealth {
        self.endpoint.actuator_health().at(Utc::now())
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(format!(
            "{action_id}: declarative http actions are single-shot"
        )))
    }

    /// estop：只有「裝置真的收下了停止請求」才算確認停止。
    ///
    /// runtime 的 estop 摘要把 `Ok(Ok(()))` 當成 stoppedActuators 的計數
    /// （已確認停止）。舊版無條件回 `Ok(())`：沒宣告 stopRequest 時什麼都
    /// 沒送、送了逾時／連不上／HTTP 500 也被 `let _ =` 丟掉——一台正在
    /// 發聲／亮燈的裝置會被列進「已停止」。誠實階梯：未確認不得算成已停止。
    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        let Some(stop) = &self.spec.stop_request else {
            return Err(ActuatorError::Unavailable(format!(
                "{}: no stop endpoint is declared for this actuator (spec has no stopRequest); \
                 nothing was sent — the device's state is UNKNOWN",
                qualified_id(&self.adapter_id, &self.spec.id)
            )));
        };
        match send_request(
            &self.client,
            stop,
            stop.body.clone(),
            2_000,
            None,
            self.home.as_deref(),
        )
        .await
        {
            // 2xx＝裝置收下了停止請求。這是這個傳輸能誠實達到的最深等級
            // （acknowledged），足以計入 stoppedActuators。
            Ok((status, _)) if (200..300).contains(&status) => {
                self.endpoint.record_ok();
                Ok(())
            }
            Ok((status, _)) => {
                let detail = format!(
                    "stop request refused by the device (HTTP {status}) — stop UNCONFIRMED"
                );
                self.endpoint.record_failure(&detail);
                Err(ActuatorError::Unavailable(detail))
            }
            Err(SendError::OutcomeUnknown(detail)) => {
                let detail = format!(
                    "stop request was written but not answered ({detail}) — stop UNCONFIRMED"
                );
                self.endpoint.record_failure(&detail);
                Err(ActuatorError::Unavailable(detail))
            }
            Err(SendError::NotSent(detail)) => {
                let detail = format!("stop request NOT sent ({detail}) — stop UNCONFIRMED");
                self.endpoint.record_failure(&detail);
                Err(ActuatorError::Unavailable(detail))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Engine: spec → instances
// ---------------------------------------------------------------------------

pub struct BuiltCapabilities {
    pub receptors: Vec<Arc<dyn Receptor>>,
    pub actuators: Vec<Arc<dyn Actuator>>,
    /// 這個 spec 實際開出來的裝置連線（每種 link 傳輸一條）。
    /// provider 被 disable／revoke 時，主機用它真的把連線關掉——
    /// 停用的 provider 不得繼續佔著序列埠／broker 連線做無盡重連。
    pub links: Vec<Arc<dyn protocol::LinkShutdown>>,
    /// 這個 spec 開出來的 AIP 通道（每條 link 傳輸一條）。Runtime 用它把裝置
    /// 綁進 Character Session——型別抹除，所以核心不必認得 serial／mqtt／ble。
    /// HTTP／SSE 沒有這種通道（那不是持續連線的裝置線協定），因此是空的。
    pub aip_channels: Vec<Arc<dyn protocol::DeviceAipChannel>>,
    /// 建置時發現、但不足以拒絕整份 spec 的問題（例如明文憑證）。
    /// 可稽核：呼叫端應把它顯示出來，不要靜靜吞掉。內容只點名能力與欄位，
    /// **永遠不含憑證值**。
    pub warnings: Vec<String>,
}

/// 明文憑證檢查：`pairingCode`／`password` 應該是 `secret://name` 參照
/// （env `INTERACT_AI_SECRET_<NAME>` 或 state/secrets.json），寫死在 YAML 裡
/// 等於把憑證留在 config 檔。這裡只警告不拒絕（既有 spec 不能一升級就壞掉），
/// 訊息只點名能力與欄位，絕不回顯值。
pub fn credential_warnings(spec: &DeclarativeSpec) -> Vec<String> {
    let mut out = Vec::new();
    let mut check = |cap_id: &str, field: &str, value: &Option<String>| {
        if let Some(v) = value {
            if !v.starts_with("secret://") {
                out.push(format!(
                    "{cap_id}: {field} 是寫在 adapter spec 裡的明文憑證；請改成 \
                     secret://<name>（環境變數 INTERACT_AI_SECRET_<NAME> 或 state/secrets.json）"
                ));
            }
        }
    };
    for cap in &spec.capabilities {
        if let Some(s) = &cap.serial {
            check(&cap.id, "pairingCode", &s.pairing_code);
        }
        if let Some(m) = &cap.mqtt {
            check(&cap.id, "pairingCode", &m.pairing_code);
            check(&cap.id, "password", &m.password);
        }
        if let Some(b) = &cap.ble {
            check(&cap.id, "pairingCode", &b.pairing_code);
        }
    }
    out
}

/// provider id → 該 provider 的實體連線（弱參照：能力被丟棄後自動失效，
/// 這張表不會把連線續命）。
///
/// 為什麼放在 adapter 引擎而不是 Runtime：連線的所有權本來就在這裡，
/// Runtime 只需要一個「關掉這個 provider 的連線」的入口。
type LinkTable = BTreeMap<String, Vec<std::sync::Weak<dyn protocol::LinkShutdown>>>;

static PROVIDER_LINKS: std::sync::OnceLock<std::sync::Mutex<LinkTable>> =
    std::sync::OnceLock::new();

fn lock_links() -> std::sync::MutexGuard<'static, LinkTable> {
    let table = PROVIDER_LINKS.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()));
    match table.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 記住「這個 provider 的連線是這幾條」。同一 provider 重新註冊會覆蓋。
pub fn register_provider_links(provider_id: &str, links: &[Arc<dyn protocol::LinkShutdown>]) {
    let mut table = lock_links();
    // 順手清掉已經沒人持有的條目（有界維護，不積垃圾）。
    table.retain(|_, v| {
        v.retain(|w| w.strong_count() > 0);
        !v.is_empty()
    });
    if links.is_empty() {
        table.remove(provider_id);
        return;
    }
    table.insert(
        provider_id.to_string(),
        links.iter().map(Arc::downgrade).collect(),
    );
}

/// 關掉某個 provider 的所有裝置連線（disable／revoke 用）。
/// 回傳實際關掉的連線描述（診斷用；不含 secret）。
pub fn shutdown_provider_links(provider_id: &str) -> Vec<String> {
    let weaks = {
        let mut table = lock_links();
        table.remove(provider_id).unwrap_or_default()
    };
    let mut closed = Vec::new();
    for weak in weaks {
        if let Some(link) = weak.upgrade() {
            link.shutdown();
            closed.push(link.describe());
        }
    }
    closed
}

pub fn build(
    spec: &DeclarativeSpec,
    home: Option<std::path::PathBuf>,
) -> Result<BuiltCapabilities, String> {
    validate_spec(spec)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        // 連線階段獨立設限：這樣「連不上」會以 connect 錯誤現身（確定沒送出、
        // 可安全重試），不會掉進請求層逾時而被當成「可能已送達」。
        .connect_timeout(Duration::from_secs(3))
        // No redirects: a redirect Location is never re-validated by the SSRF
        // guard, so following one would let an allowlisted host bounce us to a
        // metadata/internal target. Fail instead of following.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let warnings = credential_warnings(spec);
    for w in &warnings {
        // 可稽核：明文憑證不阻擋，但一定要留下痕跡（值本身不在訊息裡）。
        tracing::warn!(adapter = %spec.id, "{w}");
    }
    let mut out = BuiltCapabilities {
        receptors: vec![],
        actuators: vec![],
        links: vec![],
        aip_channels: vec![],
        warnings,
    };
    // 每種 link 傳輸共享一條連線（同一 adapter 內設定必須一致——不一致代表
    // spec 想同時對到兩個裝置，應拆成兩個 adapter）。
    #[cfg(feature = "transport-serial")]
    let mut serial_link: Option<(
        SerialSpec,
        Arc<protocol::DeviceLink<Arc<serial::SerialRawLink>>>,
    )> = None;
    #[cfg(feature = "transport-mqtt")]
    let mut mqtt_link: Option<(MqttSpec, Arc<protocol::DeviceLink<Arc<mqtt::MqttRawLink>>>)> = None;
    #[cfg(all(
        feature = "transport-ble",
        any(target_os = "macos", target_os = "windows")
    ))]
    let mut ble_link: Option<(BleSpec, Arc<protocol::DeviceLink<Arc<ble::BleRawLink>>>)> = None;

    for cap in &spec.capabilities {
        match cap.transport {
            Transport::Http | Transport::Sse => {
                let request = cap
                    .request
                    .clone()
                    .ok_or_else(|| format!("{}: missing request", cap.id))?;
                match cap.kind {
                    CapabilityKindSpec::Receptor => {
                        out.receptors.push(Arc::new(DeclarativeHttpReceptor {
                            spec: cap.clone(),
                            request,
                            adapter_id: spec.id.clone(),
                            client: client.clone(),
                            home: home.clone(),
                            endpoint: EndpointHealth::default(),
                        }))
                    }
                    CapabilityKindSpec::Actuator => {
                        // 有外部副作用、卻沒有停止端點：emergency stop 對它
                        // 永遠只能誠實回「未確認」。載入時就講清楚，不要等到
                        // 真的按下 estop 才發現這台裝置停不了。
                        if cap.external_side_effect && cap.stop_request.is_none() {
                            tracing::warn!(
                                adapter = %spec.id,
                                capability = %cap.id,
                                "actuator has external side effects but declares no stopRequest: \
                                 emergency stop cannot be confirmed for it (it will never be \
                                 counted as stopped)"
                            );
                        }
                        out.actuators.push(Arc::new(DeclarativeHttpActuator {
                            spec: cap.clone(),
                            request,
                            adapter_id: spec.id.clone(),
                            client: client.clone(),
                            home: home.clone(),
                            endpoint: EndpointHealth::default(),
                        }))
                    }
                }
            }
            #[cfg(feature = "transport-serial")]
            Transport::Serial => {
                let cfg = cap
                    .serial
                    .clone()
                    .ok_or_else(|| format!("{}: missing serial", cap.id))?;
                let link = match &serial_link {
                    Some((existing, link)) => {
                        if existing.port != cfg.port
                            || existing.baud != cfg.baud
                            || existing.expected_device_id != cfg.expected_device_id
                        {
                            return Err(format!(
                                "{}: all serial capabilities in one adapter must share the same \
                                 serial config (one adapter = one device)",
                                cap.id
                            ));
                        }
                        link.clone()
                    }
                    None => {
                        let pairing = cfg
                            .pairing_code
                            .as_deref()
                            .map(|code| resolve_secret(code, home.as_deref()))
                            .transpose()?;
                        let raw = serial::SerialRawLink::spawn(cfg.port.clone(), cfg.baud);
                        let link = Arc::new(protocol::DeviceLink::new(
                            raw,
                            cfg.expected_device_id.clone(),
                            pairing,
                        ));
                        serial_link = Some((cfg.clone(), link.clone()));
                        out.links
                            .push(link.clone() as Arc<dyn protocol::LinkShutdown>);
                        out.aip_channels.push(Arc::new(protocol::AipChannel {
                            link: link.clone(),
                            transport: "serial",
                        })
                            as Arc<dyn protocol::DeviceAipChannel>);
                        link
                    }
                };
                push_link_cap(&mut out, cap, spec, link, "serial")?;
            }
            #[cfg(feature = "transport-mqtt")]
            Transport::Mqtt => {
                let cfg = cap
                    .mqtt
                    .clone()
                    .ok_or_else(|| format!("{}: missing mqtt", cap.id))?;
                let link = match &mqtt_link {
                    Some((existing, link)) => {
                        if existing.broker_host != cfg.broker_host
                            || existing.broker_port != cfg.broker_port
                            || existing.topic_prefix != cfg.topic_prefix
                            || existing.expected_device_id != cfg.expected_device_id
                        {
                            return Err(format!(
                                "{}: all mqtt capabilities in one adapter must share the same \
                                 mqtt config (one adapter = one device)",
                                cap.id
                            ));
                        }
                        link.clone()
                    }
                    None => {
                        let pairing = cfg
                            .pairing_code
                            .as_deref()
                            .map(|code| resolve_secret(code, home.as_deref()))
                            .transpose()?;
                        let credentials = match (&cfg.username, &cfg.password) {
                            (Some(user), Some(pass)) => Some((
                                resolve_secret(user, home.as_deref())?,
                                resolve_secret(pass, home.as_deref())?,
                            )),
                            _ => None,
                        };
                        let raw = mqtt::MqttRawLink::spawn(
                            cfg.broker_host.clone(),
                            cfg.broker_port,
                            cfg.topic_prefix.clone(),
                            &spec.id,
                            credentials,
                            cfg.liveness_timeout_ms,
                        );
                        let link = Arc::new(protocol::DeviceLink::new(
                            raw,
                            cfg.expected_device_id.clone(),
                            pairing,
                        ));
                        mqtt_link = Some((cfg.clone(), link.clone()));
                        out.links
                            .push(link.clone() as Arc<dyn protocol::LinkShutdown>);
                        out.aip_channels.push(Arc::new(protocol::AipChannel {
                            link: link.clone(),
                            transport: "mqtt",
                        })
                            as Arc<dyn protocol::DeviceAipChannel>);
                        link
                    }
                };
                push_link_cap(&mut out, cap, spec, link, "mqtt")?;
            }
            #[cfg(all(
                feature = "transport-ble",
                any(target_os = "macos", target_os = "windows")
            ))]
            Transport::Ble => {
                let cfg = cap
                    .ble
                    .clone()
                    .ok_or_else(|| format!("{}: missing ble", cap.id))?;
                let link = match &ble_link {
                    Some((existing, link)) => {
                        if existing.device_name != cfg.device_name
                            || existing.expected_device_id != cfg.expected_device_id
                        {
                            return Err(format!(
                                "{}: all ble capabilities in one adapter must share the same \
                                 ble config (one adapter = one device)",
                                cap.id
                            ));
                        }
                        link.clone()
                    }
                    None => {
                        let parse = |raw: &str, label: &str| {
                            raw.parse::<uuid::Uuid>()
                                .map_err(|e| format!("{}: {label}: {e}", cap.id))
                        };
                        let pairing = cfg
                            .pairing_code
                            .as_deref()
                            .map(|code| resolve_secret(code, home.as_deref()))
                            .transpose()?;
                        let raw = ble::BleRawLink::new(
                            cfg.device_name.clone(),
                            parse(&cfg.service_uuid, "serviceUuid")?,
                            parse(&cfg.command_char_uuid, "commandCharUuid")?,
                            parse(&cfg.state_char_uuid, "stateCharUuid")?,
                        );
                        let link = Arc::new(protocol::DeviceLink::new(
                            raw,
                            cfg.expected_device_id.clone(),
                            pairing,
                        ));
                        ble_link = Some((cfg.clone(), link.clone()));
                        out.links
                            .push(link.clone() as Arc<dyn protocol::LinkShutdown>);
                        out.aip_channels.push(Arc::new(protocol::AipChannel {
                            link: link.clone(),
                            transport: "ble",
                        })
                            as Arc<dyn protocol::DeviceAipChannel>);
                        link
                    }
                };
                push_link_cap(&mut out, cap, spec, link, "ble")?;
            }
            // validate_spec 已誠實拒絕的組合：不可能到這裡。
            #[allow(unreachable_patterns)]
            other => {
                return Err(format!(
                    "{}: transport {other:?} not supported in this build",
                    cap.id
                ))
            }
        }
    }
    Ok(out)
}

/// 把一個 link capability 塞進輸出（receptor / actuator 共用）。
#[cfg(any(feature = "transport-serial", feature = "transport-mqtt"))]
fn push_link_cap<L: protocol::RawLink + 'static>(
    out: &mut BuiltCapabilities,
    cap: &CapabilitySpec,
    spec: &DeclarativeSpec,
    link: Arc<protocol::DeviceLink<L>>,
    label: &'static str,
) -> Result<(), String> {
    match cap.kind {
        CapabilityKindSpec::Receptor => {
            out.receptors.push(Arc::new(link_caps::LinkReceptor {
                spec: cap.clone(),
                adapter_id: spec.id.clone(),
                link,
                transport_label: label,
            }));
        }
        CapabilityKindSpec::Actuator => {
            let command = cap
                .command
                .clone()
                .ok_or_else(|| format!("{}: missing command", cap.id))?;
            out.actuators.push(Arc::new(link_caps::LinkActuator::new(
                cap.clone(),
                command,
                spec.id.clone(),
                link,
                label,
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_guard_blocks_metadata_and_odd_schemes() {
        // LAN devices (RFC1918 / loopback IPv4) stay allowed.
        assert!(validate_url("http://192.168.1.50/status").is_ok());
        assert!(validate_url("http://127.0.0.1:9000/x").is_ok());
        assert!(validate_url("https://example.com/x").is_ok());
        // Cloud metadata, every encoding.
        assert!(validate_url("http://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_url("http://metadata.google.internal/x").is_err());
        assert!(validate_url("http://100.100.100.200/x").is_err()); // Alibaba
                                                                    // IPv4-mapped IPv6 must NOT smuggle the link-local metadata target.
        assert!(validate_url("http://[::ffff:169.254.169.254]/latest").is_err());
        assert!(validate_url("http://[::ffff:a9fe:a9fe]/latest").is_err());
        // IPv6 loopback / link-local.
        assert!(validate_url("http://[::1]/x").is_err());
        assert!(validate_url("http://[fe80::1]/x").is_err());
        // Wrong schemes.
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("gopher://x").is_err());
    }

    #[test]
    fn unsupported_transports_are_honestly_refused() {
        let yaml = r#"
schemaVersion: "1.0"
id: desk
capabilities:
  - kind: actuator
    id: buzz
    transport: websocket
    request: { url: "http://192.168.1.9/x" }
"#;
        let err = parse_spec(yaml).unwrap_err();
        assert!(err.contains("NOT supported in this build"));
    }

    #[test]
    fn link_transports_validate_identity_command_and_facts() {
        // serial actuator 少 command → 拒。
        let missing_command = r#"
schemaVersion: "1.0"
id: esp32-desk
capabilities:
  - kind: actuator
    id: led
    transport: serial
    serial: { port: "/dev/cu.usbmodem1", expectedDeviceId: "esp32-desk01" }
"#;
        let err = parse_spec(missing_command).unwrap_err();
        assert!(err.contains("needs a `command`"), "{err}");

        // 空 expectedDeviceId → 拒（埠不是身分）。
        let empty_identity = r#"
schemaVersion: "1.0"
id: esp32-desk
capabilities:
  - kind: actuator
    id: led
    transport: serial
    command: { name: "led.set" }
    serial: { port: "/dev/cu.usbmodem1", expectedDeviceId: "  " }
"#;
        let err = parse_spec(empty_identity).unwrap_err();
        assert!(err.contains("never an identity"), "{err}");

        // receptor 少 facts → 拒。
        let missing_facts = r#"
schemaVersion: "1.0"
id: esp32-desk
capabilities:
  - kind: receptor
    id: env
    transport: mqtt
    mqtt: { brokerHost: "127.0.0.1", topicPrefix: "companion/desk", expectedDeviceId: "esp32-desk01" }
"#;
        let err = parse_spec(missing_facts).unwrap_err();
        assert!(err.contains("facts"), "{err}");

        // 完整 serial＋mqtt spec → 建立成功（有 transport-serial/mqtt features）。
        let ok = r#"
schemaVersion: "1.0"
id: esp32-desk
capabilities:
  - kind: actuator
    id: led
    channel: light
    transport: serial
    command:
      name: "led.set"
      params: { r: "{{magnitude}}", g: 0, b: 64 }
    serial: { port: "/dev/cu.usbmodem1", baud: 115200, expectedDeviceId: "esp32-desk01" }
  - kind: receptor
    id: env
    transport: serial
    facts:
      lux: "/facts/lux"
      distanceMm: "/facts/distanceMm"
    serial: { port: "/dev/cu.usbmodem1", baud: 115200, expectedDeviceId: "esp32-desk01" }
"#;
        let spec = parse_spec(ok).unwrap();
        let built = build(&spec, None).unwrap();
        assert_eq!(built.receptors.len(), 1);
        assert_eq!(built.actuators.len(), 1);
        let m = built.actuators[0].manifest();
        assert!(m.requires_consent, "device output must be consent-gated");

        // ble：壞 UUID → 拒。
        let bad_uuid = r#"
schemaVersion: "1.0"
id: esp32-desk
capabilities:
  - kind: actuator
    id: led
    transport: ble
    command: { name: "led.set" }
    ble:
      deviceName: "esp32-companion"
      serviceUuid: "not-a-uuid"
      commandCharUuid: "6e400002-b5a3-f393-e0a9-e50e24dcca9e"
      stateCharUuid: "6e400003-b5a3-f393-e0a9-e50e24dcca9e"
      expectedDeviceId: "esp32-desk01"
"#;
        let err = parse_spec(bad_uuid).unwrap_err();
        // 有 BLE 的平台（macOS／Windows＋transport-ble）拒的是壞 UUID；
        // 沒有 BLE 的平台（Linux CI）必須誠實拒絕「此 build 不支援」，兩者都算通過。
        assert!(
            err.contains("UUID") || err.contains("NOT supported in this build"),
            "{err}"
        );

        // 同一 adapter 兩個不同 serial port → 拒（一個 adapter＝一台裝置）。
        let two_devices = r#"
schemaVersion: "1.0"
id: esp32-desk
capabilities:
  - kind: actuator
    id: led
    transport: serial
    command: { name: "led.set" }
    serial: { port: "/dev/cu.usbmodemA", expectedDeviceId: "esp32-a" }
  - kind: actuator
    id: buzz
    transport: serial
    command: { name: "buzzer.beep" }
    serial: { port: "/dev/cu.usbmodemB", expectedDeviceId: "esp32-b" }
"#;
        let spec = parse_spec(two_devices).unwrap();
        let err = match build(&spec, None) {
            Err(e) => e,
            Ok(_) => panic!("two devices in one adapter must be refused"),
        };
        assert!(err.contains("same"), "{err}");
    }

    #[test]
    fn substitution_uses_only_bounded_effective_values() {
        use interaction_core::*;
        let now = Utc::now();
        let action = BoundedAction {
            action_id: ActionId::new("action-1"),
            plan_id: PlanId::new("plan-1"),
            session_id: SessionId::new("sess-1"),
            actuator_id: ActuatorId::new("desk.set"),
            intent: "calm".into(),
            risk_class: RiskClass::BoundedSideEffect,
            requested: ActionParameters {
                magnitude: Some(1.0), // what the AI asked for
                ..Default::default()
            },
            effective: ActionParameters {
                magnitude: Some(0.3), // what policy allowed
                duration_ms: Some(1200),
                message: Some("hi".into()),
                extra: None,
            },
            policy_decisions: vec![],
            expires_at: now + chrono::Duration::minutes(1),
            issued_at: now,
            correlation_id: CorrelationId::new("c1"),
            metadata: Default::default(),
            schema_version: "1.0".into(),
        };
        let template = json!({
            "level": "{{magnitude}}",
            "note": "intent={{intent}} for {{durationMs}}ms",
        });
        let out = substitute(&template, &action);
        // Exactly-a-placeholder becomes a NUMBER with the BOUNDED value.
        assert_eq!(out["level"], json!(0.3));
        assert_eq!(out["note"], json!("intent=calm for 1200ms"));
    }

    #[test]
    fn secret_references_resolve_from_env_and_never_echo() {
        std::env::set_var("INTERACT_AI_SECRET_DESK_TOKEN", "s3cr3t");
        assert_eq!(
            resolve_secret("secret://desk-token", None).unwrap(),
            "s3cr3t"
        );
        std::env::remove_var("INTERACT_AI_SECRET_DESK_TOKEN");
        let err = resolve_secret("secret://missing-one", None).unwrap_err();
        assert!(err.contains("missing-one"));
        // Plain values pass through untouched.
        assert_eq!(resolve_secret("Bearer abc", None).unwrap(), "Bearer abc");
    }

    #[test]
    fn spec_parses_full_example() {
        let yaml = r#"
schemaVersion: "1.0"
id: desk-light
displayName: 書桌燈
provider:
  id: provider.device.desk-01
  kind: device
  displayName: 書桌互動裝置
capabilities:
  - kind: receptor
    id: status
    transport: http
    request: { method: GET, url: "http://192.168.1.50/status" }
    pollIntervalMs: 30000
    facts:
      "on": "/power"
      brightness: "/brightness"
  - kind: actuator
    id: set
    channel: light
    transport: http
    confirmation: acknowledged
    request:
      method: POST
      url: "http://192.168.1.50/set"
      headers: { Authorization: "secret://desk-light-token" }
      body: { brightness: "{{magnitude}}" }
"#;
        let spec = parse_spec(yaml).unwrap();
        assert_eq!(spec.capabilities.len(), 2);
        let built = build(&spec, None).unwrap();
        assert_eq!(built.receptors.len(), 1);
        assert_eq!(built.actuators.len(), 1);
        let m = built.actuators[0].manifest();
        assert_eq!(m.id.as_str(), "desk-light.set");
        assert!(m.requires_consent, "device output must be consent-gated");
    }
}
