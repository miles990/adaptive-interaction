//! Character Session Host：AIP Character Session（`docs/aip/character-session.md`）的
//! Runtime 接線（use case 層）。
//!
//! 權威狀態機是純函式 crate `interaction-session`；這個模組只做它做不到的事：持久化、
//! 注入時間、把 [`Output`] 真的派送出去（裝置出站通道登記表、SSE、CPP renderer）、寫稽核。
//!
//! # 不變量
//!
//! - 語意狀態只有 [`CharacterSession`] 能改；這裡不推論、不改寫真相。
//! - `verified` 只能經 [`Runtime::character_session_submit_runtime`] 從 Runtime 的人類驗證
//!   路徑進來；device／renderer 送 `task.*`／`runtime.*` 一律 `scope-denied`。
//! - 身分是綁定出來的，不是宣稱：transport 先比對 `source`，不符即 `identity-mismatch`，
//!   不「幫忙修正」後執行。
//! - 裝置的收送都是**型別抹除**的：入站帶 [`DeviceOrigin`]、出站查 [`DeviceOutbound`] 登記表。
//!   這個模組沒有任何 `iphone`／`serial` 特判——傳輸標籤與身分強度都由那一側說出來。
//! - 所有集合有界（成員、事件日誌、去重環、pending intent 都在 session crate 內夾住）；
//!   這裡不開任何無界佇列、不做 blocking sleep，持久化走 `spawn_blocking`。
//! - `INTERACT_AI_CHARACTER_SESSION=0`：host 為 `None`，HTTP 入口 503 `session-disabled`、
//!   iPhone 的 `aip` frame 回 `error{unsupported-capability}`，其餘 v0.5.1 行為不變。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{Duration as ChronoDuration, Utc};
use interaction_aip::{
    bind_identity, negotiate_version, CapabilityAnnouncement, ErrorCode, ErrorPayload,
    IdentityDecision, MemberRole, MessageType, Outcome, PartyKind, SyncClass, Timestamp,
    SPEC_VERSION,
};
/// Transport 層（HTTP／Tauri IPC）直接用得到的 AIP 型別。從這裡 re-export，讓
/// 介面層不必各自依賴 `interaction-aip`（契約仍然只有一份）。
pub use interaction_aip::{Envelope, Party};
use interaction_core::{DomainError, DomainResult, EventType, RuntimeEvent};
use interaction_session::ports::{PortError, SaveOutcome, SessionStore};
use interaction_session::{
    CharacterSession, Output, Presence, Resume, RuntimeFact, SessionConfig, Snapshot, Submission,
    EVENT_TOUCH, HOST_INPUTS, HOST_INTENTS,
};
use serde_json::{json, Map, Value};

use crate::runtime::Runtime;

/// Feature flag（預設開）。`0` = 不啟動 Session Host。
pub const CHARACTER_SESSION_ENV: &str = "INTERACT_AI_CHARACTER_SESSION";
/// diagnostics `storeNote`：持久化檔壞掉時的固定文字。故意不含錯誤細節與路徑
/// （AIP §5：診斷不得洩漏路徑／輸入內容）。
pub const STORE_NOTE_UNUSABLE: &str =
    "stored character session state was unusable; it was quarantined and a new session was started";
/// diagnostics `storeNote`：持久化檔讀不到（權限／EIO／fd 用盡）時的固定文字。
///
/// **不得宣稱已隔離**：這條路徑刻意**不動**那個檔案（一次暫時性 I/O 失敗不該變成永久
/// 資料遺失），store 改成 parked，這一輪整個以記憶體模式跑。
pub const STORE_NOTE_UNREADABLE: &str =
    "character session state could not be read; the stored file was left untouched and this run \
     uses a fresh in-memory session";
/// diagnostics `storeNote`：快照由更新版本寫成（`format` 比這個版本認得的大）。
/// 保留、不隔離、不覆寫——把它蓋掉等於替使用者做了降級決定。
pub const STORE_NOTE_FUTURE_FORMAT: &str =
    "character session state was written by a newer version; it was kept as it is and this \
     version will not overwrite it";
/// diagnostics `storeNote`：舊格式的快照已遷移（原檔另外備份了一份）。
pub const STORE_NOTE_MIGRATED: &str =
    "character session state was written in an older format; it was migrated and the original \
     was kept as a backup";
/// diagnostics `storeNote`：舊格式的快照這次**沒有**遷移（備份做不出來）。原檔保留、不隔離。
pub const STORE_NOTE_MIGRATION_DEFERRED: &str =
    "character session state is in an older format; it was kept as it is because a backup could \
     not be made";
/// diagnostics `storeNote`：舊格式的快照**備份成功了**，但新格式沒有落地
/// （寫入失敗，或這一輪的 store 跳過了這次寫入）。
///
/// 為什麼與 [`STORE_NOTE_MIGRATION_DEFERRED`] 分開：那一句說的是「備份做不出來」，
/// 是使用者要去看備份路徑被什麼佔住的故障；這一句是「備份在了、檔案沒換成新格式」，
/// 要看的是磁碟／權限。共用一句話會把人指到錯的地方。
///
/// **不 park**：備份已經在磁碟上了，之後的 persist 把新格式寫上去不會弄丟任何東西
/// （這正是 [`STORE_NOTE_UNREADABLE`] 那條路徑要 park 的理由——那裡沒有備份）。
/// `SaveOutcome::SkippedStale`／`SkippedParked` 也走這一句：它們同樣是「沒有落地」，
/// 而這句話的重點（磁碟上仍是舊格式）對它們一樣為真。
pub const STORE_NOTE_MIGRATION_WRITE_FAILED: &str =
    "character session state is in an older format; the migrated snapshot could not be written, \
     so the original format is still on disk";
/// diagnostics `store.note`：目前寫不進去（持續的持久化失敗）。
pub const STORE_NOTE_PERSIST_FAILING: &str =
    "character session state is not being persisted right now";
/// 遷移備份的副檔名前綴：`character-session.json.pre-format-<n>`。同一個來源格式只保留
/// 一份（再遷移一次會覆寫它）。
pub const SESSION_BACKUP_SUFFIX: &str = "pre-format";
/// 持久化檔名（`<home>/state/`）。
pub const SESSION_STORE_FILE: &str = "character-session.json";
/// 只記 epoch 的小檔（`<home>/state/`）。快照壞掉時 epoch 從這裡續接——
/// 見 [`JsonSessionStore::next_epoch`]。
pub const SESSION_EPOCH_FILE: &str = "character-session.epoch";
/// 1.0 只有一個 session。
pub const SESSION_ID: &str = "session.home";
/// 桌面可信 host surface 的 party id（human token 綁定出來的身分）。
pub const DESKTOP_SURFACE_ID: &str = "desktop";
/// Host 送出的 `error`／`response` 用的 name。
const NAME_SESSION_ERROR: &str = "character.session.error";
const NAME_SESSION_RESUME: &str = "character.session.resume";
const NAME_SESSION_SNAPSHOT_QUERY: &str = "character.session.snapshot";
/// 停用時的固定人話（不含路徑、不回顯輸入）。
pub const SESSION_DISABLED_MESSAGE: &str = "character session is turned off on this runtime";

/// 桌面可信 host surface 的 Party。
pub fn desktop_party() -> Party {
    Party::human_surface(DESKTOP_SURFACE_ID)
}

// ---------------------------------------------------------------------------
// 裝置出站通道：型別抹除的登記表
// ---------------------------------------------------------------------------

/// 已配對 iPhone 的身分強度：配對時交換的 per-device token，host 端以
/// sha256(token) 逐次驗證。
pub const IDENTITY_STRENGTH_PAIRED_TOKEN: &str = "paired-token";

/// 宣告式裝置線（serial／mqtt／ble）的身分強度：傳輸層 hello 的 `deviceId`
/// 明文字串比對 ＋ 配對碼由**裝置端**比對（host 只送碼、等 pair-ok）。
///
/// 誠實：這比 [`IDENTITY_STRENGTH_PAIRED_TOKEN`] 弱。任何能開那個埠／topic
/// 的程序都可以自稱是這台裝置。文件與 UI 不得把它寫成「已驗證身分」。
pub const IDENTITY_STRENGTH_DEVICE_LINK: &str = "transport-hello+device-side-pairing";

/// 桌面可信 host surface 的身分強度：human token 在 transport 綁定出來的身分
/// （宣稱的 `source` 必須與它相符才收）。
pub const IDENTITY_STRENGTH_HOST_SURFACE: &str = "host-surface";

/// 通道回報了一個核心不認得的身分強度。誠實說「不知道」——不沿用任何既有
/// 標籤，更不預設它是強的那一種。
pub const IDENTITY_STRENGTH_UNKNOWN: &str = "unknown";

/// 出站通道登記表的上限。一台主機同時連著的裝置是個位數；這個數字只是不讓
/// 一個壞掉的（或惡意的）登記端把表撐成無界成長。
pub const MAX_DEVICE_OUTBOUND: usize = 64;

/// 一條「送得到某台裝置」的出站通道，**型別抹除**。
///
/// 為什麼要有它：`Output::Broadcast`（別的成員造成的 shared state 變更）必須
/// 送到**每一個**線上裝置成員。在此之前那條路徑直接呼叫 iPhone 的 wss 出站，
/// 所以第二種裝置（serial／mqtt／ble）永遠收不到廣播——桌面顯示「已加入、
/// 已同步」，那台裝置其實一則狀態都沒有收到。核心不該（也不能）逐一列舉傳輸
/// 種類，所以它只認得這條 trait；`transport` 與身分強度由**通道自己**說出來。
#[async_trait::async_trait]
pub trait DeviceOutbound: Send + Sync {
    /// 把一則 envelope 送上這條線。回 `Ok` 只代表「已寫上線」——不是對端收到、
    /// 更不是對端套用了（誠實階梯）。
    async fn send_aip(&self, envelope: &Envelope) -> Result<(), DomainError>;
    /// 傳輸種類（`iphone`／`serial`／`mqtt`／`ble`）。稽核用。
    fn transport_label(&self) -> &str;
    /// 這條線的身分是**怎麼來的**（見上面三個 `IDENTITY_STRENGTH_*`）。
    /// 不同傳輸的身分強度不同，diagnostics 必須說得出差別。
    fn identity_strength(&self) -> &str;
}

/// 一則裝置 frame 的來源事實（稽核用）。
///
/// 核心不認得傳輸種類：這兩個值一律由**收到這則 frame 的那條路徑**提供。
/// 寫死成某一種傳輸的話，「有人從序列線偽造身分」在稽核上會長得像
/// 「某台 iPhone 出問題」——查錯方向、也查不到真正的來源。
#[derive(Debug, Clone, Copy)]
pub struct DeviceOrigin<'a> {
    pub transport: &'a str,
    pub identity_strength: &'a str,
}

/// 讀 feature flag（只有明確的 `0` 會關閉）。
pub fn character_session_enabled_from_env() -> bool {
    !matches!(
        std::env::var(CHARACTER_SESSION_ENV).ok().as_deref(),
        Some("0")
    )
}

// ---------------------------------------------------------------------------
// 持久化：JSON 檔（原子寫入 tmp+rename、0600）
// ---------------------------------------------------------------------------

/// 持久化檔的大小上限。快照是有界的（成員 ≤ limits::MAX_MEMBERS、事件日誌不進快照），
/// 正常值是幾 KiB；超過這個量級代表檔案被別的東西寫過，不該整個讀進記憶體。
pub const MAX_SNAPSHOT_BYTES: u64 = 1024 * 1024;

/// `lastPersistError` 的固定文字（不含路徑、不含 I/O 錯誤細節）。
pub const PERSIST_ERROR_WRITE: &str = "the character session snapshot could not be written";
pub const PERSIST_ERROR_ENCODE: &str = "the character session snapshot could not be encoded";
pub const PERSIST_ERROR_TASK: &str = "the character session persistence task did not finish";

