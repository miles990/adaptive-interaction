//! Built-in actuators. Local, low-risk channels are enabled by default;
//! anything that leaves the machine (webhook) or simulates a physical device
//! (mock haptic) starts disabled and consent-gated per policy defaults.

use crate::outbox::{Outbox, OutboxMessage};
use async_trait::async_trait;
use chrono::Utc;
use interaction_adapter_sdk::{ActuatorManifestBuilder, DriverReceipt};
use interaction_core::{
    ActionId, ActionReceipt, Actuator, ActuatorError, ActuatorLimits, ActuatorManifest,
    BoundedAction, ComponentHealth, Observation, ReceptorId, RiskClass,
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn expired_check(action: &BoundedAction) -> Result<(), ActuatorError> {
    if action.is_expired(Utc::now()) {
        return Err(ActuatorError::Expired);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Conversation
// ---------------------------------------------------------------------------

/// Default actuator: renders text into the shared outbox (and the log).
pub struct ConversationActuator {
    outbox: Outbox,
}

impl ConversationActuator {
    pub fn new(outbox: Outbox) -> Self {
        Self { outbox }
    }
}

#[async_trait]
impl Actuator for ConversationActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new(
            "conversation",
            "Conversation",
            "conversation",
            "builtin.conversation",
        )
        .description("Concise textual responses rendered in the conversation surface")
        .capabilities(&["text", "silence"])
        .risk(RiskClass::Low)
        .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        expired_check(&action)?;
        let text = action.effective.message.clone();
        self.outbox.push(OutboxMessage {
            channel: "conversation".into(),
            intent: action.intent.clone(),
            text: text.clone(),
            action_id: action.action_id.clone(),
            at: Utc::now(),
        });
        if let Some(t) = &text {
            tracing::info!(target: "interaction.conversation", intent = %action.intent, "{t}");
        }
        Ok(DriverReceipt::start(&action, Utc::now())
            .dispatched()
            .acknowledged()
            .note("rendered", json!(text.is_some()))
            .finish())
    }

    async fn status(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(format!(
            "{action_id}: conversation output cannot be recalled"
        )))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Web UI feed
// ---------------------------------------------------------------------------

/// Pushes cards into the web-ui feed (rendered by the desktop app / web view).
pub struct WebUiActuator {
    outbox: Outbox,
}

impl WebUiActuator {
    pub fn new(outbox: Outbox) -> Self {
        Self { outbox }
    }
}

#[async_trait]
impl Actuator for WebUiActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new("web-ui", "Web UI feed", "web-ui", "builtin.web-ui")
            .description("Non-interrupting cards in the control-center feed")
            .capabilities(&["text", "card"])
            .risk(RiskClass::Low)
            .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        expired_check(&action)?;
        self.outbox.push(OutboxMessage {
            channel: "web-ui".into(),
            intent: action.intent.clone(),
            text: action.effective.message.clone(),
            action_id: action.action_id.clone(),
            at: Utc::now(),
        });
        Ok(DriverReceipt::start(&action, Utc::now())
            .dispatched()
            .acknowledged()
            .finish())
    }

    async fn status(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(format!(
            "{action_id}: web-ui cards are instantaneous"
        )))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Local log
// ---------------------------------------------------------------------------

pub struct LocalLogActuator;

#[async_trait]
impl Actuator for LocalLogActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new("local-log", "Local log", "log", "builtin.local-log")
            .description("Structured log line on the local machine")
            .capabilities(&["text"])
            .risk(RiskClass::Low)
            .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        expired_check(&action)?;
        tracing::info!(
            target: "interaction.local_log",
            intent = %action.intent,
            action_id = %action.action_id,
            message = action.effective.message.as_deref().unwrap_or("(silent)"),
            "interaction"
        );
        Ok(DriverReceipt::start(&action, Utc::now())
            .dispatched()
            .acknowledged()
            .finish())
    }

    async fn status(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(format!(
            "{action_id}: log lines cannot be cancelled"
        )))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Local notification (best effort, platform tools; degrades gracefully)
// ---------------------------------------------------------------------------

pub struct LocalNotificationActuator;

impl LocalNotificationActuator {
    async fn send(title: &str, body: &str) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            let script = format!(
                "display notification \"{}\" with title \"{}\"",
                body.replace('"', "'"),
                title.replace('"', "'")
            );
            let out = tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).to_string())
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let out = tokio::process::Command::new("notify-send")
                .arg(title)
                .arg(body)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).to_string())
            }
        }
        #[cfg(windows)]
        {
            let _ = (title, body);
            Err("local notifications not implemented on this platform".to_string())
        }
    }
}

