//! Presentation Provider：把桌面角色（小樞）註冊為一級 provider，能力**逐項**
//! 宣告（7 個語意 receptor＋7 個 actuator），不是一個籠統的「角色」能力。
//!
//! 誠實迴路：
//! ```text
//! actuator.execute → pending ack 登記＋`presentation.command` SSE 事件
//!   → 角色視窗實際渲染 → POST /v1/presentation/ack
//!   → receipt Dispatched→Acknowledged→Completed（證據=表面自報渲染，
//!     verdict=AcknowledgedOnly，誠實標示無獨立觀察者）
//!   → TTL 內沒有 ack → Uncertain（絕不靜默宣稱完成）
//! ```
//!
//! 可用性誠實：沒有存活的角色視窗時 actuator 為 Offline 並拒絕執行；角色被
//! 隱藏時，視窗內命令 actuator 同樣回報 Offline（Degraded 會被規劃器視為
//! 可用，導致挑選必然拒絕 dispatch 的目標），視窗內 receptor 拒絕 ingest
//! （隱藏角色**不是** Emergency Stop——Runtime、tray 與 agent session 全部
//! 保留；隱藏狀態仍透過 presentation status／`presentation.state` 事件呈現）。

use crate::runtime::{Runtime, RuntimeInner};
use adapters_builtin::PushReceptor;
use async_trait::async_trait;
use chrono::Utc;
use interaction_adapter_sdk::{ActuatorManifestBuilder, DriverReceipt, ReceptorManifestBuilder};
use interaction_core::{
    ActionId, ActionReceipt, ActionStatus, Actuator, ActuatorError, ActuatorManifest,
    BoundedAction, ComponentHealth, EventType, ReceptorMode, RiskClass, RuntimeEvent, Sensitivity,
    Timestamp, VerificationEvidence, VerificationVerdict,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

/// 角色視窗必須在此時限內 ack，否則 receipt 誠實標為 Uncertain。
pub const ACK_TTL_MS: i64 = 10_000;
/// 超過此秒數沒有 hello 心跳即視為視窗斷線。
pub const PRESENCE_STALE_SECS: i64 = 20;
const PENDING_CAP: usize = 64;
const MAX_BUBBLE_CHARS: usize = 200;

/// AI 可直接點播的動畫白名單。成功／驗證／阻擋／未知／失敗／緊急等
/// 「真相狀態」動畫**不在**此列——它們只能由 runtime 事件真實驅動。
pub const PLAYABLE_ANIMATIONS: &[&str] = &[
    "idle",
    "notice",
    "curious",
    "listening",
    "thinking",
    "working",
    "waiting",
    "quiet",
    "stretch",
];

/// behaviorIntent 白名單（spec §7）：AI 只能提出高層意圖，映射由前端
/// Behavior Runtime 完成；不含任何成功／安全語意。
pub const BEHAVIOR_INTENTS: &[&str] = &[
    "rest",
    "notice",
    "curious",
    "listen",
    "think",
    "work",
    "wait-attention",
    "look-at-confirmation",
    "acknowledge-briefly",
];

pub const TONES: &[&str] = &["neutral", "attentive", "gentle", "playful", "serious"];
pub const SOUNDS: &[&str] = &["chime", "soft-pop", "tick"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationKind {
    StatePresent,
    AnimationPlay,
    BubbleShow,
    SoundPlay,
    Speak,
    WindowAdjust,
    PresenceSet,
}

impl PresentationKind {
    pub fn actuator_id(&self) -> &'static str {
        match self {
            Self::StatePresent => "companion.state.present",
            Self::AnimationPlay => "companion.animation.play",
            Self::BubbleShow => "companion.bubble.show",
            Self::SoundPlay => "companion.sound.play",
            Self::Speak => "companion.speak",
            Self::WindowAdjust => "companion.window.adjust",
            Self::PresenceSet => "companion.presence.set",
        }
    }

    pub fn command(&self) -> &'static str {
        match self {
            Self::StatePresent => "state-present",
            Self::AnimationPlay => "animation-play",
            Self::BubbleShow => "bubble-show",
            Self::SoundPlay => "sound-play",
            Self::Speak => "speak",
            Self::WindowAdjust => "window-adjust",
            Self::PresenceSet => "presence-set",
        }
    }

    pub const ALL: [PresentationKind; 7] = [
        Self::StatePresent,
        Self::AnimationPlay,
        Self::BubbleShow,
        Self::SoundPlay,
        Self::Speak,
        Self::WindowAdjust,
        Self::PresenceSet,
    ];
}

#[derive(Debug, Clone)]
pub struct PendingCommand {
    pub command: &'static str,
    /// Runtime-validated parameters. Native surfaces retrieve this authoritative
    /// copy by action id instead of trusting values supplied by WebView JS.
    pub params: Value,
    pub enqueued_at: Timestamp,
    pub expires_at: Timestamp,
}

#[derive(Debug, Default)]
struct BridgeState {
    last_seen: Option<Timestamp>,
    visible: bool,
    pack_id: Option<String>,
    /// Sanitized, low-risk semantic state reported by the local companion
    /// window. It contains no pointer coordinates, raw input or free-form
    /// reasoning; the human explanation is derived here, not trusted from JS.
    behavior_state: Option<Value>,
}

/// 角色視窗與 runtime 之間的橋：presence 心跳＋待 ack 命令登記。
#[derive(Default)]
pub struct PresentationBridge {
    state: Mutex<BridgeState>,
    pending: Mutex<BTreeMap<String, PendingCommand>>,
}