/// 持久化的可觀測狀態（進 diagnostics）。全部是計數與固定文字，不含路徑、不回顯輸入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreStatus {
    /// 這一輪從檔案讀到的格式版本（沒有檔案／讀不到＝`None`）。
    pub format: Option<u32>,
    /// 最後一次**真的落地**的 revision（誠實階梯：被要求寫≠寫成功）。
    pub last_persisted_revision: Option<u64>,
    /// 寫入失敗次數。
    pub persist_failures: u64,
    /// 因為不比已落地者新而略過的次數。
    pub skipped_stale: u64,
    /// store 是否 parked（一律拒絕寫入）。
    pub parked: bool,
    /// 最近一次寫入失敗的固定文字。
    pub last_persist_error: Option<&'static str>,
    /// 給人看的固定文字（parked 原因，或「目前寫不進去」）。
    pub note: Option<&'static str>,
}

/// [`JsonSessionStore`] 的可變狀態。**檢查與寫入必須在同一個鎖裡**：兩個併發的 `save`
/// 若各自先檢查再寫，會同時通過 guard 再亂序 rename，等於沒有 guard。
#[derive(Debug, Default)]
struct StoreState {
    /// 已經落地的 `(epoch, revision)`。`None` ＝ 還沒播種。
    committed: Option<(u64, u64)>,
    /// 已經嘗試過播種（避免每次 save 都去 stat 一次檔案）。
    seeded: bool,
    format: Option<u32>,
    parked: Option<&'static str>,
    persist_failures: u64,
    skipped_stale: u64,
    last_persisted_revision: Option<u64>,
    last_persist_error: Option<&'static str>,
}

/// Snapshot 的 JSON 檔 store。檔案內容就是 [`Snapshot`]（含 `epoch`、`format`）。
///
/// # 三個不變量
///
/// 1. **不倒退**：`(epoch, revision)` 字典序不比已落地者新的快照一律略過
///    （[`SaveOutcome::SkippedStale`]），因為持久化是由多個併發任務各自派送的。
/// 2. **parked 就完全不寫**：檔案讀不到（權限／EIO）或由更新版本寫成時，這個 store
///    變成唯讀。只跳過「開機那一次」不夠——之後每一次 persist 都會 rename 蓋掉它。
/// 3. **有界讀取**：超過 [`MAX_SNAPSHOT_BYTES`] 直接判 [`PortError::Corrupt`]，不整個讀進記憶體。
pub struct JsonSessionStore {
    path: PathBuf,
    state: Mutex<StoreState>,
}

impl JsonSessionStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(StoreState::default()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 鎖中毒不讓持久化整條斷掉（毒化只代表某個 panic 發生過，資料仍然是一致的計數）。
    fn state(&self) -> MutexGuard<'_, StoreState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 目前的可觀測狀態（給 diagnostics）。
    pub fn status(&self) -> StoreStatus {
        let state = self.state();
        StoreStatus {
            format: state.format,
            last_persisted_revision: state.last_persisted_revision,
            persist_failures: state.persist_failures,
            skipped_stale: state.skipped_stale,
            parked: state.parked.is_some(),
            last_persist_error: state.last_persist_error,
            note: state.parked.or({
                if state.last_persist_error.is_some() {
                    Some(STORE_NOTE_PERSIST_FAILING)
                } else {
                    None
                }
            }),
        }
    }

    /// 讓這個 store 變成唯讀。`note` 必須是固定文字（會進 diagnostics）。
    pub fn park(&self, note: &'static str) {
        self.state().parked = Some(note);
    }

    /// 記一次「持久化沒有完成」（例如 `spawn_blocking` 的 join 失敗）。
    /// 錯誤不得被吞掉：queued≠completed。
    pub fn note_persist_failure(&self, error: &'static str) {
        let mut state = self.state();
        state.persist_failures = state.persist_failures.saturating_add(1);
        state.last_persist_error = Some(error);
    }

    /// 從檔案上已經有的東西播種順序 guard（`load` 之後呼叫；沒有 `load` 過的 store
    /// 在第一次 `save` 時也會補做一次，避免跨程序的第一筆寫入就倒退）。
    fn seed_from(state: &mut StoreState, snapshot: Option<&Snapshot>) {
        state.seeded = true;
        if let Some(snapshot) = snapshot {
            state.format = Some(snapshot.format);
            let seen = (snapshot.epoch, snapshot.revision);
            state.committed = Some(match state.committed {
                Some(current) => current.max(seen),
                None => seen,
            });
        }
    }

    /// best-effort：從磁碟上那份檔案讀出 `(epoch, revision)`。讀不到就當作沒有。
    fn on_disk_progress(&self) -> Option<Snapshot> {
        self.read_bounded().ok().flatten().and_then(|body| {
            serde_json::from_str::<Snapshot>(&body)
                .ok()
                .filter(|s| s.session_id == SESSION_ID)
        })
    }

    /// 有界讀取。`Ok(None)` ＝ 檔案不存在；超過上限 → `Corrupt`；其餘 I/O 失敗 → `Unavailable`。
    fn read_bounded(&self) -> Result<Option<String>, PortError> {
        use std::io::Read as _;
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(PortError::Unavailable),
        };
        if let Ok(metadata) = file.metadata() {
            if metadata.is_file() && metadata.len() > MAX_SNAPSHOT_BYTES {
                return Err(PortError::Corrupt);
            }
        }
        let mut body = String::new();
        // 即使 metadata 說謊（檔案在 stat 之後長大／是特殊檔案），take 也把讀取夾住。
        file.take(MAX_SNAPSHOT_BYTES.saturating_add(1))
            .read_to_string(&mut body)
            .map_err(|_| PortError::Unavailable)?;
        if body.len() as u64 > MAX_SNAPSHOT_BYTES {
            return Err(PortError::Corrupt);
        }
        Ok(Some(body))
    }

    /// 只記 epoch 的小檔。**故意與快照分開**：快照壞到救不回 epoch 時，它是
    /// 「成員記得的 epoch 至少有多大」的唯一線索。
    fn epoch_path(&self) -> PathBuf {
        self.path
            .parent()
            .map(|parent| parent.join(SESSION_EPOCH_FILE))
            .unwrap_or_else(|| PathBuf::from(SESSION_EPOCH_FILE))
    }

    /// 上次落地時記下的 epoch（讀不到／壞掉＝0，不猜）。
    fn remembered_epoch(&self) -> u64 {
        let Ok(body) = std::fs::read_to_string(self.epoch_path()) else {
            return 0;
        };
        salvaged_epoch(&body)
    }

    /// 記住這個 epoch（best-effort：記不下來也不能讓快照落不了地）。
    fn remember_epoch(&self, epoch: u64) {
        let body = json!({ "epoch": epoch }).to_string();
        if let Err(error) = self.write_owner_only_to(&self.epoch_path(), &body) {
            tracing::warn!(%error, "the character session epoch marker was not persisted");
        }
    }

    /// 重建 session 時要用的下一個 epoch。
    ///
    /// 契約 §1：「host 每次重建 session 時 epoch+1」。壞掉的快照自己救回來的數字
    /// **不夠**：整個檔案被清成 NUL／截成空檔時 `salvaged_epoch` 回 0，+1 之後又回到
    /// 全新 session 的 1——與成員記得的 epoch 撞號，resume 不走 EpochMismatch，
    /// 成員把 host 的新狀態當 rollback 忽略，兩邊都以為「已同步」。
    ///
    /// 所以取 `max(從壞檔救回的, 另外記住的)`；兩邊都是 0（連 epoch 檔都沒有）時
    /// 退回一個**單調的牆鐘來源**（unix 秒），它保證大於任何以 1 起算的遞增 epoch。
    fn next_epoch(&self) -> u64 {
        let salvaged = std::fs::read_to_string(&self.path)
            .map(|body| salvaged_epoch(&body))
            .unwrap_or(0);
        let known = salvaged.max(self.remembered_epoch());
        let next = if known == 0 {
            fresh_epoch_seed()
        } else {
            known.saturating_add(1)
        };
        self.remember_epoch(next);
        next
    }

    /// 壞掉的檔案改名成 `<file>.corrupt` 並回傳「下一個 epoch」。
    /// 不靜默：epoch 一定往前跳，讓所有成員在 resume 時拿到 `session-reset`，
    /// 而不是默默對齊到一個從頭開始的 revision。
    fn quarantine(&self) -> u64 {
        let next = self.next_epoch();
        {
            // 檔案被移走了：順序 guard 的播種基準跟著失效，否則新 session 的
            // revision 1 會被當成「落後」而永遠寫不進去。
            let mut state = self.state();
            state.committed = None;
            state.seeded = true;
            state.format = None;
        }
        let quarantined = self.path.with_extension("json.corrupt");
        if let Err(e) = std::fs::rename(&self.path, &quarantined) {
            tracing::warn!(error = %e, "could not quarantine the unreadable character session file");
            let _ = std::fs::remove_file(&self.path);
        }
        next
    }

    fn write_owner_only(&self, contents: &str) -> Result<(), PortError> {
        self.write_owner_only_to(&self.path, contents)
    }

    fn write_owner_only_to(&self, target: &Path, contents: &str) -> Result<(), PortError> {
        let parent = target.parent().ok_or(PortError::Unavailable)?;
        std::fs::create_dir_all(parent).map_err(|_| PortError::Unavailable)?;
        // 每次寫入都用自己的暫存檔：兩個持久化同時發生時（`Output::Persist` 各自
        // 走一次 `spawn_blocking`），共用檔名的 `truncate` 會截掉另一個寫入者已經
        // 寫進去的內容，rename 出去的就是兩份 JSON 的拼接。程序內用計數器分開，
        // 跨程序用 pid 分開。
        let ticket = TMP_TICKET.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stem = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| SESSION_STORE_FILE.to_string());
        let tmp = parent.join(format!(".{stem}.tmp-{}-{ticket}", std::process::id()));
        let written = (|| -> Result<(), PortError> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp).map_err(|_| PortError::Unavailable)?;
            use std::io::Write as _;
            file.write_all(contents.as_bytes())
                .map_err(|_| PortError::Unavailable)?;
            file.sync_all().map_err(|_| PortError::Unavailable)
        })();
        if written.is_err() {
            // 半成品不留在 state 目錄裡（否則每次失敗都多一個檔案）。
            let _ = std::fs::remove_file(&tmp);
            return written;
        }
        if let Err(error) = std::fs::rename(&tmp, target) {
            let _ = std::fs::remove_file(&tmp);
            tracing::debug!(%error, "character session snapshot could not be renamed into place");
            return Err(PortError::Unavailable);
        }
        Ok(())
    }
}

/// 完全救不回 epoch 時的起點：單調的牆鐘秒數。任何以 1 起算、每次重建 +1 的
/// epoch 都不可能長到這個量級，所以成員記得的 epoch 一定與它不同（§1）。
fn fresh_epoch_seed() -> u64 {
    Utc::now().timestamp().max(1) as u64
}