#[async_trait]
impl Actuator for LocalNotificationActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new(
            "local-notification",
            "Local notification",
            "notification",
            "builtin.local-notification",
        )
        .description("Operating-system notification on the local machine")
        .capabilities(&["text"])
        .risk(RiskClass::BoundedSideEffect)
        .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        expired_check(&action)?;
        let title = "Adaptive Interaction";
        let body = action
            .effective
            .message
            .clone()
            .unwrap_or_else(|| action.intent.clone());
        let receipt = DriverReceipt::start(&action, Utc::now()).dispatched();
        match tokio::time::timeout(std::time::Duration::from_secs(5), Self::send(title, &body))
            .await
        {
            Ok(Ok(())) => Ok(receipt.acknowledged().finish()),
            Ok(Err(e)) => Ok(receipt.failed("notification_failed", &e).finish()),
            Err(_) => Ok(receipt
                .failed("notification_timeout", "5s timeout")
                .finish()),
        }
    }

    async fn status(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(format!(
            "{action_id}: notifications cannot be recalled"
        )))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Webhook output (external side effect — disabled by default, URL allowlist)
// ---------------------------------------------------------------------------

pub struct WebhookActuator {
    /// Explicit target allowlist from human-owned config. Empty = nothing allowed.
    allowed_urls: Vec<String>,
    client: reqwest::Client,
}

impl WebhookActuator {
    pub fn new(allowed_urls: Vec<String>) -> Self {
        Self {
            allowed_urls,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl Actuator for WebhookActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new(
            "webhook.output",
            "Webhook output",
            "webhook",
            "builtin.webhook",
        )
        .description("HTTP POST to a pre-approved URL from the human-owned config")
        .capabilities(&["json"])
        .risk(RiskClass::ExternalWrite)
        .external(true)
        .requires_consent(true)
        .limits(ActuatorLimits {
            max_payload_bytes: Some(64 * 1024),
            ..Default::default()
        })
        .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        expired_check(&action)?;
        let url = action
            .effective
            .extra
            .as_ref()
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| ActuatorError::Rejected("missing extra.url".into()))?
            .to_string();
        // SSRF guard: only explicitly allowlisted URLs, only http(s).
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ActuatorError::Rejected(
                "only http(s) URLs are allowed".into(),
            ));
        }
        if !self
            .allowed_urls
            .iter()
            .any(|allowed| url.starts_with(allowed))
        {
            return Err(ActuatorError::Rejected(format!(
                "url not in the configured allowlist ({} entries)",
                self.allowed_urls.len()
            )));
        }
        let body = json!({
            "intent": action.intent,
            "message": action.effective.message,
            "actionId": action.action_id.as_str(),
            "sessionId": action.session_id.as_str(),
        });
        let receipt = DriverReceipt::start(&action, Utc::now()).dispatched();
        match self.client.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => Ok(receipt
                .acknowledged()
                .note("httpStatus", json!(resp.status().as_u16()))
                .finish()),
            Ok(resp) => Ok(receipt
                .failed("webhook_http_error", &format!("status {}", resp.status()))
                .finish()),
            Err(e) => Ok(receipt.failed("webhook_transport", &e.to_string()).finish()),
        }
    }

    async fn status(&self) -> ComponentHealth {
        if self.allowed_urls.is_empty() {
            ComponentHealth::degraded("no allowlisted URLs configured").at(Utc::now())
        } else {
            ComponentHealth::healthy().at(Utc::now())
        }
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(format!(
            "{action_id}: webhook posts cannot be recalled"
        )))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock actuator (simulated physical device with observable state)
// ---------------------------------------------------------------------------

