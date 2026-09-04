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
    ComponentHealth, DomainError, DomainResult, EventType, ProviderDescriptor, ProviderId,
    ProviderIdentity, ProviderKind, ProviderState, ReceptorId, RiskClass, Sensitivity, TrustLevel,
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
/// 「停止所有感測」等每台手機確認（`ack{stopAll}` 或 `status{micLevel:false}`）
/// 的有界等待。逾時＝結果未知（不重送、不謊稱已停）。
pub const STOP_SENSORS_WAIT: Duration = Duration::from_secs(2);
/// `stop-all` wire 訊息的 `reason`：使用者按了「停止所有感測」。
/// 只影響手機端顯示哪一句停用說明，不改變停的範圍。
pub const STOP_REASON_USER: &str = "user";
/// `stop-all` wire 訊息的 `reason`：桌面緊急停止。
/// iOS 端對缺席／不認得的值一律降級成 emergency（保守的那一邊）。
pub const STOP_REASON_EMERGENCY: &str = "emergency";
/// 「測試這台手機」等 Pong 的有界等待（只證明 socket 會回答，不證明 App 功能）。
pub const MOBILE_TEST_TIMEOUT: Duration = Duration::from_secs(3);
/// 是否對外廣播 Bonjour／綁 0.0.0.0 的環境開關（`INTERACT_AI_MOBILE_ADVERTISE=0`
/// ＝只綁 127.0.0.1、不對區網廣播；E2E／CI 用，模擬不得有區網副作用）。
pub const MOBILE_ADVERTISE_ENV: &str = "INTERACT_AI_MOBILE_ADVERTISE";
/// BLE 掃描回覆的額外寬限（掃描時間＋此值＝逾時）。
const BLE_REPLY_GRACE: Duration = Duration::from_secs(2);
/// 同時允許的手機連線上限：超過就在 accept 當下拒絕（區網上任何 peer 都能
/// 連進來，不能讓它把檔案描述子／記憶體吃光）。
pub const MOBILE_MAX_CONNS: usize = 8;
/// TLS 交握的有界時間（半開的 TLS 連線不得無限期佔用一個名額）。
const MOBILE_TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// WebSocket 交握的有界時間。
const MOBILE_WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// 連上之後多久還沒完成配對／認證就關閉。Ping、Pong 與未知訊息**不能續命**
/// ——否則未認證 peer 只要每 10 秒送一個 Ping 就能永遠佔著連線。
const MOBILE_AUTH_TIMEOUT_DEFAULT_MS: u64 = 10_000;
/// 單一訊息／單一 frame 上限（正常語意事件遠小於此；預設 64 MiB 等於讓任何
/// peer 用一則訊息就吃掉記憶體）。
pub const MOBILE_WS_MAX_MESSAGE_BYTES: usize = 128 * 1024;
/// 每條連線每秒可處理的入站訊息數；超過視為濫用，關閉並留 audit。
pub const MOBILE_MAX_INBOUND_PER_SEC: u32 = 30;
/// accept 連續錯誤多少次之後放棄（放棄＝誠實回報伺服器已停，不是假裝還活著）。
const MOBILE_ACCEPT_MAX_CONSECUTIVE_ERRORS: u32 = 20;
/// Runtime 投影角色真相狀態（緊急停止／人工驗證）時等每台手機 ack 的有界時間。
pub const CHARACTER_PROJECT_WAIT: Duration = Duration::from_millis(1_500);

/// 是否允許對區網廣播／綁 0.0.0.0（`INTERACT_AI_MOBILE_ADVERTISE=0|false|off`
/// ＝不允許）。預設允許——只有明確關掉才降級成 loopback。
pub fn mobile_advertise_enabled() -> bool {
    match std::env::var(MOBILE_ADVERTISE_ENV) {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        Err(_) => true,
    }
}

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
/// 其中 `verified-success` 與 `emergency` 是 **Runtime 專屬的真相狀態**：
/// 前者只能由人工驗證路徑（`Runtime::mobile_present_verified`）直送、後者只能
/// 由真正的緊急停止路徑（`Runtime::mobile_project_estop`）直送；plan／policy／
/// agent 路徑（含 `extra.state` 與 message 推導）一律被 `map_wire_params` 拒絕。
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
/// 只有 runtime 的緊急停止路徑能送出的角色狀態（手機的「緊急停止中」）。
pub const EMERGENCY_STATE: &str = "emergency";
/// Runtime 專屬真相狀態：AI 不得冒充緊急停止，也不得自封已驗證。
pub const RUNTIME_ONLY_STATES: &[&str] = &[VERIFIED_STATE, EMERGENCY_STATE];
/// `tts.speak` 的文字上限（與 iOS `ActuatorCenter.handleTtsSpeak` 一致）。
pub const TTS_MAX_CHARS: usize = 200;
/// `notify.show` 的伺服器端長度上限（App 不設限，但沒有理由把整篇文章
/// 塞進通知；超過就在這裡誠實拒絕，不代為截斷）。
pub const NOTIFY_TITLE_MAX_CHARS: usize = 64;
pub const NOTIFY_BODY_MAX_CHARS: usize = 300;

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
/// - screen.flash：`color` 預設 `#FFB347`（必須是 6 位十六進位）、
///   `durationMs` 1..=1500（預設 400）。
/// - character.present：`state` 必須在白名單；`verified-success` 與 `emergency`
///   **一律拒絕**（那是 Runtime 專屬真相狀態，只能由人工驗證／真正的緊急
///   停止路徑直送）。
///
/// **型別與長度在這裡就擋掉**（見 [`validate_wire_params`]）：不能靠手機回
/// `bad-params` 才發現——那時 policy 已經授權、動作已經送出去了。
/// `extra.deviceId` 是「送到哪一台手機」的路由參數，不是 wire 參數，
/// 在這裡被移除（見 [`target_device_id`]）。
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
    // 目標手機是路由參數，不進 wire params（手機不需要知道自己是誰）。
    p.remove("deviceId");
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
                    if RUNTIME_ONLY_STATES.contains(&s) {
                        // 誠實階梯：completed ≠ verified，而「緊急停止中」是
                        // 只有 Runtime 知道的真相。任何 plan／agent 路徑
                        // （含 extra.state）都不得讓手機顯示這兩個狀態。
                        let why = if s == VERIFIED_STATE {
                            "human-verification only"
                        } else {
                            "emergency-stop only (runtime-owned truth)"
                        };
                        return Err(format!(
                            "character.present: `{s}` is {why}; it can never be requested through a plan"
                        ));
                    }
                    if !CHARACTER_STATES.contains(&s) {
                        return Err(format!(
                            "character.present: state `{s}` not allowed (idle/working/waiting/failed/unknown)"
                        ));
                    }
                    s.to_string()
                }
                // 從 message 推導絕不產生 Runtime 專屬真相狀態。
                None => message
                    .filter(|m| CHARACTER_STATES.contains(m) && !RUNTIME_ONLY_STATES.contains(m))
                    .unwrap_or("idle")
                    .to_string(),
            };
            p.insert("state".into(), json!(state));
            "character.present"
        }
        other => return Err(format!("unknown mobile actuator {other}")),
    };
    // 伺服器端就把 App 會拒絕的形狀擋下來（型別、長度、色碼、區間）。
    validate_wire_params(wire, &p)?;
    Ok((wire, Value::Object(p)))
}

/// 動作參數裡的目標手機（`extra.deviceId`）。`None` ＝未指定
/// （只有恰好一台手機連線時才成立，多台連線一律拒絕、絕不替使用者猜）。
pub fn target_device_id(e: &ActionParameters) -> Result<Option<String>, String> {
    match e.extra.as_ref().and_then(|v| v.get("deviceId")) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        Some(other) => Err(format!(
            "deviceId must be a non-empty string naming a connected iPhone, got {other}"
        )),
    }
}

/// 停止感測的 audit reason → `stop-all` wire 上的 `reason`。
/// 只有緊急停止那條路徑（`mobile_estop_stop_sensors`）是 `emergency`；
/// 其餘（使用者按「停止所有感測」、撤回麥克風授權、結束工作階段）都是
/// 使用者發起的，手機端不得把它顯示成緊急停止。
/// 不認得的 reason 保守地當成 `emergency`（顯示比較嚴格的那一句）。
pub fn stop_all_wire_reason(audit_reason: &str) -> &'static str {
    match audit_reason {
        "stop-all-sensors" => STOP_REASON_USER,
        _ => STOP_REASON_EMERGENCY,
    }
}

fn wire_text<'a>(p: &'a Map<String, Value>, key: &str, wire: &str) -> Result<&'a str, String> {
    match p.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.as_str()),
        Some(other) => Err(format!(
            "{wire}: `{key}` must be a non-empty string, got {other}"
        )),
        None => Err(format!("{wire}: `{key}` is required")),
    }
}

fn wire_int(p: &Map<String, Value>, key: &str, wire: &str) -> Result<i64, String> {
    p.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{wire}: `{key}` must be a whole number of milliseconds"))
}

/// 伺服器端的 wire 參數守門：與 iOS
/// `apps/interaction-ios/InteractionCompanion/Services/ActuatorCenter.swift`
/// 的驗證逐條對齊（外加 `notify.show` 的長度上限）。
/// **不代為截斷／改寫使用者的文字**——超界就誠實拒絕。
pub fn validate_wire_params(wire: &str, p: &Map<String, Value>) -> Result<(), String> {
    match wire {
        "haptic.pulse" => {
            let style = wire_text(p, "style", wire)?;
            if haptic_style_rank(style).is_none() {
                return Err(format!("haptic.pulse: unknown style `{style}`"));
            }
            let count = wire_int(p, "count", wire)?;
            if !(1..=5).contains(&count) {
                return Err(format!("haptic.pulse: count {count} is outside 1..=5"));
            }
        }
        "notify.show" => {
            let title = wire_text(p, "title", wire)?;
            let n = title.chars().count();
            if n > NOTIFY_TITLE_MAX_CHARS {
                return Err(format!(
                    "notify.show: title is {n} chars, the limit is {NOTIFY_TITLE_MAX_CHARS}"
                ));
            }
            let body = wire_text(p, "body", wire)?;
            let n = body.chars().count();
            if n > NOTIFY_BODY_MAX_CHARS {
                return Err(format!(
                    "notify.show: body is {n} chars, the limit is {NOTIFY_BODY_MAX_CHARS}"
                ));
            }
        }
        "tts.speak" => {
            let text = wire_text(p, "text", wire)?;
            let n = text.chars().count();
            if n > TTS_MAX_CHARS {
                return Err(format!(
                    "tts.speak: text is {n} chars, the iPhone limit is {TTS_MAX_CHARS} (nothing was sent, and it is not truncated for you)"
                ));
            }
        }
        "torch.set" => {
            let on = p
                .get("on")
                .and_then(Value::as_bool)
                .ok_or_else(|| "torch.set: `on` must be a boolean".to_string())?;
            let d = wire_int(p, "durationMs", wire)?;
            if on && !(1..=5_000).contains(&d) {
                return Err(format!("torch.set: durationMs {d} is outside 1..=5000"));
            }
        }
        "screen.flash" => {
            let color = wire_text(p, "color", wire)?;
            let hex = color.strip_prefix('#').unwrap_or(color);
            if hex.len() != 6 || u32::from_str_radix(hex, 16).is_err() {
                return Err(format!(
                    "screen.flash: color `{color}` must be 6 hex digits like #FFB347"
                ));
            }
            let d = wire_int(p, "durationMs", wire)?;
            if !(1..=1_500).contains(&d) {
                return Err(format!("screen.flash: durationMs {d} is outside 1..=1500"));
            }
        }
        "character.present" => {
            let state = wire_text(p, "state", wire)?;
            if !CHARACTER_STATES.contains(&state) {
                return Err(format!("character.present: state `{state}` not allowed"));
            }
        }
        _ => {}
    }
    Ok(())
}

/// accept 迴圈遇到錯誤時的處置：短暫退避後重試，或誠實停下來。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptErrorAction {
    /// 暫時性錯誤（連線在交握前就沒了、檔案描述子用盡…）：退避後繼續 accept。
    RetryAfter(Duration),
    /// 監聽 socket 本身不可用，或連續錯誤太多次：停下來並誠實回報伺服器已停。
    Stop,
}

