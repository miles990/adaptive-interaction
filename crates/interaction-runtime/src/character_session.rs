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

    /// 壞掉的檔案改名成 `<file>.corrupt` 並回傳「下一個 epoch」。
    /// 不靜默：epoch+1 讓所有成員在 resume 時拿到 `session-reset`，而不是默默對齊到一個
    /// 從頭開始的 revision。
    fn quarantine(&self) -> u64 {
        let salvaged = std::fs::read_to_string(&self.path)
            .map(|body| salvaged_epoch(&body))
            .unwrap_or(0);
        let quarantined = self.path.with_extension("json.corrupt");
        if let Err(e) = std::fs::rename(&self.path, &quarantined) {
            tracing::warn!(error = %e, "could not quarantine the unreadable character session file");
            let _ = std::fs::remove_file(&self.path);
        }
        salvaged.saturating_add(1)
    }

    fn write_owner_only(&self, contents: &str) -> Result<(), PortError> {
        let parent = self.path.parent().ok_or(PortError::Unavailable)?;
        std::fs::create_dir_all(parent).map_err(|_| PortError::Unavailable)?;
        // 每次寫入都用自己的暫存檔：兩個持久化同時發生時（`Output::Persist` 各自
        // 走一次 `spawn_blocking`），共用檔名的 `truncate` 會截掉另一個寫入者已經
        // 寫進去的內容，rename 出去的就是兩份 JSON 的拼接。程序內用計數器分開，
        // 跨程序用 pid 分開。
        let ticket = TMP_TICKET.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = parent.join(format!(
            ".{SESSION_STORE_FILE}.tmp-{}-{ticket}",
            std::process::id()
        ));
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
        if let Err(error) = std::fs::rename(&tmp, &self.path) {
            let _ = std::fs::remove_file(&tmp);
            tracing::debug!(%error, "character session snapshot could not be renamed into place");
            return Err(PortError::Unavailable);
        }
        Ok(())
    }
}

/// 暫存檔名的程序內序號（見 [`JsonSessionStore::write_owner_only`]）。
static TMP_TICKET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl SessionStore for JsonSessionStore {
    fn save(&self, snapshot: &Snapshot) -> Result<(), PortError> {
        let body = serde_json::to_string(snapshot).map_err(|_| PortError::Rejected)?;
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
        let (session, load_note) = match store.load(SESSION_ID) {
            Ok(Some(snapshot)) => match CharacterSession::restore(config.clone(), &snapshot, now) {
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
            },
            Ok(None) => (CharacterSession::new(config.clone(), 1, now), None),
            Err(error) => {
                tracing::warn!(%error, "character session state could not be read");
                let epoch = store.quarantine();
                (
                    CharacterSession::new(config.clone(), epoch, now),
                    Some(STORE_NOTE_UNREADABLE.to_string()),
                )
            }
        };
        // 立刻落一份：重啟後才續接得到 revision／epoch，而不是默默從頭開始。
        if let Err(error) = store.save(&session.snapshot()) {
            tracing::warn!(%error, "character session snapshot was not persisted at startup");
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
            // 離線超過 presence timeout 的成員不留在名單上（幽靈成員＝假的「已連接」）。
            let timeout = ChronoDuration::milliseconds(session.config().presence_timeout_ms);
            let stale: Vec<Party> = session
                .members()
                .into_iter()
                .filter(|member| {
                    member.presence == Presence::Offline
                        && now.signed_duration_since(member.last_seen_at) >= timeout
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
                Some(envelope.message_id.clone()),
                now,
            ));
        }
        if !envelope.message_type.is_known() {
            return AipFrameOutcome::reply(self.character_session_error(
                ErrorCode::UnsupportedMessageType,
                "messageType is not one of the 12 known AIP message types",
                Some(envelope.message_id.clone()),
                now,
            ));
        }
        // 宣稱不是身分（§5）：不符一律拒絕並稽核，不得修正後執行。
        if let IdentityDecision::Reject { .. } = bind_identity(&party, &envelope.source) {
            let _ = self.store.audit(
                "aip.identity-mismatch",
                "runtime",
                &json!({
                    "transport": "iphone",
                    "boundKind": "device",
                    "claimedKind": envelope.source.kind,
                    "name": envelope.name,
                }),
            );
            return AipFrameOutcome::reply(self.character_session_error(
                ErrorCode::IdentityMismatch,
                "source does not match the paired identity of this connection",
                Some(envelope.message_id.clone()),
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

    /// 這個 party 目前是不是成員（決定 capability 走 join 還是走安全管線）。
    fn character_session_is_member(&self, party: &Party) -> bool {
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
