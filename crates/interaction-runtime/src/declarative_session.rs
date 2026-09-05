//! 宣告式裝置（serial／mqtt／ble）→ Character Session 的綁定（M2 §3.3）。
//!
//! 為什麼要有這個模組：在此之前，AIP Character Session 只有**一條**入口——
//! `mobile.rs` 的 iPhone wss 迴圈。第二種裝置要進來，就得在核心裡再寫一次
//! 「哪一種傳輸→哪一個 party→哪一個 session 呼叫」，而且每加一種就多一段。
//! 這個模組把那條路徑抽成「一條說得出自己身分、能收發 `aip`、能 stop-all 的
//! 通道」（[`DeviceAipChannel`]），核心因此**沒有任何 Serial 特判**。
//!
//! 不變量：
//! - 身分：party 一律是 **spec 宣告的** `expectedDeviceId`，不是裝置自報的
//!   `envelope.source`。宣稱不是身分——比對才是（`bind_identity` 那一關）。
//! - 准入：只有通過 hello 身分驗證＋配對握手的連線才收 `aip`
//!   （[`DeviceLink::admit_aip`]），比照 iPhone 的 auth-ok 閘門。被擋下來的
//!   一律留稽核，不靜默丟棄。
//! - 身分強度：Serial／MQTT／BLE 的身分是「傳輸層 hello ＋**裝置端**比對配對碼」，
//!   **弱於**已配對 iPhone 的 host 端 sha256(token) 驗證。所有稽核都必須說得
//!   出這個差別（[`IDENTITY_STRENGTH_DEVICE_LINK`]），不得沿用「已驗證身分」。
//! - 誠實階梯：`send_aip` 回 Ok 只是「已寫上線」；stop-all 沒有 ack 就是
//!   `unknown`，不冒充已停。
//! - 有界：握手重試有退避上限、每次等待都有 deadline、沒有無界佇列；provider
//!   被撤銷／停用時 task 立刻收掉。

use crate::providers::ProviderCapabilityDeclaration;
use crate::runtime::{Runtime, RuntimeInner};
use crate::sensor_source::{upgrade, SensorSource, SensorStopReport, SensorStopStatus};
use crate::sensors::{SensorUse, SENSOR_STATE_ACTIVE};
use interaction_adapter_declarative::protocol::{
    AipAdmission, DeviceAipChannel, LinkError, LinkReadiness,
};
use interaction_adapter_declarative::DeclarativeSpec;
use interaction_aip::Party;
use interaction_core::ReceptorId;
use interaction_session::Presence;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// 已配對 iPhone 的身分強度：配對時交換的 per-device token，host 端以
/// sha256(token) 逐次驗證。
pub const IDENTITY_STRENGTH_PAIRED_TOKEN: &str = "paired-token";

/// 宣告式裝置線（serial／mqtt／ble）的身分強度：傳輸層 hello 的 `deviceId`
/// 明文字串比對 ＋ 配對碼由**裝置端**比對（host 只送碼、等 pair-ok）。
///
/// 誠實：這比 [`IDENTITY_STRENGTH_PAIRED_TOKEN`] 弱。任何能開這個埠／topic
/// 的程序都可以自稱是這台裝置；配對碼是否真的被比對過，host 只能從裝置的
/// `hello.pairing` 推斷（見 `DeviceLink::pairing_unverified`）。文件與 UI 不得
/// 把它寫成「已驗證身分」。
pub const IDENTITY_STRENGTH_DEVICE_LINK: &str = "transport-hello+device-side-pairing";

/// 一則 AIP frame 寫上線的預算（過期就不再送出：遲到的訊息比失敗更糟）。
const AIP_SEND_TIMEOUT: Duration = Duration::from_millis(1_500);
/// 收訊迴圈的輪詢窗：每一輪回頭檢查連線／握手是否還成立。
const RX_POLL: Duration = Duration::from_millis(400);
/// 握手重試退避（有界：不會退到無限久，也不會忙碌重試）。
const HANDSHAKE_BACKOFF_MIN: Duration = Duration::from_millis(500);
const HANDSHAKE_BACKOFF_MAX: Duration = Duration::from_secs(15);
/// 一條通道最少分到多少停止預算（多條通道平分時的下限）。
const STOP_MIN_BUDGET: Duration = Duration::from_millis(300);
/// 「確認來源」：裝置對 stop-all 回的 ack。只有它算確認，沒有 ack 就是未知。
const STOP_CONFIRMED_VIA_ACK: &str = "ack";

