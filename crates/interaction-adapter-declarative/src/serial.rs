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
/// 參考韌體的單行上限（`g_serialBuf[640]`：一行最多 639 bytes，第 640 個
/// 位元組起整行丟棄並回一則**沒有 id** 的 `err bad-json`）。超過的訊息在
/// host 端就拒絕——確定沒寫出，不製造「裝置回了無 id 錯誤」的未知。
/// 模擬器 `scripts/esp32-serial-sim.py` 的 MAX_LINE_BYTES 與此相同。
pub const MAX_LINE_BYTES: usize = 639;
/// 跨讀取逾時保留的「未完成一行」上限。裝置若一直不送換行，緩衝不得無界
/// 成長（parse_device_msg 本來就只解析 ≤16KB）。
const MAX_PARTIAL_LINE_BYTES: usize = 16 * 1024;
/// shutdown 時等 reader 執行緒收尾的上限。pty fallback 的 read 沒有逾時，
/// 裝置完全沉默時可能還卡在 read——關閉必須有界，不能無限 join。
const READER_JOIN_GRACE_MS: u64 = 500;
/// 一段連線活多久才算「真的連上過」。開得起來、但立刻 EOF 的節點
/// （一般檔案、/dev/null、peer 已關閉的 pty）不能被當成成功連線而
/// 把退避歸零——否則會變成每 ≤200ms 重連一輪的 churn。
const SESSION_MIN_ALIVE_MS: u64 = 1_000;
/// 連續幾段「立刻斷」的連線之後，健康度必須誠實落到 offline。
/// 一直在 Connecting／已連線之間跳動＝永遠不告訴使用者這台裝置不能用。
const SHORT_SESSIONS_BEFORE_OFFLINE: u32 = 2;

/// 被放生（沒 join 到）的 reader 執行緒累計數。
///
/// 已知限制：reader 與 writer 是同一個 fd 的 `dup`，POSIX 下關掉其中一個
/// **不會**讓另一個上面的 blocking read 返回。serialport 路徑有 200ms 讀取
/// 逾時所以無妨；ENOTTY 的檔案／pty fallback 沒有逾時，裝置完全沉默時
/// reader 會一直卡著，寬限期過後只能 detach。計數在這裡，測試與診斷才看得到
/// 洩漏，而不是「契約說會回收、實際上沒有」。
static DETACHED_READERS: AtomicU64 = AtomicU64::new(0);

