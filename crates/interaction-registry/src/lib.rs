//! Dynamic capability registry.
//!
//! Holds live receptor/actuator instances plus tool-operation manifests,
//! tracks enable/disable state and health, and produces
//! [`CapabilitySnapshot`]s for planning. Registration is dynamic: adapters can
//! be added and removed at runtime without touching the orchestrator.

use interaction_core::{
    Actuator, ActuatorId, ActuatorManifest, Availability, CapabilityConstraint, CapabilitySnapshot,
    ComponentHealth, DiscoveryContext, DomainError, DomainResult, EventType, PolicyConfig,
    Receptor, ReceptorId, ReceptorManifest, ToolOperationManifest,
};
use interaction_events::EventBus;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

struct ReceptorEntry {
    instance: Arc<dyn Receptor>,
    enabled: bool,
    last_health: ComponentHealth,
}

struct ActuatorEntry {
    instance: Arc<dyn Actuator>,
    enabled: bool,
    last_health: ComponentHealth,
}

pub struct CapabilityRegistry {
    receptors: RwLock<BTreeMap<ReceptorId, ReceptorEntry>>,
    actuators: RwLock<BTreeMap<ActuatorId, ActuatorEntry>>,
    tools: RwLock<BTreeMap<String, ToolOperationManifest>>,
    version: AtomicU64,
    events: EventBus,
}