/// 這一族宣告式裝置的人話種類名（介面只知道「是哪一種來源」時用它）。
const DECLARATIVE_CLASS_LABEL: &str = "外部裝置";

/// 這份 spec 對核心的能力語意宣告。
///
/// - `declaration_id` ＝ provider id：`SensorSource::source_id` 用同一個值，
///   通用的 provider 撤銷／停用（`sensor_source_for_provider`）才找得到它。
/// - 高風險受器 ＝ spec 自己標 `requiresConsent` 的受器。核心不猜哪一個受器
///   敏感，spec 說了算——與 iPhone 那一族「能力表自己標 high_risk」同構。
pub fn declaration_for_spec(
    spec: &DeclarativeSpec,
    provider_id: &str,
    receptor_ids: &[String],
) -> (ProviderCapabilityDeclaration, Vec<String>) {
    let high_risk: Vec<String> = interaction_adapter_declarative::receptor_consent_map(spec)
        .into_iter()
        .filter(|(id, requires_consent)| *requires_consent && receptor_ids.iter().any(|r| r == id))
        .map(|(id, _)| id)
        .collect();
    let label = spec
        .display_name
        .clone()
        .unwrap_or_else(|| DECLARATIVE_CLASS_LABEL.to_string());
    let declaration = ProviderCapabilityDeclaration::new(provider_id)
        .with_class_label(label)
        .with_receptors(receptor_ids.iter().cloned())
        .with_high_risk_receptors(high_risk.clone());
    (declaration, high_risk)
}

/// 出站邊界：我們自己送出去的 envelope 也必須符合 AIP（§1 id／name 語法、
/// §11 上限）。不合格就誠實丟棄並稽核，不把違反契約的東西丟上線。
///
/// 與 `mobile.rs` 的出站關卡同一條規則；兩邊各自解析同一份 `interaction_aip`
/// 契約，沒有第二套判準。
fn outbound_envelope(reply: &Value) -> Option<Value> {
    let envelope: interaction_aip::Envelope = serde_json::from_value(reply.clone()).ok()?;
    envelope.validate().ok()?;
    Some(reply.clone())
}

/// 一台宣告式裝置的 session 綁定：一條 AIP 通道 ↔ 一個 `Party::device(...)`。
struct DeviceBinding {
    runtime: Weak<RuntimeInner>,
    provider_id: String,
    channel: Arc<dyn DeviceAipChannel>,
    /// 目前是否已經對這台裝置送過 `Reconnecting`（一次斷線只送一次）。
    announced_reconnecting: AtomicBool,
    /// 已經為「這一次握手」留過身分強度稽核（每次重新握手才再留一次）。
    announced_identity: AtomicBool,
}

impl DeviceBinding {
    fn party(&self) -> Party {
        Party::device(self.channel.expected_device_id())
    }

    /// 握手成立：留一次身分強度稽核，並讓已經是成員的裝置回到 online。
    async fn note_ready(&self, rt: &Runtime) {
        if !self.announced_identity.swap(true, Ordering::SeqCst) {
            let _ = rt.store.audit(
                "aip.device-channel-ready",
                "runtime",
                &json!({
                    "providerId": self.provider_id,
                    "deviceId": self.channel.expected_device_id(),
                    "transport": self.channel.transport_label(),
                    // 誠實：這條線的身分比已配對 iPhone 弱，稽核必須說得出來。
                    "identityStrength": IDENTITY_STRENGTH_DEVICE_LINK,
                    "pairingUnverified": self.channel.pairing_unverified(),
                }),
            );
        }
        self.announced_reconnecting.store(false, Ordering::SeqCst);
        let party = self.party();
        if rt.character_session_is_member(&party) {
            rt.character_session_presence(&party, Presence::Online)
                .await;
        }
    }

