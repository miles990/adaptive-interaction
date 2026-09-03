//! iPhone Mobile Provider（v0.5 Phase 6）。
//!
//! 桌面端伺服器：TLS WebSocket（自簽憑證、指紋由 QR 配對載荷釘選）＋
//! Bonjour 廣播（`_interact-ai._tcp`；服務型別 label 依 RFC 6763 §7.2 須
//! ≤15 bytes，舊名 `_interact-ai-mobile` 18 bytes 會被 mDNS 拒絕）＋配對
//! （一次性配對碼，HMAC challenge-response）＋每台 iPhone 獨立 device token。
//!
//! 誠實不變量：
//! - 配對碼一次一段（5 分鐘）；HMAC 驗證失敗即拒，不重試。
//! - token 只存 SHA-256（永不落地明文）；撤銷立即生效：現有 wss 連線收到
//!   `auth-fail(revoked)` 後由伺服器端關閉，之後重連一律 auth-fail。
//! - 心跳：無訊息 15 秒送 Ping；45 秒完全無訊息（含 Pong）視為半開連線斷開，
//!   health 不再續報 healthy。
//! - 手機斷線 → provider Disconnected、能力 unavailable；高風險受器
//!   （`iphone.mic-level`）由桌面端強制 disabled，重連不自動恢復。
//! - act 逾時＝結果未知（不重送）；ack/err 只接受「同一台手機」對「同一 id」
//!   的回覆，未認證 peer 或另一台手機不能解除 pending。
//! - estop → stop-all 直送所有手機（500ms 內去重一次）；在途 act 立即以
//!   stopped 收場；沒有任何手機連線時誠實回 Err（沒有東西被停）。
//! - 觀察只收語意事件：receptor 白名單＋facts 鍵白名單（＝manifest provides）；
//!   原始軌跡不進 runtime；丟棄計數在 status 誠實顯示。
//! - 桌面 Consent 不取代 iOS 系統權限：手機回報的 permissions 誠實顯示。

use crate::runtime::Runtime;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use interaction_adapter_sdk::{ActuatorManifestBuilder, DriverReceipt, ReceptorManifestBuilder};
use interaction_core::{
    ActionId, ActionParameters, ActionReceipt, Actuator, ActuatorError, BoundedAction,
    ComponentHealth, DomainError, DomainResult, ProviderDescriptor, ProviderId, ProviderIdentity,
    ProviderKind, ProviderState, ReceptorId, RiskClass, Sensitivity, TrustLevel,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

type HmacSha256 = Hmac<Sha256>;

pub const MOBILE_PORT_DEFAULT: u16 = 18790;
/// Bonjour 服務型別（完整 mDNS 名稱）。RFC 6763 §7.2：service name label
/// （去掉前置 `_`）最長 15 bytes；`interact-ai` = 11 bytes。
pub const MDNS_SERVICE_TYPE: &str = "_interact-ai._tcp.local.";
/// 給 iOS `NSBonjourServices` ／狀態顯示用的短型別（不含 `.local.`）。
pub const MDNS_SERVICE_SHORT: &str = "_interact-ai._tcp";
const PAIRING_TTL_SECS: i64 = 300;
const ACT_TIMEOUT: Duration = Duration::from_secs(4);
const OUTBOUND_QUEUE: usize = 32;
/// 心跳：無訊息多久送一次 Ping；多久完全無訊息視為半開連線而斷開。
const PING_INTERVAL_DEFAULT_MS: u64 = 15_000;
const IDLE_TIMEOUT_DEFAULT_MS: u64 = 45_000;
/// estop 廣播去重窗（6 個 mobile actuator 共用一條 stop-all）。
const STOP_ALL_DEDUP: Duration = Duration::from_millis(500);
/// BLE 掃描回覆的額外寬限（掃描時間＋此值＝逾時）。
const BLE_REPLY_GRACE: Duration = Duration::from_secs(2);

/// 觀察 receptor 規格：id、名稱、接受的 facts 鍵（＝manifest provides）、
/// 敏感度、是否需 consent、是否高風險（斷線強制 disabled＋不落地）。
pub struct MobileReceptorSpec {
    pub id: &'static str,
    pub name: &'static str,
    /// manifest `provides` ＝ facts 鍵白名單；不在此列的鍵一律剝除。
    pub provides: &'static [&'static str],
    pub sensitivity: Sensitivity,
    pub requires_consent: bool,
    /// 高風險受器：任何斷線／撤銷都由桌面端強制 disabled，重連不自動恢復；
    /// 觀察宣告 `retention: none`（runtime `ingest` 據此不落地）。
    pub high_risk: bool,
}

pub const MOBILE_RECEPTOR_SPECS: &[MobileReceptorSpec] = &[
    MobileReceptorSpec {
        id: "iphone.motion",
        name: "iPhone 動作（語意）",
        provides: &["event", "at"],
        sensitivity: Sensitivity::Internal,
        requires_consent: false,
        high_risk: false,
    },
    MobileReceptorSpec {
        id: "iphone.battery",
        name: "iPhone 電量與前景",
        provides: &["level", "charging", "foreground"],
        sensitivity: Sensitivity::Internal,
        requires_consent: false,
        high_risk: false,
    },
    MobileReceptorSpec {
        id: "iphone.touch",
        name: "iPhone 角色觸碰",
        provides: &["kind"],
        sensitivity: Sensitivity::Internal,
        requires_consent: false,
        high_risk: false,
    },
    MobileReceptorSpec {
        id: "iphone.mic-level",
        name: "iPhone 環境音量",
        provides: &["level"],
        sensitivity: Sensitivity::Personal,
        requires_consent: true,
        high_risk: true,
    },
];

/// 觀察 receptor 白名單（手機只能推語意事件到這些）。
pub const MOBILE_RECEPTORS: &[&str] = &[
    "iphone.motion",
    "iphone.battery",
    "iphone.touch",
    "iphone.mic-level",
];
pub const MOBILE_ACTUATORS: &[(&str, &str, &str)] = &[
    // (id, channel, 人話名稱)
    ("iphone.haptic", "haptic", "iPhone 觸覺回饋"),
    ("iphone.notify", "notification", "iPhone 通知"),
    ("iphone.tts", "audio", "iPhone 語音"),
    ("iphone.torch", "light", "iPhone 手電筒"),
    ("iphone.flash", "display", "iPhone 螢幕閃示"),
    ("iphone.character", "desktop-pet", "iPhone 角色呈現"),
];
/// `character.present` 允許的狀態（與 iOS `CharacterPresentState` 一致）。
/// `verified-success` **只能**由 runtime 的人工驗證路徑
/// （`Runtime::mobile_present_verified`）直送——plan／policy／agent 路徑
/// （含 `extra.state`）一律被 `map_wire_params` 拒絕。
pub const CHARACTER_STATES: &[&str] = &[
    "idle",
    "working",
    "waiting",
    "verified-success",
    "failed",
    "unknown",
    "emergency",
];

/// 只有 runtime 的人工驗證路徑能送出的角色狀態（手機的綠勾）。
pub const VERIFIED_STATE: &str = "verified-success";

pub fn mobile_receptor_spec(id: &str) -> Option<&'static MobileReceptorSpec> {
    MOBILE_RECEPTOR_SPECS.iter().find(|s| s.id == id)
}

