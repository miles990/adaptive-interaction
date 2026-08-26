//! Adapter SDK: helpers for writing receptors and actuators without touching
//! the orchestrator. Third-party drivers depend on this crate and
//! `interaction-core` only.

use chrono::Utc;
use interaction_core::{
    ActionReceipt, ActionStatus, ActuatorLimits, ActuatorManifest, Availability, BoundedAction,
    ComponentHealth, CostDescriptor, ReceptorId, ReceptorManifest, ReceptorMode, RiskClass,
    Sensitivity, Timestamp, SCHEMA_VERSION,
};

/// Fluent builder for receptor manifests with sane defaults.
pub struct ReceptorManifestBuilder {
    manifest: ReceptorManifest,
}

impl ReceptorManifestBuilder {
    pub fn new(id: &str, name: &str, driver: &str) -> Self {
        Self {
            manifest: ReceptorManifest {
                id: ReceptorId::new(id),
                name: name.to_string(),
                description: String::new(),
                category: "general".into(),
                provides: Vec::new(),
                mode: ReceptorMode::Poll,
                sensitivity: Sensitivity::Internal,
                requires_consent: false,
                latency_ms: None,
                refresh_interval_ms: None,
                config_schema: None,
                health: ComponentHealth::healthy(),
                availability: Availability::Available,
                driver: driver.to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                schema_version: SCHEMA_VERSION.to_string(),
                human: None,
            },
        }
    }

    pub fn description(mut self, d: &str) -> Self {
        self.manifest.description = d.to_string();
        self
    }
    pub fn category(mut self, c: &str) -> Self {
        self.manifest.category = c.to_string();
        self
    }
    pub fn provides(mut self, keys: &[&str]) -> Self {
        self.manifest.provides = keys.iter().map(|s| s.to_string()).collect();
        self
    }
    pub fn mode(mut self, mode: ReceptorMode) -> Self {
        self.manifest.mode = mode;
        self
    }
    pub fn sensitivity(mut self, s: Sensitivity, requires_consent: bool) -> Self {
        self.manifest.sensitivity = s;
        self.manifest.requires_consent = requires_consent;
        self
    }
    pub fn refresh_interval_ms(mut self, ms: u64) -> Self {
        self.manifest.refresh_interval_ms = Some(ms);
        self
    }
    /// Attach the formal human layer (data semantics / presentation).
    pub fn human(mut self, human: interaction_core::HumanMeta) -> Self {
        self.manifest.human = Some(human);
        self
    }

    pub fn build(self) -> ReceptorManifest {
        self.manifest
    }
}

/// Fluent builder for actuator manifests with safe defaults
/// (`risk = BoundedSideEffect`, no external side effect, reversible).
pub struct ActuatorManifestBuilder {
    manifest: ActuatorManifest,
}

impl ActuatorManifestBuilder {
    pub fn new(id: &str, name: &str, channel: &str, driver: &str) -> Self {
        Self {
            manifest: ActuatorManifest {
                id: interaction_core::ActuatorId::new(id),
                name: name.to_string(),
                description: String::new(),
                channel: channel.to_string(),
                capabilities: Vec::new(),
                parameters_schema: None,
                supports_cancel: false,
                supports_pattern: false,
                requires_consent: false,
                external_side_effect: false,
                reversible: true,
                risk_class: RiskClass::BoundedSideEffect,
                latency_ms: None,
                cost: CostDescriptor::default(),
                limits: ActuatorLimits::default(),
                health: ComponentHealth::healthy(),
                availability: Availability::Available,
                driver: driver.to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                schema_version: SCHEMA_VERSION.to_string(),
                human: None,
            },
        }
    }

    pub fn description(mut self, d: &str) -> Self {
        self.manifest.description = d.to_string();
        self
    }
    pub fn capabilities(mut self, caps: &[&str]) -> Self {
        self.manifest.capabilities = caps.iter().map(|s| s.to_string()).collect();
        self
    }
    pub fn risk(mut self, risk: RiskClass) -> Self {
        self.manifest.risk_class = risk;
        self
    }
    pub fn external(mut self, external: bool) -> Self {
        self.manifest.external_side_effect = external;
        self
    }
    pub fn requires_consent(mut self, v: bool) -> Self {
        self.manifest.requires_consent = v;
        self
    }
    pub fn supports_cancel(mut self, v: bool) -> Self {
        self.manifest.supports_cancel = v;
        self
    }
    pub fn supports_pattern(mut self, v: bool) -> Self {
        self.manifest.supports_pattern = v;
        self
    }
    pub fn limits(mut self, limits: ActuatorLimits) -> Self {
        self.manifest.limits = limits;
        self
    }
    pub fn cost(mut self, cost: CostDescriptor) -> Self {
        self.manifest.cost = cost;
        self
    }
    /// Attach the formal human layer (effect semantics / presentation).
    pub fn human(mut self, human: interaction_core::HumanMeta) -> Self {
        self.manifest.human = Some(human);
        self
    }

    pub fn build(self) -> ActuatorManifest {
        self.manifest
    }
}

/// Driver-side receipt protocol.
///
/// Drivers receive a [`BoundedAction`] and return an [`ActionReceipt`] that
/// reflects *what the driver actually knows*: `Dispatched` when the command
/// left the driver, `Acknowledged` only when the target confirmed it.
/// The runtime merges this into its own authoritative receipt.
pub struct DriverReceipt {
    receipt: ActionReceipt,
}

