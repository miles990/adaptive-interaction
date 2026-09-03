//! Character Presentation Protocol（CPP v1.0）在 Runtime 內的接線。
//!
//! `interaction_character::Gateway` 是純狀態機（沒有 I/O、時間由呼叫端注入）；
//! 這裡的 [`CharacterHub`] 把它接到 Runtime 的真相來源與傳輸：
//!
//! - **instance 登記**：桌面視窗經 `POST /v1/character/hello`（可信 host、human token），
//!   外部 adapter 經 `GET /v1/character/ws?token=<adapter token>`（每連線 outbound 有界 32、
//!   heartbeat 15 s、45 s 無訊息視為斷線、斷線 → pending 一律 `uncertain`＋generation+1）。
//! - **truth projection**（README §11）：runtime 事件在 emit 點呼叫 `character_project_*`，
//!   由 Runtime 建構 [`IntentEnvelope`]——`truthState` 只在這裡決定，adapter 永遠拿不到
//!   `verified` 的判定權。`character.*` 事件本身永不投影（沒有自我遞迴）。
//! - **Gateway 輸出**：`Send` → 桌面 instance ＝ `character.intent` 事件（payload
//!   `{envelope, targets}`）、外部 instance ＝ WebSocket；`SystemText` → `character.system-text`
//!   （零呈現能力時安全訊息不得遺失）；`Receipt` → `character.receipt`＋audit，且對應到
//!   AI presentation 命令（`correlationId = actionId`）時誠實推進 presentation receipt：
//!   `completed` → Completed（AcknowledgedOnly）、`unsupported`／`failed` → Failed、
//!   `cancelled` → Cancelled、`expired`／`uncertain` → Uncertain——永遠不是 verified。
//! - **input event → receptor observation**：正規化後才進 `ingest`（file-drop 只帶 metadata、
//!   hover 30 s 節流、拖曳合併），仍經 Runtime policy／consent。
//! - **adapter token**：32 bytes 隨機、只存 sha256、撤銷即斷線且重啟後仍撤銷。

use crate::presentation::{display_name_zh, ActiveCharacter, PresentationKind};
use crate::runtime::Runtime;
use chrono::Utc;
use interaction_character::{
    default_system_text, intent_variant_aliases, parse_wire, validate_manifest, AdapterKind,
    CharacterInputEvent, CharacterIntent, CharacterManifest, CharacterRole, CommandReceipt,
    DisconnectReason, DurationHint, Entrypoint, Gateway, GatewayConfig, GatewayOutput,
    InputDecision, InputEventKind, InstanceId, IntentEnvelope, InterruptPolicy, Negotiate,
    PresentationHints, ReceiptStatus, TruthState, ValidationLimits, WireMessage, PROTOCOL_VERSION,
};
use interaction_core::{
    ActionReceipt, BoundedAction, CorrelationId, DomainError, DomainResult, EventType, Observation,
    ProviderId, ProviderState, RuntimeEvent, Timestamp, VerificationVerdict,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// 桌面視窗（可信 host）的預設 instance id。
pub const DESKTOP_INSTANCE_ID: &str = "desktop-companion";
/// 桌面角色 provider id（顯示名隨 active character 變動）。
pub const COMPANION_PROVIDER_ID: &str = "provider.companion.desktop";
/// 每個外部連線的 outbound 佇列上限（README §8）。
pub const OUTBOUND_CAP: usize = 32;
/// 外部 adapter heartbeat 間隔。
pub const HEARTBEAT_INTERVAL_MS: u64 = 15_000;
/// 外部 adapter 無訊息視為斷線的時限。
pub const IDLE_TIMEOUT_MS: u64 = 45_000;
/// runtime 投影 envelope 的最短存活時間（`expiresAt = now + max(durationHint, 30 s)`）。
pub const DEFAULT_INTENT_TTL_MS: i64 = 30_000;
/// `hover-entered` → `companion.pointer{pointer-approached}` 每 instance 最多 30 s 一次。
pub const POINTER_THROTTLE_MS: i64 = 30_000;
/// `dragged`（gateway 已合併為 ≤ 10/s）→ observation 每 instance 最多 1 s 一次；
/// `drag-started`／`dropped` 不節流。
pub const DRAG_OBSERVATION_THROTTLE_MS: i64 = 1_000;
/// `receptor.observation` → `notice(listening)` 每受器最多 2 s 一次（高頻受器不灌爆佇列）。
pub const OBSERVATION_PROJECTION_THROTTLE_MS: i64 = 2_000;
/// runtime 非安全投影的 requested priority（安全 intent 由 floor 決定）。
pub const RUNTIME_INTENT_PRIORITY: u8 = 40;
/// AI（`companion.state.present`／`animation.play`）請求的 priority（上限 50）。
pub const AI_INTENT_PRIORITY: u8 = 30;
const AI_COMMANDS_CAP: usize = 256;
const THROTTLE_MAP_CAP: usize = 512;
const MAX_HINT_MESSAGE_CHARS: usize = 200;

/// 可注入的時鐘（測試用；預設 `Utc::now`）。
pub type NowFn = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// 外部 adapter 登記（persisted 到 storage v8 `character_adapters`；token 只存 sha256）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAdapterRecord {
    pub adapter_id: String,
    pub display_name: String,
    pub manifest: CharacterManifest,
    pub token_sha256: String,
    pub created_at: Timestamp,
    #[serde(default)]
    pub revoked: bool,
}

/// instance 來源：內建純資料角色／使用者匯入／外部 adapter。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstanceOrigin {
    Builtin,
    Imported,
    External,
}

impl InstanceOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            InstanceOrigin::Builtin => "builtin",
            InstanceOrigin::Imported => "imported",
            InstanceOrigin::External => "external",
        }
    }
}

/// Runtime 端對 instance 的補充資訊（gateway 只知道 manifest／role／generation）。
#[derive(Debug, Clone)]
pub struct InstanceMeta {
    pub instance_id: String,
    pub manifest: CharacterManifest,
    pub role: CharacterRole,
    pub origin: InstanceOrigin,
    pub adapter_id: Option<String>,
    /// 曾收到 adapter 的 `completed` 回執（真的演過一次）。
    pub tested: bool,
}

struct Connection {
    tx: mpsc::Sender<WireMessage>,
    close: CancellationToken,
    conn_id: u64,
}

/// AI presentation 命令（`companion.state.present`／`animation.play`）在 gateway 裡的對應。
struct AiCommand {
    action_id: String,
    command: &'static str,
}

/// messageId → AI 命令；有界（256），最舊者先出。
#[derive(Default)]
struct AiCommands {
    by_message: BTreeMap<String, AiCommand>,
    ring: VecDeque<String>,
}

/// 同一 messageId 的 intent 送達多個 instance 時，只發一則 `character.intent`
/// （envelope 取桌面的，沒有桌面則取第一個；targets 列出全部送達的 instance）。
struct IntentGroup {
    message_id: String,
    desktop: Option<IntentEnvelope>,
    first: Option<IntentEnvelope>,
    targets: Vec<String>,
}

/// 一條外部 WebSocket 連線在 Runtime 端的把手（由 API 層驅動 socket）。
pub struct WsSession {
    pub instance_id: String,
    pub conn_id: u64,
    /// Runtime → adapter 的訊息（有界 32）；第一則一定是 `hello`。
    pub rx: mpsc::Receiver<WireMessage>,
    /// 撤銷／被新連線取代／heartbeat 逾時 → 取消，API 層關閉 socket。
    pub close: CancellationToken,
}

/// 處理完一則 adapter 訊息後 socket 該怎麼辦。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsStep {
    KeepOpen,
    Close,
}

/// README §11 的一筆 truth projection 輸入。
#[derive(Debug, Clone)]
pub struct Projection {
    pub intent: CharacterIntent,
    pub truth_state: TruthState,
    pub correlation_id: Option<String>,
    pub hints: Option<PresentationHints>,
    pub duration_ms: Option<u64>,
    pub interrupt: Option<InterruptPolicy>,
    pub parameters: BTreeMap<String, Value>,
}

impl Projection {
    pub fn new(intent: CharacterIntent, truth_state: TruthState) -> Self {
        Projection {
            intent,
            truth_state,
            correlation_id: None,
            hints: None,
            duration_ms: None,
            interrupt: None,
            parameters: BTreeMap::new(),
        }
    }

    pub fn with_correlation(mut self, correlation: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation.into());
        self
    }

    pub fn with_variant(mut self, variant: impl Into<String>) -> Self {
        self.hints.get_or_insert_with(Default::default).variant = Some(variant.into());
        self
    }

    /// 提示文字（≤ 200 字；安全 intent 的固定語句仍由 host 決定）。
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        let message: String = message.into();
        let clipped: String = message.chars().take(MAX_HINT_MESSAGE_CHARS).collect();
        self.hints.get_or_insert_with(Default::default).message = Some(clipped);
        self
    }

    pub fn with_interrupt(mut self, policy: InterruptPolicy) -> Self {
        self.interrupt = Some(policy);
        self
    }

    pub fn with_duration_ms(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    pub fn with_parameter(mut self, key: &str, value: Value) -> Self {
        self.parameters.insert(key.to_string(), value);
        self
    }

    fn envelope(
        &self,
        message_id: &str,
        instance_id: &str,
        now: Timestamp,
        expires_at: Timestamp,
    ) -> IntentEnvelope {
        let mut envelope = IntentEnvelope::from_runtime(
            message_id,
            instance_id,
            self.correlation_id.clone(),
            self.intent,
            self.truth_state,
            RUNTIME_INTENT_PRIORITY,
            now,
            expires_at,
        );
        envelope.presentation_hints = self.hints.clone();
        envelope.duration_hint = self.duration_ms.map(|ms| DurationHint {
            ms: ms.min(interaction_character::MAX_DURATION_HINT_MS),
            looped: false,
        });
        envelope.parameters = self.parameters.clone();
        if let Some(policy) = self.interrupt {
            envelope.interrupt_policy = policy;
        }
        envelope
    }
}