/// 單一 `accept()` 錯誤該退避重試還是放棄。純函式，方便直接回歸測試。
///
/// 預設「重試」：`EMFILE`／`ENFILE` 這類在 Rust 沒有穩定 `ErrorKind` 的錯誤
/// 一律當成暫時性——一個連線失敗不該讓整個 iPhone 伺服器悄悄消失。
/// 只有監聽 socket 本身不可用（權限／位址）或連續錯誤超過上限才停。
pub fn accept_error_action(err: &std::io::Error, consecutive: u32) -> AcceptErrorAction {
    use std::io::ErrorKind;
    let listener_is_unusable = matches!(
        err.kind(),
        ErrorKind::PermissionDenied
            | ErrorKind::AddrInUse
            | ErrorKind::AddrNotAvailable
            | ErrorKind::InvalidInput
            | ErrorKind::NotFound
            | ErrorKind::Unsupported
    );
    if listener_is_unusable || consecutive >= MOBILE_ACCEPT_MAX_CONSECUTIVE_ERRORS {
        return AcceptErrorAction::Stop;
    }
    // 25ms → 50 → … → 800ms（上限 1s）：有界退避，不是 blocking sleep。
    let backoff = (25u64 << consecutive.min(5)).min(1_000);
    AcceptErrorAction::RetryAfter(Duration::from_millis(backoff))
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
    /// 手機自報／實際推來觀察所推導的「麥克風音量串流中」起算時間（感測不
    /// 靜默：status／tray／首頁／角色視窗都以此顯示 activeSensors）。None＝沒在串流。
    mic_since: Option<chrono::DateTime<Utc>>,
    /// 撤銷／被新連線取代時觸發 → handler 立即收尾關閉。
    close: CancellationToken,
    /// 「停止所有感測」的確認追蹤（請求時間／確認時間／確認來源）。
    /// `ack{stopAll:true}` 沒有 id，不能走 `resolve_pending`。
    stop_sensors: Arc<StopSensorsTracker>,
    /// `mobile_test` 的 Ping／Pong 等待者（nonce → 回覆通道）。
    ping_waiters: PingWaiters,
}

/// 等待中的 Ping（nonce, 回覆通道）。conn loop 收到相同 payload 的 Pong 才解除。
type PingWaiters = Arc<std::sync::Mutex<Vec<(Vec<u8>, oneshot::Sender<Instant>)>>>;

/// 一台手機對「停止感測」請求的狀態。誠實階梯：requested ≠ stopped——
/// 只有手機回 `ack{stopAll:true}` 或請求之後回報 `micLevel:false` 才算確認。
#[derive(Default)]
struct StopSensorsState {
    requested_at: Option<Instant>,
    confirmed_at: Option<Instant>,
    via: Option<&'static str>,
    /// 等待期間連線就斷了：無法確認（unreachable），不得算成已停止。
    disconnected: bool,
}

#[derive(Default)]
struct StopSensorsTracker {
    state: std::sync::Mutex<StopSensorsState>,
    notify: tokio::sync::Notify,
}

impl StopSensorsTracker {
    fn lock(&self) -> std::sync::MutexGuard<'_, StopSensorsState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 記一次新的停止請求（清掉上一次的確認：確認必須是「這一次」的）。
    fn request(&self) {
        let mut state = self.lock();
        state.requested_at = Some(Instant::now());
        state.confirmed_at = None;
        state.via = None;
    }

    /// 手機確認停了（`ack`／`status`）。只有在有未完成請求時才算數；
    /// 30 秒心跳 status 在請求之前到達的不會被誤認（以請求時間為界）。
    fn confirm(&self, via: &'static str) {
        {
            let mut state = self.lock();
            if state.requested_at.is_none() || state.confirmed_at.is_some() {
                return;
            }
            state.confirmed_at = Some(Instant::now());
            state.via = Some(via);
        }
        self.notify.notify_waiters();
    }

    fn mark_disconnected(&self) {
        self.lock().disconnected = true;
        self.notify.notify_waiters();
    }

    /// (已確認的來源, 等待期間斷線, 請求時間)。
    fn snapshot(&self) -> (Option<&'static str>, bool, Option<Instant>) {
        let state = self.lock();
        (state.via, state.disconnected, state.requested_at)
    }

    /// 這條連線上是否曾要求停止感測（確認之後仍為 true）。
    fn was_requested(&self) -> bool {
        self.lock().requested_at.is_some()
    }

    /// 目前是否「已請求停止但尚未確認」（activeSensors 據此標停止中／結果未知）。
    fn pending_since(&self) -> Option<Instant> {
        let state = self.lock();
        match (state.requested_at, state.confirmed_at) {
            (Some(at), None) => Some(at),
            _ => None,
        }
    }
}

/// 一台手機的停止結果（誠實：stopped／unknown／unreachable 三態，不合併）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileStopOutcome {
    pub device_id: String,
    pub name: String,
    pub outcome: StopOutcome,
    /// 實際等待毫秒（有界；逾時＝unknown）。
    pub waited_ms: u64,
    /// 確認來源：`ack`（手機回 stop-all 確認）或 `status`（手機回報不再串流）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StopOutcome {
    /// 手機明確確認已停止感測。
    Stopped,
    /// 送出去了，但沒有在有界時間內收到確認——手機可能還在錄音。
    Unknown,
    /// 根本送不出去（佇列滿／連線已斷）：沒有任何東西被停。
    Unreachable,
}

impl StopOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            StopOutcome::Stopped => "stopped",
            StopOutcome::Unknown => "unknown",
            StopOutcome::Unreachable => "unreachable",
        }
    }
}

struct PendingAct {
    /// act 送往哪台手機：只有同一台的 ack/err 能解除。
    device_id: String,
    reply: oneshot::Sender<Value>,
}

/// 等待回覆期間的 pending 表項守衛：不論正常逾時、收到回覆，還是等待端的
/// future 被丟棄（HTTP client 斷線／CLI Ctrl-C），離開作用域一定清掉表項。
struct PendingGuard<'a> {
    bridge: &'a MobileBridge,
    id: &'a str,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.bridge.remove_pending(self.id);
    }
}