    /// 連線／握手不再成立：成員保留，presence 誠實降級成 `reconnecting`。
    /// 之後由**既有** tick 把它轉成 offline、再逾時 leave——不另造第二條逾時路徑。
    async fn note_disconnected(&self, rt: &Runtime, why: &str) {
        if self.announced_reconnecting.swap(true, Ordering::SeqCst) {
            return;
        }
        self.announced_identity.store(false, Ordering::SeqCst);
        let party = self.party();
        if !rt.character_session_is_member(&party) {
            return;
        }
        rt.character_session_presence(&party, Presence::Reconnecting)
            .await;
        let _ = rt.store.audit(
            "aip.device-channel-lost",
            "runtime",
            &json!({
                "providerId": self.provider_id,
                "deviceId": self.channel.expected_device_id(),
                "transport": self.channel.transport_label(),
                "reason": why,
                "presence": "reconnecting",
            }),
        );
    }

    /// 一則裝置送來的 `aip` 行。
    async fn on_aip(&self, rt: &Runtime, admission: AipAdmission) {
        let device_id = self.channel.expected_device_id();
        let envelope = match admission {
            AipAdmission::Admitted(envelope) => envelope,
            AipAdmission::RefusedNotPaired => {
                // 靜默丟棄會把「裝置說了話但我們不接受」講成「裝置沒說話」。
                let _ = rt.store.audit(
                    "aip.rejected",
                    "runtime",
                    &json!({
                        "transport": self.channel.transport_label(),
                        "stage": "transport-admission",
                        "reason": "the link has no valid hello+pairing handshake",
                        "deviceId": device_id,
                    }),
                );
                return;
            }
            AipAdmission::RefusedTooLarge { bytes } => {
                let _ = rt.store.audit(
                    "aip.rejected",
                    "runtime",
                    &json!({
                        "transport": self.channel.transport_label(),
                        "stage": "transport-admission",
                        "reason": "envelope over the wire limit",
                        "bytes": bytes,
                        "deviceId": device_id,
                    }),
                );
                return;
            }
        };
        // 身分綁定＝spec 的 expectedDeviceId。`character_session_device_frame`
        // 會把 envelope 的 `source` 拿去跟它比對（宣稱不是身分）。
        let outcome = rt
            .character_session_device_frame(&device_id, &json!({"envelope": envelope}))
            .await;
        for reply in outcome.replies {
            match outbound_envelope(&reply) {
                Some(value) => {
                    let bytes = serde_json::to_vec(&value).map(|b| b.len()).unwrap_or(0);
                    if let Err(error) = self.channel.send_aip(value, AIP_SEND_TIMEOUT).await {
                        // 送不到不等於送到了：不重送、不假裝成功——但**也不靜默**。
                        // 只落一行 debug log 的話，「這台裝置已加入 session」與
                        // 「它其實一則狀態都沒收到」在畫面上長得一模一樣。
                        // （最常見的原因：envelope 超過這條線的單行上限，例如
                        //  參考韌體的 639 bytes——見 `serial::MAX_LINE_BYTES`。）
                        tracing::debug!(
                            device = %device_id,
                            %error,
                            "an aip reply did not reach the declarative device"
                        );
                        let _ = rt.store.audit(
                            "aip.outbound-undeliverable",
                            "runtime",
                            &json!({
                                "transport": self.channel.transport_label(),
                                "deviceId": device_id,
                                "bytes": bytes,
                                "reason": error.to_string(),
                            }),
                        );
                    }
                }
                None => {
                    let _ = rt.store.audit(
                        "aip.outbound-refused",
                        "runtime",
                        &json!({
                            "transport": self.channel.transport_label(),
                            "deviceId": device_id,
                            "reason": "reply did not satisfy the aip profile",
                        }),
                    );
                }
            }
        }
    }