/// 暫存檔名的程序內序號（見 [`JsonSessionStore::write_owner_only`]）。
static TMP_TICKET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl SessionStore for JsonSessionStore {
    fn save(&self, snapshot: &Snapshot) -> Result<SaveOutcome, PortError> {
        let body = match serde_json::to_string(snapshot) {
            Ok(body) => body,
            Err(_) => {
                self.note_persist_failure(PERSIST_ERROR_ENCODE);
                return Err(PortError::Rejected);
            }
        };
        // 檢查與寫入在同一個鎖裡（見型別文件的不變量 1）。這個鎖只跨檔案 I/O，
        // 不跨 `.await`；呼叫端已經在 `spawn_blocking` 上。
        let mut state = self.state();
        if let Some(note) = state.parked {
            tracing::debug!(
                note,
                "the character session store is parked; nothing was written"
            );
            return Ok(SaveOutcome::SkippedParked);
        }
        if !state.seeded {
            drop(state);
            let existing = self.on_disk_progress();
            state = self.state();
            Self::seed_from(&mut state, existing.as_ref());
            // 播種期間可能已經被 park（另一條路徑同時發現檔案讀不到）。
            if let Some(note) = state.parked {
                tracing::debug!(
                    note,
                    "the character session store is parked; nothing was written"
                );
                return Ok(SaveOutcome::SkippedParked);
            }
        }
        let incoming = (snapshot.epoch, snapshot.revision);
        if let Some(committed) = state.committed {
            if incoming <= committed {
                state.skipped_stale = state.skipped_stale.saturating_add(1);
                return Ok(SaveOutcome::SkippedStale);
            }
        }
        // 先記 epoch 再寫快照：epoch 檔只能領先、不能落後，否則快照壞掉時就少了
        // 「成員記得的 epoch 至少有多大」這個線索。
        if snapshot.epoch > self.remembered_epoch() {
            self.remember_epoch(snapshot.epoch);
        }
        match self.write_owner_only(&body) {
            Ok(()) => {
                state.committed = Some(incoming);
                state.format = Some(snapshot.format);
                state.last_persisted_revision = Some(snapshot.revision);
                state.last_persist_error = None;
                Ok(SaveOutcome::Written)
            }
            Err(error) => {
                state.persist_failures = state.persist_failures.saturating_add(1);
                state.last_persist_error = Some(PERSIST_ERROR_WRITE);
                Err(error)
            }
        }
    }

    fn load(&self, session_id: &str) -> Result<Option<Snapshot>, PortError> {
        let Some(body) = self.read_bounded()? else {
            return Ok(None);
        };
        // 先解析成 `Value` 辨識格式：比這個版本新的檔案**不是**壞檔，不得隔離也不得覆寫。
        let value: Value = serde_json::from_str(&body).map_err(|_| PortError::Corrupt)?;
        let format = value.get("format").and_then(Value::as_u64).unwrap_or(0);
        if format > u64::from(interaction_session::SNAPSHOT_FORMAT) {
            return Err(PortError::FutureFormat);
        }
        let snapshot: Snapshot = serde_json::from_value(value).map_err(|_| PortError::Corrupt)?;
        if snapshot.session_id != session_id {
            return Err(PortError::Corrupt);
        }
        Self::seed_from(&mut self.state(), Some(&snapshot));
        Ok(Some(snapshot))
    }
}

// ---------------------------------------------------------------------------
// Host
// ---------------------------------------------------------------------------

/// Runtime 端的 Session Host：權威 session＋持久化＋桌面 presence 記憶。
pub struct CharacterSessionHost {
    session: Mutex<CharacterSession>,
    store: Arc<JsonSessionStore>,
    /// 已經投影出去的桌面 presence。只有變化時才動狀態——否則每個 watchdog tick
    /// 都會製造一個新的 revision 與一次廣播。
    desktop_presence: Mutex<Option<Presence>>,
    /// 載入時**讓 session 重建**的異常（→ diagnostics 的 `storeNote`）。
    ///
    /// 語意刻意保持與 v0.6.0 相同：「保存的紀錄用不了，這一輪是全新的 session」。
    /// 桌面把非 null 的 `storeNote` 當成「紀錄曾經重建過」的灰色附註（v0.6.x 起不再是警告狀態；
    /// 現在存不下來才是 `store.parked`／persist 失敗的 active issue），所以
    /// **遷移不得寫進這裡**——遷移沒有重建 session，也沒有丟掉任何東西。
    load_note: Option<String>,
    /// 這一輪做過的格式遷移（來源格式，固定文字）。只進新的 `store` 欄位。
    migration: Option<(u32, &'static str)>,
}

impl CharacterSessionHost {
    /// 從 `state/character-session.json` 續接。五種結果各自不同（M1 §2.2）：
    ///
    /// | 檔案狀態 | 做法 |
    /// |---|---|
    /// | 沒有檔案 | 新 session（epoch 1 或接續 epoch 檔） |
    /// | 現行格式且 canonical | 直接續接 |
    /// | 舊格式（`format < SNAPSHOT_FORMAT` 或 canonical 不同） | 續接＋備份原檔＋以新格式落地 |
    /// | 更新的格式（`format > SNAPSHOT_FORMAT`） | **保留、不隔離、不覆寫**；記憶體模式＋store parked |
    /// | 讀不到（權限／EIO） | 同上：保留、parked、記憶體模式 |
    /// | 真的損毀（解不開／不是這個 session） | 隔離成 `.corrupt`、epoch+1、新 session |
    pub fn open(state_dir: &Path, now: Timestamp) -> Arc<Self> {
        let store = Arc::new(JsonSessionStore::new(state_dir.join(SESSION_STORE_FILE)));
        let config = SessionConfig {
            session_id: SESSION_ID.to_string(),
            ..SessionConfig::default()
        };
        /// 開機後要不要落一份快照。
        enum Startup {
            /// 照常落地（新 session、或續接得上的快照）。
            Persist,
            /// 先把原檔備份成 `<file>.pre-format-<n>` 再落地。
            Migrate { from: u32 },
            /// 什麼都不寫：那份檔案不是我們有資格覆寫的。
            Preserve,
        }

        let (session, load_note, startup) = match store.load(SESSION_ID) {
            Ok(Some(mut snapshot)) => {
                // epoch 只能往前：這台機器曾經以更大的 epoch 跑過（例如上次啟動讀不到
                // 這份快照而另開了一個 session），成員記得的就是那個更大的值。
                snapshot.epoch = snapshot.epoch.max(store.remembered_epoch());
                match CharacterSession::restore_report(config.clone(), &snapshot, now) {
                    // 遷移**不是**重建：epoch 不變、成員不掉，所以不寫 `storeNote`。
                    Ok((session, report)) if report.needs_migration() => (
                        session,
                        None,
                        Startup::Migrate {
                            from: report.format_from,
                        },
                    ),
                    Ok((session, _)) => (session, None, Startup::Persist),
                    Err(error) => {
                        // 錯誤細節只進 log；diagnostics 的 note 是固定文字（不帶路徑、不帶
                        // 反序列化訊息——那些可能回顯檔案內容或檔案系統路徑）。
                        tracing::warn!(%error, "stored character session state was unusable");
                        let epoch = store.quarantine();
                        (
                            CharacterSession::new(config.clone(), epoch, now),
                            Some(STORE_NOTE_UNUSABLE.to_string()),
                            Startup::Persist,
                        )
                    }
                }
            }
            Ok(None) => {
                // 全新安裝：epoch 1。曾經跑過的機器不會走到這裡（檔案在），
                // 走到這裡卻留有 epoch 檔的話，`next_epoch` 會接續下去。
                let epoch = if store.remembered_epoch() == 0 {
                    store.remember_epoch(1);
                    1
                } else {
                    store.next_epoch()
                };
                (
                    CharacterSession::new(config.clone(), epoch, now),
                    None,
                    Startup::Persist,
                )
            }
            // 內容壞掉（解不開／不是這個 session 的快照／超過大小上限）：隔離。
            Err(PortError::Corrupt) => {
                tracing::warn!("stored character session state was unusable");
                let epoch = store.quarantine();
                (
                    CharacterSession::new(config.clone(), epoch, now),
                    Some(STORE_NOTE_UNUSABLE.to_string()),
                    Startup::Persist,
                )
            }
            // 更新版本寫的快照：檔案是好的，只是這個版本讀不懂。不隔離、不覆寫。
            Err(PortError::FutureFormat) => {
                tracing::warn!("stored character session state was written by a newer version");
                store.park(STORE_NOTE_FUTURE_FORMAT);
                let epoch = store.next_epoch();
                (
                    CharacterSession::new(config.clone(), epoch, now),
                    Some(STORE_NOTE_FUTURE_FORMAT.to_string()),
                    Startup::Preserve,
                )
            }
            // 暫時性 I/O 失敗（權限、EIO、fd 用盡）：檔案**不動**。一次讀不到就把
            // 一份可能完好的快照改名丟棄，是把暫時性故障變成永久資料遺失；只跳過開機
            // 那一次 save 也不夠——之後每一次 persist 都會 rename 蓋掉它，所以 store parked。
            Err(error) => {
                tracing::warn!(%error, "character session state could not be read");
                store.park(STORE_NOTE_UNREADABLE);
                let epoch = store.next_epoch();
                (
                    CharacterSession::new(config.clone(), epoch, now),
                    Some(STORE_NOTE_UNREADABLE.to_string()),
                    Startup::Preserve,
                )
            }
        };

        // 立刻落一份：重啟後才續接得到 revision／epoch，而不是默默從頭開始。
        let mut migration = None;
        match startup {
            Startup::Preserve => {}
            Startup::Migrate { from } => {
                // 一個來源格式只保留一份備份（再遷移一次會覆寫它），不會隨啟動次數長大。
                let backup = store
                    .path()
                    .with_extension(format!("json.{SESSION_BACKUP_SUFFIX}-{from}"));
                // 備份失敗就不遷移：原檔留著，等下一次啟動（或人）處理。**不隔離**。
                // 檔案沒換成新格式就不得說「已遷移」（誠實階梯：記憶體裡遷移了≠檔案
                // 遷移了），而且「為什麼沒遷移」的兩種原因不得共用同一句話。
                let note = match std::fs::copy(store.path(), &backup) {
                    Err(error) => {
                        tracing::warn!(%error, "the character session snapshot was not backed up before migrating");
                        // 這一條要 park：沒有備份，之後每一次 persist 都會 rename
                        // 蓋掉那份唯一的舊格式檔。
                        store.park(STORE_NOTE_MIGRATION_DEFERRED);
                        STORE_NOTE_MIGRATION_DEFERRED
                    }
                    Ok(_) => match store.save(&session.snapshot()) {
                        Ok(SaveOutcome::Written) => STORE_NOTE_MIGRATED,
                        // 寫不進去：rename 從來沒發生，原檔仍在（store 已經記下失敗）。
                        // `SkippedStale`／`SkippedParked` 也算沒有落地——磁碟上仍是
                        // 舊格式，這句話對它們一樣為真。**不 park**：備份已經在磁碟
                        // 上，下一次 persist 把新格式寫上去不會弄丟任何東西。
                        outcome => {
                            tracing::warn!(
                                ?outcome,
                                "the migrated character session snapshot was not persisted"
                            );
                            STORE_NOTE_MIGRATION_WRITE_FAILED
                        }
                    },
                };
                migration = Some((from, note));
            }
            Startup::Persist => {
                if let Err(error) = store.save(&session.snapshot()) {
                    tracing::warn!(%error, "character session snapshot was not persisted at startup");
                }
            }
        }
        if let Some(note) = &load_note {
            tracing::warn!(note, "character session started from a clean state");
        }
        if let Some((from, note)) = migration {
            tracing::info!(
                from,
                note,
                "the stored character session snapshot was migrated"
            );
        }
        Arc::new(Self {
            session: Mutex::new(session),
            store,
            desktop_presence: Mutex::new(None),
            load_note,
            migration,
        })
    }

    /// 權威 session。呼叫端必須在 `.await` 之前放掉這個 guard。
    pub fn session(&self) -> MutexGuard<'_, CharacterSession> {
        self.session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn desktop_presence(&self) -> MutexGuard<'_, Option<Presence>> {
        self.desktop_presence
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn store(&self) -> Arc<JsonSessionStore> {
        self.store.clone()
    }

    pub fn load_note(&self) -> Option<&str> {
        self.load_note.as_deref()
    }

