//! The runtime facade: one set of application services shared by the CLI
//! daemon, HTTP API and Tauri shell. Cheap to clone (Arc inner).

use crate::config::{ConfigService, Paths, RuntimeConfig};
use crate::lock::InstanceLock;
use crate::orchestrator::{build_plan, ActuatorUsageHint, PlanRequest};
use crate::text::TextSelector;
use adapters_builtin::{
    builtin_push_receptors, ConversationActuator, LocalLogActuator, LocalNotificationActuator,
    MockActuator, MockDeviceStatusReceptor, Outbox, OutboxMessage, PushReceptor,
    SystemTimeReceptor, WebUiActuator, WebhookActuator,
};
use chrono::Utc;
use interaction_core::{
    ActionId, ActionReceipt, ActionStatus, CapabilityConstraint, CapabilitySnapshot, ConsentScope,
    DiscoveryContext, DomainError, DomainResult, EventType, MessageStrategy, Observation,
    ObservationQuery, Plan, PlanId, PlanStatus, PolicyConfig, ReceptorId, RuntimeEvent,
    SemanticIntent, Session, SessionId, Timestamp,
};
use interaction_events::EventBus;
use interaction_policy::ActionSource;
use interaction_recipe::{evaluate_trigger, Recipe, TriggerDecision};
use interaction_registry::CapabilityRegistry;
use interaction_storage::Store;
use rand::Rng;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Default)]
pub struct RuntimeOptions {
    pub home: Option<PathBuf>,
    /// Acquire the single-instance lock (daemon mode). Tests may skip it.
    pub acquire_lock: bool,
    pub in_memory_db: bool,
    pub spawn_watchdog: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RecipeState {
    pub last_fired_at: Option<Timestamp>,
    pub executions_this_session: u32,
    pub fired_last_hour: Vec<Timestamp>,
    /// Observation ids already consumed by a firing; a consumed event can
    /// never satisfy this recipe's trigger again (bounded FIFO).
    pub consumed_observations: Vec<String>,
}

pub struct RecipeEntry {
    pub recipe: Recipe,
    pub state: RecipeState,
}

pub struct RuntimeInner {
    pub paths: Paths,
    pub config_service: ConfigService,
    pub config: RwLock<RuntimeConfig>,
    pub policy_config: RwLock<PolicyConfig>,
    pub registry: CapabilityRegistry,
    pub providers: interaction_registry::providers::ProviderRegistry,
    pub store: Store,
    pub events: EventBus,
    pub outbox: Outbox,
    pub texts: TextSelector,
    estop: AtomicBool,
    pub push_receptors: BTreeMap<String, Arc<PushReceptor>>,
    dynamic_push: RwLock<BTreeMap<String, Arc<PushReceptor>>>,
    pub mock_actuator: Arc<MockActuator>,
    pub recipes: RwLock<BTreeMap<String, RecipeEntry>>,
    pub recipe_errors: RwLock<Vec<(PathBuf, String)>>,
    session: RwLock<Option<Session>>,
    pub shutdown_token: CancellationToken,
    pub started_at: Timestamp,
    lock: std::sync::Mutex<Option<InstanceLock>>,
    /// Config load errors surfaced in status (last-known-good semantics).
    pub config_errors: Vec<String>,
    /// Proactive-interaction pause: a normal user control, deliberately a
    /// DIFFERENT state from emergency stop (which is a safety mechanism).
    pub(crate) pause: RwLock<crate::human::PauseState>,
    /// Pending AI assist requests awaiting an external AI host (bounded).
    pub(crate) ai_assists: RwLock<BTreeMap<String, crate::human::PendingAssist>>,
    /// Live agent sessions (mailboxes are memory-only; records persist).
    pub(crate) agent_sessions: RwLock<BTreeMap<String, crate::agents::AgentSessionEntry>>,
    /// Memory-only, hashed bearer capabilities issued to a single live Agent
    /// Session. They are never serialized and die on close/expiry/restart.
    pub(crate) agent_session_capabilities:
        RwLock<BTreeMap<String, crate::agents::AgentSessionCapability>>,
    /// Serializes agent-session creation so count/estop checks aren't TOCTOU.
    pub(crate) agent_create_lock: tokio::sync::Mutex<()>,
    /// Serializes Governor usage snapshot → authorization → Accepted receipt.
    pub(crate) authorization_lock: tokio::sync::Mutex<()>,
    /// Costs authorized but not yet committed to the persisted session.
    pub(crate) monetary_reservations: tokio::sync::Mutex<BTreeMap<String, (SessionId, f64)>>,
    /// Same-process single-flight claims for plan execution.
    pub(crate) executing_plans: std::sync::Mutex<BTreeSet<String>>,
    /// Currently-capturing sensors → always-visible indicators.
    pub(crate) sensors: std::sync::Mutex<BTreeMap<String, crate::sensors::SensorUse>>,
    /// Typed handle to the microphone receptor (None when not registered).
    pub mic_receptor: Option<Arc<adapters_media::MicListenReceptor>>,
    /// Presentation bridge: companion-window presence + pending command acks.
    pub presentation: Arc<crate::presentation::PresentationBridge>,
    /// iPhone Mobile Provider（v0.5 Phase 6）。
    pub mobile: Arc<crate::mobile::MobileBridge>,
    /// Character Presentation Protocol：gateway 宿主（instance 登記、truth
    /// projection、adapter token、外部 WebSocket 連線）。
    pub character: Arc<crate::character::CharacterHub>,
    /// 主動式對話政策狀態（確定性頻率限制；持久化到 meta）。
    pub(crate) proactive_dialogue: RwLock<crate::proactive::ProactiveDialogueState>,
    /// Generated proactive candidates waiting for a real local Agent result.
    /// Memory-only and lease-bounded: restart expires the associated Agent
    /// Session and never replays an unverified candidate.
    pub(crate) proactive_agent_tasks:
        RwLock<BTreeMap<String, crate::proactive::PendingProactiveTask>>,
    /// Agent Gateway：真實 agent 子程序（codex/claude-code）管理。
    pub(crate) gateway: crate::gateway::GatewayManager,
    /// 「已測試」證據（spec §9.3）：掃描到 metadata／設定檔存在，都不等於
    /// 連線完成，更不等於測過。這裡只記錄**實際觀察到**的成功／失敗，
    /// 由 providers.rs 統一寫入並在讀取時併進 descriptor.detail。
    pub(crate) provider_tested:
        std::sync::Mutex<BTreeMap<String, crate::providers::ProviderTested>>,
    /// 哪些 provider 有實體裝置連線（serial/mqtt/ble）。這些連線的一次成功
    /// 讀取／命令必然通過 hello 身分＋pair-ok 握手，所以證據等級是
    /// handshake；HTTP 宣告式或內建能力只能記到 capability。
    pub(crate) device_link_providers: std::sync::Mutex<BTreeSet<String>>,
    /// 知識檢索的可替換向量候選介面。
    pub(crate) vector_index: Box<dyn crate::knowledge::VectorIndex>,
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl std::ops::Deref for Runtime {
    type Target = RuntimeInner;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl Runtime {
    pub fn clone_handle(&self) -> Runtime {
        self.clone()
    }

    /// Rebuild a handle from the inner Arc (crate-internal, e.g. actuators
    /// that hold a Weak reference back to the runtime).
    pub(crate) fn from_inner(inner: Arc<RuntimeInner>) -> Runtime {
        Runtime { inner }
    }

    pub async fn start(opts: RuntimeOptions) -> DomainResult<Runtime> {
        let paths = Paths::resolve(opts.home.as_deref());
        let config_service = ConfigService::new(paths.clone());
        let mut config_errors = Vec::new();
        let config = match config_service.load_runtime_config() {
            Ok(c) => c,
            Err(e) => {
                config_errors.push(format!(
                    "interaction.yaml invalid, using last-known-good defaults: {e}"
                ));
                RuntimeConfig::default()
            }
        };
        let policy = match config_service.load_policy() {
            Ok(p) => p,
            Err(e) => {
                config_errors.push(format!("policy.yaml invalid, using defaults: {e}"));
                PolicyConfig::default()
            }
        };

        let lock = if opts.acquire_lock {
            Some(InstanceLock::acquire(paths.lock_file())?)
        } else {
            None
        };

        let store = if opts.in_memory_db {
            Store::open_in_memory()?
        } else {
            Store::open(&paths.db_file())?
        };

        // Crash / restart recovery: anything still open from a previous run is
        // UNKNOWN — mark uncertain, never re-dispatch, never resume high-risk.
        let clean = store.get_meta("clean_shutdown")?.as_deref() == Some("true");
        let stale = store.open_receipts()?;
        if !stale.is_empty() {
            for mut receipt in stale {
                receipt.push_error(
                    "runtime_restart",
                    if clean {
                        "runtime restarted with open action"
                    } else {
                        "runtime crashed with open action"
                    },
                    Utc::now(),
                );
                let _ = receipt.transition(ActionStatus::Uncertain, Utc::now());
                store.upsert_receipt(&receipt, "")?;
            }
        }
        store.set_meta("clean_shutdown", "false")?;
        let estop_engaged = store.get_meta("estop_engaged")?.as_deref() == Some("true");

        let events = EventBus::default();
        let registry = CapabilityRegistry::new(events.clone());
        let outbox = Outbox::new();

        // ---- builtin receptors ----
        let mut push_receptors = BTreeMap::new();
        for receptor in builtin_push_receptors() {
            push_receptors.insert(receptor.id().as_str().to_string(), receptor.clone());
            registry.register_receptor(receptor).await?;
        }
        registry
            .register_receptor(Arc::new(SystemTimeReceptor))
            .await?;

        // ---- presentation (companion surface) receptors ----
        // Itemized semantic receptors whose health mirrors the companion
        // window's presence: hidden or disconnected ⇒ offline, honestly.
        let presentation_bridge = crate::presentation::PresentationBridge::new();
        for receptor in crate::presentation::presentation_receptors(&presentation_bridge) {
            push_receptors.insert(receptor.id().as_str().to_string(), receptor.clone());
            registry.register_receptor(receptor).await?;
        }

        // ---- builtin actuators ----
        registry
            .register_actuator(Arc::new(ConversationActuator::new(outbox.clone())))
            .await?;
        registry
            .register_actuator(Arc::new(WebUiActuator::new(outbox.clone())))
            .await?;
        registry
            .register_actuator(Arc::new(LocalLogActuator))
            .await?;
        registry
            .register_actuator(Arc::new(LocalNotificationActuator))
            .await?;
        registry
            .register_actuator(Arc::new(WebhookActuator::new(
                config.webhook_allowlist.clone(),
            )))
            .await?;
        // ---- microphone receptor (default OFF, consent-gated; capture only
        //      via explicit bounded listen windows) ----
        let sensor_cb_slot: Arc<std::sync::OnceLock<std::sync::Weak<RuntimeInner>>> =
            Arc::new(std::sync::OnceLock::new());
        let cb_slot = sensor_cb_slot.clone();
        let sensor_cb: adapters_media::SensorStateCallback = Arc::new(move |kind, active| {
            if let Some(weak) = cb_slot.get() {
                if let Some(inner) = weak.upgrade() {
                    Runtime::from_inner(inner).sensor_state_changed(kind, active);
                }
            }
        });
        #[cfg(feature = "mic-capture")]
        let mic_source: Arc<dyn adapters_media::CaptureSource> =
            Arc::new(adapters_media::cpal_source::CpalSource);
        #[cfg(not(feature = "mic-capture"))]
        let mic_source: Arc<dyn adapters_media::CaptureSource> =
            Arc::new(adapters_media::UnavailableSource);
        let mic_receptor = Arc::new(adapters_media::MicListenReceptor::new(
            mic_source,
            Some(sensor_cb),
        ));
        registry.register_receptor(mic_receptor.clone()).await?;

        let mock_actuator = Arc::new(MockActuator::new("mock.actuator", "haptic"));
        registry
            .register_receptor(Arc::new(MockDeviceStatusReceptor::new(
                mock_actuator.device_state(),
            )))
            .await?;
        registry.register_actuator(mock_actuator.clone()).await?;

        // ---- canonical tools ----
        for tool in interaction_tool_schema::canonical_tools() {
            registry.register_tool_operation(tool).await?;
        }

        // ---- recipes (File=Truth) ----
        if !opts.in_memory_db {
            seed_default_recipes(&config_service);
        }
        let session = store.latest_active_session()?;
        let active_session_id = session.as_ref().map(|s| s.session_id.clone());
        let (loaded, recipe_load_errors) = config_service.load_recipes();
        let mut recipes = BTreeMap::new();
        for recipe in loaded {
            // Cooldowns and budgets survive restarts (loaded from the store).
            let state =
                Self::load_recipe_state(&store, recipe.id.as_str(), active_session_id.as_ref());
            recipes.insert(
                recipe.id.as_str().to_string(),
                RecipeEntry { recipe, state },
            );
        }

        let pause_state = crate::human::PauseState::load(&store);
        let proactive_state: crate::proactive::ProactiveDialogueState = store
            .get_meta(crate::proactive::PROACTIVE_META_KEY)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let runtime = Runtime {
            inner: Arc::new(RuntimeInner {
                paths,
                config_service,
                config: RwLock::new(config),
                policy_config: RwLock::new(policy),
                registry,
                providers: interaction_registry::providers::ProviderRegistry::new(events.clone()),
                store,
                events,
                outbox,
                texts: TextSelector::default(),
                estop: AtomicBool::new(estop_engaged),
                push_receptors,
                dynamic_push: RwLock::new(BTreeMap::new()),
                mobile: crate::mobile::MobileBridge::new(),
                character: crate::character::CharacterHub::new(),
                mock_actuator,
                recipes: RwLock::new(recipes),
                recipe_errors: RwLock::new(recipe_load_errors.into_iter().collect()),
                session: RwLock::new(session),
                shutdown_token: CancellationToken::new(),
                started_at: Utc::now(),
                lock: std::sync::Mutex::new(lock),
                config_errors,
                pause: RwLock::new(pause_state),
                ai_assists: RwLock::new(BTreeMap::new()),
                agent_sessions: RwLock::new(BTreeMap::new()),
                agent_session_capabilities: RwLock::new(BTreeMap::new()),
                agent_create_lock: tokio::sync::Mutex::new(()),
                authorization_lock: tokio::sync::Mutex::new(()),
                monetary_reservations: tokio::sync::Mutex::new(BTreeMap::new()),
                executing_plans: std::sync::Mutex::new(BTreeSet::new()),
                sensors: std::sync::Mutex::new(BTreeMap::new()),
                mic_receptor: Some(mic_receptor),
                presentation: presentation_bridge,
                proactive_dialogue: RwLock::new(proactive_state),
                proactive_agent_tasks: RwLock::new(BTreeMap::new()),
                gateway: crate::gateway::GatewayManager::new(),
                provider_tested: std::sync::Mutex::new(BTreeMap::new()),
                device_link_providers: std::sync::Mutex::new(BTreeSet::new()),
                vector_index: Box::new(crate::knowledge::LocalSubwordEmbeddingIndex::default()),
            }),
        };
        let _ = sensor_cb_slot.set(Arc::downgrade(&runtime.inner));

        // Delegation actuator goes through the same registry/governor path.
        let delegate = crate::agents::DelegateActuator::new(Arc::downgrade(&runtime.inner));
        let _ = runtime.registry.register_actuator(Arc::new(delegate)).await;

        // Presentation actuators (itemized; consent-gated ones start disabled
        // by the registry's default rules).
        for kind in crate::presentation::PresentationKind::ALL {
            let actuator = crate::presentation::PresentationActuator::new(
                kind,
                runtime.presentation.clone(),
                Arc::downgrade(&runtime.inner),
            );
            let _ = runtime.registry.register_actuator(Arc::new(actuator)).await;
        }
        runtime.restore_agent_sessions().await;
        runtime.init_providers().await;
        // 外部 character adapter 登記（token sha256＋撤銷旗標）跨重啟保留。
        runtime.character_load_adapters();
        runtime.rebuild_vector_index();

        // 測試模式（無 watchdog）＝模擬：iPhone 伺服器不得把 Bonjour 記錄廣播到實體區網。
        runtime.mobile.set_advertise_mdns(opts.spawn_watchdog);
        if opts.spawn_watchdog {
            runtime.spawn_watchdog();
            // 背景發現本機 AI agent（codex/claude-code）；不阻塞啟動。
            // 測試模式（無 watchdog）不做，避免在單元測試裡生子程序。
            runtime.spawn_agent_discovery();
        }
        runtime.mobile_autostart_if_paired();
        Ok(runtime)
    }

    /// Graceful shutdown: cancel open actions, stop drivers, mark clean.
    pub async fn shutdown(&self) {
        self.shutdown_token.cancel();
        self.character_shutdown();
        if let Ok(open) = self.store.open_receipts() {
            for mut receipt in open {
                let _ = receipt.transition(ActionStatus::Cancelled, Utc::now());
                receipt.push_error("shutdown", "runtime shutting down", Utc::now());
                let _ = self.store.upsert_receipt(&receipt, "");
                self.emit_action_event(
                    EventType::ActionCancelled,
                    &receipt,
                    json!({"reason": "shutdown"}),
                );
            }
        }
        for actuator in self.registry.all_actuator_instances().await {
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(2), actuator.emergency_stop())
                    .await;
        }
        // Gateway agent 子程序：關機時依已記錄的 pgid 整樹終結（「子程序
        // 絕不跨 runtime 重啟存活」）。必須 inline await——serve 返回後緊接
        // process::exit，spawn 出去的 kill task 沒有機會跑完。
        let _ = self.reap_recorded_gateway_pgids("shutdown").await;
        let _ = self.store.set_meta("clean_shutdown", "true");
        let mut lock = self.lock.lock().expect("lock mutex");
        lock.take(); // drop → releases pid file
    }

    // ------------------------------------------------------------------
    // Status / capabilities
    // ------------------------------------------------------------------

    pub fn is_estopped(&self) -> bool {
        self.estop.load(Ordering::SeqCst)
    }

    pub async fn status(&self) -> Value {
        let session = self.session.read().await.clone();
        let recipes = self.recipes.read().await;
        let recipe_errors = self.recipe_errors.read().await;
        // 安靜時段（policy.quietHours 依本機時間判定）：角色視窗與 Director 靠
        // 這個鍵進入 quiet 基態；沒有它，quiet 路徑在生產環境永遠不可達。
        let quiet_hours = {
            let policy = self.policy().await;
            let local = chrono::Local::now().time();
            policy
                .quiet_hours
                .iter()
                .any(|w| quiet_window_active(&w.start, &w.end, local))
        };
        let receipts = self
            .store
            .receipts(None, 1)
            .ok()
            .map(|r| r.len())
            .unwrap_or(0);
        json!({
            "name": "adaptive-interaction",
            "version": env!("CARGO_PKG_VERSION"),
            "schemaVersion": interaction_core::SCHEMA_VERSION,
            "startedAt": self.started_at,
            "uptimeSeconds": Utc::now().signed_duration_since(self.started_at).num_seconds(),
            "emergencyStop": self.is_estopped(),
            "session": session.map(|s| json!({
                "sessionId": s.session_id.as_str(),
                "state": s.state,
                "startedAt": s.started_at,
                "consents": s.consents.len(),
            })),
            "capabilityVersion": self.registry.version(),
            "recipes": {"loaded": recipes.len(), "errors": recipe_errors.len()},
            "configErrors": self.config_errors,
            "hasReceipts": receipts > 0,
            "eventSequence": self.events.last_sequence(),
            "proactivePause": self.pause_status().await,
            "pendingAiAssists": self.pending_ai_assists().await.len(),
            "agentSessions": self.open_agent_sessions().await,
            "activeSensors": self.active_sensors_all().await,
            "presentation": self.presentation_status(),
            "characterProtocol": self.character_status(),
            "quietHours": quiet_hours,
            "onboardingCompleted": self.onboarding_state().await
                .get("completed").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    pub async fn capabilities(&self, ctx: &DiscoveryContext) -> CapabilitySnapshot {
        let policy = self.policy().await;
        let mut constraints = Vec::new();
        if self.is_estopped() {
            constraints.push(CapabilityConstraint {
                kind: "emergency-stop".into(),
                detail: "emergency stop engaged; all actuation blocked until cleared".into(),
            });
        }
        let local = chrono::Local::now().time();
        for window in &policy.quiet_hours {
            let active = crate::runtime::quiet_window_active(&window.start, &window.end, local);
            if active {
                constraints.push(CapabilityConstraint {
                    kind: "quiet-hours".into(),
                    detail: format!(
                        "quiet hours {}-{} active; intrusive channels silenced",
                        window.start, window.end
                    ),
                });
            }
        }
        if self.session.read().await.is_none() {
            constraints.push(CapabilityConstraint {
                kind: "session".into(),
                detail: "no active session; start one before executing".into(),
            });
        }
        self.registry
            .snapshot(ctx, policy, constraints, Utc::now())
            .await
    }

    pub async fn policy(&self) -> PolicyConfig {
        self.policy_config.read().await.clone()
    }

    /// Merge-patch the policy. `resumeHighRiskAfterRestart` is pinned false.
    pub async fn update_policy(&self, patch: Value) -> DomainResult<PolicyConfig> {
        let current = self.policy().await;
        let mut merged =
            serde_json::to_value(&current).map_err(|e| DomainError::Internal(e.to_string()))?;
        merge_json(&mut merged, &patch);
        let mut updated: PolicyConfig = serde_json::from_value(merged)
            .map_err(|e| DomainError::Validation(format!("policy patch: {e}")))?;
        updated.resume_high_risk_after_restart = false;
        self.config_service.save_policy(&updated)?;
        *self.policy_config.write().await = updated.clone();
        self.events.emit(EventType::PolicyChanged, json!({}));
        self.store
            .audit("policy.changed", "api", &json!({"patch": redact(&patch)}))?;
        Ok(updated)
    }

    // ------------------------------------------------------------------
    // Observations
    // ------------------------------------------------------------------

    pub async fn observe_stored(&self, query: &ObservationQuery) -> DomainResult<Vec<Observation>> {
        self.store.query_observations(query)
    }

    /// Live read from a receptor; the observation is stored and announced.
    pub async fn observe_fresh(&self, receptor_id: &ReceptorId) -> DomainResult<Observation> {
        let receptor = self.registry.receptor(receptor_id).await?;
        let mut obs = receptor.read().await?;
        obs.session_id = self
            .session
            .read()
            .await
            .as_ref()
            .map(|s| s.session_id.clone());
        // Honor a `retention: none` privacy declaration: derived facts from a
        // no-retention receptor (e.g. the microphone's sound-level) are shown
        // and can drive recipes live, but are NEVER persisted to the store.
        if !declares_no_retention(&receptor.manifest()) {
            self.store.insert_observation(&obs)?;
        }
        self.publish_observation_event(&obs);
        // 「已測試」證據（spec §9.3）：真的讀到資料才算，只有 metadata 不算。
        self.note_capability_tested(
            crate::providers::TestedCapability::Receptor,
            receptor_id.as_str(),
        )
        .await;
        Ok(obs)
    }

    /// Ingest an external event into a push receptor, then evaluate recipes.
    pub async fn ingest(
        &self,
        receptor_id: &str,
        facts: BTreeMap<String, Value>,
        inferences: BTreeMap<String, Value>,
        confidence: f64,
    ) -> DomainResult<Observation> {
        self.ingest_with_gate(receptor_id, facts, inferences, confidence, true)
            .await
    }

    /// `enforce_surface_gate=false` 只給外部 character adapter 的正規化輸入用：
    /// 它們沒有桌面視窗表面，隱藏／斷線閘門不適用（policy／consent 仍然適用）。
    pub(crate) async fn ingest_with_gate(
        &self,
        receptor_id: &str,
        facts: BTreeMap<String, Value>,
        inferences: BTreeMap<String, Value>,
        confidence: f64,
        enforce_surface_gate: bool,
    ) -> DomainResult<Observation> {
        let receptor = self
            .push_receptor(receptor_id)
            .await
            .ok_or_else(|| DomainError::NotFound(format!("push receptor {receptor_id}")))?;
        // Respect enable/disable state.
        let instance = self
            .registry
            .receptor(&ReceptorId::new(receptor_id))
            .await?;
        // Companion-surface receptors stop when the companion is hidden or
        // disconnected (spec: hiding the companion stops its in-window
        // senses — deterministically, not just by frontend courtesy).
        if enforce_surface_gate
            && crate::presentation::is_companion_surface_receptor(receptor_id)
            && !self.presentation.accepts_input(Utc::now())
        {
            return Err(DomainError::Unavailable(
                "companion window hidden or not connected; its receptors are stopped".into(),
            ));
        }
        // Anti-forgery: a caller-pushed `actionId` FACT can otherwise be picked
        // up by the verifier as "observed" evidence and self-attest completion
        // of a delegated action. Rename it to `claimActionId` (symmetric to
        // report_agent_session) so pushed facts can never act as verification.
        let mut facts = facts;
        if let Some(v) = facts.remove("actionId") {
            facts.insert("claimActionId".to_string(), v);
        }
        let mut obs = receptor.push(facts, inferences, confidence);
        obs.session_id = self
            .session
            .read()
            .await
            .as_ref()
            .map(|s| s.session_id.clone());
        if !declares_no_retention(&instance.manifest()) {
            self.store.insert_observation(&obs)?;
        }
        self.publish_observation_event(&obs);
        // 使用者真實互動 → 主動對話的「未回覆不追問」解除。
        if matches!(
            receptor_id,
            "companion.click"
                | "companion.text-input"
                | "companion.quick-action"
                | "companion.drag-drop"
                | "session.input"
        ) {
            self.proactive_note_reply().await;
        }
        // Autonomous loop: observation may trigger recipes.
        self.evaluate_recipes(Some(receptor_id)).await;
        Ok(obs)
    }

    fn publish_observation_event(&self, obs: &Observation) {
        let mut event = RuntimeEvent::new(
            EventType::ReceptorObservation,
            obs.received_at,
            json!({
                "observationId": obs.observation_id.as_str(),
                "receptorId": obs.receptor_id.as_str(),
                "facts": obs.facts,
                "confidence": obs.confidence,
            }),
        );
        if let Some(s) = &obs.session_id {
            event = event.with_session(s.clone());
        }
        self.events.publish(event);
        // Character Protocol §11：receptor.observation → notice(listening)。
        self.character_project_observation(obs);
    }

    // ------------------------------------------------------------------
    // Planning
    // ------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub async fn create_plan(
        &self,
        intent: SemanticIntent,
        candidates: Vec<String>,
        min_channels: u32,
        max_channels: u32,
        allow_no_action: bool,
        message_strategy: Option<MessageStrategy>,
        metadata: BTreeMap<String, Value>,
    ) -> DomainResult<Plan> {
        let session = self.require_session().await?;
        let snapshot = self.capabilities(&DiscoveryContext::default()).await;
        let policy = self.policy().await;
        let mut usage = BTreeMap::new();
        for actuator in &snapshot.actuators {
            if let Ok((fired, _)) = self.store.actuator_usage(actuator.id.as_str(), Utc::now()) {
                usage.insert(
                    actuator.id.as_str().to_string(),
                    ActuatorUsageHint {
                        fired_last_hour: fired,
                    },
                );
            }
        }
        let strategy = message_strategy.unwrap_or_else(|| MessageStrategy {
            allow_silence: allow_no_action,
            ..Default::default()
        });
        let candidates_meta = candidates.clone();
        // Consent-gated actuators without an active grant must not crowd out
        // viable safe channels during open selection.
        let now_ts = Utc::now();
        let consent_missing: Vec<String> = snapshot
            .actuators
            .iter()
            .filter(|m| m.requires_consent)
            .filter(|m| {
                !session.has_consent(&ConsentScope::Actuator(m.id.as_str().to_string()), now_ts)
                    && !session.has_consent(&ConsentScope::Channel(m.channel.clone()), now_ts)
            })
            .map(|m| m.id.as_str().to_string())
            .collect();
        let mut plan = build_plan(
            PlanRequest {
                session_id: session.session_id.clone(),
                intent,
                snapshot: &snapshot,
                candidates,
                consent_missing,
                min_channels,
                max_channels,
                allow_no_action,
                message_strategy: strategy,
                usage,
                now: Utc::now(),
                default_ttl_ms: policy.default_ttl_ms,
            },
            &self.texts,
        );
        if !candidates_meta.is_empty() {
            plan.metadata
                .insert("candidates".to_string(), json!(candidates_meta));
        }
        for (k, v) in metadata {
            plan.metadata.insert(k, v);
        }
        // Fallback semantics: step order = caller's preference order, not
        // utility order (the whole point is "try the preferred one first").
        if plan.metadata.get("actuationMode").and_then(|v| v.as_str()) == Some("fallback") {
            let candidate_order = plan
                .steps
                .iter()
                .map(|s| s.actuator_id.as_str().to_string())
                .collect::<Vec<_>>();
            let index_of = |id: &str| {
                plan.metadata
                    .get("candidates")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.iter().position(|c| c.as_str() == Some(id)))
                    .unwrap_or_else(|| {
                        candidate_order
                            .iter()
                            .position(|c| c == id)
                            .unwrap_or(usize::MAX)
                    })
            };
            plan.steps.sort_by_key(|s| index_of(s.actuator_id.as_str()));
        }
        self.store.upsert_plan(&plan)?;
        let event_type = if plan.status == PlanStatus::Blocked {
            EventType::PlanBlocked
        } else {
            EventType::PlanCreated
        };
        self.events.publish(
            RuntimeEvent::new(
                event_type,
                Utc::now(),
                json!({
                    "planId": plan.plan_id.as_str(),
                    "intent": plan.intent.intent,
                    "steps": plan.steps.len(),
                    "status": plan.status,
                }),
            )
            .with_session(session.session_id.clone())
            .with_correlation(plan.correlation_id.clone()),
        );
        if plan.status == PlanStatus::Blocked {
            // Character Protocol §11：plan.blocked → blocked（correlationId = planId）。
            self.character_project_plan_blocked(plan.plan_id.as_str(), None);
        }
        Ok(plan)
    }

    pub fn get_plan(&self, plan_id: &PlanId) -> DomainResult<Plan> {
        self.store.plan(plan_id)
    }

    // ------------------------------------------------------------------
    // Actions
    // ------------------------------------------------------------------

    pub fn get_action(&self, action_id: &ActionId) -> DomainResult<ActionReceipt> {
        self.store.receipt(action_id)
    }

    pub fn list_actions(
        &self,
        session: Option<&SessionId>,
        limit: u32,
    ) -> DomainResult<Vec<ActionReceipt>> {
        self.store.receipts(session, limit)
    }

    pub async fn verify_action(&self, action_id: &ActionId) -> DomainResult<ActionReceipt> {
        let receipt = self.store.receipt(action_id)?;
        let strategy = self
            .store
            .plan(&receipt.plan_id)
            .ok()
            .and_then(|p| {
                p.metadata
                    .get("verification")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| "observed".to_string());
        self.verify_receipt(receipt, "", &strategy).await
    }

    pub async fn cancel_action(&self, action_id: &ActionId) -> DomainResult<ActionReceipt> {
        let mut receipt = self.store.receipt(action_id)?;
        if receipt.is_terminal() {
            return Err(DomainError::Conflict(format!(
                "action {action_id} already terminal ({:?})",
                receipt.current_status
            )));
        }
        if let Ok(actuator) = self.registry.actuator_any(&receipt.actuator_id).await {
            let _ = actuator.cancel(action_id).await;
        }
        receipt
            .transition(ActionStatus::Cancelled, Utc::now())
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        self.store.upsert_receipt(&receipt, "")?;
        self.emit_action_event(EventType::ActionCancelled, &receipt, json!({}));
        self.store.audit(
            "action.cancelled",
            "api",
            &json!({"actionId": action_id.as_str()}),
        )?;
        Ok(receipt)
    }

    /// Cancel every non-terminal action (soft stop-all; not the e-stop).
    pub async fn stop_all(&self) -> DomainResult<u32> {
        let open = self.store.open_receipts()?;
        let mut count = 0;
        for receipt in open {
            if self.cancel_action(&receipt.action_id).await.is_ok() {
                count += 1;
            }
        }
        Ok(count)
    }

    // ------------------------------------------------------------------
    // Emergency stop
    // ------------------------------------------------------------------

    pub async fn emergency_stop(&self, actor: &str, reason: Option<String>) -> DomainResult<Value> {
        self.estop.store(true, Ordering::SeqCst);
        self.store.set_meta("estop_engaged", "true")?;

        // Sensors first: releasing capture is synchronous and cheap, and must
        // not wait behind a serial per-actuator emergency_stop loop (a slow
        // declarative device driver could otherwise delay mic release).
        let _ = self.stop_all_sensors(actor).await;
        // Remote sensors too: a paired iPhone's microphone is a sensor of this
        // system. The desktop forces the high-risk receptor off and tells every
        // phone to stop sensing (`stop-all { sensors: true }`) — an emergency
        // stop that only silenced local capture was not an emergency stop.
        self.mobile_estop_stop_sensors(actor).await;

        let mut stopped_actions = 0;
        if let Ok(open) = self.store.open_receipts() {
            for mut receipt in open {
                let _ = receipt.transition(ActionStatus::Stopped, Utc::now());
                receipt.push_error(
                    "emergency_stop",
                    reason.as_deref().unwrap_or("emergency stop"),
                    Utc::now(),
                );
                let _ = self.store.upsert_receipt(&receipt, "");
                stopped_actions += 1;
            }
        }
        let mut stopped_actuators = 0;
        for actuator in self.registry.all_actuator_instances().await {
            if tokio::time::timeout(std::time::Duration::from_secs(2), actuator.emergency_stop())
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false)
            {
                stopped_actuators += 1;
            }
        }
        // Cancel every open agent session; delegated work never survives an
        // emergency stop and never resumes automatically.
        self.estop_agent_sessions().await;
        // Revoke all session consents; nothing resumes automatically.
        if let Some(session) = self.session.write().await.as_mut() {
            let scopes: Vec<ConsentScope> =
                session.consents.iter().map(|c| c.scope.clone()).collect();
            for scope in scopes {
                session.revoke(&scope, Utc::now());
            }
            let _ = self.store.upsert_session(session);
            self.events
                .emit(EventType::ConsentChanged, json!({"revokedAll": true}));
        }
        self.outbox.push(OutboxMessage {
            channel: "conversation".into(),
            intent: "emergency-stop".into(),
            text: Some("緊急停止已執行，所有輸出已中止。".into()),
            action_id: ActionId::new("emergency-stop"),
            at: Utc::now(),
        });
        let payload = json!({
            "actor": actor,
            "reason": reason,
            "stoppedActions": stopped_actions,
            "stoppedActuators": stopped_actuators,
        });
        self.events.emit(EventType::EmergencyStop, payload.clone());
        // Character Protocol §11：emergency.stop → emergency（floor 100，可搶占任何演出；
        // 沒有任何角色 instance 時走 system.text，不得遺失）。
        self.character_project_emergency(true);
        self.store.audit("emergency.stop", actor, &payload)?;
        Ok(payload)
    }

    /// Explicit human re-arm; never automatic. Latched drivers (physical
    /// devices) are unlatched here — but no cancelled action is resumed.
    pub async fn clear_emergency_stop(&self, actor: &str) -> DomainResult<()> {
        self.estop.store(false, Ordering::SeqCst);
        self.store.set_meta("estop_engaged", "false")?;
        for actuator in self.registry.all_actuator_instances().await {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                actuator.emergency_clear(),
            )
            .await;
        }
        self.events.emit(
            EventType::EmergencyStop,
            json!({"cleared": true, "actor": actor}),
        );
        self.character_project_emergency(false);
        self.store.audit("emergency.clear", actor, &json!({}))?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Sessions & consent
    // ------------------------------------------------------------------

    pub async fn require_session(&self) -> DomainResult<Session> {
        let guard = self.session.read().await;
        match guard.as_ref() {
            Some(s) if s.is_active(Utc::now()) => Ok(s.clone()),
            Some(s) => Err(DomainError::SessionInactive(format!(
                "session {} is {:?}",
                s.session_id, s.state
            ))),
            None => Err(DomainError::SessionInactive(
                "no active session; start one first (interact-ai session start)".into(),
            )),
        }
    }

    pub async fn current_session(&self) -> Option<Session> {
        self.session.read().await.clone()
    }

    pub async fn start_session(
        &self,
        label: Option<String>,
        ttl_minutes: Option<u32>,
        consents: Vec<String>,
    ) -> DomainResult<Session> {
        // Stop any previous session first (single-session model).
        if let Some(existing) = self.session.write().await.as_mut() {
            if existing.is_active(Utc::now()) {
                existing.stop(Utc::now());
                let _ = self.store.upsert_session(existing);
                self.events.emit(
                    EventType::SessionStopped,
                    json!({"sessionId": existing.session_id.as_str(), "reason": "superseded"}),
                );
            }
        }
        let ttl = match ttl_minutes {
            Some(m) => Some(m),
            None => {
                let m = self.config.read().await.session_ttl_minutes;
                if m == 0 {
                    None
                } else {
                    Some(m)
                }
            }
        }
        .map(|m| m as u64 * 60_000);
        let mut session = Session::new(Utc::now(), label, ttl);
        for scope_str in consents {
            let scope = parse_scope(&scope_str)?;
            session.grant(scope, Utc::now(), None);
        }
        self.store.upsert_session(&session)?;
        *self.session.write().await = Some(session.clone());
        // New session, fresh recipe budgets.
        for entry in self.recipes.write().await.values_mut() {
            entry.state.executions_this_session = 0;
        }
        self.events.emit(
            EventType::SessionStarted,
            json!({"sessionId": session.session_id.as_str(), "label": session.label}),
        );
        self.store.audit(
            "session.started",
            "api",
            &json!({"sessionId": session.session_id.as_str()}),
        )?;
        Ok(session)
    }

    pub async fn grant_consent(
        &self,
        scope_str: &str,
        expires_minutes: Option<u32>,
    ) -> DomainResult<Session> {
        let scope = parse_scope(scope_str)?;
        let mut guard = self.session.write().await;
        let session = guard
            .as_mut()
            .filter(|s| s.is_active(Utc::now()))
            .ok_or_else(|| DomainError::SessionInactive("no active session".into()))?;
        let expires = expires_minutes.map(|m| Utc::now() + chrono::Duration::minutes(m as i64));
        session.grant(scope, Utc::now(), expires);
        self.store.upsert_session(session)?;
        self.events.emit(
            EventType::ConsentChanged,
            json!({"sessionId": session.session_id.as_str(), "granted": scope_str}),
        );
        self.store
            .audit("consent.granted", "api", &json!({"scope": scope_str}))?;
        Ok(session.clone())
    }

    /// Revoke consent and cancel any in-flight actions covered by the scope.
    pub async fn revoke_consent(&self, scope_str: &str) -> DomainResult<Session> {
        let scope = parse_scope(scope_str)?;
        let session = {
            let mut guard = self.session.write().await;
            let session = guard
                .as_mut()
                .ok_or_else(|| DomainError::SessionInactive("no session".into()))?;
            session.revoke(&scope, Utc::now());
            self.store.upsert_session(session)?;
            session.clone()
        };
        self.events.emit(
            EventType::ConsentChanged,
            json!({"sessionId": session.session_id.as_str(), "revoked": scope_str}),
        );
        self.store
            .audit("consent.revoked", "api", &json!({"scope": scope_str}))?;
        // Revoking a receptor's consent must stop any capture it is driving
        // NOW (a sensor keeps capturing until explicitly stopped). The mic is
        // the only consent-gated capturing receptor today.
        if let ConsentScope::Receptor(id) = &scope {
            if id == "microphone.listen" {
                let _ = self.stop_all_sensors("consent-revoked").await;
            }
        }
        // Cancel matching open actions immediately.
        if let Ok(open) = self.store.open_receipts() {
            for receipt in open {
                let matches = match &scope {
                    ConsentScope::Actuator(id) => receipt.actuator_id.as_str() == id,
                    ConsentScope::Channel(channel) => self
                        .registry
                        .actuator_any(&receipt.actuator_id)
                        .await
                        .map(|a| a.manifest().channel == *channel)
                        .unwrap_or(false),
                    _ => false,
                };
                if matches {
                    let _ = self.cancel_action(&receipt.action_id).await;
                }
            }
        }
        Ok(session)
    }

    pub async fn stop_session(&self) -> DomainResult<()> {
        {
            let mut guard = self.session.write().await;
            if let Some(session) = guard.as_mut() {
                session.stop(Utc::now());
                self.store.upsert_session(session)?;
                self.events.emit(
                    EventType::SessionStopped,
                    json!({"sessionId": session.session_id.as_str()}),
                );
            }
            *guard = None;
        }
        // Session-scoped consents die with the session, so any sensor those
        // consents authorized must stop capturing now.
        let _ = self.stop_all_sensors("session-stopped").await;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Recipes
    // ------------------------------------------------------------------

    pub async fn list_recipes(&self) -> Vec<(Recipe, RecipeState)> {
        self.recipes
            .read()
            .await
            .values()
            .map(|e| (e.recipe.clone(), e.state.clone()))
            .collect()
    }

    pub async fn get_recipe(&self, id: &str) -> DomainResult<Recipe> {
        self.recipes
            .read()
            .await
            .get(id)
            .map(|e| e.recipe.clone())
            .ok_or_else(|| DomainError::NotFound(format!("recipe {id}")))
    }

    pub async fn upsert_recipe_text(&self, text: &str) -> DomainResult<Recipe> {
        let recipe = interaction_recipe::parse_and_validate(text)
            .map_err(|e| DomainError::Validation(e.to_string()))?;
        self.config_service.save_recipe(&recipe)?;
        let mut map = self.recipes.write().await;
        let state = map
            .get(recipe.id.as_str())
            .map(|e| e.state.clone())
            .unwrap_or_default();
        map.insert(
            recipe.id.as_str().to_string(),
            RecipeEntry {
                recipe: recipe.clone(),
                state,
            },
        );
        self.events.emit(
            EventType::RecipeChanged,
            json!({"recipeId": recipe.id.as_str()}),
        );
        Ok(recipe)
    }

    pub async fn set_recipe_enabled(&self, id: &str, enabled: bool) -> DomainResult<Recipe> {
        let mut map = self.recipes.write().await;
        let entry = map
            .get_mut(id)
            .ok_or_else(|| DomainError::NotFound(format!("recipe {id}")))?;
        entry.recipe.enabled = enabled;
        self.config_service.save_recipe(&entry.recipe)?;
        self.events.emit(
            EventType::RecipeChanged,
            json!({"recipeId": id, "enabled": enabled}),
        );
        Ok(entry.recipe.clone())
    }

    pub async fn remove_recipe(&self, id: &str) -> DomainResult<()> {
        let mut map = self.recipes.write().await;
        map.remove(id)
            .ok_or_else(|| DomainError::NotFound(format!("recipe {id}")))?;
        // File deletion failures must surface: a recipe that silently
        // survives on disk would resurrect on restart.
        self.config_service.delete_recipe(id)?;
        self.events.emit(
            EventType::RecipeChanged,
            json!({"recipeId": id, "removed": true}),
        );
        Ok(())
    }

    /// Explain whether the recipe would fire right now + build (but do not run)
    /// its plan.
    pub async fn simulate_recipe(&self, id: &str) -> DomainResult<Value> {
        let recipe = self.get_recipe(id).await?;
        let observations = self.recent_observations_for_recipe(&recipe).await?;
        let decision = evaluate_trigger(&recipe, &observations, Utc::now());
        let plan = if self.current_session().await.is_some() {
            Some(self.plan_from_recipe(&recipe).await?)
        } else {
            None
        };
        let simulation = if let Some(p) = &plan {
            Some(self.simulate_plan(&p.plan_id).await?)
        } else {
            None
        };
        Ok(json!({
            "recipeId": id,
            "trigger": decision,
            "plan": plan,
            "simulation": simulation,
        }))
    }

    /// Manual run: the trigger condition is bypassed, but enabled state,
    /// recipe limits, consent and the shared Policy Governor still apply.
    pub async fn run_recipe(&self, id: &str) -> DomainResult<Value> {
        self.run_recipe_inner(id, ActionSource::ExplicitRequest, false)
            .await
    }

    /// Agent/tool entry: identical recipe safety plus the user's proactive
    /// pause. An Agent cannot use a direct tool call as a hidden force-run.
    pub async fn run_recipe_for_agent(&self, id: &str) -> DomainResult<Value> {
        self.run_recipe_inner(id, ActionSource::Autonomous, true)
            .await
    }

    async fn run_recipe_inner(
        &self,
        id: &str,
        source: ActionSource,
        respect_proactive_pause: bool,
    ) -> DomainResult<Value> {
        if respect_proactive_pause && self.proactive_paused().await {
            return Err(DomainError::PolicyBlocked(
                "proactive interactions are paused".into(),
            ));
        }
        let session = self.require_session().await?;
        let now = Utc::now();
        let (recipe, state) = {
            let mut recipes = self.recipes.write().await;
            let entry = recipes
                .get_mut(id)
                .ok_or_else(|| DomainError::NotFound(format!("recipe {id}")))?;
            if !entry.recipe.enabled {
                return Err(DomainError::PolicyBlocked(format!(
                    "recipe {id} is disabled"
                )));
            }
            for required in &entry.recipe.consent.required {
                let scope = parse_scope(required)?;
                if !session.has_consent(&scope, now) {
                    return Err(DomainError::ConsentRequired(required.clone()));
                }
            }
            if !Self::recipe_limits_ok(&entry.recipe, &entry.state, now) {
                return Err(DomainError::PolicyBlocked(format!(
                    "recipe {id} cooldown or execution limit reached"
                )));
            }
            entry.state.last_fired_at = Some(now);
            entry.state.executions_this_session += 1;
            entry.state.fired_last_hour.push(now);
            entry.state.fired_last_hour.retain(|at| {
                let age = now.signed_duration_since(*at).num_milliseconds();
                (0..3_600_000).contains(&age)
            });
            (entry.recipe.clone(), entry.state.clone())
        };
        self.persist_recipe_state(id, &state).await;

        let result = async {
            let plan = self.plan_from_recipe(&recipe).await?;
            let receipts = self.execute_plan(&plan.plan_id, source, false).await?;
            Ok(json!({"plan": self.store.plan(&plan.plan_id)?, "receipts": receipts}))
        }
        .await;
        if result.is_err() {
            self.rollback_recipe_reservation(id, now).await;
        }
        result
    }

    pub(crate) async fn plan_from_recipe_public(&self, recipe: &Recipe) -> DomainResult<Plan> {
        self.plan_from_recipe(recipe).await
    }

    async fn plan_from_recipe(&self, recipe: &Recipe) -> DomainResult<Plan> {
        let mut intent = SemanticIntent::new(recipe.intent.clone());
        intent.expires_at = recipe
            .limits
            .expires_after
            .as_deref()
            .and_then(|d| interaction_recipe::parse_duration_ms(d).ok())
            .and_then(|ms| {
                Utc::now().checked_add_signed(chrono::Duration::milliseconds(ms as i64))
            });
        let mut metadata = BTreeMap::new();
        metadata.insert("recipeId".to_string(), json!(recipe.id.as_str()));
        metadata.insert(
            "actuationMode".to_string(),
            json!(format!("{:?}", recipe.actuation.mode).to_lowercase()),
        );
        metadata.insert(
            "verification".to_string(),
            json!(match recipe.verification.strategy {
                interaction_recipe::VerificationStrategy::BestEffort => "best-effort",
                interaction_recipe::VerificationStrategy::Observed => "observed",
                interaction_recipe::VerificationStrategy::None => "none",
            }),
        );
        self.create_plan(
            intent,
            recipe.actuation.candidates.clone(),
            recipe.actuation.min_channels,
            recipe.actuation.max_channels,
            recipe.decision.allow_no_action,
            Some(recipe.message.clone()),
            metadata,
        )
        .await
    }

    async fn recent_observations_for_recipe(
        &self,
        recipe: &Recipe,
    ) -> DomainResult<Vec<Observation>> {
        let window_ms = recipe
            .trigger
            .within
            .as_deref()
            .and_then(|d| interaction_recipe::parse_duration_ms(d).ok())
            .unwrap_or(600_000);
        let now = Utc::now();
        let since = now
            .checked_sub_signed(chrono::Duration::milliseconds(window_ms as i64))
            .unwrap_or(now);
        self.store.query_observations(&ObservationQuery {
            since: Some(since),
            limit: Some(200),
            ..Default::default()
        })
    }

    async fn rollback_recipe_reservation(&self, id: &str, reserved_at: Timestamp) {
        let state = {
            let mut recipes = self.recipes.write().await;
            let Some(entry) = recipes.get_mut(id) else {
                return;
            };
            let before = entry.state.fired_last_hour.len();
            entry.state.fired_last_hour.retain(|at| *at != reserved_at);
            if entry.state.fired_last_hour.len() != before {
                entry.state.executions_this_session =
                    entry.state.executions_this_session.saturating_sub(1);
                entry.state.last_fired_at = entry.state.fired_last_hour.iter().max().copied();
            }
            entry.state.clone()
        };
        self.persist_recipe_state(id, &state).await;
    }

    /// Persist recipe firing state so cooldowns / per-session budgets survive
    /// a runtime restart.
    async fn persist_recipe_state(&self, id: &str, state: &RecipeState) {
        let session_id = self
            .current_session()
            .await
            .map(|s| s.session_id.as_str().to_string());
        let payload = json!({
            "lastFiredAt": state.last_fired_at,
            "executionsThisSession": state.executions_this_session,
            "firedLastHour": state.fired_last_hour,
            "consumedObservations": state.consumed_observations,
            "sessionId": session_id,
        });
        let _ = self
            .store
            .set_meta(&format!("recipe_state:{id}"), &payload.to_string());
    }

    /// Load persisted recipe state at startup. `executions_this_session` only
    /// carries over when the stored session is still the active one.
    fn load_recipe_state(
        store: &Store,
        id: &str,
        active_session: Option<&SessionId>,
    ) -> RecipeState {
        let raw = match store.get_meta(&format!("recipe_state:{id}")) {
            Ok(Some(raw)) => raw,
            _ => return RecipeState::default(),
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            return RecipeState::default();
        };
        let same_session = match (
            value.get("sessionId").and_then(|v| v.as_str()),
            active_session,
        ) {
            (Some(stored), Some(active)) => stored == active.as_str(),
            _ => false,
        };
        let now = Utc::now();
        let mut fired_last_hour: Vec<Timestamp> = value
            .get("firedLastHour")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        fired_last_hour.retain(|at| {
            let age = now.signed_duration_since(*at).num_milliseconds();
            (0..3_600_000).contains(&age)
        });
        RecipeState {
            last_fired_at: value
                .get("lastFiredAt")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            executions_this_session: if same_session {
                value
                    .get("executionsThisSession")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32
            } else {
                0
            },
            fired_last_hour,
            consumed_observations: value
                .get("consumedObservations")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
        }
    }

    /// Evaluate all enabled recipes after a new observation arrived.
    pub async fn evaluate_recipes(&self, receptor_hint: Option<&str>) {
        if self.is_estopped() {
            return;
        }
        // Proactive pause: an ordinary user control, separate from emergency
        // stop. Recipe-triggered autonomy stays silent while paused.
        if self.proactive_paused().await {
            return;
        }
        if self.current_session().await.is_none() {
            return;
        }
        let candidates: Vec<Recipe> = {
            let map = self.recipes.read().await;
            map.values()
                .filter(|e| e.recipe.enabled)
                .filter(|e| {
                    receptor_hint
                        .map(|r| e.recipe.trigger.steps.iter().any(|s| s.receptor == r))
                        .unwrap_or(true)
                })
                .map(|e| e.recipe.clone())
                .collect()
        };
        for recipe in candidates {
            match self.try_fire_recipe(&recipe).await {
                Ok(Some(decision)) => {
                    tracing::info!(
                        recipe = recipe.id.as_str(),
                        "recipe fired: {:?}",
                        decision.explanation.last()
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(recipe = recipe.id.as_str(), error = %e, "recipe evaluation error")
                }
            }
        }
    }

    /// Pure limit check against a state snapshot.
    fn recipe_limits_ok(recipe: &Recipe, state: &RecipeState, now: Timestamp) -> bool {
        if let Some(cooldown) = recipe
            .limits
            .cooldown
            .as_deref()
            .and_then(|d| interaction_recipe::parse_duration_ms(d).ok())
        {
            if let Some(last) = state.last_fired_at {
                if (now.signed_duration_since(last).num_milliseconds().max(0) as u64) < cooldown {
                    return false;
                }
            }
        }
        if let Some(max) = recipe.limits.max_executions_per_session {
            if state.executions_this_session >= max {
                return false;
            }
        }
        if let Some(max) = recipe.limits.max_per_hour {
            let recent = state
                .fired_last_hour
                .iter()
                .filter(|at| {
                    let age = now.signed_duration_since(**at).num_milliseconds();
                    (0..3_600_000).contains(&age)
                })
                .count() as u32;
            if recent >= max {
                return false;
            }
        }
        true
    }

    async fn try_fire_recipe(&self, recipe: &Recipe) -> DomainResult<Option<TriggerDecision>> {
        let now = Utc::now();
        // Fast pre-check (cheap, read lock only); the authoritative check +
        // reservation happens atomically under the write lock below.
        let consumed: Vec<String> = {
            let map = self.recipes.read().await;
            match map.get(recipe.id.as_str()) {
                Some(entry) => {
                    if !Self::recipe_limits_ok(recipe, &entry.state, now) {
                        return Ok(None);
                    }
                    entry.state.consumed_observations.clone()
                }
                None => Vec::new(),
            }
        };
        // Consent requirements.
        let session = self.require_session().await?;
        for scope_str in &recipe.consent.required {
            let scope = parse_scope(scope_str)?;
            if !session.has_consent(&scope, now) {
                return Ok(None);
            }
        }
        // Multi-receptor fusion: drop stale data, dedupe per receptor, apply
        // explicit-input priority, surface contradictions.
        let observations = self.recent_observations_for_recipe(recipe).await?;
        let max_age_ms = recipe
            .context
            .max_age
            .as_deref()
            .and_then(|d| interaction_recipe::parse_duration_ms(d).ok())
            .unwrap_or(600_000);
        let min_confidence = recipe.context.min_confidence.unwrap_or(0.5);
        let fused = interaction_recipe::fuse(&[], &observations, now, max_age_ms, min_confidence);
        // Contradiction gate: if receptors disagree on a fact the trigger
        // conditions read, do not fire autonomously on ambiguous evidence.
        let condition_keys: Vec<String> = recipe
            .trigger
            .steps
            .iter()
            .filter_map(|s| s.condition.as_ref())
            .flat_map(|c| c.referenced_keys())
            .collect();
        let mut uncertain_reason: Option<String> = None;
        if let Some(conflicted) = fused
            .contradictions
            .iter()
            .find(|k| condition_keys.contains(k))
        {
            let defers_to_ai = recipe
                .ai
                .as_ref()
                .map(|a| a.mode == interaction_recipe::AiAssistMode::WhenUncertain)
                .unwrap_or(false);
            if defers_to_ai {
                uncertain_reason = Some(format!(
                    "receptors contradict each other on trigger fact {conflicted:?}"
                ));
            } else {
                tracing::info!(
                    recipe = recipe.id.as_str(),
                    fact = conflicted.as_str(),
                    "not firing: receptors contradict each other on a trigger fact"
                );
                return Ok(None);
            }
        }
        // Trigger evaluation over non-stale, NOT-YET-CONSUMED observations:
        // an event that already fired this recipe can never fire it again.
        let fresh: Vec<Observation> = observations
            .iter()
            .filter(|o| !o.is_stale(now, max_age_ms))
            .filter(|o| !consumed.iter().any(|c| c == o.observation_id.as_str()))
            .cloned()
            .collect();
        let decision = evaluate_trigger(recipe, &fresh, now);
        if !decision.fired {
            return Ok(None);
        }
        // Atomic reservation under the write lock: re-check limits AND the
        // high-water mark, then consume the firing slot. Closes the
        // check-then-act race between concurrent ingests (the jitter sleep
        // otherwise widens that window).
        {
            let mut map = self.recipes.write().await;
            let entry = map
                .get_mut(recipe.id.as_str())
                .ok_or_else(|| DomainError::NotFound(format!("recipe {}", recipe.id)))?;
            if !Self::recipe_limits_ok(recipe, &entry.state, now) {
                return Ok(None);
            }
            // High-water mark: only NEW matching evidence may re-fire the
            // recipe; the same old event must not fire it twice.
            if let (Some(last), Some(latest)) = (entry.state.last_fired_at, decision.latest_match) {
                if latest <= last {
                    return Ok(None);
                }
            }
            entry.state.last_fired_at = Some(now);
            entry.state.executions_this_session += 1;
            entry.state.fired_last_hour.push(now);
            entry.state.fired_last_hour.retain(|at| {
                let age = now.signed_duration_since(*at).num_milliseconds();
                (0..3_600_000).contains(&age)
            });
            // Consume the matched events (bounded FIFO).
            for id in &decision.matched_observation_ids {
                if !entry.state.consumed_observations.contains(id) {
                    entry.state.consumed_observations.push(id.clone());
                }
            }
            while entry.state.consumed_observations.len() > 128 {
                entry.state.consumed_observations.remove(0);
            }
            let state = entry.state.clone();
            drop(map);
            self.persist_recipe_state(recipe.id.as_str(), &state).await;
        }
        // ---- AI decision gate: deterministic events never involve AI ----
        let ai_spec = recipe.ai.clone().unwrap_or_default();
        let mut ai_gate = serde_json::to_value(interaction_recipe::AiGateOutcome::Disabled {})
            .unwrap_or_default();
        if ai_spec.mode == interaction_recipe::AiAssistMode::WhenUncertain {
            let mut reason = uncertain_reason.clone();
            if reason.is_none() {
                let min_conf = decision
                    .matched_observation_ids
                    .iter()
                    .filter_map(|id| {
                        observations
                            .iter()
                            .find(|o| o.observation_id.as_str() == id)
                    })
                    .map(|o| o.confidence)
                    .fold(f64::INFINITY, f64::min);
                if min_conf.is_finite() && min_conf < ai_spec.min_confidence {
                    reason = Some(format!(
                        "matched evidence confidence {min_conf:.2} below threshold {:.2}",
                        ai_spec.min_confidence
                    ));
                }
            }
            if let Some(reason) = reason {
                // Defer: publish an assist request; the timeout task applies
                // the deterministic onUnavailable behavior.
                self.open_ai_assist(recipe, reason, &ai_spec).await;
                return Ok(Some(decision));
            }
            ai_gate = serde_json::to_value(interaction_recipe::AiGateOutcome::NotNeeded {
                reason: "evidence unambiguous; deterministic path".into(),
            })
            .unwrap_or_default();
        } else if recipe.ai.is_some() {
            ai_gate = serde_json::to_value(interaction_recipe::AiGateOutcome::NotNeeded {
                reason: format!("ai mode {:?} does not gate firing", ai_spec.mode),
            })
            .unwrap_or_default();
        }
        // Chance gate (surprise factor). The reservation above is deliberately
        // consumed even when chance skips: the opportunity was spent.
        if recipe.actuation.chance < 1.0 {
            let roll: f64 = rand::thread_rng().gen();
            if roll > recipe.actuation.chance {
                tracing::debug!(recipe = recipe.id.as_str(), roll, "skipped by chance");
                return Ok(Some(decision));
            }
        }
        // Jitter (bounded, cancellable).
        if let Some(jitter) = recipe
            .actuation
            .jitter
            .as_deref()
            .and_then(|d| interaction_recipe::parse_duration_ms(d).ok())
        {
            let delay = rand::thread_rng().gen_range(0..=jitter.min(5_000));
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {}
                _ = self.shutdown_token.cancelled() => return Ok(Some(decision)),
            }
        }
        // The chance/jitter window may outlive the user's intent: re-check
        // pause and emergency stop right before actually planning/executing.
        if self.is_estopped() || self.proactive_paused().await {
            tracing::info!(
                recipe = recipe.id.as_str(),
                "not firing: paused/stopped during jitter window"
            );
            return Ok(Some(decision));
        }
        // `ai-generated` is a real asynchronous Agent workflow, never a
        // placeholder message. Reserve the deterministic policy/budget,
        // create a read-only leased local-Agent Session, validate its
        // structured candidate, then render through the ordinary governor.
        // Observation ingestion returns immediately; the pending Session and
        // final action remain visible through the shared Runtime truth.
        if recipe.message.mode == interaction_core::MessageMode::AiGenerated {
            let class_text = recipe
                .message
                .extra
                .get("proactiveClass")
                .and_then(Value::as_str)
                .unwrap_or("suggestion");
            let class = crate::proactive::class_from_metadata(&BTreeMap::from([(
                "proactiveClass".into(),
                json!(class_text),
            )]));
            let dedup_key = format!(
                "recipe:{}:{}",
                recipe.id.as_str(),
                decision.matched_observation_ids.join(",")
            );
            if let Err(error) =
                Box::pin(self.start_proactive_agent_task(recipe.clone(), class, dedup_key)).await
            {
                let _ = self.store.audit(
                    "proactive.generation-not-started",
                    "runtime",
                    &json!({"reason": error.to_string()}),
                );
            }
            return Ok(Some(decision));
        }
        let mut plan = self.plan_from_recipe(recipe).await?;
        // Attach the fused context (facts after explicit-input override) so
        // the timeline shows what evidence the decision was based on.
        plan.metadata.insert("aiGate".to_string(), ai_gate);
        plan.metadata
            .insert("contextFacts".to_string(), json!(fused.facts));
        if !fused.missing.is_empty() {
            plan.metadata
                .insert("missingReceptors".to_string(), json!(fused.missing));
        }
        self.store.upsert_plan(&plan)?;
        let _ = self
            .execute_plan(&plan.plan_id, ActionSource::Autonomous, false)
            .await?;
        Ok(Some(decision))
    }

    // ------------------------------------------------------------------
    // Dynamic adapters (driver = builtin.push / builtin.mock-actuator)
    // ------------------------------------------------------------------

    /// Register a new push receptor at runtime (e.g. a custom webhook lane).
    pub async fn add_push_receptor(
        &self,
        id: &str,
        name: &str,
        category: &str,
        sensitive: bool,
    ) -> DomainResult<()> {
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        {
            return Err(DomainError::Validation(format!(
                "receptor id {id:?} has unsafe characters"
            )));
        }
        let manifest =
            interaction_adapter_sdk::ReceptorManifestBuilder::new(id, name, "builtin.push")
                .description("dynamically added push receptor")
                .category(category)
                .mode(interaction_core::ReceptorMode::Event)
                .sensitivity(
                    if sensitive {
                        interaction_core::Sensitivity::Personal
                    } else {
                        interaction_core::Sensitivity::Internal
                    },
                    sensitive,
                )
                .build();
        let receptor = adapters_builtin::PushReceptor::new(manifest);
        // Register first (checks duplicates), then track the push handle.
        self.registry.register_receptor(receptor.clone()).await?;
        // BTreeMap is behind Arc; use interior mutability via a lock-free trick:
        // push_receptors is only mutated here, guarded by the registry conflict
        // check above. We need a mutable map — switch to RwLock at the type level.
        self.dynamic_push
            .write()
            .await
            .insert(id.to_string(), receptor);
        Ok(())
    }

    /// Register another mock actuator (simulated device) at runtime, together
    /// with its paired device-status receptor so `observed` verification can
    /// close the loop for it too.
    pub async fn add_mock_actuator(&self, id: &str, channel: &str) -> DomainResult<()> {
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        {
            return Err(DomainError::Validation(format!(
                "actuator id {id:?} has unsafe characters"
            )));
        }
        let actuator = Arc::new(MockActuator::new(id, channel));
        let status = Arc::new(adapters_builtin::MockDeviceStatusReceptor::with_id(
            actuator.status_receptor_id(),
            actuator.device_state(),
        ));
        self.registry.register_actuator(actuator.clone()).await?;
        if let Err(e) = self.registry.register_receptor(status).await {
            // Roll back so the device is never left without observability.
            let _ = self
                .registry
                .unregister_actuator(&interaction_core::ActuatorId::new(id))
                .await;
            return Err(e);
        }
        Ok(())
    }

    /// Unregister an actuator; for mock devices also drops the paired
    /// device-status receptor.
    pub async fn remove_actuator(&self, id: &str) -> DomainResult<()> {
        self.registry
            .unregister_actuator(&interaction_core::ActuatorId::new(id))
            .await?;
        let paired = format!("{id}.device-status");
        let _ = self
            .registry
            .unregister_receptor(&ReceptorId::new(&paired))
            .await;
        Ok(())
    }

    /// 動態 push receptor 掛載（registry 註冊由呼叫端先完成）。
    pub(crate) async fn register_dynamic_push(&self, id: &str, receptor: Arc<PushReceptor>) {
        self.dynamic_push
            .write()
            .await
            .insert(id.to_string(), receptor);
    }

    /// Find a push receptor (builtin or dynamically added).
    pub async fn push_receptor(&self, id: &str) -> Option<Arc<PushReceptor>> {
        if let Some(r) = self.push_receptors.get(id) {
            return Some(r.clone());
        }
        self.dynamic_push.read().await.get(id).cloned()
    }

    // ------------------------------------------------------------------
    // Shared helpers
    // ------------------------------------------------------------------

    /// Persist a receipt. Returns `false` when the write was refused because
    /// the stored receipt is already terminal (e.g. e-stop sweep or watchdog
    /// got there first) — the caller's copy is then stale and must yield.
    pub(crate) async fn persist_receipt(
        &self,
        receipt: &ActionReceipt,
        channel: &str,
    ) -> DomainResult<bool> {
        self.store.upsert_receipt(receipt, channel)
    }

    pub(crate) fn emit_action_event(
        &self,
        event_type: EventType,
        receipt: &ActionReceipt,
        extra: Value,
    ) {
        let mut payload = json!({
            "actionId": receipt.action_id.as_str(),
            "planId": receipt.plan_id.as_str(),
            "actuatorId": receipt.actuator_id.as_str(),
            "status": receipt.current_status,
            "intent": receipt.intent,
        });
        if let (Some(obj), Some(extra_obj)) = (payload.as_object_mut(), extra.as_object()) {
            for (k, v) in extra_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
        self.events.publish(
            RuntimeEvent::new(event_type, Utc::now(), payload)
                .with_session(receipt.session_id.clone())
                .with_correlation(receipt.correlation_id.clone()),
        );
        // Character Protocol §11：action.* → work／acknowledge／claim-completed／
        // verified-success／unknown／failed（角色自己的呈現 actuator 不投影）。
        self.character_project_action(event_type, receipt);
    }

    /// Test/diagnostic wrapper for [`Self::charge_session_cost`].
    pub async fn charge_session_cost_public(&self, session_id: &SessionId, cost: f64) {
        self.charge_session_cost(session_id, cost).await;
    }

    /// Accumulate monetary spend so the governor's budget check sees it on the
    /// next authorization.
    pub(crate) async fn charge_session_cost(&self, session_id: &SessionId, cost: f64) {
        if cost <= 0.0 {
            return;
        }
        let mut guard = self.session.write().await;
        if let Some(session) = guard.as_mut() {
            if &session.session_id == session_id {
                session.monetary_spent += cost;
                let _ = self.store.upsert_session(session);
            }
        }
    }

    pub(crate) async fn release_invocation_cost(&self, action_id: &ActionId) {
        self.monetary_reservations
            .lock()
            .await
            .remove(action_id.as_str());
    }

    pub(crate) async fn commit_invocation_cost(&self, action_id: &ActionId) {
        let _authorization_guard = self.authorization_lock.lock().await;
        let reservation = self
            .monetary_reservations
            .lock()
            .await
            .remove(action_id.as_str());
        if let Some((session_id, cost)) = reservation {
            self.charge_session_cost(&session_id, cost).await;
        }
    }

    pub(crate) async fn track_session_usage(
        &self,
        session_id: &SessionId,
        channel: &str,
        receipt: &ActionReceipt,
    ) {
        let duration = receipt
            .effective_bounded_parameters
            .duration_ms
            .unwrap_or(0);
        if duration == 0 {
            return;
        }
        let mut guard = self.session.write().await;
        if let Some(session) = guard.as_mut() {
            if &session.session_id == session_id {
                *session
                    .channel_usage_ms
                    .entry(channel.to_string())
                    .or_insert(0) += duration;
                let _ = self.store.upsert_session(session);
            }
        }
    }

    // ------------------------------------------------------------------
    // Watchdog
    // ------------------------------------------------------------------

    fn spawn_watchdog(&self) {
        let runtime = self.clone();
        tokio::spawn(async move {
            let interval_ms = runtime.config.read().await.watchdog_interval_ms.max(100);
            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
            let mut tick: u64 = 0;
            loop {
                tokio::select! {
                    _ = runtime.shutdown_token.cancelled() => break,
                    _ = ticker.tick() => {}
                }
                tick += 1;
                // Sensor listen-window deadlines are hard: sweep every tick.
                if let Some(mic) = runtime.mic_receptor.as_ref() {
                    // Belt-and-braces: if estop is engaged, capture must not
                    // continue for even one more tick, whatever raced it open.
                    if runtime.is_estopped() {
                        mic.stop_listen();
                    } else {
                        let _ = mic.is_listening(); // enforces the deadline
                    }
                }
                // Presentation ack deadlines: unconfirmed commands go
                // Uncertain, never silently "completed".
                runtime.sweep_presentation().await;
                // Character gateway：heartbeat 逾時／過期／acknowledged→uncertain／
                // 桌面 presence 過期 → transport-closed。
                runtime.character_sweep().await;
                // Gateway：逾時 approval 自動拒絕＋殘留子程序清理。
                runtime.gateway_sweep().await;
                // 記憶到期清除（expiresAt 到＝停止使用並刪除）。
                if tick.is_multiple_of(60) {
                    runtime.sweep_memory().await;
                }
                // 知識新鮮度：過 reviewAfter 的 Active → Stale（確定性健檢）。
                if tick.is_multiple_of(600) {
                    let _ = runtime.knowledge_freshness_sweep().await;
                }
                // TTL sweep: expire non-terminal receipts past their deadline.
                if let Ok(open) = runtime.store.open_receipts() {
                    let now = Utc::now();
                    for mut receipt in open {
                        if receipt.expires_at.map(|e| now > e).unwrap_or(false) {
                            let _ = receipt.transition(ActionStatus::Expired, now);
                            receipt.push_error("ttl", "watchdog expired the action", now);
                            let _ = runtime.store.upsert_receipt(&receipt, "");
                            runtime.emit_action_event(
                                EventType::ActionExpired,
                                &receipt,
                                json!({}),
                            );
                        }
                    }
                }
                // Emergency-stop marker file (out-of-band trigger).
                let estop_file = runtime.paths.estop_file();
                if estop_file.exists() {
                    let _ = std::fs::remove_file(&estop_file);
                    if !runtime.is_estopped() {
                        let _ = runtime.emergency_stop("estop-file", None).await;
                    }
                }
                if tick.is_multiple_of(10) {
                    runtime.registry.refresh_health().await;
                }
                if tick.is_multiple_of(600) {
                    let hours = runtime.config.read().await.observation_retention_hours;
                    let cutoff = Utc::now() - chrono::Duration::hours(hours as i64);
                    let _ = runtime.store.prune_observations(cutoff);
                }
            }
        });
    }
}

pub(crate) fn parse_scope_public(scope_str: &str) -> DomainResult<ConsentScope> {
    parse_scope(scope_str)
}

fn parse_scope(scope_str: &str) -> DomainResult<ConsentScope> {
    let (kind, id) = scope_str
        .split_once(':')
        .ok_or_else(|| DomainError::Validation(format!("scope {scope_str:?}: expected kind:id")))?;
    if id.trim().is_empty() {
        return Err(DomainError::Validation(format!(
            "scope {scope_str:?}: empty id"
        )));
    }
    match kind {
        "channel" => Ok(ConsentScope::Channel(id.to_string())),
        "actuator" => Ok(ConsentScope::Actuator(id.to_string())),
        "receptor" => Ok(ConsentScope::Receptor(id.to_string())),
        "tool" => Ok(ConsentScope::ToolOperation(id.to_string())),
        other => Err(DomainError::Validation(format!(
            "unknown scope kind {other:?}"
        ))),
    }
}

/// JSON merge-patch (RFC 7396 flavor).
pub(crate) fn recipe_limits_ok_public(
    recipe: &Recipe,
    state: &RecipeState,
    now: Timestamp,
) -> bool {
    Runtime::recipe_limits_ok(recipe, state, now)
}

pub(crate) fn merge_json(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                if v.is_null() {
                    t.remove(k);
                } else {
                    merge_json(t.entry(k.clone()).or_insert(Value::Null), v);
                }
            }
        }
        (t, p) => *t = p.clone(),
    }
}

fn redact(v: &Value) -> Value {
    // Shallow redaction of obviously sensitive keys in audit payloads.
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                let lower = k.to_ascii_lowercase();
                if lower.contains("token") || lower.contains("secret") || lower.contains("password")
                {
                    out.insert(k.clone(), Value::String("[redacted]".into()));
                } else {
                    out.insert(k.clone(), redact(val));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        other => other.clone(),
    }
}

pub(crate) fn quiet_window_active(start: &str, end: &str, local: chrono::NaiveTime) -> bool {
    let parse = |s: &str| chrono::NaiveTime::parse_from_str(s, "%H:%M").ok();
    match (parse(start), parse(end)) {
        (Some(s), Some(e)) => {
            if s <= e {
                local >= s && local < e
            } else {
                local >= s || local < e
            }
        }
        _ => false,
    }
}

fn seed_default_recipes(config_service: &ConfigService) {
    let dir = config_service.paths.recipes_dir();
    let has_any = std::fs::read_dir(&dir)
        .map(|entries| entries.flatten().next().is_some())
        .unwrap_or(false);
    if has_any {
        return;
    }
    let default_recipe = include_str!("../assets/adaptive-task-completion.yaml");
    if let Ok(recipe) = interaction_recipe::parse_and_validate(default_recipe) {
        let _ = config_service.save_recipe(&recipe);
    }
}

/// True when a receptor formally declares `data.retention: none` — its derived
/// facts must never be persisted (privacy: e.g. the microphone's sound level).
fn declares_no_retention(m: &interaction_core::ReceptorManifest) -> bool {
    m.human
        .as_ref()
        .and_then(|h| h.data.as_ref())
        .map(|d| d.retention == interaction_core::DataRetention::None)
        .unwrap_or(false)
}

#[cfg(test)]
mod recipe_limit_tests {
    use super::*;

    #[test]
    fn hourly_recipe_limit_is_a_rolling_window() {
        let recipe = interaction_recipe::parse_and_validate(
            r#"
id: rolling-limit
name: rolling limit
trigger: { mode: any, steps: [{ receptor: manual.event }] }
decision: { objective: test, allowNoAction: true }
actuation: { candidates: [conversation], minChannels: 0, maxChannels: 1 }
limits: { maxPerHour: 1 }
"#,
        )
        .unwrap();
        let now = Utc::now();
        let stale = RecipeState {
            fired_last_hour: vec![now - chrono::Duration::hours(2)],
            ..Default::default()
        };
        assert!(Runtime::recipe_limits_ok(&recipe, &stale, now));

        let recent = RecipeState {
            fired_last_hour: vec![now - chrono::Duration::minutes(30)],
            ..Default::default()
        };
        assert!(!Runtime::recipe_limits_ok(&recipe, &recent, now));
    }
}