impl CapabilityRegistry {
    pub fn new(events: EventBus) -> Self {
        Self {
            receptors: RwLock::new(BTreeMap::new()),
            actuators: RwLock::new(BTreeMap::new()),
            tools: RwLock::new(BTreeMap::new()),
            version: AtomicU64::new(1),
            events,
        }
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::SeqCst);
        self.events.emit(EventType::CapabilityChanged, json!({}));
    }

    // ---- receptors ----

    pub async fn register_receptor(&self, receptor: Arc<dyn Receptor>) -> DomainResult<()> {
        let manifest = receptor.manifest();
        let id = manifest.id.clone();
        let mut map = self.receptors.write().await;
        if map.contains_key(&id) {
            return Err(DomainError::Conflict(format!(
                "receptor {id} already registered"
            )));
        }
        map.insert(
            id.clone(),
            ReceptorEntry {
                instance: receptor,
                // Sensitive receptors start disabled; policy also enforces this.
                enabled: !manifest.requires_consent,
                last_health: manifest.health.clone(),
            },
        );
        drop(map);
        self.events.emit(
            EventType::ReceptorRegistered,
            json!({ "receptorId": id.as_str() }),
        );
        self.bump();
        Ok(())
    }

    pub async fn unregister_receptor(&self, id: &ReceptorId) -> DomainResult<()> {
        let mut map = self.receptors.write().await;
        let entry = map
            .remove(id)
            .ok_or_else(|| DomainError::NotFound(format!("receptor {id}")))?;
        drop(map);
        let _ = entry.instance.stop().await;
        self.events.emit(
            EventType::ReceptorOffline,
            json!({ "receptorId": id.as_str() }),
        );
        self.bump();
        Ok(())
    }

    pub async fn set_receptor_enabled(&self, id: &ReceptorId, enabled: bool) -> DomainResult<()> {
        let mut map = self.receptors.write().await;
        let entry = map
            .get_mut(id)
            .ok_or_else(|| DomainError::NotFound(format!("receptor {id}")))?;
        entry.enabled = enabled;
        drop(map);
        let event = if enabled {
            EventType::ReceptorOnline
        } else {
            EventType::ReceptorOffline
        };
        self.events
            .emit(event, json!({ "receptorId": id.as_str() }));
        self.bump();
        Ok(())
    }

    pub async fn receptor(&self, id: &ReceptorId) -> DomainResult<Arc<dyn Receptor>> {
        let map = self.receptors.read().await;
        let entry = map
            .get(id)
            .ok_or_else(|| DomainError::NotFound(format!("receptor {id}")))?;
        if !entry.enabled {
            return Err(DomainError::Unavailable(format!(
                "receptor {id} is disabled"
            )));
        }
        Ok(entry.instance.clone())
    }

    /// Access even when disabled — used by inspect/test paths (still policied upstream).
    pub async fn receptor_any(&self, id: &ReceptorId) -> DomainResult<Arc<dyn Receptor>> {
        let map = self.receptors.read().await;
        map.get(id)
            .map(|e| e.instance.clone())
            .ok_or_else(|| DomainError::NotFound(format!("receptor {id}")))
    }

    pub async fn receptor_manifests(&self) -> Vec<ReceptorManifest> {
        let map = self.receptors.read().await;
        map.values()
            .map(|entry| {
                let mut m = entry.instance.manifest();
                m.health = entry.last_health.clone();
                m.availability = availability_of(entry.enabled, &entry.last_health);
                m
            })
            .collect()
    }

    // ---- actuators ----

    pub async fn register_actuator(&self, actuator: Arc<dyn Actuator>) -> DomainResult<()> {
        let manifest = actuator.manifest();
        let id = manifest.id.clone();
        let mut map = self.actuators.write().await;
        if map.contains_key(&id) {
            return Err(DomainError::Conflict(format!(
                "actuator {id} already registered"
            )));
        }
        // Physical / external-write actuators start disabled by default.
        let default_enabled = !manifest.external_side_effect
            && manifest.risk_class <= interaction_core::RiskClass::BoundedSideEffect
            && !manifest.requires_consent;
        map.insert(
            id.clone(),
            ActuatorEntry {
                instance: actuator,
                enabled: default_enabled,
                last_health: manifest.health.clone(),
            },
        );
        drop(map);
        self.events.emit(
            EventType::ActuatorRegistered,
            json!({ "actuatorId": id.as_str() }),
        );
        self.bump();
        Ok(())
    }

    pub async fn unregister_actuator(&self, id: &ActuatorId) -> DomainResult<()> {
        let mut map = self.actuators.write().await;
        let entry = map
            .remove(id)
            .ok_or_else(|| DomainError::NotFound(format!("actuator {id}")))?;
        drop(map);
        let _ = entry.instance.emergency_stop().await;
        self.events.emit(
            EventType::ActuatorOffline,
            json!({ "actuatorId": id.as_str() }),
        );
        self.bump();
        Ok(())
    }

    pub async fn set_actuator_enabled(&self, id: &ActuatorId, enabled: bool) -> DomainResult<()> {
        let mut map = self.actuators.write().await;
        let entry = map
            .get_mut(id)
            .ok_or_else(|| DomainError::NotFound(format!("actuator {id}")))?;
        entry.enabled = enabled;
        drop(map);
        let event = if enabled {
            EventType::ActuatorOnline
        } else {
            EventType::ActuatorOffline
        };
        self.events
            .emit(event, json!({ "actuatorId": id.as_str() }));
        self.bump();
        Ok(())
    }

    pub async fn actuator(&self, id: &ActuatorId) -> DomainResult<Arc<dyn Actuator>> {
        let map = self.actuators.read().await;
        let entry = map
            .get(id)
            .ok_or_else(|| DomainError::NotFound(format!("actuator {id}")))?;
        if !entry.enabled {
            return Err(DomainError::Unavailable(format!(
                "actuator {id} is disabled"
            )));
        }
        Ok(entry.instance.clone())
    }

    pub async fn actuator_any(&self, id: &ActuatorId) -> DomainResult<Arc<dyn Actuator>> {
        let map = self.actuators.read().await;
        map.get(id)
            .map(|e| e.instance.clone())
            .ok_or_else(|| DomainError::NotFound(format!("actuator {id}")))
    }

    /// All actuator instances regardless of enabled state — used by emergency stop.
    pub async fn all_actuator_instances(&self) -> Vec<Arc<dyn Actuator>> {
        let map = self.actuators.read().await;
        map.values().map(|e| e.instance.clone()).collect()
    }

    pub async fn actuator_manifests(&self) -> Vec<ActuatorManifest> {
        let map = self.actuators.read().await;
        map.values()
            .map(|entry| {
                let mut m = entry.instance.manifest();
                m.health = entry.last_health.clone();
                m.availability = availability_of(entry.enabled, &entry.last_health);
                m
            })
            .collect()
    }

    // ---- tools ----

    pub async fn register_tool_operation(
        &self,
        manifest: ToolOperationManifest,
    ) -> DomainResult<()> {
        let mut map = self.tools.write().await;
        if map.contains_key(&manifest.name) {
            return Err(DomainError::Conflict(format!(
                "tool operation {} already registered",
                manifest.name
            )));
        }
        map.insert(manifest.name.clone(), manifest);
        drop(map);
        self.bump();
        Ok(())
    }

    pub async fn tool_operations(&self) -> Vec<ToolOperationManifest> {
        self.tools.read().await.values().cloned().collect()
    }

    pub async fn tool_operation(&self, name: &str) -> DomainResult<ToolOperationManifest> {
        self.tools
            .read()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| DomainError::NotFound(format!("tool operation {name}")))
    }

    // ---- health & snapshot ----

    /// Refresh cached health for all components. Called periodically by the runtime.
    pub async fn refresh_health(&self) {
        let receptor_ids: Vec<ReceptorId> =
            { self.receptors.read().await.keys().cloned().collect() };
        for id in receptor_ids {
            let instance = {
                self.receptors
                    .read()
                    .await
                    .get(&id)
                    .map(|e| e.instance.clone())
            };
            if let Some(instance) = instance {
                let health = instance.health().await;
                let mut map = self.receptors.write().await;
                if let Some(entry) = map.get_mut(&id) {
                    entry.last_health = health;
                }
            }
        }
        let actuator_ids: Vec<ActuatorId> =
            { self.actuators.read().await.keys().cloned().collect() };
        for id in actuator_ids {
            let instance = {
                self.actuators
                    .read()
                    .await
                    .get(&id)
                    .map(|e| e.instance.clone())
            };
            if let Some(instance) = instance {
                let health = instance.status().await;
                let mut map = self.actuators.write().await;
                if let Some(entry) = map.get_mut(&id) {
                    entry.last_health = health;
                }
            }
        }
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    pub async fn snapshot(
        &self,
        context: &DiscoveryContext,
        policy: PolicyConfig,
        constraints: Vec<CapabilityConstraint>,
        now: interaction_core::Timestamp,
    ) -> CapabilitySnapshot {
        let mut receptors = self.receptor_manifests().await;
        let mut actuators = self.actuator_manifests().await;
        let tool_operations = self.tool_operations().await;
        if !context.include_unavailable {
            receptors.retain(|m| m.availability.is_available());
            actuators.retain(|m| m.availability.is_available());
        }
        CapabilitySnapshot {
            receptors,
            actuators,
            tool_operations,
            constraints,
            session_policy: policy,
            generated_at: now,
            version: self.version(),
            schema_version: interaction_core::SCHEMA_VERSION.to_string(),
        }
    }
}

