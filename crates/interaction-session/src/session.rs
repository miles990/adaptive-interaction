//! §1／§2／§6／§8：權威 Character Session Host。
//!
//! 這裡是唯一能改 [`SemanticState`] 的地方（§2 State Ownership），也是 §8 安全管線的唯一入口。
//! 沒有 I/O：`now` 由呼叫端注入，輸出是 [`Output`] 清單，由 Runtime 端的 use case 真的送出去。

use std::collections::{BTreeMap, VecDeque};

use chrono::Duration;
use interaction_aip::{
    bind_identity, is_runtime_only_name, is_valid_name, limits, negotiate_capabilities,
    CapabilityAnnouncement, DedupeRing, Envelope, ErrorCode, HostOffer, IdentityDecision,
    IntentSupport, MemberRole, MessageType, NegotiatedCapabilities, Outcome, Party, PartyKind,
    SyncClass, Timestamp,
};
use interaction_character::TruthState;
use serde_json::{json, Map, Value};

use crate::cpp::{behavior_to_cpp, CppProjection};
use crate::director::{self, InteractionEvent, TouchKind};
use crate::patch::{apply_patch, merge_diff, state_hash};
use crate::state::{Activity, Member, MemberView, Presence, SemanticState};
use crate::types::{
    BehaviorIntent, RuntimeFact, SessionConfig, SessionError, Snapshot, EVENT_DISMISS, EVENT_TOUCH,
    HOST_INPUTS, HOST_INTENTS, MAX_PENDING_INTENTS, NAME_BEHAVIOR_REQUEST, NAME_SESSION_CAPABILITY,
    NAME_SESSION_PATCH, NAME_SESSION_RESULT, NAME_SESSION_SNAPSHOT, REASON_RECOVERY,
    REASON_SESSION_RESET, SNAPSHOT_FORMAT,
};

/// 全新 session 的初始 revision（§1「revision 從 1 起」）。
pub const INITIAL_REVISION: u64 = 1;
/// `character.behavior.cancel`：撤銷已送出的 Behavior Intent。
pub const NAME_BEHAVIOR_CANCEL: &str = "character.behavior.cancel";

// 稽核種類（固定字串集合，不含輸入回顯）。
const AUDIT_REJECTED: &str = "aip.rejected";
const AUDIT_IDENTITY_MISMATCH: &str = "aip.identity-mismatch";
const AUDIT_DUPLICATE: &str = "aip.duplicate";
const AUDIT_JOIN: &str = "character.session.join";
const AUDIT_LEAVE: &str = "character.session.leave";
const AUDIT_PRESENCE: &str = "character.session.presence";
const AUDIT_APPLIED: &str = "character.session.applied";
const AUDIT_TRUTH: &str = "character.session.truth";
const AUDIT_EMERGENCY: &str = "character.session.emergency";
const AUDIT_INTENT_EXPIRED: &str = "character.session.intent-expired";
const AUDIT_INTENT_DROPPED: &str = "character.session.intent-dropped";
const AUDIT_INTENT_SETTLED: &str = "character.session.intent-settled";
/// 不在 `outstanding` 名單裡的成員回報了一則對得上 causation／correlation 的終態。
const AUDIT_INTENT_UNSOLICITED: &str = "character.session.intent-report-unsolicited";
const AUDIT_CANCEL: &str = "character.session.cancel";
const AUDIT_REPORT: &str = "character.session.report";
const AUDIT_INTERNAL: &str = "aip.internal";
/// 成員自報的 `role` 與 Transport 綁定身分不相容，已被夾回可證實的角色。
const AUDIT_ROLE_CORRECTED: &str = "character.session.role-corrected";
/// `occurredAt` 與 host 時鐘的偏差超過 [`limits::MAX_CLOCK_SKEW_MS`]（只記偏差量，不回顯 payload）。
const AUDIT_CLOCK_SKEW: &str = "aip.clock-skew";

// counters 的固定鍵（有界：`rejected.*` 只會出現在已知的 19 個錯誤碼上）。
const C_ACCEPTED: &str = "accepted";
const C_APPLIED: &str = "applied";
const C_DUPLICATES: &str = "duplicates";
const C_EXPIRED: &str = "expired";
const C_RESUMES: &str = "resumes";
/// 成員宣稱的進度超前 host（host 沒有證據自己倒退過時只記數，不重建 session）。
const C_RESUMES_AHEAD: &str = "resumes.ahead";
/// 送出去的 `reason:"recovery"` snapshot（host 的權威狀態真的比對方記得的舊）。
const C_SNAPSHOTS_RECOVERY: &str = "snapshots.recovery";
const C_SNAPSHOTS: &str = "snapshots";
const C_PATCHES: &str = "patches";
const C_IDENTITY_MISMATCH: &str = "identity_mismatch";
const C_INTENTS_EMITTED: &str = "intents.emitted";
const C_INTENTS_EXPIRED: &str = "intents.expired";
const C_INTENTS_DROPPED: &str = "intents.dropped";
const C_INTENTS_OBSERVED: &str = "intents.observed";
const C_INTENTS_REJECTED: &str = "intents.rejected";
const C_INTENTS_FAILED: &str = "intents.failed";
/// 冒領：不是待覆目標的成員回報了終態（只記數與稽核，不結清、不計進上面三個）。
const C_INTENTS_UNSOLICITED: &str = "intents.unsolicited";
const C_INTERNAL: &str = "internal";

/// host **投影**進協商回覆的 `unsupportedInputs` 上限（有界；host 送出的 capability 回覆不得無界成長）。
///
/// 與 [`interaction_aip::limits::MAX_UNSUPPORTED_INPUTS`]（32，協商函式本身的截斷點、發布在 golden schema
/// 的 `limits` 表）是**兩個不同的數字**：那一個是 AIP 層「協商結果最多保留幾筆」，這一個是 session host
/// 在那之上再收緊的投影上限（16 ≤ 32）。過去兩者同名、在各自 crate root 都可見，很容易誤引用。
pub const MAX_PROJECTED_UNSUPPORTED_INPUTS: usize = 16;

/// 上面那段文字裡的 `16 ≤ 32` 不能只靠註解提醒下一個人：任何一邊改動破壞這個
/// 關係，這裡就編不過（比紅燈更早）。
const _: () = assert!(
    MAX_PROJECTED_UNSUPPORTED_INPUTS <= interaction_aip::limits::MAX_UNSUPPORTED_INPUTS,
    "host 的投影上限不得大於 AIP 協商本身的截斷點"
);

/// 成員回報 `result` 時的終態（§5：`observed`／`rejected`／`failed`／`cancel-confirmed`）。
/// 終態才結清 host 的 pending intent；`accepted`／`acknowledged` 只是「收到了」。
/// 誠實階梯不變：`observed` 只是對方說它演了，**不是** verified。
fn terminal_report(status: &str) -> bool {
    matches!(
        status,
        "observed" | "rejected" | "failed" | "cancel-confirmed"
    )
}

/// 終態對應的計數器（`cancel-confirmed` 已由 cancel 路徑記過，不重複計）。
fn report_counter(status: &str) -> Option<&'static str> {
    match status {
        "observed" => Some(C_INTENTS_OBSERVED),
        "rejected" => Some(C_INTENTS_REJECTED),
        "failed" => Some(C_INTENTS_FAILED),
        _ => None,
    }
}

/// 有界事件日誌的一筆（§6 `EventLog`）。
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub message_id: String,
    pub sequence: u64,
    pub base_revision: u64,
    pub revision: u64,
    pub patch: Value,
    pub hash: String,
    pub at: Timestamp,
}

/// Session 要求 host 執行的動作。Session 自己不送任何東西。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "output", rename_all = "kebab-case")]
pub enum Output {
    /// 送給單一對象。
    Send { to: Party, envelope: Envelope },
    /// 送給所有成員（`except` 是剛剛已經拿到更新的那一位）。
    Broadcast {
        envelope: Envelope,
        except: Option<Party>,
    },
    /// 稽核紀錄。`detail` 不含 payload 回顯、token 或路徑。
    Audit { kind: String, detail: Value },
    /// 建議 host 持久化這份 snapshot（§6）。
    Persist(Snapshot),
    /// 給 host 端 renderer 的 Behavior Intent（含 CPP 投影；`celebrate` 不投影）。
    RendererIntent {
        intent: BehaviorIntent,
        cpp: Option<CppProjection>,
    },
}

/// 一則外部訊息的處理結果。**每則訊息只有一則** `result`。
#[derive(Debug, Clone, PartialEq)]
pub struct Submission {
    pub result: Envelope,
    pub outcome: Outcome,
    pub error: Option<ErrorCode>,
    pub outputs: Vec<Output>,
    /// host 是否應該把 `result` 真的送出去。AIP §2.1：`result`／`heartbeat` 不再回 `result`，
    /// `query` 的答覆是 `response`；但任何被拒絕的訊息一定要回，否則對方學不到。
    pub reply: bool,
}

/// §6 resume 的三種結果。
#[derive(Debug, Clone, PartialEq)]
pub enum Resume {
    /// 日誌內有 `lastRevision+1..=current`：回這些 patch（沿用原本的 sequence 與 messageId）。
    Patches { envelopes: Vec<Envelope> },
    /// 日誌不足（有界環滿了）：回完整 snapshot。這不是錯誤。
    Snapshot { envelope: Envelope },
    /// epoch 不同：host 重建過 session，接收端必須丟棄本地狀態。
    EpochMismatch { envelope: Envelope },
}

/// [`CharacterSession::join`] 的結果。
#[derive(Debug, Clone, PartialEq)]
pub struct JoinOutcome {
    pub negotiated: NegotiatedCapabilities,
    pub capability_envelope: Envelope,
    pub snapshot_envelope: Envelope,
    pub outputs: Vec<Output>,
}