    /// 這條通道的收送迴圈。結束條件：link 被關閉（provider 停用／撤銷）或
    /// Runtime 已經消失——兩者都不會讓它變成永遠跑著的孤兒 task。
    async fn run(self: Arc<Self>) {
        let mut backoff = HANDSHAKE_BACKOFF_MIN;
        loop {
            let Some(rt) = upgrade(&self.runtime) else {
                return; // Runtime 已經被釋放：這條 task 沒有主人了。
            };
            if matches!(self.channel.readiness(), LinkReadiness::Closed) {
                return; // provider 被停用／撤銷：連線已關，收工。
            }
            if let Err(error) = self.channel.ensure_ready().await {
                self.note_disconnected(&rt, &error.to_string()).await;
                drop(rt);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(HANDSHAKE_BACKOFF_MAX);
                continue;
            }
            backoff = HANDSHAKE_BACKOFF_MIN;
            self.note_ready(&rt).await;
            let mut rx = self.channel.subscribe();
            drop(rt);
            loop {
                // 有界輪詢：裝置沉默時也要回頭檢查連線／握手是否還成立，
                // 否則拔線會被演成「一切正常，只是很安靜」。
                match tokio::time::timeout(RX_POLL, rx.recv()).await {
                    Ok(Ok(msg)) => {
                        let Some(admission) = self.channel.admit_aip(&msg) else {
                            continue; // 不是 aip：交給既有的 cmd/read 路徑。
                        };
                        let Some(rt) = upgrade(&self.runtime) else {
                            return;
                        };
                        self.on_aip(&rt, admission).await;
                    }
                    // 追不上＝可能漏了裝置的 frame：留痕，不假裝什麼都沒發生。
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                        if let Some(rt) = upgrade(&self.runtime) {
                            let _ = rt.store.audit(
                                "aip.inbound-lagged",
                                "runtime",
                                &json!({
                                    "deviceId": self.channel.expected_device_id(),
                                    "dropped": n,
                                }),
                            );
                        }
                    }
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                    Err(_) => {}
                }
                match self.channel.readiness() {
                    LinkReadiness::Closed => return,
                    LinkReadiness::Ready | LinkReadiness::Stale { .. } => {}
                    other => {
                        if let Some(rt) = upgrade(&self.runtime) {
                            self.note_disconnected(&rt, &format!("{other:?}")).await;
                        }
                        break;
                    }
                }
                if !self.channel.handshake_ready() {
                    if let Some(rt) = upgrade(&self.runtime) {
                        self.note_disconnected(&rt, "handshake invalidated").await;
                    }
                    break;
                }
            }
        }
    }
}

/// 一組 task 的擁有權：drop（或 `abort_all`）就全部收掉。
#[derive(Default)]
struct TaskGroup(Mutex<Vec<tokio::task::JoinHandle<()>>>);

impl TaskGroup {
    fn push(&self, handle: tokio::task::JoinHandle<()>) {
        if let Ok(mut guard) = self.0.lock() {
            guard.push(handle);
        }
    }

    fn abort_all(&self) {
        let handles = match self.0.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        for handle in handles {
            handle.abort();
        }
    }
}

impl Drop for TaskGroup {
    fn drop(&mut self) {
        self.abort_all();
    }
}

/// 一個宣告式 adapter（＝一台裝置）作為一般的 [`SensorSource`]。
///
/// 停止管道就是這條線本來就有的 `stop-all`＋ack 等待——沒有第二套停止語意，
/// 也沒有 `no-stop-path`：這一族的高風險受器一律由這裡回報。
pub struct DeclarativeSensorSource {
    runtime: Weak<RuntimeInner>,
    provider_id: String,
    label: String,
    channels: Vec<Arc<dyn DeviceAipChannel>>,
    high_risk: Vec<String>,
    tasks: TaskGroup,
    retired: AtomicBool,
    /// 每個受器**第一次**被看到「開著」的時間。`activeSensors` 的 `startedAt`
    /// 讀它——每次查詢都回「現在」等於宣稱它剛剛才開始擷取，那是假的。
    /// 有界：鍵只可能是這一族宣告的高風險受器。
    started_at: Mutex<std::collections::BTreeMap<String, chrono::DateTime<chrono::Utc>>>,
}