    /// 這一輪的格式遷移：`(來源格式, 固定文字)`。沒有遷移就是 `None`。
    pub fn migration(&self) -> Option<(u32, &'static str)> {
        self.migration
    }
}

/// 一則 iPhone `aip` frame 的處理結果（transport 只負責送出去）。
pub struct AipFrameOutcome {
    /// 要回給這台手機的 envelope（已經是 JSON；transport 包成 `{"type":"aip",…}`）。
    pub replies: Vec<Value>,
    /// 已套用的互動觸碰 kind（recipe 相容：落成一筆 `iphone.touch` observation）。
    pub applied_touch: Option<String>,
}

impl AipFrameOutcome {
    fn reply(envelope: Envelope) -> Self {
        Self {
            replies: vec![serde_json::to_value(&envelope).unwrap_or(Value::Null)],
            applied_touch: None,
        }
    }
}

/// 從一份壞掉的持久化檔案盡量救回 `epoch`：先試完整 JSON，再退回掃描
/// `"epoch": <n>`（截斷的檔案也救得回來）。救不回來就當 0——新 epoch 至少是 1，
/// 不會與任何成員記得的 epoch 相同。
fn salvaged_epoch(body: &str) -> u64 {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Some(epoch) = value.get("epoch").and_then(Value::as_u64) {
            return epoch;
        }
    }
    let Some(index) = body.find("\"epoch\"") else {
        return 0;
    };
    let window: String = body[index + 7..].chars().take(32).collect();
    window
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// 成員清除門檻 ＝ presence 逾時 × 這個倍數。中間那一段就是契約 §11 的
/// 「iPhone 暫時離線」可以被看見的區間（見 [`Runtime::character_session_tick_at`]）。
pub const MEMBER_EVICTION_TIMEOUT_FACTOR: i64 = 2;

/// 一則 capability 宣告裡 `intents`／`inputs` 各自的筆數上限。
const MAX_ANNOUNCED_NAMES: usize = 64;

/// 稽核欄位的字元上限（攻擊者可控的字串不得無界寫進 audit）。
const AUDIT_FIELD_MAX_CHARS: usize = 64;

/// 夾住一段可能來自外部的字串（以字元為單位切，不會切壞 UTF-8）。
fn audit_snippet(value: &str) -> String {
    value.chars().take(AUDIT_FIELD_MAX_CHARS).collect()
}

/// `PartyKind` 的稽核書寫。`Unknown(String)` 是 untagged 的任意字串，一樣要夾住。
fn audit_party_kind(kind: &PartyKind) -> String {
    match serde_json::to_value(kind) {
        Ok(Value::String(text)) => audit_snippet(&text),
        _ => "unknown".to_string(),
    }
}

/// 一則 replay patch 的線上形狀（`response{kind:"patches"}` 的項目）。
/// 不內嵌完整 envelope：AIP §11 的 payload 深度上限是 8，包一層 envelope 會超。
fn patch_item(envelope: &Envelope) -> Value {
    json!({
        "sequence": envelope.sequence,
        "baseRevision": envelope.base_revision,
        "revision": envelope.payload.get("revision"),
        "patch": envelope.payload.get("patch"),
        "hash": envelope.payload.get("hash"),
        "sessionEpoch": envelope.payload.get("sessionEpoch"),
    })
}

fn transport_message_id() -> String {
    format!("aip-x-{}", uuid::Uuid::new_v4().simple())
}

impl Runtime {
    // ------------------------------------------------------------------
    // 生命週期與查詢
    // ------------------------------------------------------------------

    /// Session Host 是否啟用（`INTERACT_AI_CHARACTER_SESSION=0` 時為 false）。
    pub fn character_session_enabled(&self) -> bool {
        self.character_session.is_some()
    }

    fn session_host(&self) -> DomainResult<&Arc<CharacterSessionHost>> {
        self.character_session
            .as_ref()
            .ok_or_else(|| DomainError::Unavailable(SESSION_DISABLED_MESSAGE.to_string()))
    }

    /// 目前的權威快照（純讀，不消耗 sequence）。
    pub fn character_session_peek(&self) -> DomainResult<Snapshot> {
        Ok(self.session_host()?.session().snapshot())
    }

    // ------------------------------------------------------------------
    // 裝置出站通道登記表
    // ------------------------------------------------------------------

