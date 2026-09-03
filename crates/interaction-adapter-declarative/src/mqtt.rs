//! MQTT 傳輸（feature = transport-mqtt）。
//!
//! 每個 adapter 一條 rumqttc 連線：eventloop task 收 `<prefix>/from-device`
//! → 廣播；host 訊息發佈到 `<prefix>/to-device`（QoS 1，at-least-once——
//! 重複由兩端 dedupe：裝置 dedupe cmd id、host 端 ack 等待者以 id 對應）。
//! 斷線：整組 client＋eventloop 換新的再退避重連（1s→…→15s cap），世代 +1 →
//! DeviceLink 重新 hello/pair。**絕不重播**上一代未 ack 的 publish：rumqttc
//! 預設會把它們搬進 pending 並在重連後補送，對實體命令那是「遲到的實體效果」
//! ——host 早已把它們記成結果未知、不重送。broker 不是身分：身分仍靠
//! hello.deviceId＋配對碼；broker 連著也不代表裝置活著（見 last_heard）。

use crate::protocol::{parse_device_msg, DeviceMsg, LinkError, LinkState, RawLink, TaskSlot};
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

const BROADCAST_CAP: usize = 64;
const BACKOFF_START_MS: u64 = 1_000;
const BACKOFF_MAX_MS: u64 = 15_000;
/// rumqttc 的請求 channel 容量（每次重連換一組新的）。
const REQUEST_CHANNEL_CAP: usize = 16;
/// 多久沒聽到裝置就算失聯。參考韌體每 5 秒對已配對通道推播一次 state，
/// 取 3 倍當窗（一次掉包不算離線，連掉三次就不能再說「此刻能用它」）。
pub const DEFAULT_LIVENESS_TIMEOUT_MS: u64 = 15_000;
/// 參考韌體 mqttCallback 的單則上限：`length >= 640` 即回一則**沒有 id** 的
/// `err bad-json`（與 serial 的 639 bytes 相同）。超過的訊息在 host 端就
/// 拒絕——確定沒發佈，不製造「裝置回了無 id 錯誤」的未知。
pub const MAX_MESSAGE_BYTES: usize = 639;

// link_state 的原子編碼（與 serial 對齊）。
const STATE_CONNECTING: u8 = 0;
const STATE_CONNECTED: u8 = 1;
const STATE_DISCONNECTED: u8 = 2;
const STATE_CLOSED: u8 = 3;

