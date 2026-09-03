//! Human-layer application services: proactive pause (distinct from emergency
//! stop), UI preferences, onboarding draft/commit, the human capability
//! projection, AI-assisted descriptions, the recipe AI decision gate, and
//! scenario simulation.
//!
//! Everything here goes through the same policy governor and storage as the
//! CLI / HTTP API / Tauri paths; nothing is a second source of truth.

use crate::orchestrator::{build_plan, ActuatorUsageHint, PlanRequest};
use crate::runtime::Runtime;
use chrono::Utc;
use interaction_core::{
    Availability, ConsentScope, DiscoveryContext, DomainError, DomainResult, EventType,
    Observation, ObservationQuery, PolicyConfig, QuietHours, SemanticIntent, Session, Timestamp,
};
use interaction_policy::{ActionSource, AuthorizationRequest, Governor, UsageContext};
use interaction_recipe::{
    evaluate_trigger, AiAssistMode, AiAssistSpec, AiGateOutcome, AiUnavailableBehavior, Recipe,
};
use interaction_registry::catalog::Catalog;
use interaction_registry::human_view::{
    manifest_hash, resolve_actuator_card, resolve_receptor_card, resolve_tool_card, ResolveContext,
};
use interaction_storage::Store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const PAUSE_META_KEY: &str = "proactive_pause";
const PREFS_META_KEY: &str = "ui_prefs";
const ONBOARDING_META_KEY: &str = "onboarding";
const ONBOARDING_DRAFT_META_KEY: &str = "onboarding_draft";
const MAX_DRAFT_BYTES: usize = 64 * 1024;
const MAX_PENDING_ASSISTS: usize = 32;

// ---------------------------------------------------------------------------
// Proactive pause (an ordinary user control — NOT emergency stop)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PauseState {
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<Timestamp>,
}

