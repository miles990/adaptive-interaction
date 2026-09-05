//! BLE GATT 傳輸（僅 macOS / Windows 目標編譯；Linux 誠實拒絕）。
//!
//! 模型：一個 service 下兩個 characteristic——command（host 寫入，
//! write-with-response）＋ state（裝置 notify）。訊息仍是同一套 JSON 協定
//! （一則訊息一個 write / notification；上限 480 bytes，超過誠實拒絕）。
//!
//! 誠實：藍牙關閉、系統未授權、掃描逾時、裝置不見了——全部以
//! Unavailable/Refused 明確回報；絕不假裝已送達。裝置名稱不是身分：
//! 連上後仍要 hello.deviceId＋配對碼握手（DeviceLink 統一處理）。

use crate::protocol::{parse_device_msg, DeviceMsg, LinkError, LinkState, RawLink, TaskSlot};
use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, PeripheralProperties,
    ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

const BROADCAST_CAP: usize = 64;
const SCAN_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_WRITE_BYTES: usize = 480;

/// 掃描時要不要連這台？**service UUID 為主、名稱為輔**——名稱不是身分。
///
/// 為什麼不能只比名稱：參考韌體把 128-bit service UUID 放在主廣播封包
/// （flags 3B＋UUID 18B 已佔掉 31B 的大半），名稱只放得進 scan response；
/// 而 NimBLE-Arduino 2.x 預設不廣播名稱，CoreBluetooth 對還沒連過的裝置
/// 也不填 name。掃描到我們的 service 卻因為 `local_name == None` 而「掃不到」
/// 是假陰性——身分本來就由連上後的 hello.deviceId＋配對碼決定。
///
/// 規則：
/// - 廣播含我們的 service UUID：名稱缺席也連；名稱存在但**不同**才不連
///   （同一個 service 的另一台裝置——要靠名稱分辨）。
/// - 廣播不含 service UUID（平台沒回報 services、或舊韌體）：只有名稱完全
///   相符才連（之後 discover_services 找不到 characteristic 會誠實 Refused）。
/// - `device_name` 為空＝不做名稱過濾（只看 service UUID）。
pub(crate) fn peripheral_matches(
    props: &PeripheralProperties,
    service_uuid: &uuid::Uuid,
    device_name: &str,
) -> bool {
    let advertises_service = props.services.iter().any(|s| s == service_uuid);
    let name_filter = (!device_name.is_empty()).then_some(device_name);
    match (advertises_service, name_filter, props.local_name.as_deref()) {
        // service 命中：名稱只在「有廣播且不同」時排除。
        (true, Some(wanted), Some(seen)) => seen == wanted,
        (true, _, _) => true,
        // service 沒命中：只有名稱完全相符才當候選。
        (false, Some(wanted), Some(seen)) => seen == wanted,
        (false, _, _) => false,
    }
}

/// 重組緩衝上限。裝置一直不送換行也不得讓緩衝無界成長
/// （parse_device_msg 本來就只解析 ≤16KB）。
const NOTIFY_BUFFER_MAX: usize = 16 * 1024;

/// device→host 的 notification 重組器。
///
/// 為什麼需要它：ATT 的 Handle-Value-Notification 可攜 payload 只有
/// `ATT_MTU − 3`（預設 MTU 23 → 20 bytes），超過的部分由協定棧直接截掉。
/// 參考韌體的 `state` 訊息在預設 deviceId 下就有 193 bytes——只要協商到的
/// MTU 不夠大，host 每次收到的都是破 JSON。舊版把解不出來的 bytes 無聲
/// 丟掉，`read_state` 只會逾時說「device did not answer read」，沒有任何
/// 一行 log 指向真因。
///
/// 規則（與韌體的分段送出配套）：
/// - 換行界定一則訊息：湊齊一行就解析、送出。
/// - 沒有換行時，若目前緩衝**整段**就是一則合法訊息，也送出（相容
///   「一則訊息一個 notification、不加換行」的舊韌體）。
/// - 解不開就 warn＋計數，絕不靜默丟棄。
#[derive(Default)]
pub(crate) struct NotifyAssembler {
    buffer: Vec<u8>,
}

