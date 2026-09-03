//! 裝置線協定（serial / mqtt / ble 共用）＋誠實核心 `DeviceLink`。
//!
//! 一行（或一則）一個 JSON 物件：
//!   device → hello / pair-ok / pair-fail / ack / err / state
//!   host   → who / pair / cmd / cancel / read / stop-all
//!
//! 誠實不變量：
//! - 身分：hello.deviceId 必須等於 spec 的 expectedDeviceId——連線埠、IP、
//!   topic 都不是身分；不符即拒絕（不重試冒認的裝置）。
//! - 配對：spec 有 pairingCode 時，未通過 pair 握手不得送任何 cmd/read。
//! - 冪等：cmd 帶 action id＋nonce；裝置端 dedupe；host 端 ack 逾時
//!   絕不自動重送 cmd（重送可能重複實體效果）——逾時＝結果未知。
//! - ack ≠ observed：ack 只代表裝置說「已套用」，獨立驗證靠 state 觀察。

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::broadcast;

/// 協定版本（與 ESP32 參考韌體對齊）。
pub const PROTO_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum DeviceMsg {
    Hello {
        #[serde(rename = "deviceId")]
        device_id: String,
        #[serde(default)]
        fw: Option<String>,
        #[serde(default)]
        proto: Option<u32>,
        #[serde(default)]
        caps: Vec<String>,
        #[serde(default)]
        pairing: bool,
    },
    PairOk,
    PairFail,
    Ack {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        applied: Option<Value>,
        #[serde(default)]
        dup: Option<bool>,
        #[serde(default)]
        cancelled: Option<bool>,
        #[serde(default, rename = "stopAll")]
        stop_all: Option<bool>,
    },
    Err {
        #[serde(default)]
        id: Option<String>,
        reason: String,
    },
    State {
        #[serde(default, rename = "deviceId")]
        device_id: Option<String>,
        #[serde(default)]
        facts: Value,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum HostMsg {
    Who,
    Pair {
        code: String,
    },
    Cmd {
        id: String,
        nonce: String,
        name: String,
        params: Value,
    },
    Cancel {
        id: String,
    },
    Read,
    StopAll,
}

pub fn parse_device_msg(line: &str) -> Option<DeviceMsg> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.len() > 16 * 1024 {
        return None; // 過長訊息直接丟棄（有界解析）
    }
    serde_json::from_str::<DeviceMsg>(trimmed).ok()
}

pub fn encode_host_msg(msg: &HostMsg) -> String {
    serde_json::to_string(msg).unwrap_or_else(|_| "{}".into())
}

pub fn new_nonce() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// RawLink：三種傳輸各自實作的最小介面
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum LinkError {
    /// 傳輸不可用（未連線、埠不存在、broker 不通…）。
    Unavailable(String),
    /// 身分或配對失敗（不重試）。
    Refused(String),
    /// 等待回覆逾時——結果未知，呼叫端不得自動重送 cmd。
    Timeout(String),
    /// 裝置在 hello.caps 沒有宣告這個能力：cmd 從未送出（無實體效果風險）。
    NotAdvertised(String),
    /// 送出「途中」失敗（例如 BLE write 已寫出但沒有回應）：是否送達未知。
    /// 呼叫端**不得重試**——重試可能讓實體效果重複觸發。
    Uncertain(String),
    /// 等待期間連線世代改變（重連／握手作廢）：舊連線的請求就此結束，
    /// 結果未知；佇列中的舊命令一律不再送出（遲到的實體效果比失敗更糟）。
    Reset(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Unavailable(s) => write!(f, "unavailable: {s}"),
            LinkError::Refused(s) => write!(f, "refused: {s}"),
            LinkError::Timeout(s) => write!(f, "timeout: {s}"),
            LinkError::NotAdvertised(s) => write!(f, "capability-not-advertised: {s}"),
            LinkError::Uncertain(s) => write!(f, "send-outcome-unknown: {s}"),
            LinkError::Reset(s) => write!(f, "link-reset: {s}"),
        }
    }
}