/// 回執 → presentation receipt 的推進（由 async 端執行；永遠不是 verified）。
#[derive(Debug, Clone)]
pub struct Settlement {
    pub action_id: String,
    pub outcome: &'static str,
    pub detail: Option<String>,
}

/// Runtime 內的 Character Gateway 宿主。
pub struct CharacterHub {
    gateway: Mutex<Gateway>,
    instances: Mutex<BTreeMap<String, InstanceMeta>>,
    connections: Mutex<BTreeMap<String, Connection>>,
    adapters: Mutex<BTreeMap<String, CharacterAdapterRecord>>,
    /// AI presentation 命令：envelope messageId → (actionId, command)。有界（256）。
    ai_commands: Mutex<AiCommands>,
    pointer_last: Mutex<BTreeMap<String, Timestamp>>,
    drag_last: Mutex<BTreeMap<String, Timestamp>>,
    observation_last: Mutex<BTreeMap<String, Timestamp>>,
    conn_seq: AtomicU64,
    clock: Mutex<NowFn>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // poisoned lock：純狀態，繼續使用比整個 Runtime 崩潰誠實。
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 外部 adapter 的 instance id（固定前綴，桌面 hello 不得使用）。
pub fn adapter_instance_id(adapter_id: &str) -> String {
    format!("adapter:{adapter_id}")
}

pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn valid_instance_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '#' | ':'))
}

pub fn adapter_kind_str(kind: AdapterKind) -> &'static str {
    match kind {
        AdapterKind::InProcess => "in-process",
        AdapterKind::Web => "web",
        AdapterKind::ExternalProcess => "external-process",
        AdapterKind::RemoteDevice => "remote-device",
    }
}

/// 呈現表面（角色自己）的 actuator：它們的 receipt 不投影成 `action.*` intent。
pub fn is_presentation_surface_actuator(actuator_id: &str) -> bool {
    actuator_id.starts_with("companion.") || actuator_id == "iphone.character"
}

fn settlement_outcome(status: ReceiptStatus) -> &'static str {
    match status {
        ReceiptStatus::Completed => "completed",
        ReceiptStatus::Unsupported => "unsupported",
        ReceiptStatus::Failed => "failed",
        ReceiptStatus::Cancelled => "interrupted",
        ReceiptStatus::Expired => "expired",
        _ => "uncertain",
    }
}

fn sent_targets(outputs: &[GatewayOutput], message_id: &str) -> Vec<String> {
    outputs
        .iter()
        .filter_map(|o| match o {
            GatewayOutput::Send {
                instance,
                message: WireMessage::Intent { envelope },
            } if envelope.message_id == message_id => Some(instance.0.clone()),
            _ => None,
        })
        .collect()
}

fn throttled(
    map: &Mutex<BTreeMap<String, Timestamp>>,
    key: &str,
    now: Timestamp,
    window_ms: i64,
) -> bool {
    let mut last = lock(map);
    if last
        .get(key)
        .is_some_and(|t| now.signed_duration_since(*t).num_milliseconds() < window_ms)
    {
        return true;
    }
    if last.len() >= THROTTLE_MAP_CAP {
        last.clear();
    }
    last.insert(key.to_string(), now);
    false
}

// ---------------------------------------------------------------------------
// README §11 投影表（純函式，可獨立測試）
// ---------------------------------------------------------------------------

/// `agent.session.state` → intent／truthState。
pub fn session_projection(state: &str) -> Option<(CharacterIntent, TruthState)> {
    use CharacterIntent as I;
    use TruthState as T;
    Some(match state {
        "created" | "queued" => (I::Wait, T::Queued),
        "fetched" => (I::Think, T::Working),
        "working" | "active" => (I::Work, T::Working),
        "waiting-input" | "waiting-for-input" => (I::Ask, T::WaitingInput),
        "waiting-consent" | "waiting-for-consent" => (I::RequestConsent, T::WaitingConsent),
        "claimed-completed" => (I::ClaimCompleted, T::Claimed),
        "verified" => (I::VerifiedSuccess, T::Verified),
        "failed" => (I::Failed, T::Failed),
        "timed-out" => (I::Failed, T::TimedOut),
        "unknown" => (I::Unknown, T::Unknown),
        "cancelled" => (I::Cancelled, T::Cancelled),
        "closed" => (I::Idle, T::None),
        "expired" => (I::Unknown, T::Expired),
        _ => return None,
    })
}

/// `action.*`（非角色 actuator）→ intent／truthState。
pub fn action_projection(event_type: EventType) -> Option<(CharacterIntent, TruthState)> {
    use CharacterIntent as I;
    use TruthState as T;
    Some(match event_type {
        EventType::ActionDispatched => (I::Work, T::Working),
        EventType::ActionAcknowledged => (I::Acknowledge, T::Working),
        EventType::ActionCompleted => (I::ClaimCompleted, T::Claimed),
        EventType::ActionObserved => (I::VerifiedSuccess, T::Verified),
        EventType::ActionUncertain => (I::Unknown, T::Unknown),
        EventType::ActionFailed => (I::Failed, T::Failed),
        EventType::ActionCancelled => (I::Cancelled, T::Cancelled),
        EventType::ActionExpired => (I::Unknown, T::Expired),
        _ => return None,
    })
}

/// `provider.state-changed` → intent＋hint variant。
pub fn provider_projection(state: ProviderState) -> Option<(CharacterIntent, &'static str)> {
    match state {
        ProviderState::Available | ProviderState::Paired => {
            Some((CharacterIntent::Greet, "device-online"))
        }
        ProviderState::Disconnected | ProviderState::Revoked => {
            Some((CharacterIntent::Notice, "device-offline"))
        }
        _ => None,
    }
}

/// AI 的 `companion.state.present` behaviorIntent → intent（＋variant）。
/// `wait-attention`／`look-at-confirmation` 一律經 `ai_safe_substitute`：AI 永遠
/// 不能點播有 floor 的 intent（wait／ask）。
pub fn behavior_intent_projection(behavior: &str) -> (CharacterIntent, Option<&'static str>) {
    use CharacterIntent as I;
    match behavior {
        "rest" => (I::Rest, None),
        "notice" => (I::Notice, None),
        "curious" => (I::Notice, Some("curious")),
        "listen" => (I::Notice, Some("listening")),
        "think" => (I::Think, None),
        "work" => (I::Work, None),
        "wait-attention" => (I::Wait.ai_safe_substitute(), Some("wait-attention")),
        "look-at-confirmation" => (I::Ask.ai_safe_substitute(), Some("look-at-confirmation")),
        "acknowledge-briefly" => (I::Acknowledge, None),
        _ => (I::Notice, None),
    }
}

/// AI 的 `companion.animation.play` 動畫名 → intent（＋variant）。安全 intent 名稱
/// 或其別名一律降級成 AI 可請求的替代 intent（名稱保留為 variant 提示）。
pub fn animation_projection(name: &str) -> (CharacterIntent, Option<String>) {
    if let Some(intent) = CharacterIntent::parse(name) {
        if intent.is_safety() {
            return (intent.ai_safe_substitute(), Some(name.to_string()));
        }
        return (intent, None);
    }
    for intent in CharacterIntent::ALL {
        if intent_variant_aliases(intent).contains(&name) {
            return (intent.ai_safe_substitute(), Some(name.to_string()));
        }
    }
    (CharacterIntent::Notice, Some(name.to_string()))
}

/// manifest 宣告且 `supported` 的輸入能力 id（連接頁「可以接收」）。
/// 只列 id，不做任何能力推論：manifest 沒宣告就不會出現。
pub fn supported_input_capabilities(manifest: &CharacterManifest) -> Vec<String> {
    manifest
        .input_capabilities
        .iter()
        .filter(|(_, decl)| decl.supported)
        .map(|(id, _)| id.clone())
        .collect()
}

/// manifest 的（可執行, 需要網路）旗標——README §9 的分級依據：外部程序或宣告
/// `executable` 即可執行；遠端裝置或宣告 `network` 即需要網路。
pub fn manifest_security_flags(manifest: &CharacterManifest) -> (bool, bool) {
    let executable = manifest.entrypoint.is_executable()
        || manifest.security_requirements.executable
        || matches!(manifest.adapter_kind, AdapterKind::ExternalProcess);
    let network = manifest.security_requirements.network
        || matches!(manifest.adapter_kind, AdapterKind::RemoteDevice);
    (executable, network)
}

// ---------------------------------------------------------------------------
// CharacterHub
// ---------------------------------------------------------------------------

impl Default for CharacterHub {
    fn default() -> Self {
        Self::build(Arc::new(Utc::now))
    }
}