/// [`CharacterSession::restore_report`] 的附帶結果：這份快照是從哪個格式讀進來的，
/// 以及還原後的 canonical state 是否與檔案裡的原文不同。
///
/// `canonical_changed` 為真代表「檔案裡那份 state 不是本實作現在會寫出來的形狀」——
/// 缺了帶 `default` 的新欄位、鍵序不同、數字書寫不同都算。它**不是**錯誤：權威狀態一律
/// 以 canonical 為準，host 只是需要知道「落地時要順便把檔案遷移成新格式」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreReport {
    /// 快照宣告的格式版本（0 ＝ v0.6.0，沒有 `format` 鍵）。
    pub format_from: u32,
    /// canonical state 與檔案裡的原文不同（→ 需要遷移）。
    pub canonical_changed: bool,
}

impl RestoreReport {
    /// 這份快照需要以現行格式重新落地嗎？
    pub fn needs_migration(&self) -> bool {
        self.format_from < SNAPSHOT_FORMAT || self.canonical_changed
    }
}

/// `attention` 的「允許鍵」檢查。
///
/// `Attention` 是 internally tagged enum（`{"kind":…}`），serde **不支援**對它做
/// `deny_unknown_fields`：`{"kind":"none","evilKey":…}` 會被靜默接受成 `None`。
/// v0.6.0 靠「canonical 重 hash」間接擋住這種污染；那道檢查改成 migration 訊號之後，
/// 這裡必須明確依 `kind` 比對允許鍵。未知 `kind`／缺 `kind` 交給後面的反序列化拒絕。
fn attention_keys_are_known(state: &Value) -> bool {
    let Some(attention) = state.get("attention") else {
        // 缺 `attention`：`SemanticState` 沒有 `default`，反序列化會拒絕。
        return true;
    };
    let Some(map) = attention.as_object() else {
        return true;
    };
    let allowed: &[&str] = match map.get("kind").and_then(Value::as_str) {
        Some("none") => &["kind"],
        Some("member") => &["kind", "id"],
        Some("task") => &["kind", "correlationId"],
        // 未知或缺 kind：形狀本來就不合法，讓反序列化去回 `InvalidState`。
        _ => return true,
    };
    map.keys().all(|key| allowed.contains(&key.as_str()))
}

/// `members[*].party` 的「允許鍵」檢查。
///
/// [`Party`] 是**線上共用**型別（`Envelope.source`／`target`、`CapabilityAnnouncement`
/// 都用它），對它加 `deny_unknown_fields` 會改變 wire 的向前相容行為；但持久化快照
/// 是另一回事：只有本實作寫得出來的 canonical state 才能成為權威狀態。`SemanticState`
/// 與 `MemberView` 都已經 `deny_unknown_fields`，唯獨再深一層的 `party` 物件會靜默
/// 吃掉未知鍵——攻擊者只要能改檔案，就能連 hash 一起重算，把任意欄位夾帶進權威狀態。
///
/// 這裡在反序列化**之前**擋掉（serde 事後就看不出來了）。`Attention::Member` 與
/// `LastInteraction.source` 走 `party_ref` 字串形式，沒有物件可污染，不在此列。
fn member_party_keys_are_known(state: &Value) -> bool {
    let Some(members) = state.get("members").and_then(Value::as_array) else {
        // 缺 `members`／形狀不對：`SemanticState` 沒有 `default`，反序列化會拒絕。
        return true;
    };
    const ALLOWED: &[&str] = &["kind", "id"];
    members.iter().all(|member| {
        match member.get("party").and_then(Value::as_object) {
            // `party` 不是物件（缺鍵／字串）：形狀本來就不合法，交給反序列化拒絕。
            None => true,
            Some(map) => map.keys().all(|key| ALLOWED.contains(&key.as_str())),
        }
    })
}

/// §10 diagnostics（不含 token、路徑、原始 payload）。
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostics {
    pub session_id: String,
    pub epoch: u64,
    pub revision: u64,
    pub sequence: u64,
    pub members: Vec<MemberView>,
    pub counters: BTreeMap<String, u64>,
    pub event_log_len: usize,
    pub event_log_cap: usize,
}

/// 權威 Character Session。
#[derive(Debug, Clone)]
pub struct CharacterSession {
    config: SessionConfig,
    epoch: u64,
    revision: u64,
    sequence: u64,
    emitted: u64,
    state: SemanticState,
    state_json: Value,
    members: Vec<MemberRuntime>,
    log: EventLog,
    pending: VecDeque<PendingIntent>,
    counters: BTreeMap<String, u64>,
    reacting_since: Option<Timestamp>,
    updated_at: Timestamp,
    last_persist_at: Timestamp,
    last_persist_revision: u64,
    /// 這個 session 是從持久化 snapshot 還原的，而那份 snapshot **無法證明**自己就是當機前
    /// 最後廣播出去的那一版（§6 持久化是有間隔的）。只有在這個旗標為真時，「成員宣稱的進度
    /// 超前 host」才是 host 真的倒退過的證據 → 重建 session（epoch+1）並發 `session-reset`。
    restored_from_snapshot: bool,
}

impl CharacterSession {
    /// 全新 session：revision 從 [`INITIAL_REVISION`] 起、sequence 從 0 起（第一則送出的訊息是 1）。
    pub fn new(config: SessionConfig, epoch: u64, now: Timestamp) -> Self {
        let config = config.normalized();
        let state = SemanticState::new(config.character_id.clone());
        let state_json = state_value(&state);
        let log = EventLog::new(config.event_log_cap);
        Self {
            config,
            epoch,
            revision: INITIAL_REVISION,
            sequence: 0,
            emitted: 0,
            state,
            state_json,
            members: Vec::new(),
            log,
            pending: VecDeque::new(),
            counters: BTreeMap::new(),
            reacting_since: None,
            updated_at: now,
            last_persist_at: now,
            last_persist_revision: INITIAL_REVISION,
            restored_from_snapshot: false,
        }
    }

    /// 從持久化 snapshot 續接（§1：重啟後 revision 不歸零、也**不倒退**）。
    /// hash 不符、帶未知欄位或狀態違反不變量 → `Err`。
    ///
    /// 還原出來的成員保留 presence 投影，但**沒有**協商結果：他們必須重送 `capability`
    /// （§7 重連流程第 2 步）才能再送 event，否則會拿到 `scope-denied`。
    ///
    /// # 兩個安全取捨
    ///
    /// 1. **未知欄位一律拒絕**（session-integrity-061）：hash 自洽只證明「這份 JSON 沒有被改過」，
    ///    不證明「這份 JSON 是本實作寫出來的」。被污染的 state 若原樣還原，會被當成權威狀態
    ///    重新廣播給所有成員。擋下污染的是**反序列化本身**：`SemanticState`／`Mood`／`TruthView`／
    ///    `LastInteraction`／`MemberView` 都是 `deny_unknown_fields`，多一個鍵就 `InvalidState`。
    ///    唯一的例外是 `attention`——internally tagged enum，serde 不支援 `deny_unknown_fields`，
    ///    未知鍵會被靜默忽略，所以另外用 [`attention_keys_are_known`] 明確檢查允許鍵。
    ///    落地的權威狀態永遠是 canonical（`state_value(&state)`），不是檔案裡的原文，
    ///    所以就算某個鍵溜過檢查也不會被重新廣播出去。
    ///
    ///    **canonical 與原始不同不再是拒絕條件**（M1 §2.2）：v0.6.0 拿它當第二道 hash 檢查，
    ///    代價是任何帶 `#[serde(default)]` 的新欄位都會讓舊快照被判 `HashMismatch`
    ///    （v0.6.0 已知限制 #21：`MemberView.unsupportedIntents`）。現在它是「這份快照需要遷移」
    ///    的訊號，由 [`CharacterSession::restore_report`] 回報給 host。
    /// 2. **revision 保守跳號**（session-integrity-058）：持久化是有間隔的（預設每
    ///    `persist_every_revisions` 個 revision 或每 `persist_interval_ms`），所以當機前最後幾個
    ///    revision 可能已經廣播出去卻沒落地。直接從 snapshot 的 revision 續接會讓 host 倒退，
    ///    成員依 §6 的 rollback 防護忽略權威狀態、永久停在舊版本。這裡以「一個持久化間隔」為
    ///    上界往前跳號。**這只是啟發式**：跳號不足以涵蓋所有情況（`presence()`／`note_alive_party`
    ///    這些路徑不呼叫 `persist_if_due`），真正的保證由 [`CharacterSession::resume`] 提供——
    ///    只要有成員拿出「我看過更高的 revision」的證據，host 就 epoch+1 並發 `session-reset`。
    pub fn restore(
        config: SessionConfig,
        snapshot: &Snapshot,
        now: Timestamp,
    ) -> Result<Self, SessionError> {
        Self::restore_report(config, snapshot, now).map(|(session, _)| session)
    }

    /// 與 [`CharacterSession::restore`] 相同，但額外回報這份快照的來源格式與是否被遷移。
    ///
    /// Host 用它決定「要不要先備份原檔再以新格式落地」；`restore` 是它的薄包裝。
    pub fn restore_report(
        config: SessionConfig,
        snapshot: &Snapshot,
        now: Timestamp,
    ) -> Result<(Self, RestoreReport), SessionError> {
        let config = config.normalized();
        if snapshot.session_id != config.session_id {
            return Err(SessionError::SessionMismatch);
        }
        if state_hash(&snapshot.state) != snapshot.hash {
            return Err(SessionError::HashMismatch);
        }
        // `attention` 的補洞檢查（見上面取捨 1）：要在反序列化之前做，因為 serde 會把
        // 未知鍵吃掉，之後就看不出來了。
        if !attention_keys_are_known(&snapshot.state) {
            return Err(SessionError::InvalidState);
        }
        if !member_party_keys_are_known(&snapshot.state) {
            return Err(SessionError::InvalidState);
        }
        let state: SemanticState = serde_json::from_value(snapshot.state.clone())
            .map_err(|_| SessionError::InvalidState)?;
        if state.violates_limits(config.max_members) {
            return Err(SessionError::InvalidState);
        }
        // 只有本實作寫得出來的 canonical state 才能成為權威狀態（見上面取捨 1）。
        let canonical = state_value(&state);
        let report = RestoreReport {
            format_from: snapshot.format,
            canonical_changed: canonical != snapshot.state,
        };
        let revision = snapshot
            .revision
            .saturating_add(config.persist_every_revisions);
        let members = state
            .members
            .iter()
            .map(|view| {
                MemberRuntime::new(
                    Member {
                        party: view.party.clone(),
                        role: view.role,
                        presence: view.presence,
                        last_seen_at: view.last_seen_at,
                        negotiated: unnegotiated(view.role),
                    },
                    config.rate_limit_per_sec,
                    now,
                )
            })
            .collect();
        let log = EventLog::new(config.event_log_cap);
        let reacting_since = (state.activity == Activity::Reacting).then_some(now);
        let session = Self {
            config,
            epoch: snapshot.epoch,
            revision,
            sequence: snapshot.sequence,
            // messageId 空間跟著已持久化的 sequence 走，避免重啟後與舊訊息撞號。
            emitted: snapshot.sequence,
            state_json: canonical,
            state,
            members,
            log,
            pending: VecDeque::new(),
            counters: BTreeMap::new(),
            reacting_since,
            updated_at: snapshot.at,
            last_persist_at: now,
            last_persist_revision: revision,
            restored_from_snapshot: true,
        };
        Ok((session, report))
    }

