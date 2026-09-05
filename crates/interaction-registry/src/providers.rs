//! Provider registry: who provides which capabilities, with an explicit,
//! deterministic lifecycle (discovered → paired → installed → disabled →
//! enabled). Shortcut transitions are refused here — not in the UI.

use interaction_core::{
    DomainError, DomainResult, EventType, ProviderDescriptor, ProviderId, ProviderIdentity,
    ProviderState,
};
use interaction_events::EventBus;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 「這個 provider 已經被停下來了」的狀態集合：人類按下停用／撤銷、租約到
/// 期、連線關閉。處在這些狀態的 provider 不得再觀察或派工。
///
/// 為什麼不是 `!ProviderState::is_operational()`：`Installed`／`Paired` 這類
/// 「還沒啟用」的狀態同樣不是 operational，但宣告式裝置正常就停在 `Installed`
/// （授權是逐能力的 enable，不是 provider 狀態），把它們一起擋掉會讓所有
/// 設定檔裝置直接失效。`Disconnected` 也不在這裡：那是連線的事實，由健康度
/// 閘門處理，不是誰做的決定。
pub fn provider_stopped(state: ProviderState) -> bool {
    matches!(
        state,
        ProviderState::Disabled
            | ProviderState::Expired
            | ProviderState::Revoked
            | ProviderState::Closed
    )
}

/// 一個能力被它的 provider 擋下來的理由（誰、什麼狀態）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBlock {
    pub provider: ProviderId,
    pub state: ProviderState,
}

impl ProviderBlock {
    /// 人話理由，形狀比照 registry 的 `… is disabled`。
    pub fn reason(&self, capability: &str) -> String {
        let state = format!("{:?}", self.state).to_lowercase();
        let provider = self.provider.as_str();
        format!("{capability} is disabled: its provider {provider} is {state}")
    }
}

/// capability → provider 的反向索引＋每個 provider 的狀態，供執行期閘門與
/// 能力清單投影查詢（同步、無 await，可以在持有其他鎖時查）。
///
/// 一個能力可以由多個 provider 提供（例如所有已配對的 iPhone 共用同一組
/// 動作能力、每個 AI session 都掛著 `agent.delegate`）。只有在**所有**擁有
/// 者都被停下來時才擋：還有一個活著的提供者就仍然做得到。沒有任何 provider
/// 記錄的能力（內建、動態註冊）視為可用——這個閘門只收緊，不改變既有預設。
#[derive(Default)]
pub struct ProviderGate {
    inner: std::sync::RwLock<GateInner>,
}

#[derive(Default)]
struct GateInner {
    states: BTreeMap<ProviderId, ProviderState>,
    receptors: BTreeMap<String, BTreeSet<ProviderId>>,
    actuators: BTreeMap<String, BTreeSet<ProviderId>>,
}

impl GateInner {
    fn detach(&mut self, id: &ProviderId) {
        for owners in self.receptors.values_mut() {
            owners.remove(id);
        }
        for owners in self.actuators.values_mut() {
            owners.remove(id);
        }
        self.receptors.retain(|_, owners| !owners.is_empty());
        self.actuators.retain(|_, owners| !owners.is_empty());
    }

    fn attach(&mut self, id: &ProviderId, receptors: &[String], actuators: &[String]) {
        self.detach(id);
        for rid in receptors {
            self.receptors
                .entry(rid.clone())
                .or_default()
                .insert(id.clone());
        }
        for aid in actuators {
            self.actuators
                .entry(aid.clone())
                .or_default()
                .insert(id.clone());
        }
    }

    fn block(
        &self,
        index: &BTreeMap<String, BTreeSet<ProviderId>>,
        capability: &str,
    ) -> Option<ProviderBlock> {
        let owners = index.get(capability)?;
        let mut blocked: Option<ProviderBlock> = None;
        for owner in owners {
            let state = self.states.get(owner).copied()?;
            if !provider_stopped(state) {
                return None; // 還有一個活著的提供者。
            }
            if blocked.is_none() {
                blocked = Some(ProviderBlock {
                    provider: owner.clone(),
                    state,
                });
            }
        }
        blocked
    }
}

