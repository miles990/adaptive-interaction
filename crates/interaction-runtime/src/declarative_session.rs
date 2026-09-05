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
//! - 出站對稱：握手成立時把這條線登記成一條型別抹除的
//!   [`crate::character_session::DeviceOutbound`]，撤銷時移除。沒有這一步的話，
//!   這台裝置只收得到「對自己那則 frame 的直接回覆」——別的成員造成的 shared
//!   state 變更（`Output::Broadcast`）永遠到不了它，而桌面顯示「已同步」。
//! - 有界：握手重試有退避上限、每次等待都有 deadline、沒有無界佇列；provider
//!   被撤銷／停用時 task 立刻收掉。

use crate::character_session::{DeviceOrigin, DeviceOutbound};
use crate::providers::ProviderCapabilityDeclaration;
use crate::runtime::{Runtime, RuntimeInner};
use crate::sensor_source::{upgrade, SensorSource, SensorStopReport, SensorStopStatus};
use crate::sensors::{
    SensorUse, SENSOR_STATE_ACTIVE, SENSOR_STATE_STOPPING, SENSOR_STATE_STOP_UNKNOWN,
};
use interaction_adapter_declarative::protocol::{
    AipAdmission, DeviceAipChannel, LinkError, LinkReadiness,
};
use interaction_adapter_declarative::DeclarativeSpec;
use interaction_aip::{Envelope, Party};
use interaction_core::ReceptorId;
use interaction_session::Presence;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// 身分強度的標籤只有一份（現在由 `character_session` 擁有：出站通道登記表與
/// diagnostics 都要用同一組值）。舊路徑仍然解析得到。
pub use crate::character_session::{IDENTITY_STRENGTH_DEVICE_LINK, IDENTITY_STRENGTH_PAIRED_TOKEN};

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

/// 這條裝置線作為一條**型別抹除**的 AIP 出站通道。
///
/// 為什麼要登記：對這台裝置的直接回覆走 [`DeviceBinding::on_aip`]，但別的成員
/// 造成的 shared state 變更是 `Output::Broadcast`——那條路徑只認得
/// [`DeviceOutbound`]。沒有登記的話，這台裝置從加入的第一秒起就只收得到自己
/// 講話的回音。
struct DeclarativeOutbound {
    channel: Arc<dyn DeviceAipChannel>,
}

#[async_trait::async_trait]
impl DeviceOutbound for DeclarativeOutbound {
    async fn send_aip(&self, envelope: &Envelope) -> Result<(), interaction_core::DomainError> {
        // 出站邊界與 `on_aip` 同一條規則：不合 AIP 的東西不上線。
        let value = serde_json::to_value(envelope)
            .map_err(|_| interaction_core::DomainError::Validation("envelope".into()))?;
        let Some(value) = outbound_envelope(&value) else {
            return Err(interaction_core::DomainError::Validation(
                "reply did not satisfy the aip profile".into(),
            ));
        };
        self.channel
            .send_aip(value, AIP_SEND_TIMEOUT)
            .await
            .map_err(|e| interaction_core::DomainError::Unavailable(e.to_string()))
    }

    fn transport_label(&self) -> &str {
        self.channel.transport_label()
    }

    fn identity_strength(&self) -> &str {
        IDENTITY_STRENGTH_DEVICE_LINK
    }

    fn max_line_bytes(&self) -> Option<usize> {
        self.channel.max_line_bytes()
    }

    fn supports_fragmentation(&self) -> bool {
        self.channel.supports_fragmentation()
    }
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
        // 出站登記：從這一刻起，別的成員造成的 shared state 變更才送得到這台
        // 裝置。冪等（同一台裝置重複握手只是覆蓋同一個鍵）。
        rt.register_device_outbound(
            &self.channel.expected_device_id(),
            Arc::new(DeclarativeOutbound {
                channel: self.channel.clone(),
            }),
        );
        let party = self.party();
        if rt.character_session_is_member(&party) {
            rt.character_session_presence(&party, Presence::Online)
                .await;
        }
        // 這條線真的握上手了：如果 provider 還停在「重新連線中」
        // （`Disconnected`），現在才是把它收斂成 `Available` 的時刻。
        // 只認 `Disconnected` 這一個入口——握手不得冒充人類的啟用決定。
        rt.converge_provider_after_rebind(&interaction_core::ProviderId::new(&self.provider_id))
            .await;
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

    /// 一筆被丟棄的分片傳輸：留稽核（原因是固定字串，不回顯裝置輸入）。
    fn audit_fragment_drop(
        &self,
        rt: &Runtime,
        drop: interaction_adapter_declarative::fragment::FragmentDrop,
    ) {
        let _ = rt.store.audit(
            "aip.fragment-dropped",
            "runtime",
            &json!({
                "transport": self.channel.transport_label(),
                "deviceId": self.channel.expected_device_id(),
                "providerId": self.provider_id,
                "xfer": drop.xfer,
                "reason": drop.reason,
                "received": drop.received,
                "total": drop.total,
            }),
        );
    }