    pub fn config(&self) -> &SessionConfig {
        &self.config
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn state(&self) -> &SemanticState {
        &self.state
    }
    /// Host 私有的成員紀錄（含協商結果）。
    pub fn members(&self) -> Vec<Member> {
        self.members.iter().map(|m| m.member.clone()).collect()
    }
    /// 尚未過期、尚未取消、也還沒被成員回報成終態的 Behavior Intent
    /// （有界 ≤ [`MAX_PENDING_INTENTS`]）。
    pub fn pending_intents(&self) -> Vec<BehaviorIntent> {
        self.pending.iter().map(|p| p.intent.clone()).collect()
    }
    /// 有界事件日誌內容（§6 delta replay 用）。
    pub fn event_log(&self) -> Vec<LogEntry> {
        self.log.entries.iter().cloned().collect()
    }
    /// Host 自行合成互動事件時該用的 deadline（§7 互動事件必填 `expiresAt`）。
    pub fn interaction_deadline(&self, now: Timestamp) -> Timestamp {
        now + Duration::milliseconds(self.config.touch_ttl_ms)
    }

    /// §4.2 host 端能力：要求對方能呈現的 intent、接受的 event name、支援的同步等級。
    pub fn host_offer(&self) -> HostOffer {
        HostOffer {
            intents: HOST_INTENTS.iter().map(|s| s.to_string()).collect(),
            inputs: HOST_INPUTS.iter().map(|s| s.to_string()).collect(),
            sync_classes: vec![SyncClass::Semantic],
        }
    }

    /// 加入或重新協商。重複 join 就是重新協商（§7 重連流程第 2 步）。
    pub fn join(
        &mut self,
        party: Party,
        announcement: &CapabilityAnnouncement,
        now: Timestamp,
    ) -> Result<JoinOutcome, SessionError> {
        let mut negotiated = negotiate_capabilities(&self.host_offer(), announcement)
            .map_err(SessionError::Negotiation)?;
        // §5 身分綁定的延伸：`role` 也是宣稱。`host-renderer`（可信桌面 surface、拿得到
        // 安全 overlay 的那一個）只能由 `human-surface` 身分擔任；device／renderer 自報
        // host-renderer 只會得到一個「共享狀態說它能演、但它永遠不在派送名單上」的假象。
        // 夾回可證實的角色並稽核，不拒絕連線（identity-binding-007）。
        let claimed_role = negotiated.role;
        negotiated.role = effective_role(&party.kind, claimed_role);
        // `unsupportedInputs` 直接來自對方宣告的 inputs，本身無界：截斷成有界清單，
        // 否則 host 自己送出的 capability 回覆會超過 §11 的 payload 上限（session-integrity-060）。
        let announced_inputs = negotiated.unsupported_inputs.len();
        let truncated_upstream = negotiated.unsupported_inputs_truncated;
        negotiated
            .unsupported_inputs
            .truncate(MAX_PROJECTED_UNSUPPORTED_INPUTS);
        let existing = self.index_of(&party);
        if existing.is_none() && self.members.len() >= self.config.max_members {
            return Err(SessionError::MembersFull);
        }
        match existing {
            Some(index) => {
                self.members[index].member.role = negotiated.role;
                self.members[index].member.negotiated = negotiated.clone();
                // 重新協商是**存活證明**，不是狀態變更：`lastSeenAt` 走 §12.7 的投影格線
                // （identity-binding-004），token bucket 也**不重填**——否則成員只要每隔幾則
                // 就插一則 capability，就能自己解除 §8 第 10 關的速率上限
                // （capability-consent-049／identity-binding-005）。
                self.note_alive(index, now);
            }
            None => {
                self.members.push(MemberRuntime::new(
                    Member {
                        party: party.clone(),
                        role: negotiated.role,
                        presence: Presence::Online,
                        last_seen_at: now,
                        negotiated: negotiated.clone(),
                    },
                    self.config.rate_limit_per_sec,
                    now,
                ));
                self.project_members();
            }
        }
        let mut outputs = Vec::new();
        if let Some(envelope) = self.commit_and_patch(now) {
            outputs.push(Output::Broadcast {
                envelope,
                except: Some(party.clone()),
            });
        }
        let capability_envelope = self.capability_envelope(&party, &negotiated, now);
        let snapshot_envelope = self.snapshot_envelope_inner(&party, now, None);
        let unsupported: Vec<&str> = negotiated
            .intents
            .iter()
            .filter(|(_, support)| **support == IntentSupport::Unsupported)
            .map(|(name, _)| name.as_str())
            .collect();
        outputs.push(audit(
            AUDIT_JOIN,
            json!({
                "party": safe_party(&party),
                "role": negotiated.role,
                "unsupportedIntents": unsupported,
                "unsupportedInputs": negotiated.unsupported_inputs.len(),
                // 上游（`negotiate_capabilities`）已經先截斷過一次，`announced_inputs`
                // 因此可能已經是被截過的數字：兩層都要納入判斷，否則會少報截斷。
                "unsupportedInputsTruncated": truncated_upstream
                    || announced_inputs > negotiated.unsupported_inputs.len(),
                "members": self.members.len(),
            }),
        ));
        if negotiated.role != claimed_role {
            outputs.push(audit(
                AUDIT_ROLE_CORRECTED,
                json!({
                    "party": safe_party(&party),
                    "claimed": claimed_role,
                    "effective": negotiated.role,
                }),
            ));
        }
        if let Some(persist) = self.persist_if_due(now) {
            outputs.push(persist);
        }
        Ok(JoinOutcome {
            negotiated,
            capability_envelope,
            snapshot_envelope,
            outputs,
        })
    }

    /// 離開（冪等）。離開後同一個 party 再送訊息會得到 `not-a-member`。
    pub fn leave(&mut self, party: &Party, now: Timestamp) -> Vec<Output> {
        let Some(index) = self.index_of(party) else {
            return Vec::new();
        };
        self.members.remove(index);
        self.project_members();
        let mut outputs = Vec::new();
        if let Some(envelope) = self.commit_and_patch(now) {
            outputs.push(Output::Broadcast {
                envelope,
                except: None,
            });
        }
        outputs.push(audit(
            AUDIT_LEAVE,
            json!({"party": safe_party(party), "members": self.members.len()}),
        ));
        if let Some(persist) = self.persist_if_due(now) {
            outputs.push(persist);
        }
        outputs
    }

    /// **存活證明**：Transport 收到這個成員的任何已驗證訊息（含 AIP 之前的舊協定 frame）。
    ///
    /// 與 [`CharacterSession::presence`] 的差別：這裡走 `lastSeenAt` 的投影格線，所以
    /// 每 30 秒一則的舊 `status` 心跳不會每次都推進一個 revision；presence 本身的變化
    /// （Offline／Reconnecting → Online）仍然即時反映。不是成員就什麼都不做。
    pub fn note_alive_party(&mut self, party: &Party, now: Timestamp) -> Vec<Output> {
        let Some(index) = self.index_of(party) else {
            return Vec::new();
        };
        self.note_alive(index, now);
        match self.commit_and_patch(now) {
            Some(envelope) => vec![Output::Broadcast {
                envelope,
                except: None,
            }],
            None => Vec::new(),
        }
    }

    /// 更新 presence（host 或 transport 觀察到的）。
    pub fn presence(&mut self, party: &Party, presence: Presence, now: Timestamp) -> Vec<Output> {
        let Some(index) = self.index_of(party) else {
            return Vec::new();
        };
        self.members[index].member.presence = presence;
        self.members[index].member.last_seen_at = now;
        self.members[index].projected_seen_at = now;
        self.project_members();
        let mut outputs = Vec::new();
        if let Some(envelope) = self.commit_and_patch(now) {
            outputs.push(Output::Broadcast {
                envelope,
                except: None,
            });
        }
        outputs.push(audit(
            AUDIT_PRESENCE,
            json!({"party": safe_party(party), "presence": presence.as_str()}),
        ));
        outputs
    }

    /// §8 安全管線（順序固定）＋套用。每則訊息只回一則 result。
    pub fn submit(
        &mut self,
        envelope: Envelope,
        bound_identity: &Party,
        now: Timestamp,
    ) -> Submission {
        // §11 時鐘偏差：只稽核偏差量（不拒絕、不回顯 payload）。成員身分先確認過才記，
        // 陌生人的訊息不該替我們製造稽核。
        let skew = self
            .index_of(bound_identity)
            .and_then(|_| clock_skew_audit(&envelope, now));
        let message_id = envelope.message_id.clone();
        let mut submission = match self.gate(&envelope, bound_identity, now) {
            Err(failure) => self.rejection(&envelope, failure, now),
            Ok(Gate::Duplicate) => {
                self.bump(C_DUPLICATES);
                let result = self.result_envelope(&envelope, Outcome::Accepted, None, true, now);
                Submission {
                    result,
                    outcome: Outcome::Accepted,
                    error: None,
                    outputs: vec![audit(
                        AUDIT_DUPLICATE,
                        json!({"messageId": safe_id(&envelope.message_id), "name": safe_name(&envelope.name)}),
                    )],
                    reply: true,
                }
            }
            Ok(Gate::Proceed) => {
                let submission = self.dispatch(envelope, bound_identity, now);
                // §7 去重：`accepted{duplicate:true}` 的意思是「上一次真的處理過了」。
                // 被後面任何一關（emergency、handler 自己的 schema 檢查）拒絕的訊息
                // **不佔**去重環的位置，否則重送會拿到 duplicate 卻從未被套用
                // （session-integrity-062）。
                if !matches!(submission.outcome, Outcome::Rejected | Outcome::Expired) {
                    if let Some(index) = self.index_of(bound_identity) {
                        self.members[index].dedupe.note(&message_id);
                    }
                }
                submission
            }
        };
        if let Some(skew) = skew {
            submission.outputs.push(skew);
        }
        // 存活證明（見 `gate` 第 4.1 關）造成的 presence／`lastSeenAt` 變動也要廣播出去：
        // 被拒絕／過期／重複的路徑不會經過任何 handler 的 commit。已經 commit 過的路徑
        // 在這裡拿到 `None`，所以一則訊息永遠只產生一則 patch。
        if let Some(envelope) = self.commit_and_patch(now) {
            submission.outputs.push(Output::Broadcast {
                envelope,
                except: None,
            });
        }
        submission
    }

    /// 可信來源（Runtime）的真相事實。不經身分管線，但一樣只轉錄、不推論。
    pub fn submit_runtime(
        &mut self,
        fact: RuntimeFact,
        correlation: Option<String>,
        now: Timestamp,
    ) -> Vec<Output> {
        let correlation = correlation.map(|c| safe_id(&c));
        // 緊急停止守衛：**只有** `runtime.emergency{engaged:false}` 能離開 emergency。
        // 任何 `task.*` 的真相轉錄都會把 `truth`／`activity` 寫回非 emergency 的值，等於讓
        // 一個不相關的工作解除守衛，互動立刻重新被接受（CLAUDE.md：AI 不可解除 emergency
        // stop；session-integrity-056）。被擋下的真相只留稽核，不進狀態。
        if self.state.truth.state == TruthState::Emergency {
            let blocked = match fact {
                RuntimeFact::TaskState { .. } => Some("task.state"),
                RuntimeFact::TaskVerified { .. } => Some("task.verified"),
                _ => None,
            };
            if let Some(blocked) = blocked {
                return vec![audit(
                    AUDIT_EMERGENCY,
                    json!({"engaged": true, "blocked": blocked}),
                )];
            }
        }
        let Some((patch, intents)) = director::on_fact(
            &self.state,
            &fact,
            correlation.as_deref(),
            &self.config,
            now,
        ) else {
            return Vec::new();
        };
        if !self.apply_semantic_patch(&patch) {
            return vec![audit(AUDIT_INTERNAL, json!({"stage": "runtime-fact"}))];
        }
        let mut outputs = Vec::new();
        let mut cancelled = 0usize;
        // 緊急停止不只清掉 host 自己的帳：已經拿到 command 的 renderer 必須收到
        // `character.behavior.cancel`，否則它會把還沒演完的動作演完（capability-consent-054）。
        let mut cancel_sends = Vec::new();
        if matches!(fact, RuntimeFact::Emergency { engaged: true }) {
            let pending: Vec<PendingIntent> = self.pending.drain(..).collect();
            cancelled = pending.len();
            for entry in &pending {
                for party in &entry.outstanding {
                    let envelope = self.cancel_envelope(party, &entry.intent, now);
                    cancel_sends.push(Output::Send {
                        to: party.clone(),
                        envelope,
                    });
                }
            }
        }
        if let Some(envelope) = self.commit_and_patch(now) {
            outputs.push(Output::Broadcast {
                envelope,
                except: None,
            });
        }
        outputs.append(&mut cancel_sends);
        self.sync_reacting_clock(now);
        for intent in intents {
            outputs.extend(self.emit_intent(intent, now));
        }
        if let RuntimeFact::Emergency { engaged } = fact {
            outputs.push(audit(
                AUDIT_EMERGENCY,
                json!({"engaged": engaged, "cancelledIntents": cancelled}),
            ));
        }
        outputs.push(audit(
            AUDIT_TRUTH,
            json!({"truth": self.state.truth.state, "revision": self.revision}),
        ));
        self.bump(C_APPLIED);
        if let Some(persist) = self.persist_if_due(now) {
            outputs.push(persist);
        }
        outputs
    }

    /// 取消（冪等）。已終態或找不到對應的 pending intent → `cancel-confirmed{alreadyTerminal:true}`。
    pub fn cancel(
        &mut self,
        envelope: Envelope,
        bound_identity: &Party,
        now: Timestamp,
    ) -> Submission {
        if envelope.message_type != MessageType::Cancel {
            return self.rejection(
                &envelope,
                GateFailure::rejected(ErrorCode::SchemaInvalid),
                now,
            );
        }
        self.submit(envelope, bound_identity, now)
    }

    /// 目前的權威快照（純讀；`at` 是最後一次狀態變更的時間）。
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            format: SNAPSHOT_FORMAT,
            session_id: self.config.session_id.clone(),
            epoch: self.epoch,
            revision: self.revision,
            sequence: self.sequence,
            state: self.state_json.clone(),
            hash: state_hash(&self.state_json),
            at: self.updated_at,
        }
    }

    /// `state{kind:"snapshot"}`，消耗一個 sequence。
    pub fn snapshot_envelope(&mut self, to: &Party, now: Timestamp) -> Envelope {
        self.snapshot_envelope_inner(to, now, None)
    }

    /// §6 resume：epoch 不同 → session-reset snapshot；日誌涵蓋得到 → patches；否則 → snapshot。
    pub fn resume(
        &mut self,
        party: &Party,
        last_revision: u64,
        last_sequence: u64,
        epoch: u64,
        now: Timestamp,
    ) -> Resume {
        self.bump(C_RESUMES);
        // 對方宣稱看過的進度超前 host。這是**宣稱**，不是證據——除非本 session 是從一份
        // 無法證明自己最新的 snapshot 還原出來的（見 [`CharacterSession::restore`] 的取捨 2），
        // 那時它就是「host 真的倒退過」的證據：重建 session（epoch+1）並發 `session-reset`，
        // 讓成員依 §6 合法丟棄本地狀態。沒有這一步，重啟後的權威 snapshot 會被 rollback
        // 防護忽略，成員永遠停在舊狀態（session-integrity-058）。
        let ahead = last_revision > self.revision || last_sequence > self.sequence;
        if ahead && self.restored_from_snapshot {
            self.restored_from_snapshot = false;
            self.epoch = self.epoch.saturating_add(1);
            let envelope = self.snapshot_envelope_inner(party, now, Some(REASON_SESSION_RESET));
            return Resume::EpochMismatch { envelope };
        }
        if epoch != self.epoch {
            let envelope = self.snapshot_envelope_inner(party, now, Some(REASON_SESSION_RESET));
            return Resume::EpochMismatch { envelope };
        }
        // host 沒有倒退過的證據時，成員自己宣稱超前不能讓它重建整個 session
        // （否則任何一個成員都能用一則 resume 逼所有人重來）：不動 epoch、只記數。
        //
        // 但回一則**沒有 reason** 的 snapshot 等於沒回答：它的 revision 比對方記得的小，
        // 接收端的 rollback 防護會直接忽略它，兩邊永久分歧而畫面都寫著「已同步」
        // （`capability-consent-048`）。謊稱 `session-reset` 也沒有用——§7 的 reset 例外
        // 要求 epoch **不同**，同 epoch 的 `session-reset` 一樣被忽略。所以這裡用
        // [`REASON_RECOVERY`]：同一個 session、epoch 不變，host 的 revision 就是真相
        // （AIP 1.0 接收端澄清規則 6；只認得舊 reason 值的接收端仍當普通 snapshot＝與現況相同）。
        if ahead {
            self.bump(C_RESUMES_AHEAD);
            self.bump(C_SNAPSHOTS_RECOVERY);
            return Resume::Snapshot {
                envelope: self.snapshot_envelope_inner(party, now, Some(REASON_RECOVERY)),
            };
        }
        if last_revision == self.revision {
            return Resume::Patches {
                envelopes: Vec::new(),
            };
        }
        let entries: Vec<LogEntry> = self
            .log
            .entries
            .iter()
            .filter(|e| e.revision > last_revision)
            .cloned()
            .collect();
        let contiguous = entries
            .first()
            .is_some_and(|first| first.base_revision == last_revision);
        if !contiguous || entries.len() as u64 != self.revision - last_revision {
            return Resume::Snapshot {
                envelope: self.snapshot_envelope_inner(party, now, None),
            };
        }
        let envelopes = entries
            .iter()
            .map(|entry| self.replay_envelope(entry, party))
            .collect();
        Resume::Patches { envelopes }
    }

    /// 週期性維護：reacting 逾時 → idle、presence 逾時 → offline、清掉過期的 pending intent。
    pub fn tick(&mut self, now: Timestamp) -> Vec<Output> {
        let mut outputs = Vec::new();
        let before = self.pending.len();
        self.pending
            .retain(|pending| pending.intent.expires_at > now);
        let expired = before - self.pending.len();
        if expired > 0 {
            self.bump_by(C_INTENTS_EXPIRED, expired as u64);
            outputs.push(audit(AUDIT_INTENT_EXPIRED, json!({"count": expired})));
        }

        let timeout = Duration::milliseconds(self.config.presence_timeout_ms);
        let mut timed_out: Vec<String> = Vec::new();
        for entry in self.members.iter_mut() {
            if entry.member.presence != Presence::Offline
                && now.signed_duration_since(entry.member.last_seen_at) >= timeout
            {
                entry.member.presence = Presence::Offline;
                timed_out.push(safe_party(&entry.member.party));
            }
        }
        if !timed_out.is_empty() {
            self.project_members();
            outputs.push(audit(
                AUDIT_PRESENCE,
                json!({"presence": "offline", "parties": timed_out, "reason": "timeout"}),
            ));
        }

        if self.state.activity == Activity::Reacting {
            match self.reacting_since {
                Some(since)
                    if now.signed_duration_since(since)
                        >= Duration::milliseconds(self.config.reaction_ms) =>
                {
                    self.apply_semantic_patch(&director::settle_to_idle());
                    self.reacting_since = None;
                }
                None => self.reacting_since = Some(now),
                _ => {}
            }
        } else {
            self.reacting_since = None;
        }

        if let Some(envelope) = self.commit_and_patch(now) {
            outputs.push(Output::Broadcast {
                envelope,
                except: None,
            });
        }
        if let Some(persist) = self.persist_if_due(now) {
            outputs.push(persist);
        }
        outputs
    }

    /// §10 diagnostics。不含 token、路徑、原始 payload。
    pub fn diagnostics(&self) -> Diagnostics {
        Diagnostics {
            session_id: self.config.session_id.clone(),
            epoch: self.epoch,
            revision: self.revision,
            sequence: self.sequence,
            members: self.state.members.clone(),
            counters: self.counters.clone(),
            event_log_len: self.log.entries.len(),
            event_log_cap: self.log.cap,
        }
    }

    // ------------------------------------------------------------------ 管線

    fn gate(
        &mut self,
        envelope: &Envelope,
        bound_identity: &Party,
        now: Timestamp,
    ) -> Result<Gate, GateFailure> {
        // 1. schema／profile／大小／深度／版本／name 語法（payload 上限含在 validate 內）。
        if let Err(err) = envelope.validate() {
            return Err(GateFailure::rejected(err.code));
        }
        // 2. 身分綁定：宣稱不符一律拒絕，不「幫忙修正」。
        if let IdentityDecision::Reject { .. } = bind_identity(bound_identity, &envelope.source) {
            return Err(GateFailure {
                code: ErrorCode::IdentityMismatch,
                outcome: Outcome::Rejected,
                audit_kind: AUDIT_IDENTITY_MISMATCH,
            });
        }
        // 3. 外部訊息不得宣稱自己是 Runtime。
        if envelope.source.kind == PartyKind::Runtime {
            return Err(GateFailure {
                code: ErrorCode::IdentityMismatch,
                outcome: Outcome::Rejected,
                audit_kind: AUDIT_IDENTITY_MISMATCH,
            });
        }
        // 4. membership。
        let Some(index) = self.index_of(bound_identity) else {
            return Err(GateFailure::rejected(ErrorCode::NotAMember));
        };
        // 4.1 **存活證明**：通過身分綁定與 membership 的 inbound envelope 就證明這個成員
        //     還在，不論 messageType、也不論後面是 applied／rejected／duplicate／expired。
        //     只送互動事件、從不送 heartbeat 的裝置因此不會被 presence 逾時誤判成離線，
        //     再被 host 的 stale 清除踢出成員（之後每一則 event 都會變成 `not-a-member`）。
        //     沿用 heartbeat 的投影格線，所以高頻訊息不會把 revision 打成無界成長。
        self.note_alive(index, now);
        // 5. 跨 session 注入。
        if let Some(session_id) = &envelope.session_id {
            if session_id != &self.config.session_id {
                return Err(GateFailure::rejected(ErrorCode::NotAMember));
            }
        }
        // 6. `task.*`／`runtime.*` 只有 Runtime 可送。
        if is_runtime_only_name(&envelope.name) {
            return Err(GateFailure::rejected(ErrorCode::ScopeDenied));
        }
        // 7. capability 宣告過的 inputs 才能送 event。
        if envelope.message_type == MessageType::Event
            && !self.members[index]
                .member
                .negotiated
                .inputs
                .iter()
                .any(|input| input == &envelope.name)
        {
            return Err(GateFailure::rejected(ErrorCode::ScopeDenied));
        }
        // 8. `verified` 只有 Runtime 的人類驗證路徑能產生。
        if envelope.message_type == MessageType::Result
            && envelope.payload.get("status").and_then(Value::as_str) == Some("verified")
        {
            return Err(GateFailure::rejected(ErrorCode::ScopeDenied));
        }
        // 8.1 `consentGrantId` 只出現在 host→裝置、需要授權的 command 上。成員送來的
        //     inbound 訊息一律沒有理由帶 grant：AI／adapter／裝置**不能**授予 consent
        //     （CLAUDE.md 不變量），所以 1.0 直接 `scope-denied`，不去問任何驗證器
        //     （見 `ports::ConsentVerifier` 的說明）。
        if envelope
            .consent_grant_id
            .as_ref()
            .is_some_and(|grant| !grant.is_empty())
        {
            return Err(GateFailure::rejected(ErrorCode::ScopeDenied));
        }
        // 9. member 能送的 message type（`command`／`state` 是 host 的權力）。
        if !matches!(
            envelope.message_type,
            MessageType::Event
                | MessageType::Cancel
                | MessageType::Query
                | MessageType::Result
                | MessageType::Heartbeat
                | MessageType::Capability
        ) {
            return Err(GateFailure::rejected(ErrorCode::ScopeDenied));
        }
        // 10. rate limit（token bucket，時間注入）。
        if !self.members[index]
            .bucket
            .take(now, self.config.rate_limit_per_sec)
        {
            return Err(GateFailure::rejected(ErrorCode::RateLimited));
        }
        // 11. deadline。成員自報的 `expiresAt` 是宣稱，不是授權：互動事件的有效期一律夾在
        //     `occurredAt + touch_ttl_ms` 內（§8「touch 是 expire-by-deadline」）。沒有這個夾制，
        //     一台離線幾分鐘的手機只要把 `expiresAt` 寫成一小時後，重連時排隊的舊觸摸就會被
        //     當成新鮮互動套用（reconnect-recovery-042）。
        if now >= self.effective_deadline(envelope) {
            return Err(GateFailure {
                code: ErrorCode::Expired,
                outcome: Outcome::Expired,
                audit_kind: AUDIT_REJECTED,
            });
        }
        // 12. 去重（重複回 accepted{duplicate:true}，不重套用）。這裡**只查不記**：
        //     真的被套用之後才由 `submit` 佔位（session-integrity-062）。
        if self.members[index].dedupe.contains(&envelope.message_id) {
            return Ok(Gate::Duplicate);
        }
        // 13. emergency 中不接受互動。
        if self.state.truth.state == TruthState::Emergency
            && envelope.name.starts_with("character.interaction.")
        {
            return Err(GateFailure::rejected(ErrorCode::ScopeDenied));
        }
        Ok(Gate::Proceed)
    }

    /// 這則訊息真正的有效期限：成員自報的 `expiresAt` 與 host 上界取小。
    /// `character.interaction.*` 的上界是 `occurredAt + touch_ttl_ms`；沒帶 `expiresAt`
    /// 的其他訊息沒有 deadline（回 `Timestamp::MAX` 語意的遠期時間）。
    fn effective_deadline(&self, envelope: &Envelope) -> Timestamp {
        let claimed = envelope.expires_at;
        if envelope.name.starts_with("character.interaction.") {
            let ceiling = envelope.occurred_at + Duration::milliseconds(self.config.touch_ttl_ms);
            return match claimed {
                Some(claimed) => claimed.min(ceiling),
                None => ceiling,
            };
        }
        claimed.unwrap_or(chrono::DateTime::<chrono::Utc>::MAX_UTC)
    }

    fn dispatch(
        &mut self,
        envelope: Envelope,
        bound_identity: &Party,
        now: Timestamp,
    ) -> Submission {
        match envelope.message_type {
            MessageType::Event => self.handle_event(envelope, now),
            MessageType::Cancel => self.handle_cancel(envelope, now),
            MessageType::Capability => self.handle_capability(envelope, bound_identity, now),
            MessageType::Heartbeat => self.handle_heartbeat(envelope, now),
            MessageType::Query => {
                self.bump(C_ACCEPTED);
                let outputs = vec![audit(
                    AUDIT_REPORT,
                    json!({"kind": "query", "name": safe_name(&envelope.name)}),
                )];
                let result = self.result_envelope(&envelope, Outcome::Accepted, None, false, now);
                Submission {
                    result,
                    outcome: Outcome::Accepted,
                    error: None,
                    outputs,
                    reply: false,
                }
            }
            _ => self.handle_report(envelope, bound_identity, now),
        }
    }

    /// `result`：member 回報 host 送出的 command 的進度。只記錄，不再回 result。
    ///
    /// 終態（`observed`／`rejected`／`failed`／`cancel-confirmed`）會**結清**對應的
    /// pending intent：只有真的沒人回覆的 intent 才留到 TTL 到期被稽核成 `intent-expired`。
    /// 誠實階梯不變：`observed` 是「對方說它演了」，不是 verified（`verified` 在 `gate`
    /// 第 8 關就被擋掉了）。
    fn handle_report(
        &mut self,
        envelope: Envelope,
        bound_identity: &Party,
        now: Timestamp,
    ) -> Submission {
        self.bump(C_ACCEPTED);
        let status = envelope
            .payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("received")
            .to_string();
        let mut outputs = vec![audit(
            AUDIT_REPORT,
            json!({"kind": "result", "status": status, "name": safe_name(&envelope.name)}),
        )];
        outputs.extend(self.settle_intent(&envelope, bound_identity, &status));
        let result = self.result_envelope(&envelope, Outcome::Accepted, None, false, now);
        Submission {
            result,
            outcome: Outcome::Accepted,
            error: None,
            outputs,
            reply: false,
        }
    }

    /// 把一則成員回報對應到 host 的 pending intent 並依 status 結清。
    /// 對應不到（已經結清、已過期、根本不是回報 intent）就什麼都不做——重播的終態回報
    /// 因此不會重複計數。
    ///
    /// **回報者必須真的在 `outstanding` 名單裡**：`causationId`／`correlationId` 是任何成員
    /// 都看得到、也編得出來的識別字，不是授權。沒有這一關的話，一台從未收到 command 的裝置
    /// 只要用自己那則 touch 的 messageId 當 `correlationId`，就能把 intent 結清、把
    /// `intents.observed` 灌上去，還順手吃掉之後的 `intent-expired` 稽核
    /// （capability-consent-050／session-integrity-057／evidence-honesty-010）。
    /// 已經回報過終態的目標會被移出 `outstanding`，所以同一個目標重播也只會計一次。
    fn settle_intent(
        &mut self,
        envelope: &Envelope,
        reporter: &Party,
        status: &str,
    ) -> Vec<Output> {
        let Some(index) = self
            .pending
            .iter()
            .position(|pending| pending.answers(envelope))
        else {
            return Vec::new();
        };
        if !self.pending[index]
            .outstanding
            .iter()
            .any(|party| party == reporter)
        {
            self.bump(C_INTENTS_UNSOLICITED);
            return vec![audit(
                AUDIT_INTENT_UNSOLICITED,
                json!({
                    "intent": self.pending[index].intent.intent,
                    "status": status,
                    "party": safe_party(reporter),
                }),
            )];
        }
        if !terminal_report(status) {
            // `accepted`／`acknowledged` 只是「收到了」：更新狀態，intent 繼續掛著。
            self.pending[index].last_status = Some(status.to_string());
            return Vec::new();
        }
        let intent_name = self.pending[index].intent.intent.clone();
        self.pending[index].last_status = Some(status.to_string());
        self.pending[index]
            .outstanding
            .retain(|party| party != reporter);
        let settled = self.pending[index].outstanding.is_empty();
        if settled {
            self.pending.remove(index);
        }
        if let Some(counter) = report_counter(status) {
            self.bump(counter);
        }
        vec![audit(
            AUDIT_INTENT_SETTLED,
            json!({
                "intent": intent_name,
                "status": status,
                "party": safe_party(reporter),
                "settled": settled,
            }),
        )]
    }

    fn handle_event(&mut self, envelope: Envelope, now: Timestamp) -> Submission {
        let correlation = envelope
            .correlation_id
            .clone()
            .unwrap_or_else(|| envelope.message_id.clone());
        let kind = if envelope.name == EVENT_DISMISS {
            "dismiss".to_string()
        } else {
            envelope
                .payload
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        if envelope.name == EVENT_TOUCH && TouchKind::parse(&kind).is_none() {
            return self.rejection(
                &envelope,
                GateFailure::rejected(ErrorCode::SchemaInvalid),
                now,
            );
        }
        let event = InteractionEvent {
            name: envelope.name.clone(),
            kind,
            intensity: envelope.payload.get("intensity").and_then(Value::as_f64),
            source: envelope.source.clone(),
            correlation_id: correlation,
            at: now,
        };
        let Some((patch, intents)) = director::react(&self.state, &event, &self.config, now) else {
            return self.rejection(
                &envelope,
                GateFailure::rejected(ErrorCode::UnknownName),
                now,
            );
        };
        if !self.apply_semantic_patch(&patch) {
            return self.rejection(&envelope, GateFailure::rejected(ErrorCode::Internal), now);
        }
        self.bump(C_ACCEPTED);
        self.bump(C_APPLIED);
        let mut outputs = Vec::new();
        if let Some(state_envelope) = self.commit_and_patch(now) {
            outputs.push(Output::Broadcast {
                envelope: state_envelope,
                except: None,
            });
        }
        self.sync_reacting_clock(now);
        for intent in intents {
            outputs.extend(self.emit_intent(intent, now));
        }
        outputs.push(audit(
            AUDIT_APPLIED,
            json!({
                "name": safe_name(&envelope.name),
                "messageId": safe_id(&envelope.message_id),
                "source": safe_party(&envelope.source),
                "revision": self.revision,
            }),
        ));
        if let Some(persist) = self.persist_if_due(now) {
            outputs.push(persist);
        }
        let result = self.result_envelope(&envelope, Outcome::Applied, None, false, now);
        Submission {
            result,
            outcome: Outcome::Applied,
            error: None,
            outputs,
            reply: true,
        }
    }

    fn handle_cancel(&mut self, envelope: Envelope, now: Timestamp) -> Submission {
        let target = envelope.causation_id.clone().or_else(|| {
            envelope
                .payload
                .get("messageId")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        let correlation = envelope.correlation_id.clone();
        let matches_target = |pending: &PendingIntent| {
            correlation.as_deref() == Some(pending.intent.correlation_id.as_str())
                || target.as_deref() == Some(pending.intent.correlation_id.as_str())
                || target
                    .as_deref()
                    .is_some_and(|id| pending.commands.iter().any(|sent| sent == id))
        };
        // 撤銷只送給**真的拿到過 command 的那些成員**（`outstanding`），不是「現在剛好符合
        // 條件的 renderer」：後者會漏掉已經在演、但協商結果剛被改掉的目標。
        let cancelled: Vec<PendingIntent> = self
            .pending
            .iter()
            .filter(|pending| matches_target(pending))
            .cloned()
            .collect();
        self.pending.retain(|pending| !matches_target(pending));
        let mut outputs = Vec::new();
        for entry in &cancelled {
            for party in &entry.outstanding {
                let envelope = self.cancel_envelope(party, &entry.intent, now);
                outputs.push(Output::Send {
                    to: party.clone(),
                    envelope,
                });
            }
        }
        outputs.push(audit(
            AUDIT_CANCEL,
            json!({"cancelled": cancelled.len(), "messageId": safe_id(&envelope.message_id)}),
        ));
        self.bump(C_ACCEPTED);
        let mut result =
            self.result_envelope(&envelope, Outcome::CancelConfirmed, None, false, now);
        if cancelled.is_empty() {
            if let Value::Object(map) = &mut result.payload {
                map.insert("alreadyTerminal".into(), Value::Bool(true));
            }
        }
        Submission {
            result,
            outcome: Outcome::CancelConfirmed,
            error: None,
            outputs,
            reply: true,
        }
    }

    fn handle_capability(
        &mut self,
        envelope: Envelope,
        bound_identity: &Party,
        now: Timestamp,
    ) -> Submission {
        let announcement: CapabilityAnnouncement =
            match serde_json::from_value(envelope.payload.clone()) {
                Ok(value) => value,
                Err(_) => {
                    return self.rejection(
                        &envelope,
                        GateFailure::rejected(ErrorCode::SchemaInvalid),
                        now,
                    )
                }
            };
        match self.join(bound_identity.clone(), &announcement, now) {
            Ok(joined) => {
                let mut outputs = joined.outputs;
                outputs.push(Output::Send {
                    to: bound_identity.clone(),
                    envelope: joined.capability_envelope,
                });
                outputs.push(Output::Send {
                    to: bound_identity.clone(),
                    envelope: joined.snapshot_envelope,
                });
                self.bump(C_ACCEPTED);
                self.bump(C_APPLIED);
                let result = self.result_envelope(&envelope, Outcome::Applied, None, false, now);
                Submission {
                    result,
                    outcome: Outcome::Applied,
                    error: None,
                    outputs,
                    reply: true,
                }
            }
            Err(error) => self.rejection(&envelope, GateFailure::rejected(error.code()), now),
        }
    }

    fn handle_heartbeat(&mut self, envelope: Envelope, now: Timestamp) -> Submission {
        // presence 已經由 `gate` 的存活證明更新過（heartbeat 不再是唯一的證明），
        // 廣播則交給 `submit` 收尾的那一次 commit。
        let outputs = Vec::new();
        self.bump(C_ACCEPTED);
        let result = self.result_envelope(&envelope, Outcome::Applied, None, false, now);
        Submission {
            result,
            outcome: Outcome::Applied,
            error: None,
            outputs,
            reply: false,
        }
    }

    fn rejection(
        &mut self,
        envelope: &Envelope,
        failure: GateFailure,
        now: Timestamp,
    ) -> Submission {
        self.bump(&format!("rejected.{}", failure.code.as_str()));
        if failure.code == ErrorCode::IdentityMismatch {
            self.bump(C_IDENTITY_MISMATCH);
        }
        if failure.outcome == Outcome::Expired {
            self.bump(C_EXPIRED);
        }
        let detail = json!({
            "code": failure.code.as_str(),
            "messageId": safe_id(&envelope.message_id),
            "name": safe_name(&envelope.name),
            "source": safe_party(&envelope.source),
        });
        let result =
            self.result_envelope(envelope, failure.outcome, Some(&failure.code), false, now);
        Submission {
            result,
            outcome: failure.outcome,
            error: Some(failure.code),
            outputs: vec![audit(failure.audit_kind, detail)],
            reply: true,
        }
    }

    // ---------------------------------------------------------------- 狀態

    /// 把 patch 套在**目前**的 `self.state` 上（不是上次 commit 的 `state_json`）：
    /// `project_members` 這類直接改 state 的動作可能還沒 commit，用舊 JSON 當底會把它們吃掉。
    fn apply_semantic_patch(&mut self, patch: &Value) -> bool {
        let candidate = apply_patch(&state_value(&self.state), patch);
        match serde_json::from_value::<SemanticState>(candidate) {
            Ok(next) if !next.violates_limits(self.config.max_members) => {
                self.state = next;
                true
            }
            _ => {
                self.bump(C_INTERNAL);
                false
            }
        }
    }

    /// 把成員投影進 `SemanticState`。`lastSeenAt` 走投影格線（presence timeout 的三分之一），
    /// presence 變化則一律即時反映——「感測不靜默」是對 presence 的要求，不是對毫秒精度的要求。
    fn project_members(&mut self) {
        let grid = Duration::milliseconds((self.config.presence_timeout_ms / 3).max(1));
        for entry in self.members.iter_mut() {
            if entry
                .member
                .last_seen_at
                .signed_duration_since(entry.projected_seen_at)
                >= grid
            {
                entry.projected_seen_at = entry.member.last_seen_at;
            }
        }
        self.state.members = self
            .members
            .iter()
            .map(|entry| MemberView {
                last_seen_at: entry.projected_seen_at,
                ..entry.member.view()
            })
            .collect();
    }

    /// 記下「這個成員剛剛證明自己還在」。`lastSeenAt` 走投影格線（見 [`Self::project_members`]），
    /// presence 的變化即時反映。呼叫端負責在之後 commit 一次（`submit` 收尾會做）。
    fn note_alive(&mut self, index: usize, now: Timestamp) {
        let entry = &mut self.members[index];
        // 時鐘倒退時不把 `lastSeenAt` 拉回過去（否則會憑空製造一次逾時）。
        if now > entry.member.last_seen_at {
            entry.member.last_seen_at = now;
        }
        if entry.member.presence != Presence::Online {
            entry.member.presence = Presence::Online;
            entry.projected_seen_at = entry.member.last_seen_at;
        }
        self.project_members();
    }

    fn sync_reacting_clock(&mut self, now: Timestamp) {
        self.reacting_since = (self.state.activity == Activity::Reacting).then_some(now);
    }

    /// 把 `self.state` 的變更記成 revision＋日誌，回傳要廣播的 patch envelope（沒變就 `None`）。
    fn commit_and_patch(&mut self, now: Timestamp) -> Option<Envelope> {
        let next_json = state_value(&self.state);
        let patch = merge_diff(&self.state_json, &next_json);
        if matches!(&patch, Value::Object(map) if map.is_empty()) {
            return None;
        }
        let base_revision = self.revision;
        self.revision += 1;
        self.state_json = next_json;
        self.updated_at = now;
        let hash = state_hash(&self.state_json);
        let sequence = self.next_sequence();
        let message_id = self.next_message_id(now);
        self.log.push(LogEntry {
            message_id: message_id.clone(),
            sequence,
            base_revision,
            revision: self.revision,
            patch: patch.clone(),
            hash: hash.clone(),
            at: now,
        });
        self.bump(C_PATCHES);
        Some(
            Envelope::new(
                MessageType::State,
                NAME_SESSION_PATCH,
                Party::runtime(),
                message_id,
                now,
            )
            .with_session(self.config.session_id.clone())
            .with_sequence(sequence)
            .with_base_revision(base_revision)
            .with_payload(json!({
                "kind": "patch",
                "revision": self.revision,
                "patch": patch,
                "hash": hash,
                "sessionEpoch": self.epoch,
            })),
        )
    }

    fn replay_envelope(&self, entry: &LogEntry, to: &Party) -> Envelope {
        Envelope::new(
            MessageType::State,
            NAME_SESSION_PATCH,
            Party::runtime(),
            entry.message_id.clone(),
            entry.at,
        )
        .with_session(self.config.session_id.clone())
        .with_target(to.clone())
        .with_sequence(entry.sequence)
        .with_base_revision(entry.base_revision)
        .with_payload(json!({
            "kind": "patch",
            "revision": entry.revision,
            "patch": entry.patch,
            "hash": entry.hash,
            "sessionEpoch": self.epoch,
        }))
    }

    fn snapshot_envelope_inner(
        &mut self,
        to: &Party,
        now: Timestamp,
        reason: Option<&str>,
    ) -> Envelope {
        let sequence = self.next_sequence();
        let message_id = self.next_message_id(now);
        let mut payload = Map::new();
        payload.insert("kind".into(), Value::String("snapshot".into()));
        payload.insert("revision".into(), json!(self.revision));
        payload.insert("sequence".into(), json!(sequence));
        payload.insert("state".into(), self.state_json.clone());
        payload.insert("hash".into(), Value::String(state_hash(&self.state_json)));
        payload.insert("sessionEpoch".into(), json!(self.epoch));
        if let Some(reason) = reason {
            payload.insert("reason".into(), Value::String(reason.to_string()));
        }
        self.bump(C_SNAPSHOTS);
        Envelope::new(
            MessageType::State,
            NAME_SESSION_SNAPSHOT,
            Party::runtime(),
            message_id,
            now,
        )
        .with_session(self.config.session_id.clone())
        .with_target(to.clone())
        .with_sequence(sequence)
        .with_payload(Value::Object(payload))
    }

    /// host→member 的協商結果。**host 自己送出的訊息也必須通過 `validate()`**：協商結果的
    /// `unsupportedInputs` 來自對方宣告的 inputs，超限時寧可降級成一則只帶計數的摘要，
    /// 也不送一則違反 §11 的訊息出去（session-integrity-060）。
    fn capability_envelope(
        &mut self,
        to: &Party,
        negotiated: &NegotiatedCapabilities,
        now: Timestamp,
    ) -> Envelope {
        let message_id = self.next_message_id(now);
        let build = |payload: Value| {
            Envelope::new(
                MessageType::Capability,
                NAME_SESSION_CAPABILITY,
                Party::runtime(),
                message_id.clone(),
                now,
            )
            .with_session(self.config.session_id.clone())
            .with_target(to.clone())
            .with_payload(payload)
        };
        let full = serde_json::to_value(negotiated).unwrap_or_else(|_| Value::Object(Map::new()));
        let envelope = build(full);
        if envelope.validate().is_ok() {
            return envelope;
        }
        // 降級 1：丟掉兩份字串清單，只留計數。
        let mut reduced = negotiated.clone();
        let unsupported_inputs = reduced.unsupported_inputs.len();
        reduced.unsupported_inputs = Vec::new();
        reduced.inputs = Vec::new();
        let mut payload =
            serde_json::to_value(&reduced).unwrap_or_else(|_| Value::Object(Map::new()));
        if let Value::Object(map) = &mut payload {
            map.insert("truncated".into(), Value::Bool(true));
            map.insert("unsupportedInputsCount".into(), json!(unsupported_inputs));
        }
        let envelope = build(payload);
        if envelope.validate().is_ok() {
            return envelope;
        }
        // 降級 2：只留角色與版本（對方至少知道自己被協商成什麼）。
        build(json!({
            "specVersion": reduced.spec_version,
            "role": reduced.role,
            "syncClass": reduced.sync_class,
            "truncated": true,
        }))
    }

    fn result_envelope(
        &mut self,
        source: &Envelope,
        outcome: Outcome,
        code: Option<&ErrorCode>,
        duplicate: bool,
        now: Timestamp,
    ) -> Envelope {
        let message_id = self.next_message_id(now);
        let name = if is_valid_name(&source.name) {
            source.name.clone()
        } else {
            NAME_SESSION_RESULT.to_string()
        };
        let mut payload = Map::new();
        payload.insert("status".into(), Value::String(outcome.as_str().to_string()));
        if let Some(code) = code {
            payload.insert("code".into(), Value::String(code.as_str().to_string()));
            payload.insert("retryable".into(), Value::Bool(code.retryable()));
        }
        if duplicate {
            payload.insert("duplicate".into(), Value::Bool(true));
        }
        let mut envelope =
            Envelope::new(MessageType::Result, name, Party::runtime(), message_id, now)
                .with_session(self.config.session_id.clone())
                .with_causation(safe_id(&source.message_id))
                .with_payload(Value::Object(payload));
        if let Some(correlation) = source.correlation_id.as_ref().filter(|c| is_safe_id(c)) {
            envelope = envelope.with_correlation(correlation.clone());
        }
        envelope
    }

    // ------------------------------------------------------------ Intent

    fn emit_intent(&mut self, intent: BehaviorIntent, now: Timestamp) -> Vec<Output> {
        let mut outputs = Vec::new();
        // 先組 command（才知道每個目標拿到的 messageId），再掛 pending：成員的
        // `result{causationId}` 靠這些 id 對回來。
        let targets = self.intent_targets(&intent.intent);
        let mut commands = Vec::new();
        let mut sends = Vec::new();
        for party in targets.iter() {
            let envelope = self.command_envelope(party, &intent, now);
            commands.push(envelope.message_id.clone());
            sends.push(Output::Send {
                to: party.clone(),
                envelope,
            });
        }
        if targets.is_empty() {
            // §8 `character.behavior.*` 是 drop-if-offline：沒有任何遠端目標時這則 intent
            // **從未被派送**。把它掛進 pending 等 TTL 會誠實階梯倒退——`intents.expired`
            // 與 `intent-expired` 稽核講的是「送出去了卻沒人回報」，不是「沒有人被問過」
            // （capability-consent-053／reconnect-recovery-043）。host 端 renderer 仍照收
            // 下面的 `Output::RendererIntent`。
            self.bump(C_INTENTS_DROPPED);
            outputs.push(audit(
                AUDIT_INTENT_DROPPED,
                json!({"intent": intent.intent, "reason": "no-online-renderer"}),
            ));
        } else {
            self.push_pending(
                PendingIntent {
                    intent: intent.clone(),
                    outstanding: targets,
                    commands,
                    last_status: None,
                },
                &mut outputs,
            );
        }
        outputs.extend(sends);
        self.bump(C_INTENTS_EMITTED);
        outputs.push(Output::RendererIntent {
            cpp: behavior_to_cpp(&intent),
            intent,
        });
        outputs
    }

    /// 協商為 `exact` 且 online 的 remote-renderer。§8 `character.behavior.*` 是 drop-if-offline。
    fn intent_targets(&self, intent: &str) -> Vec<Party> {
        self.members
            .iter()
            .filter(|entry| {
                entry.member.role == MemberRole::RemoteRenderer
                    && entry.member.presence == Presence::Online
                    && entry.member.negotiated.intents.get(intent) == Some(&IntentSupport::Exact)
            })
            .map(|entry| entry.member.party.clone())
            .collect()
    }

    fn push_pending(&mut self, pending: PendingIntent, outputs: &mut Vec<Output>) {
        if self.pending.len() >= MAX_PENDING_INTENTS {
            if let Some(dropped) = self.pending.pop_front() {
                self.bump(C_INTENTS_DROPPED);
                outputs.push(audit(
                    AUDIT_INTENT_DROPPED,
                    json!({"intent": dropped.intent.intent, "reason": "pending-queue-full"}),
                ));
            }
        }
        self.pending.push_back(pending);
    }

    fn command_envelope(
        &mut self,
        to: &Party,
        intent: &BehaviorIntent,
        now: Timestamp,
    ) -> Envelope {
        let message_id = self.next_message_id(now);
        let sequence = self.next_sequence();
        Envelope::new(
            MessageType::Command,
            NAME_BEHAVIOR_REQUEST,
            Party::runtime(),
            message_id,
            now,
        )
        .with_session(self.config.session_id.clone())
        .with_target(to.clone())
        .with_correlation(safe_id(&intent.correlation_id))
        .with_expiry(intent.expires_at)
        .with_sequence(sequence)
        .with_payload(intent.payload())
    }

    fn cancel_envelope(&mut self, to: &Party, intent: &BehaviorIntent, now: Timestamp) -> Envelope {
        let message_id = self.next_message_id(now);
        let sequence = self.next_sequence();
        Envelope::new(
            MessageType::Command,
            NAME_BEHAVIOR_CANCEL,
            Party::runtime(),
            message_id,
            now,
        )
        .with_session(self.config.session_id.clone())
        .with_target(to.clone())
        .with_correlation(safe_id(&intent.correlation_id))
        .with_expiry(now + Duration::milliseconds(self.config.intent_ttl_ms))
        .with_sequence(sequence)
        .with_payload(json!({"intent": intent.intent}))
    }

    // ------------------------------------------------------------- 小工具

    fn index_of(&self, party: &Party) -> Option<usize> {
        self.members
            .iter()
            .position(|entry| &entry.member.party == party)
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    /// Host 送出的 messageId：`aip-<epoch>-<epochMillis>-<n>`，長度遠低於 `MAX_ID_CHARS`。
    fn next_message_id(&mut self, now: Timestamp) -> String {
        self.emitted += 1;
        let id = format!(
            "aip-{}-{}-{}",
            self.epoch,
            now.timestamp_millis(),
            self.emitted
        );
        if id.chars().count() > limits::MAX_ID_CHARS {
            return format!("aip-{}-{}", self.epoch, self.emitted);
        }
        id
    }

    fn persist_if_due(&mut self, now: Timestamp) -> Option<Output> {
        if self.revision == self.last_persist_revision {
            return None;
        }
        let by_revision =
            self.revision >= self.last_persist_revision + self.config.persist_every_revisions;
        let by_time = now.signed_duration_since(self.last_persist_at)
            >= Duration::milliseconds(self.config.persist_interval_ms);
        if !by_revision && !by_time {
            return None;
        }
        self.last_persist_at = now;
        self.last_persist_revision = self.revision;
        Some(Output::Persist(self.snapshot()))
    }

    fn bump(&mut self, key: &str) {
        self.bump_by(key, 1);
    }

    fn bump_by(&mut self, key: &str, amount: u64) {
        let entry = self.counters.entry(key.to_string()).or_insert(0);
        *entry = entry.saturating_add(amount);
    }
}

// ---------------------------------------------------------------- 內部型別

enum Gate {
    Proceed,
    Duplicate,
}

struct GateFailure {
    code: ErrorCode,
    outcome: Outcome,
    audit_kind: &'static str,
}

impl GateFailure {
    fn rejected(code: ErrorCode) -> Self {
        Self {
            code,
            outcome: Outcome::Rejected,
            audit_kind: AUDIT_REJECTED,
        }
    }
}

/// Host 私有的待決 Behavior Intent 紀錄（不進 `SemanticState`）。
///
/// `outstanding` 是還沒回報終態的目標成員；空了就代表這個 intent 已經結清，
/// 不必再等 TTL。`commands` 是送出去的 command messageId，成員的
/// `result{causationId}` 靠它對回來。
#[derive(Debug, Clone)]
struct PendingIntent {
    intent: BehaviorIntent,
    outstanding: Vec<Party>,
    commands: Vec<String>,
    /// 最後一次收到的成員回報（`accepted`／`acknowledged` 這類非終態也記下來）。
    last_status: Option<String>,
}

impl PendingIntent {
    /// 這則 `result` 是在回報這個 intent 嗎？
    /// `causationId` 對到送出去的 command messageId 最精確；`correlationId` 是備援
    /// （AIP §6：correlation 貫穿一次互動）。
    fn answers(&self, envelope: &Envelope) -> bool {
        if let Some(causation) = envelope.causation_id.as_deref() {
            if self.commands.iter().any(|sent| sent == causation)
                || causation == self.intent.correlation_id
            {
                return true;
            }
        }
        envelope.correlation_id.as_deref() == Some(self.intent.correlation_id.as_str())
    }
}

#[derive(Debug, Clone)]
struct MemberRuntime {
    member: Member,
    /// 已經投影進 `SemanticState` 的 `lastSeenAt`。heartbeat 只在超過投影格線時才更新它，
    /// 否則一個每秒 30 則 heartbeat 的成員就能逼出每秒 30 個 revision 與 30 則廣播。
    projected_seen_at: Timestamp,
    dedupe: DedupeRing,
    bucket: RateBucket,
}

impl MemberRuntime {
    fn new(member: Member, rate_limit_per_sec: u32, now: Timestamp) -> Self {
        let projected_seen_at = member.last_seen_at;
        Self {
            member,
            projected_seen_at,
            dedupe: DedupeRing::default(),
            bucket: RateBucket::new(rate_limit_per_sec, now),
        }
    }
}

/// Token bucket（時間注入，無 sleep、無背景任務）。
#[derive(Debug, Clone)]
struct RateBucket {
    tokens: f64,
    last: Timestamp,
}

impl RateBucket {
    fn new(per_sec: u32, now: Timestamp) -> Self {
        Self {
            tokens: f64::from(per_sec.max(1)),
            last: now,
        }
    }

    fn take(&mut self, now: Timestamp, per_sec: u32) -> bool {
        let capacity = f64::from(per_sec.max(1));
        let elapsed_ms = now.signed_duration_since(self.last).num_milliseconds();
        if elapsed_ms > 0 {
            self.tokens = (self.tokens + (elapsed_ms as f64) * capacity / 1_000.0).min(capacity);
            self.last = now;
        } else if elapsed_ms < 0 {
            // 時鐘倒退：不補 token，只把基準拉回來（不給白吃的額度）。
            self.last = now;
        }
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// 有界事件日誌（環）。滿了就淘汰最舊的，resume 時自動 snapshot fallback。
#[derive(Debug, Clone)]
struct EventLog {
    cap: usize,
    entries: VecDeque<LogEntry>,
}

impl EventLog {
    fn new(cap: usize) -> Self {
        Self {
            cap: cap.clamp(1, limits::EVENT_LOG_RING),
            entries: VecDeque::new(),
        }
    }

    fn push(&mut self, entry: LogEntry) {
        while self.entries.len() >= self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }
}

// ------------------------------------------------------------------ 純函式

fn audit(kind: &str, detail: Value) -> Output {
    Output::Audit {
        kind: kind.to_string(),
        detail,
    }
}

/// [`SemanticState`] 只含有限的 f64 與字串，序列化不會失敗；萬一失敗回空物件而不是 panic。
fn state_value(state: &SemanticState) -> Value {
    serde_json::to_value(state).unwrap_or_else(|_| Value::Object(Map::new()))
}

/// 依 Transport 綁定身分夾住成員自報的 `role`（identity-binding-007）。
///
/// `host-renderer` 是「可信人類操作面上的 renderer」——它拿得到 host overlay、也是安全訊息的
/// 落點，因此只有 `human-surface` 身分能擔任。device／renderer／agent 自報 host-renderer 一律
/// 降級成 `remote-renderer`：這樣共享狀態裡的 `role` 與 `intent_targets()` 的實際派送名單一致，
/// 一般模式不會因為一個宣稱就顯示「已同步」。其餘角色（remote-renderer／input-device／observer）
/// 不需要額外權力，照對方宣告採用。
fn effective_role(kind: &PartyKind, claimed: MemberRole) -> MemberRole {
    match claimed {
        MemberRole::HostRenderer if *kind != PartyKind::HumanSurface => MemberRole::RemoteRenderer,
        other => other,
    }
}

/// `occurredAt` 與 host 時鐘的偏差超過 §11 上限時的稽核（只記名稱與偏差毫秒數）。
fn clock_skew_audit(envelope: &Envelope, now: Timestamp) -> Option<Output> {
    let skew_ms = now
        .signed_duration_since(envelope.occurred_at)
        .num_milliseconds();
    (skew_ms.abs() > limits::MAX_CLOCK_SKEW_MS).then(|| {
        audit(
            AUDIT_CLOCK_SKEW,
            json!({
                "name": safe_name(&envelope.name),
                "messageId": safe_id(&envelope.message_id),
                "skewMs": skew_ms,
                "maxMs": limits::MAX_CLOCK_SKEW_MS,
            }),
        )
    })
}

/// 沒有協商過的成員（restore 之後）：不能呈現任何 intent、不能送任何 event（§12.10）。
///
/// `intents` 明確寫成「每個 host intent 都 unsupported」而不是空表：兩者對派送的效果一樣，
/// 但只有前者投影得出 `members[].unsupportedIntents`——空表會被讀成「沒有任何不支援的能力」，
/// 那是一句 host 現在證明不了的話（誠實階梯：不知道不等於做得到）。
fn unnegotiated(role: MemberRole) -> NegotiatedCapabilities {
    NegotiatedCapabilities {
        spec_version: interaction_aip::SPEC_VERSION.to_string(),
        newer_minor: false,
        role,
        sync_class: SyncClass::Semantic,
        intents: HOST_INTENTS
            .iter()
            .map(|name| (name.to_string(), IntentSupport::Unsupported))
            .collect(),
        inputs: Vec::new(),
        unsupported_inputs: Vec::new(),
        limits: interaction_aip::CapabilityLimits {
            max_message_bytes: Some(limits::MAX_MESSAGE_BYTES),
        },
        unsupported_inputs_truncated: false,
    }
}

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= limits::MAX_ID_CHARS
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// 稽核／causationId 用的識別字：只保留合法字元並截斷，永遠不回顯 payload 內容。
fn safe_id(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .take(limits::MAX_ID_CHARS)
        .collect();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

fn safe_name(value: &str) -> String {
    if is_valid_name(value) {
        value.to_string()
    } else {
        "invalid-name".to_string()
    }
}

fn safe_party(party: &Party) -> String {
    let kind = match serde_json::to_value(&party.kind) {
        Ok(Value::String(s)) => s,
        _ => "unknown".to_string(),
    };
    format!("{}:{}", kind, safe_id(&party.id))
}
