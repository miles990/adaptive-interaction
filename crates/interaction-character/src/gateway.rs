//! Character Presentation Gateway：純狀態機（沒有 I/O、沒有計時器；時間由呼叫端注入）。
//!
//! 職責（§0／§5／§7／§8）：能力協商、priority 下限、去重（messageId 環 256）、過期、
//! pending 上限 64／outbound 上限 32（安全 intent 永不丟）、搶占（§5）、回執合法順序、
//! `acknowledged → uncertain`、世代（舊世代回執／事件一律丟棄）、斷線 → 全部 `uncertain`、
//! 多實例安全去重、零能力時回退 `system.text`。
//!
//! **絕不**接受來自 adapter 的 `truthState`／`verified`：adapter → runtime 的訊息型別沒有這些欄位，
//! 只有 Runtime 建構的 [`IntentEnvelope`] 帶 `truthState`。

use crate::capability::{
    capability_channels, negotiate, IntentResolution, NegotiationError, Resolution,
};
use crate::input::{
    CharacterInputEvent, InputDecision, InputDropReason, InputLimits, InputNormalizer,
};
use crate::intent::{
    normalize_envelope, CharacterIntent, IntentEnvelope, InterruptPolicy, ResumePolicy, TruthState,
};
use crate::lifecycle::{AdapterLifecycleState, CharacterRole};
use crate::manifest::{CapabilityDecl, CharacterManifest};
use crate::receipt::{ack_uncertain_deadline, can_transition, CommandReceipt, ReceiptStatus};
use crate::wire::{Hello, HelloLimits, Limits, Negotiate, Negotiated, RateLimiter, WireMessage};
use crate::{parse_protocol_version, Timestamp, PROTOCOL_MAJOR, PROTOCOL_VERSION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Character Instance id（`characterInstanceId`）。
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct InstanceId(pub String);

impl InstanceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Gateway 設定（上限只能收緊，不能放寬協定常數）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfig {
    pub runtime_version: String,
    pub locale: String,
    pub reduced_motion: bool,
    pub max_pending: usize,
    pub max_outbound: usize,
    pub disconnect_after_ms: i64,
    pub dedupe_ring: usize,
    /// `started` 之後超過 `expiresAt + max(durationHint, 這個值)` 仍無回執 → `uncertain`（watchdog）。
    pub started_watchdog_ms: i64,
    pub input_limits: InputLimits,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        GatewayConfig {
            runtime_version: env!("CARGO_PKG_VERSION").to_string(),
            locale: "zh-TW".to_string(),
            reduced_motion: false,
            max_pending: Limits::MAX_PENDING,
            max_outbound: Limits::MAX_OUTBOUND,
            disconnect_after_ms: Limits::DISCONNECT_AFTER_MS,
            dedupe_ring: Limits::DEDUPE_RING,
            started_watchdog_ms: 60_000,
            input_limits: InputLimits::default(),
        }
    }
}

/// Gateway 的輸出：呼叫端（transport／Runtime）負責實際送出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "output",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum GatewayOutput {
    /// 送給 adapter 的 wire message。
    Send {
        instance: InstanceId,
        message: WireMessage,
    },
    /// 零能力／失敗時的安全退路：由 Runtime 以文字／通知呈現（不得遺失）。
    SystemText {
        instance: InstanceId,
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        correlation_id: Option<String>,
        intent: CharacterIntent,
        truth_state: TruthState,
        message: String,
    },
    /// 回執（進 Runtime audit／`character.receipt` 事件；不改動任何工作 verification）。
    Receipt(CommandReceipt),
    /// 稽核訊息（舊世代、非法轉移、去重、rate limit…）。
    Audit(String),
}

/// 斷線原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DisconnectReason {
    Goodbye,
    HeartbeatTimeout,
    Crash,
    TransportClosed,
    Revoked,
}

impl DisconnectReason {
    fn as_str(&self) -> &'static str {
        match self {
            DisconnectReason::Goodbye => "goodbye",
            DisconnectReason::HeartbeatTimeout => "heartbeat-timeout",
            DisconnectReason::Crash => "crash",
            DisconnectReason::TransportClosed => "transport-closed",
            DisconnectReason::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone)]
struct PendingCommand {
    envelope: IntentEnvelope,
    status: ReceiptStatus,
    resolution: IntentResolution,
    interruptible: bool,
    channels: Vec<String>,
    /// adapter 是否已對這個 command 回過任何回執（outbound 佇列以此計算）。
    adapter_seen: bool,
    acked_at: Option<Timestamp>,
    /// `system.text` 交棒：不在 adapter 上，不佔 pending 上限。
    handoff: bool,
}

/// 單一 instance 的狀態（唯讀檢視）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InstanceView {
    pub id: InstanceId,
    pub character_id: String,
    pub role: CharacterRole,
    pub generation: u64,
    pub lifecycle: AdapterLifecycleState,
    pub connected: bool,
    pub negotiated: bool,
    pub pending: usize,
    pub handoffs: usize,
}

struct InstanceState {
    id: InstanceId,
    manifest: CharacterManifest,
    role: CharacterRole,
    generation: u64,
    /// 這個 instance 的 Reduced Motion（由可信 host 的 hello 帶進來）；`None` = 沿用 config 預設。
    reduced_motion: Option<bool>,
    negotiated: Option<Negotiated>,
    lifecycle: AdapterLifecycleState,
    connected: bool,
    last_seen: Option<Timestamp>,
    pending: Vec<PendingCommand>,
    dedupe_ring: VecDeque<String>,
    dedupe_set: BTreeSet<String>,
    terminal: BTreeMap<String, CommandReceipt>,
    cancel_results: BTreeMap<String, CommandReceipt>,
    resume_stack: Vec<IntentEnvelope>,
    resume_seq: u64,
    input: InputNormalizer,
    rate: RateLimiter,
    /// 上一次寫出 wire-rejected 稽核的時間（毫秒）；`None` = 還沒寫過。
    wire_reject_audit_at: Option<i64>,
    /// 自上次稽核以來被壓下的畸形訊息數（有界稽核用的計數器）。
    wire_reject_suppressed: u64,
}

/// 畸形（parse 失敗）訊息的計費結果：先扣速率預算，再決定要不要寫稽核。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireRejectVerdict {
    /// 這則訊息是否還在 50 則/s 的預算內（false = 已超量，連錯誤回覆都不再送）。
    pub within_rate: bool,
    /// 是否應該寫一列稽核（每個 instance 每 [`WIRE_REJECT_AUDIT_WINDOW_MS`] 至多一列）。
    pub audit: bool,
    /// 這一列稽核代表的、上次稽核後被壓下的畸形訊息數（`audit=false` 時為 0）。
    pub suppressed: u64,
}