impl CharacterHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn with_clock(now: NowFn) -> Arc<Self> {
        Arc::new(Self::build(now))
    }

    fn build(now: NowFn) -> Self {
        CharacterHub {
            gateway: Mutex::new(Gateway::new(GatewayConfig::default())),
            instances: Mutex::new(BTreeMap::new()),
            connections: Mutex::new(BTreeMap::new()),
            adapters: Mutex::new(BTreeMap::new()),
            ai_commands: Mutex::new(AiCommands::default()),
            pointer_last: Mutex::new(BTreeMap::new()),
            drag_last: Mutex::new(BTreeMap::new()),
            observation_last: Mutex::new(BTreeMap::new()),
            conn_seq: AtomicU64::new(0),
            clock: Mutex::new(now),
        }
    }

    /// 替換時鐘（測試模擬 heartbeat 逾時／節流窗）。
    pub fn set_clock(&self, now: NowFn) {
        *lock(&self.clock) = now;
    }

    pub fn now(&self) -> Timestamp {
        let clock = lock(&self.clock).clone();
        clock()
    }

    pub fn gateway(&self) -> MutexGuard<'_, Gateway> {
        lock(&self.gateway)
    }

    pub fn instance_meta(&self, instance_id: &str) -> Option<InstanceMeta> {
        lock(&self.instances).get(instance_id).cloned()
    }

    pub fn instance_ids(&self) -> Vec<String> {
        lock(&self.instances).keys().cloned().collect()
    }

    /// 已連線且協商完成的 instance（投影的目標）。
    pub fn connected_instance_ids(&self) -> Vec<String> {
        lock(&self.gateway)
            .instances()
            .into_iter()
            .filter(|view| view.connected && view.negotiated)
            .map(|view| view.id.0)
            .collect()
    }

    /// 目前接上的桌面角色（桌面 instance 已協商且連線中）。
    pub fn active_character(&self) -> Option<InstanceMeta> {
        let connected = lock(&self.gateway)
            .instance(&InstanceId(DESKTOP_INSTANCE_ID.to_string()))
            .is_some_and(|view| view.connected && view.negotiated);
        if !connected {
            return None;
        }
        self.instance_meta(DESKTOP_INSTANCE_ID)
    }

    /// `GET /v1/character/instances` 的一筆。除了連線狀態，也帶 manifest 的
    /// 作者／版本／可接收的輸入能力（README §9：可執行 adapter 必須顯示來源、
    /// 作者、版本、能力、網路需求；連接頁據此顯示「可以接收／作者／版本」）。
    pub fn instance_entry(&self, instance_id: &str) -> Option<Value> {
        let meta = self.instance_meta(instance_id)?;
        let view = lock(&self.gateway).instance(&InstanceId(instance_id.to_string()))?;
        let manifest = &meta.manifest;
        let (executable, network) = manifest_security_flags(manifest);
        Some(json!({
            "instanceId": instance_id,
            "characterId": manifest.character_id,
            "displayName": manifest.display_name,
            "author": manifest.author,
            "version": manifest.version,
            "inputCapabilities": supported_input_capabilities(manifest),
            "role": meta.role,
            "generation": view.generation,
            "lifecycle": view.lifecycle,
            "connected": view.connected,
            "negotiated": view.negotiated,
            "pending": view.pending,
            "adapterKind": manifest.adapter_kind,
            "origin": meta.origin,
            "executable": executable,
            "network": network,
            "tested": meta.tested,
            "adapterId": meta.adapter_id,
        }))
    }

    pub fn instances_view(&self) -> Vec<Value> {
        self.instance_ids()
            .iter()
            .filter_map(|id| self.instance_entry(id))
            .collect()
    }

    pub fn adapter_record(&self, adapter_id: &str) -> Option<CharacterAdapterRecord> {
        lock(&self.adapters).get(adapter_id).cloned()
    }

    /// `GET /v1/character/adapters` 的清單（永遠不含 token 或其 hash）。
    /// `displayName` 是註冊時給的名字（字串）；manifest 自己的多語名稱放在
    /// `characterDisplayName`。作者／版本／可接收的輸入能力／可執行／需要網路
    /// 來自 manifest，所以從未連線過的 adapter 也能誠實顯示，不必寫「未回報」。
    pub fn adapters_view(&self) -> Vec<Value> {
        let connections = lock(&self.connections);
        lock(&self.adapters)
            .values()
            .map(|record| {
                let manifest = &record.manifest;
                let (executable, network) = manifest_security_flags(manifest);
                json!({
                    "adapterId": record.adapter_id,
                    "displayName": record.display_name,
                    "characterId": manifest.character_id,
                    "characterDisplayName": manifest.display_name,
                    "author": manifest.author,
                    "version": manifest.version,
                    "inputCapabilities": supported_input_capabilities(manifest),
                    "adapterKind": manifest.adapter_kind,
                    "executable": executable,
                    "network": network,
                    "createdAt": record.created_at,
                    "revoked": record.revoked,
                    "connected": connections.contains_key(&adapter_instance_id(&record.adapter_id)),
                })
            })
            .collect()
    }

    /// token → adapterId（sha256 常數時間比對；已撤銷者視同不存在）。
    pub fn adapter_for_token(&self, token: &str) -> Option<String> {
        if token.trim().is_empty() {
            return None;
        }
        let digest = sha256_hex(token);
        lock(&self.adapters)
            .values()
            .filter(|record| !record.revoked)
            .find(|record| constant_time_eq(record.token_sha256.as_bytes(), digest.as_bytes()))
            .map(|record| record.adapter_id.clone())
    }

    pub fn load_adapters(&self, records: Vec<CharacterAdapterRecord>) {
        let mut adapters = lock(&self.adapters);
        for record in records {
            adapters.insert(record.adapter_id.clone(), record);
        }
    }

    fn remember_ai_command(&self, message_id: &str, action_id: &str, command: &'static str) {
        let mut guard = lock(&self.ai_commands);
        if guard.by_message.len() >= AI_COMMANDS_CAP {
            if let Some(old) = guard.ring.pop_front() {
                guard.by_message.remove(&old);
            }
        }
        guard.by_message.insert(
            message_id.to_string(),
            AiCommand {
                action_id: action_id.to_string(),
                command,
            },
        );
        guard.ring.push_back(message_id.to_string());
    }

    fn take_ai_command(&self, message_id: &str) -> Option<(String, &'static str)> {
        let mut guard = lock(&self.ai_commands);
        let taken = guard.by_message.remove(message_id)?;
        guard.ring.retain(|id| id != message_id);
        Some((taken.action_id, taken.command))
    }

    fn mark_tested(&self, instance_id: &str) {
        if let Some(meta) = lock(&self.instances).get_mut(instance_id) {
            meta.tested = true;
        }
    }

    fn connection_id(&self, instance_id: &str) -> Option<u64> {
        lock(&self.connections)
            .get(instance_id)
            .map(|conn| conn.conn_id)
    }

    /// 送到外部連線的有界 outbound：滿了先丟非安全 intent／heartbeat／error，
    /// 安全 intent 與握手／cancel／goodbye 永不丟（改為非同步等待空位，連線關閉即結束）。
    fn send_external(&self, instance_id: &str, message: WireMessage) {
        let connections = lock(&self.connections);
        let Some(conn) = connections.get(instance_id) else {
            tracing::debug!(
                instance = instance_id,
                kind = message.kind(),
                "no live connection; dropped"
            );
            return;
        };
        match conn.tx.try_send(message) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(message)) => {
                let droppable = match &message {
                    WireMessage::Intent { envelope } => !envelope.intent.is_safety(),
                    WireMessage::Heartbeat { .. } | WireMessage::Error { .. } => true,
                    _ => false,
                };
                if droppable {
                    tracing::warn!(
                        instance = instance_id,
                        kind = message.kind(),
                        "character outbound full; non-safety message dropped (gateway will expire it honestly)"
                    );
                    return;
                }
                match tokio::runtime::Handle::try_current() {
                    Ok(handle) => {
                        let tx = conn.tx.clone();
                        handle.spawn(async move {
                            let _ = tx.send(message).await;
                        });
                    }
                    Err(_) => tracing::warn!(
                        instance = instance_id,
                        "character outbound full and no async runtime; safety message not delivered"
                    ),
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!(
                    instance = instance_id,
                    "character connection closed; dropped"
                );
            }
        }
    }

    /// `status.characterProtocol`。
    pub fn status(&self) -> Value {
        let active = self.active_character().map(|meta| {
            json!({
                "characterId": meta.manifest.character_id,
                "displayName": meta.manifest.display_name,
            })
        });
        json!({
            "version": PROTOCOL_VERSION,
            "instances": lock(&self.instances).len(),
            "activeCharacter": active,
        })
    }
}

// ---------------------------------------------------------------------------
// Runtime 介面
// ---------------------------------------------------------------------------

/// `POST /v1/character/hello` 的輸入。
#[derive(Debug, Clone)]
pub struct CharacterHelloInput {
    pub instance_id: Option<String>,
    pub role: Option<CharacterRole>,
    pub manifest: CharacterManifest,
    pub negotiate: Negotiate,
    pub visible: bool,
    pub pack_id: Option<String>,
    pub behavior_state: Option<Value>,
}

impl Runtime {
    /// 開機：載入外部 adapter 登記（撤銷旗標跟著 body 持久化）。
    pub(crate) fn character_load_adapters(&self) {
        let Ok(bodies) = self.store.all_character_adapters() else {
            return;
        };
        let records = bodies
            .iter()
            .filter_map(|body| serde_json::from_str::<CharacterAdapterRecord>(body).ok())
            .collect();
        self.character.load_adapters(records);
    }

