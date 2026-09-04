//! Character Session Host：AIP Character Session（`docs/aip/character-session.md`）的
//! Runtime 接線（use case 層）。
//!
//! 權威狀態機是純函式 crate `interaction-session`；這個模組只做它做不到的事：持久化、
//! 注入時間、把 [`Output`] 真的派送出去（iPhone wss、SSE、CPP renderer）、寫稽核。
//!
//! # 不變量
//!
//! - 語意狀態只有 [`CharacterSession`] 能改；這裡不推論、不改寫真相。
//! - `verified` 只能經 [`Runtime::character_session_submit_runtime`] 從 Runtime 的人類驗證
//!   路徑進來；device／renderer 送 `task.*`／`runtime.*` 一律 `scope-denied`。
//! - 身分是綁定出來的，不是宣稱：transport 先比對 `source`，不符即 `identity-mismatch`，
//!   不「幫忙修正」後執行。
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
use interaction_session::ports::{PortError, SessionStore};
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
/// diagnostics `storeNote`：持久化檔讀不到時的固定文字。
pub const STORE_NOTE_UNREADABLE: &str =
    "character session state could not be read; it was quarantined and a new session was started";
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

/// Snapshot 的 JSON 檔 store。檔案內容就是 [`Snapshot`]（含 `epoch`）。
pub struct JsonSessionStore {
    path: PathBuf,
}

impl JsonSessionStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
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
    fn save(&self, snapshot: &Snapshot) -> Result<(), PortError> {
        let body = serde_json::to_string(snapshot).map_err(|_| PortError::Rejected)?;
        // 先記 epoch 再寫快照：epoch 檔只能領先、不能落後，否則快照壞掉時就少了
        // 「成員記得的 epoch 至少有多大」這個線索。
        if snapshot.epoch > self.remembered_epoch() {
            self.remember_epoch(snapshot.epoch);
        }
        self.write_owner_only(&body)
    }

    fn load(&self, session_id: &str) -> Result<Option<Snapshot>, PortError> {
        let body = match std::fs::read_to_string(&self.path) {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(PortError::Unavailable),
        };
        let snapshot: Snapshot = serde_json::from_str(&body).map_err(|_| PortError::Corrupt)?;
        if snapshot.session_id != session_id {
            return Err(PortError::Corrupt);
        }
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
    /// 載入時的異常（誠實顯示在 diagnostics，不靜默）。
    load_note: Option<String>,
}