    fn device_outbound_table(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, std::collections::BTreeMap<String, Arc<dyn DeviceOutbound>>>
    {
        match self.device_outbound.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// 登記一台裝置的出站通道（iPhone 認證後、宣告式裝置握手成立後）。
    /// 有界：滿了誠實拒絕並留稽核，不靜默覆蓋別台裝置。
    pub(crate) fn register_device_outbound(
        &self,
        device_id: &str,
        channel: Arc<dyn DeviceOutbound>,
    ) {
        let mut table = self.device_outbound_table();
        if table.len() >= MAX_DEVICE_OUTBOUND && !table.contains_key(device_id) {
            drop(table);
            let _ = self.store.audit(
                "aip.outbound-rejected",
                "runtime",
                &json!({
                    "deviceId": audit_snippet(device_id),
                    "limit": MAX_DEVICE_OUTBOUND,
                    "reason": "the device outbound registry is full",
                }),
            );
            return;
        }
        table.insert(device_id.to_string(), channel);
    }

    /// 解除登記（斷線／撤銷／provider 下架）。留著等於之後每一則廣播都往一條
    /// 已經不存在的線上送。
    pub(crate) fn unregister_device_outbound(&self, device_id: &str) {
        self.device_outbound_table().remove(device_id);
    }

    fn device_outbound(&self, device_id: &str) -> Option<Arc<dyn DeviceOutbound>> {
        match self.device_outbound.read() {
            Ok(guard) => guard.get(device_id).cloned(),
            Err(poisoned) => poisoned.into_inner().get(device_id).cloned(),
        }
    }

    /// 目前登記中的裝置出站通道 id（診斷用；不含連線細節、不含 secret）。
    pub fn device_outbound_ids(&self) -> Vec<String> {
        match self.device_outbound.read() {
            Ok(guard) => guard.keys().cloned().collect(),
            Err(poisoned) => poisoned.into_inner().keys().cloned().collect(),
        }
    }

    /// 這個成員的身分是**怎麼來的**。查不到就回 `None`——省略欄位，不猜、
    /// 也不冒充「已驗證身分」。
    fn member_identity_strength(&self, party: &Party) -> Option<&'static str> {
        match party.kind {
            // 裝置：由登記表上那條通道自己說（核心不認得傳輸種類）。
            PartyKind::Device => self.device_outbound(&party.id).map(|channel| {
                // 通道的字串是 `&str`，但實際值都是上面的常數；映射回常數才
                // 能給呼叫端 `'static`，也順便擋掉「adapter 自己編一個強度」。
                match channel.identity_strength() {
                    IDENTITY_STRENGTH_PAIRED_TOKEN => IDENTITY_STRENGTH_PAIRED_TOKEN,
                    IDENTITY_STRENGTH_DEVICE_LINK => IDENTITY_STRENGTH_DEVICE_LINK,
                    _ => IDENTITY_STRENGTH_UNKNOWN,
                }
            }),
            PartyKind::HumanSurface if party == &desktop_party() => {
                Some(IDENTITY_STRENGTH_HOST_SURFACE)
            }
            _ => None,
        }
    }

    /// §10 diagnostics（不含 token、路徑、原始 payload）。
    pub fn character_session_diagnostics_value(&self) -> DomainResult<Value> {
        let host = self.session_host()?;
        let diagnostics = host.session().diagnostics();
        let store = host.store().status();
        let members: Vec<Value> = diagnostics
            .members
            .iter()
            .map(|member| {
                let mut item = json!({
                    "party": member.party,
                    "role": member.role,
                    "presence": member.presence.as_str(),
                    "lastSeenAt": member.last_seen_at,
                    // 協商為 unsupported 的 intent 名（沒有就是空陣列）。這是
                    // §11「部分能力目前不可用」的唯一真實來源；協商結果的其餘細節
                    // 仍是 host 私有，不外洩。
                    "unsupportedIntents": member.unsupported_intents,
                });
                // 這個成員的身分是怎麼來的（選填）。三種來源的強度不同——
                // 桌面 host surface、已配對 iPhone 的 per-device token、宣告式
                // 裝置線的傳輸層 hello——把它們講成同一件事就是不誠實。
                // 查不到就**省略**這個欄位，不猜。
                if let Some(strength) = self.member_identity_strength(&member.party) {
                    item["identityStrength"] = Value::String(strength.to_string());
                }
                item
            })
            .collect();
        Ok(json!({
            "sessionId": diagnostics.session_id,
            "sessionEpoch": diagnostics.epoch,
            "revision": diagnostics.revision,
            "sequence": diagnostics.sequence,
            "members": members,
            "counters": diagnostics.counters,
            "eventLog": {"len": diagnostics.event_log_len, "cap": diagnostics.event_log_cap},
            "storeNote": host.load_note(),
            // 持久化的誠實狀態（M1 §2.3）。全部是計數與固定文字：不含路徑、不回顯輸入。
            // 既有欄位不動，這是**新增的選填欄位**。
            "store": {
                "format": store.format,
                // 這一輪把檔案從哪個格式遷過來（沒有遷移＝null）。遷移不是重建：
                // `storeNote` 保持 v0.6.0 的語意（只在 session 真的被重建時非 null）。
                "migratedFrom": host.migration().map(|(from, _)| from),
                "migrationNote": host.migration().map(|(_, note)| note),
                "lastPersistedRevision": store.last_persisted_revision,
                "persistFailures": store.persist_failures,
                "skippedStale": store.skipped_stale,
                "parked": store.parked,
                "lastPersistError": store.last_persist_error,
                "note": store.note,
            },
        }))
    }

    /// 目前線上的裝置成員（廣播對象）。
    fn character_session_online_devices(&self) -> Vec<Party> {
        let Some(host) = self.character_session.as_ref() else {
            return Vec::new();
        };
        host.session()
            .members()
            .into_iter()
            .filter(|member| {
                member.party.kind == PartyKind::Device && member.presence == Presence::Online
            })
            .map(|member| member.party)
            .collect()
    }

    // ------------------------------------------------------------------
    // Use cases
    // ------------------------------------------------------------------

    /// 加入或重新協商（capability）。
    pub async fn character_session_join(
        &self,
        party: Party,
        announcement: &CapabilityAnnouncement,
    ) -> DomainResult<(Envelope, Envelope)> {
        let host = self.session_host()?;
        // 宣告的名稱是外部輸入：協商結果（含 `unsupported`）會留在成員紀錄裡，
        // 筆數不夾住的話一則 32 KiB 的 payload 就能塞進上千筆。真實的 renderer
        // 只宣告個位數（`HOST_INTENTS` 4 個、`HOST_INPUTS` 2 個）。
        if announcement.inputs.len() > MAX_ANNOUNCED_NAMES
            || announcement.intents.len() > MAX_ANNOUNCED_NAMES
        {
            return Err(DomainError::Validation(format!(
                "capability rejected: at most {MAX_ANNOUNCED_NAMES} intents and inputs"
            )));
        }
        let now = Utc::now();
        let joined = {
            let mut session = host.session();
            session
                .join(party.clone(), announcement, now)
                .map_err(|e| DomainError::Validation(format!("capability rejected: {e}")))?
        };
        if party == desktop_party() {
            *host.desktop_presence() = Some(Presence::Online);
        }
        self.character_session_apply(joined.outputs).await;
        Ok((joined.capability_envelope, joined.snapshot_envelope))
    }

    /// 離開（撤銷、關閉視窗）。冪等。
    pub async fn character_session_leave(&self, party: &Party) {
        let Some(host) = self.character_session.as_ref() else {
            return;
        };
        let outputs = {
            let mut session = host.session();
            session.leave(party, Utc::now())
        };
        if party == &desktop_party() {
            *host.desktop_presence() = None;
        }
        self.character_session_apply(outputs).await;
    }

    /// presence 變化（連上／斷線）。只在真的變化時呼叫。
    pub async fn character_session_presence(&self, party: &Party, presence: Presence) {
        let Some(host) = self.character_session.as_ref() else {
            return;
        };
        let outputs = {
            let mut session = host.session();
            session.presence(party, presence, Utc::now())
        };
        if party == &desktop_party() {
            *host.desktop_presence() = Some(presence);
        }
        self.character_session_apply(outputs).await;
    }

    /// **存活證明**：這個成員剛剛送來一則已驗證身分的訊息。
    ///
    /// AIP frame 走 `submit` 時 session 自己就會記下來；這個入口是給**舊協定**用的
    /// （iOS App 目前只送 v1 的 `status` 心跳，還沒送 AIP heartbeat）。與
    /// [`Runtime::character_session_presence`] 的差別：`lastSeenAt` 走投影格線，
    /// 所以每 30 秒一則的心跳不會每次都製造一個 revision 與一次廣播。
    /// 不是成員（沒協商過的舊 App）就什麼都不做。
    pub async fn character_session_touch_presence(&self, party: &Party) {
        let Some(host) = self.character_session.as_ref() else {
            return;
        };
        let outputs = {
            let mut session = host.session();
            session.note_alive_party(party, Utc::now())
        };
        if party == &desktop_party() {
            *host.desktop_presence() = Some(Presence::Online);
        }
        self.character_session_apply(outputs).await;
    }

    /// §8 安全管線：一則外部訊息 → 一則 result（host 自己決定要不要送出去）。
    pub async fn character_session_submit(
        &self,
        envelope: Envelope,
        bound_identity: &Party,
    ) -> DomainResult<Submission> {
        let host = self.session_host()?;
        let submission = {
            let mut session = host.session();
            session.submit(envelope, bound_identity, Utc::now())
        };
        self.character_session_apply(submission.outputs.clone())
            .await;
        Ok(submission)
    }

    /// Runtime 的可信真相事實（同步入口：呼叫端多半是同步的投影點）。
    /// 狀態變更是同步完成的（順序確定），只有派送交給背景任務。
    pub fn character_session_submit_runtime(&self, fact: RuntimeFact, correlation: Option<String>) {
        let Some(host) = self.character_session.as_ref() else {
            return;
        };
        let outputs = {
            let mut session = host.session();
            session.submit_runtime(fact, correlation, Utc::now())
        };
        self.character_session_dispatch_later(outputs);
    }

    /// `agent.session.state` → Session 真相（只轉錄，不推論）。
    pub(crate) fn character_session_note_agent_state(&self, session_id: &str, state: &str) {
        if self.character_session.is_none() {
            return;
        }
        let fact = if state == "verified" {
            // `verified` 只能從人類驗證路徑來（`verify_agent_session`）。
            RuntimeFact::TaskVerified {
                correlation_id: session_id.to_string(),
            }
        } else {
            match crate::character::session_projection(state) {
                Some((_, truth)) => RuntimeFact::TaskState {
                    truth,
                    correlation_id: Some(session_id.to_string()),
                },
                None => return,
            }
        };
        self.character_session_submit_runtime(fact, Some(session_id.to_string()));
    }

    /// §6 resume。
    pub async fn character_session_resume(
        &self,
        party: &Party,
        last_revision: u64,
        last_sequence: u64,
        epoch: u64,
    ) -> DomainResult<Resume> {
        let host = self.session_host()?;
        let resume = {
            let mut session = host.session();
            session.resume(party, last_revision, last_sequence, epoch, Utc::now())
        };
        // host 的進度**落後**成員時，session 自己已經在 snapshot 上標了 `reason`
        // （`session-reset`＝真的重建過、epoch 也換了；`recovery`＝同一個 session、
        // epoch 不變，host 就是比對方舊）。Runtime 這一層以前會把後者硬改寫成
        // `session-reset`——但 AIP §7 的 reset 例外要求 epoch **不同**，同 epoch 的
        // `session-reset` 一樣會被接收端忽略，所以那個改寫沒有效果，只是多說了一句
        // 不實的「session 被重建了」。現在原樣轉交（AIP 1.0 接收端澄清規則 6）。
        Ok(resume)
    }

    /// `state{kind:"snapshot"}`（消耗一個 sequence）。
    pub async fn character_session_snapshot_envelope(&self, to: &Party) -> DomainResult<Envelope> {
        let host = self.session_host()?;
        let envelope = {
            let mut session = host.session();
            session.snapshot_envelope(to, Utc::now())
        };
        Ok(envelope)
    }

    /// 桌面視窗（可信 host surface）在 `/v1/character/hello` 協商成功後加入 session。
    pub(crate) async fn character_session_join_desktop(&self, reduced_motion: bool) {
        if self.character_session.is_none() {
            return;
        }
        let mut features = Map::new();
        features.insert("reducedMotion".into(), Value::Bool(reduced_motion));
        let announcement = CapabilityAnnouncement {
            spec_versions: vec![SPEC_VERSION.to_string()],
            role: Some(MemberRole::HostRenderer),
            profiles: vec![interaction_session::PROFILE.to_string()],
            sync_classes: vec![SyncClass::Semantic],
            intents: HOST_INTENTS.iter().map(|s| s.to_string()).collect(),
            inputs: HOST_INPUTS.iter().map(|s| s.to_string()).collect(),
            features,
            limits: None,
            extra: Map::new(),
        };
        if let Err(error) = self
            .character_session_join(desktop_party(), &announcement)
            .await
        {
            tracing::warn!(%error, "desktop surface did not join the character session");
            return;
        }
        // Reduced Motion 的主人是視窗回報的值；session 只轉錄。
        self.character_session_submit_runtime(RuntimeFact::ReducedMotion(reduced_motion), None);
    }

    /// 週期維護（沿用既有 watchdog sweep，不新開任務）。
    pub(crate) async fn character_session_tick(&self) {
        self.character_session_tick_at(Utc::now()).await;
    }

    /// 注入時間的維護：桌面 presence、reacting 逾時、presence 逾時、過期 intent、
    /// 離線太久的成員清除。
    pub async fn character_session_tick_at(&self, now: Timestamp) {
        let Some(host) = self.character_session.as_ref() else {
            return;
        };
        let desktop_online = self.presentation.connected(now);
        let mut outputs = Vec::new();
        {
            let mut session = host.session();
            let desktop = desktop_party();
            if session
                .members()
                .iter()
                .any(|member| member.party == desktop)
            {
                let desired = if desktop_online {
                    Presence::Online
                } else {
                    Presence::Offline
                };
                let mut last = host.desktop_presence();
                if *last != Some(desired) {
                    outputs.extend(session.presence(&desktop, desired, now));
                    *last = Some(desired);
                }
            }
            outputs.extend(session.tick(now));
            // 離線太久的成員不留在名單上（幽靈成員＝假的「已連接」），但清除的門檻
            // 必須**比 presence 逾時晚**：兩者相同時，`tick` 剛把成員標成 Offline，
            // 同一輪就把它 leave 掉，§11 的「iPhone 暫時離線」在逾時路徑上一個 tick
            // 都活不過，UI 直接從「已連接」跳到「沒有裝置」。
            let eviction = ChronoDuration::milliseconds(
                session
                    .config()
                    .presence_timeout_ms
                    .saturating_mul(MEMBER_EVICTION_TIMEOUT_FACTOR),
            );
            let stale: Vec<Party> = session
                .members()
                .into_iter()
                .filter(|member| {
                    member.presence == Presence::Offline
                        && now.signed_duration_since(member.last_seen_at) >= eviction
                })
                .map(|member| member.party)
                .collect();
            for party in stale {
                if party == desktop {
                    *host.desktop_presence() = None;
                }
                outputs.extend(session.leave(&party, now));
            }
        }
        self.character_session_apply(outputs).await;
    }

    /// 關機：把最後的快照落地（重啟才續接得到 revision）。
    pub(crate) fn character_session_persist_now(&self) {
        let Some(host) = self.character_session.as_ref() else {
            return;
        };
        let snapshot = host.session().snapshot();
        match host.store().save(&snapshot) {
            Ok(SaveOutcome::Written) => {}
            Ok(outcome) => {
                tracing::debug!(?outcome, "the shutdown snapshot was not written");
            }
            Err(error) => {
                tracing::warn!(%error, "character session snapshot was not persisted on shutdown");
            }
        }
    }

    // ------------------------------------------------------------------
    // 裝置 binding（`{"type":"aip","envelope":…}`）：iPhone wss、宣告式裝置線
    // ——**同一條**入口，差別只在呼叫端傳進來的 `DeviceOrigin`。
    // ------------------------------------------------------------------

    /// 一台**已通過該傳輸自己的准入閘門**的裝置送來的一則 `aip` frame。
    /// 回傳要送回去的 envelope。
    ///
    /// `origin` 是呼叫端（iPhone wss 迴圈／宣告式裝置線）對「這一則從哪條線
    /// 進來、身分是怎麼來的」的陳述：核心不認得傳輸種類，稽核的 `transport`
    /// 一律由它提供。
    pub(crate) async fn character_session_device_frame(
        &self,
        device_id: &str,
        origin: DeviceOrigin<'_>,
        frame: &Value,
    ) -> AipFrameOutcome {
        let now = Utc::now();
        let party = Party::device(device_id);
        let causation = frame
            .get("envelope")
            .and_then(|e| e.get("messageId"))
            .and_then(Value::as_str)
            .filter(|id| {
                !id.is_empty()
                    && id.chars().count() <= interaction_aip::limits::MAX_ID_CHARS
                    && !id.chars().any(|c| c.is_control() || c.is_whitespace())
            })
            .map(str::to_string);

        if self.character_session.is_none() {
            return AipFrameOutcome::reply(self.character_session_error(
                ErrorCode::UnsupportedCapability,
                SESSION_DISABLED_MESSAGE,
                causation,
                now,
            ));
        }
        let Some(raw) = frame.get("envelope") else {
            return AipFrameOutcome::reply(self.character_session_error(
                ErrorCode::SchemaInvalid,
                "the aip frame carries no envelope",
                causation,
                now,
            ));
        };
        let bytes = match serde_json::to_vec(raw) {
            Ok(bytes) => bytes,
            Err(_) => {
                return AipFrameOutcome::reply(self.character_session_error(
                    ErrorCode::SchemaInvalid,
                    "the envelope could not be read",
                    causation,
                    now,
                ))
            }
        };
        let envelope = match Envelope::parse(&bytes) {
            Ok(envelope) => envelope,
            Err(error) => {
                // 只回穩定錯誤碼與固定人話，不回顯輸入內容（§5）。
                return AipFrameOutcome::reply(self.character_session_error(
                    error.code,
                    "the envelope was refused before it reached the session",
                    causation,
                    now,
                ));
            }
        };
        // 未知 major／未知 message type：不執行、不猜。
        if let Err(error) = negotiate_version(&envelope.spec_version) {
            return AipFrameOutcome::reply(self.character_session_error(
                error.code,
                "this runtime speaks aip/1.x only",
                causation.clone(),
                now,
            ));
        }
        if !envelope.message_type.is_known() {
            return AipFrameOutcome::reply(self.character_session_error(
                ErrorCode::UnsupportedMessageType,
                "messageType is not one of the 12 known AIP message types",
                causation.clone(),
                now,
            ));
        }
        // §8 第 1 關：schema／profile 驗證 → payload ≤ 32 KiB、深度、字串長度、
        // id／name 語法。**每一則**都要過，不分 message type：非成員的第一則
        // capability 走的是 join（不經 `CharacterSession::gate`），少了這一關就等於
        // 每台已配對 iPhone 的第一則訊息只剩 64 KiB 整包上限與身分綁定。
        // 這一關也讓後面的稽核與回覆只可能帶到有界、合法語法的字串。
        if let Err(error) = envelope.validate() {
            let _ = self.store.audit(
                "aip.rejected",
                "runtime",
                &json!({
                    "transport": origin.transport,
                    "identityStrength": origin.identity_strength,
                    "stage": "profile-validation",
                    "code": error.code.as_str(),
                }),
            );
            return AipFrameOutcome::reply(self.character_session_error(
                error.code,
                "the envelope did not satisfy the aip profile limits",
                causation.clone(),
                now,
            ));
        }
        // 宣稱不是身分（§5）：不符一律拒絕並稽核，不得修正後執行。
        if let IdentityDecision::Reject { .. } = bind_identity(&party, &envelope.source) {
            // 稽核欄位一律夾住長度：稽核不截斷、不過期，攻擊者可控的字串不得
            // 無界寫進去（§8 已經先擋過一次，這裡不依賴呼叫順序）。
            let _ = self.store.audit(
                "aip.identity-mismatch",
                "runtime",
                &json!({
                    "transport": origin.transport,
                    "identityStrength": origin.identity_strength,
                    "boundKind": "device",
                    "claimedKind": audit_party_kind(&envelope.source.kind),
                    "name": audit_snippet(&envelope.name),
                }),
            );
            return AipFrameOutcome::reply(self.character_session_error(
                ErrorCode::IdentityMismatch,
                "source does not match the paired identity of this connection",
                causation.clone(),
                now,
            ));
        }

        match envelope.message_type {
            MessageType::Capability => {
                self.character_session_device_capability(&party, origin, envelope, now)
                    .await
            }
            MessageType::Query => {
                self.character_session_device_query(&party, envelope, now)
                    .await
            }
            _ => {
                let message_id = envelope.message_id.clone();
                let name = envelope.name.clone();
                let touch_kind = envelope
                    .payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match self.character_session_submit(envelope, &party).await {
                    Ok(submission) => {
                        let applied_touch = (name == EVENT_TOUCH
                            && submission.outcome == Outcome::Applied)
                            .then_some(touch_kind)
                            .flatten();
                        let replies = if submission.reply {
                            vec![serde_json::to_value(&submission.result).unwrap_or(Value::Null)]
                        } else {
                            Vec::new()
                        };
                        AipFrameOutcome {
                            replies,
                            applied_touch,
                        }
                    }
                    Err(_) => AipFrameOutcome::reply(self.character_session_error(
                        ErrorCode::SessionDisabled,
                        SESSION_DISABLED_MESSAGE,
                        Some(message_id),
                        now,
                    )),
                }
            }
        }
    }

    /// 這個 party 目前是不是成員（決定 capability 走 join 還是走安全管線；
    /// 也決定斷線收尾要送 `Reconnecting` 還是什麼都不做）。
    pub(crate) fn character_session_is_member(&self, party: &Party) -> bool {
        self.character_session.as_ref().is_some_and(|host| {
            host.session()
                .members()
                .iter()
                .any(|member| &member.party == party)
        })
    }

    async fn character_session_device_capability(
        &self,
        party: &Party,
        origin: DeviceOrigin<'_>,
        envelope: Envelope,
        now: Timestamp,
    ) -> AipFrameOutcome {
        let causation = envelope.message_id.clone();
        // 已經是成員的重新協商走完整安全管線（速率上限＋去重）：否則一台已配對
        // 的裝置可以用 capability 洪水把 revision 與廣播打成無界成長。
        // 第一次協商（還不是成員）只能走 join——安全管線的第 4 關就是 membership。
        if self.character_session_is_member(party) {
            return match self.character_session_submit(envelope, party).await {
                Ok(submission) => AipFrameOutcome {
                    replies: if submission.reply {
                        vec![serde_json::to_value(&submission.result).unwrap_or(Value::Null)]
                    } else {
                        Vec::new()
                    },
                    applied_touch: None,
                },
                Err(_) => AipFrameOutcome::reply(self.character_session_error(
                    ErrorCode::SessionDisabled,
                    SESSION_DISABLED_MESSAGE,
                    Some(causation),
                    now,
                )),
            };
        }
        // §8 的 `sessionId` 這一關：成員走 `gate` 時比對過，非成員的第一則
        // capability 一樣要比對——否則另一個 session 的 frame 可以直接 join 進來。
        if envelope
            .session_id
            .as_deref()
            .is_some_and(|session_id| session_id != SESSION_ID)
        {
            // 稽核的 transport 由來源說出來：宣告式序列裝置第一次 capability 帶錯
            // sessionId 時，不能被記成「某台 iPhone 出問題」。
            let _ = self.store.audit(
                "aip.rejected",
                "runtime",
                &json!({
                    "transport": origin.transport,
                    "identityStrength": origin.identity_strength,
                    "stage": "session-binding",
                    "code": ErrorCode::NotAMember.as_str(),
                }),
            );
            return AipFrameOutcome::reply(self.character_session_error(
                ErrorCode::NotAMember,
                "the envelope belongs to a different character session",
                Some(causation),
                now,
            ));
        }
        let announcement: CapabilityAnnouncement =
            match serde_json::from_value(envelope.payload.clone()) {
                Ok(announcement) => announcement,
                Err(_) => {
                    return AipFrameOutcome::reply(self.character_session_error(
                        ErrorCode::SchemaInvalid,
                        "the capability announcement could not be read",
                        Some(causation),
                        now,
                    ))
                }
            };
        match self
            .character_session_join(party.clone(), &announcement)
            .await
        {
            Ok((capability, snapshot)) => AipFrameOutcome {
                replies: vec![
                    serde_json::to_value(&capability).unwrap_or(Value::Null),
                    serde_json::to_value(&snapshot).unwrap_or(Value::Null),
                ],
                applied_touch: None,
            },
            Err(error) => {
                let code = match &error {
                    DomainError::Unavailable(_) => ErrorCode::SessionDisabled,
                    _ => ErrorCode::UnsupportedCapability,
                };
                AipFrameOutcome::reply(self.character_session_error(
                    code,
                    "capability negotiation was refused",
                    Some(causation),
                    now,
                ))
            }
        }
    }

    /// `query`：先過安全管線（membership／rate／dedupe），再自行路由到 resume／snapshot。
    /// session crate 刻意不產生 `response`，所以 `response` 由 transport 組。
    async fn character_session_device_query(
        &self,
        party: &Party,
        envelope: Envelope,
        now: Timestamp,
    ) -> AipFrameOutcome {
        let causation = envelope.message_id.clone();
        let name = envelope.name.clone();
        let payload = envelope.payload.clone();
        let submission = match self.character_session_submit(envelope, party).await {
            Ok(submission) => submission,
            Err(_) => {
                return AipFrameOutcome::reply(self.character_session_error(
                    ErrorCode::SessionDisabled,
                    SESSION_DISABLED_MESSAGE,
                    Some(causation),
                    now,
                ))
            }
        };
        if submission.error.is_some() {
            return AipFrameOutcome::reply(submission.result);
        }
        // 去重命中（§8 第 12 關）走的是 `Gate::Duplicate`：`outcome: accepted`、`error: None`，
        // 所以上面那道守衛不會觸發。`accepted{duplicate:true}` 已經是這則訊息的答案，
        // 不得再跑一次 resume／snapshot——那會多消耗一個 sequence（其他成員看到假跳號）、
        // 把 diagnostics 計數器灌大，而且把該回給對方的 duplicate 回覆丟掉（identity-binding-009）。
        if submission
            .result
            .payload
            .get("duplicate")
            .and_then(Value::as_bool)
            == Some(true)
        {
            return AipFrameOutcome::reply(submission.result);
        }
        match name.as_str() {
            NAME_SESSION_RESUME => {
                let last_revision = payload
                    .get("lastRevision")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let last_sequence = payload
                    .get("lastSequence")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let epoch = payload
                    .get("sessionEpoch")
                    .or_else(|| payload.get("epoch"))
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                match self
                    .character_session_resume(party, last_revision, last_sequence, epoch)
                    .await
                {
                    Ok(resume) => AipFrameOutcome::reply(
                        self.character_session_resume_response(party, &causation, resume, now)
                            .await,
                    ),
                    Err(_) => AipFrameOutcome::reply(self.character_session_error(
                        ErrorCode::SessionDisabled,
                        SESSION_DISABLED_MESSAGE,
                        Some(causation),
                        now,
                    )),
                }
            }
            NAME_SESSION_SNAPSHOT_QUERY => {
                match self.character_session_snapshot_envelope(party).await {
                    Ok(snapshot) => AipFrameOutcome::reply(self.character_session_response(
                        party,
                        &causation,
                        snapshot.payload,
                        now,
                    )),
                    Err(_) => AipFrameOutcome::reply(self.character_session_error(
                        ErrorCode::SessionDisabled,
                        SESSION_DISABLED_MESSAGE,
                        Some(causation),
                        now,
                    )),
                }
            }
            _ => {
                // 未知 name：不猜、不執行（§4.1）。
                AipFrameOutcome::reply(self.character_session_result(
                    &causation,
                    Outcome::Rejected,
                    Some(ErrorCode::UnknownName),
                    now,
                ))
            }
        }
    }

    /// `response` 的三種 resume 結果。patches 塞不進 payload 上限時誠實退回 snapshot。
    async fn character_session_resume_response(
        &self,
        party: &Party,
        causation: &str,
        resume: Resume,
        now: Timestamp,
    ) -> Envelope {
        let payload = self.character_session_resume_value(party, resume).await;
        self.character_session_response(party, causation, payload, now)
    }

    /// resume 結果的 payload（HTTP 與 iPhone wss 共用同一個形狀）。
    pub async fn character_session_resume_value(&self, party: &Party, resume: Resume) -> Value {
        match resume {
            Resume::Patches { envelopes } => {
                let patches: Vec<Value> = envelopes.iter().map(patch_item).collect();
                let payload = json!({"kind": "patches", "patches": patches});
                if interaction_aip::envelope::check_payload(&payload).is_ok() {
                    payload
                } else {
                    // 補得起來但塞不進 payload 上限：退回 snapshot（§6：不是錯誤）。
                    match self.character_session_snapshot_envelope(party).await {
                        Ok(snapshot) => snapshot.payload,
                        Err(_) => payload,
                    }
                }
            }
            // snapshot 直接內嵌 `state` 訊息的 payload（同一個形狀，少一層巢狀：
            // AIP §11 的深度上限只有 8，包一層完整 envelope 會超）。
            Resume::Snapshot { envelope } => envelope.payload,
            Resume::EpochMismatch { envelope } => {
                let mut payload = envelope.payload;
                if let Value::Object(map) = &mut payload {
                    map.insert(
                        "reason".into(),
                        Value::String(interaction_session::REASON_SESSION_RESET.to_string()),
                    );
                }
                payload
            }
        }
    }

    fn character_session_response(
        &self,
        to: &Party,
        causation: &str,
        payload: Value,
        now: Timestamp,
    ) -> Envelope {
        Envelope::new(
            MessageType::Response,
            NAME_SESSION_RESUME,
            Party::runtime(),
            transport_message_id(),
            now,
        )
        .with_session(SESSION_ID)
        .with_target(to.clone())
        .with_causation(causation.to_string())
        .with_payload(payload)
    }

    fn character_session_result(
        &self,
        causation: &str,
        outcome: Outcome,
        code: Option<ErrorCode>,
        now: Timestamp,
    ) -> Envelope {
        let mut payload = Map::new();
        payload.insert("status".into(), json!(outcome.as_str()));
        if let Some(code) = code {
            payload.insert("code".into(), json!(code.as_str()));
            payload.insert("retryable".into(), Value::Bool(code.retryable()));
        }
        Envelope::new(
            MessageType::Result,
            NAME_SESSION_RESUME,
            Party::runtime(),
            transport_message_id(),
            now,
        )
        .with_session(SESSION_ID)
        .with_causation(causation.to_string())
        .with_payload(Value::Object(payload))
    }

    fn character_session_error(
        &self,
        code: ErrorCode,
        message: &str,
        causation: Option<String>,
        now: Timestamp,
    ) -> Envelope {
        let payload = serde_json::to_value(ErrorPayload::new(code, message)).unwrap_or(Value::Null);
        let mut envelope = Envelope::new(
            MessageType::Error,
            NAME_SESSION_ERROR,
            Party::runtime(),
            transport_message_id(),
            now,
        )
        .with_session(SESSION_ID)
        .with_payload(payload);
        if let Some(causation) = causation {
            envelope = envelope.with_causation(causation);
        }
        envelope
    }

    // ------------------------------------------------------------------
    // Output 派送
    // ------------------------------------------------------------------

    fn character_session_dispatch_later(&self, outputs: Vec<Output>) {
        if outputs.is_empty() {
            return;
        }
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let runtime = self.clone();
                handle.spawn(async move {
                    runtime.character_session_apply(outputs).await;
                });
            }
            Err(_) => tracing::warn!(
                count = outputs.len(),
                "character session outputs dropped: no async runtime"
            ),
        }
    }