pub struct MobileBridge {
    /// 測試模式（Runtime 無 watchdog）不得把 Bonjour 服務記錄廣播到實體區網——
    /// 測試是模擬，不能有外部副作用；生產 daemon 才廣播。
    advertise_mdns: AtomicBool,
    /// 關閉廣播的原因（誠實顯示在 `status.bonjour.error`；不得假裝 advertised）。
    advertise_off_reason: std::sync::Mutex<String>,
    started: AtomicBool,
    /// 序列化啟動：`started` 只在 bind 等全部成功後才設，失敗可重試。
    start_lock: Mutex<()>,
    port: RwLock<Option<u16>>,
    fingerprint: RwLock<Option<String>>,
    pairing: Mutex<Option<PairingSession>>,
    devices: RwLock<BTreeMap<String, PairedDevice>>,
    conns: RwLock<BTreeMap<String, ConnState>>,
    conn_seq: AtomicU64,
    /// 在途 act（id → 目標手機＋回覆通道）。用 std Mutex：所有臨界區都是同步
    /// 短操作，Drop guard 才能在等待端 future 被丟棄時同步清掉自己的表項。
    pending_acts: std::sync::Mutex<BTreeMap<String, PendingAct>>,
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
    /// 未認證連線的存活上限（Ping／未知訊息不能續命）。
    auth_timeout_ms: AtomicU64,
    /// 同時連線名額：accept 當下拿不到就直接拒絕（不是先接進來再說）。
    conn_permits: Arc<tokio::sync::Semaphore>,
    /// 因為超過連線上限而被拒絕的次數（status 誠實顯示）。
    refused_connections: AtomicU64,
    /// 認證失敗（未知裝置／錯 token／已撤銷）的次數。撤銷之後手機還在重連、
    /// 或區網上有人猜 deviceId／token，都必須看得見（audit 有逐筆，這裡是計數）。
    failed_auths: AtomicU64,
    /// `receptor.offline` watcher 是否已經在跑（伺服器重啟不得疊第二個）。
    receptor_watcher: AtomicBool,
    /// runtime 自己關掉高風險受器（斷線／撤銷／緊急停止）時先記一筆：那些
    /// 路徑各自已經送過停止請求，`receptor.offline` watcher 不必（也不得）
    /// 再送一則，否則同一次事件會對手機多廣播一次 stop-all。
    self_receptor_disables: std::sync::Mutex<BTreeMap<String, u32>>,
    /// 配對清單讀不到／解析失敗的原因（`None`＝清單是可信的）。
    /// 誠實階梯：unknown ≠ 「還沒有配對的 iPhone」——UI 必須看得出差別。
    devices_load_error: RwLock<Option<String>>,
    /// 測試用故障注入：讓 accept 迴圈走幾次錯誤分支（預設 0＝永不注入）。
    inject_accept_errors: AtomicU64,
    /// 測試用故障注入：讓所有出站訊息一律排不進去（模擬「連線還在、但佇列
    /// 塞爆／寫入端卡住」——這正是誠實階梯最容易破功的情境）。預設 false。
    inject_outbound_failure: AtomicBool,
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
            advertise_off_reason: std::sync::Mutex::new(String::new()),
            started: AtomicBool::new(false),
            start_lock: Mutex::new(()),
            port: RwLock::new(None),
            fingerprint: RwLock::new(None),
            pairing: Mutex::new(None),
            devices: RwLock::new(BTreeMap::new()),
            conns: RwLock::new(BTreeMap::new()),
            conn_seq: AtomicU64::new(1),
            pending_acts: std::sync::Mutex::new(BTreeMap::new()),
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
            auth_timeout_ms: AtomicU64::new(MOBILE_AUTH_TIMEOUT_DEFAULT_MS),
            conn_permits: Arc::new(tokio::sync::Semaphore::new(MOBILE_MAX_CONNS)),
            refused_connections: AtomicU64::new(0),
            failed_auths: AtomicU64::new(0),
            receptor_watcher: AtomicBool::new(false),
            self_receptor_disables: std::sync::Mutex::new(BTreeMap::new()),
            devices_load_error: RwLock::new(None),
            inject_accept_errors: AtomicU64::new(0),
            inject_outbound_failure: AtomicBool::new(false),
            character_title: std::sync::Mutex::new(None),
        })
    }

    pub async fn any_connected(&self) -> bool {
        !self.conns.read().await.is_empty()
    }

    /// 是否對外廣播 Bonjour（測試模式／環境開關關閉；status.bonjour 誠實回報
    /// disabled）。關閉時伺服器只綁 127.0.0.1——模擬與 E2E 不得對區網開埠。
    pub fn set_advertise_mdns(&self, on: bool) {
        self.advertise_mdns.store(on, Ordering::SeqCst);
    }

    /// 關閉廣播的原因（人話，直接進 `status.bonjour.error`）。
    pub fn set_advertise_off_reason(&self, reason: String) {
        *self
            .advertise_off_reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = reason;
    }

    fn advertise_off_reason(&self) -> String {
        let reason = self
            .advertise_off_reason
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if reason.is_empty() {
            "disabled (no LAN side effects)".to_string()
        } else {
            format!("disabled ({reason})")
        }
    }

    /// 廣播關閉時只綁 loopback：模擬／E2E 不得把 iPhone 伺服器開到區網上。
    pub fn bind_ip(&self) -> &'static str {
        if self.advertise_mdns.load(Ordering::SeqCst) {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        }
    }

    /// 覆寫心跳／閒置逾時（新連線起算；測試用短值）。
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

    /// 覆寫「未認證連線」的存活上限（測試用短值）。
    pub fn set_auth_timeout(&self, timeout: Duration) {
        self.auth_timeout_ms
            .store(timeout.as_millis().max(1) as u64, Ordering::SeqCst);
    }

    fn auth_timeout(&self) -> Duration {
        Duration::from_millis(self.auth_timeout_ms.load(Ordering::SeqCst))
    }

    pub fn refused_connections(&self) -> u64 {
        self.refused_connections.load(Ordering::SeqCst)
    }

    fn note_refused_connection(&self) {
        self.refused_connections.fetch_add(1, Ordering::SeqCst);
    }

    /// 認證失敗次數（`status.heartbeat.failedAuths`）。
    pub fn failed_auths(&self) -> u64 {
        self.failed_auths.load(Ordering::SeqCst)
    }

    fn note_failed_auth(&self) {
        self.failed_auths.fetch_add(1, Ordering::SeqCst);
    }

    /// 測試用故障注入：接下來 `times` 次 accept 直接走錯誤分支。
    /// 預設 0（生產路徑永遠不會注入），只用來回歸「暫時性錯誤不會殺掉伺服器」。
    #[doc(hidden)]
    pub fn inject_accept_errors(&self, times: u64) {
        self.inject_accept_errors.store(times, Ordering::SeqCst);
    }

    /// 測試用故障注入：出站訊息一律視為排不進去（連線仍在 `conns` 裡）。
    /// 預設 false（生產路徑永遠不會注入），只用來回歸「沒送出去不得謊報停止」。
    #[doc(hidden)]
    pub fn inject_outbound_failure(&self, on: bool) {
        self.inject_outbound_failure.store(on, Ordering::SeqCst);
    }

    /// 對一條連線送一則出站訊息；回傳「有沒有真的排進佇列」。
    /// 排進佇列 ≠ 手機收到——但排不進去一定是沒收到。
    async fn send_outbound(&self, outbound: &mpsc::Sender<Message>, msg: Message) -> bool {
        if self.inject_outbound_failure.load(Ordering::SeqCst) {
            return false;
        }
        outbound
            .send_timeout(msg, Duration::from_millis(300))
            .await
            .is_ok()
    }

    fn take_injected_accept_error(&self) -> Option<std::io::Error> {
        self.inject_accept_errors
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .ok()
            .map(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "injected accept error (test fault injection)",
                )
            })
    }

    /// runtime 自己要關掉這個受器了（斷線／撤銷／緊急停止路徑）。
    fn note_self_receptor_disable(&self, receptor_id: &str) {
        *self
            .self_receptor_disables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(receptor_id.to_string())
            .or_insert(0) += 1;
    }

    /// 這一則 `receptor.offline` 是不是 runtime 自己剛剛造成的（消費一次）。
    fn take_self_receptor_disable(&self, receptor_id: &str) -> bool {
        let mut map = self
            .self_receptor_disables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match map.get_mut(receptor_id) {
            Some(n) if *n > 0 => {
                *n -= 1;
                true
            }
            _ => false,
        }
    }

    /// 配對清單目前是不是「讀不到／壞掉」（status 誠實顯示；`None`＝可信）。
    pub async fn devices_load_error(&self) -> Option<String> {
        self.devices_load_error.read().await.clone()
    }

    pub fn dropped_observations(&self) -> u64 {
        self.dropped_observations.load(Ordering::SeqCst)
    }

    fn note_dropped(&self, receptor: &str, why: &str) {
        self.dropped_observations.fetch_add(1, Ordering::SeqCst);
        tracing::debug!(receptor, why, "mobile observation dropped");
    }

    /// 挑一台已連線手機。指定 `target` 就只找那一台；沒指定時**只有恰好一台
    /// 連線**才成立——多台連線時絕不替使用者猜（實體效果送錯手機無法收回）。
    async fn pick_conn(
        &self,
        target: Option<&str>,
    ) -> Result<(String, mpsc::Sender<Message>), String> {
        let conns = self.conns.read().await;
        let connected = || {
            conns
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        };
        match target {
            Some(want) => conns
                .get(want)
                .map(|c| (want.to_string(), c.outbound.clone()))
                .ok_or_else(|| {
                    if conns.is_empty() {
                        format!("iPhone `{want}` is not connected (no iPhone is connected)")
                    } else {
                        format!(
                            "iPhone `{want}` is not connected (connected: {})",
                            connected()
                        )
                    }
                }),
            None if conns.is_empty() => Err("no iPhone connected".to_string()),
            None if conns.len() == 1 => conns
                .iter()
                .next()
                .map(|(id, c)| (id.clone(), c.outbound.clone()))
                .ok_or_else(|| "no iPhone connected".to_string()),
            None => Err(format!(
                "{} iPhones are connected ({}) — name the target one with deviceId; nothing was sent",
                conns.len(),
                connected()
            )),
        }
    }

    /// 已配對手機的人話名稱（收據／audit 要記得出「送到哪一台」）。
    pub async fn device_name(&self, device_id: &str) -> Option<String> {
        self.devices
            .read()
            .await
            .get(device_id)
            .map(|d| d.name.clone())
    }

    /// 廣播給所有手機；回傳成功排入佇列的連線數。
    async fn broadcast(&self, text: String) -> usize {
        let conns = self.conns.read().await;
        let mut delivered = 0;
        for conn in conns.values() {
            if self
                .send_outbound(&conn.outbound, Message::Text(text.clone()))
                .await
            {
                delivered += 1;
            }
        }
        delivered
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, PendingAct>> {
        self.pending_acts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn pending_len(&self) -> usize {
        self.pending().len()
    }

    fn remove_pending(&self, id: &str) {
        self.pending().remove(id);
    }

    /// 送一則 AIP envelope 給指定手機（`{"type":"aip","envelope":…}`）。
    ///
    /// 不登記 pending：AIP 的回覆是對端自己送的 `result`／`response`，不是 wire 層 ack。
    /// 送不出去就誠實回 Err（呼叫端只記錄，不重送、不假裝送到了）。
    pub async fn send_aip(
        &self,
        device_id: &str,
        envelope: &interaction_aip::Envelope,
    ) -> Result<(), String> {
        let bytes = envelope
            .encode()
            .map_err(|_| format!("envelope for `{device_id}` exceeds the AIP message limit"))?;
        if bytes.len() > MOBILE_WS_MAX_MESSAGE_BYTES {
            return Err(format!("envelope for `{device_id}` exceeds the wire limit"));
        }
        let (device_id, outbound) = self.pick_conn(Some(device_id)).await?;
        let value = serde_json::to_value(envelope)
            .map_err(|_| format!("envelope for `{device_id}` could not be encoded"))?;
        let frame = json!({"type": "aip", "envelope": value}).to_string();
        outbound
            .send_timeout(Message::Text(frame), Duration::from_millis(500))
            .await
            .map_err(|_| format!("iPhone `{device_id}` outbound queue full or closed"))
    }

    /// 送一則需要回覆的訊息並登記 pending（綁定目標手機）。
    /// 回傳 (實際送往哪一台, 回覆接收端)——收據／audit 一律記真正的那一台。
    pub(crate) async fn dispatch(
        &self,
        target: Option<&str>,
        id: &str,
        msg: Value,
    ) -> Result<(String, oneshot::Receiver<Value>), String> {
        let (device_id, outbound) = self.pick_conn(target).await?;
        let (tx, rx) = oneshot::channel();
        self.pending().insert(
            id.to_string(),
            PendingAct {
                device_id: device_id.clone(),
                reply: tx,
            },
        );
        if outbound
            .send_timeout(Message::Text(msg.to_string()), Duration::from_millis(500))
            .await
            .is_err()
        {
            self.remove_pending(id);
            return Err(format!(
                "iPhone `{device_id}` outbound queue full or closed"
            ));
        }
        Ok((device_id, rx))
    }

    /// 送一則 act 並登記 pending；呼叫端自己等 ack（逾時＝結果未知，不重送）。
    pub(crate) async fn dispatch_act(
        &self,
        target: Option<&str>,
        name: &str,
        params: Value,
        action_id: &str,
    ) -> Result<(String, oneshot::Receiver<Value>), String> {
        let msg = json!({"type":"act","id":action_id,"name":name,"params":params});
        self.dispatch(target, action_id, msg).await
    }

    /// 等回覆；逾時、通道關閉、**或等待端 future 被丟棄**（HTTP client 斷線／
    /// CLI 中斷）一律移除 pending——不洩漏、不留下永遠等不到的表項。
    pub(crate) async fn await_reply(
        &self,
        id: &str,
        rx: oneshot::Receiver<Value>,
        timeout: Duration,
    ) -> Option<Value> {
        let _guard = PendingGuard { bridge: self, id };
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => Some(reply),
            _ => None,
        }
    }

    /// 送 act 並等 ack（逾時＝結果未知；絕不重送）。
    /// 回傳 (實際送往哪一台, 回覆)。
    pub async fn act(
        &self,
        target: Option<&str>,
        name: &str,
        params: Value,
        action_id: &str,
    ) -> Result<(String, Value), String> {
        let (device_id, rx) = self.dispatch_act(target, name, params, action_id).await?;
        match self.await_reply(action_id, rx, ACT_TIMEOUT).await {
            Some(reply) => Ok((device_id, reply)),
            None => Err(format!(
                "no ack for {action_id} from `{device_id}` — outcome UNKNOWN (not retried)"
            )),
        }
    }

    /// 只有「同一台手機」對「同一 id」的回覆能解除 pending。
    fn resolve_pending(&self, device_id: &str, reply: &Value) {
        let Some(id) = reply["id"].as_str() else {
            return;
        };
        let mut pending = self.pending();
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
    fn fail_all_pending(&self, reply: Value) -> usize {
        let drained = std::mem::take(&mut *self.pending());
        let n = drained.len();
        for (_, p) in drained {
            let _ = p.reply.send(reply.clone());
        }
        n
    }

    /// estop：在途 act 立刻以 `stopped` 收場（結果未知，永不重送）。
    pub fn fail_inflight_stopped(&self) -> usize {
        self.fail_all_pending(json!({
            "type": "err",
            "reason": "stopped",
            "stopAll": true,
        }))
    }

    /// 手機斷線：只讓「那一台」的在途 act 立即以 `disconnected` 收場，
    /// 不必等滿 4 秒逾時（結果一樣是未知，但呼叫端立刻知道）。
    fn fail_pending_for_device(&self, device_id: &str) -> usize {
        let mut pending = self.pending();
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, p)| p.device_id == device_id)
            .map(|(id, _)| id.clone())
            .collect();
        let n = ids.len();
        for id in ids {
            if let Some(p) = pending.remove(&id) {
                let _ = p.reply.send(json!({
                    "type": "err",
                    "id": id,
                    "reason": "disconnected",
                }));
            }
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
        self.fail_inflight_stopped();
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
        // 這條路徑只有緊急停止會走（六個 mobile actuator 的 emergency_stop），
        // 所以 wire reason 一律是 emergency：手機端要說「因桌面緊急停止而停用」，
        // 不能講成使用者自己按的「停止所有感測」。
        let delivered = self
            .broadcast(
                json!({"type":"stop-all","sensors":sensors,"reason":STOP_REASON_EMERGENCY})
                    .to_string(),
            )
            .await;
        if delivered == 0 {
            // 一則都沒排進去＝什麼都沒被停：去重窗**不得**因此關上，否則接下來
            // 的五個動器會落進去重分支回 Ok(())，被算進 `stoppedActuators`
            // ——同一份 payload 一邊說手機 unreachable、一邊說六個動器已停止。
            return Err(ActuatorError::Unavailable(
                "stop-all could not be queued to any iPhone".into(),
            ));
        }
        *last = Some((Instant::now(), sensors));
        Ok(())
    }

    /// 手機自報「麥克風音量串流中」的連線
    /// （deviceId, 起算時間, 尚未確認的停止請求時間）。
    async fn mic_streaming_devices(&self) -> Vec<(String, chrono::DateTime<Utc>, Option<Instant>)> {
        self.conns
            .read()
            .await
            .iter()
            .filter_map(|(id, c)| {
                c.mic_since
                    .map(|since| (id.clone(), since, c.stop_sensors.pending_since()))
            })
            .collect()
    }

    /// 對所有（或指定一台）已連線手機送 `stop-all { sensors: true }`，
    /// 並**有界等待**每台確認。誠實階梯：送進佇列 ≠ 手機停了——只有
    /// `ack{stopAll:true}` 或請求之後回報的 `micLevel:false` 才算 stopped，
    /// 逾時是 unknown（手機可能還在錄音），送不出去是 unreachable。
    ///
    /// 注意：wire 上就是既有的 stop-all，iOS 端會連短效動器一起停
    /// （haptics／tts／torch／flash）——這是「停止所有感測」的已知副作用。
    pub async fn stop_sensors_and_wait(
        &self,
        timeout: Duration,
        reason: &str,
    ) -> Vec<MobileStopOutcome> {
        self.stop_sensors_for(None, timeout, reason).await
    }

    /// `reason` 是 wire 上的 `STOP_REASON_USER`／`STOP_REASON_EMERGENCY`：
    /// 手機據此顯示正確的停用說明。它**不改變停的範圍**（兩者都連感測一起停），
    /// 缺席或不認得時 iOS 端保守地當成 emergency。
    pub(crate) async fn stop_sensors_for(
        &self,
        target: Option<&str>,
        timeout: Duration,
        reason: &str,
    ) -> Vec<MobileStopOutcome> {
        let text = json!({"type":"stop-all","sensors":true,"reason":reason}).to_string();
        // 快照要送的連線（不在讀鎖裡 await）。
        let targets: Vec<(String, mpsc::Sender<Message>, Arc<StopSensorsTracker>)> = {
            let conns = self.conns.read().await;
            conns
                .iter()
                .filter(|(id, _)| target.is_none_or(|want| want == id.as_str()))
                .map(|(id, c)| (id.clone(), c.outbound.clone(), c.stop_sensors.clone()))
                .collect()
        };
        let names = self.devices.read().await;
        let name_of = |id: &str| {
            names
                .get(id)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| id.to_string())
        };
        let started = Instant::now();
        let mut waiting = Vec::new();
        let mut outcomes = Vec::new();
        for (device_id, outbound, tracker) in targets {
            tracker.request();
            if self
                .send_outbound(&outbound, Message::Text(text.clone()))
                .await
            {
                waiting.push((device_id, tracker));
            } else {
                outcomes.push(MobileStopOutcome {
                    name: name_of(&device_id),
                    device_id,
                    outcome: StopOutcome::Unreachable,
                    waited_ms: 0,
                    via: None,
                });
            }
        }
        drop(names);
        // 共用一個截止時間：整批等待仍然有界（不是每台各等 timeout）。
        let deadline = tokio::time::Instant::now() + timeout;
        for (device_id, tracker) in waiting {
            let outcome = wait_for_stop_confirmation(&tracker, deadline).await;
            let names = self.devices.read().await;
            let name = names
                .get(&device_id)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| device_id.clone());
            drop(names);
            outcomes.push(MobileStopOutcome {
                device_id,
                name,
                outcome: outcome.0,
                waited_ms: started.elapsed().as_millis() as u64,
                via: outcome.1.map(str::to_string),
            });
        }
        // 去重窗從「這一輪停止序列結束」起算：緊急停止時 6 個動器隨後各自
        // 呼叫 stop-all，那是同一次事件，不該再對手機多送一則。
        // 但**只有真的排進去至少一台**才算送過：全部 unreachable 時關上去重窗
        // 等於替失敗的那一則代簽，讓六個動器一起謊報「已停止」。
        if outcomes
            .iter()
            .any(|o| o.outcome != StopOutcome::Unreachable)
        {
            *self.last_stop_all.lock().await = Some((Instant::now(), true));
        }
        outcomes
    }

    /// 送一則 WebSocket Ping（帶 nonce）並等對應的 Pong。
    /// 只證明「socket 會回答」——不證明 App 功能正常。
    pub(crate) async fn ping_device(
        &self,
        device_id: &str,
        timeout: Duration,
    ) -> Result<Option<u64>, String> {
        let nonce = token_hex(8).into_bytes();
        let (tx, rx) = oneshot::channel();
        let (outbound, waiters) = {
            let conns = self.conns.read().await;
            let conn = conns
                .get(device_id)
                .ok_or_else(|| "iPhone not connected".to_string())?;
            (conn.outbound.clone(), conn.ping_waiters.clone())
        };
        waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((nonce.clone(), tx));
        let started = Instant::now();
        if outbound
            .send_timeout(Message::Ping(nonce.clone()), Duration::from_millis(300))
            .await
            .is_err()
        {
            drop_ping_waiter(&waiters, &nonce);
            return Err("iPhone outbound queue full or closed".into());
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(at)) => Ok(Some(at.duration_since(started).as_millis() as u64)),
            _ => {
                drop_ping_waiter(&waiters, &nonce);
                Ok(None)
            }
        }
    }
}