/// 傳輸連線狀態。單一 bool 不夠誠實：「還沒連上（連線中／首次使用才連）」
/// 與「試過但不通」不是同一件事，健康度必須分得開。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// 尚未連上，但仍在嘗試（或按設計在首次使用時才連）——結果未知。
    Connecting,
    /// 目前確定連線中。
    Connected,
    /// 嘗試過但不通（拔線、broker 不通、掃不到裝置）。
    Disconnected,
    /// 已被 `shutdown()` 主動關閉（provider disable/revoke）：不再重連。
    Closed,
}

/// 傳輸層：送一則訊息＋訂閱裝置訊息。實作負責自己的重連/退避；
/// `ensure_open` 失敗必須誠實回報，不得默默排隊。
#[async_trait::async_trait]
pub trait RawLink: Send + Sync {
    async fn ensure_open(&self) -> Result<(), LinkError>;
    async fn send(&self, line: String) -> Result<(), LinkError>;
    /// 送一則「有時限」的訊息：`deadline` 過了就不得再送出。斷線期間排進
    /// 傳輸佇列的命令若在重連後才寫出，會產生遲到的實體效果——實作若有
    /// 佇列，必須把 deadline 一起帶進去，過期即丟棄。預設＝立即送出。
    async fn send_before(
        &self,
        line: String,
        deadline: std::time::Instant,
    ) -> Result<(), LinkError> {
        if std::time::Instant::now() >= deadline {
            return Err(LinkError::Unavailable(
                "deadline passed before the message could be sent; nothing was written".into(),
            ));
        }
        self.send(line).await
    }
    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg>;
    /// 連線世代：每次（重）連線遞增。DeviceLink 用它偵測「重連過」並
    /// 重新走 hello/pair 握手——重連不得沿用舊握手。
    fn generation(&self) -> u64 {
        0
    }
    /// 目前是否確定連線中。健康度不得硬編 healthy：實作必須真的知道
    /// （serial＝supervisor 的 connected 旗標、mqtt＝ConnAck 後為真、
    /// ble＝GATT session 存在）。
    fn connected(&self) -> bool;
    /// 細緻連線狀態（預設由 `connected()` 推導；實作可覆寫以區分
    /// 「連線中」與「連不上」）。
    fn link_state(&self) -> LinkState {
        if self.connected() {
            LinkState::Connected
        } else {
            LinkState::Disconnected
        }
    }
    /// 主動關閉連線：停止重連、回收執行緒／task。呼叫後 `connected()`
    /// 必須為 false 且 `send()` 必須回 Err（不得默默排隊）。冪等。
    fn shutdown(&self);
    /// 傳輸描述（診斷用；不得含 secret）。
    fn describe(&self) -> String;
}

#[async_trait::async_trait]
impl<T: RawLink + ?Sized> RawLink for Arc<T> {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        (**self).ensure_open().await
    }
    async fn send(&self, line: String) -> Result<(), LinkError> {
        (**self).send(line).await
    }
    async fn send_before(
        &self,
        line: String,
        deadline: std::time::Instant,
    ) -> Result<(), LinkError> {
        (**self).send_before(line, deadline).await
    }
    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        (**self).subscribe()
    }
    fn generation(&self) -> u64 {
        (**self).generation()
    }
    fn connected(&self) -> bool {
        (**self).connected()
    }
    fn link_state(&self) -> LinkState {
        (**self).link_state()
    }
    fn shutdown(&self) {
        (**self).shutdown()
    }
    fn describe(&self) -> String {
        (**self).describe()
    }
}

/// 型別抹除的關閉入口：provider 被 disable／revoke 時，runtime 用它真的
/// 把連線關掉（停用的 provider 不得繼續佔著埠／broker 連線重試）。
pub trait LinkShutdown: Send + Sync {
    fn shutdown(&self);
    fn describe(&self) -> String;
}

/// 單槽 task handle：放新的先 abort 舊的，drop 時 abort。
/// 抽出來是為了「不依賴藍牙堆疊也能測」BLE notification task 的回收邏輯。
#[derive(Default)]
pub struct TaskSlot {
    inner: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl TaskSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// 放入新 handle，並 abort（丟棄）舊的。
    pub fn replace(&self, handle: tokio::task::JoinHandle<()>) {
        let mut slot = lock_ignoring_poison(&self.inner);
        if let Some(old) = slot.replace(handle) {
            old.abort();
        }
    }

