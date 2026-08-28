//! BLE GATT 傳輸（僅 macOS / Windows 目標編譯；Linux 誠實拒絕）。
//!
//! 模型：一個 service 下兩個 characteristic——command（host 寫入，
//! write-with-response）＋ state（裝置 notify）。訊息仍是同一套 JSON 協定
//! （一則訊息一個 write / notification；上限 480 bytes，超過誠實拒絕）。
//!
//! 誠實：藍牙關閉、系統未授權、掃描逾時、裝置不見了——全部以
//! Unavailable/Refused 明確回報；絕不假裝已送達。裝置名稱不是身分：
//! 連上後仍要 hello.deviceId＋配對碼握手（DeviceLink 統一處理）。

use crate::protocol::{parse_device_msg, DeviceMsg, LinkError, RawLink};
use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

const BROADCAST_CAP: usize = 64;
const SCAN_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_WRITE_BYTES: usize = 480;

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
    session: Mutex<Option<BleSession>>,
    generation: Arc<AtomicU64>,
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
            session: Mutex::new(None),
            generation: Arc::new(AtomicU64::new(0)),
        })
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
        // 有界掃描：找名字相符的裝置。
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
                    "ble device {:?} not found within {}s scan",
                    self.device_name,
                    SCAN_TIMEOUT.as_secs()
                )));
            }
            match tokio::time::timeout(remaining, events.next()).await {
                Ok(Some(CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id))) => {
                    if let Ok(p) = adapter.peripheral(&id).await {
                        let name = p
                            .properties()
                            .await
                            .ok()
                            .flatten()
                            .and_then(|props| props.local_name);
                        if name.as_deref() == Some(self.device_name.as_str()) {
                            break p;
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
        tokio::spawn(async move {
            while let Some(data) = notifications.next().await {
                if data.uuid == state_uuid {
                    if let Ok(text) = std::str::from_utf8(&data.value) {
                        if let Some(msg) = parse_device_msg(text) {
                            let _ = inbound.send(msg);
                        }
                    }
                }
            }
        });
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(BleSession {
            peripheral,
            command_char,
        })
    }
}

#[async_trait::async_trait]
impl RawLink for BleRawLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        let mut session = self.session.lock().await;
        if let Some(existing) = session.as_ref() {
            if existing.peripheral.is_connected().await.unwrap_or(false) {
                return Ok(());
            }
            *session = None; // 斷線：丟掉舊 session，重連＝新世代＝重新握手
        }
        *session = Some(self.connect().await?);
        Ok(())
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
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
            .map_err(|e| LinkError::Unavailable(format!("ble write: {e}")))
    }

    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        self.inbound.subscribe()
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn describe(&self) -> String {
        format!("ble {:?} service {}", self.device_name, self.service_uuid)
    }
}