impl NotifyAssembler {
    pub(crate) fn push(
        &mut self,
        chunk: &[u8],
        device: &str,
        inbound: &broadcast::Sender<DeviceMsg>,
        undecodable: &AtomicU64,
    ) {
        self.buffer.extend_from_slice(chunk);
        while let Some(idx) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=idx).collect();
            Self::emit(&line[..line.len() - 1], device, inbound, undecodable, true);
        }
        if self.buffer.is_empty() {
            return;
        }
        // 沒有換行：可能是舊韌體「一則訊息一個 notification」。整段解得開
        // 就當一則訊息；解不開就先留著等後續分段（超過上限才誠實丟棄）。
        if Self::emit(&self.buffer, device, inbound, undecodable, false) {
            self.buffer.clear();
        } else if self.buffer.len() > NOTIFY_BUFFER_MAX {
            undecodable.fetch_add(1, Ordering::SeqCst);
            tracing::warn!(
                device = %device,
                bytes = self.buffer.len(),
                "ble: unterminated notification data exceeded the buffer limit; discarded"
            );
            self.buffer.clear();
        }
    }

    /// 回傳「這段 bytes 是否是一則可解析的訊息」。`complete`＝這是一整行
    /// （換行界定），解不開就是真的壞掉，必須 warn＋計數。
    fn emit(
        bytes: &[u8],
        device: &str,
        inbound: &broadcast::Sender<DeviceMsg>,
        undecodable: &AtomicU64,
        complete: bool,
    ) -> bool {
        let text = std::str::from_utf8(bytes).ok();
        if let Some(msg) = text.and_then(parse_device_msg) {
            let _ = inbound.send(msg);
            return true;
        }
        if complete && !bytes.iter().all(|b| b.is_ascii_whitespace()) {
            undecodable.fetch_add(1, Ordering::SeqCst);
            tracing::warn!(
                device = %device,
                bytes = bytes.len(),
                utf8 = text.is_some(),
                "ble: a notification could not be decoded (truncated by the ATT MTU, or not this \
                 protocol); discarded"
            );
        }
        false
    }
}

// link_state 的原子編碼（與 serial/mqtt 對齊）。BLE 是「用到才連」，
// 所以初始狀態是 Connecting（尚未連線，但會在首次使用時連）。
const STATE_CONNECTING: u8 = 0;
const STATE_CONNECTED: u8 = 1;
const STATE_DISCONNECTED: u8 = 2;
const STATE_CLOSED: u8 = 3;

/// 斷線觀察者看得懂的事件（只保留「是不是這台裝置斷了」）。
/// 抽掉 btleplug 型別是為了不碰藍牙堆疊也能測狀態機。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkEvent<I> {
    Disconnected(I),
    Other,
}