impl CharacterSessionHost {
    /// 從 `state/character-session.json` 續接；讀不到／壞掉 → 新 session（epoch+1）。
    pub fn open(state_dir: &Path, now: Timestamp) -> Arc<Self> {
        let store = Arc::new(JsonSessionStore::new(state_dir.join(SESSION_STORE_FILE)));
        let config = SessionConfig {
            session_id: SESSION_ID.to_string(),
            ..SessionConfig::default()
        };
        // 讀不到快照時**不覆寫**它（見下方 `Err(error)` 分支）：一次暫時性讀取失敗
        // 不該把一份可能完好的紀錄變成永久資料遺失。
        let mut preserve_existing = false;
        let (session, load_note) = match store.load(SESSION_ID) {
            Ok(Some(mut snapshot)) => {
                // epoch 只能往前：這台機器曾經以更大的 epoch 跑過（例如上次啟動讀不到
                // 這份快照而另開了一個 session），成員記得的就是那個更大的值。
                snapshot.epoch = snapshot.epoch.max(store.remembered_epoch());
                match CharacterSession::restore(config.clone(), &snapshot, now) {
                    Ok(session) => (session, None),
                    Err(error) => {
                        // 錯誤細節只進 log；diagnostics 的 note 是固定文字（不帶路徑、不帶
                        // 反序列化訊息——那些可能回顯檔案內容或檔案系統路徑）。
                        tracing::warn!(%error, "stored character session state was unusable");
                        let epoch = store.quarantine();
                        (
                            CharacterSession::new(config.clone(), epoch, now),
                            Some(STORE_NOTE_UNUSABLE.to_string()),
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
                (CharacterSession::new(config.clone(), epoch, now), None)
            }
            // 內容壞掉（解不開／不是這個 session 的快照）：隔離。
            Err(PortError::Corrupt) => {
                tracing::warn!("stored character session state was unusable");
                let epoch = store.quarantine();
                (
                    CharacterSession::new(config.clone(), epoch, now),
                    Some(STORE_NOTE_UNUSABLE.to_string()),
                )
            }
            // 暫時性 I/O 失敗（權限、EIO、fd 用盡）：檔案**不動**。一次讀不到就把
            // 一份可能完好的快照改名丟棄，是把暫時性故障變成永久資料遺失。
            Err(error) => {
                tracing::warn!(%error, "character session state could not be read");
                preserve_existing = true;
                let epoch = store.next_epoch();
                (
                    CharacterSession::new(config.clone(), epoch, now),
                    Some(STORE_NOTE_UNREADABLE.to_string()),
                )
            }
        };
        // 立刻落一份：重啟後才續接得到 revision／epoch，而不是默默從頭開始。
        // 例外是「讀不到但檔案還在」：那一份留給下一次啟動（或人）去救，這次不覆寫。
        if !preserve_existing {
            if let Err(error) = store.save(&session.snapshot()) {
                tracing::warn!(%error, "character session snapshot was not persisted at startup");
            }
        }
        if let Some(note) = &load_note {
            tracing::warn!(note, "character session started from a clean state");
        }
        Arc::new(Self {
            session: Mutex::new(session),
            store,
            desktop_presence: Mutex::new(None),
            load_note,
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

    /// §10 diagnostics（不含 token、路徑、原始 payload）。
    pub fn character_session_diagnostics_value(&self) -> DomainResult<Value> {
        let host = self.session_host()?;
        let diagnostics = host.session().diagnostics();
        let members: Vec<Value> = diagnostics
            .members
            .iter()
            .map(|member| {
                json!({
                    "party": member.party,
                    "role": member.role,
                    "presence": member.presence.as_str(),
                    "lastSeenAt": member.last_seen_at,
                    // 協商為 unsupported 的 intent 名（沒有就是空陣列）。這是
                    // §11「部分能力目前不可用」的唯一真實來源；協商結果的其餘細節
                    // 仍是 host 私有，不外洩。
                    "unsupportedIntents": member.unsupported_intents,
                })
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
        // host 的進度**落後**成員：這只可能是 session 被重建過（或還原了更舊的快照）。
        // 這種 snapshot 對接收端而言長得像 rollback（revision 比它自己記得的小），
        // AIP §6 的防重播規則會直接忽略它，畫面卻仍顯示「已同步」——兩邊都不會察覺。
        // 所以要明說這是重新開始（`reason: session-reset`），讓接收端合法地丟掉本地狀態。
        let resume = match resume {
            Resume::Snapshot { envelope }
                if envelope
                    .payload
                    .get("revision")
                    .and_then(Value::as_u64)
                    .is_some_and(|revision| revision < last_revision) =>
            {
                Resume::EpochMismatch { envelope }
            }
            other => other,
        };
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
        if let Err(error) = host.store().save(&snapshot) {
            tracing::warn!(%error, "character session snapshot was not persisted on shutdown");
        }
    }

    // ------------------------------------------------------------------
    // iPhone wss binding（`{"type":"aip","envelope":…}`）
    // ------------------------------------------------------------------

    /// 已認證手機送來的一則 `aip` frame。回傳要送回去的 envelope。
    pub(crate) async fn character_session_device_frame(
        &self,
        device_id: &str,
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
                    "transport": "iphone",
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
                    "transport": "iphone",
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
                self.character_session_device_capability(&party, envelope, now)
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
            let _ = self.store.audit(
                "aip.rejected",
                "runtime",
                &json!({
                    "transport": "iphone",
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
                if let Err(reason) = self.mobile.send_aip(&to.id, envelope).await {
                    // 送不到不等於送到了：只記錄，不重送、不假裝成功。
                    tracing::debug!(reason, "an aip frame did not reach a paired iPhone");
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

    async fn character_session_persist(&self, snapshot: Snapshot) {
        let Some(host) = self.character_session.as_ref() else {
            return;
        };
        let store = host.store();
        // hot path 不做同步檔案 I/O。
        let _ = tokio::task::spawn_blocking(move || store.save(&snapshot)).await;
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
        let mut writers = Vec::new();
        // 大小差很多的兩份快照：拼接起來一定是壞 JSON，而不是碰巧還讀得回來。
        for width in [8usize, 60_000] {
            let store = store.clone();
            let stop = stop.clone();
            writers.push(std::thread::spawn(move || {
                let snapshot = snapshot(width);
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
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