/// 畸形訊息稽核的時間窗：同一個 instance 每 5 秒至多留下一列（其餘只累加計數）。
pub const WIRE_REJECT_AUDIT_WINDOW_MS: i64 = 5_000;

impl InstanceState {
    fn remember_message_id(&mut self, message_id: &str, ring: usize) {
        if self.dedupe_ring.len() >= ring.max(1) {
            if let Some(old) = self.dedupe_ring.pop_front() {
                self.dedupe_set.remove(&old);
                self.terminal.remove(&old);
                self.cancel_results.remove(&old);
            }
        }
        self.dedupe_ring.push_back(message_id.to_string());
        self.dedupe_set.insert(message_id.to_string());
    }

    fn receipt(&self, message_id: &str, status: ReceiptStatus, at: Timestamp) -> CommandReceipt {
        CommandReceipt::new(message_id, self.id.as_str(), self.generation, status, at)
    }

    fn live_pending(&self) -> usize {
        self.pending
            .iter()
            .filter(|p| !p.handoff && !p.status.is_terminal())
            .count()
    }

    fn outbound_unseen(&self) -> usize {
        self.pending
            .iter()
            .filter(|p| !p.handoff && !p.status.is_terminal() && !p.adapter_seen)
            .count()
    }

    fn finish(&mut self, idx: usize, receipt: CommandReceipt) {
        let cmd = self.pending.remove(idx);
        self.terminal
            .insert(cmd.envelope.message_id.clone(), receipt);
    }
}

/// §3.4 步驟 5／6：零能力或執行失敗時的固定安全語句（由 Runtime／host 呈現）。
pub fn default_system_text(intent: CharacterIntent) -> &'static str {
    match intent {
        CharacterIntent::Emergency => "緊急停止已啟動，所有動作已停止。",
        CharacterIntent::Offline => "裝置離線。",
        CharacterIntent::Blocked => "動作被安全政策擋下。",
        CharacterIntent::Failed => "動作失敗。",
        CharacterIntent::RequestConsent => "需要你的同意才能繼續。",
        CharacterIntent::Unknown => "結果未知，需要確認。",
        CharacterIntent::VerifiedSuccess => "已由人類驗證完成。",
        CharacterIntent::ClaimCompleted => "AI 宣稱已完成（尚未驗證）。",
        CharacterIntent::Wait => "排隊等待中。",
        CharacterIntent::Ask => "需要你的輸入。",
        CharacterIntent::Cancelled => "已取消。",
        CharacterIntent::Idle => "待命。",
        CharacterIntent::Notice => "有新狀態。",
        CharacterIntent::Acknowledge => "已收到。",
        CharacterIntent::Think => "思考中。",
        CharacterIntent::Work => "工作中。",
        CharacterIntent::Greet => "你好。",
        CharacterIntent::Play => "玩耍中。",
        CharacterIntent::Rest => "休息中。",
        CharacterIntent::Sleep => "休眠中。",
    }
}

/// Gateway 純狀態機。
pub struct Gateway {
    config: GatewayConfig,
    instances: BTreeMap<InstanceId, InstanceState>,
    seq: u64,
    /// correlationId → role class → 已送達的 instance（安全 intent 多實例去重）。
    safety_routes: BTreeMap<String, BTreeMap<String, InstanceId>>,
    safety_ring: VecDeque<String>,
}

impl Default for Gateway {
    fn default() -> Self {
        Gateway::new(GatewayConfig::default())
    }
}

impl Gateway {
    pub fn new(config: GatewayConfig) -> Self {
        let mut config = config;
        config.max_pending = config.max_pending.clamp(1, Limits::MAX_PENDING);
        config.max_outbound = config.max_outbound.clamp(1, Limits::MAX_OUTBOUND);
        config.dedupe_ring = config.dedupe_ring.clamp(1, Limits::DEDUPE_RING);
        Gateway {
            config,
            instances: BTreeMap::new(),
            seq: 0,
            safety_routes: BTreeMap::new(),
            safety_ring: VecDeque::new(),
        }
    }

    pub fn config(&self) -> &GatewayConfig {
        &self.config
    }

    /// 註冊一個 instance（id 為 `characterId#序號`，確定性）。
    pub fn register_instance(
        &mut self,
        manifest: CharacterManifest,
        role: CharacterRole,
    ) -> InstanceId {
        self.seq += 1;
        let id = InstanceId(format!("{}#{}", manifest.character_id, self.seq));
        self.register_instance_with_id(id.clone(), manifest, role);
        id
    }

    /// 以呼叫端指定的 id 註冊（同 id 重複註冊會覆蓋舊狀態）。
    pub fn register_instance_with_id(
        &mut self,
        id: InstanceId,
        manifest: CharacterManifest,
        role: CharacterRole,
    ) {
        let state = InstanceState {
            id: id.clone(),
            manifest,
            role,
            generation: 0,
            reduced_motion: None,
            negotiated: None,
            lifecycle: AdapterLifecycleState::Validated,
            connected: false,
            last_seen: None,
            pending: Vec::new(),
            dedupe_ring: VecDeque::with_capacity(self.config.dedupe_ring),
            dedupe_set: BTreeSet::new(),
            terminal: BTreeMap::new(),
            cancel_results: BTreeMap::new(),
            resume_stack: Vec::new(),
            resume_seq: 0,
            input: InputNormalizer::new(role, self.config.input_limits),
            rate: RateLimiter::new(Limits::MAX_MESSAGES_PER_SEC, 0),
            wire_reject_audit_at: None,
            wire_reject_suppressed: 0,
        };
        self.instances.insert(id, state);
    }

    /// 設定這個 instance 的 Reduced Motion（只有可信 host／Runtime 能呼叫；adapter 不能自己宣告）。
    /// 之後的 `hello_for`／`on_negotiate` 都以這個值協商。回傳 instance 是否存在。
    pub fn set_reduced_motion(&mut self, id: &InstanceId, on: bool) -> bool {
        match self.instances.get_mut(id) {
            Some(inst) => {
                inst.reduced_motion = Some(on);
                true
            }
            None => false,
        }
    }

    /// 這個 instance 目前協商用的 Reduced Motion（沒設定過就是 config 預設）。
    pub fn reduced_motion(&self, id: &InstanceId) -> Option<bool> {
        self.instances
            .get(id)
            .map(|i| i.reduced_motion.unwrap_or(self.config.reduced_motion))
    }

    /// 對這個 instance 扣一則速率預算（§8：每個 adapter ≤ 50 則/s）。
    /// HTTP `POST /v1/character/{receipts,events}` 與 WebSocket 共用同一個計數器。
    /// 未知 instance 回 `true`（由呼叫端各自回 404／unknown-instance）。
    pub fn allow_message(&mut self, id: &InstanceId, now: Timestamp) -> bool {
        match self.instances.get_mut(id) {
            Some(inst) => inst.rate.allow(now.timestamp_millis()),
            None => true,
        }
    }