impl PresentationBridge {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 視窗心跳。回傳 (connected 是否改變, visible 是否改變)。
    pub fn hello(&self, visible: bool, pack_id: Option<String>, now: Timestamp) -> (bool, bool) {
        self.hello_with_behavior(visible, pack_id, None, now)
    }

    pub fn hello_with_behavior(
        &self,
        visible: bool,
        pack_id: Option<String>,
        behavior_state: Option<Value>,
        now: Timestamp,
    ) -> (bool, bool) {
        let mut s = self.state.lock().expect("presentation state lock");
        let was_connected = Self::fresh(s.last_seen, now);
        let was_visible = was_connected && s.visible;
        s.last_seen = Some(now);
        s.visible = visible;
        if pack_id.is_some() {
            s.pack_id = pack_id;
        }
        if let Some(raw) = behavior_state {
            if let Some(clean) = sanitize_behavior_telemetry(raw) {
                s.behavior_state = Some(clean);
            }
        }
        (!was_connected, was_visible != visible)
    }

    fn fresh(last_seen: Option<Timestamp>, now: Timestamp) -> bool {
        last_seen.is_some_and(|t| now.signed_duration_since(t).num_seconds() < PRESENCE_STALE_SECS)
    }

    pub fn connected(&self, now: Timestamp) -> bool {
        let s = self.state.lock().expect("presentation state lock");
        Self::fresh(s.last_seen, now)
    }

    /// 視窗內能力（點擊／氣泡／動畫…）是否可用＝已連線且可見。
    pub fn accepts_input(&self, now: Timestamp) -> bool {
        let s = self.state.lock().expect("presentation state lock");
        Self::fresh(s.last_seen, now) && s.visible
    }

    pub fn snapshot(&self, now: Timestamp) -> Value {
        let s = self.state.lock().expect("presentation state lock");
        let connected = Self::fresh(s.last_seen, now);
        let behavior = connected.then(|| s.behavior_state.clone()).flatten();
        let explanation = behavior
            .as_ref()
            .and_then(|value| value.get("explanation"))
            .cloned();
        json!({
            "connected": connected,
            "visible": connected && s.visible,
            "packId": s.pack_id,
            "lastSeenSecondsAgo": s.last_seen.map(|t| now.signed_duration_since(t).num_seconds()),
            "pendingCommands": self.pending.lock().expect("pending lock").len(),
            "behaviorState": behavior,
            "behaviorExplanation": explanation,
        })
    }

    fn enqueue(
        &self,
        action_id: &ActionId,
        kind: PresentationKind,
        params: Value,
        now: Timestamp,
    ) -> Result<Timestamp, ActuatorError> {
        let mut p = self.pending.lock().expect("pending lock");
        if p.len() >= PENDING_CAP {
            return Err(ActuatorError::Rejected(
                "presentation queue full (companion not consuming)".into(),
            ));
        }
        let expires_at = now + chrono::Duration::milliseconds(ACK_TTL_MS);
        p.insert(
            action_id.as_str().to_string(),
            PendingCommand {
                command: kind.command(),
                params,
                enqueued_at: now,
                expires_at,
            },
        );
        Ok(expires_at)
    }

    pub fn take_pending(&self, action_id: &str) -> Option<PendingCommand> {
        self.pending.lock().expect("pending lock").remove(action_id)
    }

    pub fn pending(&self, action_id: &str, now: Timestamp) -> Option<PendingCommand> {
        self.pending
            .lock()
            .expect("pending lock")
            .get(action_id)
            .filter(|pending| pending.expires_at > now)
            .cloned()
    }

    pub fn sweep_expired(&self, now: Timestamp) -> Vec<String> {
        let mut p = self.pending.lock().expect("pending lock");
        let expired: Vec<String> = p
            .iter()
            .filter(|(_, c)| c.expires_at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            p.remove(id);
        }
        expired
    }

    pub fn clear_pending(&self) -> Vec<String> {
        let mut p = self.pending.lock().expect("pending lock");
        let ids = p.keys().cloned().collect();
        p.clear();
        ids
    }

    /// actuator 表面健康：與 execute 的可用性閘門完全一致——presence-set
    /// 只需連線，其餘命令需要視窗可見。隱藏回報 Offline 而非 Degraded：
    /// Degraded 在 availability 映射裡算可用，規劃器會挑到必然拒絕
    /// dispatch 的 actuator；隱藏／離線的區別仍由 message 誠實呈現。
    pub fn actuator_health(&self, kind: PresentationKind, now: Timestamp) -> ComponentHealth {
        let s = self.state.lock().expect("presentation state lock");
        if !Self::fresh(s.last_seen, now) {
            return ComponentHealth::offline("companion window not connected").at(now);
        }
        match kind {
            PresentationKind::PresenceSet => ComponentHealth::healthy().at(now),
            _ if s.visible => ComponentHealth::healthy().at(now),
            _ => ComponentHealth::offline("companion hidden").at(now),
        }
    }

    /// 視窗內 receptor 健康：隱藏時即 Offline（隱藏後角色停止感知）。
    pub fn receptor_health(&self, now: Timestamp) -> ComponentHealth {
        let s = self.state.lock().expect("presentation state lock");
        if Self::fresh(s.last_seen, now) && s.visible {
            ComponentHealth::healthy().at(now)
        } else {
            ComponentHealth::offline("companion window hidden or not connected").at(now)
        }
    }
}