impl DriverReceipt {
    /// Start from the accepted state (the runtime already accepted the action).
    pub fn start(action: &BoundedAction, now: Timestamp) -> Self {
        let mut receipt = ActionReceipt::for_action(action, now);
        // Runtime accepted before calling the driver.
        let _ = receipt.transition(ActionStatus::Accepted, now);
        Self { receipt }
    }

    pub fn dispatched(mut self) -> Self {
        let _ = self
            .receipt
            .transition(ActionStatus::Dispatched, Utc::now());
        self
    }

    pub fn acknowledged(mut self) -> Self {
        let _ = self
            .receipt
            .transition(ActionStatus::Acknowledged, Utc::now());
        self
    }

    pub fn failed(mut self, code: &str, message: &str) -> Self {
        let now = Utc::now();
        self.receipt.push_error(code, message, now);
        let _ = self.receipt.transition(ActionStatus::Failed, now);
        self
    }

    pub fn note(mut self, key: &str, value: serde_json::Value) -> Self {
        self.receipt.driver_response.insert(key.to_string(), value);
        self
    }

    pub fn finish(self) -> ActionReceipt {
        self.receipt
    }
}

/// Merge a driver-produced receipt into the runtime's authoritative receipt.
/// Applies the driver's forward progress (statuses past `Accepted`) in order,
/// and copies driver responses / errors. Illegal transitions are ignored
/// rather than trusted.
pub fn merge_driver_receipt(base: &mut ActionReceipt, driver: &ActionReceipt) {
    for (status, at) in &driver.timestamps {
        if matches!(
            status,
            ActionStatus::Planned | ActionStatus::Authorized | ActionStatus::Accepted
        ) {
            continue;
        }
        if base.current_status.can_transition_to(*status) {
            let _ = base.transition(*status, *at);
        }
    }
    for (k, v) in &driver.driver_response {
        base.driver_response.insert(k.clone(), v.clone());
    }
    base.errors.extend(driver.errors.iter().cloned());
}

/// Formal declaration for a receptor whose data is produced and stays on this
/// machine (stored in the local state DB until pruned/deleted).
pub fn local_data_semantics() -> interaction_core::HumanMeta {
    use interaction_core::*;
    HumanMeta {
        data: Some(DataSemantics {
            source: DataSource::Local,
            leaves_device: TriState::No,
            retention: DataRetention::Persistent,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Full formal data declaration for a receptor: categories, whether the data
/// is personal, and where it comes from. Local + never-leaves-device +
/// persisted unless stated otherwise.
pub fn data_semantics(
    categories: &[&str],
    personal: interaction_core::TriState,
    source: interaction_core::DataSource,
) -> interaction_core::HumanMeta {
    use interaction_core::*;
    HumanMeta {
        data: Some(DataSemantics {
            data_categories: categories.iter().map(|s| s.to_string()).collect(),
            personal_data: personal,
            source,
            leaves_device: TriState::No,
            retention: DataRetention::Persistent,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Formal declaration for a local-only actuator: how disruptive it is and the
/// deepest delivery level the driver can honestly confirm.
pub fn local_effect_semantics(
    interruptiveness: interaction_core::Interruptiveness,
    confirmation: interaction_core::ConfirmationLevel,
) -> interaction_core::HumanMeta {
    use interaction_core::*;
    HumanMeta {
        effect: Some(EffectSemantics {
            external_side_effect: TriState::No,
            physical_effect: TriState::No,
            interruptiveness,
            reversible: TriState::Unknown,
            confirmation_level: confirmation,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interaction_core::*;

    fn action() -> BoundedAction {
        let now = Utc::now();
        BoundedAction {
            action_id: ActionId::generate(),
            plan_id: PlanId::generate(),
            session_id: SessionId::generate(),
            actuator_id: ActuatorId::new("mock"),
            intent: "test".into(),
            risk_class: RiskClass::Low,
            requested: ActionParameters::default(),
            effective: ActionParameters::default(),
            policy_decisions: vec![],
            expires_at: now + chrono::Duration::seconds(10),
            issued_at: now,
            correlation_id: CorrelationId::generate(),
            metadata: Default::default(),
            schema_version: SCHEMA_VERSION.into(),
        }
    }

    #[test]
    fn driver_receipt_protocol() {
        let a = action();
        let receipt = DriverReceipt::start(&a, Utc::now())
            .dispatched()
            .acknowledged()
            .note("queue", serde_json::json!("q1"))
            .finish();
        assert_eq!(receipt.current_status, ActionStatus::Acknowledged);
    }

    #[test]
    fn merge_applies_forward_progress_only() {
        let a = action();
        let now = Utc::now();
        let mut base = ActionReceipt::for_action(&a, now);
        base.transition(ActionStatus::Accepted, now).unwrap();
        let driver = DriverReceipt::start(&a, now)
            .dispatched()
            .acknowledged()
            .finish();
        merge_driver_receipt(&mut base, &driver);
        assert_eq!(base.current_status, ActionStatus::Acknowledged);
        // Merge again: no duplicate transitions possible.
        merge_driver_receipt(&mut base, &driver);
        assert_eq!(base.current_status, ActionStatus::Acknowledged);
    }

    #[test]
    fn merge_does_not_trust_driver_completed_claim() {
        let a = action();
        let now = Utc::now();
        let mut base = ActionReceipt::for_action(&a, now);
        base.transition(ActionStatus::Accepted, now).unwrap();
        // Malicious/buggy driver claims Completed straight from Accepted.
        let mut fake = ActionReceipt::for_action(&a, now);
        fake.timestamps.push((ActionStatus::Completed, now));
        merge_driver_receipt(&mut base, &fake);
        assert_ne!(base.current_status, ActionStatus::Completed);
    }
}