impl DeclarativeSensorSource {
    /// 目前「還開著」的高風險受器（registry 對停用中的受器回 Err）。
    async fn enabled_high_risk(&self, rt: &Runtime) -> Vec<String> {
        let mut open = Vec::new();
        for id in &self.high_risk {
            if rt.registry.receptor(&ReceptorId::new(id)).await.is_ok() {
                open.push(id.clone());
            }
        }
        open
    }

    fn report(
        &self,
        sensors: Vec<String>,
        outcome: SensorStopStatus,
        waited_ms: u64,
    ) -> SensorStopReport {
        SensorStopReport::new(
            self.provider_id.clone(),
            self.provider_id.clone(),
            sensors,
            outcome,
            waited_ms,
        )
        .with_label(self.label.clone())
    }
}

#[async_trait::async_trait]
impl SensorSource for DeclarativeSensorSource {
    fn source_id(&self) -> String {
        self.provider_id.clone()
    }

    fn declaration_id(&self) -> String {
        self.provider_id.clone()
    }

    async fn active_captures(&self) -> Vec<SensorUse> {
        let Some(rt) = upgrade(&self.runtime) else {
            return vec![];
        };
        let open = self.enabled_high_risk(&rt).await;
        let now = chrono::Utc::now();
        let mut seen = match self.started_at.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // 已經關掉的受器不再佔位（這張表跟著「目前開著的」走，不無界成長）。
        seen.retain(|id, _| open.iter().any(|o| o == id));
        open.into_iter()
            .map(|id| {
                let started_at = *seen.entry(id.clone()).or_insert(now);
                SensorUse {
                    kind: id,
                    started_at,
                    started_by: "device".into(),
                    purpose: format!("{}（宣告式 adapter）", self.label),
                    auto_stop_at: None,
                    state: SENSOR_STATE_ACTIVE.to_string(),
                }
            })
            .collect()
    }

    async fn request_stop(
        &self,
        _target: Option<&str>,
        deadline: Duration,
        _reason: &str,
    ) -> Vec<SensorStopReport> {
        let started = std::time::Instant::now();
        let Some(rt) = upgrade(&self.runtime) else {
            return vec![];
        };
        let open = self.enabled_high_risk(&rt).await;
        if open.is_empty() {
            // 本來就沒有在擷取。`sensors` 仍然列出這一族宣告的高風險受器，
            // 停止掃描才知道它們**有**被問過（否則會落成 no-stop-path）。
            return vec![self.report(self.high_risk.clone(), SensorStopStatus::AlreadyStopped, 0)];
        }
        // 先關掉本機這一側的輪詢：不先關，下一次 poll 立刻又去讀感測器，
        // 「已停止」在一個輪詢週期內就變成謊話。
        for id in &open {
            let _ = rt
                .registry
                .set_receptor_enabled(&ReceptorId::new(id), false)
                .await;
        }
        if self.channels.is_empty() {
            // 沒有裝置線（純 HTTP adapter）：本機側已經停了，但裝置那端有沒有
            // 停我們**不知道**——不冒充已停。
            return vec![self
                .report(
                    open,
                    SensorStopStatus::Unknown,
                    started.elapsed().as_millis() as u64,
                )
                .with_detail("this adapter has no device link to send stop-all on")];
        }
        let budget = (deadline / self.channels.len() as u32).max(STOP_MIN_BUDGET);
        let mut reports = Vec::new();
        for channel in &self.channels {
            let outcome = match channel.stop_all(budget).await {
                Ok(()) => (SensorStopStatus::Stopped, None),
                // 送不出去＝什麼都沒被停（確定，不是未知）。
                Err(LinkError::Unavailable(detail)) => {
                    (SensorStopStatus::Unreachable, Some(detail))
                }
                // 裝置明確拒絕：比逾時更確定它還在。
                Err(LinkError::Refused(detail)) => (SensorStopStatus::Refused, Some(detail)),
                // 已送出、沒確認：結果未知（逾時／連線世代改變／寫出途中失敗）。
                Err(other) => (SensorStopStatus::Unknown, Some(other.to_string())),
            };
            let waited = started.elapsed().as_millis() as u64;
            let mut report = self.report(open.clone(), outcome.0, waited);
            if outcome.0 == SensorStopStatus::Stopped {
                report = report.with_via(Some(STOP_CONFIRMED_VIA_ACK));
            }
            if let Some(detail) = outcome.1 {
                report = report.with_detail(detail);
            }
            reports.push(report);
        }
        reports
    }