    async fn character_session_apply(&self, outputs: Vec<Output>) {
        for output in outputs {
            match output {
                Output::Send { to, envelope } => self.character_session_send(&to, &envelope).await,
                Output::Broadcast { envelope, except } => {
                    for party in self.character_session_online_devices() {
                        if except.as_ref() == Some(&party) {
                            continue;
                        }
                        self.character_session_send(&party, &envelope).await;
                    }
                    self.publish_character_session_state(&envelope);
                }
                Output::Audit { kind, detail } => {
                    let _ = self.store.audit(&kind, "runtime", &detail);
                }
                Output::Persist(snapshot) => self.character_session_persist(snapshot).await,
                Output::RendererIntent { intent, cpp } => {
                    // `celebrate` 不投影（桌面已由既有 verified-success 表達，不雙播）。
                    if let Some(cpp) = cpp {
                        self.character_dispatch_session_intent(
                            &cpp,
                            &intent.correlation_id,
                            intent.expires_at,
                        );
                    }
                }
            }
        }
    }

    async fn character_session_send(&self, to: &Party, envelope: &Envelope) {
        match to.kind {
            PartyKind::Device => {
                // 型別抹除：核心只問「這台裝置現在有沒有一條送得出去的線」，
                // 不認得 iPhone／serial／mqtt／ble。
                let Some(channel) = self.device_outbound(&to.id) else {
                    // 沒有通道**不是**送到了。只落一行 debug log 的話，
                    // 「這台裝置已加入 session」與「它其實一則狀態都沒收到」
                    // 在畫面上長得一模一樣。
                    let _ = self.store.audit(
                        "aip.outbound-undeliverable",
                        "runtime",
                        &json!({
                            "deviceId": audit_snippet(&to.id),
                            "reason": "no-channel",
                            "name": audit_snippet(&envelope.name),
                        }),
                    );
                    return;
                };
                if let Err(error) = channel.send_aip(envelope).await {
                    // 送不到不等於送到了：不重送、不假裝成功——但也不靜默。
                    let _ = self.store.audit(
                        "aip.outbound-undeliverable",
                        "runtime",
                        &json!({
                            "transport": channel.transport_label(),
                            "deviceId": audit_snippet(&to.id),
                            "reason": audit_snippet(&error.to_string()),
                            "name": audit_snippet(&envelope.name),
                        }),
                    );
                }
            }
            // 桌面可信 host surface：走 SSE（human token）。
            _ => self.publish_character_session_state(envelope),
        }
    }