/// Bounded push shared by the mock actuator's bookkeeping vectors.
fn push_bounded<T>(store: &Arc<Mutex<Vec<T>>>, value: T) {
    let mut guard = store.lock().expect("mock store lock");
    if guard.len() >= 256 {
        guard.remove(0);
    }
    guard.push(value);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockBehavior {
    /// Dispatch + acknowledge + write observable device state.
    Normal,
    /// Accept and dispatch but never acknowledge (silent device).
    NoAck,
    /// Fail on execute.
    Fail,
    /// Simulate an offline device.
    Offline,
}

/// Simulated physical device (default channel: haptic). Execution writes its
/// state into a shared buffer that `mock.device-status` reads, closing the
/// act → observe loop without hardware.
pub struct MockActuator {
    channel: String,
    id: String,
    behavior: Arc<Mutex<MockBehavior>>,
    pub executed: Arc<Mutex<Vec<BoundedAction>>>,
    pub cancelled: Arc<Mutex<Vec<ActionId>>>,
    stopped: Arc<AtomicBool>,
    device_state: Arc<Mutex<VecDeque<Observation>>>,
}

impl MockActuator {
    pub fn new(id: &str, channel: &str) -> Self {
        Self {
            channel: channel.to_string(),
            id: id.to_string(),
            behavior: Arc::new(Mutex::new(MockBehavior::Normal)),
            executed: Arc::new(Mutex::new(Vec::new())),
            cancelled: Arc::new(Mutex::new(Vec::new())),
            stopped: Arc::new(AtomicBool::new(false)),
            device_state: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn set_behavior(&self, behavior: MockBehavior) {
        *self.behavior.lock().expect("behavior lock") = behavior;
    }

    /// Shared state buffer for pairing with [`crate::MockDeviceStatusReceptor`].
    pub fn device_state(&self) -> Arc<Mutex<VecDeque<Observation>>> {
        self.device_state.clone()
    }

    pub fn was_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    fn record_device_state(&self, action: &BoundedAction) {
        let obs = Observation::now(
            ReceptorId::new("mock.device-status"),
            "builtin.mock-device",
            Utc::now(),
        )
        .with_fact("actionId", action.action_id.as_str())
        .with_fact("magnitude", action.effective.magnitude.unwrap_or(0.0))
        .with_fact("state", "executed");
        let mut q = self.device_state.lock().expect("device state lock");
        if q.len() >= 64 {
            q.pop_front();
        }
        q.push_back(obs);
    }
}

#[async_trait]
impl Actuator for MockActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new(
            &self.id,
            "Mock device",
            &self.channel,
            "builtin.mock-actuator",
        )
        .description("Simulated physical device with observable state")
        .capabilities(&["magnitude", "pattern", "cancel"])
        .risk(RiskClass::BoundedSideEffect)
        .requires_consent(true)
        .supports_cancel(true)
        .supports_pattern(true)
        .limits(ActuatorLimits {
            max_magnitude: Some(0.8),
            max_duration_ms: Some(10_000),
            max_pattern_steps: Some(32),
            ..Default::default()
        })
        .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(ActuatorError::Unavailable(
                "device is emergency-stopped".into(),
            ));
        }
        expired_check(&action)?;
        let behavior = *self.behavior.lock().expect("behavior lock");
        match behavior {
            MockBehavior::Offline => Err(ActuatorError::Unavailable("device offline".into())),
            MockBehavior::Fail => Ok(DriverReceipt::start(&action, Utc::now())
                .dispatched()
                .failed("device_error", "simulated failure")
                .finish()),
            MockBehavior::NoAck => {
                self.executed
                    .lock()
                    .expect("executed lock")
                    .push(action.clone());
                Ok(DriverReceipt::start(&action, Utc::now())
                    .dispatched()
                    .note("note", json!("dispatched but device stayed silent"))
                    .finish())
            }
            MockBehavior::Normal => {
                self.executed
                    .lock()
                    .expect("executed lock")
                    .push(action.clone());
                self.record_device_state(&action);
                Ok(DriverReceipt::start(&action, Utc::now())
                    .dispatched()
                    .acknowledged()
                    .note("device", json!({"id": self.id, "channel": self.channel}))
                    .finish())
            }
        }
    }

    async fn status(&self) -> ComponentHealth {
        let behavior = *self.behavior.lock().expect("behavior lock");
        match behavior {
            MockBehavior::Offline => ComponentHealth::offline("simulated offline").at(Utc::now()),
            _ => ComponentHealth::healthy().at(Utc::now()),
        }
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        push_bounded(&self.cancelled, action_id.clone());
        Err(ActuatorError::NotFound(format!(
            "{action_id}: mock device has no long-running action to cancel"
        )))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        self.stopped.store(true, Ordering::SeqCst);
        let obs = Observation::now(
            ReceptorId::new("mock.device-status"),
            "builtin.mock-device",
            Utc::now(),
        )
        .with_fact("state", "stopped");
        self.device_state
            .lock()
            .expect("device state lock")
            .push_back(obs);
        Ok(())
    }
}