/// Accept only the fixed semantic Behavior Runtime fields. Values outside the
/// documented ranges make the whole telemetry sample invalid, avoiding a UI
/// that presents malformed WebView data as Runtime truth.
fn sanitize_behavior_telemetry(raw: Value) -> Option<Value> {
    let input = raw.as_object()?;
    let number = |key: &str, max: f64| -> Option<f64> {
        let value = input.get(key)?.as_f64()?;
        value
            .is_finite()
            .then_some(value)
            .filter(|v| (0.0..=max).contains(v))
    };
    let activation = number("activation", 1.0)?;
    let attention = number("attention", 1.0)?;
    let task_load = number("taskLoad", 1.0)?;
    let readiness = number("interactionReadiness", 1.0)?;
    let familiarity = number("familiarity", 1.0)?;
    let interruptions = number("recentInterruptions", 5.0)?;
    let base = input.get("base")?.as_str()?;
    if !["idle", "quiet", "paused", "emergency", "offline"].contains(&base) {
        return None;
    }
    let transient = match input.get("transient") {
        None | Some(Value::Null) => None,
        Some(Value::String(value))
            if [
                "clicked",
                "listening",
                "thinking",
                "routing",
                "requesting-consent",
                "acting",
                "waiting-for-receipt",
                "succeeded",
                "succeeded-verified",
                "uncertain",
                "failed",
                "blocked",
                "performing",
            ]
            .contains(&value.as_str()) =>
        {
            Some(value.clone())
        }
        _ => return None,
    };
    let focus = match input.get("currentFocus") {
        None | Some(Value::Null) => None,
        Some(Value::String(value))
            if value.chars().count() <= 128 && !value.chars().any(|ch| ch.is_control()) =>
        {
            Some(value.clone())
        }
        _ => return None,
    };
    let last_interaction_at = input.get("lastInteractionAt")?.as_i64()?;

    // v0.5 Roll Call：「現在大家在做什麼」——有界的人話清單（角色＋使魔，
    // 最多 4 筆；名稱/活動皆長度受限且不含控制字元）。缺漏＝不顯示，
    // 不合法＝整個 hello 拒絕（與其他欄位同一誠實標準）。
    let roll_call = match input.get("rollCall") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) if items.len() <= 4 => {
            let mut clean = Vec::with_capacity(items.len());
            for item in items {
                let obj = item.as_object()?;
                let name = obj.get("name")?.as_str()?;
                let activity = obj.get("activity")?.as_str()?;
                if name.chars().count() > 24
                    || activity.chars().count() > 32
                    || name.chars().any(char::is_control)
                    || activity.chars().any(char::is_control)
                {
                    return None;
                }
                clean.push(json!({ "name": name, "activity": activity }));
            }
            Some(Value::Array(clean))
        }
        _ => return None,
    };

    let explanation = match base {
        "emergency" => "緊急停止優先，所有俏皮與主動動作都已中斷。".to_string(),
        "offline" => "Runtime 離線，角色維持固定安全姿態。".to_string(),
        "paused" => "主動互動已暫停，只保留必要的安全狀態。".to_string(),
        "quiet" => "目前在勿擾時段，已降低非必要表現。".to_string(),
        _ => match transient.as_deref() {
            Some("requesting-consent") => "正在等待你的授權，所以保持專注並停止玩鬧。".to_string(),
            Some("acting" | "thinking" | "routing") => {
                "目前有工作進行中，所以注意力集中在任務上。".to_string()
            }
            Some("waiting-for-receipt") => "動作已送出，正在等待可核對的結果。".to_string(),
            Some("uncertain") => "目前無法確認結果，因此維持未知姿態。".to_string(),
            Some("failed" | "blocked") => "工作失敗或被安全規則阻擋，正在保持警覺。".to_string(),
            Some("succeeded-verified") => "結果已通過驗證，正在短暫確認完成。".to_string(),
            Some("succeeded") => "Agent 回報完成，但仍未當成已驗證成功。".to_string(),
            Some(_) => "正在回應最近的角色視窗互動。".to_string(),
            None if focus.is_some() => "正在留意最近的語意事件；沒有保存游標軌跡。".to_string(),
            None => "目前沒有任務或風險，維持低干擾的自然待機。".to_string(),
        },
    };

    Some(json!({
        "activation": activation,
        "attention": attention,
        "taskLoad": task_load,
        "interactionReadiness": readiness,
        "familiarity": familiarity,
        "recentInterruptions": interruptions,
        "currentFocus": focus,
        "lastInteractionAt": last_interaction_at,
        "base": base,
        "transient": transient,
        "explanation": explanation,
        "rollCall": roll_call,
    }))
}

// ---------------------------------------------------------------------------
// 參數驗證：runtime 端確定性驗證，不信任 AI 端。
// ---------------------------------------------------------------------------

fn extra_value<'a>(action: &'a BoundedAction, key: &str) -> Option<&'a Value> {
    action.effective.extra.as_ref().and_then(|e| e.get(key))
}

fn extra_str<'a>(action: &'a BoundedAction, key: &str) -> Option<&'a str> {
    extra_value(action, key).and_then(|v| v.as_str())
}

fn clean_text(text: &str, max: usize, field: &str) -> Result<String, ActuatorError> {
    if text.chars().count() > max {
        return Err(ActuatorError::PayloadTooLarge(format!(
            "{field} exceeds {max} chars"
        )));
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(ActuatorError::Rejected(format!(
            "{field} contains control characters"
        )));
    }
    Ok(text.to_string())
}