impl ProviderGate {
    /// 這個受器的所有 provider 都被停下來了嗎？是的話回傳理由。
    pub fn receptor_block(&self, id: &str) -> Option<ProviderBlock> {
        let inner = self.inner.read().ok()?;
        inner.block(&inner.receptors, id)
    }

    /// 這個動器的所有 provider 都被停下來了嗎？是的話回傳理由。
    pub fn actuator_block(&self, id: &str) -> Option<ProviderBlock> {
        let inner = self.inner.read().ok()?;
        inner.block(&inner.actuators, id)
    }

    /// 這個能力登記在哪些 provider 底下（沒有記錄＝空）。
    pub fn providers_of_receptor(&self, id: &str) -> Vec<ProviderId> {
        self.inner
            .read()
            .ok()
            .and_then(|inner| inner.receptors.get(id).cloned())
            .map(|owners| owners.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn providers_of_actuator(&self, id: &str) -> Vec<ProviderId> {
        self.inner
            .read()
            .ok()
            .and_then(|inner| inner.actuators.get(id).cloned())
            .map(|owners| owners.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn state_of(&self, id: &ProviderId) -> Option<ProviderState> {
        self.inner.read().ok()?.states.get(id).copied()
    }

    fn upsert(&self, descriptor: &ProviderDescriptor) {
        if let Ok(mut inner) = self.inner.write() {
            let id = descriptor.identity.id.clone();
            inner.states.insert(id.clone(), descriptor.state);
            inner.attach(&id, &descriptor.receptors, &descriptor.actuators);
        }
    }

    fn set_state(&self, id: &ProviderId, state: ProviderState) {
        if let Ok(mut inner) = self.inner.write() {
            inner.states.insert(id.clone(), state);
        }
    }

    fn set_capabilities(&self, id: &ProviderId, receptors: &[String], actuators: &[String]) {
        if let Ok(mut inner) = self.inner.write() {
            inner.attach(id, receptors, actuators);
        }
    }

    fn forget(&self, id: &ProviderId) {
        if let Ok(mut inner) = self.inner.write() {
            inner.states.remove(id);
            inner.detach(id);
        }
    }
}

pub struct ProviderRegistry {
    providers: RwLock<BTreeMap<ProviderId, ProviderDescriptor>>,
    gate: Arc<ProviderGate>,
    events: EventBus,
    /// 每個 provider id 一把序列化鎖。
    ///
    /// 為什麼需要它：[`ProviderRegistry::transition`] 這一步本身是原子的，
    /// 但呼叫端做的是一整段**複合**動作（換狀態 → 關連線 → 請來源停止 →
    /// 翻能力旗標 → 落地 → 重新綁定），中間有很多 await。兩個並行的決定
    /// （例如「停用」與「重新綁定」）會在那些 await 點交錯，於是同一台裝置
    /// 可能被下架兩次，或是剛建好的新連線被上一個決定漏掉而繼續活著。
    /// 這把鎖讓「對同一台 provider 的一次完整決定」不可分割。
    locks: std::sync::Mutex<BTreeMap<ProviderId, Arc<tokio::sync::Mutex<()>>>>,
}

impl ProviderRegistry {
    pub fn new(events: EventBus) -> Self {
        Self {
            providers: RwLock::new(BTreeMap::new()),
            gate: Arc::new(ProviderGate::default()),
            events,
            locks: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// 取得這個 provider 的序列化鎖。同一個 id 的複合決定一次只跑一個。
    ///
    /// 有界：只有**還被持有或還在等待**的鎖留在表上（每次取用順手清掉沒人
    /// 用的），所以這張表不會隨著歷史 provider 無界成長。
    pub async fn lock_provider(&self, id: &ProviderId) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut map = match self.locks.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            map.retain(|key, value| key == id || Arc::strong_count(value) > 1);
            map.entry(id.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }

    /// 執行期閘門用的投影（capability → provider 狀態）。
    pub fn gate(&self) -> Arc<ProviderGate> {
        self.gate.clone()
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
        self.gate.upsert(&descriptor);
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
        self.gate.set_state(id, next);
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
        self.gate
            .set_capabilities(id, &entry.receptors, &entry.actuators);
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
        self.gate.forget(id);
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
        reg.transition(&id, ProviderState::Paired, None)
            .await
            .unwrap();
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

    /// 共用能力（多台 iPhone／多個 AI session 掛同一組能力）：只有在
    /// **所有**擁有者都被停下來時才擋，而且沒有 provider 記錄的能力不受影響。
    #[tokio::test]
    async fn gate_blocks_only_when_every_owner_is_stopped() {
        let reg = ProviderRegistry::new(EventBus::new(64));
        let gate = reg.gate();
        let a = ProviderId::new("provider.device.a");
        let b = ProviderId::new("provider.device.b");
        for id in [&a, &b] {
            let mut desc = discovered(identity(id.as_str()));
            desc.receptors = vec!["shared.status".into()];
            desc.actuators = vec!["shared.set".into()];
            desc.state = ProviderState::Installed;
            reg.register(desc).await.unwrap();
        }

        // 沒有 provider 記錄的能力永遠不擋（內建／動態註冊）。
        assert!(gate.receptor_block("builtin.clock").is_none());
        assert!(gate.actuator_block("conversation").is_none());
        // 都還在：不擋。
        assert!(gate.receptor_block("shared.status").is_none());
        assert_eq!(gate.providers_of_actuator("shared.set").len(), 2);

        // 只停掉一台：另一台還提供得了。
        reg.transition(&a, ProviderState::Disabled, None)
            .await
            .unwrap();
        assert_eq!(gate.state_of(&a), Some(ProviderState::Disabled));
        assert!(gate.receptor_block("shared.status").is_none());
        assert!(gate.actuator_block("shared.set").is_none());

        // 兩台都停：擋，而且理由點名 provider 與狀態。
        reg.transition(&b, ProviderState::Revoked, None)
            .await
            .unwrap();
        let block = gate.receptor_block("shared.status").expect("blocked");
        assert!(provider_stopped(block.state));
        let reason = block.reason("receptor shared.status");
        assert!(reason.contains("receptor shared.status"), "{reason}");
        assert!(
            reason.contains("provider.device.a") || reason.contains("provider.device.b"),
            "{reason}"
        );
        assert!(gate.actuator_block("shared.set").is_some());

        // 移除記錄＝不再有擁有者：能力回到「沒有 provider 記錄」。
        reg.remove(&a).await.unwrap();
        reg.remove(&b).await.unwrap();
        assert!(gate.receptor_block("shared.status").is_none());
        assert!(gate.state_of(&a).is_none());
    }

    /// 「還沒啟用」不等於「被停下來」：宣告式裝置正常停在 Installed，
    /// 授權是逐能力的 enable，閘門不得把它們一起擋掉。
    #[test]
    fn only_stopped_states_close_the_gate() {
        use ProviderState::*;
        for s in [Disabled, Expired, Revoked, Closed] {
            assert!(provider_stopped(s), "{s:?} 必須擋");
        }
        for s in [
            Discovered,
            Unpaired,
            Paired,
            Installed,
            Available,
            Busy,
            Degraded,
            Disconnected,
        ] {
            assert!(!provider_stopped(s), "{s:?} 不得被 provider 閘門擋掉");
        }
    }

    /// attach_capabilities 換掉能力清單時，反向索引不得留下舊的擁有關係。
    #[tokio::test]
    async fn attaching_capabilities_replaces_the_reverse_index() {
        let reg = ProviderRegistry::new(EventBus::new(64));
        let gate = reg.gate();
        let id = ProviderId::new("provider.device.swap");
        let mut desc = discovered(identity("provider.device.swap"));
        desc.receptors = vec!["old.status".into()];
        reg.register(desc).await.unwrap();
        reg.attach_capabilities(&id, vec!["new.status".into()], vec![], vec![])
            .await
            .unwrap();
        reg.transition(&id, ProviderState::Paired, None)
            .await
            .unwrap();
        reg.transition(&id, ProviderState::Revoked, None)
            .await
            .unwrap();
        assert!(gate.receptor_block("old.status").is_none());
        assert!(gate.receptor_block("new.status").is_some());
    }

    #[tokio::test]
    async fn duplicate_registration_conflicts() {
        let reg = ProviderRegistry::new(EventBus::new(64));
        reg.register(discovered(identity("provider.x")))
            .await
            .unwrap();
        assert!(reg
            .register(discovered(identity("provider.x")))
            .await
            .is_err());
    }
}