    /// abort 目前的 task（冪等）。
    pub fn abort(&self) {
        let mut slot = lock_ignoring_poison(&self.inner);
        if let Some(handle) = slot.take() {
            handle.abort();
        }
    }

    /// 是否還有一個「尚未結束」的 task 在槽裡。
    pub fn is_active(&self) -> bool {
        let slot = lock_ignoring_poison(&self.inner);
        slot.as_ref().map(|h| !h.is_finished()).unwrap_or(false)
    }
}

impl Drop for TaskSlot {
    fn drop(&mut self) {
        self.abort();
    }
}

/// panic 過的 Mutex 仍要能收尾（shutdown 路徑不可因 poison 而 panic）。
fn lock_ignoring_poison<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ---------------------------------------------------------------------------
// DeviceLink：hello/pair 握手＋cmd/ack＋read/state 的誠實核心
// ---------------------------------------------------------------------------

pub struct DeviceLink<L: RawLink> {
    raw: L,
    expected_device_id: String,
    pairing_code: Option<String>,
    /// (已握手, 握手時的連線世代)。世代不符＝重連過→重新握手。
    handshaken: tokio::sync::Mutex<(bool, u64)>,
    /// 上面那把鎖的無鎖鏡像：0＝未握手，否則＝握手當下的世代 +1。
    /// health/status 用它，才不會在握手進行中被鎖住（健康檢查不能阻塞）。
    ready_generation: AtomicU64,
    /// 裝置 hello 宣告的能力清單（握手後填）。None＝還沒握手過。
    advertised_caps: RwLock<Option<Arc<Vec<String>>>>,
}

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(2_500);

/// 握手完成的世代編碼（0 保留給「從未握手」）。
fn ready_marker(generation: u64) -> u64 {
    generation.wrapping_add(1)
}

impl<L: RawLink> DeviceLink<L> {
    pub fn new(raw: L, expected_device_id: String, pairing_code: Option<String>) -> Self {
        Self {
            raw,
            expected_device_id,
            pairing_code,
            handshaken: tokio::sync::Mutex::new((false, 0)),
            ready_generation: AtomicU64::new(0),
            advertised_caps: RwLock::new(None),
        }
    }

    pub fn raw(&self) -> &L {
        &self.raw
    }

    /// 目前的誠實可用狀態（health/status 用；不做任何 I/O、不阻塞）。
    pub fn readiness(&self) -> LinkReadiness {
        match self.raw.link_state() {
            LinkState::Closed => LinkReadiness::Closed,
            LinkState::Disconnected => LinkReadiness::Disconnected,
            LinkState::Connecting => LinkReadiness::Connecting,
            LinkState::Connected => {
                if self.ready_generation.load(Ordering::SeqCst)
                    == ready_marker(self.raw.generation())
                {
                    LinkReadiness::Ready
                } else {
                    LinkReadiness::NotHandshaken
                }
            }
        }
    }

    /// 裝置在 hello.caps 是否宣告了這個能力。
    /// - `None`＝還不知道（尚未握手，或裝置根本沒宣告 caps——舊韌體相容：
    ///   沒有宣告不等於「宣告沒有」，此時不阻擋，只在握手時 debug 記錄）
    /// - `Some(false)`＝裝置明確宣告了能力清單但不含這一項 → 不得送 cmd
    pub fn advertises(&self, name: &str) -> Option<bool> {
        let caps = self.caps_snapshot()?;
        if caps.is_empty() {
            return None;
        }
        Some(caps.iter().any(|c| c == name))
    }