    /// 一則裝置送來的 `aip` 行。
    async fn on_aip(&self, rt: &Runtime, admission: AipAdmission) {
        let device_id = self.channel.expected_device_id();
        // 綁定不成立時一律不收（「停新請求」）。
        //
        // 為什麼不能只靠 `tasks.abort_all()`：abort 只在下一個 await 點生效，
        // 一則**已經在處理中**的 frame 會照樣跑完——實測過的後果是裝置在
        // `character.session.leave` 之後又 join 回來，於是「停用了」與「還在
        // session 裡」同時成立。這是一道確定性的閘門，不是時序運氣。
        //
        // 沒有生命週期記錄（例如綁定表滿了）＝這道閘門對它一無所知，維持原本
        // 行為，不無中生有地拒絕。
        if let Some(lifecycle) = rt.declarative_lifecycle(&self.provider_id) {
            if lifecycle != crate::declarative_lifecycle::DeclarativeLifecycle::Bound {
                let _ = rt.store.audit(
                    "aip.rejected",
                    "runtime",
                    &json!({
                        "transport": self.channel.transport_label(),
                        "stage": "provider-binding",
                        "reason": "this device's binding is not established",
                        "lifecycle": lifecycle.label(),
                        "providerId": self.provider_id,
                        "deviceId": device_id,
                    }),
                );
                return;
            }
        }
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
            // 線協定 v1.2：這一片已經進了傳輸層的重組緩衝。核心不認得「被
            // 分片」——它只會在整份組好之後看到一則 `Admitted`。
            AipAdmission::FragmentBuffered => return,
            AipAdmission::FragmentDropped(drop) => {
                // 整筆丟棄不得靜默：「裝置說了話但我們組不回來」與「裝置沒說話」
                // 在畫面上長得一模一樣。
                self.audit_fragment_drop(rt, drop);
                return;
            }
        };
        // 身分綁定＝spec 的 expectedDeviceId。`character_session_device_frame`
        // 會把 envelope 的 `source` 拿去跟它比對（宣稱不是身分）。
        let outcome = rt
            .character_session_device_frame(
                &device_id,
                // 這一則是從**這條線**進來的：稽核的 transport 由通道自己說，
                // 核心不猜、也不沿用別種傳輸的標籤。
                DeviceOrigin {
                    transport: self.channel.transport_label(),
                    identity_strength: IDENTITY_STRENGTH_DEVICE_LINK,
                },
                &json!({"envelope": envelope}),
            )
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
                // 逾時／重連守衛：一筆停在半路的分片傳輸不得無限期佔著緩衝，
                // 也不得靜默消失。
                if let Some(drop) = self.channel.expire_fragments() {
                    if let Some(rt) = upgrade(&self.runtime) {
                        self.audit_fragment_drop(&rt, drop);
                    }
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
    /// 「已經要求停止、但還沒拿到明確確認」的受器（比照 mobile 的 stop_pending）。
    ///
    /// 為什麼需要它：`request_stop` 會**先**把 registry 旗標關掉（不先關，下一個
    /// 輪詢週期又去讀感測器），而 `active_captures` 只看旗標——於是拔線／裝置拒絕
    /// 的受器會在停止請求送出的瞬間從 `activeSensors` 靜默消失，等於宣稱它停了
    /// （違反「感測不靜默」與誠實階梯），也讓 `unregister_sensor_source` 的孤兒
    /// 安全網永遠打不開。只有明確的 `Stopped`／`AlreadyStopped` 才移除表項。
    ///
    /// 有界：鍵只可能是這一族宣告的高風險受器（`high_risk`）。
    stop_pending: Mutex<std::collections::BTreeMap<String, StopPendingState>>,
}

/// 一筆「已要求停止」的目前狀態。時間不進表：`request_stop` 是**同步等完**
/// 自己的 deadline 才回來的，所以結果一回來就已經確定，不需要再用時鐘猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopPendingState {
    /// 請求還在途中（正在等裝置的 ack）。
    InFlight,
    /// 問過了、沒拿到確認（逾時／送不出去／裝置拒絕）：可能仍在擷取。
    Unresolved,
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

