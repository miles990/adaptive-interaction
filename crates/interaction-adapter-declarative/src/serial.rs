//! USB Serial 傳輸（feature = transport-serial）。
//!
//! I/O 模型：每次連線起 reader（讀行→廣播）＋writer（佇列→寫）兩條
//! std thread；斷線＝alive=false → supervisor 以指數退避重開
//! （1s→2s→…→15s cap），世代 +1 → DeviceLink 重新 hello/pair 握手。
//! 佇列有界、無 busy-loop；drop 時以 shutdown flag 收掉。
//!
//! 開埠：優先 serialport crate（真硬體：termios＋baud）。macOS 對 pty
//! （模擬器）做 baud ioctl 會回 ENOTTY——此時誠實退回「純檔案 I/O」
//! （pty 由模擬器端設 raw）。這個 fallback 只在 ENOTTY 時啟用，
//! 真硬體路徑不受影響。
//!
//! 身分誠實：埠路徑（/dev/cu.*）不是身分——握手時 hello.deviceId 必須
//! 等於 spec.expectedDeviceId，否則拒絕（macOS 無 stable serial id 的
//! 已知限制由此補上：身分由裝置自報＋配對碼驗證，而非路徑）。

use crate::protocol::{parse_device_msg, DeviceMsg, LinkError, RawLink};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tokio::sync::broadcast;

const BROADCAST_CAP: usize = 64;
const WRITE_QUEUE_CAP: usize = 32;
const BACKOFF_START_MS: u64 = 1_000;
const BACKOFF_MAX_MS: u64 = 15_000;

enum PortHalves {
    Serial(Box<dyn serialport::SerialPort>),
    File(std::fs::File),
}

fn open_port(port: &str, baud: u32) -> Result<PortHalves, String> {
    match serialport::new(port, baud)
        .timeout(Duration::from_millis(200))
        .open()
    {
        Ok(handle) => Ok(PortHalves::Serial(handle)),
        Err(e) => {
            // macOS pty（模擬器）：baud ioctl 回 ENOTTY → 純檔案 fallback。
            let is_enotty = e.to_string().contains("typewriter")
                || matches!(e.kind, serialport::ErrorKind::Io(std::io::ErrorKind::Other));
            if is_enotty {
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(port)
                    .map(PortHalves::File)
                    .map_err(|fe| format!("serialport: {e}; file fallback: {fe}"))
            } else {
                Err(e.to_string())
            }
        }
    }
}

pub struct SerialRawLink {
    port: String,
    baud: u32,
    tx_out: mpsc::SyncSender<String>,
    inbound: broadcast::Sender<DeviceMsg>,
    connected: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    /// 連線世代：每次重開 +1，DeviceLink 靠它偵測重連並重新握手。
    generation: Arc<AtomicU64>,
}