    /// 握手取得的能力清單快照（診斷／收據用）。
    pub fn caps_snapshot(&self) -> Option<Arc<Vec<String>>> {
        match self.advertised_caps.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn store_caps(&self, caps: Vec<String>) {
        let value = Some(Arc::new(caps));
        match self.advertised_caps.write() {
            Ok(mut guard) => *guard = value,
            Err(poisoned) => *poisoned.into_inner() = value,
        }
    }

    /// hello（身分驗證）＋pair（配對碼）握手。冪等：已握手且未重連直接通過。
    pub async fn ensure_ready(&self) -> Result<(), LinkError> {
        self.raw.ensure_open().await?;
        let gen = self.raw.generation();
        let mut done = self.handshaken.lock().await;
        if done.0 && done.1 == gen {
            return Ok(());
        }
        done.0 = false;
        self.ready_generation.store(0, Ordering::SeqCst);
        let mut rx = self.raw.subscribe();
        self.raw.send(encode_host_msg(&HostMsg::Who)).await?;
        let (hello, caps, proto, device_wants_pairing) =
            wait_for(&mut rx, HANDSHAKE_TIMEOUT, |m| match m {
                DeviceMsg::Hello {
                    device_id,
                    caps,
                    proto,
                    pairing,
                    ..
                } => Some((device_id.clone(), caps.clone(), *proto, *pairing)),
                _ => None,
            })
            .await
            .map_err(|e| handshake_wait_error("hello", e))?;
        if hello != self.expected_device_id {
            // IP／埠／topic 不是身分：deviceId 不符即拒絕，不得配對或送命令。
            return Err(LinkError::Refused(format!(
                "device identity mismatch: expected {:?}, got {:?}",
                self.expected_device_id, hello
            )));
        }
        // 協定版本不同＝訊息語意可能不同：拒絕，不猜。
        if let Some(proto) = proto {
            if proto != PROTO_VERSION {
                return Err(LinkError::Refused(format!(
                    "device speaks protocol v{proto}, host speaks v{PROTO_VERSION}; refusing to guess"
                )));
            }
        }
        // 裝置自報「我需要配對」但 spec 沒有 pairingCode：握手誠實失敗，
        // 不得以「未配對」狀態送任何 cmd（那只會換來一連串 not-paired）。
        if device_wants_pairing && self.pairing_code.is_none() {
            return Err(LinkError::Refused(
                "device requires a pairing code but the adapter spec has none (add pairingCode)"
                    .into(),
            ));
        }
        if let Some(code) = &self.pairing_code {
            self.raw
                .send(encode_host_msg(&HostMsg::Pair { code: code.clone() }))
                .await?;
            let ok = wait_for(&mut rx, HANDSHAKE_TIMEOUT, |m| match m {
                DeviceMsg::PairOk => Some(true),
                DeviceMsg::PairFail => Some(false),
                _ => None,
            })
            .await
            .map_err(|e| handshake_wait_error("pairing", e))?;
            if !ok {
                return Err(LinkError::Refused("pairing code rejected by device".into()));
            }
        }
        // hello.caps 是裝置自報的能力清單：握手後才知道「這台裝置到底
        // 宣告了什麼」，後續 cmd 以此為準（沒宣告的能力不送線）。
        if caps.is_empty() {
            tracing::debug!(
                device = %self.expected_device_id,
                "device advertised no caps in hello; capability gating disabled for it"
            );
        }
        self.store_caps(caps);
        *done = (true, gen);
        self.ready_generation
            .store(ready_marker(gen), Ordering::SeqCst);
        Ok(())
    }

    /// 握手作廢：裝置端配對狀態被重置（MQTT 重連、ESP32 重開機）後，
    /// 下一個請求前必須重新 hello/pair —— 否則之後每個 cmd 都 not-paired。
    async fn invalidate_handshake(&self, why: &str) {
        tracing::warn!(
            device = %self.expected_device_id,
            reason = why,
            "device reports it is no longer paired; handshake invalidated (re-hello before the next request)"
        );
        self.ready_generation.store(0, Ordering::SeqCst);
        let mut done = self.handshaken.lock().await;
        done.0 = false;
    }

    /// 裝置明確拒絕：`not-paired` 代表裝置端配對狀態已重置。
    /// 這一次的請求仍誠實回失敗（**絕不自動重送實體命令**），
    /// 但握手要作廢，下一個請求前重新 hello/pair。
    async fn note_device_error(&self, msg: &DeviceMsg) {
        if let DeviceMsg::Err { reason, .. } = msg {
            if reason.contains("not-paired") {
                self.invalidate_handshake(reason).await;
            }
        }
    }

    /// 等回覆，並在「連線世代改變」時立刻中止：重連＝握手作廢，等待中的
    /// 請求不得沿用舊連線的語意繼續等（且佇列中的舊命令不再送出）。
    async fn wait_reply<T, F>(
        &self,
        rx: &mut broadcast::Receiver<DeviceMsg>,
        timeout: Duration,
        generation: u64,
        mut pick: F,
    ) -> Result<T, WaitEnd>
    where
        F: FnMut(&DeviceMsg) -> Option<T>,
    {
        const RESET_POLL: Duration = Duration::from_millis(100);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.raw.generation() != generation {
                return Err(WaitEnd::Reset);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(WaitEnd::TimedOut);
            }
            match tokio::time::timeout(remaining.min(RESET_POLL), rx.recv()).await {
                Ok(Ok(msg)) => {
                    if let Some(v) = pick(&msg) {
                        return Ok(v);
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => return Err(WaitEnd::TimedOut),
                // 只是輪詢窗到期：回頭檢查世代後繼續等。
                Err(_) => continue,
            }
        }
    }

    /// 送命令並等 ack。逾時＝結果未知（不重送）。
    pub async fn command(
        &self,
        action_id: &str,
        name: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<DeviceMsg, LinkError> {
        // 握手逾時 ≠ ack 逾時：前者代表 cmd 根本沒送出（沒有實體效果的
        // 未知），後者才是「送出了但不知道結果」。兩者不能共用 Timeout，
        // 否則收據會宣稱 dispatched——那是硬編的「已送出」。
        self.ensure_ready().await.map_err(|e| match e {
            LinkError::Timeout(detail) => LinkError::Unavailable(format!(
                "handshake did not complete ({detail}); no cmd was sent"
            )),
            other => other,
        })?;
        // 能力識別：裝置在 hello.caps 明確沒宣告 → 根本不把 cmd 送上線
        // （不試、不猜、不製造未知的實體效果）。
        if self.advertises(name) == Some(false) {
            let advertised = self
                .caps_snapshot()
                .map(|c| c.join(","))
                .unwrap_or_default();
            tracing::warn!(
                device = %self.expected_device_id,
                capability = %name,
                advertised = %advertised,
                "device did not advertise this capability in hello.caps; cmd NOT sent"
            );
            return Err(LinkError::NotAdvertised(format!(
                "裝置未宣告此能力：{name}（hello.caps: [{advertised}]）"
            )));
        }
        let mut rx = self.raw.subscribe();
        let generation = self.raw.generation();
        let msg = HostMsg::Cmd {
            id: action_id.to_string(),
            nonce: new_nonce(),
            name: name.to_string(),
            params,
        };
        // deadline＝ack 逾時：排在傳輸佇列裡的命令過期就不再送出。
        let deadline = std::time::Instant::now() + timeout;
        self.raw
            .send_before(encode_host_msg(&msg), deadline)
            .await?;
        let reply = self
            .wait_reply(&mut rx, timeout, generation, |m| match m {
                DeviceMsg::Ack { id: Some(id), .. } if id == action_id => Some(m.clone()),
                DeviceMsg::Err { id: Some(id), .. } if id == action_id => Some(m.clone()),
                // 裝置對「這一則」的明確拒絕不一定帶 id（not-paired／
                // bad-json／unknown-type／line-too-long）：等待期間收到就是
                // 這次請求的結果，誠實記為失敗，不要演成「逾時、結果未知」。
                DeviceMsg::Err { id: None, .. } => Some(m.clone()),
                _ => None,
            })
            .await
            .map_err(|end| match end {
                WaitEnd::Reset => LinkError::Reset(format!(
                    "link reset (reconnected) while waiting for action {action_id} — outcome UNKNOWN (not retried)"
                )),
                WaitEnd::TimedOut => LinkError::Timeout(format!(
                    "no ack for action {action_id} — outcome UNKNOWN (not retried: physical effects must not double-fire)"
                )),
            })?;
        self.note_device_error(&reply).await;
        Ok(reply)
    }

    /// 取消進行中的命令。
    pub async fn cancel(&self, action_id: &str, timeout: Duration) -> Result<DeviceMsg, LinkError> {
        self.ensure_ready().await?;
        let mut rx = self.raw.subscribe();
        let generation = self.raw.generation();
        let deadline = std::time::Instant::now() + timeout;
        self.raw
            .send_before(
                encode_host_msg(&HostMsg::Cancel {
                    id: action_id.to_string(),
                }),
                deadline,
            )
            .await?;
        let reply = self
            .wait_reply(&mut rx, timeout, generation, |m| match m {
                DeviceMsg::Ack { id: Some(id), .. } if id == action_id => Some(m.clone()),
                DeviceMsg::Err { id: Some(id), .. } if id == action_id => Some(m.clone()),
                DeviceMsg::Err { id: None, .. } => Some(m.clone()),
                _ => None,
            })
            .await
            .map_err(|end| match end {
                WaitEnd::Reset => {
                    LinkError::Reset(format!("link reset while cancelling {action_id}"))
                }
                WaitEnd::TimedOut => LinkError::Timeout(format!("no cancel ack for {action_id}")),
            })?;
        self.note_device_error(&reply).await;
        Ok(reply)
    }

    /// 請求一次狀態（獨立觀察來源）。回傳 `{"deviceId":…, "facts":{…}}`，
    /// spec 的 json-pointer 以此為根（例如 `/facts/distanceMm`）。
    ///
    /// 身分：`state.deviceId` 必須等於 expectedDeviceId——同一個 topic／埠上
    /// 冒名的 state 不得被當成這台裝置的觀察（丟棄＋warn）。
    pub async fn read_state(&self, timeout: Duration) -> Result<Value, LinkError> {
        self.ensure_ready().await?;
        let mut rx = self.raw.subscribe();
        let generation = self.raw.generation();
        let deadline = std::time::Instant::now() + timeout;
        self.raw
            .send_before(encode_host_msg(&HostMsg::Read), deadline)
            .await?;
        let expected = self.expected_device_id.clone();
        let reply = self
            .wait_reply(&mut rx, timeout, generation, |m| match m {
                DeviceMsg::State { device_id, facts } => {
                    if device_id.as_deref() == Some(expected.as_str()) {
                        Some(StateReply::State(serde_json::json!({
                            "deviceId": device_id,
                            "facts": facts,
                        })))
                    } else {
                        tracing::warn!(
                            expected = %expected,
                            got = %device_id.clone().unwrap_or_else(|| "<none>".into()),
                            "state from a foreign/anonymous deviceId discarded (a port or topic is not an identity)"
                        );
                        None
                    }
                }
                // 裝置明確拒絕這次 read（not-paired／bad-json…）：失敗，不是逾時。
                DeviceMsg::Err { id: None, reason } => Some(StateReply::Refused(reason.clone())),
                _ => None,
            })
            .await
            .map_err(|end| match end {
                WaitEnd::Reset => {
                    LinkError::Reset("link reset (reconnected) while reading device state".into())
                }
                WaitEnd::TimedOut => LinkError::Timeout("device did not answer read".into()),
            })?;
        match reply {
            StateReply::State(value) => Ok(value),
            StateReply::Refused(reason) => {
                self.note_device_error(&DeviceMsg::Err {
                    id: None,
                    reason: reason.clone(),
                })
                .await;
                Err(LinkError::Refused(format!("device refused read: {reason}")))
            }
        }
    }

    /// 緊急停止：送 stop-all 並**等裝置 ack**。沒有 ack ＝「已送出／未確認」，
    /// 誠實回 Err——runtime 的 estop 摘要才不會把沒確認的裝置算成已停止。
    pub async fn stop_all(&self, timeout: Duration) -> Result<(), LinkError> {
        // estop 路徑刻意不做完整握手：能送就送（配對過的連線本來就 ready；
        // 沒 ready 的連線 ensure_open 失敗就誠實回報）。
        self.raw.ensure_open().await?;
        let mut rx = self.raw.subscribe();
        let generation = self.raw.generation();
        let deadline = std::time::Instant::now() + timeout;
        self.raw
            .send_before(encode_host_msg(&HostMsg::StopAll), deadline)
            .await?;
        self.wait_reply(&mut rx, timeout, generation, |m| match m {
            DeviceMsg::Ack {
                stop_all: Some(true),
                ..
            } => Some(()),
            _ => None,
        })
        .await
        .map_err(|end| match end {
            WaitEnd::Reset => {
                LinkError::Reset("stop-all dispatched, link reset before ack — UNCONFIRMED".into())
            }
            WaitEnd::TimedOut => {
                LinkError::Timeout("stop-all dispatched, no ack — device stop UNCONFIRMED".into())
            }
        })
    }
}

/// `read_state` 等到的東西：狀態，或裝置對這次 read 的明確拒絕。
enum StateReply {
    State(Value),
    Refused(String),
}

/// `wait_reply` 的結束原因（逾時 vs 連線世代改變）。
enum WaitEnd {
    TimedOut,
    Reset,
}

/// DeviceLink 的誠實可用狀態（傳輸狀態＋握手狀態）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkReadiness {
    /// 已連線且完成當前世代的 hello/pair 握手。
    Ready,
    /// 已連線，但還沒（或需要重新）握手——首次讀取／命令時才做。
    NotHandshaken,
    Connecting,
    Disconnected,
    Closed,
}

impl<L: RawLink> LinkShutdown for DeviceLink<L> {
    fn shutdown(&self) {
        self.raw.shutdown();
        self.ready_generation.store(0, Ordering::SeqCst);
    }
    fn describe(&self) -> String {
        self.raw.describe()
    }
}

/// `wait_for` 的失敗原因。具體型別而不是 `()`：呼叫端要能誠實區分
/// 「裝置沒回」「連線已經沒了」「答覆可能被 broadcast 丟掉」——三者對
/// 收據與重連決策的意義不同，不能都講成 timeout。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitError {
    /// 期限內沒有符合條件的訊息。`lagged` 是等待期間 receiver 落後而被
    /// broadcast 丟掉的訊息數；`> 0` 表示答覆可能已經送到卻被丟了，
    /// 呼叫端不該把它講成「裝置沒回」。
    TimedOut { lagged: u64 },
    /// broadcast sender 全部 drop（連線世代結束）：不會再有訊息，等下去沒有意義。
    Closed,
    /// receiver 落後、broadcast 丟了 `n` 則訊息（只在 [`LagPolicy::Fail`] 下回傳）。
    Lagged(u64),
}

impl std::fmt::Display for WaitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaitError::TimedOut { lagged: 0 } => write!(f, "timed out"),
            WaitError::TimedOut { lagged } => write!(
                f,
                "timed out ({lagged} message(s) dropped while waiting; the reply may have been lost)"
            ),
            WaitError::Closed => write!(f, "link closed"),
            WaitError::Lagged(n) => write!(f, "receiver lagged ({n} message(s) dropped)"),
        }
    }
}