/// spec §7 的高層角色輸出 Schema：Runtime 驗證通過才呈現。
fn validate_state_present(action: &BoundedAction) -> Result<Value, ActuatorError> {
    let behavior = extra_str(action, "behaviorIntent").ok_or_else(|| {
        ActuatorError::Rejected("behaviorIntent is required for companion.state.present".into())
    })?;
    if !BEHAVIOR_INTENTS.contains(&behavior) {
        return Err(ActuatorError::Rejected(format!(
            "behaviorIntent '{behavior}' is not in the registered whitelist"
        )));
    }
    let tone = extra_str(action, "tone").unwrap_or("neutral");
    if !TONES.contains(&tone) {
        return Err(ActuatorError::Rejected(format!("unknown tone '{tone}'")));
    }
    let message = match &action.effective.message {
        Some(m) => Some(clean_text(m, MAX_BUBBLE_CHARS, "message")?),
        None => None,
    };
    Ok(json!({
        "behaviorIntent": behavior,
        "tone": tone,
        "message": message,
    }))
}

fn validate_params(kind: PresentationKind, action: &BoundedAction) -> Result<Value, ActuatorError> {
    match kind {
        PresentationKind::StatePresent => validate_state_present(action),
        PresentationKind::AnimationPlay => {
            let name = extra_str(action, "animation")
                .ok_or_else(|| ActuatorError::Rejected("animation name is required".into()))?;
            if !PLAYABLE_ANIMATIONS.contains(&name) {
                return Err(ActuatorError::Rejected(format!(
                    "animation '{name}' is not directly playable (truth-driven states are runtime-only)"
                )));
            }
            Ok(json!({ "animation": name }))
        }
        PresentationKind::BubbleShow => {
            let text = action
                .effective
                .message
                .as_deref()
                .ok_or_else(|| ActuatorError::Rejected("bubble message is required".into()))?;
            Ok(json!({ "message": clean_text(text, MAX_BUBBLE_CHARS, "message")? }))
        }
        PresentationKind::SoundPlay => {
            let sound = extra_str(action, "sound").unwrap_or("chime");
            if !SOUNDS.contains(&sound) {
                return Err(ActuatorError::Rejected(format!("unknown sound '{sound}'")));
            }
            Ok(json!({ "sound": sound }))
        }
        PresentationKind::Speak => {
            let text = action
                .effective
                .message
                .as_deref()
                .ok_or_else(|| ActuatorError::Rejected("speech text is required".into()))?;
            Ok(json!({ "text": clean_text(text, MAX_BUBBLE_CHARS, "text")? }))
        }
        PresentationKind::WindowAdjust => {
            let mut out = serde_json::Map::new();
            for key in ["x", "y"] {
                if let Some(v) = extra_value(action, key).and_then(|v| v.as_f64()) {
                    if !(-20_000.0..=20_000.0).contains(&v) {
                        return Err(ActuatorError::Rejected(format!("{key} out of range")));
                    }
                    out.insert(key.into(), json!(v));
                }
            }
            for key in ["width", "height"] {
                if let Some(v) = extra_value(action, key).and_then(|v| v.as_f64()) {
                    if !(64.0..=1024.0).contains(&v) {
                        return Err(ActuatorError::Rejected(format!(
                            "{key} must be within 64..1024"
                        )));
                    }
                    out.insert(key.into(), json!(v));
                }
            }
            if let Some(v) = extra_value(action, "opacity").and_then(|v| v.as_f64()) {
                if !(0.2..=1.0).contains(&v) {
                    return Err(ActuatorError::Rejected("opacity must be 0.2..1.0".into()));
                }
                out.insert("opacity".into(), json!(v));
            }
            if let Some(v) = extra_value(action, "alwaysOnTop").and_then(|v| v.as_bool()) {
                out.insert("alwaysOnTop".into(), json!(v));
            }
            if out.is_empty() {
                return Err(ActuatorError::Rejected(
                    "window-adjust requires at least one of x/y/width/height/opacity/alwaysOnTop"
                        .into(),
                ));
            }
            Ok(Value::Object(out))
        }
        PresentationKind::PresenceSet => {
            let visible = extra_value(action, "visible")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| ActuatorError::Rejected("visible: bool is required".into()))?;
            Ok(json!({ "visible": visible }))
        }
    }
}

// ---------------------------------------------------------------------------
// Actuator
// ---------------------------------------------------------------------------

pub struct PresentationActuator {
    kind: PresentationKind,
    bridge: Arc<PresentationBridge>,
    runtime: Weak<RuntimeInner>,
}

impl PresentationActuator {
    pub fn new(
        kind: PresentationKind,
        bridge: Arc<PresentationBridge>,
        runtime: Weak<RuntimeInner>,
    ) -> Self {
        Self {
            kind,
            bridge,
            runtime,
        }
    }
}