impl SerialRawLink {
    /// 建立並啟動 supervisor 執行緒。
    pub fn spawn(port: String, baud: u32) -> Arc<Self> {
        let (tx_out, rx_out) = mpsc::sync_channel::<String>(WRITE_QUEUE_CAP);
        let (inbound, _) = broadcast::channel(BROADCAST_CAP);
        let link = Arc::new(Self {
            port: port.clone(),
            baud,
            tx_out,
            inbound: inbound.clone(),
            connected: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
        });
        let connected = link.connected.clone();
        let shutdown = link.shutdown.clone();
        let generation = link.generation.clone();
        std::thread::Builder::new()
            .name(format!("serial-sup-{port}"))
            .spawn(move || supervisor(port, baud, rx_out, inbound, connected, shutdown, generation))
            .ok();
        link
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

fn supervisor(
    port: String,
    baud: u32,
    rx_out: mpsc::Receiver<String>,
    inbound: broadcast::Sender<DeviceMsg>,
    connected: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
) {
    let mut backoff = BACKOFF_START_MS;
    while !shutdown.load(Ordering::SeqCst) {
        match open_port(&port, baud) {
            Ok(halves) => {
                backoff = BACKOFF_START_MS;
                let (reader, mut writer): (Box<dyn Read + Send>, Box<dyn Write + Send>) =
                    match halves {
                        PortHalves::Serial(handle) => match handle.try_clone() {
                            Ok(r) => (Box::new(r), Box::new(handle)),
                            Err(e) => {
                                tracing::warn!(port = %port, error = %e, "serial clone failed");
                                interruptible_sleep(&shutdown, backoff);
                                continue;
                            }
                        },
                        PortHalves::File(file) => match file.try_clone() {
                            Ok(r) => (Box::new(r), Box::new(file)),
                            Err(e) => {
                                tracing::warn!(port = %port, error = %e, "pty clone failed");
                                interruptible_sleep(&shutdown, backoff);
                                continue;
                            }
                        },
                    };
                generation.fetch_add(1, Ordering::SeqCst);
                connected.store(true, Ordering::SeqCst);
                let alive = Arc::new(AtomicBool::new(true));

                // Reader thread：讀行→廣播；EOF/錯誤＝連線死。
                let reader_alive = alive.clone();
                let reader_shutdown = shutdown.clone();
                let reader_inbound = inbound.clone();
                let reader_handle = std::thread::Builder::new()
                    .name(format!("serial-read-{port}"))
                    .spawn(move || {
                        let mut buf = BufReader::new(reader);
                        let mut line = String::new();
                        loop {
                            if !reader_alive.load(Ordering::SeqCst)
                                || reader_shutdown.load(Ordering::SeqCst)
                            {
                                return;
                            }
                            line.clear();
                            match buf.read_line(&mut line) {
                                Ok(0) => break, // EOF＝裝置拔線
                                Ok(_) => {
                                    if let Some(msg) = parse_device_msg(&line) {
                                        let _ = reader_inbound.send(msg);
                                    }
                                }
                                Err(e)
                                    if matches!(
                                        e.kind(),
                                        std::io::ErrorKind::TimedOut
                                            | std::io::ErrorKind::Interrupted
                                            | std::io::ErrorKind::WouldBlock
                                    ) => {}
                                Err(_) => break,
                            }
                        }
                        reader_alive.store(false, Ordering::SeqCst);
                    })
                    .ok();

                // Writer（supervisor 本身）：佇列→寫；錯誤＝連線死。
                while alive.load(Ordering::SeqCst) && !shutdown.load(Ordering::SeqCst) {
                    match rx_out.recv_timeout(Duration::from_millis(200)) {
                        Ok(msg) => {
                            if writer
                                .write_all(format!("{msg}\n").as_bytes())
                                .and_then(|_| writer.flush())
                                .is_err()
                            {
                                alive.store(false, Ordering::SeqCst);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            alive.store(false, Ordering::SeqCst);
                        }
                    }
                }
                alive.store(false, Ordering::SeqCst);
                connected.store(false, Ordering::SeqCst);
                drop(writer); // 關掉 fd，讓 reader 的 blocking read 解除
                if let Some(handle) = reader_handle {
                    let _ = handle.join();
                }
            }
            Err(e) => {
                tracing::debug!(port = %port, error = %e, "serial open failed; backing off");
                connected.store(false, Ordering::SeqCst);
                interruptible_sleep(&shutdown, backoff);
                backoff = (backoff * 2).min(BACKOFF_MAX_MS);
            }
        }
    }
}

fn interruptible_sleep(shutdown: &AtomicBool, total_ms: u64) {
    let mut waited = 0;
    while waited < total_ms && !shutdown.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(100));
        waited += 100;
    }
}

#[async_trait::async_trait]
impl RawLink for SerialRawLink {
    async fn ensure_open(&self) -> Result<(), LinkError> {
        // 首次/重連中：有界等待（最長 2 秒），等不到就誠實回報。
        for _ in 0..20 {
            if self.connected.load(Ordering::SeqCst) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(LinkError::Unavailable(format!(
            "serial port {} is not open (device unplugged or busy; reconnect keeps retrying with backoff)",
            self.port
        )))
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        self.tx_out.try_send(line).map_err(|_| {
            LinkError::Unavailable(format!(
                "serial write queue full or closed for {}",
                self.port
            ))
        })
    }

    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        self.inbound.subscribe()
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn describe(&self) -> String {
        format!("serial {}@{}", self.port, self.baud)
    }
}

impl Drop for SerialRawLink {
    fn drop(&mut self) {
        self.shutdown();
    }
}