/// 事件驅動的斷線偵測：收到**我們這台** peripheral 的 DeviceDisconnected 就把
/// state 從 CONNECTED 翻成 DISCONNECTED。
///
/// 為什麼需要：舊版只在 `ensure_open()` 時才發現斷線，只有動器沒有受器的
/// BLE adapter 在裝置離開範圍後會無限期回報 healthy／available——status()
/// 讀的是一個過期的旗標，等於對「此刻能不能用」說謊。
///
/// `compare_exchange` 而非 store：不得覆蓋 CLOSED（已被 shutdown），也不得
/// 讓上一世代的觀察者把新連線標成斷線（舊觀察者在重連前就被 abort，
/// 這是第二層保險）。
pub(crate) async fn watch_for_disconnect<S, I>(mut events: S, target: I, state: Arc<AtomicU8>)
where
    S: futures::Stream<Item = LinkEvent<I>> + Unpin,
    I: PartialEq,
{
    while let Some(event) = events.next().await {
        if let LinkEvent::Disconnected(id) = event {
            if id == target {
                let _ = state.compare_exchange(
                    STATE_CONNECTED,
                    STATE_DISCONNECTED,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
                return;
            }
        }
    }
    // 事件流結束（配接器沒了）：連線狀態不再可信，誠實降級。
    let _ = state.compare_exchange(
        STATE_CONNECTED,
        STATE_DISCONNECTED,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
}

/// 掃描守衛：不論怎麼離開 `connect()`——正常 return、`?` 提早返回，或呼叫端
/// （例如 estop 的 2 秒逾時）直接把整個 future 丟掉——都要把掃描關掉。
/// 舊版只在 return 路徑呼叫 `stop_scan()`，被取消時 radio 會一直掃到逾時。
pub(crate) struct ScanGuard<F: FnOnce()> {
    stop: Option<F>,
}

impl<F: FnOnce()> ScanGuard<F> {
    pub(crate) fn new(stop: F) -> Self {
        Self { stop: Some(stop) }
    }

    /// 已經自己關過了：解除守衛（不要關第二次）。
    pub(crate) fn disarm(&mut self) {
        self.stop = None;
    }
}

impl<F: FnOnce()> Drop for ScanGuard<F> {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

struct BleSession {
    peripheral: Peripheral,
    command_char: Characteristic,
}

pub struct BleRawLink {
    device_name: String,
    service_uuid: uuid::Uuid,
    command_uuid: uuid::Uuid,
    state_uuid: uuid::Uuid,
    inbound: broadcast::Sender<DeviceMsg>,
    session: Arc<Mutex<Option<BleSession>>>,
    generation: Arc<AtomicU64>,
    state: Arc<AtomicU8>,
    closed: Arc<AtomicBool>,
    /// notification → broadcast 的 task：每次重連前 abort 舊的，
    /// shutdown 時一併回收（否則每次重連都留一條殭屍 task）。
    notify_task: TaskSlot,
    /// CentralEvent::DeviceDisconnected 觀察者：讓 connected()／health 在
    /// 裝置離開範圍時立刻反映，而不是等下一次派工才發現。
    disconnect_task: TaskSlot,
    /// 收到但解不開的 notification 數（被 ATT MTU 截斷、非 UTF-8、非本協定）。
    /// 靜默丟棄會把「裝置有回、我們讀不懂」講成「裝置沒回」。
    undecodable: Arc<AtomicU64>,
}

impl BleRawLink {
    pub fn new(
        device_name: String,
        service_uuid: uuid::Uuid,
        command_uuid: uuid::Uuid,
        state_uuid: uuid::Uuid,
    ) -> Arc<Self> {
        let (inbound, _) = broadcast::channel(BROADCAST_CAP);
        Arc::new(Self {
            device_name,
            service_uuid,
            command_uuid,
            state_uuid,
            inbound,
            session: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            state: Arc::new(AtomicU8::new(STATE_CONNECTING)),
            closed: Arc::new(AtomicBool::new(false)),
            notify_task: TaskSlot::new(),
            disconnect_task: TaskSlot::new(),
            undecodable: Arc::new(AtomicU64::new(0)),
        })
    }

    /// 收到但解不開的 notification 累計數（診斷／誠實逾時訊息用）。
    pub fn undecodable_notifications(&self) -> u64 {
        self.undecodable.load(Ordering::SeqCst)
    }

    /// notification task 是否仍在跑（回收驗證用）。
    pub fn notification_task_active(&self) -> bool {
        self.notify_task.is_active()
    }

    async fn adapter() -> Result<Adapter, LinkError> {
        let manager = Manager::new()
            .await
            .map_err(|e| LinkError::Unavailable(format!("bluetooth manager: {e}")))?;
        manager
            .adapters()
            .await
            .map_err(|e| LinkError::Unavailable(format!("bluetooth adapters: {e}")))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                LinkError::Unavailable(
                    "no bluetooth adapter (bluetooth off, or the app lacks bluetooth permission)"
                        .into(),
                )
            })
    }

    async fn connect(&self) -> Result<BleSession, LinkError> {
        let adapter = Self::adapter().await?;
        adapter
            .start_scan(ScanFilter {
                services: vec![self.service_uuid],
            })
            .await
            .map_err(|e| LinkError::Unavailable(format!("ble scan: {e}")))?;
        // 掃描一旦開始就必須關得掉——包含「這個 future 被取消」的情況
        // （estop 對每個動器只等 2 秒，掃描卻是 6 秒）。
        let mut scan_guard = {
            let adapter = adapter.clone();
            ScanGuard::new(move || {
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        let _ = adapter.stop_scan().await;
                    });
                }
            })
        };
        // 有界掃描：service UUID 命中即連（名稱只當可選過濾；身分交給
        // 連上後的 hello.deviceId＋配對碼——見 peripheral_matches）。
        let deadline = tokio::time::Instant::now() + SCAN_TIMEOUT;
        let mut events = adapter
            .events()
            .await
            .map_err(|e| LinkError::Unavailable(format!("ble events: {e}")))?;
        let peripheral = loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                scan_guard.disarm();
                let _ = adapter.stop_scan().await;
                return Err(LinkError::Unavailable(format!(
                    "ble device {:?} (service {}) not found within {}s scan",
                    self.device_name,
                    self.service_uuid,
                    SCAN_TIMEOUT.as_secs()
                )));
            }
            match tokio::time::timeout(remaining, events.next()).await {
                Ok(Some(CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id))) => {
                    if let Ok(p) = adapter.peripheral(&id).await {
                        let props = p.properties().await.ok().flatten();
                        if let Some(props) = props {
                            if peripheral_matches(&props, &self.service_uuid, &self.device_name) {
                                break p;
                            }
                        }
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => {
                    scan_guard.disarm();
                    let _ = adapter.stop_scan().await;
                    return Err(LinkError::Unavailable(format!(
                        "ble device {:?} not found (scan ended)",
                        self.device_name
                    )));
                }
            }
        };
        scan_guard.disarm();
        let _ = adapter.stop_scan().await;
        peripheral
            .connect()
            .await
            .map_err(|e| LinkError::Unavailable(format!("ble connect: {e}")))?;
        peripheral
            .discover_services()
            .await
            .map_err(|e| LinkError::Unavailable(format!("ble discover: {e}")))?;
        let chars = peripheral.characteristics();
        let command_char = chars
            .iter()
            .find(|c| c.uuid == self.command_uuid)
            .cloned()
            .ok_or_else(|| LinkError::Refused("device lacks the command characteristic".into()))?;
        let state_char = chars
            .iter()
            .find(|c| c.uuid == self.state_uuid)
            .cloned()
            .ok_or_else(|| LinkError::Refused("device lacks the state characteristic".into()))?;
        peripheral
            .subscribe(&state_char)
            .await
            .map_err(|e| LinkError::Unavailable(format!("ble subscribe: {e}")))?;
        // notification → broadcast。
        let mut notifications = peripheral
            .notifications()
            .await
            .map_err(|e| LinkError::Unavailable(format!("ble notifications: {e}")))?;
        let inbound = self.inbound.clone();
        let state_uuid = self.state_uuid;
        let notify_state = self.state.clone();
        let notify_undecodable = self.undecodable.clone();
        let notify_device = self.device_name.clone();
        // 新 task 取代舊的（TaskSlot::replace 會 abort 前一條）——
        // 重連不得累積殭屍 notification task。
        self.notify_task.replace(tokio::spawn(async move {
            let mut assembler = NotifyAssembler::default();
            while let Some(data) = notifications.next().await {
                if data.uuid == state_uuid {
                    assembler.push(&data.value, &notify_device, &inbound, &notify_undecodable);
                }
            }
            // notification stream 結束＝GATT session 沒了（某些後端只以此
            // 表示斷線）：狀態不得停在 connected。
            let _ = notify_state.compare_exchange(
                STATE_CONNECTED,
                STATE_DISCONNECTED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }));
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.state.store(STATE_CONNECTED, Ordering::SeqCst);
        // 事件驅動的斷線偵測：裝置離開範圍後 connected()／health 立刻誠實。
        let target = peripheral.id();
        let disconnect_state = self.state.clone();
        let events = events.map(|event| match event {
            CentralEvent::DeviceDisconnected(id) => LinkEvent::Disconnected(id),
            _ => LinkEvent::Other,
        });
        self.disconnect_task
            .replace(tokio::spawn(watch_for_disconnect(
                Box::pin(events),
                target,
                disconnect_state,
            )));
        Ok(BleSession {
            peripheral,
            command_char,
        })
    }
}