    fn publish_character_session_state(&self, envelope: &Envelope) {
        let payload = serde_json::to_value(envelope).unwrap_or(Value::Null);
        self.events.publish(RuntimeEvent::new(
            EventType::CharacterSessionState,
            Utc::now(),
            payload,
        ));
    }

    /// 把一份快照落地。**錯誤不得被吞掉**：`spawn_blocking` 的 `Result` 與 `JoinError`
    /// 都要進計數與 diagnostics，否則「持久化一直失敗」在 UI 上與「一切正常」長得一樣。
    async fn character_session_persist(&self, snapshot: Snapshot) {
        let Some(host) = self.character_session.as_ref() else {
            return;
        };
        let store = host.store();
        let revision = snapshot.revision;
        // hot path 不做同步檔案 I/O。
        let joined = tokio::task::spawn_blocking({
            let store = store.clone();
            move || store.save(&snapshot)
        })
        .await;
        match joined {
            Ok(Ok(SaveOutcome::Written)) => {}
            Ok(Ok(outcome)) => {
                tracing::debug!(
                    ?outcome,
                    revision,
                    "the character session snapshot was not written"
                );
            }
            // store 已經在 `save` 裡記過這次失敗（計數＋固定文字），這裡只補 log。
            Ok(Err(error)) => {
                tracing::warn!(%error, revision, "the character session snapshot was not persisted");
            }
            Err(error) => {
                store.note_persist_failure(PERSIST_ERROR_TASK);
                tracing::warn!(%error, revision, "the character session persistence task did not finish");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 兩次持久化可能同時發生（`Output::Persist` 由各自的背景任務派送，`character_session_persist`
    /// 又把它丟到 `spawn_blocking`）。共用一個暫存檔名的話，後開檔的 `truncate` 會截掉前一個
    /// 寫入者已經寫進去的內容，先 rename 出去的就是兩份 JSON 的拼接——下次開機會把它當壞檔
    /// 隔離（epoch+1），所有成員被迫 session-reset。**檔案在任何一個瞬間都必須是完整的一份快照。**
    #[test]
    fn concurrent_persists_never_publish_a_spliced_snapshot() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(JsonSessionStore::new(dir.path().join(SESSION_STORE_FILE)));
        let snapshot = |width: usize| Snapshot {
            format: interaction_session::SNAPSHOT_FORMAT,
            session_id: SESSION_ID.to_string(),
            epoch: 1,
            revision: width as u64,
            sequence: 0,
            state: json!({"filler": "x".repeat(width)}),
            hash: "0".repeat(64),
            at: Utc::now(),
        };
        // 先放一份好的：讀到的東西一定要是「某一份完整快照」，不是「還沒寫」。
        store.save(&snapshot(8)).expect("seed");

        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // 共用的遞增 revision：兩個寫入者都必須真的寫得出去（否則順序 guard 會讓其中一個
        // 一直 SkippedStale，這個測試就不再測到並行寫入了）。
        let next_revision = Arc::new(std::sync::atomic::AtomicU64::new(100));
        let mut writers = Vec::new();
        // 大小差很多的兩份快照：拼接起來一定是壞 JSON，而不是碰巧還讀得回來。
        for width in [8usize, 60_000] {
            let store = store.clone();
            let stop = stop.clone();
            let next_revision = next_revision.clone();
            writers.push(std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let mut snapshot = snapshot(width);
                    snapshot.revision =
                        next_revision.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // 寫失敗是可接受的（另一個寫入者贏了）；讓別人讀到壞檔不可接受。
                    let _ = store.save(&snapshot);
                }
            }));
        }

        let mut spliced = 0usize;
        for _ in 0..2_000 {
            match store.load(SESSION_ID) {
                Ok(Some(_)) => {}
                // 讀不到（rename 之間的空窗）不是拼接；讀得到但解不開才是。
                Ok(None) => {}
                Err(PortError::Corrupt) => spliced += 1,
                Err(_) => {}
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for writer in writers {
            writer.join().expect("writer thread");
        }
        assert_eq!(
            spliced, 0,
            "並行持久化把兩份快照拼在一起了（{spliced} 次讀到壞檔）"
        );
    }

    /// 一台從來沒被隔離過的桌面：epoch 就是 1、revision 已經跑了一陣子。
    /// 它的檔案壞成「完全解不開、連 `"epoch"` 字樣都沒有」時，舊實作以
    /// `salvaged_epoch()==0` 為底 +1，重建的 session 又是 epoch 1——與成員記得的
    /// 完全相同。成員 resume 不會走 `EpochMismatch`，收到的是 revision 比自己小的
    /// 普通 snapshot，再被自己的 rollback 防護忽略，兩邊都以為「已同步」。
    #[test]
    fn a_rebuilt_session_never_reuses_the_epoch_members_remember() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = JsonSessionStore::new(dir.path().join(SESSION_STORE_FILE));
        store
            .save(&Snapshot {
                format: interaction_session::SNAPSHOT_FORMAT,
                session_id: SESSION_ID.to_string(),
                epoch: 1,
                revision: 50,
                sequence: 90,
                state: json!({}),
                hash: "0".repeat(64),
                at: Utc::now(),
            })
            .expect("seed a healthy snapshot");

        // 壞法：整份內容變成不含 "epoch" 字樣的亂碼（清成 NUL／被別的東西覆寫）。
        std::fs::write(dir.path().join(SESSION_STORE_FILE), "\0\0\0garbage\0\0")
            .expect("corrupt the file");

        let host = CharacterSessionHost::open(dir.path(), Utc::now());
        let epoch = host.session().epoch();
        assert!(
            epoch > 1,
            "重建的 session 不得沿用成員記得的 epoch（拿到 {epoch}）"
        );
        assert!(
            dir.path().join("character-session.json.corrupt").exists(),
            "壞檔要留證據"
        );
        assert_eq!(
            host.load_note(),
            Some(STORE_NOTE_UNUSABLE),
            "載入異常必須誠實顯示"
        );
    }