#[async_trait]
impl Actuator for PresentationActuator {
    fn manifest(&self) -> ActuatorManifest {
        let k = self.kind;
        let b = |name: &str, desc: &str| {
            ActuatorManifestBuilder::new(
                k.actuator_id(),
                name,
                "desktop-pet",
                "builtin.presentation",
            )
            .description(desc)
            .supports_cancel(true)
            .human(interaction_adapter_sdk::local_effect_semantics(
                interaction_core::Interruptiveness::Low,
                interaction_core::ConfirmationLevel::Completed,
            ))
        };
        match k {
            PresentationKind::StatePresent => b(
                "角色狀態呈現",
                "以驗證過的 behaviorIntent 切換角色的高層狀態呈現（不含成功／安全語意）",
            )
            .capabilities(&["state", "behavior-intent"])
            .risk(RiskClass::Low)
            .build(),
            PresentationKind::AnimationPlay => b(
                "播放已登記動畫",
                "播放白名單內的角色動畫；真相狀態（成功／阻擋／緊急…）只能由 runtime 事件驅動",
            )
            .capabilities(&["animation"])
            .risk(RiskClass::Low)
            .build(),
            PresentationKind::BubbleShow => {
                b("顯示文字氣泡", "在角色旁顯示一則短文字氣泡（≤200 字）")
                    .capabilities(&["bubble", "text"])
                    .risk(RiskClass::Low)
                    .build()
            }
            PresentationKind::SoundPlay => b("播放音效", "播放內建短音效（預設關閉，需明確同意）")
                .capabilities(&["sound"])
                .risk(RiskClass::Low)
                .requires_consent(true)
                .build(),
            PresentationKind::Speak => b("語音朗讀", "本機語音朗讀短文字（預設關閉，需明確同意）")
                .capabilities(&["speech", "text"])
                .risk(RiskClass::Low)
                .requires_consent(true)
                .build(),
            PresentationKind::WindowAdjust => b(
                "調整角色視窗",
                "調整角色視窗位置／大小／透明度／最上層（預設關閉，需明確同意）",
            )
            .capabilities(&["window"])
            .risk(RiskClass::BoundedSideEffect)
            .requires_consent(true)
            .build(),
            PresentationKind::PresenceSet => b(
                "顯示或隱藏角色",
                "顯示／隱藏桌面角色視窗（預設關閉，需明確同意；隱藏不影響 Runtime）",
            )
            .capabilities(&["presence"])
            .risk(RiskClass::BoundedSideEffect)
            .requires_consent(true)
            .build(),
        }
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        let now = Utc::now();
        if action.is_expired(now) {
            return Err(ActuatorError::Expired);
        }
        let rt = self
            .runtime
            .upgrade()
            .ok_or_else(|| ActuatorError::Unavailable("runtime shutting down".into()))?;
        // 誠實可用性：presence-set 只需連線（把隱藏的角色叫出來是合法的），
        // 其餘命令需要視窗可見。
        let ready = match self.kind {
            PresentationKind::PresenceSet => self.bridge.connected(now),
            _ => self.bridge.accepts_input(now),
        };
        if !ready {
            return Err(ActuatorError::Unavailable(
                "companion window not connected or hidden".into(),
            ));
        }
        let params = validate_params(self.kind, &action)?;
        let expires_at = self
            .bridge
            .enqueue(&action.action_id, self.kind, params.clone(), now)?;
        rt.events.publish(
            RuntimeEvent::new(
                EventType::PresentationCommand,
                now,
                json!({
                    "actionId": action.action_id.as_str(),
                    "command": self.kind.command(),
                    "params": params,
                    "expiresAt": expires_at,
                }),
            )
            .with_correlation(action.correlation_id.clone()),
        );
        Ok(DriverReceipt::start(&action, now)
            .dispatched()
            .note("presentation", json!(self.kind.command()))
            .finish())
    }