#[async_trait::async_trait]
impl RawLink for BleRawLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable(format!(
                "ble link {:?} was closed by the host (provider disabled/revoked); \
                 re-enabling requires reloading the adapter spec",
                self.device_name
            )));
        }
        let mut session = self.session.lock().await;
        if let Some(existing) = session.as_ref() {
            if existing.peripheral.is_connected().await.unwrap_or(false) {
                return Ok(());
            }
            // 斷線：丟掉舊 session（連同 notification／斷線觀察 task），
            // 重連＝新世代＝重新握手。
            *session = None;
            self.notify_task.abort();
            self.disconnect_task.abort();
            self.state.store(STATE_DISCONNECTED, Ordering::SeqCst);
        }
        match self.connect().await {
            Ok(fresh) => {
                *session = Some(fresh);
                Ok(())
            }
            Err(e) => {
                // 連不上就誠實記成 Disconnected（health 不得停在「連線中」）。
                self.state.store(STATE_DISCONNECTED, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable(format!(
                "ble link {:?} is closed; nothing was written",
                self.device_name
            )));
        }
        if line.len() > MAX_WRITE_BYTES {
            return Err(LinkError::Refused(format!(
                "ble message too large ({} bytes > {MAX_WRITE_BYTES})",
                line.len()
            )));
        }
        let session = self.session.lock().await;
        let Some(session) = session.as_ref() else {
            return Err(LinkError::Unavailable("ble not connected".into()));
        };
        session
            .peripheral
            .write(
                &session.command_char,
                line.as_bytes(),
                WriteType::WithResponse,
            )
            .await
            // write 途中失敗＝位元組可能已經進了裝置：結果未知，呼叫端
            // 不得重送（重送會讓實體效果重複觸發）。「確定沒送出」的情況
            // （已關閉、太長、未連線）在上面就已經以 Unavailable/Refused 回報。
            .map_err(|e| LinkError::Uncertain(format!("ble write failed mid-flight: {e}")))
    }

    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        self.inbound.subscribe()
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// BLE 的單則上限（一則訊息一個 write／notification）。
    fn max_line_bytes(&self) -> Option<usize> {
        Some(MAX_WRITE_BYTES)
    }

    fn connected(&self) -> bool {
        // GATT session 存在＝連線中。斷線有三個來源，都會把 state 翻掉：
        // CentralEvent::DeviceDisconnected 觀察者（事件驅動、最即時）、
        // notification stream 結束、以及 ensure_open 的 is_connected 檢查。
        // 這裡只讀旗標，不做阻塞 I/O（健康檢查不能卡住）。
        self.state.load(Ordering::SeqCst) == STATE_CONNECTED && !self.closed.load(Ordering::SeqCst)
    }

    fn link_state(&self) -> LinkState {
        if self.closed.load(Ordering::SeqCst) {
            return LinkState::Closed;
        }
        match self.state.load(Ordering::SeqCst) {
            STATE_CONNECTED => LinkState::Connected,
            STATE_CONNECTING => LinkState::Connecting,
            STATE_CLOSED => LinkState::Closed,
            _ => LinkState::Disconnected,
        }
    }

    fn undecodable_messages(&self) -> u64 {
        self.undecodable.load(Ordering::SeqCst)
    }

    fn shutdown(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return; // 冪等
        }
        self.state.store(STATE_CLOSED, Ordering::SeqCst);
        self.notify_task.abort();
        self.disconnect_task.abort();
        // GATT disconnect 是 async：有 runtime 就背景做（有界地做完就好），
        // 沒有 runtime（程式收尾）就只丟掉 session，由 OS 收 socket。
        let session = self.session.clone();
        let name = self.device_name.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let mut guard = session.lock().await;
                    if let Some(existing) = guard.take() {
                        if let Err(e) = existing.peripheral.disconnect().await {
                            tracing::debug!(device = %name, error = %e, "ble disconnect failed");
                        }
                    }
                });
            }
            Err(_) => {
                if let Ok(mut guard) = session.try_lock() {
                    *guard = None;
                }
            }
        }
    }

    fn describe(&self) -> String {
        format!("ble {:?} service {}", self.device_name, self.service_uuid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> uuid::Uuid {
        "7f2a0001-c701-4c9e-8f7e-2b3d5a1e9c01".parse().unwrap()
    }

    fn props(services: &[uuid::Uuid], name: Option<&str>) -> PeripheralProperties {
        PeripheralProperties {
            local_name: name.map(String::from),
            services: services.to_vec(),
            ..Default::default()
        }
    }

    /// 真板的廣播：主封包只有 flags＋128-bit service UUID，名稱不在
    /// （NimBLE 2.x 預設不廣播名稱；scan response 可能還沒到；CoreBluetooth
    /// 對未連過的裝置不填 name）。舊版只比名稱 → 永遠「掃不到」。
    #[test]
    fn a_service_match_without_a_local_name_is_a_candidate() {
        assert!(peripheral_matches(
            &props(&[svc()], None),
            &svc(),
            "esp32-companion-01"
        ));
    }

    #[test]
    fn a_service_match_with_the_same_name_is_a_candidate() {
        assert!(peripheral_matches(
            &props(&[svc()], Some("esp32-companion-01")),
            &svc(),
            "esp32-companion-01"
        ));
    }

    /// 同一個 service 的另一台裝置（名稱不同）：不連——名稱是這時唯一能
    /// 分辨兩台同款裝置的資訊。
    #[test]
    fn a_service_match_with_a_different_name_is_not_a_candidate() {
        assert!(!peripheral_matches(
            &props(&[svc()], Some("esp32-companion-02")),
            &svc(),
            "esp32-companion-01"
        ));
    }

    /// 平台沒回報 services（或舊韌體）時，名稱完全相符仍是候選；
    /// 名稱不符或缺席就不是。
    #[test]
    fn without_the_service_only_an_exact_name_matches() {
        assert!(peripheral_matches(
            &props(&[], Some("esp32-companion-01")),
            &svc(),
            "esp32-companion-01"
        ));
        assert!(!peripheral_matches(
            &props(&[], Some("other")),
            &svc(),
            "esp32-companion-01"
        ));
        assert!(!peripheral_matches(
            &props(&[], None),
            &svc(),
            "esp32-companion-01"
        ));
    }

    /// 空的 deviceName＝不做名稱過濾：只看 service UUID。
    #[test]
    fn an_empty_device_name_disables_the_name_filter() {
        assert!(peripheral_matches(
            &props(&[svc()], Some("whatever")),
            &svc(),
            ""
        ));
        assert!(!peripheral_matches(
            &props(&[], Some("whatever")),
            &svc(),
            ""
        ));
    }

    /// 斷線狀態機（不碰藍牙堆疊）：收到**我們這台**的 DeviceDisconnected
    /// 就必須把 connected 旗標翻掉——舊版只在下一次派工時才發現，
    /// 只有動器的 adapter 會無限期回報 healthy／available。
    #[tokio::test]
    async fn a_disconnect_event_for_our_device_flips_the_state() {
        let state = Arc::new(AtomicU8::new(STATE_CONNECTED));
        let events = futures::stream::iter(vec![
            LinkEvent::Other,
            LinkEvent::Disconnected("someone-else"),
            LinkEvent::Disconnected("ours"),
        ]);
        watch_for_disconnect(events, "ours", state.clone()).await;
        assert_eq!(state.load(Ordering::SeqCst), STATE_DISCONNECTED);
    }

    /// 別台裝置斷線與我們無關：狀態不得被動到。
    #[tokio::test]
    async fn another_devices_disconnect_leaves_us_connected() {
        let state = Arc::new(AtomicU8::new(STATE_CONNECTED));
        let events = futures::stream::iter(vec![LinkEvent::Disconnected("someone-else")]);
        watch_for_disconnect(events, "ours", state.clone()).await;
        // 事件流結束才降級（配接器沒了＝狀態不再可信）——但不是「別人斷線」造成的。
        assert_eq!(state.load(Ordering::SeqCst), STATE_DISCONNECTED);
    }

    /// 已經 shutdown（CLOSED）的連線不得被斷線事件改回 DISCONNECTED：
    /// 「被主機關閉」與「裝置跑掉」是兩種不同的誠實訊息。
    #[tokio::test]
    async fn a_closed_link_is_never_downgraded_by_a_late_event() {
        let state = Arc::new(AtomicU8::new(STATE_CLOSED));
        let events = futures::stream::iter(vec![LinkEvent::Disconnected("ours")]);
        watch_for_disconnect(events, "ours", state.clone()).await;
        assert_eq!(state.load(Ordering::SeqCst), STATE_CLOSED);
    }

    /// 掃描守衛：future 被取消（drop）時掃描仍要被關掉；已經自己關過就不重關。
    #[test]
    fn a_cancelled_scan_is_still_stopped() {
        let stopped = Arc::new(AtomicBool::new(false));
        {
            let flag = stopped.clone();
            let _guard = ScanGuard::new(move || flag.store(true, Ordering::SeqCst));
        }
        assert!(stopped.load(Ordering::SeqCst), "被丟掉時必須關閉掃描");

        let stopped2 = Arc::new(AtomicBool::new(false));
        {
            let flag = stopped2.clone();
            let mut guard = ScanGuard::new(move || flag.store(true, Ordering::SeqCst));
            guard.disarm();
        }
        assert!(!stopped2.load(Ordering::SeqCst), "解除後不得再關一次");
    }

    /// 韌體 buildState() 在預設 deviceId 下產生的 state（193 bytes）。
    /// 用它當基準，才不會把「這個問題只在很長的訊息上發生」講成理論。
    fn firmware_state_json() -> String {
        r#"{"type":"state","deviceId":"esp32-companion-01","facts":{"button":false,"distanceMm":842,"lux":133,"tempC":24.5,"vibeActive":false,"buzzActive":false,"servoAngle":90,"led":{"r":0,"g":0,"b":0}}}"#.to_string()
    }

    /// protocol-conformance-028：被 ATT MTU 截斷的 notification 不得靜默消失。
    /// 舊版 `if let Ok(text) … if let Some(msg) …` 沒有 else 分支：host 只會
    /// 逾時說「device did not answer read」，沒有一行 log 指向真因。
    #[test]
    fn a_truncated_notification_is_counted_and_never_silently_dropped() {
        let state = firmware_state_json();
        assert!(
            state.len() > 182,
            "the reference state must exceed a 185-MTU payload: {} bytes",
            state.len()
        );
        let (tx, mut rx) = broadcast::channel(8);
        let undecodable = AtomicU64::new(0);
        let mut assembler = NotifyAssembler::default();
        // 協定棧把它截到 MTU-3 = 182 bytes，並（如韌體般）以換行結尾。
        let mut truncated = state.as_bytes()[..182].to_vec();
        truncated.push(b'\n');
        assembler.push(&truncated, "esp32-companion-01", &tx, &undecodable);
        assert!(rx.try_recv().is_err(), "破 JSON 不得被當成一則訊息");
        assert_eq!(
            undecodable.load(Ordering::SeqCst),
            1,
            "解不開的 notification 必須被計數（等待逾時才說得出真因）"
        );
    }

    /// 韌體端分段送出（每段 ≤ MTU-3、換行結尾）後，host 必須重組回同一則。
    #[test]
    fn a_chunked_notification_is_reassembled_into_one_message() {
        let state = firmware_state_json();
        let (tx, mut rx) = broadcast::channel(8);
        let undecodable = AtomicU64::new(0);
        let mut assembler = NotifyAssembler::default();
        // 最保守的情況：協商失敗，一次只送得動 ATT 預設的 20 bytes。
        for chunk in state.as_bytes().chunks(20) {
            assembler.push(chunk, "esp32-companion-01", &tx, &undecodable);
        }
        assembler.push(b"\n", "esp32-companion-01", &tx, &undecodable);
        match rx.try_recv() {
            Ok(DeviceMsg::State { device_id, facts }) => {
                assert_eq!(device_id.as_deref(), Some("esp32-companion-01"));
                assert_eq!(facts["distanceMm"], serde_json::json!(842));
            }
            other => panic!("分段的 state 必須被重組回來，得到 {other:?}"),
        }
        assert_eq!(undecodable.load(Ordering::SeqCst), 0, "重組成功不算解不開");
    }

    /// 相容舊韌體：「一則訊息一個 notification、不加換行」仍要能用。
    #[test]
    fn a_whole_message_without_a_newline_still_works() {
        let (tx, mut rx) = broadcast::channel(8);
        let undecodable = AtomicU64::new(0);
        let mut assembler = NotifyAssembler::default();
        assembler.push(
            br#"{"type":"pair-ok"}"#,
            "esp32-companion-01",
            &tx,
            &undecodable,
        );
        assert!(matches!(rx.try_recv(), Ok(DeviceMsg::PairOk)));
        assert_eq!(undecodable.load(Ordering::SeqCst), 0);
    }

    /// 一直不送換行也不得讓緩衝無界成長（有界解析）。
    #[test]
    fn unterminated_notification_data_is_bounded() {
        let (tx, _rx) = broadcast::channel(8);
        let undecodable = AtomicU64::new(0);
        let mut assembler = NotifyAssembler::default();
        let junk = vec![b'x'; 4096];
        for _ in 0..6 {
            assembler.push(&junk, "esp32-companion-01", &tx, &undecodable);
        }
        assert_eq!(
            undecodable.load(Ordering::SeqCst),
            1,
            "超過上限要誠實丟棄＋計數，不得無限累積"
        );
        assert!(assembler.buffer.len() <= NOTIFY_BUFFER_MAX);
    }

    /// 別的 service、沒有名稱：絕不連（掃描過濾器只是建議，事件仍可能來）。
    #[test]
    fn a_foreign_service_without_a_name_is_ignored() {
        let other: uuid::Uuid = "0000180f-0000-1000-8000-00805f9b34fb".parse().unwrap();
        assert!(!peripheral_matches(
            &props(&[other], None),
            &svc(),
            "esp32-companion-01"
        ));
    }
}
