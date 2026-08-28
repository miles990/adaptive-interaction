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

use crate::protocol::{parse_device_msg, DeviceMsg, LinkError, LinkState, RawLink};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

const BROADCAST_CAP: usize = 64;
const WRITE_QUEUE_CAP: usize = 32;
const BACKOFF_START_MS: u64 = 1_000;
const BACKOFF_MAX_MS: u64 = 15_000;
/// shutdown 時等 reader 執行緒收尾的上限。pty fallback 的 read 沒有逾時，
/// 裝置完全沉默時可能還卡在 read——關閉必須有界，不能無限 join。
const READER_JOIN_GRACE_MS: u64 = 500;

// link_state 的原子編碼（AtomicU8）。
const STATE_CONNECTING: u8 = 0;
const STATE_CONNECTED: u8 = 1;
const STATE_DISCONNECTED: u8 = 2;
const STATE_CLOSED: u8 = 3;

enum PortHalves {
    Serial(Box<dyn serialport::SerialPort>),
    File(std::fs::File),
}

/// 這個 serialport 錯誤是不是 ENOTTY（「這個 fd 不是 tty／不吃 termios
/// ioctl」）？只有這一種錯誤才可以退回純檔案 I/O。
///
/// 實測字串（macOS 26.2 / serialport 4.10，2026-08 由本檔的
/// `pty_link_opens_and_shutdown_stops_the_supervisor` 與
/// `a_regular_file_reports_enotty_and_takes_the_file_fallback` 印出）：
/// - `pty.openpty()` 的 slave（/dev/ttysNNN，CLI E2E 模擬器就是它）：
///   `kind=Unknown`、`description="Not a typewriter"` → 走 fallback。
/// - 非 tty 的一般檔案：同樣是 `kind=Unknown`、`"Not a typewriter"`。
/// - Linux 的 strerror(ENOTTY) 通常印成 `"Inappropriate ioctl for device"`，
///   一併認得（兩種寫法都是 ENOTTY）。
///
/// 注意：實測 kind 是 `Unknown` 而不是 `Io(Other)`——舊版靠 `Io(Other)`
/// 那條分支根本不是模擬器走通的原因，卻會把權限／忙碌等真錯誤一起吞掉。
///
/// 舊版本把「任何 `Io(Other)`」都當 ENOTTY（權限不足、忙碌中、I/O 錯誤
/// 都被吞掉退成檔案 I/O，等於對真硬體悄悄降級）——現在只認 ENOTTY 字樣。
pub(crate) fn is_enotty(e: &serialport::Error) -> bool {
    let text = e.to_string().to_ascii_lowercase();
    text.contains("enotty")
        || text.contains("inappropriate ioctl")
        || text.contains("not a typewriter")
        || text.contains("not a tty")
}