/// 目前為止被放生的 reader 執行緒數（見 [`DETACHED_READERS`]）。
pub fn detached_reader_threads() -> u64 {
    DETACHED_READERS.load(Ordering::SeqCst)
}

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
    /// 收到但解不開的行數（亂碼、截斷、非 JSON）。靜默丟棄會把「裝置有回、
    /// 我們讀不懂」講成「裝置沒回」——等待逾時的 detail 要帶上它。
    undecodable: Arc<AtomicU64>,
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
            undecodable: Arc::new(AtomicU64::new(0)),
        });
        let ctx = SupervisorCtx {
            connected: link.connected.clone(),
            shutdown: link.shutdown.clone(),
            generation: link.generation.clone(),
            state: link.state.clone(),
            done: link.supervisor_done.clone(),
            undecodable: link.undecodable.clone(),
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
    undecodable: Arc<AtomicU64>,
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
        undecodable,
    } = ctx;
    let mut backoff = BACKOFF_START_MS;
    // 連續幾段「開得起來但立刻斷」的連線。退避只寫在「開不起來」那一邊時，
    // 這種節點會每 ≤200ms 重連一輪：thread churn、世代狂跳、排隊的命令被
    // 一直丟掉，而 health 永遠不落到 offline。
    let mut short_sessions: u32 = 0;
    while !shutdown.load(Ordering::SeqCst) {
        match open_port(&port, baud) {
            Ok(halves) => {
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
                let session_start = Instant::now();

                // Reader thread：讀行→廣播；EOF/錯誤＝連線死。
                let reader_alive = alive.clone();
                let reader_shutdown = shutdown.clone();
                let reader_inbound = inbound.clone();
                let reader_port = port.clone();
                let reader_undecodable = undecodable.clone();
                let reader_handle = std::thread::Builder::new()
                    .name(format!("serial-read-{port}"))
                    .spawn(move || {
                        pump_lines(
                            reader,
                            &reader_port,
                            &reader_alive,
                            &reader_shutdown,
                            &reader_inbound,
                            &reader_undecodable,
                        );
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
                let lived = session_start.elapsed();
                let died_immediately = lived < Duration::from_millis(SESSION_MIN_ALIVE_MS);
                if died_immediately {
                    short_sessions = short_sessions.saturating_add(1);
                } else {
                    short_sessions = 0;
                    backoff = BACKOFF_START_MS; // 真的連上過才把退避歸零
                }
                if !shutdown.load(Ordering::SeqCst) {
                    // 連續立刻斷的埠不得永遠停在「連線中」：使用者要看得出
                    // 這台裝置現在就是不能用。
                    state.store(
                        if short_sessions >= SHORT_SESSIONS_BEFORE_OFFLINE {
                            STATE_DISCONNECTED
                        } else {
                            STATE_CONNECTING
                        },
                        Ordering::SeqCst,
                    );
                }
                drop(writer); // 關掉 fd，讓 reader 的 blocking read 解除
                              // （注意：dup 過的 fd 上這不成立——見 DETACHED_READERS）
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
                        // 放生一條執行緒是真的洩漏（reader 與 writer 是同一個
                        // fd 的 dup，關掉 writer 解不開對方的 blocking read）。
                        // 計數並以 warn 記錄，才不會變成「契約說會回收、
                        // 實際上悄悄留著」。
                        let total = DETACHED_READERS.fetch_add(1, Ordering::SeqCst) + 1;
                        tracing::warn!(
                            port = %port,
                            detached_total = total,
                            "serial reader thread still blocked on read; detaching it \
                             (known limitation of the pty/file fallback: no read timeout)"
                        );
                    }
                }
                // 開得起來但立刻斷：與「開不起來」套用同一組指數退避，
                // 否則會變成無退避的重連 churn。
                if died_immediately && !shutdown.load(Ordering::SeqCst) {
                    tracing::debug!(
                        port = %port,
                        lived_ms = lived.as_millis() as u64,
                        backoff_ms = backoff,
                        short_sessions,
                        "serial link died immediately after opening; backing off"
                    );
                    interruptible_sleep(&shutdown, backoff);
                    backoff = (backoff * 2).min(BACKOFF_MAX_MS);
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

/// Reader 迴圈：讀行→解析→廣播。抽出來是為了不碰真硬體也能測「讀取逾時
/// 中間的半行」。
///
/// 關鍵：serialport 開埠帶 200ms 逾時，裝置在一行中間停頓超過 200ms 是常態
/// （韌體正在讀 DHT／超音波）。`BufRead::read_line` 在 I/O 錯誤時**會保留已
/// 讀到的位元組**，所以逾時分支絕不能清掉 `line`——清掉就等於把 ack／state
/// 的前半段丟掉，後半段解析失敗被靜默丟棄，host 端記成「ack 逾時、結果未知」。
/// 只有讀到完整一行（或超過上限）才清。
fn pump_lines<R: Read>(
    reader: R,
    port: &str,
    alive: &AtomicBool,
    shutdown: &AtomicBool,
    inbound: &broadcast::Sender<DeviceMsg>,
    undecodable: &AtomicU64,
) {
    let mut buf = BufReader::new(reader);
    let mut line = String::new();
    loop {
        if !alive.load(Ordering::SeqCst) || shutdown.load(Ordering::SeqCst) {
            return;
        }
        match buf.read_line(&mut line) {
            Ok(0) => break, // EOF＝裝置拔線
            Ok(_) => {
                match parse_device_msg(&line) {
                    Some(msg) => {
                        let _ = inbound.send(msg);
                    }
                    // 解不開的一行不得靜默消失：等待中的請求會把它講成
                    // 「裝置沒回」，人與 AI 因此去查拔線／配對，而真因是
                    // 這一行讀不懂（亂碼、被截斷、非本協定）。
                    None if !line.trim().is_empty() => {
                        undecodable.fetch_add(1, Ordering::SeqCst);
                        tracing::warn!(
                            port = %port,
                            bytes = line.trim_end().len(),
                            "serial: a line from the device could not be decoded; discarded"
                        );
                    }
                    None => {}
                }
                line.clear();
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::WouldBlock
                ) =>
            {
                // 逾時＝「還沒收到換行」，不是錯誤：保留半行等後續位元組。
                // 但要有界：永遠不送換行的裝置不得把緩衝撐大。
                if line.len() > MAX_PARTIAL_LINE_BYTES {
                    tracing::warn!(
                        port = %port,
                        bytes = line.len(),
                        "serial: unterminated line exceeded the limit; discarded"
                    );
                    line.clear();
                }
            }
            Err(_) => break,
        }
    }
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

/// 超過韌體單行上限的訊息：確定不送（Refused），而不是送出去換一則
/// 無 id 的 bad-json。detail 以 "message too large" 開頭——link_caps 靠這個
/// 慣例把收據原因記成 message-too-large（serial／mqtt／ble 三處一致）。
fn check_line_size(line: &str) -> Result<(), LinkError> {
    if line.len() > MAX_LINE_BYTES {
        return Err(LinkError::Refused(format!(
            "message too large ({} bytes > {MAX_LINE_BYTES}, the firmware's per-line limit); \
             nothing was written",
            line.len()
        )));
    }
    Ok(())
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
        check_line_size(&line)?;
        self.enqueue(line, None)
    }

    async fn send_before(&self, line: String, deadline: Instant) -> Result<(), LinkError> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(LinkError::Unavailable(format!(
                "serial link {} is closed; nothing was sent",
                self.port
            )));
        }
        check_line_size(&line)?;
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

    fn undecodable_messages(&self) -> u64 {
        self.undecodable.load(Ordering::SeqCst)
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

    /// 韌體一行最多 639 bytes（第 640 個位元組起整行丟棄、回無 id 的
    /// bad-json）。host 端必須在寫出**之前**就拒絕：確定沒送出（Refused），
    /// 而不是送出去換一則裝置端的無 id 錯誤——模擬器以前不擋，真機會。
    #[tokio::test]
    async fn an_oversize_line_is_refused_before_it_reaches_the_queue() {
        // 埠不存在也無妨：長度檢查在排隊之前，與連線狀態無關。
        let link = SerialRawLink::spawn("/nonexistent/esp32-line-limit-probe".into(), 115_200);
        // 以量到的骨架長度算 padding（不寫死），探針才會剛好落在上限。
        let probe = |pad: &str| {
            format!(r#"{{"type":"cmd","id":"e","name":"led.set","params":{{"pad":"{pad}"}}}}"#)
        };
        let skeleton = probe("").len();
        let exact = probe(&"x".repeat(MAX_LINE_BYTES - skeleton));
        assert_eq!(
            exact.len(),
            MAX_LINE_BYTES,
            "probe line should sit exactly on the limit"
        );
        assert!(
            RawLink::send(&*link, exact.clone()).await.is_ok(),
            "639 bytes is the firmware's maximum and must be accepted"
        );
        let over = format!("{exact}x");
        match RawLink::send(&*link, over.clone()).await {
            Err(LinkError::Refused(detail)) => {
                assert!(detail.starts_with("message too large"), "{detail}");
                assert!(detail.contains("640 bytes"), "{detail}");
            }
            other => panic!("640 bytes must be Refused before the wire, got {other:?}"),
        }
        match RawLink::send_before(&*link, over, Instant::now() + Duration::from_secs(5)).await {
            Err(LinkError::Refused(detail)) => {
                assert!(detail.starts_with("message too large"), "{detail}")
            }
            other => panic!("send_before must apply the same limit, got {other:?}"),
        }
        link.shutdown();
    }

    /// 分段供應的讀取來源：每次 read 回一段，或一個 TimedOut（模擬真硬體
    /// 在一行中間停頓超過 200ms 的讀取逾時）。
    struct ChunkedReader {
        chunks: std::collections::VecDeque<Result<String, std::io::ErrorKind>>,
    }

    impl ChunkedReader {
        fn new(items: Vec<Result<String, std::io::ErrorKind>>) -> Self {
            Self {
                chunks: items.into_iter().collect(),
            }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.chunks.front_mut() {
                None => Ok(0), // EOF：結束迴圈
                Some(Err(kind)) => {
                    let kind = *kind;
                    self.chunks.pop_front();
                    Err(std::io::Error::new(kind, "simulated read timeout"))
                }
                Some(Ok(text)) => {
                    let n = text.len().min(buf.len());
                    buf[..n].copy_from_slice(&text.as_bytes()[..n]);
                    text.drain(..n);
                    if text.is_empty() {
                        self.chunks.pop_front();
                    }
                    Ok(n)
                }
            }
        }
    }

    fn chunk(text: &str) -> Result<String, std::io::ErrorKind> {
        Ok(text.to_string())
    }

    /// 讀取逾時**不得**丟掉已讀到的半行。真硬體在送 ack 途中被感測器讀取
    /// 打斷（>200ms）是常態；舊版每輪先 line.clear()，半行被丟、後半解析
    /// 失敗被靜默丟棄，host 端就把一個裝置其實已經 ack 的命令記成
    /// 「ack 逾時、結果未知」。
    #[test]
    fn a_read_timeout_keeps_the_partial_line() {
        let (inbound, mut rx) = broadcast::channel(8);
        let alive = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let reader = ChunkedReader::new(vec![
            chunk(r#"{"type":"ack","id":"#),
            Err(std::io::ErrorKind::TimedOut), // 行中間停頓
            chunk(r#""a1"}"#),
            Err(std::io::ErrorKind::WouldBlock), // 換行前又停頓
            chunk("\n"),
        ]);
        pump_lines(
            reader,
            "/dev/fake",
            &alive,
            &shutdown,
            &inbound,
            &AtomicU64::new(0),
        );
        let msg = rx
            .try_recv()
            .expect("被逾時切成三段的 ack 仍必須解析得出來");
        assert_eq!(
            msg,
            DeviceMsg::Ack {
                id: Some("a1".into()),
                applied: None,
                dup: None,
                cancelled: None,
                stop_all: None,
            }
        );
        assert!(rx.try_recv().is_err(), "只該有一則訊息");
    }

    /// 完整的一行讀完就清緩衝：下一行不得黏在前一行後面。
    #[test]
    fn complete_lines_are_not_concatenated() {
        let (inbound, mut rx) = broadcast::channel(8);
        let alive = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let reader = ChunkedReader::new(vec![
            chunk("{\"type\":\"ack\",\"id\":\"a1\"}\n"),
            chunk("{\"type\":\"ack\",\"id\":\"a2\"}\n"),
        ]);
        pump_lines(
            reader,
            "/dev/fake",
            &alive,
            &shutdown,
            &inbound,
            &AtomicU64::new(0),
        );
        for want in ["a1", "a2"] {
            match rx.try_recv().expect("兩行都要收到") {
                DeviceMsg::Ack { id, .. } => assert_eq!(id.as_deref(), Some(want)),
                other => panic!("expected ack, got {other:?}"),
            }
        }
    }

    /// 保留半行必須有界：一直不送換行的裝置不得把緩衝撐大——超過上限就
    /// 丟掉並警告，之後的完整訊息仍要讀得到（不是把 reader 卡死）。
    #[test]
    fn an_unterminated_line_is_bounded_and_recovers() {
        let (inbound, mut rx) = broadcast::channel(8);
        let alive = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let reader = ChunkedReader::new(vec![
            chunk(&"x".repeat(MAX_PARTIAL_LINE_BYTES + 1_000)), // 永遠沒有換行
            Err(std::io::ErrorKind::TimedOut),
            chunk("{\"type\":\"ack\",\"id\":\"a1\"}\n"),
        ]);
        pump_lines(
            reader,
            "/dev/fake",
            &alive,
            &shutdown,
            &inbound,
            &AtomicU64::new(0),
        );
        match rx.try_recv().expect("超長殘段丟棄後仍要讀得到下一則") {
            DeviceMsg::Ack { id, .. } => assert_eq!(id.as_deref(), Some("a1")),
            other => panic!("expected ack, got {other:?}"),
        }
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

    /// link-transports-054：pty／檔案 fallback 的 read 沒有逾時，reader 與
    /// writer 又是同一個 fd 的 `dup`（關掉 writer 解不開對方的 blocking read）。
    /// 裝置完全沉默時 shutdown 只能放生那條執行緒——這是真的洩漏，所以**必須
    /// 被計數**，不能讓 `RawLink::shutdown` 的契約（「回收執行緒」）與實際
    /// 行為悄悄不一致。
    ///
    /// 若日後把 fallback 改成可中斷的讀法（O_NONBLOCK＋poll），這個測試要
    /// 反過來斷言計數**不**增加。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reader_that_cannot_be_reclaimed_is_counted_not_hidden() {
        let Some(mut pty) = Pty::spawn() else {
            eprintln!("python3 unavailable; skipping pty test");
            return;
        };
        // 只有沒有讀取逾時的 fallback 路徑才會卡住；serialport 正常路徑
        // 有 200ms 逾時，reader 自己就會結束。
        let fallback = matches!(open_port(&pty.path, 115_200), Ok(PortHalves::File(_)));
        if !fallback {
            eprintln!("this platform opens the pty through serialport (read timeout); skipping");
            pty.kill();
            return;
        }
        let before = detached_reader_threads();
        let link = SerialRawLink::spawn(pty.path.clone(), 115_200);
        // 等 supervisor 真的連上（並起了 reader）。
        for _ in 0..30 {
            if RawLink::connected(&*link) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(RawLink::connected(&*link), "pty should open");
        link.shutdown();
        // supervisor 最多 200ms 看到旗標，再給 reader 500ms 寬限。
        for _ in 0..40 {
            if detached_reader_threads() > before {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            detached_reader_threads() > before,
            "沉默的 pty 上放生的 reader 必須被計數（現在是已知限制，不是「已回收」）"
        );
        pty.kill();
    }

    /// link-transports-049：開得起來、但立刻 EOF 的節點（一般檔案／
    /// /dev/null／peer 已關的 pty）必須退避，並在連續幾輪之後把健康度
    /// 誠實降到 offline。舊版的退避只寫在「開不起來」那一邊：這種節點會
    /// 每 ≤200ms 重連一輪（thread churn、世代狂跳、排隊的命令被一直丟掉），
    /// 而 health 永遠停在「連線中」，使用者永遠不知道這台裝置不能用。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_port_that_dies_immediately_backs_off_and_is_reported_offline() {
        let path = std::env::temp_dir().join(format!(
            "serial-churn-{}-{}.probe",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, b"").expect("probe file");
        let path_str = path.to_string_lossy().to_string();
        if !matches!(open_port(&path_str, 115_200), Ok(PortHalves::File(_))) {
            eprintln!("this platform does not take the file fallback; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
        let link = SerialRawLink::spawn(path_str, 115_200);
        // 每段連線約 200ms（writer 迴圈的 recv_timeout）。沒有退避時，
        // 1.5 秒內會轉 6 輪以上；有退避（1s→2s）最多 2 輪。
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        let generations = RawLink::generation(&*link);
        assert!(
            generations <= 3,
            "立刻斷的埠必須退避（世代 {generations} 次＝無退避的重連 churn）"
        );
        assert_eq!(
            RawLink::link_state(&*link),
            LinkState::Disconnected,
            "連續立刻斷之後，健康度必須誠實落到 offline，不能一直停在「連線中」"
        );
        link.shutdown();
        let _ = std::fs::remove_file(&path);
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
