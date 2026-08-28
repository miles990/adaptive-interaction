//! MQTT 傳輸（feature = transport-mqtt）。
//!
//! 每個 adapter 一條 rumqttc 連線：eventloop task 收 `<prefix>/from-device`
//! → 廣播；host 訊息發佈到 `<prefix>/to-device`（QoS 1，at-least-once——
//! 重複由兩端 dedupe：裝置 dedupe cmd id、host 端 ack 等待者以 id 對應）。
//! 斷線：rumqttc eventloop 回錯後退避重連（1s→…→15s cap），世代 +1 →
//! DeviceLink 重新 hello/pair。broker 不是身分：身分仍靠 hello.deviceId
//! ＋配對碼。

use crate::protocol::{parse_device_msg, DeviceMsg, LinkError, RawLink};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

const BROADCAST_CAP: usize = 64;
const BACKOFF_START_MS: u64 = 1_000;
const BACKOFF_MAX_MS: u64 = 15_000;

pub struct MqttRawLink {
    client: AsyncClient,
    topic_to_device: String,
    inbound: broadcast::Sender<DeviceMsg>,
    connected: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    describe: String,
}

impl MqttRawLink {
    /// 建立 client 並啟動 eventloop task。`credentials`＝(username, password)
    /// 已解析（secret:// 由呼叫端處理，這裡永不記錄）。
    pub fn spawn(
        host: String,
        port: u16,
        topic_prefix: String,
        client_id_suffix: &str,
        credentials: Option<(String, String)>,
    ) -> Arc<Self> {
        let client_id = format!("interact-ai-{client_id_suffix}");
        let mut options = MqttOptions::new(client_id, host.clone(), port);
        options.set_keep_alive(Duration::from_secs(15));
        if let Some((user, pass)) = credentials {
            options.set_credentials(user, pass);
        }
        let (client, mut eventloop) = AsyncClient::new(options, 16);
        let (inbound, _) = broadcast::channel(BROADCAST_CAP);
        let connected = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let topic_from = format!("{topic_prefix}/from-device");
        let link = Arc::new(Self {
            client: client.clone(),
            topic_to_device: format!("{topic_prefix}/to-device"),
            inbound: inbound.clone(),
            connected: connected.clone(),
            generation: generation.clone(),
            describe: format!("mqtt {host}:{port} prefix {topic_prefix}"),
        });
        tokio::spawn(async move {
            let mut backoff = BACKOFF_START_MS;
            loop {
                match eventloop.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        backoff = BACKOFF_START_MS;
                        generation.fetch_add(1, Ordering::SeqCst);
                        connected.store(true, Ordering::SeqCst);
                        let _ = client.subscribe(&topic_from, QoS::AtLeastOnce).await;
                    }
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        if publish.topic == topic_from {
                            if let Ok(text) = std::str::from_utf8(&publish.payload) {
                                if let Some(msg) = parse_device_msg(text) {
                                    let _ = inbound.send(msg);
                                }
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        // 連線斷：誠實標記＋退避重連（eventloop.poll 會自動重試，
                        // 這裡只控制節奏，避免 busy-loop）。
                        if connected.swap(false, Ordering::SeqCst) {
                            tracing::debug!(error = %e, "mqtt disconnected; backing off");
                        }
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        backoff = (backoff * 2).min(BACKOFF_MAX_MS);
                    }
                }
            }
        });
        link
    }
}

#[async_trait::async_trait]
impl RawLink for MqttRawLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        // 首次/重連中：有界等待（最長 2 秒），等不到就誠實回報。
        for _ in 0..20 {
            if self.connected.load(Ordering::SeqCst) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(LinkError::Unavailable(format!(
            "mqtt broker not connected ({}); reconnect keeps retrying with backoff",
            self.describe
        )))
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        self.client
            .publish(&self.topic_to_device, QoS::AtLeastOnce, false, line)
            .await
            .map_err(|e| LinkError::Unavailable(format!("mqtt publish failed: {e}")))
    }

    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        self.inbound.subscribe()
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn describe(&self) -> String {
        self.describe.clone()
    }
}