fn open_port(port: &str, baud: u32) -> Result<PortHalves, String> {
    match serialport::new(port, baud)
        .timeout(Duration::from_millis(200))
        .open()
    {
        Ok(handle) => Ok(PortHalves::Serial(handle)),
        Err(e) => {
            // ENOTTY（例：某些平台的 pty／非 tty 節點不吃 baud ioctl）
            // → 誠實退回純檔案 I/O。其他錯誤（權限、忙碌、不存在）
            // 一律原樣回報，不得悄悄降級。
            if is_enotty(&e) {
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

/// 待送出的一則訊息＋它的截止時間（過期不送：斷線期間排隊、重連後才
/// 寫出的命令會製造遲到的實體效果）。
type Outgoing = (String, Option<Instant>);

pub struct SerialRawLink {
    port: String,
    baud: u32,
    tx_out: mpsc::SyncSender<Outgoing>,
    inbound: broadcast::Sender<DeviceMsg>,
    connected: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    /// 連線世代：每次重開 +1，DeviceLink 靠它偵測重連並重新握手。
    generation: Arc<AtomicU64>,
    /// 細緻狀態（health 用）：連線中／已連線／連不上／已關閉。
    state: Arc<AtomicU8>,
    /// supervisor 執行緒是否已收尾（shutdown 測試用）。
    supervisor_done: Arc<AtomicBool>,
}

impl SerialRawLink {
    /// 建立並啟動 supervisor 執行緒。
    pub fn spawn(port: String, baud: u32) -> Arc<Self> {
        let (tx_out, rx_out) = mpsc::sync_channel::<Outgoing>(WRITE_QUEUE_CAP);
        let (inbound, _) = broadcast::channel(BROADCAST_CAP);
        let link = Arc::new(Self {
            port: port.clone(),
            baud,
            tx_out,
            inbound: inbound.clone(),
            connected: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            state: Arc::new(AtomicU8::new(STATE_CONNECTING)),
            supervisor_done: Arc::new(AtomicBool::new(false)),
        });
        let ctx = SupervisorCtx {
            connected: link.connected.clone(),
            shutdown: link.shutdown.clone(),
            generation: link.generation.clone(),
            state: link.state.clone(),
            done: link.supervisor_done.clone(),
        };
        if std::thread::Builder::new()
            .name(format!("serial-sup-{port}"))
            .spawn(move || supervisor(port, baud, rx_out, inbound, ctx))
            .is_err()
        {
            // 起不了執行緒＝這條連線根本沒有 supervisor：誠實標成連不上，
            // 不能讓 health 以為還在「連線中」。
            link.state.store(STATE_DISCONNECTED, Ordering::SeqCst);
            link.supervisor_done.store(true, Ordering::SeqCst);
        }
        link
    }

    /// supervisor 執行緒是否已結束（shutdown 的回收驗證用）。
    pub fn supervisor_finished(&self) -> bool {
        self.supervisor_done.load(Ordering::SeqCst)
    }

    /// 排入寫佇列（有界；滿了＝確定沒送出，呼叫端可安全重試）。
    fn enqueue(&self, line: String, deadline: Option<Instant>) -> Result<(), LinkError> {
        self.tx_out.try_send((line, deadline)).map_err(|_| {
            LinkError::Unavailable(format!(
                "serial write queue full or closed for {}; nothing was written",
                self.port
            ))
        })
    }

    fn close(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.connected.store(false, Ordering::SeqCst);
        self.state.store(STATE_CLOSED, Ordering::SeqCst);
    }
}

struct SupervisorCtx {
    connected: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    state: Arc<AtomicU8>,
    done: Arc<AtomicBool>,
}

fn supervisor(
    port: String,
    baud: u32,
    rx_out: mpsc::Receiver<Outgoing>,
    inbound: broadcast::Sender<DeviceMsg>,
    ctx: SupervisorCtx,
) {
    let SupervisorCtx {
        connected,
        shutdown,
        generation,
        state,
        done,
    } = ctx;
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
                // 新世代＝新連線：斷線期間堆在佇列裡的舊命令一律丟棄。
                // 它們屬於上一條連線、也還沒重新 hello/pair——重連後照原樣
                // 送出等於在握手前觸發遲到的實體效果。
                let dropped = drain_stale_queue(&rx_out);
                if dropped > 0 {
                    tracing::warn!(
                        port = %port,
                        dropped,
                        "serial reconnect: dropped queued commands from the previous link generation"
                    );
                }
                generation.fetch_add(1, Ordering::SeqCst);
                connected.store(true, Ordering::SeqCst);
                state.store(STATE_CONNECTED, Ordering::SeqCst);
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
                        Ok((msg, deadline)) => {
                            // 過期的命令不送：遲到的實體效果比誠實失敗更糟。
                            if !still_in_time(deadline, Instant::now()) {
                                tracing::warn!(
                                    port = %port,
                                    "serial: queued command expired before it could be written; NOT sent"
                                );
                                continue;
                            }
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
                if !shutdown.load(Ordering::SeqCst) {
                    state.store(STATE_CONNECTING, Ordering::SeqCst);
                }
                drop(writer); // 關掉 fd，讓 reader 的 blocking read 解除
                if let Some(handle) = reader_handle {
                    // 有界 join：pty fallback 的 read 沒有逾時，裝置沉默時
                    // reader 可能還卡著——關閉不能無限等，逾時就放生
                    // （reader 只寫 broadcast，下一次讀到資料/EOF 就會看到
                    // alive=false 而自行結束）。
                    let deadline =
                        std::time::Instant::now() + Duration::from_millis(READER_JOIN_GRACE_MS);
                    while !handle.is_finished() && std::time::Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    if handle.is_finished() {
                        let _ = handle.join();
                    } else {
                        tracing::debug!(
                            port = %port,
                            "serial reader thread still blocked on read; detaching it"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::debug!(port = %port, error = %e, "serial open failed; backing off");
                connected.store(false, Ordering::SeqCst);
                if !shutdown.load(Ordering::SeqCst) {
                    state.store(STATE_DISCONNECTED, Ordering::SeqCst);
                }
                interruptible_sleep(&shutdown, backoff);
                backoff = (backoff * 2).min(BACKOFF_MAX_MS);
            }
        }
    }
    connected.store(false, Ordering::SeqCst);
    state.store(STATE_CLOSED, Ordering::SeqCst);
    done.store(true, Ordering::SeqCst);
}

/// 重連（新世代）時把待送佇列清空，回傳丟棄的則數。
/// 斷線期間排進來的命令屬於上一條連線，重連後不得原樣送出。
fn drain_stale_queue(rx: &mpsc::Receiver<Outgoing>) -> usize {
    let mut dropped = 0usize;
    while rx.try_recv().is_ok() {
        dropped += 1;
    }
    dropped
}

/// 這一則是否還在期限內（`None`＝沒有期限的控制訊息，例如 who/stop-all）。
fn still_in_time(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.map(|d| now < d).unwrap_or(true)
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
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable(format!(
                "serial port {} was closed by the host (provider disabled/revoked); \
                 re-enabling requires reloading the adapter spec",
                self.port
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
            "serial port {} is not open (device unplugged or busy; reconnect keeps retrying with backoff)",
            self.port
        )))
    }

    async fn send(&self, line: String) -> Result<(), LinkError> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable(format!(
                "serial link {} is closed; nothing was sent",
                self.port
            )));
        }
        self.enqueue(line, None)
    }

    async fn send_before(&self, line: String, deadline: Instant) -> Result<(), LinkError> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable(format!(
                "serial link {} is closed; nothing was sent",
                self.port
            )));
        }
        if Instant::now() >= deadline {
            return Err(LinkError::Unavailable(
                "deadline passed before the message could be queued; nothing was written".into(),
            ));
        }
        self.enqueue(line, Some(deadline))
    }

    fn subscribe(&self) -> broadcast::Receiver<DeviceMsg> {
        self.inbound.subscribe()
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst) && !self.shutdown.load(Ordering::SeqCst)
    }

    fn link_state(&self) -> LinkState {
        match self.state.load(Ordering::SeqCst) {
            STATE_CONNECTED if self.connected() => LinkState::Connected,
            STATE_CONNECTING => LinkState::Connecting,
            STATE_CLOSED => LinkState::Closed,
            _ => LinkState::Disconnected,
        }
    }

    fn shutdown(&self) {
        // 停旗標＋讓 supervisor 走完收尾（drop writer → reader 解除）。
        // supervisor 最多 200ms（recv_timeout）就會看到旗標。
        self.close();
    }

    fn describe(&self) -> String {
        format!("serial {}@{}", self.port, self.baud)
    }
}