/// 現在的 unix 毫秒（時鐘倒退時退回 0＝「不知道」，不假裝知道）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// panic 過的鎖仍要能用（收尾路徑不可因 poison 而 panic）。
fn client_of(slot: &std::sync::Mutex<AsyncClient>) -> AsyncClient {
    match slot.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn replace_client(slot: &std::sync::Mutex<AsyncClient>, fresh: AsyncClient) {
    match slot.lock() {
        Ok(mut guard) => *guard = fresh,
        Err(poisoned) => *poisoned.into_inner() = fresh,
    }
}

pub struct MqttRawLink {
    /// 這一代連線的 client handle。斷線時整組 client＋eventloop 會被換掉
    /// （舊的連同它未送出／未 ack 的佇列一起丟棄），所以這裡是可換的槽。
    client: Arc<std::sync::Mutex<AsyncClient>>,
    topic_to_device: String,
    inbound: broadcast::Sender<DeviceMsg>,
    connected: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    state: Arc<AtomicU8>,
    /// 已被 shutdown()：不再重連、不再送出任何東西。
    closed: Arc<AtomicBool>,
    /// 通知 eventloop task 收工（notify_one 會留下 permit，
    /// 即使 task 還沒進到 select 也不會漏掉）。
    stop: Arc<tokio::sync::Notify>,
    /// eventloop task（shutdown 時 abort，避免殭屍 task 繼續重連）。
    task: TaskSlot,
    describe: String,
    /// 最後一次聽到**裝置**訊息的時刻（unix 毫秒）。broker 連著不等於
    /// 裝置活著——健康度必須看這個。
    last_heard: Arc<AtomicU64>,
    /// 超過這麼久沒聽到裝置就算失聯（degraded）。
    liveness_window: Duration,
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
        liveness_timeout_ms: Option<u64>,
    ) -> Arc<Self> {
        let client_id = format!("interact-ai-{client_id_suffix}");
        let mut options = MqttOptions::new(client_id, host.clone(), port);
        options.set_keep_alive(Duration::from_secs(15));
        if let Some((user, pass)) = credentials {
            options.set_credentials(user, pass);
        }
        let (client, mut eventloop) = AsyncClient::new(options.clone(), REQUEST_CHANNEL_CAP);
        let (inbound, _) = broadcast::channel(BROADCAST_CAP);
        let connected = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let state = Arc::new(AtomicU8::new(STATE_CONNECTING));
        let closed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(tokio::sync::Notify::new());
        let last_heard = Arc::new(AtomicU64::new(now_ms()));
        let client_slot = Arc::new(std::sync::Mutex::new(client));
        let topic_from = format!("{topic_prefix}/from-device");
        let link = Arc::new(Self {
            client: client_slot.clone(),
            topic_to_device: format!("{topic_prefix}/to-device"),
            inbound: inbound.clone(),
            connected: connected.clone(),
            generation: generation.clone(),
            state: state.clone(),
            closed: closed.clone(),
            stop: stop.clone(),
            task: TaskSlot::new(),
            describe: format!("mqtt {host}:{port} prefix {topic_prefix}"),
            last_heard: last_heard.clone(),
            liveness_window: Duration::from_millis(
                liveness_timeout_ms
                    .unwrap_or(DEFAULT_LIVENESS_TIMEOUT_MS)
                    .clamp(200, 3_600_000),
            ),
        });
        let handle = tokio::spawn(async move {
            let mut backoff = BACKOFF_START_MS;
            loop {
                let event = tokio::select! {
                    // 主動關閉：送出 disconnect 後結束 task（不再重連）。
                    _ = stop.notified() => {
                        let _ = client_of(&client_slot).disconnect().await;
                        break;
                    }
                    event = eventloop.poll() => event,
                };
                match event {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        backoff = BACKOFF_START_MS;
                        generation.fetch_add(1, Ordering::SeqCst);
                        connected.store(true, Ordering::SeqCst);
                        state.store(STATE_CONNECTED, Ordering::SeqCst);
                        let _ = client_of(&client_slot)
                            .subscribe(&topic_from, QoS::AtLeastOnce)
                            .await;
                    }
                    Ok(Event::Incoming(Packet::Publish(publish))) => {
                        if publish.topic == topic_from {
                            // 聽到裝置了——即使這則解析不出來，也證明它還活著。
                            last_heard.store(now_ms(), Ordering::SeqCst);
                            if let Ok(text) = std::str::from_utf8(&publish.payload) {
                                if let Some(msg) = parse_device_msg(text) {
                                    let _ = inbound.send(msg);
                                }
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        // 連線斷：誠實標記＋退避重連。
                        if connected.swap(false, Ordering::SeqCst) {
                            tracing::debug!(
                                error = %e,
                                "mqtt disconnected; dropping the previous session's outgoing \
                                 queue and backing off"
                            );
                        }
                        state.store(STATE_DISCONNECTED, Ordering::SeqCst);
                        // 重點：整組 client＋eventloop 換新的。rumqttc 會把
                        // 「已送出但未收到 PubAck 的 QoS1 publish」與「還沒被
                        // eventloop 取走的請求」搬進 pending，並在重連後優先
                        // 重播——對實體命令那是**遲到的實體效果**：host 端早
                        // 已把它們記成「結果未知、不重送」，裝置卻在數秒後才
                        // 動作。舊的 client 一起丟掉，任何拿著舊 handle 的
                        // publish 會立刻失敗（誠實回「沒送出」），
                        // ConnAck 分支會在新連線上重新 subscribe。
                        let (fresh_client, fresh_loop) =
                            AsyncClient::new(options.clone(), REQUEST_CHANNEL_CAP);
                        eventloop = fresh_loop;
                        replace_client(&client_slot, fresh_client);
                        tokio::select! {
                            _ = stop.notified() => break,
                            _ = tokio::time::sleep(Duration::from_millis(backoff)) => {}
                        }
                        backoff = (backoff * 2).min(BACKOFF_MAX_MS);
                    }
                }
            }
            connected.store(false, Ordering::SeqCst);
            state.store(STATE_CLOSED, Ordering::SeqCst);
        });
        link.task.replace(handle);
        link
    }

    /// 目前這一代的 client handle（複本；鎖只在取用的瞬間持有）。
    fn current_client(&self) -> AsyncClient {
        client_of(&self.client)
    }

    /// eventloop task 是否仍在跑（shutdown 的回收驗證用）。
    pub fn eventloop_active(&self) -> bool {
        self.task.is_active()
    }
}

#[async_trait::async_trait]
impl RawLink for MqttRawLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable(format!(
                "mqtt link {} was closed by the host (provider disabled/revoked); \
                 re-enabling requires reloading the adapter spec",
                self.describe
            )));
        }
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
        if self.closed.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable(format!(
                "mqtt link {} is closed; nothing was published",
                self.describe
            )));
        }
        // 超過韌體單則上限：確定不送（Refused）。detail 以 "message too large"
        // 開頭——link_caps 靠這個慣例把收據原因記成 message-too-large。
        if line.len() > MAX_MESSAGE_BYTES {
            return Err(LinkError::Refused(format!(
                "message too large ({} bytes > {MAX_MESSAGE_BYTES}, the firmware's per-message \
                 limit); nothing was published",
                line.len()
            )));
        }
        // broker 沒連上時不得把命令塞進 client 的內部佇列：那些訊息會在
        // 重連後（hello/pair 握手前）才送達裝置，變成遲到的實體效果。
        if !self.connected.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable(format!(
                "mqtt broker not connected ({}); nothing was published",
                self.describe
            )));
        }
        // QoS 1（at-least-once）：實體命令寧可重複送達由裝置端 dedupe，
        // 也不接受「悄悄丟掉」。斷線後 client 會被整組換掉，所以拿到舊
        // handle 的 publish 會直接失敗（＝確定沒送出），不會被留著重播。
        self.current_client()
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

    fn connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst) && !self.closed.load(Ordering::SeqCst)
    }

    /// broker 連著 ≠ 裝置活著：超過存活窗沒聽到裝置就誠實回報沉默時間。
    fn device_silent_for(&self) -> Option<Duration> {
        let last = self.last_heard.load(Ordering::SeqCst);
        if last == 0 {
            return None; // 時鐘不可用：不知道就不裝作知道
        }
        let silent = now_ms().saturating_sub(last);
        (silent > self.liveness_window.as_millis() as u64).then(|| Duration::from_millis(silent))
    }

    fn link_state(&self) -> LinkState {
        if self.closed.load(Ordering::SeqCst) {
            return LinkState::Closed;
        }
        match self.state.load(Ordering::SeqCst) {
            STATE_CONNECTED if self.connected() => LinkState::Connected,
            STATE_CONNECTING => LinkState::Connecting,
            STATE_CLOSED => LinkState::Closed,
            _ => LinkState::Disconnected,
        }
    }

    fn shutdown(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return; // 冪等
        }
        self.connected.store(false, Ordering::SeqCst);
        self.state.store(STATE_CLOSED, Ordering::SeqCst);
        // 先請 eventloop task 自己送 disconnect 並收工；沒有 runtime 可跑
        // （例如程式收尾）時，abort 兜底，不留殭屍 task。
        self.stop.notify_one();
        let _ = self.current_client().try_disconnect();
    }

    fn describe(&self) -> String {
        self.describe.clone()
    }
}