    /// epoch 的記憶與快照分開存：快照救得回 4、另外記著的是 9 時，下一個 epoch
    /// 要從**大的那個**續接，否則重複用過的號碼又會與成員記得的撞號。
    #[test]
    fn the_next_epoch_continues_from_whichever_source_is_further_ahead() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = JsonSessionStore::new(dir.path().join(SESSION_STORE_FILE));
        store.remember_epoch(9);
        std::fs::write(
            dir.path().join(SESSION_STORE_FILE),
            "{\"epoch\": 4, truncated",
        )
        .expect("seed");
        assert_eq!(store.next_epoch(), 10);
        // 記憶自己也要往前走（下一次重建不得再發同一個號碼）。
        assert_eq!(store.remembered_epoch(), 10);
    }

    /// 讀不到（權限／EIO／fd 用盡）不等於壞掉：一次暫時性 I/O 失敗不得把一份
    /// 可能完好的快照改名丟棄，也不得當場覆寫掉。
    /// 這裡用「目錄佔住檔名」製造一個可攜的非 NotFound 讀取錯誤。
    #[test]
    fn a_transient_read_failure_never_throws_away_the_stored_snapshot() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SESSION_STORE_FILE);
        std::fs::create_dir(&path).expect("occupy the path with a directory");

        let host = CharacterSessionHost::open(dir.path(), Utc::now());
        assert_eq!(host.load_note(), Some(STORE_NOTE_UNREADABLE));
        assert!(
            !dir.path().join("character-session.json.corrupt").exists(),
            "讀不到不是壞掉：不得隔離"
        );
        assert!(path.is_dir(), "原本的紀錄必須原封不動留著");
        assert!(
            host.session().epoch() >= 1,
            "仍然要有一個可用的 session（誠實顯示 storeNote）"
        );
    }

    /// 測試用的快照（store 層只看 `epoch`／`revision`／JSON 是否完整）。
    fn stored_snapshot(epoch: u64, revision: u64) -> Snapshot {
        Snapshot {
            format: interaction_session::SNAPSHOT_FORMAT,
            session_id: SESSION_ID.to_string(),
            epoch,
            revision,
            sequence: revision,
            state: json!({}),
            hash: "0".repeat(64),
            at: Utc::now(),
        }
    }

    /// 持久化沒有順序保證：`Output::Persist` 由各自的 `character_session_apply` 併發派送，
    /// 每一次又各自 `spawn_blocking`，所以「先 save(rev 6) 再 save(rev 5)」是真的會發生的。
    /// 沒有 guard 的話檔案就停在 rev 5，重啟後 host 從一個**倒退過的** revision 續接。
    #[test]
    fn a_stale_snapshot_never_overwrites_a_newer_one_on_disk() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = JsonSessionStore::new(dir.path().join(SESSION_STORE_FILE));
        assert_eq!(
            store.save(&stored_snapshot(3, 6)).expect("newer"),
            SaveOutcome::Written
        );
        assert_eq!(
            store.save(&stored_snapshot(3, 5)).expect("older"),
            SaveOutcome::SkippedStale
        );
        assert_eq!(
            store
                .load(SESSION_ID)
                .expect("load")
                .map(|s| (s.epoch, s.revision)),
            Some((3, 6)),
            "落後的快照不得覆寫已落地的進度"
        );
        // epoch 比 revision 優先：重建過的 session 即使 revision 較小也是比較新的。
        assert_eq!(
            store.save(&stored_snapshot(4, 1)).expect("new epoch"),
            SaveOutcome::Written
        );
        assert_eq!(
            store
                .load(SESSION_ID)
                .expect("load")
                .map(|s| (s.epoch, s.revision)),
            Some((4, 1))
        );
    }

    /// 讀不到（權限／EIO）時 host 只跳過**開機那一次** save；之後每一次 persist 仍然
    /// rename 覆蓋那份讀不到的檔案，於是一次暫時性故障還是變成永久資料遺失。
    /// store 必須 parked：一律不寫，而不是「這次不寫」。
    #[test]
    fn a_file_that_could_not_be_read_is_never_overwritten_by_a_later_persist() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SESSION_STORE_FILE);
        let precious = "{\"marker\":\"precious\"}";
        std::fs::write(&path, precious).expect("seed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
                .expect("chmod 000");
        }
        if std::fs::read_to_string(&path).is_ok() {
            // root 讀得到 0o000 的檔案：這個測試的前提不成立，誠實跳過而不是假裝通過。
            eprintln!("skipped: this process can read a 0o000 file (running as root?)");
            return;
        }

        let host = CharacterSessionHost::open(dir.path(), Utc::now());
        assert_eq!(host.load_note(), Some(STORE_NOTE_UNREADABLE));
        let snapshot = host.session().snapshot();
        assert_eq!(
            host.store().save(&snapshot).expect("save must not error"),
            SaveOutcome::SkippedParked,
            "讀不到的檔案不得被後續的 persist 蓋掉"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("chmod back");
        }
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            precious,
            "一次暫時性讀取失敗不得變成永久資料遺失"
        );
    }

    /// 有界讀取：快照正常是幾 KiB，超過上限的檔案不整個讀進記憶體，直接判壞檔。
    #[test]
    fn an_oversized_session_file_is_corrupt_and_never_read_into_memory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SESSION_STORE_FILE);
        let mut body = serde_json::to_string(&stored_snapshot(1, 1)).expect("serialize");
        body.push_str(&" ".repeat((MAX_SNAPSHOT_BYTES + 1) as usize));
        std::fs::write(&path, &body).expect("seed an oversized file");
        let store = JsonSessionStore::new(path);
        assert_eq!(store.load(SESSION_ID), Err(PortError::Corrupt));
    }

    /// 更新版本寫的快照：**不是**壞檔。store 回 `FutureFormat`，host 保留檔案、
    /// 不隔離、不覆寫，並把 store parked。
    #[test]
    fn a_newer_format_is_preserved_parked_and_never_overwritten() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SESSION_STORE_FILE);
        let mut future = serde_json::to_value(stored_snapshot(7, 40)).expect("value");
        future["format"] = json!(99);
        future["somethingNewer"] = json!({"weCannotUnderstandThis": true});
        let body = serde_json::to_string_pretty(&future).expect("serialize");
        std::fs::write(&path, &body).expect("seed");

        assert_eq!(
            JsonSessionStore::new(path.clone()).load(SESSION_ID),
            Err(PortError::FutureFormat),
            "比較新的格式不是損毀"
        );

        let host = CharacterSessionHost::open(dir.path(), Utc::now());
        assert_eq!(host.load_note(), Some(STORE_NOTE_FUTURE_FORMAT));
        assert!(
            !dir.path().join("character-session.json.corrupt").exists(),
            "不得隔離"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            body,
            "不得覆寫更新版本寫的快照"
        );
        let status = host.store().status();
        assert!(status.parked, "store 必須 parked");
        assert_eq!(status.note, Some(STORE_NOTE_FUTURE_FORMAT));
        assert_eq!(status.last_persisted_revision, None);
        // 之後每一次 persist 也一樣拒絕。
        assert_eq!(
            host.store().save(&host.session().snapshot()).expect("save"),
            SaveOutcome::SkippedParked
        );
        assert_eq!(std::fs::read_to_string(&path).expect("read back"), body);
        assert!(
            host.session().epoch() > 7,
            "記憶體模式的 session 不得沿用成員記得的 epoch"
        );
    }

    /// 舊格式（v0.6.0：沒有 `format` 鍵，成員也缺 `unsupportedIntents`）必須**遷移**
    /// 得回來，而不是被判 HashMismatch 後隔離（v0.6.0 已知限制 #21）。原檔留一份備份。
    #[test]
    fn an_older_format_file_is_migrated_and_the_original_is_backed_up() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SESSION_STORE_FILE);
        let original = legacy_format0_body();
        std::fs::write(&path, &original).expect("seed");

        let host = CharacterSessionHost::open(dir.path(), Utc::now());
        assert_eq!(host.migration(), Some((0, STORE_NOTE_MIGRATED)));
        assert_eq!(host.load_note(), None, "遷移不是重建，storeNote 必須留空");
        assert!(
            !dir.path().join("character-session.json.corrupt").exists(),
            "舊格式不是損毀，不得隔離"
        );
        let backup = dir.path().join("character-session.json.pre-format-0");
        assert_eq!(
            std::fs::read_to_string(&backup).expect("備份必須存在"),
            original
        );
        // 落地的是新格式，而且續接得到 epoch／成員。
        let stored = JsonSessionStore::new(path)
            .load(SESSION_ID)
            .expect("load")
            .expect("present");
        assert_eq!(stored.format, interaction_session::SNAPSHOT_FORMAT);
        assert_eq!(stored.epoch, 4, "遷移不重建 session");
        assert_eq!(host.session().state().members().len(), 1);
        assert!(host.session().state().members()[0]
            .unsupported_intents
            .is_empty());
        assert_eq!(
            host.store().status().last_persisted_revision,
            Some(stored.revision)
        );
    }

    /// 遷移中斷（備份寫不出來）：原檔仍在、不隔離、不覆寫，並誠實標注這次沒有遷移。
    #[test]
    fn a_migration_that_cannot_back_up_leaves_the_original_alone() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SESSION_STORE_FILE);
        let original = legacy_format0_body();
        std::fs::write(&path, &original).expect("seed");
        // 用目錄佔住備份檔名：copy 一定失敗。
        std::fs::create_dir(dir.path().join("character-session.json.pre-format-0"))
            .expect("occupy the backup path");

        let host = CharacterSessionHost::open(dir.path(), Utc::now());
        assert_eq!(host.migration(), Some((0, STORE_NOTE_MIGRATION_DEFERRED)));
        assert_eq!(host.load_note(), None, "session 還原成功，沒有重建");
        assert!(
            !dir.path().join("character-session.json.corrupt").exists(),
            "不得隔離"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            original,
            "備份失敗就不得動原檔"
        );
        assert!(host.store().status().parked);
    }

    /// 遷移中斷的**另一半**：備份做出來了，但新格式寫不進去。
    ///
    /// 這兩件事在使用者眼裡是不同的故障（一個要去看備份路徑被誰佔住了，一個是
    /// 磁碟／權限問題），診斷文字不得共用一句「備份做不出來」。
    #[cfg(unix)]
    #[test]
    fn a_migration_that_cannot_write_says_so_instead_of_blaming_the_backup() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SESSION_STORE_FILE);
        let original = legacy_format0_body();
        std::fs::write(&path, &original).expect("seed");
        // 備份檔先建出來（可寫）：`std::fs::copy` 只需要對**這個檔**有寫權限。
        let backup = dir.path().join("character-session.json.pre-format-0");
        std::fs::write(&backup, "").expect("seed the backup path");
        // 目錄改成唯讀：`write_owner_only` 要在同一個目錄建暫存檔，一定失敗。
        let restore = std::fs::metadata(dir.path()).expect("meta").permissions();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500))
            .expect("read-only state dir");

        let host = CharacterSessionHost::open(dir.path(), Utc::now());

        std::fs::set_permissions(dir.path(), restore).expect("restore permissions");

        assert_eq!(
            host.migration(),
            Some((0, STORE_NOTE_MIGRATION_WRITE_FAILED)),
            "備份成功、寫入失敗＝另一種故障，不得沿用「備份做不出來」那一句"
        );
        assert_eq!(host.load_note(), None, "session 還原成功，沒有重建");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            original,
            "rename 從來沒發生：原檔仍是舊格式"
        );
        assert_eq!(
            std::fs::read_to_string(&backup).expect("read the backup"),
            original,
            "備份這一步是成功的"
        );
        assert!(
            !host.store().status().parked,
            "備份已經在，重試是安全的：這條路徑刻意不 park"
        );
        assert_eq!(
            host.store().status().last_persist_error,
            Some(PERSIST_ERROR_WRITE)
        );
    }

    /// 持久化失敗必須被計數並以固定文字回報（不含路徑、不含 I/O 細節）。
    #[test]
    fn persist_failures_are_counted_and_reported_without_paths() {
        let dir = tempfile::tempdir().expect("temp dir");
        let state_dir = dir.path().join("state");
        std::fs::create_dir_all(&state_dir).expect("state dir");
        let store = JsonSessionStore::new(state_dir.join(SESSION_STORE_FILE));
        assert_eq!(
            store.save(&stored_snapshot(1, 1)).expect("first"),
            SaveOutcome::Written
        );
        assert_eq!(store.status().last_persisted_revision, Some(1));
        assert_eq!(store.status().persist_failures, 0);
        assert_eq!(store.status().note, None);

        // 讓寫入失敗：把 state 目錄換成一個檔案，`create_dir_all` 與 open 都會失敗。
        std::fs::remove_dir_all(&state_dir).expect("drop the state dir");
        std::fs::write(&state_dir, "not a directory").expect("occupy the state dir");
        assert!(store.save(&stored_snapshot(1, 2)).is_err());
        let status = store.status();
        assert_eq!(status.persist_failures, 1);
        assert_eq!(status.last_persist_error, Some(PERSIST_ERROR_WRITE));
        assert_eq!(status.note, Some(STORE_NOTE_PERSIST_FAILING));
        assert_eq!(
            status.last_persisted_revision,
            Some(1),
            "失敗不得讓 lastPersistedRevision 假裝前進"
        );
        for text in [
            status.last_persist_error.unwrap_or_default(),
            status.note.unwrap_or_default(),
        ] {
            assert!(!text.contains(dir.path().to_string_lossy().as_ref()));
            assert!(!text.contains('/'));
        }

        // JoinError 之類「連結果都拿不到」的情況一樣要計數，不得靜默。
        store.note_persist_failure(PERSIST_ERROR_TASK);
        assert_eq!(store.status().persist_failures, 2);
        assert_eq!(store.status().last_persist_error, Some(PERSIST_ERROR_TASK));
    }

    /// 這條路徑刻意**不**隔離檔案，所以 note 不得宣稱它被隔離了（誠實階梯）。
    #[test]
    fn the_unreadable_note_does_not_claim_the_file_was_quarantined() {
        assert!(!STORE_NOTE_UNREADABLE.contains("quarantine"));
        assert!(STORE_NOTE_UNREADABLE.contains("left untouched"));
        assert!(STORE_NOTE_FUTURE_FORMAT.contains("will not overwrite"));
        // 反面：真的會隔離的那一條保留原本的說法。
        assert!(STORE_NOTE_UNUSABLE.contains("quarantined"));
    }

    /// v0.6.0 會寫出來的那份檔案：沒有 `format` 鍵，成員也沒有 `unsupportedIntents`。
    fn legacy_format0_body() -> String {
        let mut session = CharacterSession::new(
            SessionConfig {
                session_id: SESSION_ID.to_string(),
                ..SessionConfig::default()
            },
            4,
            Utc::now(),
        );
        let phone = Party::device("iphone-legacy");
        let announcement = CapabilityAnnouncement {
            spec_versions: vec![SPEC_VERSION.to_string()],
            role: Some(MemberRole::RemoteRenderer),
            profiles: vec![interaction_session::PROFILE.to_string()],
            sync_classes: vec![SyncClass::Semantic],
            intents: HOST_INTENTS.iter().map(|s| s.to_string()).collect(),
            inputs: HOST_INPUTS.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        session
            .join(phone, &announcement, Utc::now())
            .expect("join");
        let mut value = serde_json::to_value(session.snapshot()).expect("value");
        value.as_object_mut().expect("object").remove("format");
        for member in value["state"]["members"]
            .as_array_mut()
            .expect("members array")
        {
            member
                .as_object_mut()
                .expect("member object")
                .remove("unsupportedIntents");
        }
        value["hash"] = json!(interaction_session::state_hash(&value["state"]));
        serde_json::to_string(&value).expect("serialize")
    }

    #[test]
    fn a_truncated_session_file_still_gives_up_its_epoch() {
        assert_eq!(salvaged_epoch(r#"{"sessionId":"x","epoch":7,"revi"#), 7);
        assert_eq!(salvaged_epoch(r#"{"epoch": 12, "revision": 3}"#), 12);
        assert_eq!(salvaged_epoch("not json at all"), 0);
        assert_eq!(salvaged_epoch(r#"{"epoch":"nine"}"#), 0);
    }

    /// diagnostics 的 `storeNote` 只能是固定文字：壞檔的反序列化錯誤會回顯檔案內容，
    /// I/O 錯誤會帶檔案系統路徑，兩者都不得進到任何 API 回應。
    #[test]
    fn store_note_never_carries_error_details_or_paths() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SESSION_STORE_FILE);
        std::fs::write(&path, "{\"epoch\": 4, \"secret-looking-content\": ")
            .expect("seed corrupt file");
        let host = CharacterSessionHost::open(dir.path(), Utc::now());
        let note = host.load_note().expect("a corrupt store must be reported");
        assert!(
            note == STORE_NOTE_UNUSABLE || note == STORE_NOTE_UNREADABLE,
            "note must be one of the fixed strings, got {note:?}"
        );
        assert!(!note.contains("secret-looking-content"));
        assert!(!note.contains(dir.path().to_string_lossy().as_ref()));
        assert!(!note.contains('('), "no interpolated error detail");
        assert!(
            dir.path().join("character-session.json.corrupt").exists(),
            "the unreadable file is quarantined, not silently replaced"
        );
        assert_eq!(
            host.session.lock().expect("lock").epoch(),
            5,
            "epoch salvaged from the broken file +1"
        );
    }
}