    /// 桌面視窗（可信 host）登記角色並協商。同 instanceId 重送＝重新協商
    /// （generation+1、進行中的 command 一律 `uncertain`）。
    pub async fn character_hello(&self, input: CharacterHelloInput) -> DomainResult<Value> {
        let hub = self.character.clone();
        let instance_id = input
            .instance_id
            .unwrap_or_else(|| DESKTOP_INSTANCE_ID.to_string());
        if !valid_instance_id(&instance_id) {
            return Err(DomainError::Validation(
                "instanceId must be 1..=64 chars of [A-Za-z0-9._#:-]".into(),
            ));
        }
        if instance_id.starts_with("adapter:") {
            return Err(DomainError::Validation(
                "instanceId prefix 'adapter:' is reserved for external adapters".into(),
            ));
        }
        let bytes = serde_json::to_vec(&input.manifest)
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        validate_manifest(bytes, &input.manifest, &ValidationLimits::default())
            .map_err(|e| DomainError::Validation(format!("manifest rejected: {e}")))?;
        if matches!(
            input.manifest.adapter_kind,
            AdapterKind::ExternalProcess | AdapterKind::RemoteDevice
        ) {
            return Err(DomainError::Validation(
                "external-process / remote-device characters connect over /v1/character/ws with an adapter token, not /v1/character/hello".into(),
            ));
        }
        let role = input.role.unwrap_or_default();
        let origin = match input.manifest.entrypoint {
            Entrypoint::Builtin { .. } => InstanceOrigin::Builtin,
            _ => InstanceOrigin::Imported,
        };
        let now = hub.now();
        let iid = InstanceId(instance_id.clone());
        let manifest_value = serde_json::to_value(&input.manifest).unwrap_or(Value::Null);
        let (negotiated, outputs, generation) = {
            let mut gw = hub.gateway();
            let mut outputs = Vec::new();
            let same = gw
                .manifest(&iid)
                .is_some_and(|m| serde_json::to_value(m).unwrap_or(Value::Null) == manifest_value)
                && hub
                    .instance_meta(&instance_id)
                    .is_some_and(|meta| meta.role == role);
            if !same {
                if gw.instance(&iid).is_some() {
                    outputs.extend(gw.remove_instance(&iid, now));
                }
                gw.register_instance_with_id(iid.clone(), input.manifest.clone(), role);
            }
            let (negotiated, outs) = gw
                .on_negotiate(&iid, input.negotiate, now)
                .map_err(|e| DomainError::Validation(format!("negotiate rejected: {e}")))?;
            outputs.extend(outs);
            let generation = gw.generation(&iid).unwrap_or(negotiated.generation);
            (negotiated, outputs, generation)
        };
        let tested = hub
            .instance_meta(&instance_id)
            .map(|meta| meta.tested)
            .unwrap_or(false);
        lock(&hub.instances).insert(
            instance_id.clone(),
            InstanceMeta {
                instance_id: instance_id.clone(),
                manifest: input.manifest.clone(),
                role,
                origin,
                adapter_id: None,
                tested,
            },
        );
        if instance_id == DESKTOP_INSTANCE_ID {
            let expression_variants = negotiated
                .capabilities
                .get("visual.expression")
                .map(|decl| decl.variants.clone())
                .unwrap_or_default();
            self.presentation.set_character(ActiveCharacter {
                character_id: input.manifest.character_id.clone(),
                display_name: input.manifest.display_name.clone(),
                version: input.manifest.version.clone(),
                adapter_kind: input.manifest.adapter_kind,
                origin: origin.as_str().to_string(),
                capabilities: negotiated.capabilities.keys().cloned().collect(),
                expression_variants,
                generation,
            });
            self.mobile
                .set_character_title(display_name_zh(&input.manifest.display_name));
            let pack_id = input
                .pack_id
                .or_else(|| Some(input.manifest.character_id.clone()));
            self.presentation_hello_with_behavior(input.visible, pack_id, input.behavior_state)
                .await;
        }
        self.character_apply(outputs).await;
        self.publish_character_instance(&instance_id);
        let _ = self.store.audit(
            "character.hello",
            "user",
            &json!({
                "instanceId": instance_id,
                "characterId": input.manifest.character_id,
                "generation": generation,
                "origin": origin,
            }),
        );
        Ok(json!({
            "instanceId": instance_id,
            "generation": generation,
            "negotiated": negotiated,
        }))
    }

    /// `POST /v1/character/receipts`：adapter 回執進 gateway（舊世代／非法轉移一律
    /// 丟棄並 audit）；對應 AI presentation 命令時推進 presentation receipt。
    pub async fn character_receipt(
        &self,
        instance_id: &str,
        receipt: CommandReceipt,
    ) -> DomainResult<Value> {
        let hub = self.character.clone();
        let now = hub.now();
        let iid = InstanceId(instance_id.to_string());
        let (current_generation, outputs, status_after) = {
            let mut gw = hub.gateway();
            let generation = gw.generation(&iid).ok_or_else(|| {
                DomainError::NotFound(format!("character instance {instance_id}"))
            })?;
            let outputs = gw.on_receipt(&iid, receipt.clone(), now);
            let status_after = gw.command_status(&iid, &receipt.message_id);
            (generation, outputs, status_after)
        };
        let emitted = outputs.iter().any(|o| {
            matches!(o, GatewayOutput::Receipt(r) if r.message_id == receipt.message_id && r.status == receipt.status)
        });
        let stale = receipt.generation != current_generation;
        let accepted = !stale && (emitted || status_after == Some(receipt.status));
        self.character_apply(outputs).await;
        let status = if stale {
            Some("stale-generation".to_string())
        } else {
            status_after
                .and_then(|s| serde_json::to_value(s).ok())
                .and_then(|v| v.as_str().map(String::from))
                .or_else(|| Some("unknown-message".to_string()))
        };
        Ok(json!({ "accepted": accepted, "status": status }))
    }