    /// 畸形訊息（連 wire 都解不開）：**先**扣速率預算，再決定要不要寫稽核。
    /// 稽核有界：同一個 instance 每 [`WIRE_REJECT_AUDIT_WINDOW_MS`] 至多一列，
    /// 被壓下的次數累加到下一列的 `suppressed`。
    pub fn note_wire_rejected(&mut self, id: &InstanceId, now: Timestamp) -> WireRejectVerdict {
        let ms = now.timestamp_millis();
        let Some(inst) = self.instances.get_mut(id) else {
            return WireRejectVerdict {
                within_rate: true,
                audit: true,
                suppressed: 0,
            };
        };
        let within_rate = inst.rate.allow(ms);
        let due = inst
            .wire_reject_audit_at
            .is_none_or(|last| ms.saturating_sub(last) >= WIRE_REJECT_AUDIT_WINDOW_MS);
        if due {
            let suppressed = inst.wire_reject_suppressed;
            inst.wire_reject_suppressed = 0;
            inst.wire_reject_audit_at = Some(ms);
            WireRejectVerdict {
                within_rate,
                audit: true,
                suppressed,
            }
        } else {
            inst.wire_reject_suppressed = inst.wire_reject_suppressed.saturating_add(1);
            WireRejectVerdict {
                within_rate,
                audit: false,
                suppressed: 0,
            }
        }
    }

    /// 移除 instance（dispose）：進行中的 command 全部 `uncertain`。
    pub fn remove_instance(&mut self, id: &InstanceId, now: Timestamp) -> Vec<GatewayOutput> {
        let mut out = self.on_disconnect(id, DisconnectReason::Revoked, now);
        if let Some(mut inst) = self.instances.remove(id) {
            for p in inst.pending.drain(..) {
                if !p.status.is_terminal() {
                    out.push(GatewayOutput::Receipt(
                        CommandReceipt::new(
                            p.envelope.message_id,
                            id.as_str(),
                            inst.generation,
                            ReceiptStatus::Uncertain,
                            now,
                        )
                        .with_reason("disposed"),
                    ));
                }
            }
            out.push(GatewayOutput::Audit(format!("instance {id} disposed")));
        }
        out
    }

    pub fn instance(&self, id: &InstanceId) -> Option<InstanceView> {
        self.instances.get(id).map(|i| InstanceView {
            id: i.id.clone(),
            character_id: i.manifest.character_id.clone(),
            role: i.role,
            generation: i.generation,
            lifecycle: i.lifecycle,
            connected: i.connected,
            negotiated: i.negotiated.is_some(),
            pending: i.live_pending(),
            handoffs: i.pending.iter().filter(|p| p.handoff).count(),
        })
    }

    pub fn instances(&self) -> Vec<InstanceView> {
        self.instances
            .keys()
            .filter_map(|id| self.instance(id))
            .collect()
    }

    pub fn generation(&self, id: &InstanceId) -> Option<u64> {
        self.instances.get(id).map(|i| i.generation)
    }

    pub fn negotiated(&self, id: &InstanceId) -> Option<&Negotiated> {
        self.instances.get(id).and_then(|i| i.negotiated.as_ref())
    }

    pub fn manifest(&self, id: &InstanceId) -> Option<&CharacterManifest> {
        self.instances.get(id).map(|i| &i.manifest)
    }

    /// 目前狀態（pending 或已終結）。
    pub fn command_status(&self, id: &InstanceId, message_id: &str) -> Option<ReceiptStatus> {
        let inst = self.instances.get(id)?;
        inst.pending
            .iter()
            .find(|p| p.envelope.message_id == message_id)
            .map(|p| p.status)
            .or_else(|| inst.terminal.get(message_id).map(|r| r.status))
    }

    /// §3.3 步驟 1：`hello`。
    pub fn hello_for(&self, id: &InstanceId) -> Option<Hello> {
        let inst = self.instances.get(id)?;
        Some(Hello {
            protocol_version: PROTOCOL_VERSION.to_string(),
            runtime_version: self.config.runtime_version.clone(),
            character_instance_id: inst.id.0.clone(),
            role: inst.role,
            locale: self.config.locale.clone(),
            reduced_motion: inst.reduced_motion.unwrap_or(self.config.reduced_motion),
            requires: CharacterIntent::ALL.to_vec(),
            limits: HelloLimits {
                max_message_bytes: Limits::MAX_MESSAGE_BYTES,
                max_messages_per_second: Limits::MAX_MESSAGES_PER_SEC,
                max_pending: self.config.max_pending,
            },
        })
    }

    /// §3.3 步驟 2→3：處理 `negotiate`，回 `negotiated` 與輸出。
    /// 有效能力 = manifest 宣告 ∩ offer 宣告（兩邊都 supported）；重新協商時 pending 先全部 `uncertain`。
    pub fn on_negotiate(
        &mut self,
        id: &InstanceId,
        offer: Negotiate,
        now: Timestamp,
    ) -> Result<(Negotiated, Vec<GatewayOutput>), NegotiationError> {
        let hello = self
            .hello_for(id)
            .ok_or(NegotiationError::UnknownInstance)?;
        let inst = self
            .instances
            .get_mut(id)
            .ok_or(NegotiationError::UnknownInstance)?;
        inst.last_seen = Some(now);
        if offer.character_id != inst.manifest.character_id {
            return Err(NegotiationError::CharacterMismatch {
                expected: inst.manifest.character_id.clone(),
                offered: crate::truncate_for_echo(&offer.character_id),
            });
        }
        let intersect = |manifest: &BTreeMap<String, CapabilityDecl>,
                         offered: &BTreeMap<String, CapabilityDecl>| {
            offered
                .iter()
                .filter(|(k, d)| d.supported && manifest.get(*k).is_some_and(|m| m.supported))
                .map(|(k, d)| (k.clone(), d.clone()))
                .collect::<BTreeMap<_, _>>()
        };
        let manifest_intents: BTreeSet<&String> = inst.manifest.intents.iter().collect();
        let manifest_channels: BTreeSet<&String> = inst.manifest.channels.iter().collect();
        let effective = Negotiate {
            capabilities: intersect(&inst.manifest.capabilities, &offer.capabilities),
            input_capabilities: intersect(
                &inst.manifest.input_capabilities,
                &offer.input_capabilities,
            ),
            intents: offer
                .intents
                .iter()
                .filter(|i| manifest_intents.contains(i))
                .cloned()
                .collect(),
            channels: offer
                .channels
                .iter()
                .filter(|c| manifest_channels.contains(c))
                .cloned()
                .collect(),
            ..offer.clone()
        };
        let mut negotiated = negotiate(&hello, &effective, &inst.manifest.fallbacks)?;

        let mut out = Vec::new();
        if inst.negotiated.is_some() || inst.live_pending() > 0 {
            out.push(GatewayOutput::Audit(format!(
                "instance {id} re-negotiated; pending commands marked uncertain"
            )));
            let _ = Self::mark_all_uncertain(inst, "re-negotiated", now, &mut out);
        }
        inst.generation += 1;
        negotiated.generation = inst.generation;
        inst.negotiated = Some(negotiated.clone());
        inst.connected = true;
        inst.lifecycle = AdapterLifecycleState::Ready;
        inst.resume_stack.clear();
        out.push(GatewayOutput::Audit(format!(
            "instance {id} negotiated generation {} ({} capabilities, {} ignored channels)",
            inst.generation,
            negotiated.capabilities.len(),
            negotiated.ignored_channels.len()
        )));
        out.push(GatewayOutput::Send {
            instance: id.clone(),
            message: WireMessage::Negotiated(negotiated.clone()),
        });
        Ok((negotiated, out))
    }