impl std::error::Error for WaitError {}

/// 等待期間 receiver 落後（broadcast 丟訊息）時怎麼辦。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LagPolicy {
    /// 繼續等後面的訊息，但把丟掉的數量記進 [`WaitError::TimedOut`]（握手預設）。
    #[default]
    Tolerate,
    /// 立刻以 [`WaitError::Lagged`] 結束：適用於「漏掉任何一則都不能繼續」的等待。
    Fail,
}

/// 在 broadcast stream 上等第一個符合條件的訊息（有界時間；lag 時繼續等）。
pub async fn wait_for<T, F>(
    rx: &mut broadcast::Receiver<DeviceMsg>,
    timeout: Duration,
    pick: F,
) -> Result<T, WaitError>
where
    F: FnMut(&DeviceMsg) -> Option<T>,
{
    wait_for_with(rx, timeout, LagPolicy::Tolerate, pick).await
}

/// [`wait_for`] 的完整版：可指定 lag 政策。
pub async fn wait_for_with<T, F>(
    rx: &mut broadcast::Receiver<DeviceMsg>,
    timeout: Duration,
    lag: LagPolicy,
    mut pick: F,
) -> Result<T, WaitError>
where
    F: FnMut(&DeviceMsg) -> Option<T>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let mut lagged: u64 = 0;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(WaitError::TimedOut { lagged });
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(msg)) => {
                if let Some(v) = pick(&msg) {
                    return Ok(v);
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                lagged = lagged.saturating_add(n);
                if lag == LagPolicy::Fail {
                    return Err(WaitError::Lagged(lagged));
                }
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => return Err(WaitError::Closed),
            Err(_) => return Err(WaitError::TimedOut { lagged }),
        }
    }
}

