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

// link_state 的原子編碼（與 serial/mqtt 對齊）。BLE 是「用到才連」，
// 所以初始狀態是 Connecting（尚未連線，但會在首次使用時連）。
const STATE_CONNECTING: u8 = 0;
const STATE_CONNECTED: u8 = 1;
const STATE_DISCONNECTED: u8 = 2;
const STATE_CLOSED: u8 = 3;

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
        })
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
                    let _ = adapter.stop_scan().await;
                    return Err(LinkError::Unavailable(format!(
                        "ble device {:?} not found (scan ended)",
                        self.device_name
                    )));
                }
            }
        };
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
        // 新 task 取代舊的（TaskSlot::replace 會 abort 前一條）——
        // 重連不得累積殭屍 notification task。
        self.notify_task.replace(tokio::spawn(async move {
            while let Some(data) = notifications.next().await {
                if data.uuid == state_uuid {
                    if let Ok(text) = std::str::from_utf8(&data.value) {
                        if let Some(msg) = parse_device_msg(text) {
                            let _ = inbound.send(msg);
                        }
                    }
                }
            }
        }));
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.state.store(STATE_CONNECTED, Ordering::SeqCst);
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
            // 斷線：丟掉舊 session（連同 notification task），
            // 重連＝新世代＝重新握手。
            *session = None;
            self.notify_task.abort();
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

    fn connected(&self) -> bool {
        // GATT session 存在＝連線中（斷線由 ensure_open 的 is_connected
        // 檢查與 state 一起更新；這裡不做阻塞 I/O）。
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

    fn shutdown(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return; // 冪等
        }
        self.state.store(STATE_CLOSED, Ordering::SeqCst);
        self.notify_task.abort();
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