    /// 進行中的 command 全部 `uncertain`（不猜 completed），回傳其中的安全 intent envelope
    /// （呼叫端決定要不要用 `system.text` 補送——斷線／crash 要補，重新協商不補）。
    fn mark_all_uncertain(
        inst: &mut InstanceState,
        reason: &str,
        now: Timestamp,
        out: &mut Vec<GatewayOutput>,
    ) -> Vec<IntentEnvelope> {
        let mut safety = Vec::new();
        let mut idx = 0;
        while idx < inst.pending.len() {
            if inst.pending[idx].handoff || inst.pending[idx].status.is_terminal() {
                idx += 1;
                continue;
            }
            let message_id = inst.pending[idx].envelope.message_id.clone();
            let resolution = inst.pending[idx].resolution.resolution;
            let envelope = inst.pending[idx].envelope.clone();
            let receipt = inst
                .receipt(&message_id, ReceiptStatus::Uncertain, now)
                .with_resolution(resolution)
                .with_reason(reason);
            inst.finish(idx, receipt.clone());
            if envelope.intent.is_safety() {
                safety.push(envelope);
            }
            out.push(GatewayOutput::Receipt(receipt));
        }
        safety
    }

    /// 斷線／crash 時，把還沒演完的安全 intent 以 `system.text` 補送（§9：crash → `uncertain` ＋ fallback）。
    /// 用衍生的 messageId，原本的 `uncertain` 回執不被覆蓋。
    fn resend_safety_as_system_text(
        inst: &mut InstanceState,
        envelopes: Vec<IntentEnvelope>,
        ring: usize,
        detail: &str,
        now: Timestamp,
        out: &mut Vec<GatewayOutput>,
    ) {
        for envelope in envelopes {
            let mut fallback = envelope.clone();
            fallback.message_id = format!("{}/system-text", envelope.message_id);
            if inst.dedupe_set.contains(&fallback.message_id) {
                continue;
            }
            inst.remember_message_id(&fallback.message_id, ring);
            out.push(GatewayOutput::Audit(format!(
                "disconnect: safety intent {} was in flight; falling back to system.text",
                envelope.message_id
            )));
            Self::system_text(
                inst,
                &fallback,
                IntentResolution::system_text(),
                detail,
                now,
                out,
            );
        }
    }

    fn system_text(
        inst: &mut InstanceState,
        envelope: &IntentEnvelope,
        resolution: IntentResolution,
        detail: &str,
        now: Timestamp,
        out: &mut Vec<GatewayOutput>,
    ) {
        let hint = envelope
            .presentation_hints
            .as_ref()
            .and_then(|h| h.message.clone());
        let message = match hint {
            Some(h) => format!("{} — {}", default_system_text(envelope.intent), h),
            None => default_system_text(envelope.intent).to_string(),
        };
        out.push(GatewayOutput::SystemText {
            instance: inst.id.clone(),
            message_id: envelope.message_id.clone(),
            correlation_id: envelope.correlation_id.clone(),
            intent: envelope.intent,
            truth_state: envelope.truth_state,
            message,
        });
        let receipt = inst
            .receipt(&envelope.message_id, ReceiptStatus::Acknowledged, now)
            .with_resolution(Resolution::Substituted)
            .with_detail(detail);
        // handoff 數量以去重環為界：超過即把最舊的記成 uncertain。
        let handoffs = inst.pending.iter().filter(|p| p.handoff).count();
        if handoffs >= Limits::DEDUPE_RING {
            if let Some(idx) = inst.pending.iter().position(|p| p.handoff) {
                let old_id = inst.pending[idx].envelope.message_id.clone();
                let r = inst
                    .receipt(&old_id, ReceiptStatus::Uncertain, now)
                    .with_reason("handoff-overflow");
                inst.finish(idx, r.clone());
                out.push(GatewayOutput::Receipt(r));
            }
        }
        inst.pending.push(PendingCommand {
            envelope: envelope.clone(),
            status: ReceiptStatus::Acknowledged,
            resolution,
            interruptible: true,
            channels: Vec::new(),
            adapter_seen: true,
            acked_at: Some(now),
            handoff: true,
        });
        out.push(GatewayOutput::Receipt(receipt));
    }

    fn cancel_pending(
        inst: &mut InstanceState,
        idx: usize,
        reason: &str,
        now: Timestamp,
        out: &mut Vec<GatewayOutput>,
    ) {
        let message_id = inst.pending[idx].envelope.message_id.clone();
        let resolution = inst.pending[idx].resolution.resolution;
        let handoff = inst.pending[idx].handoff;
        let receipt = inst
            .receipt(&message_id, ReceiptStatus::Cancelled, now)
            .with_resolution(resolution)
            .with_reason(reason);
        inst.finish(idx, receipt.clone());
        inst.cancel_results
            .insert(message_id.clone(), receipt.clone());
        if !handoff && inst.connected {
            out.push(GatewayOutput::Send {
                instance: inst.id.clone(),
                message: WireMessage::Cancel {
                    message_id,
                    reason: Some(reason.to_string()),
                },
            });
        }
        out.push(GatewayOutput::Receipt(receipt));
    }