/// 握手等待失敗 → [`LinkError`]：連線已關閉是 `Reset`（不是裝置沒回），
/// 逾時才是 `Timeout`；lag 數量一併寫進 detail，收據不會把「答覆被丟」講成「裝置沒回」。
fn handshake_wait_error(what: &str, err: WaitError) -> LinkError {
    match err {
        WaitError::Closed => LinkError::Reset(format!("link closed while waiting for {what}")),
        WaitError::TimedOut { .. } | WaitError::Lagged(_) => {
            LinkError::Timeout(format!("device did not answer {what} ({err})"))
        }
    }
}

#[cfg(test)]
mod wait_for_tests {
    use super::*;

    fn ack(id: &str) -> DeviceMsg {
        DeviceMsg::Ack {
            id: Some(id.to_string()),
            applied: None,
            dup: None,
            cancelled: None,
            stop_all: None,
        }
    }

    #[tokio::test]
    async fn times_out_without_lag_is_a_plain_timeout() {
        let (tx, mut rx) = broadcast::channel::<DeviceMsg>(8);
        let err = wait_for(&mut rx, Duration::from_millis(30), |m| match m {
            DeviceMsg::Ack { id: Some(id), .. } if id == "never" => Some(()),
            _ => None,
        })
        .await
        .unwrap_err();
        assert_eq!(err, WaitError::TimedOut { lagged: 0 });
        assert_eq!(err.to_string(), "timed out");
        drop(tx);
    }

