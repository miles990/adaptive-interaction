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
    NAME_SESSION_PATCH, NAME_SESSION_RESULT, NAME_SESSION_SNAPSHOT, REASON_SESSION_RESET,
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
const AUDIT_CANCEL: &str = "character.session.cancel";
const AUDIT_REPORT: &str = "character.session.report";
const AUDIT_INTERNAL: &str = "aip.internal";

// counters 的固定鍵（有界：`rejected.*` 只會出現在已知的 19 個錯誤碼上）。
const C_ACCEPTED: &str = "accepted";
const C_APPLIED: &str = "applied";
const C_DUPLICATES: &str = "duplicates";
const C_EXPIRED: &str = "expired";
const C_RESUMES: &str = "resumes";
const C_SNAPSHOTS: &str = "snapshots";
const C_PATCHES: &str = "patches";
const C_IDENTITY_MISMATCH: &str = "identity_mismatch";
const C_INTENTS_EMITTED: &str = "intents.emitted";
const C_INTENTS_EXPIRED: &str = "intents.expired";
const C_INTENTS_DROPPED: &str = "intents.dropped";
const C_INTENTS_OBSERVED: &str = "intents.observed";
const C_INTENTS_REJECTED: &str = "intents.rejected";
const C_INTENTS_FAILED: &str = "intents.failed";
const C_INTERNAL: &str = "internal";

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
        }
    }

    /// 從持久化 snapshot 續接（§1：重啟後 revision 不歸零）。hash 不符或狀態違反不變量 → `Err`。
    ///
    /// 還原出來的成員保留 presence 投影，但**沒有**協商結果：他們必須重送 `capability`
    /// （§7 重連流程第 2 步）才能再送 event，否則會拿到 `scope-denied`。
    pub fn restore(
        config: SessionConfig,
        snapshot: &Snapshot,
        now: Timestamp,
    ) -> Result<Self, SessionError> {
        let config = config.normalized();
        if snapshot.session_id != config.session_id {
            return Err(SessionError::SessionMismatch);
        }
        if state_hash(&snapshot.state) != snapshot.hash {
            return Err(SessionError::HashMismatch);
        }
        let state: SemanticState = serde_json::from_value(snapshot.state.clone())
            .map_err(|_| SessionError::InvalidState)?;
        if state.violates_limits(config.max_members) {
            return Err(SessionError::InvalidState);
        }
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
        Ok(Self {
            config,
            epoch: snapshot.epoch,
            revision: snapshot.revision,
            sequence: snapshot.sequence,
            // messageId 空間跟著已持久化的 sequence 走，避免重啟後與舊訊息撞號。
            emitted: snapshot.sequence,
            state_json: snapshot.state.clone(),
            state,
            members,
            log,
            pending: VecDeque::new(),
            counters: BTreeMap::new(),
            reacting_since,
            updated_at: snapshot.at,
            last_persist_at: now,
            last_persist_revision: snapshot.revision,
        })
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
        let negotiated = negotiate_capabilities(&self.host_offer(), announcement)
            .map_err(SessionError::Negotiation)?;
        let existing = self.index_of(&party);
        if existing.is_none() && self.members.len() >= self.config.max_members {
            return Err(SessionError::MembersFull);
        }
        match existing {
            Some(index) => {
                let entry = &mut self.members[index];
                entry.member.role = negotiated.role;
                entry.member.presence = Presence::Online;
                entry.member.last_seen_at = now;
                entry.member.negotiated = negotiated.clone();
                entry.projected_seen_at = now;
                entry.bucket = RateBucket::new(self.config.rate_limit_per_sec, now);
            }
            None => self.members.push(MemberRuntime::new(
                Member {
                    party: party.clone(),
                    role: negotiated.role,
                    presence: Presence::Online,
                    last_seen_at: now,
                    negotiated: negotiated.clone(),
                },
                self.config.rate_limit_per_sec,
                now,
            )),
        }
        self.project_members();
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
                "members": self.members.len(),
            }),
        ));
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
            Ok(Gate::Proceed) => self.dispatch(envelope, bound_identity, now),
        };
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
        if matches!(fact, RuntimeFact::Emergency { engaged: true }) {
            cancelled = self.pending.len();
            self.pending.clear();
        }
        if let Some(envelope) = self.commit_and_patch(now) {
            outputs.push(Output::Broadcast {
                envelope,
                except: None,
            });
        }
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
        if epoch != self.epoch {
            let envelope = self.snapshot_envelope_inner(party, now, Some(REASON_SESSION_RESET));
            return Resume::EpochMismatch { envelope };
        }
        // 對方宣稱看過的進度超前 host → 只能用 snapshot 對齊。
        if last_revision > self.revision || last_sequence > self.sequence {
            return Resume::Snapshot {
                envelope: self.snapshot_envelope_inner(party, now, None),
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
        // 11. deadline。
        if envelope.is_expired(now) {
            return Err(GateFailure {
                code: ErrorCode::Expired,
                outcome: Outcome::Expired,
                audit_kind: AUDIT_REJECTED,
            });
        }
        // 12. 去重（重複回 accepted{duplicate:true}，不重套用）。
        if !self.members[index].dedupe.note(&envelope.message_id) {
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
        let cancelled: Vec<BehaviorIntent> = self
            .pending
            .iter()
            .filter(|pending| matches_target(pending))
            .map(|pending| pending.intent.clone())
            .collect();
        self.pending.retain(|pending| !matches_target(pending));
        let mut outputs = Vec::new();
        for intent in &cancelled {
            for party in self.intent_targets(&intent.intent) {
                let envelope = self.cancel_envelope(&party, intent, now);
                outputs.push(Output::Send {
                    to: party,
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

    fn capability_envelope(
        &mut self,
        to: &Party,
        negotiated: &NegotiatedCapabilities,
        now: Timestamp,
    ) -> Envelope {
        let message_id = self.next_message_id(now);
        let payload =
            serde_json::to_value(negotiated).unwrap_or_else(|_| Value::Object(Map::new()));
        Envelope::new(
            MessageType::Capability,
            NAME_SESSION_CAPABILITY,
            Party::runtime(),
            message_id,
            now,
        )
        .with_session(self.config.session_id.clone())
        .with_target(to.clone())
        .with_payload(payload)
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
        self.push_pending(
            PendingIntent {
                intent: intent.clone(),
                outstanding: targets,
                commands,
                last_status: None,
            },
            &mut outputs,
        );
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

/// 沒有協商過的成員（restore 之後）：不能呈現任何 intent、不能送任何 event。
fn unnegotiated(role: MemberRole) -> NegotiatedCapabilities {
    NegotiatedCapabilities {
        spec_version: interaction_aip::SPEC_VERSION.to_string(),
        newer_minor: false,
        role,
        sync_class: SyncClass::Semantic,
        intents: BTreeMap::new(),
        inputs: Vec::new(),
        unsupported_inputs: Vec::new(),
        limits: interaction_aip::CapabilityLimits {
            max_message_bytes: Some(limits::MAX_MESSAGE_BYTES),
        },
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