    /// provider 被撤銷／停用：這台裝置也要離開 session、撤回能力宣告、
    /// 解除來源登記，並收掉綁定 task。
    async fn release(&self, _target: Option<&str>, reason: &str) -> Option<Value> {
        let rt = upgrade(&self.runtime)?;
        Some(self.retire(&rt, reason).await)
    }
}

impl DeclarativeSensorSource {
    /// 下架這一族：leave → retract → unregister → 收 task。冪等。
    async fn retire(&self, rt: &Runtime, reason: &str) -> Value {
        if self.retired.swap(true, Ordering::SeqCst) {
            return json!({"providerId": self.provider_id, "retired": "already"});
        }
        self.tasks.abort_all();
        let mut left = Vec::new();
        for channel in &self.channels {
            let device_id = channel.expected_device_id();
            rt.character_session_leave(&Party::device(&device_id)).await;
            left.push(device_id);
        }
        let retracted = rt.retract_provider_capabilities(&self.provider_id);
        let _ = rt.store.audit(
            "aip.device-retired",
            "runtime",
            &json!({
                "providerId": self.provider_id,
                "reason": reason,
                "leftSession": left,
                "declarationRetracted": retracted,
                "identityStrength": IDENTITY_STRENGTH_DEVICE_LINK,
            }),
        );
        // 最後才解除登記：稽核與 leave 都已經做完。`unregister_sensor_source`
        // 只會回頭呼叫 `active_captures()`（純讀），不會再進到這裡——所以直接
        // await，不丟給背景 task（背景 task ＝「撤銷了沒有」在呼叫端變成競態）。
        rt.unregister_sensor_source(&self.provider_id).await;
        json!({
            "providerId": self.provider_id,
            "leftSession": left,
            "declarationRetracted": retracted,
        })
    }
}

impl Runtime {
    /// 把一份宣告式 spec 的裝置線接上 Character Session ＋停止感測路徑。
    ///
    /// `kept_off` ＝ 人類把這個 provider 關掉了：**不**開綁定 task（停用中的
    /// 裝置不得在背景重連握手），但仍然登記 SensorSource ——否則「它現在有沒有
    /// 在擷取」就沒有人回答得出來。
    pub(crate) async fn bind_declarative_device(
        &self,
        provider_id: &str,
        label: &str,
        channels: Vec<Arc<dyn DeviceAipChannel>>,
        high_risk: Vec<String>,
        kept_off: bool,
    ) {
        let tasks = TaskGroup::default();
        if !kept_off {
            for channel in &channels {
                let binding = Arc::new(DeviceBinding {
                    runtime: self.weak_inner(),
                    provider_id: provider_id.to_string(),
                    channel: channel.clone(),
                    announced_reconnecting: AtomicBool::new(false),
                    announced_identity: AtomicBool::new(false),
                });
                tasks.push(tokio::spawn(binding.run()));
            }
        }
        let source = Arc::new(DeclarativeSensorSource {
            runtime: self.weak_inner(),
            provider_id: provider_id.to_string(),
            label: label.to_string(),
            channels,
            high_risk,
            tasks,
            retired: AtomicBool::new(false),
            started_at: Mutex::new(std::collections::BTreeMap::new()),
        });
        if let Err(error) = self.register_sensor_source(source).await {
            // 登記表滿了：綁定不成立，誠實留痕（此時停止路徑對它一無所知）。
            tracing::warn!(
                provider = %provider_id,
                %error,
                "the declarative device could not be registered as a sensor source"
            );
        }
    }
}