    #[tokio::test]
    async fn closed_channel_is_reported_as_closed_not_timeout() {
        let (tx, mut rx) = broadcast::channel::<DeviceMsg>(8);
        drop(tx);
        let err = wait_for(&mut rx, Duration::from_secs(5), |_| Some(()))
            .await
            .unwrap_err();
        assert_eq!(err, WaitError::Closed);
    }

    #[tokio::test]
    async fn lag_is_tolerated_by_default_and_counted_on_timeout() {
        let (tx, mut rx) = broadcast::channel::<DeviceMsg>(2);
        // 塞滿 capacity 2 的 channel 再多送兩則：receiver 會落後 2 則。
        for i in 0..4 {
            tx.send(ack(&format!("noise-{i}"))).unwrap();
        }
        let err = wait_for(&mut rx, Duration::from_millis(30), |m| match m {
            DeviceMsg::Ack { id: Some(id), .. } if id == "wanted" => Some(()),
            _ => None,
        })
        .await
        .unwrap_err();
        match err {
            WaitError::TimedOut { lagged } => assert!(lagged >= 2, "lagged={lagged}"),
            other => panic!("expected TimedOut with lag, got {other:?}"),
        }
        assert!(err.to_string().contains("dropped"), "{err}");
    }

    #[tokio::test]
    async fn lag_tolerant_wait_still_finds_a_later_reply() {
        let (tx, mut rx) = broadcast::channel::<DeviceMsg>(2);
        for i in 0..4 {
            tx.send(ack(&format!("noise-{i}"))).unwrap();
        }
        tx.send(ack("wanted")).unwrap();
        let got = wait_for(&mut rx, Duration::from_millis(200), |m| match m {
            DeviceMsg::Ack { id: Some(id), .. } if id == "wanted" => Some(id.clone()),
            _ => None,
        })
        .await
        .unwrap();
        assert_eq!(got, "wanted");
    }