    /// `POST /v1/character/events`：input event 正規化 → receptor observation。
    pub async fn character_event(
        &self,
        instance_id: &str,
        event: CharacterInputEvent,
    ) -> DomainResult<Value> {
        let hub = self.character.clone();
        let now = hub.now();
        let iid = InstanceId(instance_id.to_string());
        let meta = hub
            .instance_meta(instance_id)
            .ok_or_else(|| DomainError::NotFound(format!("character instance {instance_id}")))?;
        let (decision, drained) = {
            let mut gw = hub.gateway();
            let decision = gw.on_event(&iid, event, now);
            let drained = gw.drain_input(&iid);
            (decision, drained)
        };
        let mut final_decision = match &decision {
            InputDecision::Queued => "queued",
            InputDecision::Merged => "merged",
            InputDecision::Throttled => "throttled",
            InputDecision::Dropped(_) => "dropped",
        };
        let mut reason: Option<String> = match &decision {
            InputDecision::Dropped(why) => serde_json::to_value(why)
                .ok()
                .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(String::from)),
            _ => None,
        };
        for normalized in drained {
            match self
                .character_input_to_observation(instance_id, meta.origin, normalized)
                .await
            {
                Ok(InputOutcome::Observed(_)) | Ok(InputOutcome::AuditOnly) => {}
                Ok(InputOutcome::Throttled) => {
                    final_decision = "throttled";
                }
                Err(err) => {
                    // 桌面角色隱藏／斷線時視窗內受器是關的：誠實回 dropped。
                    final_decision = "dropped";
                    reason = Some(err.code().to_string());
                    let _ = self.store.audit(
                        "character.event-refused",
                        "runtime",
                        &json!({"instanceId": instance_id, "reason": err.to_string()}),
                    );
                }
            }
        }
        Ok(json!({ "decision": final_decision, "reason": reason }))
    }

    pub fn character_instances(&self) -> Value {
        json!({ "instances": self.character.instances_view() })
    }

    /// 目前桌面角色的 manifest（尚未 hello → None）。
    pub fn character_manifest(&self) -> Option<CharacterManifest> {
        self.character
            .instance_meta(DESKTOP_INSTANCE_ID)
            .map(|meta| meta.manifest)
    }

    pub fn character_adapters(&self) -> Value {
        json!({ "adapters": self.character.adapters_view() })
    }

    /// 註冊外部 adapter：回傳 adapterId＋**只此一次**的 token（只存 sha256）。
    pub async fn character_adapter_add(
        &self,
        display_name: &str,
        manifest: CharacterManifest,
    ) -> DomainResult<Value> {
        let display_name = display_name.trim();
        if display_name.is_empty()
            || display_name.chars().count() > 48
            || display_name.chars().any(char::is_control)
        {
            return Err(DomainError::Validation(
                "displayName must be 1..=48 printable chars".into(),
            ));
        }
        let bytes = serde_json::to_vec(&manifest)
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        validate_manifest(bytes, &manifest, &ValidationLimits::default())
            .map_err(|e| DomainError::Validation(format!("manifest rejected: {e}")))?;
        if matches!(manifest.adapter_kind, AdapterKind::InProcess) {
            return Err(DomainError::Validation(
                "in-process characters are loaded by the desktop host; adapter tokens are for external-process / remote-device / web adapters".into(),
            ));
        }
        let token = random_token();
        let adapter_id = format!("adp-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let record = CharacterAdapterRecord {
            adapter_id: adapter_id.clone(),
            display_name: display_name.to_string(),
            manifest,
            token_sha256: sha256_hex(&token),
            created_at: Utc::now(),
            revoked: false,
        };
        let body = serde_json::to_string(&record)
            .map_err(|e| DomainError::Internal(format!("serialize adapter record: {e}")))?;
        self.store.save_character_adapter(&adapter_id, &body)?;
        lock(&self.character.adapters).insert(adapter_id.clone(), record.clone());
        let _ = self.store.audit(
            "character.adapter-added",
            "user",
            &json!({
                "adapterId": adapter_id,
                "displayName": record.display_name,
                "characterId": record.manifest.character_id,
                "adapterKind": record.manifest.adapter_kind,
            }),
        );
        Ok(json!({ "adapterId": adapter_id, "token": token }))
    }

    /// 撤銷外部 adapter：token 立即失效、連線立即斷開（pending → uncertain）、
    /// 撤銷旗標持久化（重啟後仍撤銷）。
    pub async fn character_adapter_revoke(&self, adapter_id: &str) -> DomainResult<Value> {
        let record = {
            let mut adapters = lock(&self.character.adapters);
            let record = adapters
                .get_mut(adapter_id)
                .ok_or_else(|| DomainError::NotFound(format!("character adapter {adapter_id}")))?;
            record.revoked = true;
            record.clone()
        };
        let body = serde_json::to_string(&record)
            .map_err(|e| DomainError::Internal(format!("serialize adapter record: {e}")))?;
        self.store.save_character_adapter(adapter_id, &body)?;
        let was_connected = self
            .character_disconnect_adapter(adapter_id, DisconnectReason::Revoked)
            .await;
        let _ = self.store.audit(
            "character.adapter-revoked",
            "user",
            &json!({"adapterId": adapter_id, "wasConnected": was_connected}),
        );
        Ok(json!({
            "adapterId": adapter_id,
            "revoked": true,
            "disconnected": was_connected,
        }))
    }

    async fn character_disconnect_adapter(
        &self,
        adapter_id: &str,
        reason: DisconnectReason,
    ) -> bool {
        let hub = self.character.clone();
        let instance_id = adapter_instance_id(adapter_id);
        let iid = InstanceId(instance_id.clone());
        let now = hub.now();
        let final_entry = hub.instance_entry(&instance_id).map(|mut entry| {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("connected".into(), json!(false));
                obj.insert("negotiated".into(), json!(false));
                obj.insert("lifecycle".into(), json!("disposed"));
            }
            entry
        });
        let conn = lock(&hub.connections).remove(&instance_id);
        let was_connected = conn.is_some();
        if let Some(conn) = conn {
            let _ = conn.tx.try_send(WireMessage::Goodbye {
                reason: Some("revoked".into()),
            });
            conn.close.cancel();
        }
        let outputs = {
            let mut gw = hub.gateway();
            if gw.instance(&iid).is_some() {
                gw.remove_instance(&iid, now)
            } else {
                Vec::new()
            }
        };
        lock(&hub.instances).remove(&instance_id);
        self.character_apply(outputs).await;
        if let Some(entry) = final_entry {
            self.events
                .publish(RuntimeEvent::new(EventType::CharacterInstance, now, entry));
        }
        let _ = reason;
        was_connected
    }

    /// adapter token → adapterId（已撤銷／未知 → None）。
    pub fn character_adapter_for_token(&self, token: &str) -> Option<String> {
        self.character.adapter_for_token(token)
    }

    /// `status().characterProtocol`。
    pub fn character_status(&self) -> Value {
        self.character.status()
    }

    /// 人類手動測試：只允許**非安全** intent（`truthState: none`）。安全 intent
    /// 只能由 runtime 事件投影產生。
    pub async fn character_manual_intent(
        &self,
        intent: &str,
        message: Option<String>,
    ) -> DomainResult<Value> {
        let parsed = CharacterIntent::parse(intent).ok_or_else(|| {
            DomainError::Validation(format!("unknown character intent '{intent}'"))
        })?;
        if parsed.is_safety() {
            return Err(DomainError::PolicyBlocked(format!(
                "'{intent}' is a safety intent (priority floor {}); it can only be produced by runtime truth projection",
                parsed.priority_floor()
            )));
        }
        if let Some(text) = &message {
            if text.chars().count() > MAX_HINT_MESSAGE_CHARS || text.chars().any(char::is_control) {
                return Err(DomainError::Validation(
                    "message must be <= 200 printable chars".into(),
                ));
            }
        }
        let correlation = format!("manual:{}", uuid::Uuid::new_v4().simple());
        let mut projection =
            Projection::new(parsed, TruthState::None).with_correlation(correlation.clone());
        if let Some(text) = message {
            projection = projection.with_message(text);
        }
        let (message_id, targets) = self.character_project(projection);
        let _ = self.store.audit(
            "character.manual-intent",
            "user",
            &json!({"intent": intent, "messageId": message_id, "targets": targets}),
        );
        let note = targets
            .is_empty()
            .then_some("no connected character instance; nothing was presented");
        Ok(json!({
            "messageId": message_id,
            "intent": intent,
            "truthState": TruthState::None,
            "correlationId": correlation,
            "targets": targets,
            "note": note,
        }))
    }

    // ------------------------------------------------------------------
    // 外部 adapter WebSocket（API 層驅動 socket，這裡只管狀態與佇列）
    // ------------------------------------------------------------------

    /// 掛上一條外部連線：registered adapter → instance `adapter:<id>`（role familiar）；
    /// 同一 adapter 的舊連線被取代（goodbye＋pending → uncertain）。第一則 outbound 是 `hello`。
    pub async fn character_ws_attach(&self, adapter_id: &str) -> DomainResult<WsSession> {
        let hub = self.character.clone();
        let record = hub
            .adapter_record(adapter_id)
            .filter(|record| !record.revoked)
            .ok_or_else(|| DomainError::NotFound(format!("character adapter {adapter_id}")))?;
        let instance_id = adapter_instance_id(adapter_id);
        let iid = InstanceId(instance_id.clone());
        let conn_id = hub.conn_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let now = hub.now();
        let previous = lock(&hub.connections).remove(&instance_id);
        if let Some(prev) = previous {
            let _ = prev.tx.try_send(WireMessage::Goodbye {
                reason: Some("superseded".into()),
            });
            prev.close.cancel();
        }
        let mut outputs = Vec::new();
        {
            let mut gw = hub.gateway();
            match gw.instance(&iid) {
                Some(view) if view.connected => {
                    outputs.extend(gw.on_disconnect(&iid, DisconnectReason::TransportClosed, now));
                }
                Some(_) => {}
                None => gw.register_instance_with_id(
                    iid.clone(),
                    record.manifest.clone(),
                    CharacterRole::Familiar,
                ),
            }
        }
        lock(&hub.instances)
            .entry(instance_id.clone())
            .or_insert_with(|| InstanceMeta {
                instance_id: instance_id.clone(),
                manifest: record.manifest.clone(),
                role: CharacterRole::Familiar,
                origin: InstanceOrigin::External,
                adapter_id: Some(adapter_id.to_string()),
                tested: false,
            });
        let (tx, rx) = mpsc::channel(OUTBOUND_CAP);
        let close = CancellationToken::new();
        let hello = hub.gateway().hello_for(&iid).ok_or_else(|| {
            DomainError::Internal("character instance vanished during attach".into())
        })?;
        let _ = tx.try_send(WireMessage::Hello(hello));
        lock(&hub.connections).insert(
            instance_id.clone(),
            Connection {
                tx,
                close: close.clone(),
                conn_id,
            },
        );
        self.character_apply(outputs).await;
        self.publish_character_instance(&instance_id);
        let _ = self.store.audit(
            "character.adapter-connected",
            "adapter",
            &json!({"adapterId": adapter_id, "instanceId": instance_id, "connId": conn_id}),
        );
        Ok(WsSession {
            instance_id,
            conn_id,
            rx,
            close,
        })
    }

    /// 一則 adapter → runtime 訊息（≤ 64 KB；rate limit 50/s、方向檢查由 gateway 做）。
    pub async fn character_ws_message(
        &self,
        instance_id: &str,
        conn_id: u64,
        bytes: &[u8],
    ) -> WsStep {
        let hub = self.character.clone();
        if hub.connection_id(instance_id) != Some(conn_id) {
            return WsStep::Close;
        }
        let now = hub.now();
        let iid = InstanceId(instance_id.to_string());
        let message = match parse_wire(bytes) {
            Ok(message) => message,
            Err(err) => {
                let _ = self.store.audit(
                    "character.wire-rejected",
                    "adapter",
                    &json!({"instanceId": instance_id, "code": err.code(), "bytes": bytes.len()}),
                );
                hub.send_external(instance_id, WireMessage::error(err.code(), err.to_string()));
                return WsStep::KeepOpen;
            }
        };
        let is_goodbye = matches!(message, WireMessage::Goodbye { .. });
        let (before, outputs, drained, after) = {
            let mut gw = hub.gateway();
            let snapshot = |gw: &Gateway| {
                gw.instance(&iid)
                    .map(|v| (v.connected, v.negotiated, v.generation, v.lifecycle))
            };
            let before = snapshot(&gw);
            let outputs = gw.on_message(&iid, message, now);
            let drained = gw.drain_input(&iid);
            let after = snapshot(&gw);
            (before, outputs, drained, after)
        };
        let handshake_rejected = outputs.iter().any(|o| {
            matches!(
                o,
                GatewayOutput::Send { message: WireMessage::Error { code, .. }, .. }
                    if code == "protocol-version" || code == "character-mismatch"
            )
        });
        self.character_apply(outputs).await;
        for event in drained {
            if let Err(err) = self
                .character_input_to_observation(instance_id, InstanceOrigin::External, event)
                .await
            {
                tracing::debug!(instance = instance_id, error = %err, "character input not ingested");
            }
        }
        if before != after {
            self.publish_character_instance(instance_id);
        }
        if is_goodbye || handshake_rejected {
            WsStep::Close
        } else {
            WsStep::KeepOpen
        }
    }

    /// socket 關閉（任何原因）：pending → `uncertain`、generation+1、需重新 hello。
    pub async fn character_ws_closed(
        &self,
        instance_id: &str,
        conn_id: u64,
        reason: DisconnectReason,
    ) {
        let hub = self.character.clone();
        {
            let mut connections = lock(&hub.connections);
            match connections.get(instance_id) {
                Some(conn) if conn.conn_id == conn_id => {
                    connections.remove(instance_id);
                }
                _ => return,
            }
        }
        let now = hub.now();
        let iid = InstanceId(instance_id.to_string());
        let outputs = {
            let mut gw = hub.gateway();
            if gw.instance(&iid).is_some_and(|v| v.connected) {
                gw.on_disconnect(&iid, reason, now)
            } else {
                Vec::new()
            }
        };
        self.character_apply(outputs).await;
        self.publish_character_instance(instance_id);
        let _ = self.store.audit(
            "character.adapter-disconnected",
            "adapter",
            &json!({"instanceId": instance_id, "connId": conn_id, "reason": reason}),
        );
    }

    pub fn character_generation(&self, instance_id: &str) -> Option<u64> {
        self.character
            .gateway()
            .generation(&InstanceId(instance_id.to_string()))
    }

    /// 桌面 presence 心跳（`POST /v1/presentation/hello`）也算 gateway 的 heartbeat。
    pub(crate) fn character_desktop_heartbeat(&self, now: Timestamp) {
        self.character
            .gateway()
            .heartbeat(&InstanceId(DESKTOP_INSTANCE_ID.to_string()), now);
    }

    /// watchdog：heartbeat 逾時、過期、acknowledged→uncertain、桌面 presence 過期。
    pub(crate) async fn character_sweep(&self) {
        let now = self.character.now();
        self.character_sweep_at(now).await;
    }

    pub async fn character_sweep_at(&self, now: Timestamp) {
        let hub = self.character.clone();
        let mut changed = Vec::new();
        let outputs = {
            let mut gw = hub.gateway();
            let before: BTreeMap<String, bool> = gw
                .instances()
                .into_iter()
                .map(|v| (v.id.0, v.connected))
                .collect();
            let mut outputs = gw.sweep(now);
            let desktop = InstanceId(DESKTOP_INSTANCE_ID.to_string());
            if gw.instance(&desktop).is_some_and(|v| v.connected)
                && !self.presentation.connected(now)
            {
                outputs.extend(gw.on_disconnect(&desktop, DisconnectReason::TransportClosed, now));
            }
            let connections = lock(&hub.connections);
            for (id, was_connected) in before {
                let now_connected = gw
                    .instance(&InstanceId(id.clone()))
                    .is_some_and(|v| v.connected);
                if was_connected != now_connected {
                    if !now_connected {
                        if let Some(conn) = connections.get(&id) {
                            conn.close.cancel();
                        }
                    }
                    changed.push(id);
                }
            }
            outputs
        };
        self.character_apply(outputs).await;
        for id in changed {
            self.publish_character_instance(&id);
        }
    }

    /// 關機：告訴外部 adapter 再見（不等待）。
    pub(crate) fn character_shutdown(&self) {
        let mut connections = lock(&self.character.connections);
        for (_, conn) in connections.iter() {
            let _ = conn.tx.try_send(WireMessage::Goodbye {
                reason: Some("runtime-shutdown".into()),
            });
            conn.close.cancel();
        }
        connections.clear();
    }

    // ------------------------------------------------------------------
    // Truth projection（README §11）
    // ------------------------------------------------------------------

    /// 把一筆投影派給所有已連線 instance；回傳 (messageId, 實際送達的 instance)。
    /// 沒有任何 instance 時：安全 intent 走 `character.system-text`（不得遺失），
    /// 非安全 intent 靜默略過。
    pub fn character_project(&self, projection: Projection) -> (String, Vec<String>) {
        let hub = self.character.clone();
        let now = hub.now();
        let message_id = format!("rt-{}", uuid::Uuid::new_v4().simple());
        let targets = hub.connected_instance_ids();
        if targets.is_empty() {
            if projection.intent.is_safety() {
                let base = default_system_text(projection.intent);
                let message = match projection.hints.as_ref().and_then(|h| h.message.clone()) {
                    Some(hint) => format!("{base} — {hint}"),
                    None => base.to_string(),
                };
                self.publish_character_system_text(
                    None,
                    &message_id,
                    projection.correlation_id.as_deref(),
                    projection.intent,
                    projection.truth_state,
                    &message,
                );
            }
            return (message_id, Vec::new());
        }
        let ttl_ms = projection
            .duration_ms
            .map(|ms| i64::try_from(ms).unwrap_or(DEFAULT_INTENT_TTL_MS))
            .unwrap_or(0)
            .max(DEFAULT_INTENT_TTL_MS);
        let expires_at = now + chrono::Duration::milliseconds(ttl_ms);
        let outputs = {
            let mut gw = hub.gateway();
            let mut outputs = Vec::new();
            for id in &targets {
                let envelope = projection.envelope(&message_id, id, now, expires_at);
                outputs.extend(gw.dispatch(&InstanceId(id.clone()), envelope, now));
            }
            outputs
        };
        let sent = sent_targets(&outputs, &message_id);
        self.character_apply_sync(outputs);
        (message_id, sent)
    }

    /// `action.*`（非角色 actuator）投影。回傳實際投影的 (intent, truthState)。
    pub fn character_project_action(
        &self,
        event_type: EventType,
        receipt: &ActionReceipt,
    ) -> Option<(CharacterIntent, TruthState)> {
        if is_presentation_surface_actuator(receipt.actuator_id.as_str()) {
            return None;
        }
        let (intent, truth_state) = action_projection(event_type)?;
        // observed 之後緊接的 completed 已由 verified-success 表達，不降級成 claim。
        if event_type == EventType::ActionCompleted
            && receipt
                .verification
                .as_ref()
                .is_some_and(|v| v.verdict == VerificationVerdict::Observed)
        {
            return None;
        }
        let projection = Projection::new(intent, truth_state)
            .with_correlation(receipt.action_id.as_str())
            .with_parameter("actuatorId", json!(receipt.actuator_id.as_str()))
            .with_parameter("planId", json!(receipt.plan_id.as_str()))
            .with_parameter("actionIntent", json!(receipt.intent));
        self.character_project(projection);
        Some((intent, truth_state))
    }

    /// `agent.session.state` 投影（correlationId = agentSessionId）。
    pub fn character_project_session(
        &self,
        session_id: &str,
        state: &str,
    ) -> Option<(CharacterIntent, TruthState)> {
        let (intent, truth_state) = session_projection(state)?;
        let projection = Projection::new(intent, truth_state)
            .with_correlation(session_id)
            .with_parameter("agentSessionId", json!(session_id))
            .with_parameter("sessionState", json!(state));
        self.character_project(projection);
        Some((intent, truth_state))
    }

    /// `provider.state-changed` 投影（available／paired → greet、disconnected／revoked → notice）。
    /// 桌面角色自己的 provider 不投影。
    pub fn character_project_provider(
        &self,
        id: &ProviderId,
        state: ProviderState,
    ) -> Option<CharacterIntent> {
        if id.as_str() == COMPANION_PROVIDER_ID {
            return None;
        }
        let (intent, variant) = provider_projection(state)?;
        let projection = Projection::new(intent, TruthState::None)
            .with_correlation(id.as_str())
            .with_variant(variant)
            .with_parameter("providerId", json!(id.as_str()))
            .with_parameter("providerState", json!(state));
        self.character_project(projection);
        Some(intent)
    }

    /// `receptor.observation` → `notice(listening)`。角色自己表面的受器（companion.*）
    /// 不投影（不對自己的輸入回音）；同一受器 2 s 內只投影一次。
    pub fn character_project_observation(&self, observation: &Observation) -> bool {
        let receptor = observation.receptor_id.as_str();
        if crate::presentation::is_companion_surface_receptor(receptor) {
            return false;
        }
        let now = self.character.now();
        if throttled(
            &self.character.observation_last,
            receptor,
            now,
            OBSERVATION_PROJECTION_THROTTLE_MS,
        ) {
            return false;
        }
        let projection = Projection::new(CharacterIntent::Notice, TruthState::None)
            .with_correlation(format!("receptor:{receptor}"))
            .with_variant("listening")
            .with_interrupt(InterruptPolicy::Merge)
            .with_parameter("receptorId", json!(receptor))
            .with_parameter("observationId", json!(observation.observation_id.as_str()));
        self.character_project(projection);
        true
    }

    /// `emergency.stop`／cleared → emergency／idle。
    pub fn character_project_emergency(&self, engaged: bool) {
        let projection = if engaged {
            Projection::new(CharacterIntent::Emergency, TruthState::Emergency)
                .with_correlation("emergency-stop")
        } else {
            Projection::new(CharacterIntent::Idle, TruthState::None)
                .with_correlation("emergency-stop")
                .with_variant("emergency-cleared")
        };
        self.character_project(projection);
    }

    /// `proactive.paused`／`resumed` → rest／idle。
    pub fn character_project_proactive(&self, paused: bool) {
        let projection = if paused {
            Projection::new(CharacterIntent::Rest, TruthState::None)
                .with_correlation("proactive-pause")
                .with_variant("paused")
        } else {
            Projection::new(CharacterIntent::Idle, TruthState::None)
                .with_correlation("proactive-pause")
                .with_variant("resumed")
        };
        self.character_project(projection);
    }

    /// `plan.blocked` → blocked（correlationId = planId）。
    pub fn character_project_plan_blocked(&self, plan_id: &str, reason: Option<&str>) {
        let mut projection = Projection::new(CharacterIntent::Blocked, TruthState::Blocked)
            .with_correlation(plan_id)
            .with_parameter("planId", json!(plan_id));
        if let Some(reason) = reason {
            projection = projection.with_message(reason);
        }
        self.character_project(projection);
    }

    /// AI 的 `companion.state.present`／`animation.play`：`truthState: none`、priority ≤ 50、
    /// `messageId = correlationId = actionId`，回執回來才推進 presentation receipt。
    pub(crate) fn character_ai_present(
        &self,
        action: &BoundedAction,
        kind: PresentationKind,
        params: &Value,
        now: Timestamp,
        expires_at: Timestamp,
    ) -> Vec<String> {
        let hub = self.character.clone();
        let targets = hub.connected_instance_ids();
        if targets.is_empty() {
            return Vec::new();
        }
        let (intent, variant, message, tone) = match kind {
            PresentationKind::StatePresent => {
                let behavior = params["behaviorIntent"].as_str().unwrap_or("notice");
                let (intent, variant) = behavior_intent_projection(behavior);
                (
                    intent,
                    variant.map(String::from),
                    params["message"].as_str().map(String::from),
                    params["tone"].as_str().map(String::from),
                )
            }
            PresentationKind::AnimationPlay => {
                let name = params["animation"].as_str().unwrap_or("notice");
                let (intent, variant) = animation_projection(name);
                (intent, variant, None, None)
            }
            _ => return Vec::new(),
        };
        let action_id = action.action_id.as_str().to_string();
        let message_id = action_id.clone();
        let mut outputs = Vec::new();
        {
            let mut gw = hub.gateway();
            for id in &targets {
                let envelope = match IntentEnvelope::from_ai_request(
                    message_id.clone(),
                    id.clone(),
                    Some(action_id.clone()),
                    intent,
                    AI_INTENT_PRIORITY,
                    now,
                    expires_at,
                ) {
                    Ok(envelope) => envelope,
                    Err(err) => {
                        tracing::warn!(error = %err, "ai presentation intent refused by protocol");
                        continue;
                    }
                };
                let mut envelope = envelope;
                envelope.presentation_hints = Some(PresentationHints {
                    tone: tone.clone(),
                    message: message.clone(),
                    variant: variant.clone(),
                    channels: BTreeMap::new(),
                });
                envelope
                    .parameters
                    .insert("command".into(), json!(kind.command()));
                outputs.extend(gw.dispatch(&InstanceId(id.clone()), envelope, now));
            }
        }
        hub.remember_ai_command(&message_id, &action_id, kind.command());
        let sent = sent_targets(&outputs, &message_id);
        self.character_apply_sync(outputs);
        sent
    }

    // ------------------------------------------------------------------
    // Provider 顯示名（隨 active character）
    // ------------------------------------------------------------------

    pub fn companion_provider_display_name(&self) -> String {
        match self.character.active_character() {
            Some(meta) => format!(
                "桌面角色：{}（Presentation）",
                display_name_zh(&meta.manifest.display_name)
            ),
            None => "桌面角色（尚未連線）".to_string(),
        }
    }

    pub fn companion_provider_detail(&self) -> String {
        match self.character.active_character() {
            Some(meta) => format!(
                "角色 {}（{}／{}）；能力逐項授權；隱藏角色只停用視窗內能力，不影響 Runtime",
                meta.manifest.character_id,
                adapter_kind_str(meta.manifest.adapter_kind),
                meta.origin.as_str()
            ),
            None => "尚未有角色經 /v1/character/hello 協商；能力逐項授權；隱藏角色只停用視窗內能力，不影響 Runtime".to_string(),
        }
    }

    // ------------------------------------------------------------------
    // Gateway 輸出 → 事件／傳輸／presentation receipt
    // ------------------------------------------------------------------

    fn apply_character_outputs(&self, outputs: Vec<GatewayOutput>) -> Vec<Settlement> {
        let hub = self.character.clone();
        let now = hub.now();
        let mut settlements = Vec::new();
        let mut intents: Vec<IntentGroup> = Vec::new();
        for output in outputs {
            match output {
                GatewayOutput::Send { instance, message } => {
                    let instance_id = instance.0;
                    if let WireMessage::Intent { envelope } = &message {
                        let idx = match intents
                            .iter()
                            .position(|group| group.message_id == envelope.message_id)
                        {
                            Some(idx) => idx,
                            None => {
                                intents.push(IntentGroup {
                                    message_id: envelope.message_id.clone(),
                                    desktop: None,
                                    first: None,
                                    targets: Vec::new(),
                                });
                                intents.len() - 1
                            }
                        };
                        if instance_id == DESKTOP_INSTANCE_ID {
                            intents[idx].desktop = Some(envelope.clone());
                        } else if intents[idx].first.is_none() {
                            intents[idx].first = Some(envelope.clone());
                        }
                        intents[idx].targets.push(instance_id.clone());
                    }
                    if instance_id == DESKTOP_INSTANCE_ID {
                        // 桌面視窗的傳輸就是事件：intent → `character.intent`；negotiated
                        // 隨 hello 的 HTTP 回應；cancel 由 `character.receipt{cancelled}` 表達。
                        if !matches!(message, WireMessage::Intent { .. }) {
                            tracing::debug!(
                                kind = message.kind(),
                                "desktop character message carried by events/receipts"
                            );
                        }
                    } else {
                        hub.send_external(&instance_id, message);
                    }
                }
                GatewayOutput::SystemText {
                    instance,
                    message_id,
                    correlation_id,
                    intent,
                    truth_state,
                    message,
                } => {
                    self.publish_character_system_text(
                        Some(instance.as_str()),
                        &message_id,
                        correlation_id.as_deref(),
                        intent,
                        truth_state,
                        &message,
                    );
                }
                GatewayOutput::Receipt(receipt) => {
                    if receipt.status == ReceiptStatus::Completed {
                        hub.mark_tested(&receipt.character_instance_id);
                    }
                    self.publish_character_receipt(&receipt);
                    if receipt.is_terminal() {
                        if let Some((action_id, _command)) =
                            hub.take_ai_command(&receipt.message_id)
                        {
                            settlements.push(Settlement {
                                action_id,
                                outcome: settlement_outcome(receipt.status),
                                detail: receipt.detail.clone().or_else(|| receipt.reason.clone()),
                            });
                        }
                    }
                }
                GatewayOutput::Audit(text) => {
                    tracing::debug!(target: "interaction_runtime::character", "{text}");
                }
            }
        }
        for group in intents {
            if let Some(envelope) = group.desktop.or(group.first) {
                let mut event = RuntimeEvent::new(
                    EventType::CharacterIntent,
                    now,
                    json!({ "envelope": envelope, "targets": group.targets }),
                );
                if let Some(corr) = &envelope.correlation_id {
                    event = event.with_correlation(CorrelationId::new(corr.clone()));
                }
                self.events.publish(event);
            }
        }
        settlements
    }

    async fn character_apply(&self, outputs: Vec<GatewayOutput>) {
        let settlements = self.apply_character_outputs(outputs);
        self.settle_character_receipts(settlements).await;
    }

    /// 同步呼叫端（emit 點）：settlement 需要 async（persist receipt），交給 runtime task。
    fn character_apply_sync(&self, outputs: Vec<GatewayOutput>) {
        let settlements = self.apply_character_outputs(outputs);
        if settlements.is_empty() {
            return;
        }
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let runtime = self.clone();
                handle.spawn(async move {
                    runtime.settle_character_receipts(settlements).await;
                });
            }
            Err(_) => tracing::warn!(
                count = settlements.len(),
                "character receipt settlements dropped: no async runtime"
            ),
        }
    }

    async fn settle_character_receipts(&self, settlements: Vec<Settlement>) {
        for settlement in settlements {
            match self
                .presentation_ack(
                    &settlement.action_id,
                    settlement.outcome,
                    settlement.detail.clone(),
                )
                .await
            {
                Ok(_) => {}
                Err(DomainError::NotFound(_)) => tracing::debug!(
                    action = settlement.action_id,
                    outcome = settlement.outcome,
                    "presentation command already settled or expired"
                ),
                Err(err) => tracing::warn!(
                    action = settlement.action_id,
                    error = %err,
                    "presentation settlement from character receipt failed"
                ),
            }
        }
    }

    fn publish_character_receipt(&self, receipt: &CommandReceipt) {
        let now = self.character.now();
        self.events.publish(RuntimeEvent::new(
            EventType::CharacterReceipt,
            now,
            json!({ "instanceId": receipt.character_instance_id, "receipt": receipt }),
        ));
        let _ = self.store.audit(
            "character.receipt",
            "runtime",
            &json!({
                "instanceId": receipt.character_instance_id,
                "messageId": receipt.message_id,
                "generation": receipt.generation,
                "status": receipt.status,
                "resolution": receipt.resolution,
                "reason": receipt.reason,
            }),
        );
    }

    fn publish_character_system_text(
        &self,
        instance_id: Option<&str>,
        message_id: &str,
        correlation_id: Option<&str>,
        intent: CharacterIntent,
        truth_state: TruthState,
        message: &str,
    ) {
        let now = self.character.now();
        let mut event = RuntimeEvent::new(
            EventType::CharacterSystemText,
            now,
            json!({
                "instanceId": instance_id,
                "messageId": message_id,
                "correlationId": correlation_id,
                "intent": intent,
                "truthState": truth_state,
                "message": message,
            }),
        );
        if let Some(corr) = correlation_id {
            event = event.with_correlation(CorrelationId::new(corr));
        }
        self.events.publish(event);
        let _ = self.store.audit(
            "character.system-text",
            "runtime",
            &json!({
                "instanceId": instance_id,
                "messageId": message_id,
                "intent": intent,
                "truthState": truth_state,
            }),
        );
        tracing::info!(
            intent = intent.as_str(),
            message,
            "character system.text fallback"
        );
    }

    fn publish_character_instance(&self, instance_id: &str) {
        if let Some(entry) = self.character.instance_entry(instance_id) {
            let now = self.character.now();
            self.events
                .publish(RuntimeEvent::new(EventType::CharacterInstance, now, entry));
        }
    }

    // ------------------------------------------------------------------
    // Input event → receptor observation
    // ------------------------------------------------------------------

    async fn character_input_to_observation(
        &self,
        instance_id: &str,
        origin: InstanceOrigin,
        event: CharacterInputEvent,
    ) -> DomainResult<InputOutcome> {
        let now = self.character.now();
        let mut facts: BTreeMap<String, Value> = BTreeMap::new();
        facts.insert("instanceId".into(), json!(instance_id));
        facts.insert("eventId".into(), json!(event.event_id));
        let receptor = match event.kind {
            InputEventKind::Clicked | InputEventKind::DoubleClicked => {
                facts.insert("kind".into(), json!("companion-clicked"));
                if event.kind == InputEventKind::DoubleClicked {
                    facts.insert("double".into(), json!(true));
                }
                "companion.click"
            }
            InputEventKind::TextSubmitted => {
                facts.insert("kind".into(), json!("text-submitted"));
                facts.insert("modality".into(), json!("text"));
                facts.insert(
                    "text".into(),
                    event.payload.get("text").cloned().unwrap_or(json!("")),
                );
                "companion.text-input"
            }
            InputEventKind::ActionRequested => {
                facts.insert("kind".into(), json!("action-selected"));
                facts.insert(
                    "action".into(),
                    event.payload.get("action").cloned().unwrap_or(Value::Null),
                );
                "companion.quick-action"
            }
            InputEventKind::FileDropped => {
                // 只有 metadata＋短效 grant：沒有路徑、沒有內容。正規化器接受兩種形狀
                // （README §6 扁平鍵，或 TS gateway 的 `files:[…]`）；這裡把全部檔案都回報，
                // 不再只讀第一個。
                let grant_of = |o: &Value| {
                    json!({
                        "grantId": o.get("grantId"),
                        "mediaType": o.get("mediaType"),
                        "bytes": o.get("bytes"),
                        "readableScope": o.get("readableScope"),
                        "expiresAt": o.get("expiresAt"),
                    })
                };
                let files: Vec<Value> = match event.payload.get("files").and_then(Value::as_array) {
                    Some(list) if !list.is_empty() => list.clone(),
                    _ => vec![Value::Object(event.payload.clone().into_iter().collect())],
                };
                let names: Vec<Value> = files
                    .iter()
                    .map(|f| f.get("name").cloned().unwrap_or(json!("")))
                    .collect();
                let grants: Vec<Value> = files.iter().map(grant_of).collect();
                facts.insert("kind".into(), json!("companion-dropped"));
                facts.insert("modality".into(), json!("file-drop"));
                facts.insert("fileCount".into(), json!(files.len()));
                facts.insert("names".into(), Value::Array(names));
                facts.insert("grants".into(), Value::Array(grants));
                "companion.drag-drop"
            }
            InputEventKind::HoverEntered => {
                if throttled(
                    &self.character.pointer_last,
                    instance_id,
                    now,
                    POINTER_THROTTLE_MS,
                ) {
                    return Ok(InputOutcome::Throttled);
                }
                facts.insert("kind".into(), json!("pointer-approached"));
                "companion.pointer"
            }
            InputEventKind::DragStarted | InputEventKind::Dropped => {
                facts.insert("kind".into(), json!("companion-dragged"));
                facts.insert(
                    "phase".into(),
                    json!(if event.kind == InputEventKind::DragStarted {
                        "started"
                    } else {
                        "dropped"
                    }),
                );
                "companion.click"
            }
            InputEventKind::Dragged => {
                if throttled(
                    &self.character.drag_last,
                    instance_id,
                    now,
                    DRAG_OBSERVATION_THROTTLE_MS,
                ) {
                    return Ok(InputOutcome::Throttled);
                }
                facts.insert("kind".into(), json!("companion-dragged"));
                facts.insert("phase".into(), json!("moving"));
                "companion.click"
            }
            InputEventKind::HoverLeft
            | InputEventKind::ToyThrown
            | InputEventKind::Dismissed
            | InputEventKind::VisibilityChanged => {
                let _ = self.store.audit(
                    "character.input",
                    "runtime",
                    &json!({
                        "instanceId": instance_id,
                        "kind": event.kind,
                        "payload": event.payload,
                    }),
                );
                return Ok(InputOutcome::AuditOnly);
            }
        };
        // 桌面角色的視窗內受器受隱藏／斷線閘門管制；外部 adapter 沒有桌面表面。
        let enforce_surface_gate = origin != InstanceOrigin::External;
        self.ingest_with_gate(receptor, facts, BTreeMap::new(), 1.0, enforce_surface_gate)
            .await?;
        Ok(InputOutcome::Observed(receptor))
    }
}