/// 移除一個不再等待的 Ping（逾時／送不出去）：等待表不得無界成長。
fn drop_ping_waiter(waiters: &PingWaiters, nonce: &[u8]) {
    waiters
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|(n, _)| n != nonce);
}

/// 等一台手機確認停止感測，最多等到 `deadline`（不 blocking sleep）。
async fn wait_for_stop_confirmation(
    tracker: &StopSensorsTracker,
    deadline: tokio::time::Instant,
) -> (StopOutcome, Option<&'static str>) {
    loop {
        // 先登記等待再檢查狀態：確認在這兩步之間到達也不會漏掉。
        let notified = tracker.notify.notified();
        let (via, disconnected, _) = tracker.snapshot();
        if let Some(via) = via {
            return (StopOutcome::Stopped, Some(via));
        }
        if disconnected {
            return (StopOutcome::Unreachable, None);
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            // 逾時前的最後一次檢查（確認可能剛好落在逾時邊界）。
            let (via, disconnected, _) = tracker.snapshot();
            return match (via, disconnected) {
                (Some(via), _) => (StopOutcome::Stopped, Some(via)),
                (None, true) => (StopOutcome::Unreachable, None),
                (None, false) => (StopOutcome::Unknown, None),
            };
        }
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
        // 目標手機：`extra.deviceId`；沒指定時只有恰好一台連線才成立。
        let target = target_device_id(&action.effective).map_err(ActuatorError::Rejected)?;
        // 參數：policy-bounded effective 值 → App 驗證的 wire 形狀（手機端仍有硬限制）。
        let title = self.bridge.character_title();
        let (wire_name, params) =
            map_wire_params_titled(self.id, &action.effective, title.as_deref())
                .map_err(ActuatorError::Rejected)?;
        // 送出去之前先知道「送給誰」——收據與 audit 一律記真正的那一台。
        let (device_id, rx) = match self
            .bridge
            .dispatch_act(
                target.as_deref(),
                wire_name,
                params,
                action.action_id.as_str(),
            )
            .await
        {
            Ok(pair) => pair,
            Err(e) => {
                return Ok(DriverReceipt::start(&action, Utc::now())
                    .failed("iphone-unreachable", &e)
                    .finish())
            }
        };
        let device_name = self
            .bridge
            .device_name(&device_id)
            .await
            .unwrap_or_else(|| device_id.clone());
        let dispatched = || {
            DriverReceipt::start(&action, Utc::now())
                .dispatched()
                .note("transport", json!("mobile-wss"))
                .note("deviceId", json!(device_id))
                .note("deviceName", json!(device_name))
        };
        match self
            .bridge
            .await_reply(action.action_id.as_str(), rx, ACT_TIMEOUT)
            .await
        {
            Some(reply) if reply["type"] == "ack" => {
                let mut receipt = dispatched().acknowledged();
                if let Some(applied) = reply.get("applied") {
                    receipt = receipt.note("deviceApplied", applied.clone());
                }
                Ok(receipt.finish())
            }
            // 手機在 ack 之前就斷線：送出去了但沒有回執——結果未知，不重送。
            Some(reply) if reply["reason"] == "disconnected" => Ok(dispatched()
                .note("outcomeUnknown", json!(true))
                .failed(
                    "iphone-disconnected",
                    "iPhone disconnected before it acknowledged — effect unknown",
                )
                .finish()),
            Some(reply) if reply["stopAll"] == true => Ok(dispatched()
                .note("outcomeUnknown", json!(true))
                .failed(
                    "emergency-stopped",
                    "stop-all issued before iPhone acknowledged — effect unknown",
                )
                .finish()),
            Some(reply) => Ok(dispatched()
                .failed(
                    "device-refused",
                    reply["reason"].as_str().unwrap_or("iPhone refused"),
                )
                .finish()),
            // 沒有回覆＝結果未知（不是失敗、也不是成功），絕不重送。
            None => Ok(dispatched().note("ackTimeout", json!(true)).finish()),
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

/// 公開的身分指紋 **不等於** 認證用的驗證值。`token_hash` 是 auth 比對的那個
/// 雜湊，直接當識別碼公開，等於把驗證值送給每個讀得到裝置清單的人。這裡再
/// 雜湊一層並綁 deviceId：穩定、可重現，但推不回驗證值。
fn mobile_identity_fingerprint(device: &PairedDevice) -> String {
    sha256_hex(
        format!(
            "mobile-identity:v1:{}:{}",
            device.device_id, device.token_hash
        )
        .as_bytes(),
    )
}

/// 每台手機的人話註記：六個動作能力是所有已配對 iPhone **共用**的同一組，
/// 不是這一台專屬；同時連線多台時，動作必須指定目標手機。
fn mobile_provider_note(device_id: &str) -> String {
    format!(
        "已連線。這些動作能力由所有已配對 iPhone 共用（不是這一台專屬）；\
         同時連線多台時，動作必須指定目標手機 deviceId：{device_id}——\
         沒有指定就會被拒絕，不會替你猜一台。"
    )
}

/// 原子落地：暫存檔（0600）→ `write_all` → `sync_all` → `rename`。
/// `fs::write` 會先把目標截斷再寫，ENOSPC／崩潰因此留下半截檔案；rename 是
/// 同一個檔案系統上的原子操作，讀到的永遠是「上一份完整的」或「這一份完整的」。
/// 目錄本身盡力 fsync（失敗不致命：rename 已經生效，只是耐當機性差一點）。
fn write_file_atomically(
    tmp: &std::path::Path,
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), String> {
    use std::io::Write;
    // 上一次失敗留下的暫存檔不該擋住這一次（create_new 需要它不存在）。
    if let Err(e) = std::fs::remove_file(tmp) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("{}: {e}", tmp.display()));
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 清單裡有每台手機的 token 雜湊：不必給同機其他使用者看。
        options.mode(0o600);
    }
    let mut file = options
        .open(tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    file.sync_all()
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    drop(file);
    std::fs::rename(tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// 私鑰落地：先以 0600 建立暫存檔、寫完再 rename——檔案從第一刻起就只有
/// 擁有者可讀（「先 write 再 chmod」會有一小段 umask 決定的空窗）。
/// 任何一步失敗都往上拋，不用 `let _` 吞掉。
fn write_private_key(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let tmp = path.with_extension("der.tmp");
    // 上一次失敗留下的暫存檔不該擋住這一次（create_new 需要它不存在）。
    if let Err(e) = std::fs::remove_file(&tmp) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("{}: {e}", tmp.display()));
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    file.sync_all()
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// 私鑰只能是擁有者可讀寫。發現 group／other 位元就修正；修不掉就拒絕啟動
/// ——安靜地用一把全機可讀的私鑰，等於 TLS 指紋釘選形同虛設。
#[cfg(unix)]
fn ensure_key_is_owner_only(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 == 0 {
        return Ok(());
    }
    tracing::warn!(
        path = %path.display(),
        mode = format!("{mode:o}"),
        "mobile private key was group/world accessible — tightening to 0600"
    );
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
        format!(
            "{} is group/world accessible ({mode:o}) and could not be tightened to 0600: {e}",
            path.display()
        )
    })?;
    let after = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .permissions()
        .mode()
        & 0o777;
    if after & 0o077 != 0 {
        return Err(format!(
            "{} is still group/world accessible ({after:o}) — refusing to start the iPhone server",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_key_is_owner_only(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

impl Runtime {
    fn mobile_devices_path(&self) -> std::path::PathBuf {
        self.paths.home.join("state").join("mobile-devices.json")
    }

    /// 寫入配對裝置清單。**寫檔失敗必須被呼叫端看見**：撤銷若只改了記憶體，
    /// 重啟後 token 會復活（等於撤銷沒發生）。
    ///
    /// 原子寫入（暫存檔 0600 ＋ `sync_all` ＋ `rename`）：`fs::write` 是
    /// 「先截斷再寫」，ENOSPC／崩潰會在磁碟上留下半截 JSON——下次啟動時所有
    /// 已配對的 iPhone 就無聲消失。半截檔案永遠不得取代好的那一份。
    async fn mobile_persist_devices(&self) -> Result<(), String> {
        let devices: Vec<PairedDevice> =
            self.mobile.devices.read().await.values().cloned().collect();
        let path = self.mobile_devices_path();
        let text = serde_json::to_string_pretty(&json!({ "devices": devices }))
            .map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = write_file_atomically(&tmp, &path, text.as_bytes()) {
            // 暫存檔不留垃圾；原本的清單原封不動（撤銷／配對都會誠實回 Err）。
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{}: {e}", path.display()));
        }
        // 清單重新可信：先前的「讀不到」不得繼續掛在 status 上。
        *self.mobile.devices_load_error.write().await = None;
        Ok(())
    }

    /// 讀取配對裝置清單。**「讀不到」與「解析失敗」不是「沒有配對過」**：
    /// 靜默當成空清單會讓已配對的 iPhone 無聲消失（UI 顯示「還沒有配對的
    /// iPhone」、autostart 也不開埠），而使用者無從得知狀態檔壞了。
    /// 壞掉的檔案改名保留成證據、留 audit，原因記進 status（`devicesUnknown`）。
    pub async fn mobile_load_devices(&self) -> Result<(), String> {
        let path = self.mobile_devices_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // 從來沒有配對過：這是**已知**的空，不是未知。
            // （檔案不見也可能是剛剛被隔離成 `.corrupt`——那時先前記下的原因
            // 必須留著，不得被「沒有檔案」洗成可信的空清單。）
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(self
                    .mobile_note_devices_unreadable(format!("read: {e}"), false)
                    .await)
            }
        };
        let parsed = serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("parse: {e}"))
            .and_then(|value| {
                serde_json::from_value::<Vec<PairedDevice>>(value["devices"].clone())
                    .map_err(|e| format!("devices: {e}"))
            });
        match parsed {
            Ok(devices) => {
                let mut map = self.mobile.devices.write().await;
                for d in devices {
                    map.insert(d.device_id.clone(), d);
                }
                drop(map);
                *self.mobile.devices_load_error.write().await = None;
                Ok(())
            }
            Err(e) => Err(self.mobile_note_devices_unreadable(e, true).await),
        }
    }

    /// 清單讀不到／壞掉：留證據（改名 `.corrupt`，人可以救回來）、留 audit、
    /// 記下 status 要顯示的原因。回傳同一段原因字串給呼叫端。
    async fn mobile_note_devices_unreadable(&self, error: String, quarantine: bool) -> String {
        let path = self.mobile_devices_path();
        let mut detail = json!({"path": path.display().to_string(), "error": error});
        if quarantine {
            let backup = path.with_extension("json.corrupt");
            match std::fs::rename(&path, &backup) {
                Ok(()) => detail["quarantined"] = json!(backup.display().to_string()),
                Err(e) => detail["quarantineError"] = json!(e.to_string()),
            }
        }
        *self.mobile.devices_load_error.write().await = Some(error.clone());
        tracing::warn!(
            error = %error,
            path = %path.display(),
            "paired iPhone list unreadable — reported as unknown, not as “no devices”"
        );
        self.store
            .audit("mobile.devices-load-failed", "runtime", &detail)
            .ok();
        error
    }

    /// 憑證：state/mobile-cert.der＋mobile-key.der（自簽、首次產生、指紋供 QR 釘選）。
    ///
    /// 私鑰一律只有擁有者可讀寫：建立時就以 0600 開暫存檔再 rename（過程中
    /// 不曾出現 0644 的空窗），載入時發現 group／other 位元就修正；修不掉一律
    /// 拒絕啟動——手機端只釘葉憑證指紋，私鑰外洩等於誰都能冒充這台電腦。
    fn mobile_cert(&self) -> Result<(Vec<u8>, Vec<u8>, String), String> {
        let dir = self.paths.home.join("state");
        let cert_path = dir.join("mobile-cert.der");
        let key_path = dir.join("mobile-key.der");
        if let (Ok(cert), Ok(key)) = (std::fs::read(&cert_path), std::fs::read(&key_path)) {
            ensure_key_is_owner_only(&key_path)?;
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
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(&cert_path, &cert_der)
            .map_err(|e| format!("{}: {e}", cert_path.display()))?;
        write_private_key(&key_path, &key_der)?;
        // 寫完立刻驗一次：權限設不起來就不要開伺服器（不吞錯）。
        ensure_key_is_owner_only(&key_path)?;
        let fp = sha256_hex(&cert_der);
        Ok((cert_der, key_der, fp))
    }

    /// 目前仍有配對裝置 → daemon/桌面啟動時自動起 mobile 伺服器（讓 iPhone
    /// 能重連）。沒配對過、或全部撤銷後（檔案存在但 devices 為空）就不開網路服務。
    ///
    /// 清單讀不到／壞掉是**第三種**情況：不開伺服器（沒有可信的裝置清單，
    /// 誰都認證不過），但必須留 audit ＋在 status 標成未知——不得靜默演成
    /// 「還沒有配對的 iPhone」，那會讓已配對的手機在畫面上無聲消失。
    pub fn mobile_autostart_if_paired(&self) {
        let devices = match std::fs::read_to_string(self.mobile_devices_path()) {
            Ok(text) => serde_json::from_str::<Value>(&text)
                .map_err(|e| format!("parse: {e}"))
                .and_then(|v| {
                    v["devices"]
                        .as_array()
                        .map(|a| !a.is_empty())
                        .ok_or_else(|| "devices: not an array".to_string())
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("read: {e}")),
        };
        match devices {
            Ok(true) => {
                let rt = self.clone();
                tokio::spawn(async move {
                    if let Err(e) = rt.mobile_ensure_started().await {
                        tracing::warn!(error = %e, "mobile autostart failed");
                    }
                });
            }
            Ok(false) => {}
            Err(error) => {
                let rt = self.clone();
                tokio::spawn(async move {
                    rt.mobile_note_devices_unreadable(error, false).await;
                });
            }
        }
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
        if let Err(e) = self.mobile_load_devices().await {
            // 讀不到清單不阻止伺服器啟動（記憶體表裡可能已有裝置），但錯誤
            // 不得被吞掉：status 的 `devicesUnknown` 會照實說「未知」。
            tracing::warn!(error = %e, "mobile device list unreadable — starting with what is known");
        }
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
        // 廣播關閉時只綁 loopback——測試／E2E 不得把 iPhone 伺服器開上區網。
        let bind_ip = self.mobile.bind_ip();
        let listener = match tokio::net::TcpListener::bind((bind_ip, MOBILE_PORT_DEFAULT)).await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind((bind_ip, 0))
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
                "bindIp": bind_ip,
                "error": self.mobile.advertise_off_reason(),
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
                                "bindIp": bind_ip,
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
                                "bindIp": bind_ip,
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
                        "bindIp": bind_ip,
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

        // 受器被直接停用時，仍在串流的手機必須被請去停止（感測不靜默）。
        // 只有 mobile 伺服器真的在跑時才需要這個 watcher。
        self.mobile_spawn_receptor_off_watcher();

        // Accept loop（只有走到這裡才算啟動成功）。
        // 單一 accept 錯誤不得讓整個 iPhone 伺服器悄悄消失：退避重試，
        // 只有監聽 socket 真的不可用（或連續錯太多次）才停——而且停下來要
        // 讓 status 誠實說「沒在跑」並撤掉 Bonjour。
        let rt = self.clone();
        let permits = self.mobile.conn_permits.clone();
        tokio::spawn(async move {
            let mut consecutive = 0u32;
            let stop_reason = loop {
                let accepted = match rt.mobile.take_injected_accept_error() {
                    Some(injected) => Err(injected),
                    None => listener.accept().await,
                };
                let (stream, peer) = match accepted {
                    Ok(pair) => pair,
                    Err(e) => match accept_error_action(&e, consecutive) {
                        AcceptErrorAction::RetryAfter(backoff) => {
                            consecutive += 1;
                            tracing::warn!(
                                error = %e,
                                consecutive,
                                backoff_ms = backoff.as_millis() as u64,
                                "mobile accept error — retrying"
                            );
                            tokio::time::sleep(backoff).await;
                            continue;
                        }
                        AcceptErrorAction::Stop => break e.to_string(),
                    },
                };
                consecutive = 0;
                // 連線名額：拿不到就在這裡拒絕（不是先接進來再說）。
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    rt.mobile.note_refused_connection();
                    tracing::warn!(
                        peer = %peer,
                        max = MOBILE_MAX_CONNS,
                        "mobile connection refused — connection cap reached"
                    );
                    drop(stream);
                    continue;
                };
                let acceptor = acceptor.clone();
                let rt = rt.clone();
                tokio::spawn(async move {
                    // permit 隨這個 task 結束才釋放（連線佔一個名額）。
                    let _permit = permit;
                    let handshake = async {
                        let tls = tokio::time::timeout(
                            MOBILE_TLS_HANDSHAKE_TIMEOUT,
                            acceptor.accept(stream),
                        )
                        .await
                        .map_err(|_| "TLS handshake timed out".to_string())?
                        .map_err(|e| e.to_string())?;
                        // 訊息／frame 上限：預設 64 MiB 等於讓任何 peer 用一則
                        // 訊息就吃掉記憶體。
                        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
                            max_message_size: Some(MOBILE_WS_MAX_MESSAGE_BYTES),
                            max_frame_size: Some(MOBILE_WS_MAX_MESSAGE_BYTES),
                            ..Default::default()
                        };
                        tokio::time::timeout(
                            MOBILE_WS_HANDSHAKE_TIMEOUT,
                            tokio_tungstenite::accept_async_with_config(tls, Some(config)),
                        )
                        .await
                        .map_err(|_| "WebSocket handshake timed out".to_string())?
                        .map_err(|e| e.to_string())
                    };
                    match handshake.await {
                        Ok(ws) => rt.mobile_handle_conn(ws, peer).await,
                        Err(e) => {
                            tracing::debug!(error = %e, peer = %peer, "mobile handshake failed")
                        }
                    }
                });
            };
            rt.mobile_note_accept_loop_stopped(port, &stop_reason).await;
        });
        self.mobile.started.store(true, Ordering::SeqCst);
        self.mobile_status().await
    }

    /// accept 迴圈結束（沒有任何連線再進得來）：狀態必須誠實反映「伺服器沒在
    /// 跑」，Bonjour 也要撤掉——否則區網上還在廣播一個死掉的埠，而
    /// `mobile status` 會一直說 started:true。`started` 歸零之後，下一次
    /// `mobile_ensure_started` 會乾淨地重綁。
    #[doc(hidden)]
    pub async fn mobile_note_accept_loop_stopped(&self, port: u16, reason: &str) {
        self.mobile.started.store(false, Ordering::SeqCst);
        *self.mobile.port.write().await = None;
        if let Some(daemon) = self.mobile.mdns.lock().await.take() {
            let _ = daemon.shutdown();
        }
        *self.mobile.bonjour.write().await = json!({
            "advertised": false,
            "service": MDNS_SERVICE_SHORT,
            "instance": mdns_instance_name(port),
            "error": format!("accept loop stopped: {reason}"),
        });
        tracing::error!(port, reason, "mobile accept loop stopped");
        self.store
            .audit(
                "mobile.server-stopped",
                "runtime",
                &json!({"port": port, "error": reason}),
            )
            .ok();
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
        let devices_error = self.mobile.devices_load_error().await;
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
                // 未認證連線的死線與資源上限（看得到才知道拒絕是有原因的）。
                "authTimeoutMs": self.mobile.auth_timeout_ms.load(Ordering::SeqCst),
                "maxConnections": MOBILE_MAX_CONNS,
                "refusedConnections": self.mobile.refused_connections(),
                // 撤銷之後還在敲門的手機／區網上猜 token 的 peer：看得見才查得到。
                "failedAuths": self.mobile.failed_auths(),
                "maxMessageBytes": MOBILE_WS_MAX_MESSAGE_BYTES,
                "maxInboundPerSec": MOBILE_MAX_INBOUND_PER_SEC,
            },
            "droppedObservations": self.mobile.dropped_observations(),
            // 配對清單讀不到／壞掉＝**未知**，不是「還沒有配對的 iPhone」。
            "devicesUnknown": devices_error.is_some(),
            "devicesError": devices_error,
            // 配對期被未認證 peer 燒掉的時間（null＝沒發生過）。
            "pairingBurnedAt": *self.mobile.pairing_burned_at.read().await,
            "pendingActs": self.mobile.pending_len(),
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
        // 表項在這裡就移除，所以 handler 的收尾會判定「不是我的連線」而跳過
        // 三件收斂工作——撤銷路徑必須自己做完，否則：等停止確認的人要空等到
        // 逾時、在途 act 要等滿 ACT_TIMEOUT、麥克風正在串流也不發 sensor.stopped。
        let mic_enabled = self.mobile_mic_receptor_enabled().await;
        let conn = self.mobile.conns.write().await.remove(device_id);
        let was_connected = conn.is_some();
        if let Some(conn) = conn {
            // 斷線前這台手機的串流是否「看得見」（活在 status.activeSensors 裡）：
            // 看得見的才需要補一則 sensor.stopped，事件面與 status 保持一致。
            let was_streaming =
                conn.mic_since.is_some() && (mic_enabled || conn.stop_sensors.was_requested());
            conn.stop_sensors.mark_disconnected();
            conn.close.cancel();
            let ended = self.mobile.fail_pending_for_device(device_id);
            if ended > 0 {
                tracing::info!(
                    device_id,
                    ended,
                    "iPhone revoked — in-flight acts end with an unknown outcome"
                );
            }
            if was_streaming {
                self.events.emit(
                    EventType::SensorStopped,
                    json!({
                        "sensor": "iphone.mic-level",
                        "deviceId": device_id,
                        "source": "iphone",
                        "reason": "revoked",
                    }),
                );
            }
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
        // 撤銷＝立刻退出 Character Session（不是 offline：這台裝置不再是成員）。
        self.character_session_leave(&interaction_aip::Party::device(device_id))
            .await;
        self.mobile_disable_high_risk_receptors(device_id, "revoked")
            .await;
        // 全域受器被關掉之後，另一台仍在串流的手機不得從 activeSensors 無聲
        // 消失：請它也停止感測，並照實顯示「停止中／結果未知」直到它確認。
        self.mobile_stop_other_streaming_phones(Some(device_id), "revoked")
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
            // 這一則 `receptor.offline` 是 runtime 自己造成的：watcher 要略過
            // （這條路徑的呼叫端隨後就會送停止請求，不必也不得重送）。
            self.mobile.note_self_receptor_disable(spec.id);
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
            } else {
                // 標記沒有對應的事件：立刻收回，否則會吃掉之後**人類**發動的停用。
                self.mobile.take_self_receptor_disable(spec.id);
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

    /// 停止手機端感測的共同路徑（「停止所有感測」與緊急停止共用）：
    /// (a) 桌面端把高風險受器強制 disabled（重啟／重連不自動恢復）；
    /// (b) 對所有手機送 `stop-all { sensors: true }` 並**有界等待**每台確認。
    /// 沒有手機連線時什麼都不做（誠實：沒有東西被停，回空清單）。
    ///
    /// audit 記的是每台的 outcome（stopped／unknown／unreachable），
    /// 不是「有沒有排進出站佇列」——排進佇列不等於手機停了。
    pub(crate) async fn mobile_stop_sensors(
        &self,
        actor: &str,
        audit_kind: &str,
        reason: &str,
    ) -> Vec<MobileStopOutcome> {
        self.mobile_disable_high_risk_receptors("*", reason).await;
        if !self.mobile.any_connected().await {
            return Vec::new();
        }
        let devices = self
            .mobile
            .stop_sensors_and_wait(STOP_SENSORS_WAIT, stop_all_wire_reason(reason))
            .await;
        self.store
            .audit(
                audit_kind,
                actor,
                &json!({
                    "sensors": true,
                    "reason": reason,
                    "waitedMsBudget": STOP_SENSORS_WAIT.as_millis() as u64,
                    "devices": devices,
                }),
            )
            .ok();
        devices
    }

    /// 緊急停止 → 手機端也必須停止「感測」，不只是動器：在途 act 立刻以
    /// stopped 收場，再走與「停止所有感測」相同的請求＋等待確認路徑。
    pub(crate) async fn mobile_estop_stop_sensors(&self, actor: &str) -> Vec<MobileStopOutcome> {
        self.mobile.fail_inflight_stopped();
        self.mobile_stop_sensors(actor, "mobile.estop-stop-sensors", "emergency-stop")
            .await
    }

    /// 緊急停止**期間**才連上／重連的「這一台」手機：緊急停止在它連上之前就
    /// 發生了，所以那一輪的 `stop-all` 它沒收到。角色標籤只是說明，真正要做的
    /// 是請它停止感測——否則它會在緊急停止全程繼續擷取音訊。
    /// 誠實階梯照舊：只有手機確認才算 stopped，逾時是 unknown（可能仍在錄）。
    pub(crate) async fn mobile_estop_stop_sensors_device(
        &self,
        device_id: &str,
        reason: &str,
    ) -> Option<MobileStopOutcome> {
        // 高風險受器維持 disabled：重連不得讓它自動恢復。
        self.mobile_disable_high_risk_receptors(device_id, reason)
            .await;
        let outcome = self
            .mobile
            .stop_sensors_for(Some(device_id), STOP_SENSORS_WAIT, STOP_REASON_EMERGENCY)
            .await
            .into_iter()
            .next();
        self.store
            .audit(
                "mobile.estop-stop-sensors",
                "runtime",
                &json!({
                    "sensors": true,
                    "reason": reason,
                    "deviceId": device_id,
                    "waitedMsBudget": STOP_SENSORS_WAIT.as_millis() as u64,
                    "devices": outcome.clone().map(|o| vec![o]).unwrap_or_default(),
                }),
            )
            .ok();
        outcome
    }

    /// 高風險受器是**全域**開關：為了「這一台」斷線／撤銷而把它關掉之後，桌面
    /// 就不再收任何手機的音量資料了——另一台仍在串流的手機不能因此從
    /// `activeSensors` 無聲消失（消失＝宣稱它停了），也不該繼續錄一份沒有人
    /// 在收的音。對每一台仍在串流、且還沒有待確認停止請求的其他手機送出停止
    /// 請求：它會留在 `activeSensors` 標「停止中／結果未知」直到手機確認為止。
    async fn mobile_stop_other_streaming_phones(&self, excluding: Option<&str>, reason: &str) {
        let others: Vec<String> = self
            .mobile
            .mic_streaming_devices()
            .await
            .into_iter()
            // 已經有待確認的停止請求就不重送（重送會把已確認的等待重新打開）。
            .filter(|(id, _, stop_pending)| {
                excluding != Some(id.as_str()) && stop_pending.is_none()
            })
            .map(|(id, _, _)| id)
            .collect();
        if others.is_empty() {
            return;
        }
        // 每台各自有界等待、彼此並行：一台沒回應不得拖住另一台。
        // wire reason 是「使用者發起」——這不是緊急停止，手機不得顯示成緊急停止。
        let devices: Vec<MobileStopOutcome> =
            futures_util::future::join_all(others.iter().map(|id| {
                self.mobile
                    .stop_sensors_for(Some(id), STOP_SENSORS_WAIT, STOP_REASON_USER)
            }))
            .await
            .into_iter()
            .flatten()
            .collect();
        self.store
            .audit(
                "mobile.sensors-stop-shared-receptor-off",
                "runtime",
                &json!({
                    "reason": reason,
                    "triggeredBy": excluding,
                    "receptor": "iphone.mic-level",
                    "waitedMsBudget": STOP_SENSORS_WAIT.as_millis() as u64,
                    "devices": devices,
                }),
            )
            .ok();
    }

    /// 感測不靜默（對抗審查 safety-invariants-056）：使用者／AI 直接把
    /// `iphone.mic-level` 停用時（CapabilityCard 的「停用」、PATCH
    /// `/v1/receptors/{id}`、Tauri `set_receptor_enabled`、CLI），registry 只翻
    /// 一個旗標——手機不會知道，於是它**繼續錄**，桌面卻因為受器 disabled 把
    /// 它從 `activeSensors` 濾掉（tray／可信 overlay／首頁一起變成「沒有感測
    /// 在用」）＝畫面宣稱已停、實體仍在擷取。
    ///
    /// 這個 watcher 監聽 `receptor.offline`，替那條路徑補上唯一正確的動作：
    /// 對每一台仍在串流的手機送停止請求（reason=user）。手機因此會留在
    /// `activeSensors` 標「停止中／結果未知」直到它確認——誠實：未知≠已停。
    /// runtime 自己關（斷線／撤銷／緊急停止）的那些已在原路徑送過停止請求，
    /// 由 `note_self_receptor_disable` 標記後在此略過，不重複廣播。
    fn mobile_spawn_receptor_off_watcher(&self) {
        // accept 迴圈停掉之後重新啟動伺服器時，不得疊出第二個 watcher。
        if self.mobile.receptor_watcher.swap(true, Ordering::SeqCst) {
            return;
        }
        let rt = self.clone();
        let mut rx = self.events.subscribe();
        tokio::spawn(async move {
            loop {
                let event = match rx.recv().await {
                    Ok(event) => event,
                    // 落後只代表漏了幾則事件，不是結束：繼續聽下一則。
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if event.event_type != EventType::ReceptorOffline {
                    continue;
                }
                let Some(receptor_id) = event.payload["receptorId"].as_str() else {
                    continue;
                };
                if !MOBILE_RECEPTOR_SPECS
                    .iter()
                    .any(|spec| spec.high_risk && spec.id == receptor_id)
                {
                    continue;
                }
                if rt.mobile.take_self_receptor_disable(receptor_id) {
                    continue;
                }
                rt.mobile_stop_other_streaming_phones(None, "receptor-disabled")
                    .await;
            }
        });
    }

    /// 感測不靜默：手機端正在串流的高風險感測也要出現在 `status.activeSensors`
    /// （tray／首頁／角色視窗都吃這個欄位）。條件三者皆須成立：
    /// receptor 啟用中 ∧ 手機連線中 ∧ 手機自報 `sensors.micLevel == true`。
    pub(crate) async fn mobile_active_sensors(&self) -> Vec<crate::sensors::SensorUse> {
        // registry 對 disabled／未註冊的 receptor 回 Err —— 這就是「啟用中」。
        let enabled = self
            .registry
            .receptor(&ReceptorId::new("iphone.mic-level"))
            .await
            .is_ok();
        self.mobile
            .mic_streaming_devices()
            .await
            .into_iter()
            .filter_map(|(device_id, since, stop_pending)| {
                // 已要求停止但手機還沒確認：不得從畫面上消失（消失＝宣稱已停）。
                let (state, purpose) = match stop_pending {
                    Some(at) if at.elapsed() < STOP_SENSORS_WAIT => (
                        crate::sensors::SENSOR_STATE_STOPPING,
                        "iPhone 麥克風音量：停止中（等待 iPhone 確認）".to_string(),
                    ),
                    Some(_) => (
                        crate::sensors::SENSOR_STATE_STOP_UNKNOWN,
                        "iPhone 麥克風音量：停止結果未知（iPhone 未回覆，可能仍在擷取）"
                            .to_string(),
                    ),
                    None => (
                        crate::sensors::SENSOR_STATE_ACTIVE,
                        "iPhone 麥克風音量（僅音量值）".to_string(),
                    ),
                };
                // 受器已停用（例如緊急停止後）→ 桌面不再收資料；只有在
                // 「已要求停止、手機尚未確認」時才仍要顯示（誠實：未知≠已停）。
                if !enabled && state == crate::sensors::SENSOR_STATE_ACTIVE {
                    return None;
                }
                Some(crate::sensors::SensorUse {
                    kind: "iphone.mic-level".into(),
                    started_at: since,
                    started_by: format!("iphone:{device_id}"),
                    purpose,
                    auto_stop_at: None,
                    state: state.to_string(),
                })
            })
            .collect()
    }

    /// Runtime 專屬的角色真相投影（**單一台**）：送 `character.present` 並在
    /// 有界時間內等 ack。誠實階梯：排進佇列 ≠ 手機顯示了；沒回 ack ＝結果未知。
    async fn mobile_project_character_one(
        &self,
        device_id: &str,
        params: Value,
        id_prefix: &str,
        wait: Duration,
    ) -> Value {
        let action_id = format!("{id_prefix}-{}", token_hex(4));
        let dispatched = self
            .mobile
            .dispatch_act(Some(device_id), "character.present", params, &action_id)
            .await;
        match dispatched {
            Err(e) => json!({"deviceId": device_id, "outcome": "unreachable", "reason": e}),
            Ok((_, rx)) => match self.mobile.await_reply(&action_id, rx, wait).await {
                Some(reply) if reply["type"] == "ack" => {
                    json!({"deviceId": device_id, "outcome": "acknowledged"})
                }
                Some(reply) => json!({
                    "deviceId": device_id,
                    "outcome": "refused",
                    "reason": reply["reason"].as_str().unwrap_or("iPhone refused"),
                }),
                None => json!({"deviceId": device_id, "outcome": "unknown"}),
            },
        }
    }

    /// Runtime 專屬的角色真相投影（**每一台已連線手機**，同時進行、共用有界
    /// 等待）。plan／agent 永遠走不到這裡——`map_wire_params` 對這些狀態一律拒絕。
    async fn mobile_project_character(
        &self,
        params: Value,
        id_prefix: &str,
        wait: Duration,
    ) -> Vec<Value> {
        let device_ids: Vec<String> = self.mobile.conns.read().await.keys().cloned().collect();
        futures_util::future::join_all(
            device_ids
                .iter()
                .map(|id| self.mobile_project_character_one(id, params.clone(), id_prefix, wait)),
        )
        .await
    }

    /// 真正的緊急停止 → 每一台手機的角色都要變成「緊急停止中」；解除 → 回 idle。
    /// 這是 `emergency` 狀態**唯一**的來源（AI 不得冒充緊急停止）。
    pub(crate) async fn mobile_project_estop(&self, engaged: bool) -> Vec<Value> {
        if !self.mobile.any_connected().await {
            return Vec::new();
        }
        let state = if engaged { EMERGENCY_STATE } else { "idle" };
        let devices = self
            .mobile_project_character(
                json!({"state": state, "source": "runtime-estop"}),
                if engaged { "estop" } else { "estop-clear" },
                CHARACTER_PROJECT_WAIT,
            )
            .await;
        self.store
            .audit(
                "mobile.character-emergency",
                "runtime",
                &json!({"state": state, "engaged": engaged, "devices": devices}),
            )
            .ok();
        devices
    }

    /// 只投影給「這一台」（estop 期間才連上來的手機）。
    pub(crate) async fn mobile_project_estop_device(&self, device_id: &str, reason: &str) -> Value {
        let outcome = self
            .mobile_project_character_one(
                device_id,
                json!({"state": EMERGENCY_STATE, "source": "runtime-estop"}),
                "estop",
                CHARACTER_PROJECT_WAIT,
            )
            .await;
        self.store
            .audit(
                "mobile.character-emergency",
                "runtime",
                &json!({
                    "state": EMERGENCY_STATE,
                    "engaged": true,
                    "reason": reason,
                    "devices": [outcome.clone()],
                }),
            )
            .ok();
        outcome
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
        // 多台手機都該看到同一個真相：逐台送、逐台記結果（不挑第一台）。
        let devices = self
            .mobile_project_character(
                json!({
                    "state": VERIFIED_STATE,
                    "source": "human-verification",
                    "agentSessionId": agent_session_id,
                }),
                "verified",
                ACT_TIMEOUT,
            )
            .await;
        let first = devices.first().cloned().unwrap_or(Value::Null);
        let reply_kind = match first["outcome"].as_str() {
            Some("acknowledged") => "ack",
            Some("refused") => "err",
            _ => "none",
        };
        self.store.audit(
            "mobile.present-verified",
            "user",
            &json!({
                "agentSessionId": agent_session_id,
                "reply": reply_kind,
                "devices": devices,
            }),
        )?;
        // 手機回 err（含與 stop-all 競態的 `stopped`）＝綠勾沒上去：誠實回 Err，
        // 不得因為「有回覆」就當成成功（acknowledged ≠ completed）。
        if !devices
            .iter()
            .any(|d| d["outcome"] == json!("acknowledged"))
        {
            let why = first["reason"]
                .as_str()
                .unwrap_or("iPhone did not acknowledge");
            return Err(DomainError::Unavailable(format!(
                "iPhone did not acknowledge the verified badge ({why}) — it was not shown"
            )));
        }
        Ok(json!({"state": VERIFIED_STATE, "devices": devices}))
    }

    /// 只停「這一台」手機的感測（連接頁的每機動作）。
    /// 未配對＝NotFound；沒連線＝誠實回 unreachable（沒有任何東西被停）。
    pub async fn mobile_sensors_stop(&self, device_id: &str) -> DomainResult<Value> {
        if !self.mobile.devices.read().await.contains_key(device_id) {
            return Err(DomainError::NotFound(format!("mobile device {device_id}")));
        }
        if !self.mobile.conns.read().await.contains_key(device_id) {
            self.store
                .audit(
                    "mobile.sensors-stop-not-delivered",
                    "user",
                    &json!({"deviceId": device_id, "reason": "not connected"}),
                )
                .ok();
            return Ok(json!({
                "deviceId": device_id,
                "requested": false,
                "connected": false,
                "outcome": StopOutcome::Unreachable.as_str(),
            }));
        }
        // 連接頁的每機動作一律是使用者發起的：手機要顯示「由桌面停止全部感測」，
        // 不是「因桌面緊急停止而停用」。
        let outcomes = self
            .mobile
            .stop_sensors_for(Some(device_id), STOP_SENSORS_WAIT, STOP_REASON_USER)
            .await;
        let outcome = outcomes.into_iter().next();
        let (result, waited_ms, via) = match &outcome {
            Some(o) => (o.outcome, o.waited_ms, o.via.clone()),
            // 快照到送出之間手機剛好斷線：沒有東西被停。
            None => (StopOutcome::Unreachable, 0, None),
        };
        self.store.audit(
            "mobile.sensors-stop",
            "user",
            &json!({
                "deviceId": device_id,
                "outcome": result.as_str(),
                "waitedMs": waited_ms,
                "via": via,
            }),
        )?;
        // 注意：這裡**不**動 `iphone.mic-level` 受器——受器是全域開關，為了停
        // 一台而停用它，會讓另一台仍在串流的手機從 activeSensors 消失（＝感測
        // 靜默）。要「停到不能再開」請用「停止所有感測」／緊急停止。
        Ok(json!({
            "deviceId": device_id,
            "requested": true,
            "connected": true,
            // stopped＝手機確認；unknown＝沒回覆（可能仍在擷取）；
            // unreachable＝根本沒送出去。
            "outcome": result.as_str(),
            "waitedMs": waited_ms,
            "via": via,
        }))
    }

    /// 「測試這台手機」：送一則 WebSocket Ping 並等對應的 Pong。
    /// **ok 只代表 socket 有回答**——不代表 App 的感測／動器功能正常。
    pub async fn mobile_test(&self, device_id: &str) -> DomainResult<Value> {
        if self.is_estopped() {
            return Err(DomainError::PolicyBlocked(
                "emergency stop engaged; nothing is sent to the iPhone".into(),
            ));
        }
        if !self.mobile.devices.read().await.contains_key(device_id) {
            return Err(DomainError::NotFound(format!("mobile device {device_id}")));
        }
        if !self.mobile.conns.read().await.contains_key(device_id) {
            self.store
                .audit(
                    "mobile.test",
                    "user",
                    &json!({"deviceId": device_id, "ok": false, "reason": "not-connected"}),
                )
                .ok();
            return Ok(json!({
                "deviceId": device_id,
                "ok": false,
                "connected": false,
                "reason": "not-connected",
            }));
        }
        let result = self
            .mobile
            .ping_device(device_id, MOBILE_TEST_TIMEOUT)
            .await;
        let value = match result {
            Ok(Some(latency_ms)) => json!({
                "deviceId": device_id,
                "ok": true,
                "connected": true,
                "latencyMs": latency_ms,
                "note": "只代表連線有回應，不代表 App 功能正常",
            }),
            Ok(None) => json!({
                "deviceId": device_id,
                "ok": false,
                "connected": true,
                "uncertain": true,
                "reason": "no-pong-within-3s",
            }),
            // 送不出去：可能是剛好斷線（那就誠實說沒連線），也可能佇列滿。
            Err(e) => json!({
                "deviceId": device_id,
                "ok": false,
                "connected": !e.contains("not connected"),
                "uncertain": true,
                "reason": e,
            }),
        };
        self.store.audit("mobile.test", "user", &value)?;
        Ok(value)
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
                // 身分指紋 ≠ 認證驗證值（見 `mobile_identity_fingerprint`）。
                fingerprint: Some(mobile_identity_fingerprint(device)),
                human: None,
            },
            state: ProviderState::Available,
            receptors: MOBILE_RECEPTORS.iter().map(|s| s.to_string()).collect(),
            // 這六項不是「這一台專屬」的能力，而是所有已配對 iPhone 共用的
            // 同一組動作能力；descriptor 不得讓人以為每台各有一份。
            actuators: MOBILE_ACTUATORS
                .iter()
                .map(|(id, _, _)| id.to_string())
                .collect(),
            tool_operations: vec![],
            paired_at: Some(device.paired_at),
            last_seen: Some(Utc::now()),
            detail: Some(mobile_provider_note(&device.device_id)),
        };
        let _ = self.providers.register(descriptor).await;
        let pid = ProviderId::new(format!("provider.mobile.{}", device.device_id));
        // transition 會覆寫 detail：註記要一起帶過去，否則「能力是共用的」這件
        // 事會在連上線的那一刻消失。
        let _ = self
            .providers
            .transition(
                &pid,
                ProviderState::Available,
                Some(mobile_provider_note(&device.device_id)),
            )
            .await;
        // Character Protocol §11：iPhone 連上 → greet（device-online）。
        self.character_project_provider(&pid, ProviderState::Available);
    }

    /// 登記已認證連線；同一台手機的舊連線（多半半開）被新連線取代時踢掉，
    /// 避免舊 handler 收尾時誤刪新連線的表項。
    #[allow(clippy::too_many_arguments)]
    async fn mobile_attach_conn(
        &self,
        device_id: &str,
        conn_id: u64,
        out_tx: &mpsc::Sender<Message>,
        close: &CancellationToken,
        stop_sensors: &Arc<StopSensorsTracker>,
        ping_waiters: &PingWaiters,
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
                stop_sensors: stop_sensors.clone(),
                ping_waiters: ping_waiters.clone(),
            },
        ) {
            // 舊連線的等待者不能永遠掛著：標記斷線讓它們立刻收斂成 unreachable。
            old.stop_sensors.mark_disconnected();
            old.close.cancel();
        }
    }

    /// `iphone.mic-level` 受器是否啟用中（registry 對 disabled／未註冊回 Err）。
    async fn mobile_mic_receptor_enabled(&self) -> bool {
        self.registry
            .receptor(&ReceptorId::new("iphone.mic-level"))
            .await
            .is_ok()
    }

    /// 手機麥克風串流狀態變化（自報 status 或實際推來觀察）。
    /// 只有「真的變化」才發事件：30 秒心跳的 status 不得洗版事件流。
    /// 回傳 true＝這次真的有變化。
    async fn mobile_note_mic_state(&self, device_id: &str, conn_id: u64, on: bool) -> bool {
        let (changed, stop_requested) = {
            let mut conns = self.mobile.conns.write().await;
            match conns.get_mut(device_id) {
                Some(conn) if conn.conn_id == conn_id => {
                    let was = conn.mic_since.is_some();
                    conn.mic_since = match (on, conn.mic_since) {
                        (true, Some(since)) => Some(since),
                        (true, None) => Some(Utc::now()),
                        (false, _) => None,
                    };
                    (was != on, conn.stop_sensors.was_requested())
                }
                _ => (false, false),
            }
        };
        // 事件面與 `status.activeSensors` 必須一致：受器停用、又沒有待確認的
        // 停止請求時，手機自報的串流本來就不進桌面感測面（既有設計），
        // 那就不該只有事件在叫。
        if changed && (stop_requested || self.mobile_mic_receptor_enabled().await) {
            // 感測不靜默：手機端的開始／停止也要進事件流（tray／overlay／SSE
            // 才不必等 4 秒輪詢）。
            self.events.emit(
                if on {
                    EventType::SensorStarted
                } else {
                    EventType::SensorStopped
                },
                json!({
                    "sensor": "iphone.mic-level",
                    "deviceId": device_id,
                    "source": "iphone",
                }),
            );
        }
        changed
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
        // 這條連線的「停止感測」確認追蹤與 Ping 等待表（收尾時要收斂，不留掛單）。
        let stop_sensors = Arc::new(StopSensorsTracker::default());
        let ping_waiters: PingWaiters = Arc::new(std::sync::Mutex::new(Vec::new()));
        // 伺服器主動關閉的原因（撤銷／被新連線取代）；None＝對端斷線或閒置逾時。
        let mut closed_by_server: Option<&'static str> = None;
        let ping_interval = self.mobile.ping_interval();
        let idle_timeout = self.mobile.idle_timeout();
        let mut last_seen = Instant::now();
        // 未認證連線的絕對死線：Ping／Pong／未知訊息都不能續命。
        let auth_deadline = tokio::time::Instant::now() + self.mobile.auth_timeout();
        // 每條連線的入站速率窗（超過即關閉，留 audit）。
        let mut rate_window = Instant::now();
        let mut rate_count: u32 = 0;

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
                // 未認證卻一直賴著（送 Ping／未知訊息刷新 last_seen）：到期即關。
                _ = tokio::time::sleep_until(auth_deadline), if authed.is_none() => {
                    tracing::info!(
                        peer = %peer,
                        "mobile connection closed — not paired or authenticated in time"
                    );
                    self.store
                        .audit(
                            "mobile.unauthenticated-timeout",
                            "runtime",
                            &json!({
                                "peer": peer.to_string(),
                                "afterMs": self.mobile.auth_timeout().as_millis() as u64,
                            }),
                        )
                        .ok();
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
            // 入站速率上限：一台手機的正常流量遠低於此；超過就是濫用。
            if rate_window.elapsed() >= Duration::from_secs(1) {
                rate_window = Instant::now();
                rate_count = 0;
            }
            rate_count += 1;
            if rate_count > MOBILE_MAX_INBOUND_PER_SEC {
                tracing::warn!(
                    peer = %peer,
                    device = authed.as_deref().unwrap_or("-"),
                    limit = MOBILE_MAX_INBOUND_PER_SEC,
                    "mobile inbound rate limit exceeded — closing"
                );
                self.store
                    .audit(
                        "mobile.rate-limited",
                        "runtime",
                        &json!({
                            "peer": peer.to_string(),
                            "deviceId": authed.clone(),
                            "limitPerSec": MOBILE_MAX_INBOUND_PER_SEC,
                        }),
                    )
                    .ok();
                break;
            }
            let text = match msg {
                Message::Text(text) => text,
                Message::Close(_) => break,
                // 「測試這台手機」：只有 nonce 相同的 Pong 能解除等待
                // （心跳 Ping 是空 payload，不會誤判成測試回應）。
                Message::Pong(payload) => {
                    let waiter = {
                        let mut waiters = ping_waiters
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        waiters
                            .iter()
                            .position(|(nonce, _)| *nonce == payload)
                            .map(|i| waiters.remove(i).1)
                    };
                    if let Some(tx) = waiter {
                        let _ = tx.send(Instant::now());
                    }
                    continue;
                }
                _ => continue, // Ping/Binary：只當活著的證據
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
                    self.mobile_attach_conn(
                        &device_id,
                        conn_id,
                        &out_tx,
                        &close,
                        &stop_sensors,
                        &ping_waiters,
                    )
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
                        // 認證失敗要留痕：撤銷之後手機持續重連、或區網上有人拿
                        // 猜來的 deviceId／token 試，都必須在 audit 與 status 上
                        // 看得見（token 本身永遠不記）。
                        self.mobile.note_failed_auth();
                        let known = self.mobile.devices.read().await.contains_key(&device_id);
                        self.store
                            .audit(
                                "mobile.auth-failed",
                                "runtime",
                                &json!({
                                    "peer": peer.to_string(),
                                    "deviceId": device_id,
                                    // 沒有這台裝置＝可能是已撤銷的手機還在重連，
                                    // 也可能是區網上有人在猜 id。
                                    "knownDevice": known,
                                }),
                            )
                            .ok();
                        send(&out_tx, json!({"type":"auth-fail","reason":"unknown device or bad token (possibly revoked)"})).await;
                        break;
                    }
                    let device = self.mobile.devices.read().await.get(&device_id).cloned();
                    if let Some(device) = device {
                        self.mobile_register_provider(&device).await;
                    }
                    self.mobile_attach_conn(
                        &device_id,
                        conn_id,
                        &out_tx,
                        &close,
                        &stop_sensors,
                        &ping_waiters,
                    )
                    .await;
                    authed = Some(device_id.clone());
                    send(&out_tx, json!({"type":"auth-ok"})).await;
                    // 緊急停止中連上／重連的手機：先「真的要求它停止感測」
                    // （stop-all { sensors: true }），再投影緊急狀態。只換一個
                    // 角色文字標籤不會關掉手機的麥克風／位置／BLE 閘道——
                    // 緊急停止必須連感測一起停，新連線也不例外。
                    // 另開 task：ack 由這條迴圈接收，不能在這裡等自己。
                    if self.is_estopped() {
                        let rt = self.clone();
                        tokio::spawn(async move {
                            rt.mobile_estop_stop_sensors_device(
                                &device_id,
                                "reconnect-during-estop",
                            )
                            .await;
                            rt.mobile_project_estop_device(&device_id, "reconnect-during-estop")
                                .await;
                        });
                    }
                }
                // AIP Character Session（`docs/aip/README.md` §9.1）：只在 auth-ok 之後
                // 接受；身分＝配對出來的 deviceId，宣稱不符一律拒絕。
                Some("aip") if authed.is_some() => {
                    if let Some(device_id) = authed.clone() {
                        let outcome = self.character_session_device_frame(&device_id, &v).await;
                        for reply in outcome.replies {
                            send(&out_tx, json!({"type":"aip","envelope": reply})).await;
                        }
                        // recipe 相容：已套用的觸碰同時落成一筆 `iphone.touch` 觀察
                        // （只此一次；重複／過期／被拒的訊息不會走到這裡）。
                        if let Some(kind) = outcome.applied_touch {
                            let mut facts = BTreeMap::new();
                            facts.insert("kind".to_string(), json!(kind));
                            if let Err(e) = self
                                .ingest("iphone.touch", facts, BTreeMap::new(), 1.0)
                                .await
                            {
                                self.mobile.note_dropped("iphone.touch", &e.to_string());
                            }
                        }
                    }
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
                    // iOS 把語意事件自己的時間戳放在訊息**頂層**的 `at`
                    // （Protocol.swift），不在 `facts` 裡。manifest 宣告
                    // `provides: ["event","at"]` 就必須真的收得到它，否則對外
                    // 宣稱了一個永遠不會出現的欄位。白名單照舊管用：`at` 只有在
                    // 該 receptor 的 provides 裡才留得下來；facts 自己帶的 `at`
                    // 優先（不覆寫手機明確放進 facts 的值）。
                    let mut incoming = v["facts"].clone();
                    if let (Some(facts), Some(at)) = (incoming.as_object_mut(), v.get("at")) {
                        facts.entry("at").or_insert_with(|| at.clone());
                    }
                    match filter_mobile_facts(receptor, &incoming) {
                        None => self
                            .mobile
                            .note_dropped(receptor, "receptor not in whitelist"),
                        Some(facts) if facts.is_empty() => {
                            self.mobile.note_dropped(receptor, "no accepted fact keys")
                        }
                        Some(facts) => {
                            match self.ingest(receptor, facts, BTreeMap::new(), 1.0).await {
                                Err(e) => self.mobile.note_dropped(receptor, &e.to_string()),
                                Ok(_) => {
                                    // 感測不靜默：資料真的在流進來，就算手機的
                                    // status 還沒到（或根本沒送），activeSensors
                                    // 也不得是空的。
                                    if receptor == "iphone.mic-level" {
                                        if let Some(device_id) = authed.as_deref() {
                                            self.mobile_note_mic_state(device_id, conn_id, true)
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // stop-all 的確認 ack 沒有 id（iOS Protocol.swift），不能走
                // resolve_pending：它是「這台手機停了感測」的唯一明確證據。
                Some("ack") if v["stopAll"] == true && authed.is_some() => {
                    stop_sensors.confirm("ack");
                    if let Some(device_id) = authed.as_deref() {
                        self.mobile_note_mic_state(device_id, conn_id, false).await;
                    }
                }
                // 回覆類：只有已認證且是 act 目標的那台手機才能解除 pending。
                Some("ack") | Some("err") | Some("ble.result") | Some("ble.value")
                    if authed.is_some() =>
                {
                    if let Some(device_id) = authed.as_deref() {
                        self.mobile.resolve_pending(device_id, &v);
                    }
                }
                Some("status") if authed.is_some() => {
                    if let Some(device_id) = &authed {
                        let mic_on = v["sensors"]["micLevel"] == json!(true);
                        {
                            let mut conns = self.mobile.conns.write().await;
                            if let Some(conn) = conns.get_mut(device_id) {
                                if conn.conn_id == conn_id {
                                    conn.status = v.clone();
                                }
                            }
                        }
                        // 感測不靜默：手機自報麥克風串流中 → 桌面端 status／
                        // tray／首頁／角色視窗都要看得到（起算時間只在「關→開」
                        // 時重設），而且開／關都要發事件。
                        self.mobile_note_mic_state(device_id, conn_id, mic_on).await;
                        // 「已要求停止」之後才到的 `micLevel:false` 才算確認
                        // （心跳 status 在請求之前送出的不算——以請求時間為界）。
                        if !mic_on {
                            stop_sensors.confirm("status");
                        }
                    }
                }
                _ => {}
            }
        }
        // 斷線收尾：只移除自己的表項；provider Disconnected（已 Revoked／被取代
        // 則不轉，避免非法轉移）；高風險受器強制 disabled，不自動恢復。
        if let Some(device_id) = authed {
            // 斷線前這台手機的串流是否「看得見」（活在 status.activeSensors 裡）：
            // 看得見的才需要補一則 sensor.stopped，事件面與 status 保持一致。
            let mic_enabled = self.mobile_mic_receptor_enabled().await;
            let (was_active, was_streaming) = {
                let mut conns = self.mobile.conns.write().await;
                match conns.get(&device_id) {
                    Some(c) if c.conn_id == conn_id => {
                        let streaming = c.mic_since.is_some()
                            && (mic_enabled || c.stop_sensors.was_requested());
                        conns.remove(&device_id);
                        (true, streaming)
                    }
                    _ => (false, false),
                }
            };
            if was_active {
                // 等停止確認的人立刻收斂成 unreachable（不再空等到逾時），
                // 送給這台手機的在途 act 也立刻以 disconnected 收場——結果
                // 一樣是未知，但呼叫端不必等滿 4 秒。
                stop_sensors.mark_disconnected();
                let ended = self.mobile.fail_pending_for_device(&device_id);
                if ended > 0 {
                    tracing::info!(
                        device_id,
                        ended,
                        "iPhone disconnected — in-flight acts end with an unknown outcome"
                    );
                }
                // 斷線＝不再有任何感測資料流入：指示燈必須熄，而且要發事件
                // （tray／overlay 才不必等 4 秒輪詢）。
                if was_streaming {
                    self.events.emit(
                        EventType::SensorStopped,
                        json!({
                            "sensor": "iphone.mic-level",
                            "deviceId": device_id,
                            "source": "iphone",
                            "reason": "disconnected",
                        }),
                    );
                }
            }
            // AIP Character Session：斷線＝presence offline（成員保留，等 tick 逾時
            // 才清）；重連後要重送 capability 才能再送 event。
            if was_active {
                self.character_session_presence(
                    &interaction_aip::Party::device(&device_id),
                    interaction_session::Presence::Offline,
                )
                .await;
            }
            // 等待中的 Ping 不留掛單（連線已經沒了）。
            ping_waiters
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
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
                        .await;
                    self.mobile_stop_other_streaming_phones(Some(&device_id), reason)
                        .await;
                }
                None if was_active => {
                    self.mobile_disable_high_risk_receptors(&device_id, "disconnected")
                        .await;
                    self.mobile_stop_other_streaming_phones(Some(&device_id), "disconnected")
                        .await;
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
    ///
    /// `device_id` 指名要哪一台手機代掃；`None` 時只有**恰好一台**手機連線才
    /// 成立——多台連線時誠實回 Err（列出連線中的 id），絕不替使用者猜一台。
    pub async fn mobile_ble_scan(
        &self,
        duration_ms: u64,
        device_id: Option<&str>,
    ) -> DomainResult<Value> {
        // BLE 掃描是一次對外的無線探測（比送一則 Ping 風險更高）：緊急停止
        // 生效時什麼都不送到手機，與 `mobile_test`／`mobile_present_verified` 同。
        if self.is_estopped() {
            return Err(DomainError::PolicyBlocked(
                "emergency stop engaged; nothing is sent to the iPhone".into(),
            ));
        }
        let duration_ms = duration_ms.clamp(500, 8_000);
        let id = format!("ble-scan-{}", token_hex(4));
        let msg = json!({"type":"ble.scan","id":id,"durationMs":duration_ms});
        let target = device_id.map(str::trim).filter(|d| !d.is_empty());
        let (_device_id, rx) = self
            .mobile
            .dispatch(target, &id, msg)
            .await
            .map_err(DomainError::Unavailable)?;
        let timeout = Duration::from_millis(duration_ms) + BLE_REPLY_GRACE;
        let reply = self
            .mobile
            .await_reply(&id, rx, timeout)
            .await
            .ok_or_else(|| {
                DomainError::Unavailable(format!(
                    "iPhone did not answer the BLE scan within {}ms (outcome unknown)",
                    timeout.as_millis()
                ))
            })?;
        // 手機回 err（閘道關閉／stop-all 競態／等待中斷線）不是掃描結果：
        // 誠實回 Err，不得讓呼叫端把它當成「掃到 0 台」。
        if reply["type"] == "err" {
            let why = reply["reason"].as_str().unwrap_or("iPhone refused");
            return Err(DomainError::Unavailable(format!(
                "iPhone did not complete the BLE scan ({why}) — outcome unknown"
            )));
        }
        Ok(reply)
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