/// 依 receptor 的 facts 鍵白名單過濾手機送來的 facts。
/// `None` ＝ receptor 不在白名單；`Some(空)` ＝ 沒有任何可接受的鍵。
pub fn filter_mobile_facts(receptor: &str, facts: &Value) -> Option<BTreeMap<String, Value>> {
    let spec = mobile_receptor_spec(receptor)?;
    Some(
        facts
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(k, _)| spec.provides.contains(&k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// mDNS service name label（去掉前置 `_` 與 `._tcp.local.`／`._udp.local.`）。
/// RFC 6763 §7.2 限 15 bytes；mdns-sd 超長會直接拒絕註冊。
pub fn mdns_service_label(service_type: &str) -> &str {
    let stripped = service_type
        .strip_suffix("._tcp.local.")
        .or_else(|| service_type.strip_suffix("._udp.local."))
        .unwrap_or(service_type);
    stripped.strip_prefix('_').unwrap_or(stripped)
}

/// Bonjour instance 名稱：帶主機名（`HOSTNAME` 環境變數，清洗後）以便同網段
/// 多台電腦區分；沒有就用埠號（workspace 無 hostname crate，不新增依賴）。
fn mdns_instance_name(port: u16) -> String {
    let host: String = std::env::var("HOSTNAME")
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(32)
        .collect();
    let host = host.trim_matches('-');
    if host.is_empty() {
        format!("interact-ai-{port}")
    } else {
        format!("interact-ai-{host}")
    }
}

/// haptic 強度階：magnitude（policy 已 clamp）決定「最強能到哪裡」，
/// `extra.style` 只能往下、不能往上——否則 L3 的 magnitude 硬限制會被
/// 一個字串繞過（`style: heavy` 配 `magnitude: 0.2`）。
fn haptic_style_rank(style: &str) -> Option<u8> {
    match style {
        "light" | "purr" => Some(1),
        "medium" => Some(2),
        "heavy" | "heartbeat" => Some(3),
        _ => None,
    }
}

fn haptic_rank_style(rank: u8) -> &'static str {
    match rank {
        0 | 1 => "light",
        2 => "medium",
        _ => "heavy",
    }
}

fn haptic_magnitude_rank(magnitude: f64) -> u8 {
    if magnitude < 0.34 {
        1
    } else if magnitude < 0.67 {
        2
    } else {
        3
    }
}

/// 把 policy-bounded 的 `ActionParameters` 映射成 iOS App 真正驗證的 wire
/// 參數。回傳 (wire act 名稱, params)。
///
/// **硬限制不可被 `extra` 繞過**：`durationMs` 一律取
/// min(policy 已 clamp 的 `effective.duration_ms`, `extra.durationMs`, 裝置硬上限)；
/// `extra.durationMs` 只有在 policy 沒給值時才能當預設。同理 `extra.style`
/// 只能對應到 magnitude 允許的區間或更弱，`extra.on` 也不得在 magnitude
/// 被 clamp 成 0 時把手電筒點亮。
///
/// - haptic.pulse：`style`（magnitude <0.34 light、<0.67 medium、否則 heavy）、
///   `count` 預設 1（1–5 clamp）。
/// - notify.show：`title` 預設目前桌面角色的顯示名（未連線→「角色」）、
///   `body` = extra.body 或 message（缺 → Err）。
/// - tts.speak：`text` = extra.text 或 message（缺 → Err；不替使用者編句子）。
/// - torch.set：`on` = extra.on ∧ magnitude>0、`durationMs` ≤5000（預設 1000）。
/// - screen.flash：`color` 預設 `#FFB347`、`durationMs` 1..=1500（預設 400）。
/// - character.present：`state` 必須在白名單；`verified-success` **一律拒絕**
///   （綠勾只能由 runtime 的人工驗證路徑 `mobile_present_verified` 直送）。
pub fn map_wire_params(
    actuator_id: &str,
    e: &ActionParameters,
) -> Result<(&'static str, Value), String> {
    map_wire_params_titled(actuator_id, e, None)
}

/// 同 [`map_wire_params`]，但 `notify.show` 的預設標題可帶入目前角色名
/// （Character Protocol 協商到的 displayName；沒有就用中立的「角色」）。
pub fn map_wire_params_titled(
    actuator_id: &str,
    e: &ActionParameters,
    character_title: Option<&str>,
) -> Result<(&'static str, Value), String> {
    let mut p: Map<String, Value> = e
        .extra
        .as_ref()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    // 通用鍵：App 不驗證，但保留讓 applied 可對照；magnitude 一律以 policy
    // 的有效值為準（extra 不得改寫它）。
    if let Some(m) = e.magnitude {
        p.insert("magnitude".into(), json!(m));
    }
    let message = e
        .message
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    // 有效值 = min(policy 已 clamp 的 effective, AI 的 extra, 裝置硬上限)。
    let duration_from = |p: &Map<String, Value>, default: u64, hard_max: u64| -> u64 {
        let requested = p
            .get("durationMs")
            .and_then(Value::as_f64)
            .map(|d| d.round().max(0.0) as u64);
        let value = match (e.duration_ms, requested) {
            (Some(effective), Some(extra)) => effective.min(extra),
            (Some(effective), None) => effective,
            (None, Some(extra)) => extra,
            (None, None) => default,
        };
        value.clamp(1, hard_max)
    };
    let wire = match actuator_id {
        "iphone.haptic" => {
            let allowed = haptic_magnitude_rank(e.magnitude.unwrap_or(0.5));
            let style = match p.get("style").and_then(Value::as_str) {
                Some(requested) => {
                    let rank = haptic_style_rank(requested)
                        .ok_or_else(|| format!("haptic.pulse: unknown style `{requested}`"))?;
                    if rank <= allowed {
                        requested.to_string()
                    } else {
                        // extra 不得放大 policy 已 clamp 的 magnitude。
                        haptic_rank_style(allowed).to_string()
                    }
                }
                None => haptic_rank_style(allowed).to_string(),
            };
            p.insert("style".into(), json!(style));
            let count = p
                .get("count")
                .and_then(Value::as_f64)
                .map(|c| c.round() as i64)
                .unwrap_or(1)
                .clamp(1, 5);
            p.insert("count".into(), json!(count));
            "haptic.pulse"
        }
        "iphone.notify" => {
            p.entry("title")
                .or_insert(json!(character_title.unwrap_or("角色")));
            if !p.contains_key("body") {
                let Some(msg) = message else {
                    return Err("notify.show needs a message (or extra.body)".into());
                };
                p.insert("body".into(), json!(msg));
            }
            "notify.show"
        }
        "iphone.tts" => {
            if !p.contains_key("text") {
                let Some(msg) = message else {
                    return Err("tts.speak needs a message (or extra.text)".into());
                };
                p.insert("text".into(), json!(msg));
            }
            "tts.speak"
        }
        "iphone.torch" => {
            let requested_on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
            // policy 把 magnitude clamp 成 0 ＝「不得點亮」；extra.on 不得推翻。
            let on = match e.magnitude {
                Some(m) => requested_on && m > 0.0,
                None => requested_on,
            };
            p.insert("on".into(), json!(on));
            let d = duration_from(&p, 1_000, 5_000);
            p.insert("durationMs".into(), json!(d));
            "torch.set"
        }
        "iphone.flash" => {
            p.entry("color").or_insert(json!("#FFB347"));
            let d = duration_from(&p, 400, 1_500);
            p.insert("durationMs".into(), json!(d));
            "screen.flash"
        }
        "iphone.character" => {
            let state = match p.get("state") {
                Some(v) => {
                    let s = v.as_str().unwrap_or_default();
                    if !CHARACTER_STATES.contains(&s) {
                        return Err(format!(
                            "character.present: state `{s}` not allowed (idle/working/waiting/failed/unknown/emergency)"
                        ));
                    }
                    if s == VERIFIED_STATE {
                        // 誠實階梯：completed ≠ verified。任何 plan／agent 路徑
                        // （含 extra.state）都不得讓手機顯示綠勾。
                        return Err(format!(
                            "character.present: `{VERIFIED_STATE}` is human-verification only; it can never be requested through a plan"
                        ));
                    }
                    s.to_string()
                }
                // 從 message 推導絕不產生 verified-success。
                None => message
                    .filter(|m| CHARACTER_STATES.contains(m) && *m != VERIFIED_STATE)
                    .unwrap_or("idle")
                    .to_string(),
            };
            p.insert("state".into(), json!(state));
            "character.present"
        }
        other => return Err(format!("unknown mobile actuator {other}")),
    };
    Ok((wire, Value::Object(p)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    pub device_id: String,
    pub name: String,
    #[serde(default)]
    pub model: String,
    /// SHA-256(token)——明文只在配對回覆傳給手機一次。
    pub token_hash: String,
    pub paired_at: chrono::DateTime<Utc>,
}

struct PairingSession {
    code: String,
    expires_at: chrono::DateTime<Utc>,
}

struct ConnState {
    /// 每條連線唯一序號：收尾時只移除自己的表項（重連後的新連線不受影響）。
    conn_id: u64,
    outbound: mpsc::Sender<Message>,
    /// 手機最新自報狀態（sensors/permissions），UI 誠實顯示。
    status: Value,
    /// 手機自報「麥克風音量串流中」的起算時間（感測不靜默：status／tray／
    /// 首頁／角色視窗都以此顯示 activeSensors）。None＝沒在串流。
    mic_since: Option<chrono::DateTime<Utc>>,
    /// 撤銷／被新連線取代時觸發 → handler 立即收尾關閉。
    close: CancellationToken,
}

struct PendingAct {
    /// act 送往哪台手機：只有同一台的 ack/err 能解除。
    device_id: String,
    reply: oneshot::Sender<Value>,
}

pub struct MobileBridge {
    /// 測試模式（Runtime 無 watchdog）不得把 Bonjour 服務記錄廣播到實體區網——
    /// 測試是模擬，不能有外部副作用；生產 daemon 才廣播。
    advertise_mdns: AtomicBool,
    started: AtomicBool,
    /// 序列化啟動：`started` 只在 bind 等全部成功後才設，失敗可重試。
    start_lock: Mutex<()>,
    port: RwLock<Option<u16>>,
    fingerprint: RwLock<Option<String>>,
    pairing: Mutex<Option<PairingSession>>,
    devices: RwLock<BTreeMap<String, PairedDevice>>,
    conns: RwLock<BTreeMap<String, ConnState>>,
    conn_seq: AtomicU64,
    pending_acts: Mutex<BTreeMap<String, PendingAct>>,
    /// mdns daemon 保活。
    mdns: Mutex<Option<mdns_sd::ServiceDaemon>>,
    /// Bonjour 註冊結果（誠實：失敗要看得到）。
    bonjour: RwLock<Value>,
    /// 被丟棄的觀察（白名單外 receptor／無可接受鍵／estop／缺 consent／
    /// ingest 失敗）。
    dropped_observations: AtomicU64,
    /// (上次廣播 stop-all 的時間, 那一則是否含 sensors)。去重只在「同級或
    /// 更弱」時成立——已送過 sensors:false 不得吃掉後來的 sensors:true。
    last_stop_all: Mutex<Option<(Instant, bool)>>,
    /// 配對期被區網未認證 peer 燒掉的時間（UI 可據此告訴使用者重新開始）。
    pairing_burned_at: RwLock<Option<chrono::DateTime<Utc>>>,
    /// 已為「缺 consent」記過 audit 的 receptor（一次性，不洗版 audit log）。
    consent_audited: Mutex<std::collections::BTreeSet<String>>,
    ping_interval_ms: AtomicU64,
    idle_timeout_ms: AtomicU64,
    /// 目前桌面角色的顯示名（Character Protocol hello 更新；notify 標題用）。
    character_title: std::sync::Mutex<Option<String>>,
}

impl MobileBridge {
    /// Character Protocol：角色協商完成後更新 notify 預設標題。
    pub fn set_character_title(&self, title: String) {
        let mut guard = self
            .character_title
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(title);
    }

    pub fn character_title(&self) -> Option<String> {
        self.character_title
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            advertise_mdns: AtomicBool::new(true),
            started: AtomicBool::new(false),
            start_lock: Mutex::new(()),
            port: RwLock::new(None),
            fingerprint: RwLock::new(None),
            pairing: Mutex::new(None),
            devices: RwLock::new(BTreeMap::new()),
            conns: RwLock::new(BTreeMap::new()),
            conn_seq: AtomicU64::new(1),
            pending_acts: Mutex::new(BTreeMap::new()),
            mdns: Mutex::new(None),
            bonjour: RwLock::new(json!({
                "advertised": false,
                "service": MDNS_SERVICE_SHORT,
                "instance": Value::Null,
                "error": Value::Null,
            })),
            dropped_observations: AtomicU64::new(0),
            last_stop_all: Mutex::new(None),
            pairing_burned_at: RwLock::new(None),
            consent_audited: Mutex::new(std::collections::BTreeSet::new()),
            ping_interval_ms: AtomicU64::new(PING_INTERVAL_DEFAULT_MS),
            idle_timeout_ms: AtomicU64::new(IDLE_TIMEOUT_DEFAULT_MS),
            character_title: std::sync::Mutex::new(None),
        })
    }

    pub async fn any_connected(&self) -> bool {
        !self.conns.read().await.is_empty()
    }

    /// 覆寫心跳／閒置逾時（新連線起算；測試用短值）。
    /// 是否對外廣播 Bonjour（測試模式關閉；status.bonjour 誠實回報 disabled）。
    pub fn set_advertise_mdns(&self, on: bool) {
        self.advertise_mdns.store(on, Ordering::SeqCst);
    }

    pub fn set_timeouts(&self, ping_interval: Duration, idle_timeout: Duration) {
        self.ping_interval_ms
            .store(ping_interval.as_millis().max(1) as u64, Ordering::SeqCst);
        self.idle_timeout_ms
            .store(idle_timeout.as_millis().max(1) as u64, Ordering::SeqCst);
    }

    fn ping_interval(&self) -> Duration {
        Duration::from_millis(self.ping_interval_ms.load(Ordering::SeqCst))
    }

    fn idle_timeout(&self) -> Duration {
        Duration::from_millis(self.idle_timeout_ms.load(Ordering::SeqCst))
    }

    pub fn dropped_observations(&self) -> u64 {
        self.dropped_observations.load(Ordering::SeqCst)
    }

    fn note_dropped(&self, receptor: &str, why: &str) {
        self.dropped_observations.fetch_add(1, Ordering::SeqCst);
        tracing::debug!(receptor, why, "mobile observation dropped");
    }

    /// 挑一台已連線手機（目前：第一台）。
    async fn pick_conn(&self) -> Result<(String, mpsc::Sender<Message>), String> {
        let conns = self.conns.read().await;
        conns
            .iter()
            .next()
            .map(|(id, c)| (id.clone(), c.outbound.clone()))
            .ok_or_else(|| "no iPhone connected".to_string())
    }

    /// 廣播給所有手機；回傳成功排入佇列的連線數。
    async fn broadcast(&self, text: String) -> usize {
        let conns = self.conns.read().await;
        let mut delivered = 0;
        for conn in conns.values() {
            if conn
                .outbound
                .send_timeout(Message::Text(text.clone()), Duration::from_millis(300))
                .await
                .is_ok()
            {
                delivered += 1;
            }
        }
        delivered
    }

    /// 送一則需要回覆的訊息並登記 pending（綁定目標手機）。
    async fn dispatch(&self, id: &str, msg: Value) -> Result<oneshot::Receiver<Value>, String> {
        let (device_id, outbound) = self.pick_conn().await?;
        let (tx, rx) = oneshot::channel();
        self.pending_acts.lock().await.insert(
            id.to_string(),
            PendingAct {
                device_id,
                reply: tx,
            },
        );
        if outbound
            .send_timeout(Message::Text(msg.to_string()), Duration::from_millis(500))
            .await
            .is_err()
        {
            self.pending_acts.lock().await.remove(id);
            return Err("iPhone outbound queue full or closed".into());
        }
        Ok(rx)
    }

    /// 等回覆；逾時或通道關閉一律移除 pending（不洩漏）。
    async fn await_reply(
        &self,
        id: &str,
        rx: oneshot::Receiver<Value>,
        timeout: Duration,
    ) -> Option<Value> {
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => Some(reply),
            _ => {
                self.pending_acts.lock().await.remove(id);
                None
            }
        }
    }

    /// 送 act 並等 ack（逾時＝結果未知；絕不重送）。
    pub async fn act(&self, name: &str, params: Value, action_id: &str) -> Result<Value, String> {
        let msg = json!({"type":"act","id":action_id,"name":name,"params":params});
        let rx = self.dispatch(action_id, msg).await?;
        self.await_reply(action_id, rx, ACT_TIMEOUT)
            .await
            .ok_or_else(|| format!("no ack for {action_id} — outcome UNKNOWN (not retried)"))
    }

    /// 只有「同一台手機」對「同一 id」的回覆能解除 pending。
    async fn resolve_pending(&self, device_id: &str, reply: &Value) {
        let Some(id) = reply["id"].as_str() else {
            return;
        };
        let mut pending = self.pending_acts.lock().await;
        let owned = pending
            .get(id)
            .map(|p| p.device_id == device_id)
            .unwrap_or(false);
        if !owned {
            tracing::debug!(
                id,
                device_id,
                "reply for unknown or foreign pending act ignored"
            );
            return;
        }
        if let Some(p) = pending.remove(id) {
            let _ = p.reply.send(reply.clone());
        }
    }

    /// 讓所有在途 act 立即以指定回覆收場（estop）。
    async fn fail_all_pending(&self, reply: Value) -> usize {
        let drained = std::mem::take(&mut *self.pending_acts.lock().await);
        let n = drained.len();
        for (_, p) in drained {
            let _ = p.reply.send(reply.clone());
        }
        n
    }

    /// estop：在途 act 立即以 stopped 收場；500ms 內只廣播一次 stop-all；
    /// 沒有手機連線 → Err（誠實：沒有任何東西被停，不計入 stoppedActuators）。
    pub async fn stop_all(&self) -> Result<(), ActuatorError> {
        self.stop_all_inner(false).await
    }

    /// 緊急停止專用：stop-all **連同感測**（手機端停 mic／位置／BLE 閘道）。
    /// 去重不得讓先送出的 `sensors:false` 吃掉這一則。
    pub async fn stop_all_with_sensors(&self) -> Result<(), ActuatorError> {
        self.stop_all_inner(true).await
    }

    async fn stop_all_inner(&self, sensors: bool) -> Result<(), ActuatorError> {
        self.fail_all_pending(json!({
            "type": "err",
            "reason": "stopped",
            "stopAll": true,
        }))
        .await;
        if !self.any_connected().await {
            return Err(ActuatorError::Unavailable(
                "no iPhone connected — stop-all not delivered".into(),
            ));
        }
        let mut last = self.last_stop_all.lock().await;
        // 去重只在「已送過的那則至少和這則一樣強」時成立。
        if last
            .map(|(at, had_sensors)| at.elapsed() < STOP_ALL_DEDUP && (had_sensors || !sensors))
            .unwrap_or(false)
        {
            return Ok(());
        }
        let delivered = self
            .broadcast(json!({"type":"stop-all","sensors":sensors}).to_string())
            .await;
        *last = Some((Instant::now(), sensors));
        if delivered == 0 {
            return Err(ActuatorError::Unavailable(
                "stop-all could not be queued to any iPhone".into(),
            ));
        }
        Ok(())
    }

    /// 手機自報「麥克風音量串流中」的連線（deviceId, 起算時間）。
    async fn mic_streaming_devices(&self) -> Vec<(String, chrono::DateTime<Utc>)> {
        self.conns
            .read()
            .await
            .iter()
            .filter_map(|(id, c)| c.mic_since.map(|since| (id.clone(), since)))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Actuator：手機動器（policy/consent/receipt 全走既有管線）
// ---------------------------------------------------------------------------

pub struct MobileActuator {
    id: &'static str,
    channel: &'static str,
    display: &'static str,
    bridge: Arc<MobileBridge>,
}

#[async_trait::async_trait]
impl Actuator for MobileActuator {
    fn manifest(&self) -> interaction_core::ActuatorManifest {
        use interaction_core::{ConfirmationLevel, EffectSemantics, TriState};
        ActuatorManifestBuilder::new(self.id, self.display, self.channel, "mobile.iphone")
            .description("透過已配對 iPhone 執行（需 iPhone 連線＋雙端授權）")
            .risk(RiskClass::BoundedSideEffect)
            .external(true)
            .requires_consent(true)
            .human(interaction_core::HumanMeta {
                effect: Some(EffectSemantics {
                    confirmation_level: ConfirmationLevel::Acknowledged,
                    external_side_effect: TriState::Yes,
                    physical_effect: if self.id == "iphone.haptic" || self.id == "iphone.torch" {
                        TriState::Yes
                    } else {
                        TriState::No
                    },
                    reversible: TriState::Yes,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        if action.expires_at <= Utc::now() {
            return Err(ActuatorError::Rejected("action expired".into()));
        }
        // 參數：policy-bounded effective 值 → App 驗證的 wire 形狀（手機端仍有硬限制）。
        let title = self.bridge.character_title();
        let (wire_name, params) =
            map_wire_params_titled(self.id, &action.effective, title.as_deref())
                .map_err(ActuatorError::Rejected)?;
        match self
            .bridge
            .act(wire_name, params, action.action_id.as_str())
            .await
        {
            Ok(reply) if reply["type"] == "ack" => {
                let mut receipt = DriverReceipt::start(&action, Utc::now())
                    .dispatched()
                    .note("transport", json!("mobile-wss"))
                    .acknowledged();
                if let Some(applied) = reply.get("applied") {
                    receipt = receipt.note("deviceApplied", applied.clone());
                }
                Ok(receipt.finish())
            }
            Ok(reply) if reply["stopAll"] == true => Ok(DriverReceipt::start(&action, Utc::now())
                .dispatched()
                .note("outcomeUnknown", json!(true))
                .failed(
                    "emergency-stopped",
                    "stop-all issued before iPhone acknowledged — effect unknown",
                )
                .finish()),
            Ok(reply) => Ok(DriverReceipt::start(&action, Utc::now())
                .dispatched()
                .failed(
                    "device-refused",
                    reply["reason"].as_str().unwrap_or("iPhone refused"),
                )
                .finish()),
            Err(e) if e.contains("UNKNOWN") => Ok(DriverReceipt::start(&action, Utc::now())
                .dispatched()
                .note("ackTimeout", json!(true))
                .finish()),
            Err(e) => Ok(DriverReceipt::start(&action, Utc::now())
                .failed("iphone-unreachable", &e)
                .finish()),
        }
    }

    async fn status(&self) -> ComponentHealth {
        if self.bridge.any_connected().await {
            ComponentHealth::healthy().at(Utc::now())
        } else {
            ComponentHealth::offline("iPhone 未連線").at(Utc::now())
        }
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(format!(
            "{action_id}: iPhone 動作為短效即時效果，無可取消的長工作"
        )))
    }

    /// 六個 mobile actuator 共用一條 stop-all（bridge 500ms 去重）；
    /// 沒有手機連線 → Err，runtime 的 stoppedActuators 不計入（誠實）。
    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        self.bridge.stop_all().await
    }
}

// ---------------------------------------------------------------------------
// Runtime 整合
// ---------------------------------------------------------------------------

fn token_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

impl Runtime {
    fn mobile_devices_path(&self) -> std::path::PathBuf {
        self.paths.home.join("state").join("mobile-devices.json")
    }

    /// 寫入配對裝置清單。**寫檔失敗必須被呼叫端看見**：撤銷若只改了記憶體，
    /// 重啟後 token 會復活（等於撤銷沒發生）。
    async fn mobile_persist_devices(&self) -> Result<(), String> {
        let devices: Vec<PairedDevice> =
            self.mobile.devices.read().await.values().cloned().collect();
        let path = self.mobile_devices_path();
        let text = serde_json::to_string_pretty(&json!({ "devices": devices }))
            .map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub async fn mobile_load_devices(&self) {
        let path = self.mobile_devices_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            if let Ok(devices) =
                serde_json::from_value::<Vec<PairedDevice>>(value["devices"].clone())
            {
                let mut map = self.mobile.devices.write().await;
                for d in devices {
                    map.insert(d.device_id.clone(), d);
                }
            }
        }
    }

    /// 憑證：state/mobile-cert.pem（自簽、首次產生、指紋供 QR 釘選）。
    fn mobile_cert(&self) -> Result<(Vec<u8>, Vec<u8>, String), String> {
        let dir = self.paths.home.join("state");
        let cert_path = dir.join("mobile-cert.der");
        let key_path = dir.join("mobile-key.der");
        if let (Ok(cert), Ok(key)) = (std::fs::read(&cert_path), std::fs::read(&key_path)) {
            let fp = sha256_hex(&cert);
            return Ok((cert, key, fp));
        }
        let mut params = rcgen::CertificateParams::new(vec!["interact-ai.local".into()])
            .map_err(|e| e.to_string())?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        let keypair = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
        let cert = params.self_signed(&keypair).map_err(|e| e.to_string())?;
        let cert_der = cert.der().to_vec();
        let key_der = keypair.serialize_der();
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(&cert_path, &cert_der).map_err(|e| e.to_string())?;
        std::fs::write(&key_path, &key_der).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
        }
        let fp = sha256_hex(&cert_der);
        Ok((cert_der, key_der, fp))
    }

    /// 目前仍有配對裝置 → daemon/桌面啟動時自動起 mobile 伺服器（讓 iPhone
    /// 能重連）。沒配對過、或全部撤銷後（檔案存在但 devices 為空）就不開網路服務。
    pub fn mobile_autostart_if_paired(&self) {
        let has_devices = std::fs::read_to_string(self.mobile_devices_path())
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|v| v["devices"].as_array().map(|a| !a.is_empty()))
            .unwrap_or(false);
        if !has_devices {
            return;
        }
        let rt = self.clone();
        tokio::spawn(async move {
            if let Err(e) = rt.mobile_ensure_started().await {
                tracing::warn!(error = %e, "mobile autostart failed");
            }
        });
    }

    /// 啟動 mobile 伺服器（冪等）。`started` 只在全部步驟成功後才設；
    /// 任一步失敗回 Err 且下次呼叫會重試。
    pub async fn mobile_ensure_started(&self) -> DomainResult<Value> {
        let _start_guard = self.mobile.start_lock.lock().await;
        if self.mobile.started.load(Ordering::SeqCst) {
            return self.mobile_status().await;
        }
        // rustls 需要 process-level CryptoProvider（重複安裝無害）。
        let _ = rustls::crypto::ring::default_provider().install_default();
        self.mobile_load_devices().await;
        let (cert_der, key_der, fingerprint) = self
            .mobile_cert()
            .map_err(|e| DomainError::Internal(format!("mobile cert: {e}")))?;
        // TLS config。
        let certs = vec![rustls::pki_types::CertificateDer::from(cert_der)];
        let key = rustls::pki_types::PrivateKeyDer::try_from(key_der)
            .map_err(|e| DomainError::Internal(format!("mobile key: {e}")))?;
        let tls_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| DomainError::Internal(format!("tls config: {e}")))?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls_config));
        // Bind：預設埠，被占用退 ephemeral（誠實回報實際埠）。
        let listener = match tokio::net::TcpListener::bind(("0.0.0.0", MOBILE_PORT_DEFAULT)).await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind(("0.0.0.0", 0))
                .await
                .map_err(|e| DomainError::Internal(format!("mobile bind: {e}")))?,
        };
        let port = listener
            .local_addr()
            .map_err(|e| DomainError::Internal(e.to_string()))?
            .port();
        *self.mobile.port.write().await = Some(port);
        *self.mobile.fingerprint.write().await = Some(fingerprint.clone());

        // Bonjour 廣播（失敗不致命：手動輸入 host:port 仍可配對；但要看得到）。
        let instance = mdns_instance_name(port);
        let bonjour = if !self.mobile.advertise_mdns.load(Ordering::SeqCst) {
            json!({
                "advertised": false,
                "service": MDNS_SERVICE_SHORT,
                "instance": instance,
                "error": "disabled (test mode: no LAN side effects)",
            })
        } else {
            match mdns_sd::ServiceDaemon::new() {
                Ok(daemon) => {
                    let props = [("fp", fingerprint.as_str()), ("v", "1")];
                    let host = format!("{instance}.local.");
                    let registered = mdns_sd::ServiceInfo::new(
                        MDNS_SERVICE_TYPE,
                        &instance,
                        &host,
                        (),
                        port,
                        &props[..],
                    )
                    .map_err(|e| e.to_string())
                    .and_then(|info| {
                        daemon
                            .register(info.enable_addr_auto())
                            .map_err(|e| e.to_string())
                    });
                    match registered {
                        Ok(()) => {
                            *self.mobile.mdns.lock().await = Some(daemon);
                            json!({
                                "advertised": true,
                                "service": MDNS_SERVICE_SHORT,
                                "instance": instance,
                                "error": Value::Null,
                            })
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                service = MDNS_SERVICE_TYPE,
                                "Bonjour registration failed — pair via QR / manual host:port"
                            );
                            let _ = daemon.shutdown();
                            json!({
                                "advertised": false,
                                "service": MDNS_SERVICE_SHORT,
                                "instance": instance,
                                "error": e,
                            })
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "mDNS daemon unavailable — Bonjour not advertised");
                    json!({
                        "advertised": false,
                        "service": MDNS_SERVICE_SHORT,
                        "instance": instance,
                        "error": e.to_string(),
                    })
                }
            }
        };
        *self.mobile.bonjour.write().await = bonjour;

        // 註冊 receptors（push；健康度跟連線走）＋actuators。
        self.mobile_register_capabilities().await?;
        self.store
            .audit("mobile.server-started", "runtime", &json!({"port": port}))?;

        // Accept loop（只有走到這裡才算啟動成功）。
        let rt = self.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else {
                    break;
                };
                let acceptor = acceptor.clone();
                let rt = rt.clone();
                tokio::spawn(async move {
                    match acceptor.accept(stream).await {
                        Ok(tls) => match tokio_tungstenite::accept_async(tls).await {
                            Ok(ws) => rt.mobile_handle_conn(ws, peer).await,
                            Err(e) => tracing::debug!(error = %e, "mobile ws accept failed"),
                        },
                        Err(e) => tracing::debug!(error = %e, "mobile tls accept failed"),
                    }
                });
            }
        });
        self.mobile.started.store(true, Ordering::SeqCst);
        self.mobile_status().await
    }

    async fn mobile_register_capabilities(&self) -> DomainResult<()> {
        let bridge = self.mobile.clone();
        let health = {
            let bridge = bridge.clone();
            Arc::new(move || {
                // 同步視角：用 try_read 誠實回報（拿不到鎖＝未知→offline 保守）。
                let connected = bridge
                    .conns
                    .try_read()
                    .map(|c| !c.is_empty())
                    .unwrap_or(false);
                if connected {
                    ComponentHealth::healthy().at(Utc::now())
                } else {
                    ComponentHealth::offline("iPhone 未連線").at(Utc::now())
                }
            }) as Arc<dyn Fn() -> ComponentHealth + Send + Sync>
        };
        for spec in MOBILE_RECEPTOR_SPECS {
            let mut builder = ReceptorManifestBuilder::new(spec.id, spec.name, "mobile.iphone")
                .description("由已配對 iPhone 推送的語意事件（原始軌跡不離開手機）")
                .category("mobile")
                .provides(spec.provides)
                .mode(interaction_core::ReceptorMode::Event)
                .sensitivity(spec.sensitivity, spec.requires_consent);
            if spec.high_risk {
                // 高風險受器：正式宣告 `retention: none` —— runtime `ingest` 據此
                // 不把觀察寫進 store（只即時發事件），與麥克風音量同一規則。
                use interaction_core::{DataRetention, DataSemantics, DataSource, TriState};
                builder = builder.human(interaction_core::HumanMeta {
                    data: Some(DataSemantics {
                        data_categories: vec!["ambient-sound-level".into()],
                        personal_data: TriState::Yes,
                        source: DataSource::Device,
                        leaves_device: TriState::No,
                        retention: DataRetention::None,
                        fact_fields: spec.provides.iter().map(|s| s.to_string()).collect(),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            }
            let receptor =
                adapters_builtin::PushReceptor::with_health(builder.build(), health.clone());
            if self
                .registry
                .register_receptor(receptor.clone())
                .await
                .is_ok()
            {
                self.register_dynamic_push(spec.id, receptor).await;
            }
        }
        for (id, channel, display) in MOBILE_ACTUATORS {
            let actuator = Arc::new(MobileActuator {
                id,
                channel,
                display,
                bridge: bridge.clone(),
            });
            let _ = self.registry.register_actuator(actuator).await;
        }
        Ok(())
    }

    /// 一段配對期（5 分鐘、單一使用）。回傳 code＋QR payload＋SVG。
    pub async fn mobile_pairing_begin(&self) -> DomainResult<Value> {
        self.mobile_ensure_started().await?;
        let code = format!("{:06}", rand::thread_rng().next_u32() % 1_000_000);
        let expires_at = Utc::now() + chrono::Duration::seconds(PAIRING_TTL_SECS);
        *self.mobile.pairing.lock().await = Some(PairingSession {
            code: code.clone(),
            expires_at,
        });
        // 新的一段配對期：上一次「被別人燒掉」的提示歸零。
        *self.mobile.pairing_burned_at.write().await = None;
        let port = self.mobile.port.read().await.unwrap_or(MOBILE_PORT_DEFAULT);
        let fp = self
            .mobile
            .fingerprint
            .read()
            .await
            .clone()
            .unwrap_or_default();
        let host = local_lan_ip().unwrap_or_else(|| "<此電腦的區網 IP>".into());
        let payload = json!({"v":1,"host":host,"port":port,"fp":fp,"code":code}).to_string();
        let svg = qrcode::QrCode::new(payload.as_bytes())
            .map(|qr| {
                qr.render::<qrcode::render::svg::Color>()
                    .min_dimensions(220, 220)
                    .build()
            })
            .unwrap_or_default();
        self.store
            .audit("mobile.pairing-session", "user", &json!({"port": port}))?;
        Ok(json!({
            "code": code,
            "expiresAt": expires_at,
            "payload": payload,
            "qrSvg": svg,
            "port": port,
            "fingerprint": fp,
        }))
    }

    pub async fn mobile_status(&self) -> DomainResult<Value> {
        let conns = self.mobile.conns.read().await;
        let devices: Vec<Value> = self
            .mobile
            .devices
            .read()
            .await
            .values()
            .map(|d| {
                let conn = conns.get(&d.device_id);
                let status = conn.map(|c| c.status.clone()).unwrap_or(Value::Null);
                json!({
                    "deviceId": d.device_id,
                    "name": d.name,
                    "model": d.model,
                    "pairedAt": d.paired_at,
                    "connected": conn.is_some(),
                    // 手機自報（桌面 Consent 不取代 iOS 系統權限）。
                    "sensors": status.get("sensors").cloned().unwrap_or(Value::Null),
                    "permissions": status.get("permissions").cloned().unwrap_or(Value::Null),
                    "status": status,
                })
            })
            .collect();
        drop(conns);
        Ok(json!({
            "started": self.mobile.started.load(Ordering::SeqCst),
            "port": *self.mobile.port.read().await,
            "fingerprint": *self.mobile.fingerprint.read().await,
            "pairingActive": self.mobile.pairing.lock().await.as_ref()
                .map(|p| p.expires_at > Utc::now()).unwrap_or(false),
            "bonjour": self.mobile.bonjour.read().await.clone(),
            "heartbeat": {
                "pingIntervalMs": self.mobile.ping_interval_ms.load(Ordering::SeqCst),
                "idleTimeoutMs": self.mobile.idle_timeout_ms.load(Ordering::SeqCst),
            },
            "droppedObservations": self.mobile.dropped_observations(),
            // 配對期被未認證 peer 燒掉的時間（null＝沒發生過）。
            "pairingBurnedAt": *self.mobile.pairing_burned_at.read().await,
            "pendingActs": self.mobile.pending_acts.lock().await.len(),
            "devices": devices,
        }))
    }

    /// 撤銷：移除裝置、關閉現有連線（handler 送 `auth-fail(revoked)` 後收尾）、
    /// provider → Revoked、高風險受器強制 disabled。
    pub async fn mobile_revoke(&self, device_id: &str) -> DomainResult<Value> {
        let removed = self.mobile.devices.write().await.remove(device_id);
        let Some(removed) = removed else {
            return Err(DomainError::NotFound(format!("mobile device {device_id}")));
        };
        // 落地失敗＝撤銷沒有真的發生（重啟後 token 會復活）。誠實回 Err
        // 並把裝置放回記憶體表，不留下「UI 說撤銷了、其實沒有」的假象。
        if let Err(e) = self.mobile_persist_devices().await {
            // 重試一次（暫時性錯誤，例如目錄剛好被換掉）。
            if let Err(e2) = self.mobile_persist_devices().await {
                self.mobile
                    .devices
                    .write()
                    .await
                    .insert(device_id.to_string(), removed);
                self.store
                    .audit(
                        "mobile.revoke-failed",
                        "user",
                        &json!({"deviceId": device_id, "error": e2}),
                    )
                    .ok();
                return Err(DomainError::Internal(format!(
                    "revoking {device_id} could not be persisted ({e}; retry: {e2}); the device is still paired — fix the state directory and try again"
                )));
            }
        }
        // 立即斷線：取消 token → handler 立刻送 auth-fail 並關閉 socket。
        let conn = self.mobile.conns.write().await.remove(device_id);
        let was_connected = conn.is_some();
        if let Some(conn) = conn {
            conn.close.cancel();
        }
        let pid = ProviderId::new(format!("provider.mobile.{device_id}"));
        if let Err(e) = self
            .providers
            .transition(&pid, ProviderState::Revoked, Some("revoked by user".into()))
            .await
        {
            tracing::debug!(error = %e, device_id, "mobile provider revoke transition skipped");
        } else {
            self.character_project_provider(&pid, ProviderState::Revoked);
        }
        self.mobile_disable_high_risk_receptors(device_id, "revoked")
            .await;
        self.store.audit(
            "mobile.device-revoked",
            "user",
            &json!({"deviceId": device_id, "wasConnected": was_connected}),
        )?;
        Ok(json!({"revoked": device_id, "wasConnected": was_connected}))
    }

    /// 高風險受器不自動恢復：手機有效連線消失（斷線／撤銷）即由桌面端強制
    /// disabled，並留 audit；重連後要人類重新啟用。
    async fn mobile_disable_high_risk_receptors(&self, device_id: &str, reason: &str) {
        for spec in MOBILE_RECEPTOR_SPECS.iter().filter(|s| s.high_risk) {
            let id = ReceptorId::new(spec.id);
            // 已 disabled（Unavailable）或未註冊（NotFound）都不需動作。
            if self.registry.receptor(&id).await.is_err() {
                continue;
            }
            if self.registry.set_receptor_enabled(&id, false).await.is_ok() {
                self.store
                    .audit(
                        "mobile.high-risk-receptor-disabled",
                        "runtime",
                        &json!({
                            "receptorId": spec.id,
                            "deviceId": device_id,
                            "reason": reason,
                        }),
                    )
                    .ok();
            }
        }
    }

    /// consent-gated 受器：目前 session 是否對這個 receptor 有有效 consent。
    /// 撤銷／到期／沒有 session ＝ 沒有 consent（之後的觀察一律丟棄）。
    async fn mobile_receptor_consented(&self, receptor: &str) -> bool {
        let now = Utc::now();
        match self.current_session().await {
            Some(session) if session.is_active(now) => session.has_consent(
                &interaction_core::ConsentScope::Receptor(receptor.to_string()),
                now,
            ),
            _ => false,
        }
    }

    /// 缺 consent 的丟棄只記一次 audit（不洗版），但一定要留痕。
    async fn mobile_audit_missing_consent(&self, receptor: &str) {
        let first = {
            let mut seen = self.mobile.consent_audited.lock().await;
            seen.insert(receptor.to_string())
        };
        if first {
            self.store
                .audit(
                    "mobile.observation-without-consent",
                    "runtime",
                    &json!({
                        "receptorId": receptor,
                        "hint": format!("grant receptor:{receptor} in the current session first"),
                    }),
                )
                .ok();
        }
    }

    /// 緊急停止 → 手機端也必須停止「感測」，不只是動器。
    /// (a) 桌面端把高風險受器強制 disabled（重啟／重連不自動恢復）；
    /// (b) 對所有手機送 `stop-all { sensors: true }`（App 據此關 mic／位置／
    ///     BLE 閘道）。沒有手機連線時什麼都不做（誠實：沒有東西被停）。
    pub(crate) async fn mobile_estop_stop_sensors(&self, actor: &str) {
        self.mobile_disable_high_risk_receptors("*", "emergency-stop")
            .await;
        if !self.mobile.any_connected().await {
            return;
        }
        let outcome = self.mobile.stop_all_with_sensors().await;
        self.store
            .audit(
                "mobile.estop-stop-sensors",
                actor,
                &json!({
                    "sensors": true,
                    "delivered": outcome.is_ok(),
                    "error": outcome.as_ref().err().map(|e| e.to_string()),
                }),
            )
            .ok();
    }

    /// 感測不靜默：手機端正在串流的高風險感測也要出現在 `status.activeSensors`
    /// （tray／首頁／角色視窗都吃這個欄位）。條件三者皆須成立：
    /// receptor 啟用中 ∧ 手機連線中 ∧ 手機自報 `sensors.micLevel == true`。
    pub(crate) async fn mobile_active_sensors(&self) -> Vec<crate::sensors::SensorUse> {
        // registry 對 disabled／未註冊的 receptor 回 Err —— 這就是「啟用中」。
        if self
            .registry
            .receptor(&ReceptorId::new("iphone.mic-level"))
            .await
            .is_err()
        {
            return Vec::new();
        }
        self.mobile
            .mic_streaming_devices()
            .await
            .into_iter()
            .map(|(device_id, since)| crate::sensors::SensorUse {
                kind: "iphone.mic-level".into(),
                started_at: since,
                started_by: format!("iphone:{device_id}"),
                purpose: "iPhone 麥克風音量（僅音量值）".into(),
                auto_stop_at: None,
            })
            .collect()
    }

    /// **人工驗證專用**：讓已配對 iPhone 顯示綠勾（`verified-success`）。
    /// 這是唯一能送出該狀態的路徑——不經 plan／policy／AI
    /// （`map_wire_params` 對 `verified-success` 一律 Rejected）。
    /// 沒有手機連線／手機沒 ack 都誠實回 Err，不重送。
    pub async fn mobile_present_verified(&self, agent_session_id: &str) -> DomainResult<Value> {
        if self.is_estopped() {
            return Err(DomainError::PolicyBlocked(
                "emergency stop engaged; nothing is sent to the iPhone".into(),
            ));
        }
        if !self.mobile.any_connected().await {
            return Err(DomainError::Unavailable("no iPhone connected".into()));
        }
        let id = format!("verified-{}", token_hex(4));
        let params = json!({
            "state": VERIFIED_STATE,
            "source": "human-verification",
            "agentSessionId": agent_session_id,
        });
        let reply = self
            .mobile
            .act("character.present", params, &id)
            .await
            .map_err(DomainError::Unavailable)?;
        self.store.audit(
            "mobile.present-verified",
            "user",
            &json!({"agentSessionId": agent_session_id, "reply": reply["type"]}),
        )?;
        Ok(reply)
    }

    async fn mobile_register_provider(&self, device: &PairedDevice) {
        let descriptor = ProviderDescriptor {
            identity: ProviderIdentity {
                id: ProviderId::new(format!("provider.mobile.{}", device.device_id)),
                kind: ProviderKind::Device,
                display_name: format!("iPhone：{}", device.name),
                trust_level: TrustLevel::Paired,
                origin: "mobile-wss".into(),
                version: String::new(),
                fingerprint: Some(device.token_hash.clone()),
                human: None,
            },
            state: ProviderState::Available,
            receptors: MOBILE_RECEPTORS.iter().map(|s| s.to_string()).collect(),
            actuators: MOBILE_ACTUATORS
                .iter()
                .map(|(id, _, _)| id.to_string())
                .collect(),
            tool_operations: vec![],
            paired_at: Some(device.paired_at),
            last_seen: Some(Utc::now()),
            detail: None,
        };
        let _ = self.providers.register(descriptor).await;
        let pid = ProviderId::new(format!("provider.mobile.{}", device.device_id));
        let _ = self
            .providers
            .transition(&pid, ProviderState::Available, Some("connected".into()))
            .await;
        // Character Protocol §11：iPhone 連上 → greet（device-online）。
        self.character_project_provider(&pid, ProviderState::Available);
    }

    /// 登記已認證連線；同一台手機的舊連線（多半半開）被新連線取代時踢掉，
    /// 避免舊 handler 收尾時誤刪新連線的表項。
    async fn mobile_attach_conn(
        &self,
        device_id: &str,
        conn_id: u64,
        out_tx: &mpsc::Sender<Message>,
        close: &CancellationToken,
    ) {
        let mut conns = self.mobile.conns.write().await;
        if let Some(old) = conns.insert(
            device_id.to_string(),
            ConnState {
                conn_id,
                outbound: out_tx.clone(),
                status: Value::Null,
                mic_since: None,
                close: close.clone(),
            },
        ) {
            old.close.cancel();
        }
    }

    async fn mobile_handle_conn<S>(
        &self,
        ws: tokio_tungstenite::WebSocketStream<S>,
        peer: std::net::SocketAddr,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (mut sink, mut stream) = ws.split();
        let mut authed: Option<String> = None; // deviceId
        let mut pending_pair: Option<(String, Value)> = None; // (nonce, request)
        let conn_id = self.mobile.conn_seq.fetch_add(1, Ordering::SeqCst);
        let close = CancellationToken::new();
        // 伺服器主動關閉的原因（撤銷／被新連線取代）；None＝對端斷線或閒置逾時。
        let mut closed_by_server: Option<&'static str> = None;
        let ping_interval = self.mobile.ping_interval();
        let idle_timeout = self.mobile.idle_timeout();
        let mut last_seen = Instant::now();

        // 專用 outbound queue（有界）。
        let (out_tx, mut out_rx) = mpsc::channel::<Message>(OUTBOUND_QUEUE);
        let writer = tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let send = |out_tx: &mpsc::Sender<Message>, v: Value| {
            let out_tx = out_tx.clone();
            async move {
                let _ = out_tx
                    .send_timeout(Message::Text(v.to_string()), Duration::from_millis(500))
                    .await;
            }
        };

        loop {
            let next = tokio::select! {
                biased;
                _ = close.cancelled() => {
                    // 撤銷 → 明確告知（iOS 收到 auth-fail 會停止自動重連）；
                    // 被同一台手機的新連線取代 → 靜默關閉（不能誤報成撤銷）。
                    let still_paired = match &authed {
                        Some(id) => self.mobile.devices.read().await.contains_key(id),
                        None => false,
                    };
                    if still_paired {
                        closed_by_server = Some("superseded");
                    } else {
                        closed_by_server = Some("revoked");
                        send(&out_tx, json!({"type":"auth-fail","reason":"revoked"})).await;
                    }
                    break;
                }
                next = tokio::time::timeout(ping_interval, stream.next()) => next,
            };
            let msg = match next {
                Ok(Some(Ok(msg))) => {
                    last_seen = Instant::now();
                    msg
                }
                Ok(Some(Err(e))) => {
                    tracing::debug!(error = %e, peer = %peer, "mobile ws read error");
                    break;
                }
                Ok(None) => break,
                Err(_elapsed) => {
                    // 心跳：無訊息一段時間送 Ping；完全無訊息（含 Pong）超過
                    // idle_timeout 視為半開連線，斷開（health 才會誠實轉 offline）。
                    if last_seen.elapsed() >= idle_timeout {
                        tracing::info!(
                            peer = %peer,
                            device = authed.as_deref().unwrap_or("-"),
                            idle_ms = last_seen.elapsed().as_millis() as u64,
                            "mobile connection idle — closing"
                        );
                        break;
                    }
                    let _ = out_tx
                        .send_timeout(Message::Ping(Vec::new()), Duration::from_millis(500))
                        .await;
                    continue;
                }
            };
            let text = match msg {
                Message::Text(text) => text,
                Message::Close(_) => break,
                _ => continue, // Ping/Pong/Binary：只當活著的證據
            };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            match v["type"].as_str() {
                Some("pair-request") if authed.is_none() => {
                    let session_ok = {
                        let pairing = self.mobile.pairing.lock().await;
                        pairing
                            .as_ref()
                            .map(|p| p.expires_at > Utc::now())
                            .unwrap_or(false)
                    };
                    if !session_ok {
                        send(&out_tx, json!({"type":"pair-fail","reason":"no active pairing session — start one from the desktop first"})).await;
                        continue;
                    }
                    let nonce = token_hex(16);
                    pending_pair = Some((nonce.clone(), v.clone()));
                    send(&out_tx, json!({"type":"pair-challenge","nonce":nonce})).await;
                }
                Some("pair-response") if authed.is_none() => {
                    let Some((nonce, request)) = pending_pair.take() else {
                        send(
                            &out_tx,
                            json!({"type":"pair-fail","reason":"no challenge outstanding"}),
                        )
                        .await;
                        continue;
                    };
                    let code = {
                        let mut pairing = self.mobile.pairing.lock().await;
                        match pairing.take() {
                            Some(p) if p.expires_at > Utc::now() => p.code,
                            _ => {
                                send(
                                    &out_tx,
                                    json!({"type":"pair-fail","reason":"pairing session expired"}),
                                )
                                .await;
                                continue;
                            }
                        }
                    };
                    let mut mac = HmacSha256::new_from_slice(code.as_bytes())
                        .expect("hmac accepts any key length");
                    mac.update(nonce.as_bytes());
                    let expected: String = mac
                        .finalize()
                        .into_bytes()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    let got = v["hmac"].as_str().unwrap_or_default();
                    if got != expected {
                        // 配對碼錯：拒絕且此段配對期作廢（不允許暴力嘗試）。
                        send(
                            &out_tx,
                            json!({"type":"pair-fail","reason":"wrong pairing code"}),
                        )
                        .await;
                        // 區網上任何 peer 都能用一次錯誤回應燒掉這段配對期：
                        // 防暴力的設計不變，但使用者必須看得到「有別的裝置
                        // 試過配對，請重新開始」——否則只會覺得配對莫名失敗。
                        let burned_at = Utc::now();
                        *self.mobile.pairing_burned_at.write().await = Some(burned_at);
                        self.store
                            .audit(
                                "mobile.pair-failed",
                                "runtime",
                                &json!({"peer": peer.to_string()}),
                            )
                            .ok();
                        self.store
                            .audit(
                                "mobile.pair-burned-by-peer",
                                "runtime",
                                &json!({"peer": peer.to_string(), "burnedAt": burned_at}),
                            )
                            .ok();
                        continue;
                    }
                    let device_id = format!("iphone-{}", &token_hex(8)[..8]);
                    let token = token_hex(32);
                    let device = PairedDevice {
                        device_id: device_id.clone(),
                        name: request["deviceName"]
                            .as_str()
                            .unwrap_or("iPhone")
                            .chars()
                            .take(24)
                            .collect(),
                        model: request["model"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(32)
                            .collect(),
                        token_hash: sha256_hex(token.as_bytes()),
                        paired_at: Utc::now(),
                    };
                    self.mobile
                        .devices
                        .write()
                        .await
                        .insert(device_id.clone(), device.clone());
                    if let Err(e) = self.mobile_persist_devices().await {
                        // 誠實：配對成功但沒落地 → 重啟後這台手機要重新配對。
                        tracing::warn!(error = %e, "paired device could not be persisted");
                        self.store
                            .audit(
                                "mobile.pair-not-persisted",
                                "runtime",
                                &json!({"deviceId": device_id, "error": e}),
                            )
                            .ok();
                    }
                    self.mobile_register_provider(&device).await;
                    self.mobile_attach_conn(&device_id, conn_id, &out_tx, &close)
                        .await;
                    authed = Some(device_id.clone());
                    self.store
                        .audit("mobile.paired", "user", &json!({"deviceId": device_id}))
                        .ok();
                    send(
                        &out_tx,
                        json!({"type":"paired","deviceId":device_id,"deviceToken":token}),
                    )
                    .await;
                }
                Some("auth") if authed.is_none() => {
                    let device_id = v["deviceId"].as_str().unwrap_or_default().to_string();
                    let token = v["token"].as_str().unwrap_or_default();
                    let ok = {
                        let devices = self.mobile.devices.read().await;
                        devices
                            .get(&device_id)
                            .map(|d| d.token_hash == sha256_hex(token.as_bytes()))
                            .unwrap_or(false)
                    };
                    if !ok {
                        send(&out_tx, json!({"type":"auth-fail","reason":"unknown device or bad token (possibly revoked)"})).await;
                        break;
                    }
                    let device = self.mobile.devices.read().await.get(&device_id).cloned();
                    if let Some(device) = device {
                        self.mobile_register_provider(&device).await;
                    }
                    self.mobile_attach_conn(&device_id, conn_id, &out_tx, &close)
                        .await;
                    authed = Some(device_id);
                    send(&out_tx, json!({"type":"auth-ok"})).await;
                }
                Some("observation") if authed.is_some() => {
                    let receptor = v["receptor"].as_str().unwrap_or_default();
                    // 緊急停止期間：高風險受器的觀察一律丟棄（手機的 stop-all
                    // 可能還在路上、或使用者的 App 還沒收到——桌面端不得繼續
                    // 收環境音量）。丟棄計入 droppedObservations。
                    if self.is_estopped()
                        && mobile_receptor_spec(receptor)
                            .map(|spec| spec.high_risk)
                            .unwrap_or(false)
                    {
                        self.mobile.note_dropped(receptor, "emergency stop engaged");
                        continue;
                    }
                    // consent-gated 受器：沒有目前 session 的明確 consent 就
                    // 丟棄（撤銷／session 到期之後的觀察一樣進不來）。
                    if mobile_receptor_spec(receptor)
                        .map(|spec| spec.requires_consent)
                        .unwrap_or(false)
                        && !self.mobile_receptor_consented(receptor).await
                    {
                        self.mobile.note_dropped(receptor, "no session consent");
                        self.mobile_audit_missing_consent(receptor).await;
                        continue;
                    }
                    match filter_mobile_facts(receptor, &v["facts"]) {
                        None => self
                            .mobile
                            .note_dropped(receptor, "receptor not in whitelist"),
                        Some(facts) if facts.is_empty() => {
                            self.mobile.note_dropped(receptor, "no accepted fact keys")
                        }
                        Some(facts) => {
                            if let Err(e) = self.ingest(receptor, facts, BTreeMap::new(), 1.0).await
                            {
                                self.mobile.note_dropped(receptor, &e.to_string());
                            }
                        }
                    }
                }
                // 回覆類：只有已認證且是 act 目標的那台手機才能解除 pending。
                Some("ack") | Some("err") | Some("ble.result") | Some("ble.value")
                    if authed.is_some() =>
                {
                    if let Some(device_id) = authed.as_deref() {
                        self.mobile.resolve_pending(device_id, &v).await;
                    }
                }
                Some("status") if authed.is_some() => {
                    if let Some(device_id) = &authed {
                        let mic_on = v["sensors"]["micLevel"] == json!(true);
                        if let Some(conn) = self.mobile.conns.write().await.get_mut(device_id) {
                            if conn.conn_id == conn_id {
                                conn.status = v.clone();
                                // 感測不靜默：手機自報麥克風串流中 → 桌面端
                                // status/tray/首頁/角色視窗都要看得到（起算
                                // 時間只在「關 → 開」時重設）。
                                conn.mic_since = match (mic_on, conn.mic_since) {
                                    (true, Some(since)) => Some(since),
                                    (true, None) => Some(Utc::now()),
                                    (false, _) => None,
                                };
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // 斷線收尾：只移除自己的表項；provider Disconnected（已 Revoked／被取代
        // 則不轉，避免非法轉移）；高風險受器強制 disabled，不自動恢復。
        if let Some(device_id) = authed {
            let was_active = {
                let mut conns = self.mobile.conns.write().await;
                match conns.get(&device_id) {
                    Some(c) if c.conn_id == conn_id => {
                        conns.remove(&device_id);
                        true
                    }
                    _ => false,
                }
            };
            let pid = ProviderId::new(format!("provider.mobile.{device_id}"));
            let already_revoked = self
                .providers
                .get(&pid)
                .await
                .map(|d| d.state == ProviderState::Revoked)
                .unwrap_or(false);
            if was_active && closed_by_server.is_none() && !already_revoked {
                if let Err(e) = self
                    .providers
                    .transition(
                        &pid,
                        ProviderState::Disconnected,
                        Some("connection lost".into()),
                    )
                    .await
                {
                    tracing::debug!(error = %e, device_id, "mobile provider disconnect transition skipped");
                } else {
                    // Character Protocol §11：iPhone 斷線 → notice（device-offline）。
                    self.character_project_provider(&pid, ProviderState::Disconnected);
                }
            }
            match closed_by_server {
                Some("superseded") => {} // 同一台手機的新連線仍在：不是斷線
                Some(reason) => {
                    self.mobile_disable_high_risk_receptors(&device_id, reason)
                        .await
                }
                None if was_active => {
                    self.mobile_disable_high_risk_receptors(&device_id, "disconnected")
                        .await
                }
                None => {}
            }
        }
        // 優雅收尾：讓 writer 把最後的回覆（auth-fail/pair-fail）送完再關。
        drop(out_tx);
        let _ = tokio::time::timeout(Duration::from_secs(1), writer).await;
    }

    /// BLE gateway：請 iPhone 掃描周邊（示範閉環；connect/gatt 走同協定）。
    /// 逾時＝掃描時間＋2s 寬限；逾時一律移除 pending（不洩漏）。
    pub async fn mobile_ble_scan(&self, duration_ms: u64) -> DomainResult<Value> {
        let duration_ms = duration_ms.clamp(500, 8_000);
        let id = format!("ble-scan-{}", token_hex(4));
        let msg = json!({"type":"ble.scan","id":id,"durationMs":duration_ms});
        let rx = self
            .mobile
            .dispatch(&id, msg)
            .await
            .map_err(DomainError::Unavailable)?;
        let timeout = Duration::from_millis(duration_ms) + BLE_REPLY_GRACE;
        self.mobile
            .await_reply(&id, rx, timeout)
            .await
            .ok_or_else(|| {
                DomainError::Unavailable(format!(
                    "iPhone did not answer the BLE scan within {}ms (outcome unknown)",
                    timeout.as_millis()
                ))
            })
    }
}

/// 找一個非 loopback 的區網 IPv4（顯示在配對 QR；找不到就誠實留白）。
fn local_lan_ip() -> Option<String> {
    // 標準庫戲法：對外開 UDP socket（不真的送包）取本機路由位址。
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.168.255.255:80").ok()?;
    let addr = socket.local_addr().ok()?;
    let ip = addr.ip();
    if ip.is_loopback() {
        None
    } else {
        Some(ip.to_string())
    }
}
