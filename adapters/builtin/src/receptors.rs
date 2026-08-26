//! Built-in receptors.
//!
//! `PushReceptor` backs everything that receives events from outside
//! (session input, task lifecycle, agent activity, manual events, webhooks,
//! mock). `SystemTimeReceptor` derives wall-clock facts locally.

use async_trait::async_trait;
use chrono::{Datelike, Timelike, Utc};
use interaction_adapter_sdk::ReceptorManifestBuilder;
use interaction_core::{
    ComponentHealth, Observation, Receptor, ReceptorError, ReceptorId, ReceptorManifest,
    ReceptorMode, Sensitivity, SessionContext,
};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

const PUSH_BUFFER: usize = 128;

/// A receptor fed by external pushes (API/CLI/UI). Keeps a bounded buffer of
/// recent observations; `read` returns the most recent one.
pub struct PushReceptor {
    manifest: ReceptorManifest,
    buffer: Arc<Mutex<VecDeque<Observation>>>,
}

impl PushReceptor {
    pub fn new(manifest: ReceptorManifest) -> Arc<Self> {
        Arc::new(Self {
            manifest,
            buffer: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    /// Feed one observation built from facts/inferences.
    pub fn push(
        &self,
        facts: BTreeMap<String, Value>,
        inferences: BTreeMap<String, Value>,
        confidence: f64,
    ) -> Observation {
        let now = Utc::now();
        let mut obs = Observation::now(self.manifest.id.clone(), &self.manifest.driver, now);
        obs.facts = facts;
        obs.inferences = inferences;
        obs.confidence = confidence.clamp(0.0, 1.0);
        let mut buffer = self.buffer.lock().expect("push buffer lock");
        if buffer.len() >= PUSH_BUFFER {
            buffer.pop_front();
        }
        buffer.push_back(obs.clone());
        obs
    }

    pub fn recent(&self, limit: usize) -> Vec<Observation> {
        let buffer = self.buffer.lock().expect("push buffer lock");
        let skip = buffer.len().saturating_sub(limit);
        buffer.iter().skip(skip).cloned().collect()
    }

    pub fn id(&self) -> &ReceptorId {
        &self.manifest.id
    }
}

#[async_trait]
impl Receptor for PushReceptor {
    fn manifest(&self) -> ReceptorManifest {
        self.manifest.clone()
    }

    async fn start(&self, _context: SessionContext) -> Result<(), ReceptorError> {
        Ok(())
    }

    async fn read(&self) -> Result<Observation, ReceptorError> {
        self.buffer
            .lock()
            .expect("push buffer lock")
            .back()
            .cloned()
            .ok_or_else(|| ReceptorError::Unavailable("no observations yet".into()))
    }

    async fn health(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn stop(&self) -> Result<(), ReceptorError> {
        Ok(())
    }
}

/// Standard builtin push receptors.
pub fn builtin_push_receptors() -> Vec<Arc<PushReceptor>> {
    vec![
        PushReceptor::new(
            ReceptorManifestBuilder::new("session.input", "Session input", "builtin.push")
                .description("Explicit user text and commands within the session")
                .category("session")
                .provides(&["text", "command", "state"])
                .mode(ReceptorMode::Event)
                .sensitivity(Sensitivity::Personal, false)
                .human(interaction_adapter_sdk::data_semantics(
                    &["user-text", "commands"],
                    interaction_core::TriState::Yes,
                    interaction_core::DataSource::Local,
                ))
                .build(),
        ),
        PushReceptor::new(
            ReceptorManifestBuilder::new("task.lifecycle", "Task lifecycle", "builtin.push")
                .description("Task start/progress/completion events from agents or tools")
                .category("task")
                .provides(&["event", "taskId", "title"])
                .mode(ReceptorMode::Event)
                .sensitivity(Sensitivity::Internal, false)
                .human(interaction_adapter_sdk::data_semantics(
                    &["task-status"],
                    interaction_core::TriState::No,
                    interaction_core::DataSource::Local,
                ))
                .build(),
        ),
        PushReceptor::new(
            ReceptorManifestBuilder::new("agent.activity", "Agent activity", "builtin.push")
                .description("What the AI agent is currently doing")
                .category("agent")
                .provides(&["activity", "detail"])
                .mode(ReceptorMode::Event)
                .sensitivity(Sensitivity::Internal, false)
                .human(interaction_adapter_sdk::data_semantics(
                    &["agent-status"],
                    interaction_core::TriState::No,
                    interaction_core::DataSource::Local,
                ))
                .build(),
        ),
        PushReceptor::new(
            ReceptorManifestBuilder::new("manual.event", "Manual event", "builtin.push")
                .description("Human-injected events for testing and overrides")
                .category("manual")
                .provides(&["event"])
                .mode(ReceptorMode::Event)
                .sensitivity(Sensitivity::Internal, false)
                .human(interaction_adapter_sdk::data_semantics(
                    &["test-events"],
                    interaction_core::TriState::No,
                    interaction_core::DataSource::Local,
                ))
                .build(),
        ),
        PushReceptor::new(
            ReceptorManifestBuilder::new("webhook.input", "Webhook input", "builtin.push")
                .description("Observations delivered by external systems via the HTTP API")
                .category("integration")
                .provides(&["event", "payload"])
                .mode(ReceptorMode::Event)
                .sensitivity(Sensitivity::Internal, false)
                .human(interaction_adapter_sdk::data_semantics(
                    &["external-events"],
                    interaction_core::TriState::Unknown,
                    interaction_core::DataSource::ExternalService,
                ))
                .build(),
        ),
        PushReceptor::new(
            ReceptorManifestBuilder::new("user.presence", "User presence", "builtin.push")
                .description("Whether the user is present (explicit or agent-reported)")
                .category("session")
                .provides(&["state"])
                .mode(ReceptorMode::Event)
                .sensitivity(Sensitivity::Personal, false)
                .human(interaction_adapter_sdk::data_semantics(
                    &["presence"],
                    interaction_core::TriState::Yes,
                    interaction_core::DataSource::Local,
                ))
                .build(),
        ),
        // Desktop-companion semantic interaction events. ONLY semantic kinds
        // (clicked/dragged/dropped/action-selected/pointer-approached) plus
        // optional user text — raw pointer coordinates never reach this
        // receptor and are never persisted anywhere.
        PushReceptor::new(
            ReceptorManifestBuilder::new(
                "desktop.companion.interaction",
                "Desktop companion interaction",
                "builtin.push",
            )
            .description("Semantic interactions with the desktop companion (click, drag, drop, quick actions, text). No raw pointer coordinates.")
            .category("companion")
            .provides(&["kind", "modality", "text", "attachments"])
            .mode(ReceptorMode::Event)
            .sensitivity(Sensitivity::Personal, false)
            .human(interaction_adapter_sdk::data_semantics(
                &["companion-input"],
                interaction_core::TriState::Yes,
                interaction_core::DataSource::Local,
            ))
            .build(),
        ),
        // Coarse pointer-activity summary derived from THIS APP's windows
        // only (activeRecently / idleForMs). Honest limitation: it does not
        // observe other applications, and no positions are recorded.
        PushReceptor::new(
            ReceptorManifestBuilder::new(
                "desktop.pointer.activity",
                "Desktop pointer activity (summary)",
                "builtin.push",
            )
            .description("Summary of pointer activity within this app's windows only: activeRecently and idleForMs. No positions, no other apps.")
            .category("companion")
            .provides(&["activeRecently", "idleForMs"])
            .mode(ReceptorMode::Event)
            .sensitivity(Sensitivity::Internal, false)
            .human(interaction_adapter_sdk::data_semantics(
                &["activity-summary"],
                interaction_core::TriState::No,
                interaction_core::DataSource::Local,
            ))
            .build(),
        ),
        PushReceptor::new(
            ReceptorManifestBuilder::new("mock.receptor", "Mock receptor", "builtin.mock")
                .description("Scriptable receptor for tests and simulations")
                .category("mock")
                .provides(&["*"])
                .mode(ReceptorMode::Event)
                .sensitivity(Sensitivity::Public, false)
                .human(interaction_adapter_sdk::data_semantics(
                    &["test-data"],
                    interaction_core::TriState::No,
                    interaction_core::DataSource::Local,
                ))
                .build(),
        ),
    ]
}

/// Wall-clock receptor: hour, weekday, and a coarse day-phase fact.
pub struct SystemTimeReceptor;

#[async_trait]
impl Receptor for SystemTimeReceptor {
    fn manifest(&self) -> ReceptorManifest {
        ReceptorManifestBuilder::new("system.time", "System time", "builtin.system-time")
            .description("Local wall-clock time facts")
            .category("environment")
            .provides(&["hour", "minute", "weekday", "dayPhase", "iso"])
            .mode(ReceptorMode::Poll)
            .sensitivity(Sensitivity::Public, false)
            .refresh_interval_ms(30_000)
            .human(interaction_adapter_sdk::data_semantics(
                &["time"],
                interaction_core::TriState::No,
                interaction_core::DataSource::Local,
            ))
            .build()
    }

    async fn start(&self, _context: SessionContext) -> Result<(), ReceptorError> {
        Ok(())
    }

    async fn read(&self) -> Result<Observation, ReceptorError> {
        let now = Utc::now();
        let local = chrono::Local::now();
        let hour = local.hour();
        let phase = match hour {
            5..=11 => "morning",
            12..=17 => "afternoon",
            18..=22 => "evening",
            _ => "night",
        };
        Ok(
            Observation::now(ReceptorId::new("system.time"), "builtin.system-time", now)
                .with_fact("hour", hour)
                .with_fact("minute", local.minute())
                .with_fact("weekday", local.weekday().to_string())
                .with_fact("dayPhase", phase)
                .with_fact("iso", local.to_rfc3339()),
        )
    }

    async fn health(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn stop(&self) -> Result<(), ReceptorError> {
        Ok(())
    }
}

/// Device-status receptor paired with [`crate::MockActuator`]: reports what the
/// mock device actually executed, closing the act → observe loop.
pub struct MockDeviceStatusReceptor {
    id: String,
    pub state: Arc<Mutex<VecDeque<Observation>>>,
}

impl MockDeviceStatusReceptor {
    /// Builtin pairing (id `mock.device-status`).
    pub fn new(state: Arc<Mutex<VecDeque<Observation>>>) -> Self {
        Self::with_id("mock.device-status", state)
    }

    /// Pairing for dynamically registered mock devices.
    pub fn with_id(id: &str, state: Arc<Mutex<VecDeque<Observation>>>) -> Self {
        Self {
            id: id.to_string(),
            state,
        }
    }
}

#[async_trait]
impl Receptor for MockDeviceStatusReceptor {
    fn manifest(&self) -> ReceptorManifest {
        ReceptorManifestBuilder::new(&self.id, "Mock device status", "builtin.mock-device")
            .description("Observed state of the mock physical device")
            .category("device")
            .provides(&["actionId", "magnitude", "state"])
            .mode(ReceptorMode::Event)
            .sensitivity(Sensitivity::Public, false)
            .human(interaction_adapter_sdk::data_semantics(
                &["device-status"],
                interaction_core::TriState::No,
                interaction_core::DataSource::Device,
            ))
            .build()
    }

    async fn start(&self, _context: SessionContext) -> Result<(), ReceptorError> {
        Ok(())
    }

    async fn read(&self) -> Result<Observation, ReceptorError> {
        self.state
            .lock()
            .expect("mock device state lock")
            .back()
            .cloned()
            .ok_or_else(|| ReceptorError::Unavailable("device idle; no state yet".into()))
    }

    async fn health(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn stop(&self) -> Result<(), ReceptorError> {
        Ok(())
    }
}