impl PauseState {
    pub(crate) fn load(store: &Store) -> Self {
        store
            .get_meta(PAUSE_META_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn expired(&self, now: Timestamp) -> bool {
        self.paused && self.until.map(|u| now >= u).unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// UI preferences
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiPreferences {
    /// `simple` (default) or `advanced`.
    pub mode: String,
    pub locale: String,
    /// `system` / `dark` / `light`; presentation only.
    pub appearance: String,
    /// Control Center zoom, bounded to preserve reachability at 390px.
    pub scale_percent: u16,
    /// Explicitly reduce non-essential motion in addition to the OS setting.
    pub reduce_motion: bool,
    /// User-disabled local connectors. Runtime session creation enforces this.
    #[serde(default)]
    pub disabled_agents: Vec<String>,
    /// User-selected primary agent per semantic role. This affects routing
    /// suggestions only; it never falls back to another provider implicitly.
    pub agent_routes: BTreeMap<String, String>,
    /// Presentation-only custom names, keyed `receptor:<id>` / `actuator:<id>`
    /// / `tool:<name>` / `recipe:<id>`. Never changes safety facts.
    #[serde(default)]
    pub custom_names: BTreeMap<String, String>,
    /// 首次成功體驗（FirstSuccess）已看過。純 UI 旗標：不影響任何權限、
    /// 同意或安全事實；host 沒保存時前端會誠實退回 localStorage，所以這裡
    /// 必須真的存下並在 GET 回傳。
    #[serde(default)]
    pub first_success_seen: bool,
    pub schema_version: String,
}

impl Default for UiPreferences {
    fn default() -> Self {
        let agent_routes = BTreeMap::from([
            ("conversation".into(), "claude-code".into()),
            ("programming".into(), "codex".into()),
            ("knowledge".into(), "claude-code".into()),
            ("review".into(), "claude-code".into()),
        ]);
        Self {
            mode: "simple".into(),
            locale: "zh-TW".into(),
            appearance: "system".into(),
            scale_percent: 100,
            reduce_motion: false,
            disabled_agents: Vec::new(),
            agent_routes,
            custom_names: BTreeMap::new(),
            first_success_seen: false,
            schema_version: interaction_core::SCHEMA_VERSION.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Onboarding
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OnboardingCommit {
    #[serde(default)]
    pub enable_receptors: Vec<String>,
    #[serde(default)]
    pub disable_receptors: Vec<String>,
    #[serde(default)]
    pub enable_actuators: Vec<String>,
    #[serde(default)]
    pub disable_actuators: Vec<String>,
    /// Merge patch applied through the normal governor-validated policy path.
    #[serde(default)]
    pub policy_patch: Option<Value>,
    /// Merge patch for UI preferences.
    #[serde(default)]
    pub preferences: Option<Value>,
    /// Ids of starter recipes to install (see [`starter_recipes`]).
    #[serde(default)]
    pub starter_recipes: Vec<String>,
}

/// Built-in starter recipes offered by the onboarding wizard.
/// (id, zh-TW title, YAML)
pub fn starter_recipes() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "starter-task-complete",
            "任務完成時，用最低干擾方式回應",
            include_str!("../assets/starter-task-complete.yaml"),
        ),
        (
            "starter-quiet-log",
            "安靜時段只記錄、不打擾",
            include_str!("../assets/starter-quiet-log.yaml"),
        ),
        (
            "starter-device-warning",
            "裝置或服務異常時通知",
            include_str!("../assets/starter-device-warning.yaml"),
        ),
    ]
}

// ---------------------------------------------------------------------------
// AI assist requests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAssist {
    pub request_id: String,
    pub recipe_id: String,
    pub reason: String,
    pub created_at: Timestamp,
    pub deadline: Timestamp,
    pub on_unavailable: AiUnavailableBehavior,
    /// Data categories the recipe allows sharing; raw values are never
    /// embedded in the event payload.
    pub data_scope: Vec<String>,
    /// When true, only a human surface (desktop IPC) may resolve `proceed`.
    pub require_human_confirmation: bool,
}

// ---------------------------------------------------------------------------
// Scenario simulation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SimScenario {
    #[serde(default)]
    pub quiet_hours: bool,
    #[serde(default)]
    pub missing_consent: bool,
    #[serde(default)]
    pub actuator_offline: Vec<String>,
    #[serde(default)]
    pub ai_unavailable: bool,
    #[serde(default)]
    pub low_confidence: bool,
    #[serde(default)]
    pub stale_observations: bool,
    #[serde(default)]
    pub recently_fired: bool,
    #[serde(default)]
    pub emergency_stop: bool,
    /// Synthetic trigger event (never stored, never executed).
    #[serde(default)]
    pub event: Option<SimEvent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimEvent {
    pub receptor: String,
    #[serde(default)]
    pub facts: BTreeMap<String, Value>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

impl Runtime {
    // -------------------------------------------------------------------
    // Pause
    // -------------------------------------------------------------------

    /// Current pause state with lazy expiry.
    pub async fn pause_status(&self) -> PauseState {
        let now = Utc::now();
        let expired = { self.pause.read().await.expired(now) };
        if expired {
            // Update memory first (short-held guard, no blocking I/O inside),
            // then persist — the same ordering as pause/resume so a concurrent
            // writer cannot clobber a newer state in storage.
            let cleared = {
                let mut guard = self.pause.write().await;
                if guard.expired(now) {
                    *guard = PauseState::default();
                    true
                } else {
                    false
                }
            };
            if cleared {
                let _ = self
                    .store
                    .set_meta(PAUSE_META_KEY, &json!(PauseState::default()).to_string());
                self.events.emit(
                    EventType::ProactiveResumed,
                    json!({"reason": "pause window elapsed"}),
                );
                self.character_project_proactive(false);
            }
        }
        self.pause.read().await.clone()
    }

    /// True when recipe-triggered proactive interactions must not fire.
    pub(crate) async fn proactive_paused(&self) -> bool {
        self.pause_status().await.paused
    }

    pub async fn pause_proactive(
        &self,
        until: Option<Timestamp>,
        reason: Option<String>,
        actor: &str,
    ) -> DomainResult<PauseState> {
        if let Some(u) = until {
            if u <= Utc::now() {
                return Err(DomainError::Validation(
                    "pause 'until' must be in the future".into(),
                ));
            }
        }
        let state = PauseState {
            paused: true,
            until,
            reason: reason.clone(),
            paused_at: Some(Utc::now()),
        };
        *self.pause.write().await = state.clone();
        self.store
            .set_meta(PAUSE_META_KEY, &json!(state).to_string())?;
        self.events.emit(
            EventType::ProactivePaused,
            json!({"until": until, "reason": reason}),
        );
        // Character Protocol §11：proactive.paused → rest。
        self.character_project_proactive(true);
        self.store
            .audit("proactive.paused", actor, &json!({"until": until}))?;
        Ok(state)
    }

    pub async fn resume_proactive(&self, actor: &str) -> DomainResult<PauseState> {
        let state = PauseState::default();
        *self.pause.write().await = state.clone();
        self.store
            .set_meta(PAUSE_META_KEY, &json!(state).to_string())?;
        self.events
            .emit(EventType::ProactiveResumed, json!({"actor": actor}));
        self.character_project_proactive(false);
        self.store.audit("proactive.resumed", actor, &json!({}))?;
        Ok(state)
    }

    // -------------------------------------------------------------------
    // UI preferences
    // -------------------------------------------------------------------

    pub async fn ui_preferences(&self) -> UiPreferences {
        self.store
            .get_meta(PREFS_META_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub async fn update_ui_preferences(&self, patch: Value) -> DomainResult<UiPreferences> {
        let current = self.ui_preferences().await;
        let mut merged =
            serde_json::to_value(&current).map_err(|e| DomainError::Internal(e.to_string()))?;
        crate::runtime::merge_json(&mut merged, &patch);
        let updated: UiPreferences = serde_json::from_value(merged)
            .map_err(|e| DomainError::Validation(format!("ui preferences patch: {e}")))?;
        if !matches!(updated.mode.as_str(), "simple" | "advanced") {
            return Err(DomainError::Validation(format!(
                "mode must be 'simple' or 'advanced', got {:?}",
                updated.mode
            )));
        }
        if !matches!(updated.appearance.as_str(), "system" | "dark" | "light") {
            return Err(DomainError::Validation(format!(
                "appearance must be system, dark, or light; got {:?}",
                updated.appearance
            )));
        }
        if !(80..=150).contains(&updated.scale_percent) {
            return Err(DomainError::Validation(
                "scalePercent must be within 80..150".into(),
            ));
        }
        if updated.disabled_agents.len() > 2
            || updated
                .disabled_agents
                .iter()
                .any(|id| !matches!(id.as_str(), "codex" | "claude-code"))
        {
            return Err(DomainError::Validation(
                "disabledAgents may contain only codex and claude-code".into(),
            ));
        }
        const ROUTE_KEYS: &[&str] = &["conversation", "programming", "knowledge", "review"];
        if updated.agent_routes.len() > ROUTE_KEYS.len()
            || updated.agent_routes.iter().any(|(role, agent)| {
                !ROUTE_KEYS.contains(&role.as_str())
                    || !matches!(agent.as_str(), "codex" | "claude-code" | "none")
            })
        {
            return Err(DomainError::Validation(
                "agentRoutes supports conversation/programming/knowledge/review with codex, claude-code, or none"
                    .into(),
            ));
        }
        if updated.custom_names.len() > 256 {
            return Err(DomainError::Validation(
                "too many custom names (max 256)".into(),
            ));
        }
        self.store
            .set_meta(PREFS_META_KEY, &json!(updated).to_string())?;
        Ok(updated)
    }

    // -------------------------------------------------------------------
    // Onboarding
    // -------------------------------------------------------------------

    pub async fn onboarding_state(&self) -> Value {
        let meta: Value = self
            .store
            .get_meta(ONBOARDING_META_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| json!({"completed": false}));
        let draft: Option<Value> = self
            .store
            .get_meta(ONBOARDING_DRAFT_META_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok());
        let starters: Vec<Value> = starter_recipes()
            .iter()
            .map(|(id, title, _)| json!({"id": id, "title": title}))
            .collect();
        json!({
            "completed": meta.get("completed").and_then(Value::as_bool).unwrap_or(false),
            "completedAt": meta.get("completedAt").cloned().unwrap_or(Value::Null),
            "draft": draft,
            "starterRecipes": starters,
        })
    }

    pub async fn save_onboarding_draft(&self, draft: Value) -> DomainResult<()> {
        let raw = draft.to_string();
        if raw.len() > MAX_DRAFT_BYTES {
            return Err(DomainError::Validation(format!(
                "onboarding draft too large ({} bytes, max {MAX_DRAFT_BYTES})",
                raw.len()
            )));
        }
        self.store.set_meta(ONBOARDING_DRAFT_META_KEY, &raw)?;
        Ok(())
    }

    /// Validate everything first, then apply: policy patch → component
    /// enable/disable → starter recipes → preferences → mark completed.
    /// The commit path only uses the same governor-validated services as the
    /// rest of the system; it cannot grant anything policy would refuse.
    pub async fn commit_onboarding(&self, commit: OnboardingCommit) -> DomainResult<Value> {
        // ---- validation pass (no side effects) ----
        let snapshot = self
            .capabilities(&DiscoveryContext {
                include_unavailable: true,
                ..Default::default()
            })
            .await;
        let receptor_ids: Vec<&str> = snapshot.receptors.iter().map(|m| m.id.as_str()).collect();
        let actuator_ids: Vec<&str> = snapshot.actuators.iter().map(|m| m.id.as_str()).collect();
        for id in commit
            .enable_receptors
            .iter()
            .chain(commit.disable_receptors.iter())
        {
            if !receptor_ids.contains(&id.as_str()) {
                return Err(DomainError::NotFound(format!("receptor {id}")));
            }
        }
        for id in commit
            .enable_actuators
            .iter()
            .chain(commit.disable_actuators.iter())
        {
            if !actuator_ids.contains(&id.as_str()) {
                return Err(DomainError::NotFound(format!("actuator {id}")));
            }
        }
        // Consent-gated (sensitive/physical/external) components can NOT be
        // switched on through the bulk onboarding path — they require the
        // explicit per-component enable + consent flow. The wizard never
        // offers them; this guards the API surface.
        for id in &commit.enable_receptors {
            if let Some(m) = snapshot
                .receptors
                .iter()
                .find(|m| m.id.as_str() == id.as_str())
            {
                if m.requires_consent {
                    return Err(DomainError::ConsentRequired(format!(
                        "receptor {id} is consent-gated; enable it explicitly and grant consent instead"
                    )));
                }
            }
        }
        for id in &commit.enable_actuators {
            if let Some(m) = snapshot
                .actuators
                .iter()
                .find(|m| m.id.as_str() == id.as_str())
            {
                if m.requires_consent {
                    return Err(DomainError::ConsentRequired(format!(
                        "actuator {id} is consent-gated; enable it explicitly and grant consent instead"
                    )));
                }
            }
        }
        let starters = starter_recipes();
        for id in &commit.starter_recipes {
            if !starters.iter().any(|(sid, _, _)| sid == id) {
                return Err(DomainError::NotFound(format!("starter recipe {id}")));
            }
        }
        // Pre-validate the policy patch by test-merging it.
        if let Some(patch) = &commit.policy_patch {
            let current = self.policy().await;
            let mut merged =
                serde_json::to_value(&current).map_err(|e| DomainError::Internal(e.to_string()))?;
            crate::runtime::merge_json(&mut merged, patch);
            let _parsed: PolicyConfig = serde_json::from_value(merged)
                .map_err(|e| DomainError::Validation(format!("policy patch: {e}")))?;
        }

        // ---- apply pass ----
        let mut applied = Vec::new();
        if let Some(patch) = &commit.policy_patch {
            self.update_policy(patch.clone()).await?;
            applied.push(json!({"step": "policy", "ok": true}));
        }
        for id in &commit.enable_receptors {
            self.registry
                .set_receptor_enabled(&id.as_str().into(), true)
                .await?;
        }
        for id in &commit.disable_receptors {
            self.registry
                .set_receptor_enabled(&id.as_str().into(), false)
                .await?;
        }
        for id in &commit.enable_actuators {
            self.registry
                .set_actuator_enabled(&id.as_str().into(), true)
                .await?;
        }
        for id in &commit.disable_actuators {
            self.registry
                .set_actuator_enabled(&id.as_str().into(), false)
                .await?;
        }
        applied.push(json!({
            "step": "components",
            "receptorsEnabled": commit.enable_receptors,
            "receptorsDisabled": commit.disable_receptors,
            "actuatorsEnabled": commit.enable_actuators,
            "actuatorsDisabled": commit.disable_actuators,
        }));
        let mut installed = Vec::new();
        for id in &commit.starter_recipes {
            if let Some((_, _, yaml)) = starters.iter().find(|(sid, _, _)| sid == id) {
                self.upsert_recipe_text(yaml).await?;
                installed.push(id.clone());
            }
        }
        if !installed.is_empty() {
            applied.push(json!({"step": "starterRecipes", "installed": installed}));
        }
        if let Some(prefs) = commit.preferences {
            self.update_ui_preferences(prefs).await?;
            applied.push(json!({"step": "preferences", "ok": true}));
        }
        self.store.set_meta(
            ONBOARDING_META_KEY,
            &json!({"completed": true, "completedAt": Utc::now()}).to_string(),
        )?;
        // A committed onboarding leaves no ambiguous half-done draft behind.
        self.store.set_meta(ONBOARDING_DRAFT_META_KEY, "null")?;
        self.store
            .audit("onboarding.committed", "api", &json!({"applied": applied}))?;
        Ok(json!({"completed": true, "applied": applied}))
    }

    // -------------------------------------------------------------------
    // Human capability projection
    // -------------------------------------------------------------------

    /// Human cards for every capability, resolved deterministically from
    /// adapter manifest → catalog → fallback, with user overrides and
    /// hash-validated AI supplements. Shared verbatim by API, CLI and UI.
    pub async fn human_capabilities(&self, locale: &str, include_unavailable: bool) -> Value {
        let snapshot = self
            .capabilities(&DiscoveryContext {
                include_unavailable,
                ..Default::default()
            })
            .await;
        let prefs = self.ui_preferences().await;
        let catalog = Catalog::builtin();
        let locale = if locale.is_empty() {
            prefs.locale.as_str()
        } else {
            locale
        };

        let mut receptors = Vec::new();
        for m in &snapshot.receptors {
            let hash = stable_manifest_hash(m);
            let id = m.id.as_str();
            let ai = self
                .store
                .ai_description("receptor", id, locale, &hash)
                .ok()
                .flatten();
            let ctx = ResolveContext {
                locale,
                user_name: prefs
                    .custom_names
                    .get(&format!("receptor:{id}"))
                    .map(String::as_str),
                ai_description: ai.as_deref(),
            };
            let card = resolve_receptor_card(m, catalog, &ctx);
            receptors.push(card_with_hash(card, &hash));
        }
        let mut actuators = Vec::new();
        for m in &snapshot.actuators {
            let hash = stable_manifest_hash(m);
            let id = m.id.as_str();
            let ai = self
                .store
                .ai_description("actuator", id, locale, &hash)
                .ok()
                .flatten();
            let ctx = ResolveContext {
                locale,
                user_name: prefs
                    .custom_names
                    .get(&format!("actuator:{id}"))
                    .map(String::as_str),
                ai_description: ai.as_deref(),
            };
            let card = resolve_actuator_card(m, catalog, &ctx);
            actuators.push(card_with_hash(card, &hash));
        }
        let mut tools = Vec::new();
        for m in &snapshot.tool_operations {
            let hash = stable_manifest_hash(m);
            let ai = self
                .store
                .ai_description("tool", &m.name, locale, &hash)
                .ok()
                .flatten();
            let ctx = ResolveContext {
                locale,
                user_name: prefs
                    .custom_names
                    .get(&format!("tool:{}", m.name))
                    .map(String::as_str),
                ai_description: ai.as_deref(),
            };
            let card = resolve_tool_card(m, catalog, &ctx);
            tools.push(card_with_hash(card, &hash));
        }
        json!({
            "locale": locale,
            "catalogVersion": catalog.version,
            "capabilityVersion": snapshot.version,
            "generatedAt": snapshot.generated_at,
            "constraints": snapshot.constraints,
            "receptors": receptors,
            "actuators": actuators,
            "toolOperations": tools,
        })
    }

    /// Store an AI-assisted description. `expected_hash` must match the
    /// current manifest hash — a stale description is refused, and a manifest
    /// change silently invalidates old ones at read time.
    pub async fn set_capability_ai_description(
        &self,
        kind: &str,
        id: &str,
        locale: &str,
        text: &str,
        expected_hash: &str,
    ) -> DomainResult<Value> {
        if !matches!(kind, "receptor" | "actuator" | "tool") {
            return Err(DomainError::Validation(format!(
                "kind must be receptor|actuator|tool, got {kind:?}"
            )));
        }
        if text.len() > 4096 {
            return Err(DomainError::Validation(
                "description too long (max 4096 bytes)".into(),
            ));
        }
        let current = self.current_manifest_hash(kind, id).await?;
        if current != expected_hash {
            return Err(DomainError::Conflict(format!(
                "manifest changed (current hash {current}); re-read the capability before describing it"
            )));
        }
        self.store
            .set_ai_description(kind, id, locale, &current, text)?;
        self.store.audit(
            "ai-description.set",
            "ai",
            &json!({"kind": kind, "id": id, "locale": locale}),
        )?;
        Ok(json!({"kind": kind, "id": id, "locale": locale, "manifestHash": current}))
    }

    async fn current_manifest_hash(&self, kind: &str, id: &str) -> DomainResult<String> {
        let snapshot = self
            .capabilities(&DiscoveryContext {
                include_unavailable: true,
                ..Default::default()
            })
            .await;
        match kind {
            "receptor" => snapshot
                .receptors
                .iter()
                .find(|m| m.id.as_str() == id)
                .map(stable_manifest_hash)
                .ok_or_else(|| DomainError::NotFound(format!("receptor {id}"))),
            "actuator" => snapshot
                .actuators
                .iter()
                .find(|m| m.id.as_str() == id)
                .map(stable_manifest_hash)
                .ok_or_else(|| DomainError::NotFound(format!("actuator {id}"))),
            _ => snapshot
                .tool_operations
                .iter()
                .find(|m| m.name == id)
                .map(stable_manifest_hash)
                .ok_or_else(|| DomainError::NotFound(format!("tool {id}"))),
        }
    }

    // -------------------------------------------------------------------
    // AI assist requests (decision gate deferrals)
    // -------------------------------------------------------------------

    pub async fn pending_ai_assists(&self) -> Vec<PendingAssist> {
        let now = Utc::now();
        self.ai_assists
            .read()
            .await
            .values()
            .filter(|a| a.deadline > now)
            .cloned()
            .collect()
    }

    /// Open an assist request: publish the event, then arm a cancellable
    /// timeout that applies the deterministic `on_unavailable` behavior.
    pub(crate) async fn open_ai_assist(
        &self,
        recipe: &Recipe,
        reason: String,
        spec: &AiAssistSpec,
    ) {
        // Daily cap: deterministic fallback rather than unbounded AI calls.
        // The counter is only tracked when a cap is configured (no unbounded
        // meta-key growth otherwise) and only charged once a request is
        // actually opened.
        let cap_key = spec.daily_call_cap.map(|_| {
            format!(
                "ai_assist_count:{}:{}",
                recipe.id.as_str(),
                Utc::now().format("%Y-%m-%d")
            )
        });
        if let (Some(cap), Some(key)) = (spec.daily_call_cap, cap_key.as_deref()) {
            let count: u32 = self
                .store
                .get_meta(key)
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if count >= cap {
                tracing::info!(recipe = recipe.id.as_str(), "AI assist daily cap reached");
                self.apply_ai_unavailable(
                    recipe.id.as_str(),
                    spec.on_unavailable,
                    spec.require_human_confirmation,
                    "daily call cap reached",
                )
                .await;
                return;
            }
        }

        let request_id = format!("assist-{}", uuid::Uuid::new_v4());
        let deadline =
            Utc::now() + chrono::Duration::milliseconds(spec.max_wait_ms.min(60_000) as i64);
        let assist = PendingAssist {
            request_id: request_id.clone(),
            recipe_id: recipe.id.as_str().to_string(),
            reason: reason.clone(),
            created_at: Utc::now(),
            deadline,
            on_unavailable: spec.on_unavailable,
            data_scope: spec.data_scope.clone(),
            require_human_confirmation: spec.require_human_confirmation,
        };
        // Bound check and insert under ONE write lock (no TOCTOU).
        let inserted = {
            let mut map = self.ai_assists.write().await;
            if map.len() >= MAX_PENDING_ASSISTS {
                false
            } else {
                map.insert(request_id.clone(), assist);
                true
            }
        };
        if !inserted {
            self.apply_ai_unavailable(
                recipe.id.as_str(),
                spec.on_unavailable,
                spec.require_human_confirmation,
                "assist queue full",
            )
            .await;
            return;
        }
        if let (Some(_), Some(key)) = (spec.daily_call_cap, cap_key.as_deref()) {
            let count: u32 = self
                .store
                .get_meta(key)
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let _ = self.store.set_meta(key, &(count + 1).to_string());
        }
        self.events.emit(
            EventType::AiAssistRequested,
            json!({
                "requestId": request_id,
                "recipeId": recipe.id.as_str(),
                "reason": reason,
                "deadline": deadline,
                "dataScope": spec.data_scope,
                "onUnavailable": spec.on_unavailable,
                "requireHumanConfirmation": spec.require_human_confirmation,
            }),
        );
        let _ = self.store.audit(
            "ai-assist.requested",
            "runtime",
            &json!({"recipeId": recipe.id.as_str(), "reason": reason}),
        );
        // Cancellable timeout task; owner = runtime, exits on shutdown.
        let rt = self.clone_handle();
        let spec = spec.clone();
        let wait = std::time::Duration::from_millis(spec.max_wait_ms.min(60_000));
        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {
                    // Claim: whoever removes the entry acts on it.
                    let claimed = rt.ai_assists.write().await.remove(&request_id);
                    if let Some(assist) = claimed {
                        rt.apply_ai_unavailable(
                            &assist.recipe_id,
                            assist.on_unavailable,
                            assist.require_human_confirmation,
                            "no AI host responded in time",
                        )
                        .await;
                    }
                }
                _ = rt.shutdown_token.cancelled() => {}
            }
        });
    }

    /// Deterministic behavior when AI never answered (or was capped/absent).
    async fn apply_ai_unavailable(
        &self,
        recipe_id: &str,
        on_unavailable: AiUnavailableBehavior,
        require_human_confirmation: bool,
        why: &str,
    ) {
        let outcome = match on_unavailable {
            AiUnavailableBehavior::Fallback if require_human_confirmation => {
                // A recipe that demands human confirmation must not auto-fire
                // just because nobody answered: downgrade to no-action.
                AiGateOutcome::UnavailableNoAction {
                    reason: format!("{why}; human confirmation required, none given"),
                }
            }
            AiUnavailableBehavior::Fallback => {
                if self.proactive_paused().await || self.is_estopped() {
                    AiGateOutcome::UnavailableNoAction {
                        reason: format!("{why}; runtime paused/stopped"),
                    }
                } else {
                    match self.fire_recipe_deterministic(recipe_id, why).await {
                        Ok(()) => AiGateOutcome::UnavailableFallback {
                            reason: why.to_string(),
                        },
                        Err(e) => AiGateOutcome::UnavailableNoAction {
                            reason: format!("{why}; fallback failed: {e}"),
                        },
                    }
                }
            }
            AiUnavailableBehavior::NoAction => AiGateOutcome::UnavailableNoAction {
                reason: why.to_string(),
            },
        };
        self.events.emit(
            EventType::AiAssistResolved,
            json!({"recipeId": recipe_id, "resolution": outcome}),
        );
        let _ = self.store.audit(
            "ai-assist.unavailable",
            "runtime",
            &json!({"recipeId": recipe_id, "resolution": outcome}),
        );
    }

    /// Answer a pending assist request. `decision` is `proceed` (fire the
    /// deterministic plan) or `no-action`. `human_confirmed` is true only for
    /// human surfaces (desktop IPC); the HTTP API always passes false, so a
    /// recipe with `ai.requireHumanConfirmation: true` cannot be self-approved
    /// by the AI host that received the assist request.
    pub async fn resolve_ai_assist(
        &self,
        request_id: &str,
        decision: &str,
        note: Option<String>,
        human_confirmed: bool,
    ) -> DomainResult<Value> {
        // Validate BEFORE claiming so a bad request never un-parks the entry
        // (re-inserting would race the timeout task's claim-by-removal).
        if !matches!(decision, "proceed" | "no-action") {
            return Err(DomainError::Validation(format!(
                "decision must be 'proceed' or 'no-action', got {decision:?}"
            )));
        }
        {
            let map = self.ai_assists.read().await;
            let assist = map.get(request_id).ok_or_else(|| {
                DomainError::NotFound(format!(
                    "assist request {request_id} (expired or already resolved)"
                ))
            })?;
            if assist.deadline <= Utc::now() {
                return Err(DomainError::Expired(format!(
                    "assist request {request_id} deadline passed; deterministic fallback applies"
                )));
            }
            if decision == "proceed" && assist.require_human_confirmation && !human_confirmed {
                return Err(DomainError::ApprovalRequired(format!(
                    "assist request {request_id} requires explicit human confirmation"
                )));
            }
        }
        // Claim (whoever removes the entry acts on it).
        let Some(assist) = self.ai_assists.write().await.remove(request_id) else {
            return Err(DomainError::NotFound(format!(
                "assist request {request_id} (expired or already resolved)"
            )));
        };
        let outcome = match decision {
            "proceed" => {
                if self.proactive_paused().await {
                    json!({"decision": "proceed", "result": "skipped", "reason": "proactive interactions are paused"})
                } else {
                    match self
                        .fire_recipe_deterministic(&assist.recipe_id, "ai-approved")
                        .await
                    {
                        Ok(()) => json!({"decision": "proceed", "result": "fired"}),
                        Err(e) => {
                            json!({"decision": "proceed", "result": "failed", "reason": e.to_string()})
                        }
                    }
                }
            }
            _ => json!({"decision": "no-action", "result": "skipped"}),
        };
        // The resolution event fires regardless of the execution result so the
        // timeline never shows a dangling request.
        self.events.emit(
            EventType::AiAssistResolved,
            json!({
                "requestId": request_id,
                "recipeId": assist.recipe_id,
                "resolution": outcome,
                "note": note,
            }),
        );
        self.store.audit(
            "ai-assist.resolved",
            if human_confirmed { "user" } else { "ai" },
            &json!({"requestId": request_id, "recipeId": assist.recipe_id, "decision": decision}),
        )?;
        Ok(json!({"requestId": request_id, "recipeId": assist.recipe_id, "outcome": outcome}))
    }

    /// Fire a recipe's deterministic plan (trigger already satisfied earlier).
    /// The pending-assist window may have outlived the user's intent, so the
    /// recipe's enabled flag and recipe-level consents are re-checked here;
    /// policy then fully applies to every step as usual.
    async fn fire_recipe_deterministic(&self, recipe_id: &str, why: &str) -> DomainResult<()> {
        if self.is_estopped() {
            return Err(DomainError::PolicyBlocked("emergency stop engaged".into()));
        }
        let recipe = self.get_recipe(recipe_id).await?;
        if !recipe.enabled {
            return Err(DomainError::PolicyBlocked(format!(
                "recipe {recipe_id} was disabled while the assist was pending"
            )));
        }
        if !recipe.consent.required.is_empty() {
            let session = self.require_session().await?;
            let now = Utc::now();
            for scope_str in &recipe.consent.required {
                let scope = crate::runtime::parse_scope_public(scope_str)?;
                if !session.has_consent(&scope, now) {
                    return Err(DomainError::ConsentRequired(format!(
                        "recipe consent {scope_str} is not (or no longer) granted"
                    )));
                }
            }
        }
        let mut plan = self.plan_from_recipe_public(&recipe).await?;
        plan.metadata.insert(
            "aiGate".to_string(),
            json!({"outcome": "deferred-then-deterministic", "reason": why}),
        );
        self.store.upsert_plan(&plan)?;
        let _ = self
            .execute_plan(&plan.plan_id, ActionSource::Autonomous, false)
            .await?;
        Ok(())
    }

    // -------------------------------------------------------------------
    // Scenario simulation (no side effects, same decision code)
    // -------------------------------------------------------------------

    /// Simulate one recipe under a what-if scenario. Reuses the same pure
    /// pieces the live path uses: `evaluate_trigger`, `build_plan`,
    /// `Governor::authorize`. Never executes, never stores observations.
    pub async fn simulate_recipe_scenario(
        &self,
        id: &str,
        scenario: SimScenario,
    ) -> DomainResult<Value> {
        let recipe = self.get_recipe(id).await?;
        let now = Utc::now();
        let mut stages: Vec<Value> = Vec::new();

        // ---- observations (stored + synthetic) ----
        let mut observations = self
            .store
            .query_observations(&ObservationQuery {
                since: Some(now - chrono::Duration::minutes(10)),
                limit: Some(200),
                ..Default::default()
            })
            .unwrap_or_default();
        if let Some(event) = &scenario.event {
            let mut obs = Observation::now(event.receptor.as_str().into(), "simulation", now);
            obs.facts = event.facts.clone();
            if let Some(c) = event.confidence {
                obs.confidence = c.clamp(0.0, 1.0);
            }
            if scenario.low_confidence {
                obs.confidence = obs.confidence.min(0.2);
            }
            observations.push(obs);
        }
        if scenario.stale_observations {
            // Everything is too old: trigger sees an empty fresh set.
            observations.clear();
        }

        // ---- trigger ----
        let decision = evaluate_trigger(&recipe, &observations, now);
        stages.push(json!({
            "stage": "trigger",
            "ok": decision.fired,
            "detail": decision,
        }));

        // ---- limits ----
        let mut state = {
            let map = self.recipes.read().await;
            map.get(id).map(|e| e.state.clone()).unwrap_or_default()
        };
        if scenario.recently_fired {
            state.last_fired_at = Some(now);
            state.fired_last_hour.push(now);
        }
        let limits_ok = crate::runtime::recipe_limits_ok_public(&recipe, &state, now);
        stages.push(json!({
            "stage": "limits",
            "ok": limits_ok,
            "detail": if limits_ok { "冷卻與頻率限制皆未超過" } else { "冷卻時間或頻率上限尚未解除" },
        }));

        // ---- consent ----
        let session = self.current_session().await;
        let sim_session = if scenario.missing_consent {
            Session::new(now, Some("simulation-no-consent".into()), None)
        } else {
            session
                .clone()
                .unwrap_or_else(|| Session::new(now, Some("simulation".into()), None))
        };
        let mut missing_scopes = Vec::new();
        for scope_str in &recipe.consent.required {
            if let Some((kind, cid)) = scope_str.split_once(':') {
                let scope = match kind {
                    "channel" => ConsentScope::Channel(cid.into()),
                    "actuator" => ConsentScope::Actuator(cid.into()),
                    "receptor" => ConsentScope::Receptor(cid.into()),
                    _ => ConsentScope::ToolOperation(cid.into()),
                };
                if !sim_session.has_consent(&scope, now) {
                    missing_scopes.push(scope_str.clone());
                }
            }
        }
        stages.push(json!({
            "stage": "consent",
            "ok": missing_scopes.is_empty(),
            "missing": missing_scopes,
            "sessionActive": session.is_some(),
        }));

        // ---- AI gate ----
        let ai_spec = recipe.ai.clone().unwrap_or_default();
        let gate: AiGateOutcome = match ai_spec.mode {
            AiAssistMode::Never => AiGateOutcome::Disabled {},
            AiAssistMode::WhenUncertain => {
                let uncertain = scenario.low_confidence
                    || decision
                        .matched_observation_ids
                        .iter()
                        .filter_map(|oid| {
                            observations
                                .iter()
                                .find(|o| o.observation_id.as_str() == oid)
                        })
                        .any(|o| o.confidence < ai_spec.min_confidence);
                if !uncertain {
                    AiGateOutcome::NotNeeded {
                        reason: "證據明確，本次不需要 AI".into(),
                    }
                } else if scenario.ai_unavailable {
                    match ai_spec.on_unavailable {
                        AiUnavailableBehavior::Fallback => AiGateOutcome::UnavailableFallback {
                            reason: "AI 無法使用；改用確定性 fallback".into(),
                        },
                        AiUnavailableBehavior::NoAction => AiGateOutcome::UnavailableNoAction {
                            reason: "AI 無法使用；本次不介入".into(),
                        },
                    }
                } else {
                    AiGateOutcome::Requested {
                        reason: "證據不確定，會請 AI 協助判斷".into(),
                        deadline_ms: ai_spec.max_wait_ms,
                    }
                }
            }
            other => AiGateOutcome::NotNeeded {
                reason: format!("AI 模式 {other:?} 不影響是否觸發"),
            },
        };
        let gate_blocks = matches!(gate, AiGateOutcome::UnavailableNoAction { .. });
        stages.push(json!({"stage": "aiGate", "detail": gate}));

        // ---- planning (modified snapshot; pure) ----
        let mut snapshot = self
            .capabilities(&DiscoveryContext {
                include_unavailable: false,
                ..Default::default()
            })
            .await;
        for m in snapshot.actuators.iter_mut() {
            if scenario.actuator_offline.iter().any(|a| a == m.id.as_str()) {
                m.availability = Availability::Offline;
            }
        }
        let mut policy = self.policy().await;
        if scenario.quiet_hours {
            policy.quiet_hours = vec![QuietHours {
                start: "00:00".into(),
                end: "23:59".into(),
                silenced_channels: Vec::new(),
            }];
        }
        snapshot.session_policy = policy.clone();
        let consent_missing: Vec<String> = snapshot
            .actuators
            .iter()
            .filter(|m| m.requires_consent)
            .filter(|m| {
                !sim_session.has_consent(&ConsentScope::Actuator(m.id.as_str().into()), now)
                    && !sim_session.has_consent(&ConsentScope::Channel(m.channel.clone()), now)
            })
            .map(|m| m.id.as_str().to_string())
            .collect();
        let plan = build_plan(
            PlanRequest {
                session_id: sim_session.session_id.clone(),
                intent: SemanticIntent::new(recipe.intent.clone()),
                snapshot: &snapshot,
                candidates: recipe.actuation.candidates.clone(),
                consent_missing,
                min_channels: recipe.actuation.min_channels,
                max_channels: recipe.actuation.max_channels,
                allow_no_action: recipe.decision.allow_no_action,
                message_strategy: recipe.message.clone(),
                usage: BTreeMap::<String, ActuatorUsageHint>::new(),
                now,
                default_ttl_ms: policy.default_ttl_ms,
            },
            &self.texts,
        );
        stages.push(json!({
            "stage": "planning",
            "noAction": plan.steps.is_empty(),
            "steps": plan.steps.iter().map(|s| json!({
                "actuatorId": s.actuator_id.as_str(),
                "channel": s.channel,
            })).collect::<Vec<_>>(),
            "blockedReason": plan.metadata.get("blockedReason"),
            "rationale": plan.metadata.get("rationale"),
        }));

        // ---- policy authorization per step (pure governor) ----
        let mut policy_steps = Vec::new();
        let mut would_execute = false;
        for step in &plan.steps {
            let Some(manifest) = snapshot.actuators.iter().find(|m| m.id == step.actuator_id)
            else {
                continue;
            };
            let req = AuthorizationRequest {
                actuator: manifest,
                requested: &step.requested,
                intent: &plan.intent.intent,
                source: ActionSource::Autonomous,
                local_time: chrono::Local::now().time(),
                now,
                emergency_stop_engaged: scenario.emergency_stop || self.is_estopped(),
            };
            let result = Governor::authorize(&policy, &sim_session, &req, &UsageContext::default());
            if result.outcome == interaction_core::AuthorizationOutcome::Authorized && !gate_blocks
            {
                would_execute = true;
            }
            policy_steps.push(json!({
                "actuatorId": step.actuator_id.as_str(),
                "channel": step.channel,
                "outcome": result.outcome,
                "decisions": result.decisions,
            }));
        }
        stages.push(json!({"stage": "policy", "steps": policy_steps}));

        Ok(json!({
            "recipeId": id,
            "scenario": {
                "quietHours": scenario.quiet_hours,
                "missingConsent": scenario.missing_consent,
                "actuatorOffline": scenario.actuator_offline,
                "aiUnavailable": scenario.ai_unavailable,
                "lowConfidence": scenario.low_confidence,
                "staleObservations": scenario.stale_observations,
                "recentlyFired": scenario.recently_fired,
                "emergencyStop": scenario.emergency_stop,
                "syntheticEvent": scenario.event.is_some(),
            },
            "stages": stages,
            "wouldExecute": would_execute
                && decision.fired
                && limits_ok
                && missing_scopes.is_empty()
                && !gate_blocks,
            "sideEffects": "none — 模擬不會真的執行任何動作",
        }))
    }

    /// Human recipe summary derived from the structured recipe itself.
    pub async fn recipe_summary(&self, id: &str, locale: &str) -> DomainResult<String> {
        let recipe = self.get_recipe(id).await?;
        let cards = self.human_capabilities(locale, true).await;
        let resolve = |tech_id: &str| -> String {
            for list in ["receptors", "actuators"] {
                if let Some(arr) = cards.get(list).and_then(Value::as_array) {
                    if let Some(hit) = arr.iter().find(|c| c["id"] == tech_id) {
                        if let Some(name) = hit["displayName"].as_str() {
                            return name.to_string();
                        }
                    }
                }
            }
            tech_id.to_string()
        };
        Ok(interaction_recipe::summarize(&recipe, locale, &resolve))
    }
}

/// Hash of the description-relevant manifest content: health/availability
/// flapping must not invalidate AI descriptions.
fn stable_manifest_hash<T: Serialize>(manifest: &T) -> String {
    let mut value = serde_json::to_value(manifest).unwrap_or(Value::Null);
    if let Some(obj) = value.as_object_mut() {
        obj.remove("health");
        obj.remove("availability");
    }
    manifest_hash(&value)
}

fn card_with_hash(card: interaction_registry::human_view::HumanCard, hash: &str) -> Value {
    let mut v = serde_json::to_value(&card).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.insert("manifestHash".into(), json!(hash));
    }
    v
}