    #[tokio::test]
    async fn lag_policy_fail_stops_immediately_with_lagged() {
        let (tx, mut rx) = broadcast::channel::<DeviceMsg>(2);
        for i in 0..4 {
            tx.send(ack(&format!("noise-{i}"))).unwrap();
        }
        tx.send(ack("wanted")).unwrap();
        let err = wait_for_with(
            &mut rx,
            Duration::from_secs(5),
            LagPolicy::Fail,
            |m| match m {
                DeviceMsg::Ack { id: Some(id), .. } if id == "wanted" => Some(()),
                _ => None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WaitError::Lagged(n) if n >= 2), "{err:?}");
    }

    #[test]
    fn handshake_errors_map_honestly() {
        assert!(matches!(
            handshake_wait_error("hello", WaitError::Closed),
            LinkError::Reset(_)
        ));
        match handshake_wait_error("hello", WaitError::TimedOut { lagged: 3 }) {
            LinkError::Timeout(detail) => {
                assert!(detail.contains("hello"), "{detail}");
                assert!(detail.contains("3 message(s) dropped"), "{detail}");
            }
            other => panic!("{other}"),
        }
        assert!(matches!(
            handshake_wait_error("pairing", WaitError::Lagged(1)),
            LinkError::Timeout(_)
        ));
    }
}
