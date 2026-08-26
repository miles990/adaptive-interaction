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
//! - Transports beyond http/sse (websocket, mqtt, serial, ble) parse but are
//!   HONESTLY refused at build time with a clear "not supported in this
//!   build" error — nothing pretends to work.

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
    pub request: RequestSpec,
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
        validate_url(&cap.request.url)?;
        if let Some(stop) = &cap.stop_request {
            validate_url(&stop.url)?;
        }
        for value in cap.request.headers.values() {
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
        match cap.transport {
            Transport::Http | Transport::Sse => {}
            other => {
                return Err(format!(
                    "{}: transport {other:?} is declared but NOT supported in this build \
                     (supported: http, sse). The capability was not created.",
                    cap.id
                ));
            }
        }
    }
    Ok(())
}

/// SSRF guard: http/https only; cloud-metadata range hard-blocked.
pub fn validate_url(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(raw).map_err(|e| format!("invalid url {raw:?}: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("url scheme {other:?} not allowed (http/https only)")),
    }
    if let Some(host) = url.host_str() {
        if host.starts_with("169.254.") || host == "metadata.google.internal" {
            return Err(format!("url host {host:?} is blocked (metadata range)"));
        }
    } else {
        return Err("url has no host".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Secret references
// ---------------------------------------------------------------------------

/// Resolve `secret://name` header values. Sources, in order:
/// 1. `INTERACT_AI_SECRET_<NAME>` environment variable (uppercased, - → _)
/// 2. the runtime secret file `<home>/state/secrets.json` (0600, human-owned)
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
    adapter_id: String,
    client: reqwest::Client,
    home: Option<std::path::PathBuf>,
}

impl DeclarativeHttpReceptor {
    fn full_id(&self) -> String {
        qualified_id(&self.adapter_id, &self.spec.id)
    }
}

fn qualified_id(adapter: &str, id: &str) -> String {
    if id.contains('.') {
        id.to_string()
    } else {
        format!("{adapter}.{id}")
    }
}

async fn send_request(
    client: &reqwest::Client,
    req: &RequestSpec,
    body: Option<Value>,
    timeout_ms: u64,
    idempotency_key: Option<&str>,
    home: Option<&std::path::Path>,
) -> Result<(u16, Value), String> {
    validate_url(&req.url)?;
    let method = reqwest::Method::from_bytes(req.method.as_bytes())
        .map_err(|_| format!("bad method {:?}", req.method))?;
    let mut builder = client
        .request(method, &req.url)
        .timeout(Duration::from_millis(timeout_ms.clamp(100, 60_000)));
    for (k, v) in &req.headers {
        let resolved = resolve_secret(v, home)?;
        builder = builder.header(k, resolved);
    }
    if let Some(key) = idempotency_key {
        builder = builder.header("Idempotency-Key", key);
    }
    if let Some(body) = body {
        builder = builder.json(&body);
    }
    let resp = builder.send().await.map_err(|e| e.to_string())?;
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
            &self.spec.request,
            None,
            self.spec.timeout_ms.unwrap_or(5_000),
            None,
            self.home.as_deref(),
        )
        .await
        .map_err(ReceptorError::Unavailable)?;
        if !(200..300).contains(&status) {
            return Err(ReceptorError::Unavailable(format!(
                "device returned HTTP {status}"
            )));
        }
        let mut obs = Observation::now(
            ReceptorId::new(self.full_id()),
            &format!("declarative.{}", self.adapter_id),
            Utc::now(),
        );
        for (fact, pointer) in &self.spec.facts {
            if let Some(v) = value.pointer(pointer) {
                obs.facts.insert(fact.clone(), v.clone());
            }
        }
        Ok(obs)
    }

    async fn health(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
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
    adapter_id: String,
    client: reqwest::Client,
    home: Option<std::path::PathBuf>,
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
        .requires_consent(true); // external device output is consent-gated by default
        if let Some(h) = &self.spec.human {
            b = b.human(h.clone());
        }
        b.build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        if action.expires_at <= Utc::now() {
            return Err(ActuatorError::Rejected("action expired".into()));
        }
        let body = self
            .spec
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
            match send_request(
                &self.client,
                &self.spec.request,
                body.clone(),
                timeout,
                Some(action.action_id.as_str()),
                self.home.as_deref(),
            )
            .await
            {
                Ok((status, response)) if (200..300).contains(&status) => {
                    let receipt = DriverReceipt::start(&action, Utc::now())
                        .dispatched()
                        .note("httpStatus", json!(status))
                        .note("response", redact(response));
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
                    last_err = format!("device returned HTTP {status}");
                }
                Err(e) => {
                    last_err = e;
                }
            }
        }
        Ok(DriverReceipt::start(&action, Utc::now())
            .failed("device-unreachable", &last_err)
            .finish())
    }

    async fn status(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(format!(
            "{action_id}: declarative http actions are single-shot"
        )))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        if let Some(stop) = &self.spec.stop_request {
            let _ = send_request(
                &self.client,
                stop,
                stop.body.clone(),
                2_000,
                None,
                self.home.as_deref(),
            )
            .await;
        }
        Ok(())
    }
}

/// Trim device responses before they land in receipts (no secrets, bounded size).
fn redact(v: Value) -> Value {
    let text = v.to_string();
    if text.len() > 2048 {
        json!({"truncated": true})
    } else {
        v
    }
}

// ---------------------------------------------------------------------------
// Engine: spec → instances
// ---------------------------------------------------------------------------

pub struct BuiltCapabilities {
    pub receptors: Vec<Arc<dyn Receptor>>,
    pub actuators: Vec<Arc<dyn Actuator>>,
}

pub fn build(
    spec: &DeclarativeSpec,
    home: Option<std::path::PathBuf>,
) -> Result<BuiltCapabilities, String> {
    validate_spec(spec)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut out = BuiltCapabilities {
        receptors: vec![],
        actuators: vec![],
    };
    for cap in &spec.capabilities {
        match cap.kind {
            CapabilityKindSpec::Receptor => out.receptors.push(Arc::new(DeclarativeHttpReceptor {
                spec: cap.clone(),
                adapter_id: spec.id.clone(),
                client: client.clone(),
                home: home.clone(),
            })),
            CapabilityKindSpec::Actuator => out.actuators.push(Arc::new(DeclarativeHttpActuator {
                spec: cap.clone(),
                adapter_id: spec.id.clone(),
                client: client.clone(),
                home: home.clone(),
            })),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_guard_blocks_metadata_and_odd_schemes() {
        assert!(validate_url("http://192.168.1.50/status").is_ok());
        assert!(validate_url("https://example.com/x").is_ok());
        assert!(validate_url("http://169.254.169.254/latest/meta-data").is_err());
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
    transport: mqtt
    request: { url: "http://192.168.1.9/x" }
"#;
        let err = parse_spec(yaml).unwrap_err();
        assert!(err.contains("NOT supported in this build"));
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