    /// 派送一個 Runtime 建構的 envelope。
    pub fn dispatch(
        &mut self,
        id: &InstanceId,
        envelope: IntentEnvelope,
        now: Timestamp,
    ) -> Vec<GatewayOutput> {
        let mut out = Vec::new();
        let Some(role) = self.instances.get(id).map(|i| i.role) else {
            out.push(GatewayOutput::Audit(format!(
                "dispatch: unknown instance {id}"
            )));
            return out;
        };
        let envelope = match normalize_envelope(&envelope) {
            Ok(e) => e,
            Err(err) => {
                out.push(GatewayOutput::Audit(format!(
                    "dispatch: invalid envelope: {err}"
                )));
                if let Some(inst) = self.instances.get(id) {
                    out.push(GatewayOutput::Receipt(
                        inst.receipt(&envelope.message_id, ReceiptStatus::Failed, now)
                            .with_resolution(Resolution::Failed)
                            .with_detail(format!("invalid envelope: {err}")),
                    ));
                }
                return out;
            }
        };
        if envelope.character_instance_id != id.0 {
            out.push(GatewayOutput::Audit(format!(
                "dispatch: envelope {} addressed to another instance",
                envelope.message_id
            )));
            return out;
        }

        // 多實例安全去重（同 correlationId 每個 role class 只送一個非 notification-only instance）。
        let mut suppressed_by: Option<InstanceId> = None;
        if envelope.intent.is_safety() && !role.is_notification_only() {
            if let Some(corr) = &envelope.correlation_id {
                let class = role.as_str().to_string();
                let connected = |gw: &Gateway, other: &InstanceId| {
                    gw.instances
                        .get(other)
                        .is_some_and(|i| i.connected && i.negotiated.is_some())
                };
                let existing = self
                    .safety_routes
                    .get(corr)
                    .and_then(|m| m.get(&class))
                    .cloned();
                match existing {
                    Some(other) if other != *id && connected(self, &other) => {
                        suppressed_by = Some(other);
                    }
                    _ => {
                        if !self.safety_routes.contains_key(corr) {
                            if self.safety_ring.len() >= Limits::DEDUPE_RING {
                                if let Some(old) = self.safety_ring.pop_front() {
                                    self.safety_routes.remove(&old);
                                }
                            }
                            self.safety_ring.push_back(corr.clone());
                        }
                        self.safety_routes
                            .entry(corr.clone())
                            .or_default()
                            .insert(class, id.clone());
                    }
                }
            }
        }

        let ring = self.config.dedupe_ring;
        let max_pending = self.config.max_pending;
        let max_outbound = self.config.max_outbound;
        let Some(inst) = self.instances.get_mut(id) else {
            return out;
        };

        // 去重（環 256）。
        if inst.dedupe_set.contains(&envelope.message_id) {
            let mut receipt = inst.receipt(&envelope.message_id, ReceiptStatus::Accepted, now);
            receipt.duplicate = true;
            if let Some(p) = inst
                .pending
                .iter()
                .find(|p| p.envelope.message_id == envelope.message_id)
            {
                receipt.resolution = Some(p.resolution.resolution);
            } else if let Some(t) = inst.terminal.get(&envelope.message_id) {
                receipt.resolution = t.resolution;
            }
            out.push(GatewayOutput::Receipt(receipt));
            return out;
        }
        inst.remember_message_id(&envelope.message_id, ring);

        // 過期不播。
        if envelope.expires_at <= now {
            let receipt = inst
                .receipt(&envelope.message_id, ReceiptStatus::Expired, now)
                .with_detail("expiresAt already passed");
            inst.terminal
                .insert(envelope.message_id.clone(), receipt.clone());
            out.push(GatewayOutput::Receipt(receipt));
            return out;
        }

        if let Some(other) = suppressed_by {
            let receipt = inst
                .receipt(&envelope.message_id, ReceiptStatus::Cancelled, now)
                .with_reason("safety-deduplicated")
                .with_detail(format!("already delivered to {other}"));
            inst.terminal
                .insert(envelope.message_id.clone(), receipt.clone());
            out.push(GatewayOutput::Audit(format!(
                "dispatch: safety intent {} for correlation {} suppressed on {id} (delivered to {other})",
                envelope.intent,
                envelope.correlation_id.clone().unwrap_or_default()
            )));
            out.push(GatewayOutput::Receipt(receipt));
            return out;
        }

        // 解析。
        let resolution = inst
            .negotiated
            .as_ref()
            .filter(|_| inst.connected)
            .and_then(|n| n.resolutions.get(&envelope.intent).cloned())
            .unwrap_or_else(|| {
                if envelope.intent.is_safety() {
                    IntentResolution::system_text()
                } else {
                    IntentResolution::unsupported()
                }
            });
        if resolution.resolution == Resolution::Unsupported {
            let detail = if inst.negotiated.is_some() && inst.connected {
                "no capability can present this intent"
            } else {
                "instance not negotiated"
            };
            let receipt = inst
                .receipt(&envelope.message_id, ReceiptStatus::Unsupported, now)
                .with_resolution(Resolution::Unsupported)
                .with_detail(detail);
            inst.terminal
                .insert(envelope.message_id.clone(), receipt.clone());
            out.push(GatewayOutput::Receipt(receipt));
            return out;
        }
        if resolution.is_system_text() {
            Self::system_text(
                inst,
                &envelope,
                resolution,
                "system.text fallback",
                now,
                &mut out,
            );
            return out;
        }

        let via = resolution
            .via
            .as_ref()
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();
        let decl = inst
            .negotiated
            .as_ref()
            .and_then(|n| n.capabilities.get(&via).cloned());
        let interruptible = decl.as_ref().map(|d| d.interruptible).unwrap_or(true);
        let channels: Vec<String> = capability_channels(&via)
            .iter()
            .map(|s| s.to_string())
            .collect();

        // merge：同 intent＋同 correlation 已有進行中的 command → 不另派。
        if envelope.interrupt_policy == InterruptPolicy::Merge {
            if let Some(existing) = inst.pending.iter().find(|p| {
                !p.handoff
                    && !p.status.is_terminal()
                    && p.envelope.intent == envelope.intent
                    && p.envelope.correlation_id == envelope.correlation_id
            }) {
                let receipt = inst
                    .receipt(&envelope.message_id, ReceiptStatus::Cancelled, now)
                    .with_resolution(resolution.resolution)
                    .with_reason("merged")
                    .with_detail(format!("merged into {}", existing.envelope.message_id));
                inst.terminal
                    .insert(envelope.message_id.clone(), receipt.clone());
                out.push(GatewayOutput::Receipt(receipt));
                return out;
            }
        }

        // channel 衝突。
        let conflicts: Vec<usize> = inst
            .pending
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                !p.handoff
                    && !p.status.is_terminal()
                    && p.status != ReceiptStatus::Acknowledged
                    && p.channels.iter().any(|c| channels.contains(c))
            })
            .map(|(i, _)| i)
            .collect();
        match envelope.interrupt_policy {
            InterruptPolicy::DropIfBusy if !conflicts.is_empty() => {
                let receipt = inst
                    .receipt(&envelope.message_id, ReceiptStatus::Cancelled, now)
                    .with_resolution(resolution.resolution)
                    .with_reason("busy");
                inst.terminal
                    .insert(envelope.message_id.clone(), receipt.clone());
                out.push(GatewayOutput::Receipt(receipt));
                return out;
            }
            InterruptPolicy::Preempt => {
                // 由高索引往低索引移除，避免位移。
                for idx in conflicts.into_iter().rev() {
                    let p = &inst.pending[idx];
                    let can_preempt = envelope.priority > p.envelope.priority
                        && (p.interruptible || envelope.intent.preempts_non_interruptible());
                    if !can_preempt {
                        continue;
                    }
                    if p.envelope.resume_policy == ResumePolicy::ResumePrevious
                        && p.status == ReceiptStatus::Started
                        && envelope.intent.is_safety()
                        && inst.resume_stack.len() < 8
                    {
                        inst.resume_stack.push(p.envelope.clone());
                    }
                    Self::cancel_pending(inst, idx, "preempted", now, &mut out);
                }
            }
            _ => {}
        }

        // pending 上限 64：先丟最舊的非安全 intent；安全 intent 永不丟。
        if inst.live_pending() >= max_pending {
            if let Some(idx) = inst.pending.iter().position(|p| {
                !p.handoff && !p.status.is_terminal() && !p.envelope.intent.is_safety()
            }) {
                Self::cancel_pending(inst, idx, "queue-full", now, &mut out);
            } else if envelope.intent.is_safety() {
                out.push(GatewayOutput::Audit(format!(
                    "dispatch: pending queue of {id} is full of safety intents; {} handed to system.text",
                    envelope.message_id
                )));
                Self::system_text(
                    inst,
                    &envelope,
                    IntentResolution::system_text(),
                    "queue-full; system.text fallback",
                    now,
                    &mut out,
                );
                return out;
            } else {
                let receipt = inst
                    .receipt(&envelope.message_id, ReceiptStatus::Cancelled, now)
                    .with_resolution(resolution.resolution)
                    .with_reason("queue-full");
                inst.terminal
                    .insert(envelope.message_id.clone(), receipt.clone());
                out.push(GatewayOutput::Receipt(receipt));
                return out;
            }
        }
        // outbound 上限 32：adapter 尚未回應的 command 太多 → 丟最舊的非安全 intent。
        if inst.outbound_unseen() >= max_outbound {
            if let Some(idx) = inst.pending.iter().position(|p| {
                !p.handoff
                    && !p.status.is_terminal()
                    && !p.adapter_seen
                    && !p.envelope.intent.is_safety()
            }) {
                Self::cancel_pending(inst, idx, "outbound-full", now, &mut out);
            } else if !envelope.intent.is_safety() {
                let receipt = inst
                    .receipt(&envelope.message_id, ReceiptStatus::Cancelled, now)
                    .with_resolution(resolution.resolution)
                    .with_reason("outbound-full");
                inst.terminal
                    .insert(envelope.message_id.clone(), receipt.clone());
                out.push(GatewayOutput::Receipt(receipt));
                return out;
            }
        }

        let receipt = inst
            .receipt(&envelope.message_id, ReceiptStatus::Accepted, now)
            .with_resolution(resolution.resolution);
        out.push(GatewayOutput::Send {
            instance: id.clone(),
            message: WireMessage::Intent {
                envelope: envelope.clone(),
            },
        });
        inst.pending.push(PendingCommand {
            envelope,
            status: ReceiptStatus::Accepted,
            resolution,
            interruptible,
            channels,
            adapter_seen: false,
            acked_at: None,
            handoff: false,
        });
        out.push(GatewayOutput::Receipt(receipt));
        out
    }

    /// 處理 adapter 回執：舊世代丟棄（audit）、非法轉移丟棄（audit）、同狀態冪等。
    pub fn on_receipt(
        &mut self,
        id: &InstanceId,
        receipt: CommandReceipt,
        now: Timestamp,
    ) -> Vec<GatewayOutput> {
        let mut out = Vec::new();
        let Some(inst) = self.instances.get_mut(id) else {
            out.push(GatewayOutput::Audit(format!(
                "receipt: unknown instance {id}"
            )));
            return out;
        };
        inst.last_seen = Some(now);
        if receipt.generation != inst.generation {
            out.push(GatewayOutput::Audit(format!(
                "receipt: stale generation {} (current {}) for {} on {id}; dropped",
                receipt.generation, inst.generation, receipt.message_id
            )));
            return out;
        }
        if receipt.character_instance_id != inst.id.0 {
            out.push(GatewayOutput::Audit(format!(
                "receipt: {} addressed to another instance; dropped",
                receipt.message_id
            )));
            return out;
        }
        let Some(idx) = inst
            .pending
            .iter()
            .position(|p| p.envelope.message_id == receipt.message_id)
        else {
            let why = if inst.terminal.contains_key(&receipt.message_id) {
                "already terminal"
            } else {
                "unknown messageId"
            };
            out.push(GatewayOutput::Audit(format!(
                "receipt: {} {why}; dropped",
                receipt.message_id
            )));
            return out;
        };
        if inst.pending[idx].handoff {
            out.push(GatewayOutput::Audit(format!(
                "receipt: {} is a system.text handoff, adapter receipt ignored",
                receipt.message_id
            )));
            return out;
        }
        inst.pending[idx].adapter_seen = true;
        let current = inst.pending[idx].status;
        if receipt.status == current {
            return out;
        }
        if !can_transition(current, receipt.status) {
            out.push(GatewayOutput::Audit(format!(
                "receipt: illegal transition {current:?} -> {:?} for {}; dropped",
                receipt.status, receipt.message_id
            )));
            return out;
        }
        inst.pending[idx].status = receipt.status;
        if receipt.status == ReceiptStatus::Acknowledged {
            inst.pending[idx].acked_at = Some(now);
        }
        let negotiated_resolution = inst.pending[idx].resolution.resolution;
        // resolution 只能變差：adapter 可以誠實回報降級（reduced／substituted），
        // 但不能把協商結果升級成 exact（`Resolution` 的 Ord 即 exact < substituted < reduced < unsupported < failed）。
        let effective_resolution = if receipt.status == ReceiptStatus::Failed {
            Resolution::Failed
        } else {
            receipt
                .resolution
                .filter(|r| *r <= Resolution::Reduced)
                .map_or(negotiated_resolution, |r| r.max(negotiated_resolution))
        };
        let mut gateway_receipt = inst
            .receipt(&receipt.message_id, receipt.status, now)
            .with_resolution(effective_resolution);
        if let Some(detail) = &receipt.detail {
            gateway_receipt = gateway_receipt.with_detail(detail.clone());
        }
        if let Some(reason) = &receipt.reason {
            gateway_receipt = gateway_receipt.with_reason(crate::truncate_for_echo(reason));
        }
        out.push(GatewayOutput::Receipt(gateway_receipt.clone()));

        if receipt.status.is_terminal() {
            let envelope = inst.pending[idx].envelope.clone();
            inst.finish(idx, gateway_receipt);
            if receipt.status == ReceiptStatus::Failed && envelope.intent.is_safety() {
                out.push(GatewayOutput::Audit(format!(
                    "receipt: safety intent {} failed on adapter; falling back to system.text",
                    envelope.message_id
                )));
                let mut fallback = envelope.clone();
                fallback.message_id = format!("{}/system-text", envelope.message_id);
                inst.remember_message_id(&fallback.message_id, self.config.dedupe_ring);
                Self::system_text(
                    inst,
                    &fallback,
                    IntentResolution::system_text(),
                    "adapter failed; system.text fallback",
                    now,
                    &mut out,
                );
            }
            let resumes = Self::take_resumes(inst, now);
            for envelope in resumes {
                out.push(GatewayOutput::Audit(format!(
                    "resume: re-dispatching {} after safety presentation",
                    envelope.message_id
                )));
                out.extend(self.dispatch(id, envelope, now));
            }
        }
        out
    }

    /// 安全演出結束後恢復 `resumePolicy=resume-previous` 的演出（只在沒有其他進行中的安全 command 時）。
    fn take_resumes(inst: &mut InstanceState, now: Timestamp) -> Vec<IntentEnvelope> {
        if inst.resume_stack.is_empty() {
            return Vec::new();
        }
        let safety_live = inst
            .pending
            .iter()
            .any(|p| !p.handoff && !p.status.is_terminal() && p.envelope.intent.is_safety());
        if safety_live {
            return Vec::new();
        }
        let mut resumes = Vec::new();
        while let Some(mut envelope) = inst.resume_stack.pop() {
            if envelope.expires_at <= now {
                continue;
            }
            inst.resume_seq += 1;
            envelope.message_id = format!("{}/resume{}", envelope.message_id, inst.resume_seq);
            envelope.timestamp = now;
            resumes.push(envelope);
        }
        resumes
    }

    /// 處理 adapter 事件：世代／instance／protocol 檢查後交給正規化器（佇列 64）。
    pub fn on_event(
        &mut self,
        id: &InstanceId,
        event: CharacterInputEvent,
        now: Timestamp,
    ) -> InputDecision {
        let Some(inst) = self.instances.get_mut(id) else {
            return InputDecision::Dropped(InputDropReason::UnknownInstance);
        };
        inst.last_seen = Some(now);
        if event.generation != inst.generation {
            return InputDecision::Dropped(InputDropReason::StaleGeneration);
        }
        if event.character_instance_id != inst.id.0 {
            return InputDecision::Dropped(InputDropReason::InvalidPayload {
                field: "characterInstanceId".into(),
            });
        }
        match parse_protocol_version(&event.protocol_version) {
            Some((major, _)) if major == PROTOCOL_MAJOR => {}
            _ => return InputDecision::Dropped(InputDropReason::ProtocolVersion),
        }
        inst.input.push(event, now.timestamp_millis())
    }

    /// 取出已正規化的輸入事件（Runtime 轉成 receptor observation）。
    pub fn drain_input(&mut self, id: &InstanceId) -> Vec<CharacterInputEvent> {
        self.instances
            .get_mut(id)
            .map(|i| i.input.drain())
            .unwrap_or_default()
    }

    /// 取消（冪等）：重複 cancel 回同一結果；已終結／未知 → `cancelled{alreadyTerminal:true}`，不報錯。
    pub fn cancel(
        &mut self,
        id: &InstanceId,
        message_id: &str,
        reason: &str,
        now: Timestamp,
    ) -> Vec<GatewayOutput> {
        let mut out = Vec::new();
        let Some(inst) = self.instances.get_mut(id) else {
            out.push(GatewayOutput::Audit(format!(
                "cancel: unknown instance {id}"
            )));
            return out;
        };
        if let Some(prev) = inst.cancel_results.get(message_id) {
            out.push(GatewayOutput::Receipt(prev.clone()));
            return out;
        }
        if let Some(idx) = inst
            .pending
            .iter()
            .position(|p| p.envelope.message_id == message_id && !p.status.is_terminal())
        {
            Self::cancel_pending(inst, idx, reason, now, &mut out);
            return out;
        }
        let mut receipt = inst.receipt(message_id, ReceiptStatus::Cancelled, now);
        receipt.already_terminal = true;
        if let Some(t) = inst.terminal.get(message_id) {
            receipt.resolution = t.resolution;
            receipt = receipt.with_detail(format!("already {:?}", t.status).to_lowercase());
        } else {
            receipt = receipt.with_detail("unknown messageId");
        }
        if inst.dedupe_set.contains(message_id) {
            inst.cancel_results
                .insert(message_id.to_string(), receipt.clone());
        }
        out.push(GatewayOutput::Receipt(receipt));
        out
    }

    /// 記錄 adapter 心跳／任何訊息到達。
    pub fn heartbeat(&mut self, id: &InstanceId, now: Timestamp) {
        if let Some(inst) = self.instances.get_mut(id) {
            inst.last_seen = Some(now);
        }
    }

    /// adapter 回報生命週期狀態（非法轉移記 audit、不套用）。
    pub fn on_lifecycle(
        &mut self,
        id: &InstanceId,
        state: AdapterLifecycleState,
        now: Timestamp,
    ) -> Vec<GatewayOutput> {
        let mut out = Vec::new();
        let Some(inst) = self.instances.get_mut(id) else {
            out.push(GatewayOutput::Audit(format!(
                "lifecycle: unknown instance {id}"
            )));
            return out;
        };
        inst.last_seen = Some(now);
        if inst.lifecycle == state {
            return out;
        }
        if !inst.lifecycle.can_transition_to(state) {
            out.push(GatewayOutput::Audit(format!(
                "lifecycle: illegal {:?} -> {state:?} on {id}; ignored",
                inst.lifecycle
            )));
            return out;
        }
        inst.lifecycle = state;
        if matches!(state, AdapterLifecycleState::Crashed) {
            out.extend(self.on_disconnect(id, DisconnectReason::Crash, now));
        } else if matches!(state, AdapterLifecycleState::Disposed) {
            out.extend(self.on_disconnect(id, DisconnectReason::Goodbye, now));
        }
        out
    }

    /// 處理任何 adapter → runtime 訊息（含速率限制與方向檢查）。
    pub fn on_message(
        &mut self,
        id: &InstanceId,
        message: WireMessage,
        now: Timestamp,
    ) -> Vec<GatewayOutput> {
        let mut out = Vec::new();
        if !self.instances.contains_key(id) {
            out.push(GatewayOutput::Audit(format!(
                "message: unknown instance {id}"
            )));
            return out;
        }
        let within_rate = self.allow_message(id, now);
        let Some(inst) = self.instances.get_mut(id) else {
            return out;
        };
        if !within_rate {
            out.push(GatewayOutput::Audit(format!(
                "message: {} from {id} rate-limited; dropped",
                message.kind()
            )));
            out.push(GatewayOutput::Send {
                instance: id.clone(),
                message: WireMessage::error("rate-limited", "too many messages; dropped"),
            });
            return out;
        }
        inst.last_seen = Some(now);
        if !message.is_adapter_to_runtime() {
            out.push(GatewayOutput::Audit(format!(
                "message: {} is not an adapter→runtime message; dropped",
                message.kind()
            )));
            out.push(GatewayOutput::Send {
                instance: id.clone(),
                message: WireMessage::error(
                    "wrong-direction",
                    "message type not accepted from adapter",
                ),
            });
            return out;
        }
        match message {
            WireMessage::Negotiate(offer) => match self.on_negotiate(id, offer, now) {
                Ok((_, outs)) => out.extend(outs),
                Err(err) => {
                    out.push(GatewayOutput::Audit(format!(
                        "negotiate rejected on {id}: {err}"
                    )));
                    out.push(GatewayOutput::Send {
                        instance: id.clone(),
                        message: WireMessage::error(err.code(), err.to_string()),
                    });
                }
            },
            WireMessage::Receipt { receipt } => out.extend(self.on_receipt(id, receipt, now)),
            WireMessage::Event { event } => {
                let event_id = event.event_id.clone();
                match self.on_event(id, event, now) {
                    InputDecision::Dropped(reason) => out.push(GatewayOutput::Audit(format!(
                        "event {event_id} dropped: {}",
                        serde_json::to_string(&reason).unwrap_or_default()
                    ))),
                    InputDecision::Throttled | InputDecision::Merged | InputDecision::Queued => {}
                }
            }
            WireMessage::Lifecycle { state, .. } => out.extend(self.on_lifecycle(id, state, now)),
            WireMessage::Heartbeat { .. } => {}
            WireMessage::Error { code, message } => out.push(GatewayOutput::Audit(format!(
                "adapter error on {id}: {code}: {}",
                crate::truncate_for_echo(&message)
            ))),
            WireMessage::Goodbye { .. } => {
                out.extend(self.on_disconnect(id, DisconnectReason::Goodbye, now))
            }
            other => out.push(GatewayOutput::Audit(format!(
                "message: unexpected {} from adapter; dropped",
                other.kind()
            ))),
        }
        out
    }

    /// 斷線／crash／goodbye：pending 全部 `uncertain`、`generation += 1`、需重新 hello。
    pub fn on_disconnect(
        &mut self,
        id: &InstanceId,
        reason: DisconnectReason,
        now: Timestamp,
    ) -> Vec<GatewayOutput> {
        let mut out = Vec::new();
        let ring = self.config.dedupe_ring;
        let Some(inst) = self.instances.get_mut(id) else {
            return out;
        };
        let was_connected = inst.connected;
        let orphaned_safety = Self::mark_all_uncertain(inst, reason.as_str(), now, &mut out);
        Self::resend_safety_as_system_text(
            inst,
            orphaned_safety,
            ring,
            "adapter gone; system.text fallback",
            now,
            &mut out,
        );
        inst.generation += 1;
        inst.connected = false;
        inst.negotiated = None;
        inst.resume_stack.clear();
        inst.lifecycle = match reason {
            DisconnectReason::Crash => AdapterLifecycleState::Crashed,
            DisconnectReason::Revoked => AdapterLifecycleState::Disposed,
            _ => {
                if inst.lifecycle == AdapterLifecycleState::Disposed {
                    AdapterLifecycleState::Disposed
                } else {
                    AdapterLifecycleState::Reconnecting
                }
            }
        };
        let generation = inst.generation;
        for routes in self.safety_routes.values_mut() {
            routes.retain(|_, v| v != id);
        }
        out.push(GatewayOutput::Audit(format!(
            "instance {id} disconnected ({}); was_connected={was_connected}; generation now {generation}",
            reason.as_str()
        )));
        out
    }

    /// 定期掃描：heartbeat 逾時 → 斷線；過期；`acknowledged` 逾時 → `uncertain`；`started` watchdog。
    pub fn sweep(&mut self, now: Timestamp) -> Vec<GatewayOutput> {
        let mut out = Vec::new();
        let timeout = chrono::Duration::milliseconds(self.config.disconnect_after_ms);
        let stale: Vec<InstanceId> = self
            .instances
            .values()
            .filter(|i| i.connected && i.last_seen.is_some_and(|seen| seen + timeout < now))
            .map(|i| i.id.clone())
            .collect();
        for id in stale {
            out.extend(self.on_disconnect(&id, DisconnectReason::HeartbeatTimeout, now));
        }
        let watchdog = chrono::Duration::milliseconds(self.config.started_watchdog_ms);
        let ids: Vec<InstanceId> = self.instances.keys().cloned().collect();
        for id in ids {
            let Some(inst) = self.instances.get_mut(&id) else {
                continue;
            };
            let mut idx = 0;
            while idx < inst.pending.len() {
                let p = &inst.pending[idx];
                let duration_ms = p.envelope.duration_hint.map(|d| d.ms).unwrap_or(0);
                let next: Option<(ReceiptStatus, &str)> = match p.status {
                    ReceiptStatus::Accepted | ReceiptStatus::Scheduled
                        if p.envelope.expires_at <= now =>
                    {
                        Some((ReceiptStatus::Expired, "expired before start"))
                    }
                    ReceiptStatus::Acknowledged
                        if p.acked_at
                            .is_some_and(|t| ack_uncertain_deadline(t, duration_ms) <= now) =>
                    {
                        Some((ReceiptStatus::Uncertain, "acknowledged without completion"))
                    }
                    ReceiptStatus::Started => {
                        let grace = chrono::Duration::milliseconds(
                            i64::try_from(duration_ms).unwrap_or(i64::MAX / 4),
                        )
                        .max(watchdog);
                        if p.envelope.expires_at + grace <= now {
                            Some((ReceiptStatus::Uncertain, "watchdog: no completion"))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let Some((status, detail)) = next else {
                    idx += 1;
                    continue;
                };
                let message_id = p.envelope.message_id.clone();
                let resolution = p.resolution.resolution;
                let handoff = p.handoff;
                let receipt = inst
                    .receipt(&message_id, status, now)
                    .with_resolution(resolution)
                    .with_detail(detail);
                inst.finish(idx, receipt.clone());
                if status == ReceiptStatus::Expired && !handoff && inst.connected {
                    out.push(GatewayOutput::Send {
                        instance: id.clone(),
                        message: WireMessage::Cancel {
                            message_id,
                            reason: Some("expired".into()),
                        },
                    });
                }
                out.push(GatewayOutput::Receipt(receipt));
            }
        }
        out
    }
}