fn availability_of(enabled: bool, health: &ComponentHealth) -> Availability {
    if !enabled {
        Availability::Disabled
    } else if !health.is_usable() {
        Availability::Offline
    } else {
        Availability::Available
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use interaction_core::*;

    struct FakeReceptor {
        id: &'static str,
        sensitive: bool,
    }

    #[async_trait]
    impl Receptor for FakeReceptor {
        fn manifest(&self) -> ReceptorManifest {
            ReceptorManifest {
                id: ReceptorId::new(self.id),
                name: self.id.to_string(),
                description: "fake".into(),
                category: "test".into(),
                provides: vec!["event".into()],
                mode: ReceptorMode::Poll,
                sensitivity: if self.sensitive {
                    Sensitivity::Intimate
                } else {
                    Sensitivity::Public
                },
                requires_consent: self.sensitive,
                latency_ms: None,
                refresh_interval_ms: None,
                config_schema: None,
                health: ComponentHealth::healthy(),
                availability: Availability::Available,
                driver: "test.fake".into(),
                version: "0.1.0".into(),
                schema_version: SCHEMA_VERSION.into(),
            }
        }
        async fn start(&self, _c: SessionContext) -> Result<(), ReceptorError> {
            Ok(())
        }
        async fn read(&self) -> Result<Observation, ReceptorError> {
            Ok(Observation::now(
                ReceptorId::new(self.id),
                "test.fake",
                chrono::Utc::now(),
            ))
        }
        async fn health(&self) -> ComponentHealth {
            ComponentHealth::healthy()
        }
        async fn stop(&self) -> Result<(), ReceptorError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn register_discover_disable() {
        let registry = CapabilityRegistry::new(EventBus::default());
        registry
            .register_receptor(Arc::new(FakeReceptor {
                id: "a",
                sensitive: false,
            }))
            .await
            .unwrap();
        registry
            .register_receptor(Arc::new(FakeReceptor {
                id: "cam",
                sensitive: true,
            }))
            .await
            .unwrap();

        // Duplicate registration is rejected.
        assert!(registry
            .register_receptor(Arc::new(FakeReceptor {
                id: "a",
                sensitive: false
            }))
            .await
            .is_err());

        let snap = registry
            .snapshot(
                &DiscoveryContext::default(),
                PolicyConfig::default(),
                vec![],
                chrono::Utc::now(),
            )
            .await;
        // Sensitive receptor starts disabled, so only "a" is visible by default.
        assert_eq!(snap.receptors.len(), 1);
        assert_eq!(snap.receptors[0].id.as_str(), "a");

        let all = registry
            .snapshot(
                &DiscoveryContext {
                    include_unavailable: true,
                    ..Default::default()
                },
                PolicyConfig::default(),
                vec![],
                chrono::Utc::now(),
            )
            .await;
        assert_eq!(all.receptors.len(), 2);
        let cam = all
            .receptors
            .iter()
            .find(|m| m.id.as_str() == "cam")
            .unwrap();
        assert_eq!(cam.availability, Availability::Disabled);

        // Disabled receptor cannot be fetched for planning.
        registry
            .set_receptor_enabled(&ReceptorId::new("a"), false)
            .await
            .unwrap();
        assert!(registry.receptor(&ReceptorId::new("a")).await.is_err());
        assert!(registry.receptor_any(&ReceptorId::new("a")).await.is_ok());

        // Version bumps on every change.
        assert!(registry.version() > 1);
    }
}