    /// 待確認表的存取（poisoned 時取回內容繼續：這張表只是可見性紀錄，
    /// 為了它 panic 反而會讓感測從畫面上消失）。
    fn pending_guard(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<String, StopPendingState>> {
        match self.stop_pending.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn pending_snapshot(&self) -> Vec<(String, StopPendingState)> {
        self.pending_guard()
            .iter()
            .map(|(id, state)| (id.clone(), *state))
            .collect()
    }

    /// 記下「已要求停止」。`high_risk` 之外的 id 一律不收（有界）。
    fn note_stop_requested(&self, ids: &[String], state: StopPendingState) {
        let mut guard = self.pending_guard();
        for id in ids {
            if self.high_risk.iter().any(|h| h == id) {
                guard.insert(id.clone(), state);
            }
        }
    }

    /// 明確確認才移除：`Stopped`／`AlreadyStopped` 以外一律留著。
    fn clear_stop_pending(&self, ids: &[String]) {
        let mut guard = self.pending_guard();
        for id in ids {
            guard.remove(id);
        }
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
        // 受器又被打開了＝有人重新啟用它：那就是新的擷取，舊的「停止結果未知」
        // 不再適用（留著會把正在擷取的東西說成停止中）。
        self.clear_stop_pending(&open);
        // 已要求停止、還沒拿到確認的：旗標已經關了，但**裝置那端**停了沒有我們
        // 不知道——不得從清單上消失（消失＝宣稱它停了）。
        let pending: Vec<(String, StopPendingState)> = self
            .pending_snapshot()
            .into_iter()
            .filter(|(id, _)| !open.iter().any(|o| o == id))
            .collect();
        let now = chrono::Utc::now();
        let mut seen = match self.started_at.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // 已經關掉、也沒有待確認停止的受器不再佔位（這張表跟著「目前看得到的」
        // 走，不無界成長）。
        seen.retain(|id, _| open.iter().any(|o| o == id) || pending.iter().any(|(p, _)| p == id));
        let mut captures: Vec<SensorUse> = open
            .into_iter()
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
            .collect();
        for (id, state) in pending {
            let started_at = *seen.entry(id.clone()).or_insert(now);
            let (state, purpose) = match state {
                StopPendingState::InFlight => (
                    SENSOR_STATE_STOPPING,
                    format!("{}（宣告式 adapter：停止中，等待裝置確認）", self.label),
                ),
                StopPendingState::Unresolved => (
                    SENSOR_STATE_STOP_UNKNOWN,
                    format!(
                        "{}（宣告式 adapter：停止結果未知，裝置未確認，可能仍在擷取）",
                        self.label
                    ),
                ),
            };
            captures.push(SensorUse {
                kind: id,
                started_at,
                started_by: "device".into(),
                purpose,
                auto_stop_at: None,
                state: state.to_string(),
            });
        }
        captures
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
        // 上一次要求停止、到現在還沒拿到確認的受器也要再問一次：旗標早就關了，
        // 但「裝置停了沒有」仍然未知——只看旗標會把它們當成本來就沒在擷取，
        // 於是第二次停止請求直接回 already-stopped（＝憑空宣稱停了）。
        let mut targets = open.clone();
        for (id, _) in self.pending_snapshot() {
            if !targets.iter().any(|t| t == &id) {
                targets.push(id);
            }
        }
        if targets.is_empty() {
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
        // 旗標關掉的同一刻就記成「停止中」：`active_captures` 從這裡接手，
        // 這些受器在拿到確認之前不得從 `activeSensors` 消失。
        self.note_stop_requested(&targets, StopPendingState::InFlight);
        if self.channels.is_empty() {
            // 沒有裝置線（純 HTTP adapter）：本機側已經停了，但裝置那端有沒有
            // 停我們**不知道**——不冒充已停。
            self.note_stop_requested(&targets, StopPendingState::Unresolved);
            return vec![self
                .report(
                    targets,
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
            let mut report = self.report(targets.clone(), outcome.0, waited);
            if outcome.0 == SensorStopStatus::Stopped {
                report = report.with_via(Some(STOP_CONFIRMED_VIA_ACK));
            }
            if let Some(detail) = outcome.1 {
                report = report.with_detail(detail);
            }
            reports.push(report);
        }
        // 只有**每一條**線都明確確認才算停了；任何一條沒確認，這些受器就繼續
        // 以 stop-unknown 留在 `activeSensors`（誠實階梯：未知 ≠ 已停）。
        if reports.iter().all(|r| r.confirmed()) {
            self.clear_stop_pending(&targets);
        } else {
            self.note_stop_requested(&targets, StopPendingState::Unresolved);
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
            // 出站表先清掉：留著等於之後每一則廣播都往一條已經關掉的線上送。
            rt.unregister_device_outbound(&device_id);
            rt.character_session_leave(&Party::device(&device_id)).await;
            left.push(device_id);
        }
        let retracted = rt.retract_provider_capabilities(&self.provider_id);
        // 綁定確實被拆掉了。這條路徑只知道一個中性的 reason，說不出是「人類
        // 停用」還是「撤銷」——那兩者由 providers.rs 在呼叫這裡**之前**寫成
        // 精確的原因，所以這裡只在還沒有人講過話時補一個誠實的預設。
        rt.note_declarative_unbound_if_bound(
            &self.provider_id,
            crate::declarative_lifecycle::UnboundReason::Disconnected,
        );
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
            stop_pending: Mutex::new(std::collections::BTreeMap::new()),
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