impl Drop for MqttRawLink {
    fn drop(&mut self) {
        // TaskSlot 的 Drop 會 abort eventloop task。
        if !self.closed.swap(true, Ordering::SeqCst) {
            self.stop.notify_one();
            let _ = self.current_client().try_disconnect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 韌體 mqttCallback 對 ≥640 bytes 的訊息回無 id 的 bad-json。host 端要在
    /// 發佈**之前**就拒絕（Refused），而且長度檢查排在「broker 有沒有連上」
    /// 之前——這是訊息本身的性質，不該被連線狀態遮住。
    #[tokio::test(flavor = "multi_thread")]
    async fn an_oversize_message_is_refused_before_publish() {
        // 連不上的 broker：這裡只驗長度守門，不需要真的 broker。
        let link = MqttRawLink::spawn("127.0.0.1".into(), 1, "t/limit".into(), "limit", None, None);
        let exact = "x".repeat(MAX_MESSAGE_BYTES);
        let over = "x".repeat(MAX_MESSAGE_BYTES + 1);
        match RawLink::send(&*link, over).await {
            Err(LinkError::Refused(detail)) => {
                assert!(detail.starts_with("message too large"), "{detail}");
                assert!(detail.contains("640 bytes"), "{detail}");
            }
            other => panic!("640 bytes must be Refused before publish, got {other:?}"),
        }
        // 剛好 639 bytes 通過長度守門；沒連上 broker 才是它失敗的原因。
        match RawLink::send(&*link, exact).await {
            Err(LinkError::Unavailable(detail)) => {
                assert!(detail.contains("not connected"), "{detail}")
            }
            other => panic!("639 bytes must pass the size gate, got {other:?}"),
        }
        link.shutdown();
    }

    /// 存活窗：剛聽到裝置＝沒有沉默；把 last_heard 往回撥超過窗＝誠實回報
    /// 沉默時間（health 會據此降級）。broker 連著不等於裝置活著。
    #[tokio::test(flavor = "multi_thread")]
    async fn silence_beyond_the_liveness_window_is_reported() {
        let link = MqttRawLink::spawn(
            "127.0.0.1".into(),
            1,
            "t/live".into(),
            "live",
            None,
            Some(300),
        );
        assert_eq!(
            RawLink::device_silent_for(&*link),
            None,
            "剛建立就聽過一次：不算沉默"
        );
        // 把「最後聽到」往回撥 1 秒（> 300ms 的窗）。
        link.last_heard
            .store(now_ms().saturating_sub(1_000), Ordering::SeqCst);
        let silent = RawLink::device_silent_for(&*link).expect("超過窗必須回報沉默");
        assert!(silent >= Duration::from_millis(900), "{silent:?}");
        link.shutdown();
    }

    /// 存活窗有下界（設 0 也不會把活著的裝置一直判成離線）。
    #[tokio::test(flavor = "multi_thread")]
    async fn the_liveness_window_is_bounded() {
        let link = MqttRawLink::spawn("127.0.0.1".into(), 1, "t/w".into(), "w", None, Some(0));
        assert_eq!(link.liveness_window, Duration::from_millis(200));
        link.shutdown();
    }
}