    async fn status(&self) -> ComponentHealth {
        self.bridge.actuator_health(self.kind, Utc::now())
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        let rt = self
            .runtime
            .upgrade()
            .ok_or_else(|| ActuatorError::Unavailable("runtime shutting down".into()))?;
        if self.bridge.take_pending(action_id.as_str()).is_some() {
            rt.events.publish(RuntimeEvent::new(
                EventType::PresentationCommand,
                Utc::now(),
                json!({ "actionId": action_id.as_str(), "command": "cancel" }),
            ));
        }
        rt.store
            .receipt(action_id)
            .map_err(|e| ActuatorError::NotFound(e.to_string()))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        // 冪等：任何一個 presentation actuator 被掃到都會清空共用佇列。
        let cleared = self.bridge.clear_pending();
        if !cleared.is_empty() {
            if let Some(rt) = self.runtime.upgrade() {
                rt.events.publish(RuntimeEvent::new(
                    EventType::PresentationCommand,
                    Utc::now(),
                    json!({ "command": "clear-all", "reason": "emergency-stop" }),
                ));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Receptors（7 個逐項語意 receptor；健康與 bridge presence 綁定）
// ---------------------------------------------------------------------------

pub fn presentation_receptors(bridge: &Arc<PresentationBridge>) -> Vec<Arc<PushReceptor>> {
    let health = |bridge: Arc<PresentationBridge>| {
        Arc::new(move || bridge.receptor_health(Utc::now()))
            as Arc<dyn Fn() -> ComponentHealth + Send + Sync>
    };
    let mk = |id: &str, name: &str, desc: &str, provides: &[&str], sensitivity: Sensitivity| {
        PushReceptor::with_health(
            ReceptorManifestBuilder::new(id, name, "builtin.presentation")
                .description(desc)
                .category("companion")
                .provides(provides)
                .mode(ReceptorMode::Event)
                .sensitivity(sensitivity, false)
                .human(interaction_adapter_sdk::data_semantics(
                    &["companion-input"],
                    interaction_core::TriState::Yes,
                    interaction_core::DataSource::Local,
                ))
                .build(),
            health(bridge.clone()),
        )
    };
    vec![
        mk(
            "companion.click",
            "角色點擊",
            "使用者點擊或拖動角色（語意事件，永不記錄座標）",
            &["kind"],
            Sensitivity::Internal,
        ),
        mk(
            "companion.text-input",
            "角色文字輸入",
            "使用者在角色視窗輸入的文字",
            &["kind", "text", "modality"],
            Sensitivity::Personal,
        ),
        mk(
            "companion.quick-action",
            "角色快捷操作",
            "使用者從角色快捷選單選擇的操作",
            &["kind", "action"],
            Sensitivity::Internal,
        ),
        mk(
            "companion.drag-drop",
            "角色拖放",
            "拖放進入／離開／確認／取消（僅檔名與數量；內容讀取需另行確認）",
            &["kind", "fileCount", "names"],
            Sensitivity::Personal,
        ),
        mk(
            "companion.pointer",
            "角色游標語意",
            "游標接近／離開角色範圍的語意事件（無座標、30 秒節流）",
            &["kind"],
            Sensitivity::Internal,
        ),
        mk(
            "companion.animation-events",
            "角色動畫事件",
            "動畫完成／中斷／失敗（供配方與觀察，不作為 receipt 證據）",
            &["kind", "animation"],
            Sensitivity::Internal,
        ),
        mk(
            "companion.bubble-events",
            "角色氣泡事件",
            "氣泡顯示完成／被使用者關閉",
            &["kind"],
            Sensitivity::Internal,
        ),
    ]
}

/// 所有 companion 視窗內 receptor 的 id（ingest 隱藏閘門用）。
/// legacy 的 `desktop.companion.interaction` 承載同一批視窗內語意輸入
/// （前端未映射 kind 的 fallback 目標），必須走同一個隱藏／斷線閘門，
/// 否則隱藏角色仍可經它被餵入互動觀測。
pub fn is_companion_surface_receptor(id: &str) -> bool {
    matches!(
        id,
        "companion.click"
            | "companion.text-input"
            | "companion.quick-action"
            | "companion.drag-drop"
            | "companion.pointer"
            | "companion.animation-events"
            | "companion.bubble-events"
            | "desktop.companion.interaction"
    )
}

/// OPEN 選擇（未指名 candidates）時，規劃器無法憑空合成、缺了必然被
/// execute 拒絕的參數。`"message"` 代表需要文字訊息（由文字選擇器供給），
/// 其餘鍵必須出現在 intent payload。指名 candidates 的計畫不經此閘，
/// 保留執行期的誠實拒絕（receipt 會說明缺了什麼）。
pub fn open_selection_requirements(actuator_id: &str) -> &'static [&'static str] {
    match actuator_id {
        "companion.animation.play" => &["animation"],
        "companion.state.present" => &["behaviorIntent"],
        "companion.bubble.show" | "companion.speak" => &["message"],
        "companion.presence.set" => &["visible"],
        _ => &[],
    }
}

// ---------------------------------------------------------------------------
// Runtime 介面：hello / ack / status / sweep
// ---------------------------------------------------------------------------

impl Runtime {
    /// 角色視窗心跳（開機、每 10 秒、可見性變化時呼叫）。
    pub async fn presentation_hello(&self, visible: bool, pack_id: Option<String>) -> Value {
        self.presentation_hello_with_behavior(visible, pack_id, None)
            .await
    }

    pub async fn presentation_hello_with_behavior(
        &self,
        visible: bool,
        pack_id: Option<String>,
        behavior_state: Option<Value>,
    ) -> Value {
        let now = Utc::now();
        let (conn_changed, vis_changed) =
            self.presentation
                .hello_with_behavior(visible, pack_id, behavior_state, now);
        if conn_changed || vis_changed {
            // presence 變化立即反映到 registry 的健康快取：不等 watchdog 的
            // 週期 refresh，規劃器才不會在隱藏／剛連線的窗口內拿到過期健康。
            self.registry.refresh_health().await;
            let snap = self.presentation.snapshot(now);
            self.events.publish(RuntimeEvent::new(
                EventType::PresentationState,
                now,
                snap.clone(),
            ));
        }
        self.presentation.snapshot(now)
    }

    pub fn presentation_status(&self) -> Value {
        self.presentation.snapshot(Utc::now())
    }

    /// Return a pending command's authoritative, governor-validated parameters.
    /// This is used by the native desktop bridge so WebView JS cannot widen a
    /// companion-window operation by changing event payloads.
    pub fn presentation_pending_command(
        &self,
        action_id: &str,
    ) -> interaction_core::DomainResult<Value> {
        use interaction_core::DomainError;
        let pending = self
            .presentation
            .pending(action_id, Utc::now())
            .ok_or_else(|| {
                DomainError::NotFound(format!("no live pending presentation command {action_id}"))
            })?;
        Ok(json!({
            "actionId": action_id,
            "command": pending.command,
            "params": pending.params,
            "expiresAt": pending.expires_at,
        }))
    }

    /// 角色視窗回報一個 presentation 命令的實際結果。
    /// 只有本 bridge 發出且待 ack 的 actionId 能走此路徑——一般 ingest 的
    /// `actionId` 改名防偽規則不適用於此，因為這裡以 pending 登記表比對。
    pub async fn presentation_ack(
        &self,
        action_id: &str,
        outcome: &str,
        detail: Option<String>,
    ) -> interaction_core::DomainResult<Value> {
        use interaction_core::DomainError;
        let pending = self.presentation.take_pending(action_id).ok_or_else(|| {
            DomainError::NotFound(format!("no pending presentation command {action_id}"))
        })?;
        let aid = ActionId::new(action_id);
        let mut receipt = self.store.receipt(&aid)?;
        if !receipt.actuator_id.as_str().starts_with("companion.") {
            return Err(DomainError::Validation(
                "presentation ack only applies to companion actuators".into(),
            ));
        }
        let now = Utc::now();
        // 事件延後到 persist 成功之後才發（同 sweep_presentation_at 與
        // executor 的 guarded 模式）：estop／watchdog 可能在本函式讀取與
        // 寫回之間先把 receipt 寫成終態，先發事件會對 /v1/events 訂閱者
        // 謊稱 acknowledged/completed。快照在轉移當下取，保留各事件
        // payload 的階段性 status。
        let mut deferred_events: Vec<(EventType, ActionReceipt)> = Vec::new();
        match outcome {
            "displayed" | "completed" => {
                // 與 executor 的 Dispatched persist 競態：execute 先發
                // presentation.command SSE、executor 稍後才 persist
                // Dispatched。視窗若在 Accepted 窗口內就 ack，這裡先合法
                // 走完 Accepted→Dispatched，鏈條不斷、receipt 不孤兒化；
                // ack 寫入終態後，executor 遲到的 Dispatched persist 會被
                // sticky-terminal 守衛拒絕並改讀 store。
                if receipt.current_status == ActionStatus::Accepted {
                    let _ = receipt.transition(ActionStatus::Dispatched, now);
                    deferred_events.push((EventType::ActionDispatched, receipt.clone()));
                }
                if receipt.current_status == ActionStatus::Dispatched {
                    let _ = receipt.transition(ActionStatus::Acknowledged, now);
                    deferred_events.push((EventType::ActionAcknowledged, receipt.clone()));
                }
                if receipt.current_status == ActionStatus::Acknowledged {
                    receipt.verification = Some(VerificationEvidence {
                        observation_ids: vec![],
                        verdict: VerificationVerdict::AcknowledgedOnly,
                        detail: Some(format!(
                            "companion surface confirmed {} render; no independent observer",
                            pending.command
                        )),
                        verified_at: now,
                    });
                    let _ = receipt.transition(ActionStatus::Completed, now);
                    deferred_events.push((EventType::ActionCompleted, receipt.clone()));
                }
            }
            "interrupted" => {
                if !receipt.current_status.is_terminal() {
                    receipt.driver_response.insert(
                        "interrupted".into(),
                        json!(detail.clone().unwrap_or_default()),
                    );
                    let _ = receipt.transition(ActionStatus::Cancelled, now);
                    deferred_events.push((EventType::ActionCancelled, receipt.clone()));
                }
            }
            "failed" | "unsupported" => {
                if !receipt.current_status.is_terminal() {
                    receipt.driver_response.insert(
                        "error".into(),
                        json!(detail.clone().unwrap_or_else(|| outcome.to_string())),
                    );
                    let _ = receipt.transition(ActionStatus::Failed, now);
                    deferred_events.push((EventType::ActionFailed, receipt.clone()));
                }
            }
            other => {
                return Err(DomainError::Validation(format!(
                    "unknown presentation outcome '{other}'"
                )));
            }
        }
        if self.persist_receipt(&receipt, "desktop-pet").await? {
            for (event_type, snapshot) in &deferred_events {
                self.emit_action_event(*event_type, snapshot, json!({}));
            }
        } else {
            // estop 或 watchdog 已先寫入終態——以 store 為準，誠實回報，
            // 且一個事件都不發（不謊稱完成）。
            receipt = self.store.receipt(&aid)?;
        }
        Ok(json!({
            "actionId": aid.as_str(),
            "status": receipt.current_status,
        }))
    }

    /// watchdog 每 tick 呼叫：逾時未 ack 的 presentation 命令標為 Uncertain。
    pub(crate) async fn sweep_presentation(&self) {
        self.sweep_presentation_at(Utc::now()).await;
    }

    /// 時間可注入版本（watchdog 與測試共用；不影響語意）。
    pub async fn sweep_presentation_at(&self, now: Timestamp) {
        for action_id in self.presentation.sweep_expired(now) {
            let aid = ActionId::new(&action_id);
            let Ok(mut receipt) = self.store.receipt(&aid) else {
                continue;
            };
            if receipt.current_status.is_terminal() {
                continue;
            }
            receipt.verification = Some(VerificationEvidence {
                observation_ids: vec![],
                verdict: VerificationVerdict::Uncertain,
                detail: Some("companion window never confirmed the command".into()),
                verified_at: now,
            });
            let _ = receipt.transition(ActionStatus::Uncertain, now);
            if let Ok(true) = self.persist_receipt(&receipt, "desktop-pet").await {
                self.emit_action_event(EventType::ActionUncertain, &receipt, json!({}));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_presence_is_honest() {
        let bridge = PresentationBridge::new();
        let now = Utc::now();
        assert!(!bridge.connected(now));
        assert!(!bridge.accepts_input(now));
        bridge.hello(true, Some("shu-agile".into()), now);
        assert!(bridge.connected(now));
        assert!(bridge.accepts_input(now));
        // 隱藏：連線但不接受視窗內輸入。
        bridge.hello(false, None, now);
        assert!(bridge.connected(now));
        assert!(!bridge.accepts_input(now));
        // 隱藏：presence-set 仍可用（把角色叫回來是合法的），其餘命令
        // 誠實回報 Offline——Degraded 會被規劃器當成可用。
        assert_eq!(
            bridge
                .actuator_health(PresentationKind::PresenceSet, now)
                .status,
            interaction_core::HealthStatus::Healthy
        );
        let hidden = bridge.actuator_health(PresentationKind::BubbleShow, now);
        assert_eq!(hidden.status, interaction_core::HealthStatus::Offline);
        assert_eq!(hidden.message.as_deref(), Some("companion hidden"));
        // 心跳過期：一切離線。
        let later = now + chrono::Duration::seconds(PRESENCE_STALE_SECS + 1);
        assert!(!bridge.connected(later));
        assert_eq!(
            bridge
                .actuator_health(PresentationKind::BubbleShow, later)
                .status,
            interaction_core::HealthStatus::Offline
        );
        assert_eq!(
            bridge
                .actuator_health(PresentationKind::PresenceSet, later)
                .status,
            interaction_core::HealthStatus::Offline
        );
    }

    #[test]
    fn behavior_telemetry_is_bounded_and_explained_by_runtime() {
        let bridge = PresentationBridge::new();
        let now = Utc::now();
        bridge.hello_with_behavior(
            true,
            Some("shu-agile".into()),
            Some(json!({
                "activation": 0.4,
                "attention": 0.7,
                "taskLoad": 0.6,
                "interactionReadiness": 0.3,
                "familiarity": 0.2,
                "recentInterruptions": 1.0,
                "currentFocus": "task.progress",
                "lastInteractionAt": 1_700_000_000_000_i64,
                "base": "idle",
                "transient": "acting",
                "explanation": "untrusted WebView claim",
            })),
            now,
        );
        let snapshot = bridge.snapshot(now);
        assert_eq!(snapshot["behaviorState"]["attention"], 0.7);
        assert_eq!(
            snapshot["behaviorExplanation"],
            "目前有工作進行中，所以注意力集中在任務上。"
        );
        assert_ne!(snapshot["behaviorExplanation"], "untrusted WebView claim");

        // A malformed sample is discarded wholesale; stale good data is not
        // overwritten by arbitrary or out-of-range values.
        bridge.hello_with_behavior(
            true,
            None,
            Some(json!({"activation": 99, "base": "celebrating"})),
            now,
        );
        assert_eq!(bridge.snapshot(now)["behaviorState"]["attention"], 0.7);
    }

    #[test]
    fn open_selection_requirements_match_validators() {
        use interaction_core::{ActionParameters, BoundedAction};
        // 帶指定 payload（不帶 message）的動作——用來驗證 helper 宣告的
        // 必要參數確實是 validate_params 的硬性要求，兩者不脫鉤。
        let mk = |actuator: &str, extra: Option<Value>| BoundedAction {
            action_id: ActionId::generate(),
            plan_id: interaction_core::PlanId::generate(),
            session_id: interaction_core::SessionId::generate(),
            actuator_id: interaction_core::ActuatorId::new(actuator),
            intent: "test".into(),
            risk_class: RiskClass::Low,
            requested: ActionParameters::default(),
            effective: ActionParameters {
                extra,
                ..Default::default()
            },
            policy_decisions: vec![],
            expires_at: Utc::now() + chrono::Duration::seconds(30),
            issued_at: Utc::now(),
            correlation_id: interaction_core::CorrelationId::generate(),
            metadata: BTreeMap::new(),
            schema_version: interaction_core::SCHEMA_VERSION.into(),
        };
        for (kind, id) in [
            (PresentationKind::AnimationPlay, "companion.animation.play"),
            (PresentationKind::StatePresent, "companion.state.present"),
            (PresentationKind::BubbleShow, "companion.bubble.show"),
            (PresentationKind::Speak, "companion.speak"),
            (PresentationKind::PresenceSet, "companion.presence.set"),
        ] {
            let reqs = open_selection_requirements(id);
            assert!(!reqs.is_empty(), "{id} must declare requirements");
            // 缺了宣告的必要參數（payload 空、message 空）必被拒絕。
            assert!(
                validate_params(kind, &mk(id, None)).is_err(),
                "{id} without required params must be rejected at execute"
            );
        }
        // 未列出的 actuator 不受此閘限制。
        assert!(open_selection_requirements("companion.sound.play").is_empty());
        assert!(open_selection_requirements("conversation").is_empty());
    }

    #[test]
    fn sweep_returns_only_expired() {
        let bridge = PresentationBridge::new();
        let now = Utc::now();
        let a = ActionId::new("action-a");
        let b = ActionId::new("action-b");
        bridge
            .enqueue(
                &a,
                PresentationKind::BubbleShow,
                json!({"message": "a"}),
                now,
            )
            .unwrap();
        bridge
            .enqueue(
                &b,
                PresentationKind::BubbleShow,
                json!({"message": "b"}),
                now + chrono::Duration::seconds(8),
            )
            .unwrap();
        let expired = bridge.sweep_expired(now + chrono::Duration::milliseconds(ACK_TTL_MS + 1));
        assert_eq!(expired, vec!["action-a".to_string()]);
        assert!(bridge.take_pending("action-b").is_some());
    }

    #[test]
    fn behavior_intent_whitelist_refuses_unknown() {
        use interaction_core::{ActionParameters, BoundedAction};
        let mk = |intent: &str| BoundedAction {
            action_id: ActionId::generate(),
            plan_id: interaction_core::PlanId::generate(),
            session_id: interaction_core::SessionId::generate(),
            actuator_id: interaction_core::ActuatorId::new("companion.state.present"),
            intent: "test".into(),
            risk_class: RiskClass::Low,
            requested: ActionParameters::default(),
            effective: ActionParameters {
                extra: Some(json!({ "behaviorIntent": intent })),
                ..Default::default()
            },
            policy_decisions: vec![],
            expires_at: Utc::now() + chrono::Duration::seconds(30),
            issued_at: Utc::now(),
            correlation_id: interaction_core::CorrelationId::generate(),
            metadata: BTreeMap::new(),
            schema_version: interaction_core::SCHEMA_VERSION.into(),
        };
        assert!(
            validate_params(PresentationKind::StatePresent, &mk("look-at-confirmation")).is_ok()
        );
        // 未登記的 intent 一律拒絕——AI 不能編造動作。
        assert!(validate_params(PresentationKind::StatePresent, &mk("do-a-backflip")).is_err());
        // 成功／安全語意不可直接點播。
        assert!(
            validate_params(PresentationKind::StatePresent, &mk("celebrate-verified")).is_err()
        );
    }

    #[test]
    fn animation_whitelist_excludes_truth_states() {
        for truth in [
            "success",
            "blocked",
            "unknown",
            "failed",
            "emergency",
            "offline",
        ] {
            assert!(
                !PLAYABLE_ANIMATIONS.contains(&truth),
                "{truth} must never be directly playable"
            );
        }
    }
}
