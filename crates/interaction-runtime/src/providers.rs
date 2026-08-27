//! Provider lifecycle in the runtime: builtin provider registration,
//! declarative adapter loading (config/adapters/*.yaml), pairing, revocation
//! and persistence. Pairing / install / enable / consent stay separate steps.

use crate::runtime::Runtime;
use interaction_core::{
    ActuatorId, DomainError, DomainResult, ProviderDescriptor, ProviderId, ProviderIdentity,
    ProviderKind, ProviderState, ReceptorId, TrustLevel,
};
use interaction_registry::providers::discovered;
use sha2::{Digest, Sha256};

impl Runtime {
    /// Called once at startup: builtin provider + persisted providers +
    /// declarative adapters from `config/adapters/*.yaml`.
    pub(crate) async fn init_providers(&self) {
        // 1) Builtin local provider (trust: builtin, always available).
        let builtin = ProviderDescriptor {
            identity: ProviderIdentity {
                id: ProviderId::new("provider.local.builtin"),
                kind: ProviderKind::Local,
                display_name: "內建本機能力".into(),
                trust_level: TrustLevel::Builtin,
                origin: "builtin".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                fingerprint: None,
                human: None,
            },
            state: ProviderState::Available,
            receptors: self
                .registry
                .receptor_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| !id.starts_with("companion."))
                .collect(),
            actuators: self
                .registry
                .actuator_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| !id.starts_with("companion."))
                .collect(),
            tool_operations: self
                .registry
                .tool_operations()
                .await
                .into_iter()
                .map(|m| m.name)
                .collect(),
            paired_at: None,
            last_seen: Some(chrono::Utc::now()),
            detail: None,
        };
        let _ = self.providers.register(builtin).await;

        // 1.5) Presentation Provider：桌面角色（小樞）是一級 provider，能力
        //      逐項宣告。信任層級 Builtin（本應用自帶的表面），但可用性誠實
        //      跟隨視窗 presence（receptor/actuator 健康由 bridge 決定）。
        let companion = ProviderDescriptor {
            identity: ProviderIdentity {
                id: ProviderId::new("provider.companion.shu"),
                kind: ProviderKind::Companion,
                display_name: "桌面角色小樞（Presentation）".into(),
                trust_level: TrustLevel::Builtin,
                origin: "builtin.presentation".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                fingerprint: None,
                human: None,
            },
            state: ProviderState::Available,
            receptors: self
                .registry
                .receptor_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| id.starts_with("companion."))
                .collect(),
            actuators: self
                .registry
                .actuator_manifests()
                .await
                .into_iter()
                .map(|m| m.id.as_str().to_string())
                .filter(|id| id.starts_with("companion."))
                .collect(),
            tool_operations: vec![],
            paired_at: None,
            last_seen: Some(chrono::Utc::now()),
            detail: Some("能力逐項授權；隱藏角色只停用視窗內能力，不影響 Runtime".into()),
        };
        let _ = self.providers.register(companion).await;

        // 2) Persisted provider records (paired devices etc.).
        //    Crash/restart must NOT auto-recover an operational device: pairing
        //    survives, but any provider left Available/Busy/Degraded comes back
        //    Disabled so a physical device never re-arms itself on its own.
        if let Ok(bodies) = self.store.all_providers() {
            for body in bodies {
                if let Ok(mut desc) = serde_json::from_str::<ProviderDescriptor>(&body) {
                    if desc.state.is_operational() {
                        desc.state = ProviderState::Disabled;
                        desc.detail = Some("re-armed on restart requires explicit enable".into());
                        if let Ok(b) = serde_json::to_string(&desc) {
                            let _ = self.store.save_provider(desc.identity.id.as_str(), &b);
                        }
                    }
                    let _ = self.providers.register(desc).await;
                }
            }
        }

        // 3) Declarative adapters (File=Truth; human-owned specs).
        let dir = self.paths.home.join("config").join("adapters");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_yaml = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e, "yaml" | "yml" | "json"))
                .unwrap_or(false);
            if !is_yaml {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "adapter spec unreadable");
                    continue;
                }
            };
            match interaction_adapter_declarative::parse_spec(&text) {
                Ok(spec) => {
                    if let Err(e) = self.register_declarative_spec(&spec).await {
                        tracing::warn!(path = %path.display(), error = %e, "adapter spec rejected");
                    }
                }
                Err(e) => {
                    // Invalid spec never crashes the runtime; it is surfaced.
                    tracing::warn!(path = %path.display(), error = %e, "adapter spec invalid");
                }
            }
        }
    }

    /// Register one declarative spec's capabilities + provider record.
    pub async fn register_declarative_spec(
        &self,
        spec: &interaction_adapter_declarative::DeclarativeSpec,
    ) -> DomainResult<()> {
        let built = interaction_adapter_declarative::build(spec, Some(self.paths.home.clone()))
            .map_err(DomainError::Validation)?;
        let mut receptor_ids = Vec::new();
        let mut actuator_ids = Vec::new();
        for receptor in built.receptors {
            receptor_ids.push(receptor.manifest().id.as_str().to_string());
            self.registry.register_receptor(receptor).await?;
        }
        for actuator in built.actuators {
            actuator_ids.push(actuator.manifest().id.as_str().to_string());
            self.registry.register_actuator(actuator).await?;
        }

        let identity = spec.provider.clone().unwrap_or(ProviderIdentity {
            id: ProviderId::new(format!("provider.adapter.{}", spec.id)),
            kind: ProviderKind::Device,
            display_name: spec.display_name.clone().unwrap_or_else(|| spec.id.clone()),
            trust_level: TrustLevel::Untrusted,
            origin: "config/adapters".into(),
            version: String::new(),
            fingerprint: None,
            human: None,
        });
        let provider_id = identity.id.clone();

        // A persisted record (e.g. already paired) wins over a fresh one.
        if let Ok(existing) = self.providers.get(&provider_id).await {
            let _ = existing;
            self.providers
                .attach_capabilities(&provider_id, receptor_ids, actuator_ids, vec![])
                .await?;
            return Ok(());
        }
        let mut desc = discovered(identity);
        // Declared in human-owned config = installed; still DISABLED and
        // consent-gated until the human enables each capability.
        desc.state = ProviderState::Installed;
        desc.receptors = receptor_ids;
        desc.actuators = actuator_ids;
        self.providers.register(desc.clone()).await?;
        self.persist_provider(&provider_id).await;
        Ok(())
    }

    pub async fn list_providers(&self) -> Vec<ProviderDescriptor> {
        self.providers.list().await
    }

    pub async fn get_provider(&self, id: &ProviderId) -> DomainResult<ProviderDescriptor> {
        self.providers.get(id).await
    }

    /// Pairing ceremony (shared-code): the human enters the code the device
    /// shows. The stored fingerprint mixes a fresh random per-pairing secret so
    /// it is NOT a pure function of the public provider id + code (which anyone
    /// could recompute); an IP address is never an identity. Transitions →
    /// Paired.
    ///
    /// Honest scope note: this records that a human completed a pairing with a
    /// secret. Enforcing device identity on every request (rejecting an
    /// imposter at the same address) needs device-side crypto that HTTP/mock
    /// devices in this build do not present — see docs/ARCHITECTURE.md.
    pub async fn pair_provider(
        &self,
        id: &ProviderId,
        pairing_code: &str,
    ) -> DomainResult<ProviderDescriptor> {
        if pairing_code.trim().len() < 4 {
            return Err(DomainError::Validation(
                "pairing code must be at least 4 characters".into(),
            ));
        }
        // Random per-pairing secret: makes the fingerprint unforgeable from the
        // public (id, code) pair.
        let salt = uuid::Uuid::new_v4();
        let mut hasher = Sha256::new();
        hasher.update(id.as_str().as_bytes());
        hasher.update(b":");
        hasher.update(pairing_code.trim().as_bytes());
        hasher.update(b":");
        hasher.update(salt.as_bytes());
        let fingerprint = hex_lower(&hasher.finalize());

        // discovered/unpaired → paired (the registry refuses shortcuts).
        let desc = self.providers.get(id).await?;
        if desc.state == ProviderState::Discovered {
            self.providers
                .transition(id, ProviderState::Unpaired, None)
                .await?;
        }
        let mut updated = self
            .providers
            .transition(id, ProviderState::Paired, Some("paired via code".into()))
            .await?;
        updated.identity.fingerprint = Some(fingerprint.clone());
        updated.identity.trust_level = TrustLevel::Paired;
        // Write the fingerprint back into the registry record.
        self.providers.remove(id).await.ok();
        self.providers.register(updated.clone()).await?;
        self.persist_provider(id).await;
        self.store.audit(
            "provider.paired",
            "user",
            &serde_json::json!({"providerId": id.as_str(), "fingerprint": fingerprint}),
        )?;
        Ok(updated)
    }

    /// Explicit lifecycle transition (install/enable/disable…), persisted.
    pub async fn transition_provider(
        &self,
        id: &ProviderId,
        state: ProviderState,
    ) -> DomainResult<ProviderDescriptor> {
        let desc = self.providers.transition(id, state, None).await?;
        self.persist_provider(id).await;
        Ok(desc)
    }

    /// Revoke: capabilities disabled immediately, state sticks at Revoked.
    pub async fn revoke_provider(&self, id: &ProviderId) -> DomainResult<ProviderDescriptor> {
        let desc = self
            .providers
            .transition(id, ProviderState::Revoked, Some("revoked by user".into()))
            .await?;
        for rid in &desc.receptors {
            let _ = self
                .registry
                .set_receptor_enabled(&ReceptorId::new(rid), false)
                .await;
        }
        for aid in &desc.actuators {
            let _ = self
                .registry
                .set_actuator_enabled(&ActuatorId::new(aid), false)
                .await;
        }
        self.persist_provider(id).await;
        self.store.audit(
            "provider.revoked",
            "user",
            &serde_json::json!({"providerId": id.as_str()}),
        )?;
        Ok(desc)
    }

    async fn persist_provider(&self, id: &ProviderId) {
        if let Ok(desc) = self.providers.get(id).await {
            if desc.identity.trust_level == TrustLevel::Builtin {
                return; // builtin is reconstructed each start
            }
            if let Ok(body) = serde_json::to_string(&desc) {
                let _ = self.store.save_provider(id.as_str(), &body);
            }
        } else {
            let _ = self.store.delete_provider(id.as_str());
        }
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
