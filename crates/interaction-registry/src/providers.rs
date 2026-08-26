//! Provider registry: who provides which capabilities, with an explicit,
//! deterministic lifecycle (discovered → paired → installed → disabled →
//! enabled). Shortcut transitions are refused here — not in the UI.

use interaction_core::{
    DomainError, DomainResult, EventType, ProviderDescriptor, ProviderId, ProviderIdentity,
    ProviderState,
};
use interaction_events::EventBus;
use serde_json::json;
use std::collections::BTreeMap;
use tokio::sync::RwLock;

pub struct ProviderRegistry {
    providers: RwLock<BTreeMap<ProviderId, ProviderDescriptor>>,
    events: EventBus,
}

impl ProviderRegistry {
    pub fn new(events: EventBus) -> Self {
        Self {
            providers: RwLock::new(BTreeMap::new()),
            events,
        }
    }

    /// Register a newly discovered/declared provider in its given state.
    pub async fn register(&self, descriptor: ProviderDescriptor) -> DomainResult<()> {
        let id = descriptor.identity.id.clone();
        let mut map = self.providers.write().await;
        if map.contains_key(&id) {
            return Err(DomainError::Conflict(format!(
                "provider {id} already registered"
            )));
        }
        map.insert(id.clone(), descriptor);
        drop(map);
        self.events.emit(
            EventType::ProviderRegistered,
            json!({ "providerId": id.as_str() }),
        );
        Ok(())
    }

    /// Transition lifecycle state; refuses illegal shortcuts.
    pub async fn transition(
        &self,
        id: &ProviderId,
        next: ProviderState,
        detail: Option<String>,
    ) -> DomainResult<ProviderDescriptor> {
        let mut map = self.providers.write().await;
        let entry = map
            .get_mut(id)
            .ok_or_else(|| DomainError::NotFound(format!("provider {id}")))?;
        if !entry.state.can_transition_to(next) {
            return Err(DomainError::Validation(format!(
                "provider {id}: illegal lifecycle transition {:?} → {next:?} \
                 (pairing, install, enable and consent are separate steps)",
                entry.state
            )));
        }
        entry.state = next;
        entry.detail = detail;
        if next == ProviderState::Paired {
            entry.paired_at = Some(chrono::Utc::now());
        }
        entry.last_seen = Some(chrono::Utc::now());
        let snapshot = entry.clone();
        drop(map);
        self.events.emit(
            EventType::ProviderStateChanged,
            json!({ "providerId": id.as_str(), "state": next }),
        );
        Ok(snapshot)
    }

    /// Record which capability ids a provider contributes.
    pub async fn attach_capabilities(
        &self,
        id: &ProviderId,
        receptors: Vec<String>,
        actuators: Vec<String>,
        tool_operations: Vec<String>,
    ) -> DomainResult<()> {
        let mut map = self.providers.write().await;
        let entry = map
            .get_mut(id)
            .ok_or_else(|| DomainError::NotFound(format!("provider {id}")))?;
        entry.receptors = receptors;
        entry.actuators = actuators;
        entry.tool_operations = tool_operations;
        Ok(())
    }

    pub async fn get(&self, id: &ProviderId) -> DomainResult<ProviderDescriptor> {
        self.providers
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::NotFound(format!("provider {id}")))
    }

    pub async fn list(&self) -> Vec<ProviderDescriptor> {
        self.providers.read().await.values().cloned().collect()
    }

    pub async fn remove(&self, id: &ProviderId) -> DomainResult<ProviderDescriptor> {
        let mut map = self.providers.write().await;
        let entry = map
            .remove(id)
            .ok_or_else(|| DomainError::NotFound(format!("provider {id}")))?;
        drop(map);
        self.events.emit(
            EventType::ProviderStateChanged,
            json!({ "providerId": id.as_str(), "state": "closed", "removed": true }),
        );
        Ok(entry)
    }

    /// Mark seen (heartbeat) without a state change.
    pub async fn touch(&self, id: &ProviderId) {
        if let Some(e) = self.providers.write().await.get_mut(id) {
            e.last_seen = Some(chrono::Utc::now());
        }
    }
}

/// Convenience: descriptor for a freshly discovered provider.
pub fn discovered(identity: ProviderIdentity) -> ProviderDescriptor {
    ProviderDescriptor {
        identity,
        state: ProviderState::Discovered,
        receptors: vec![],
        actuators: vec![],
        tool_operations: vec![],
        paired_at: None,
        last_seen: Some(chrono::Utc::now()),
        detail: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interaction_core::{ProviderKind, TrustLevel};

    fn identity(id: &str) -> ProviderIdentity {
        ProviderIdentity {
            id: ProviderId::new(id),
            kind: ProviderKind::Device,
            display_name: "測試裝置".into(),
            trust_level: TrustLevel::Discovered,
            origin: "test".into(),
            version: "1".into(),
            fingerprint: None,
            human: None,
        }
    }

    #[tokio::test]
    async fn full_lifecycle_and_shortcut_refusal() {
        let reg = ProviderRegistry::new(EventBus::new(64));
        let id = ProviderId::new("provider.device.t1");
        reg.register(discovered(identity("provider.device.t1")))
            .await
            .unwrap();

        // Shortcut discovered → available refused with a human-readable reason.
        let err = reg
            .transition(&id, ProviderState::Available, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("separate steps"));

        // The mandated ceremony works step by step.
        reg.transition(&id, ProviderState::Paired, None).await.unwrap();
        reg.transition(&id, ProviderState::Installed, None)
            .await
            .unwrap();
        let d = reg
            .transition(&id, ProviderState::Disabled, None)
            .await
            .unwrap();
        assert!(d.paired_at.is_some());
        reg.transition(&id, ProviderState::Available, None)
            .await
            .unwrap();

        // Revocation sticks: no way back to available.
        reg.transition(&id, ProviderState::Revoked, Some("user revoked".into()))
            .await
            .unwrap();
        assert!(reg
            .transition(&id, ProviderState::Available, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn duplicate_registration_conflicts() {
        let reg = ProviderRegistry::new(EventBus::new(64));
        reg.register(discovered(identity("provider.x"))).await.unwrap();
        assert!(reg.register(discovered(identity("provider.x"))).await.is_err());
    }
}