impl Drop for SerialRawLink {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::{Error as SpError, ErrorKind as SpKind};

    /// 重連（世代改變）必須清空待送佇列：斷線期間排進來的 cmd 屬於上一條
    /// 連線，重連後（握手前）原樣送出就是遲到的實體效果。
    #[test]
    fn a_reconnect_drops_commands_queued_on_the_previous_link() {
        let (tx, rx) = mpsc::sync_channel::<Outgoing>(WRITE_QUEUE_CAP);
        tx.send((r#"{"type":"cmd","id":"a1"}"#.into(), None))
            .unwrap();
        tx.send((r#"{"type":"cmd","id":"a2"}"#.into(), None))
            .unwrap();
        assert_eq!(drain_stale_queue(&rx), 2, "舊連線的命令必須被丟棄");
        assert_eq!(drain_stale_queue(&rx), 0, "清過就沒有東西可丟（冪等）");
        // 清空後新命令照走。
        tx.send((r#"{"type":"cmd","id":"a3"}"#.into(), None))
            .unwrap();
        assert_eq!(
            rx.try_recv().map(|(line, _)| line).unwrap(),
            r#"{"type":"cmd","id":"a3"}"#
        );
    }

    /// 每則 cmd 帶 deadline：過期的不寫上線（沒有期限的控制訊息照送）。
    #[test]
    fn an_expired_queue_entry_is_not_written() {
        let now = Instant::now();
        assert!(still_in_time(None, now), "沒有期限的控制訊息照送");
        assert!(still_in_time(Some(now + Duration::from_millis(50)), now));
        assert!(!still_in_time(Some(now), now), "剛好到期就不送");
        assert!(!still_in_time(Some(now - Duration::from_millis(1)), now));
    }

    /// 只有 ENOTTY 才可以退回純檔案 I/O——權限不足／忙碌／找不到裝置
    /// 都必須原樣回報（舊版把任何 `Io(Other)` 都當 ENOTTY，等於對真硬體
    /// 悄悄降級成沒有 termios 設定的檔案 I/O）。
    #[test]
    fn enotty_fallback_is_narrow() {
        // macOS / Linux 的 strerror(ENOTTY)。
        assert!(is_enotty(&SpError::new(
            SpKind::Io(std::io::ErrorKind::Other),
            "Inappropriate ioctl for device"
        )));
        assert!(is_enotty(&SpError::new(
            SpKind::Io(std::io::ErrorKind::Other),
            "Not a typewriter"
        )));
        assert!(is_enotty(&SpError::new(SpKind::Unknown, "ENOTTY (25)")));
        // 這些以前會被誤判成 ENOTTY → 靜默降級。現在必須原樣回報。
        assert!(!is_enotty(&SpError::new(
            SpKind::Io(std::io::ErrorKind::Other),
            "Permission denied"
        )));
        assert!(!is_enotty(&SpError::new(
            SpKind::Io(std::io::ErrorKind::Other),
            "Device or resource busy"
        )));
        assert!(!is_enotty(&SpError::new(
            SpKind::NoDevice,
            "No such file or directory"
        )));
        assert!(!is_enotty(&SpError::new(
            SpKind::Io(std::io::ErrorKind::PermissionDenied),
            "Permission denied (os error 13)"
        )));
    }

    /// 一般檔案不是 tty：termios ioctl 會回 ENOTTY。用它固定「真實
    /// strerror 字串」這個事實（測試註解裡記的就是這一行印出來的東西）。
    #[test]
    fn a_regular_file_reports_enotty_and_takes_the_file_fallback() {
        let path = std::env::temp_dir().join(format!("serial-enotty-{}.probe", std::process::id()));
        std::fs::write(&path, b"").expect("probe file");
        let path_str = path.to_string_lossy().to_string();
        match serialport::new(&path_str, 115_200)
            .timeout(Duration::from_millis(200))
            .open()
        {
            Ok(_) => {
                // 平台居然接受了：那就不需要 fallback，也不算 ENOTTY。
                eprintln!("serialport opened a regular file (no ioctl error)");
            }
            Err(e) => {
                // 實測（macOS 26.2 / serialport 4.10）：
                //   kind=Unknown description="Not a typewriter"
                eprintln!("regular-file open error: kind={:?} description={e}", e.kind);
                assert!(is_enotty(&e), "ENOTTY 應被辨識：{e}");
                // 走 fallback：純檔案 I/O 開得起來。
                assert!(matches!(
                    open_port(&path_str, 115_200),
                    Ok(PortHalves::File(_))
                ));
            }
        }
        let _ = std::fs::remove_file(&path);
    }

    /// pty（CLI E2E 的模擬器就是這樣跑的）必須開得起來——不管走的是
    /// serialport 正常路徑還是 ENOTTY fallback，兩者都算通過。
    /// 同時驗證 shutdown()：supervisor 執行緒要在 2 秒內收工。
    #[tokio::test(flavor = "multi_thread")]
    async fn pty_link_opens_and_shutdown_stops_the_supervisor() {
        let Some(mut pty) = Pty::spawn() else {
            eprintln!("python3 unavailable; skipping pty test");
            return;
        };
        // 記錄實際行為（真硬體與 pty 的差別就靠這裡看見）。
        match serialport::new(&pty.path, 115_200)
            .timeout(Duration::from_millis(200))
            .open()
        {
            Ok(_) => eprintln!("pty {}: serialport open OK (no ENOTTY)", pty.path),
            Err(e) => eprintln!(
                "pty {}: serialport open failed kind={:?} description={e} → is_enotty={}",
                pty.path,
                e.kind,
                is_enotty(&e)
            ),
        }
        assert!(
            open_port(&pty.path, 115_200).is_ok(),
            "pty 必須開得起來（serialport 或 ENOTTY fallback）"
        );

        let link = SerialRawLink::spawn(pty.path.clone(), 115_200);
        // 有界等待連上。
        for _ in 0..40 {
            if link.connected() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(link.connected(), "pty 應該連得上");
        assert_eq!(link.link_state(), LinkState::Connected);

        link.shutdown();
        assert!(!link.connected(), "shutdown 後不得再回報連線中");
        assert_eq!(link.link_state(), LinkState::Closed);
        assert!(
            RawLink::send(&*link, "{\"type\":\"who\"}".into())
                .await
                .is_err(),
            "關閉後 send 必須誠實失敗，不得默默排隊"
        );
        // supervisor 執行緒 2 秒內結束（不得留著無盡重連）。
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !link.supervisor_finished() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            link.supervisor_finished(),
            "shutdown 後 supervisor 執行緒必須在 2 秒內收工"
        );
        pty.kill();
    }

    /// 借 python 開一個 pty，把 slave 路徑交出來（master 端保持開啟）。
    struct Pty {
        child: std::process::Child,
        path: String,
    }

    impl Pty {
        fn spawn() -> Option<Self> {
            use std::io::BufRead;
            let mut child = std::process::Command::new("python3")
                .arg("-c")
                .arg(
                    "import pty,os,sys,time,termios,tty\n\
                     m,s=pty.openpty()\n\
                     tty.setraw(m, when=termios.TCSANOW)\n\
                     tty.setraw(s, when=termios.TCSANOW)\n\
                     print(os.ttyname(s)); sys.stdout.flush()\n\
                     time.sleep(30)\n",
                )
                .stdout(std::process::Stdio::piped())
                .spawn()
                .ok()?;
            let stdout = child.stdout.take()?;
            let mut line = String::new();
            let mut reader = std::io::BufReader::new(stdout);
            if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                let _ = child.kill();
                return None;
            }
            Some(Self {
                child,
                path: line.trim().to_string(),
            })
        }

        fn kill(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    impl Drop for Pty {
        fn drop(&mut self) {
            self.kill();
        }
    }
}
