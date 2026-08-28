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
use std::sync::Arc;
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
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Unavailable(s) => write!(f, "unavailable: {s}"),
            LinkError::Refused(s) => write!(f, "refused: {s}"),
            LinkError::Timeout(s) => write!(f, "timeout: {s}"),
        }
    }
}

/// 傳輸層：送一則訊息＋訂閱裝置訊息。實作負責自己的重連/退避；
/// `ensure_open` 失敗必須誠實回報，不得默默排隊。
#[async_trait::async_trait]
pub trait RawLink: Send + Sync {
    async fn ensure_open(&self) -> Result<(), LinkError>;
    async fn send(&self, line: String) -> Result<(), LinkError>;
    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg>;
    /// 連線世代：每次（重）連線遞增。DeviceLink 用它偵測「重連過」並
    /// 重新走 hello/pair 握手——重連不得沿用舊握手。
    fn generation(&self) -> u64 {
        0
    }
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
    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        (**self).subscribe()
    }
    fn generation(&self) -> u64 {
        (**self).generation()
    }
    fn describe(&self) -> String {
        (**self).describe()
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
}

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(2_500);

impl<L: RawLink> DeviceLink<L> {
    pub fn new(raw: L, expected_device_id: String, pairing_code: Option<String>) -> Self {
        Self {
            raw,
            expected_device_id,
            pairing_code,
            handshaken: tokio::sync::Mutex::new((false, 0)),
        }
    }

    pub fn raw(&self) -> &L {
        &self.raw
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
        let mut rx = self.raw.subscribe();
        self.raw.send(encode_host_msg(&HostMsg::Who)).await?;
        let hello = wait_for(&mut rx, HANDSHAKE_TIMEOUT, |m| match m {
            DeviceMsg::Hello { device_id, .. } => Some(device_id.clone()),
            _ => None,
        })
        .await
        .map_err(|_| LinkError::Timeout("device did not answer hello".into()))?;
        if hello != self.expected_device_id {
            // IP／埠／topic 不是身分：deviceId 不符即拒絕，不得配對或送命令。
            return Err(LinkError::Refused(format!(
                "device identity mismatch: expected {:?}, got {:?}",
                self.expected_device_id, hello
            )));
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
            .map_err(|_| LinkError::Timeout("device did not answer pairing".into()))?;
            if !ok {
                return Err(LinkError::Refused("pairing code rejected by device".into()));
            }
        }
        *done = (true, gen);
        Ok(())
    }

    /// 送命令並等 ack。逾時＝結果未知（不重送）。
    pub async fn command(
        &self,
        action_id: &str,
        name: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<DeviceMsg, LinkError> {
        self.ensure_ready().await?;
        let mut rx = self.raw.subscribe();
        let msg = HostMsg::Cmd {
            id: action_id.to_string(),
            nonce: new_nonce(),
            name: name.to_string(),
            params,
        };
        self.raw.send(encode_host_msg(&msg)).await?;
        wait_for(&mut rx, timeout, |m| match m {
            DeviceMsg::Ack { id: Some(id), .. } if id == action_id => Some(m.clone()),
            DeviceMsg::Err { id: Some(id), .. } if id == action_id => Some(m.clone()),
            _ => None,
        })
        .await
        .map_err(|_| {
            LinkError::Timeout(format!(
                "no ack for action {action_id} — outcome UNKNOWN (not retried: physical effects must not double-fire)"
            ))
        })
    }

    /// 取消進行中的命令。
    pub async fn cancel(&self, action_id: &str, timeout: Duration) -> Result<DeviceMsg, LinkError> {
        self.ensure_ready().await?;
        let mut rx = self.raw.subscribe();
        self.raw
            .send(encode_host_msg(&HostMsg::Cancel {
                id: action_id.to_string(),
            }))
            .await?;
        wait_for(&mut rx, timeout, |m| match m {
            DeviceMsg::Ack { id: Some(id), .. } if id == action_id => Some(m.clone()),
            DeviceMsg::Err { id: Some(id), .. } if id == action_id => Some(m.clone()),
            _ => None,
        })
        .await
        .map_err(|_| LinkError::Timeout(format!("no cancel ack for {action_id}")))
    }

    /// 請求一次狀態（獨立觀察來源）。回傳 `{"deviceId":…, "facts":{…}}`，
    /// spec 的 json-pointer 以此為根（例如 `/facts/distanceMm`）。
    pub async fn read_state(&self, timeout: Duration) -> Result<Value, LinkError> {
        self.ensure_ready().await?;
        let mut rx = self.raw.subscribe();
        self.raw.send(encode_host_msg(&HostMsg::Read)).await?;
        wait_for(&mut rx, timeout, |m| match m {
            DeviceMsg::State { device_id, facts } => Some(serde_json::json!({
                "deviceId": device_id,
                "facts": facts,
            })),
            _ => None,
        })
        .await
        .map_err(|_| LinkError::Timeout("device did not answer read".into()))
    }

    /// 緊急停止：立即送 stop-all（不等 ack 也要送出；等到 ack 更好）。
    pub async fn stop_all(&self, timeout: Duration) -> Result<(), LinkError> {
        // estop 路徑刻意不做完整握手：能送就送（配對過的連線本來就 ready；
        // 沒 ready 的連線 ensure_open 失敗就誠實回報）。
        self.raw.ensure_open().await?;
        let mut rx = self.raw.subscribe();
        self.raw.send(encode_host_msg(&HostMsg::StopAll)).await?;
        let _ = wait_for(&mut rx, timeout, |m| match m {
            DeviceMsg::Ack {
                stop_all: Some(true),
                ..
            } => Some(()),
            _ => None,
        })
        .await;
        Ok(())
    }
}

/// 在 broadcast stream 上等第一個符合條件的訊息（有界時間）。
pub async fn wait_for<T, F>(
    rx: &mut broadcast::Receiver<DeviceMsg>,
    timeout: Duration,
    mut pick: F,
) -> Result<T, ()>
where
    F: FnMut(&DeviceMsg) -> Option<T>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(());
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(msg)) => {
                if let Some(v) = pick(&msg) {
                    return Ok(v);
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => return Err(()),
            Err(_) => return Err(()),
        }
    }
}