/// 一則正規化 input event 的下場。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOutcome {
    Observed(&'static str),
    Throttled,
    AuditOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 連接頁「可以接收」只列 manifest 宣告且 supported 的輸入能力；
    /// `supported:false` 與未宣告者都不出現。安全旗標依 README §9 分級。
    #[test]
    fn input_capabilities_and_security_flags_come_from_the_manifest() {
        let manifest: CharacterManifest = serde_json::from_value(json!({
            "schemaVersion": "1.0",
            "characterId": "flag-check",
            "displayName": { "zh-TW": "旗標檢查" },
            "author": "someone",
            "version": "2.1.0",
            "adapterKind": "external-process",
            "entrypoint": { "kind": "process", "command": ["node", "adapter.mjs"] },
            "inputCapabilities": {
                "input.text": { "supported": true },
                "input.click": { "supported": false },
                "input.fileDrop": { "supported": true }
            },
            "securityRequirements": { "network": true }
        }))
        .expect("manifest parses");
        assert_eq!(
            supported_input_capabilities(&manifest),
            vec!["input.fileDrop".to_string(), "input.text".to_string()]
        );
        assert_eq!(manifest_security_flags(&manifest), (true, true));

        let builtin: CharacterManifest = serde_json::from_value(json!({
            "schemaVersion": "1.0",
            "characterId": "plain",
            "displayName": { "zh-TW": "純文字" },
            "version": "1.0.0",
            "entrypoint": { "kind": "builtin", "id": "text" }
        }))
        .expect("builtin manifest parses");
        assert!(supported_input_capabilities(&builtin).is_empty());
        assert_eq!(manifest_security_flags(&builtin), (false, false));
        assert!(
            builtin.author.is_none(),
            "author stays honest-null when absent"
        );
    }

    #[test]
    fn projection_tables_follow_readme_section_11() {
        use CharacterIntent as I;
        use TruthState as T;
        assert_eq!(session_projection("created"), Some((I::Wait, T::Queued)));
        assert_eq!(session_projection("fetched"), Some((I::Think, T::Working)));
        assert_eq!(session_projection("working"), Some((I::Work, T::Working)));
        assert_eq!(
            session_projection("waiting-input"),
            Some((I::Ask, T::WaitingInput))
        );
        assert_eq!(
            session_projection("waiting-consent"),
            Some((I::RequestConsent, T::WaitingConsent))
        );
        assert_eq!(
            session_projection("claimed-completed"),
            Some((I::ClaimCompleted, T::Claimed))
        );
        assert_eq!(
            session_projection("verified"),
            Some((I::VerifiedSuccess, T::Verified))
        );
        assert_eq!(session_projection("failed"), Some((I::Failed, T::Failed)));
        assert_eq!(
            session_projection("timed-out"),
            Some((I::Failed, T::TimedOut))
        );
        assert_eq!(
            session_projection("unknown"),
            Some((I::Unknown, T::Unknown))
        );
        assert_eq!(
            session_projection("cancelled"),
            Some((I::Cancelled, T::Cancelled))
        );
        assert_eq!(session_projection("closed"), Some((I::Idle, T::None)));
        assert_eq!(session_projection("bogus"), None);

        assert_eq!(
            action_projection(EventType::ActionDispatched),
            Some((I::Work, T::Working))
        );
        assert_eq!(
            action_projection(EventType::ActionAcknowledged),
            Some((I::Acknowledge, T::Working))
        );
        assert_eq!(
            action_projection(EventType::ActionCompleted),
            Some((I::ClaimCompleted, T::Claimed))
        );
        assert_eq!(
            action_projection(EventType::ActionObserved),
            Some((I::VerifiedSuccess, T::Verified))
        );
        assert_eq!(
            action_projection(EventType::ActionUncertain),
            Some((I::Unknown, T::Unknown))
        );
        assert_eq!(
            action_projection(EventType::ActionFailed),
            Some((I::Failed, T::Failed))
        );
        assert_eq!(action_projection(EventType::PlanCreated), None);

        assert_eq!(
            provider_projection(ProviderState::Available),
            Some((I::Greet, "device-online"))
        );
        assert_eq!(
            provider_projection(ProviderState::Paired),
            Some((I::Greet, "device-online"))
        );
        assert_eq!(
            provider_projection(ProviderState::Disconnected),
            Some((I::Notice, "device-offline"))
        );
        assert_eq!(
            provider_projection(ProviderState::Revoked),
            Some((I::Notice, "device-offline"))
        );
        assert_eq!(provider_projection(ProviderState::Installed), None);
    }

    #[test]
    fn ai_substitution_never_yields_a_safety_intent() {
        // wait-attention → think（variant wait-attention）、look-at-confirmation →
        // notice（variant look-at-confirmation）；其餘 behaviorIntent 也都非安全。
        assert_eq!(
            behavior_intent_projection("wait-attention"),
            (CharacterIntent::Think, Some("wait-attention"))
        );
        assert_eq!(
            behavior_intent_projection("look-at-confirmation"),
            (CharacterIntent::Notice, Some("look-at-confirmation"))
        );
        for behavior in crate::presentation::BEHAVIOR_INTENTS {
            let (intent, _) = behavior_intent_projection(behavior);
            assert!(
                !intent.is_safety(),
                "{behavior} must map to a non-safety intent"
            );
            assert!(intent.ai_allowed());
        }
        // 動畫名：安全 intent 名稱／別名一律降級。
        for name in [
            "emergency",
            "blocked",
            "success",
            "verified-success",
            "waiting",
            "failed",
            "unknown",
        ] {
            let (intent, variant) = animation_projection(name);
            assert!(!intent.is_safety(), "{name} → {intent} must not be safety");
            assert_eq!(variant.as_deref(), Some(name));
        }
        assert_eq!(
            animation_projection("think"),
            (CharacterIntent::Think, None)
        );
        assert_eq!(
            animation_projection("thinking"),
            (CharacterIntent::Think, Some("thinking".into()))
        );
        assert_eq!(
            animation_projection("curious"),
            (CharacterIntent::Notice, Some("curious".into()))
        );
    }

    #[test]
    fn adapter_token_is_hashed_and_compared_in_constant_time() {
        let hub = CharacterHub::default();
        let token = random_token();
        assert_eq!(token.len(), 64);
        let manifest = interaction_character::minimal_manifest("fixture", "text");
        let mut manifest = manifest;
        manifest.adapter_kind = AdapterKind::ExternalProcess;
        manifest.entrypoint = Entrypoint::Process {
            command: vec!["node".into()],
        };
        hub.load_adapters(vec![CharacterAdapterRecord {
            adapter_id: "adp-1".into(),
            display_name: "fixture".into(),
            manifest,
            token_sha256: sha256_hex(&token),
            created_at: Utc::now(),
            revoked: false,
        }]);
        assert_eq!(hub.adapter_for_token(&token).as_deref(), Some("adp-1"));
        assert_eq!(hub.adapter_for_token("nope"), None);
        assert_eq!(hub.adapter_for_token(""), None);
        let view = hub.adapters_view();
        assert_eq!(view.len(), 1);
        assert!(view[0].get("token").is_none());
        assert!(view[0].get("tokenSha256").is_none());
        assert_eq!(view[0]["connected"], false);
    }

    #[test]
    fn settlement_outcomes_are_honest() {
        assert_eq!(settlement_outcome(ReceiptStatus::Completed), "completed");
        assert_eq!(
            settlement_outcome(ReceiptStatus::Unsupported),
            "unsupported"
        );
        assert_eq!(settlement_outcome(ReceiptStatus::Failed), "failed");
        assert_eq!(settlement_outcome(ReceiptStatus::Cancelled), "interrupted");
        assert_eq!(settlement_outcome(ReceiptStatus::Expired), "expired");
        assert_eq!(settlement_outcome(ReceiptStatus::Uncertain), "uncertain");
        assert!(is_presentation_surface_actuator("companion.bubble.show"));
        assert!(is_presentation_surface_actuator("iphone.character"));
        assert!(!is_presentation_surface_actuator("iphone.haptic"));
        assert!(!is_presentation_surface_actuator("conversation"));
    }
}
