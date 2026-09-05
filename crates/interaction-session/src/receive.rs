//! AIP 1.0 **接收端決策表**（純函式）。
//!
//! 契約：`docs/aip/character-session.md` §6／§7（「AIP 1.0 接收端澄清（2026-09-05，v0.7.0）」）。
//! wire 沒有變：這個模組只是把「收到一則 `state` 之後到底要做什麼」寫成一張三端共用、
//! 可執行的表，取代原本散在 Rust／TypeScript／Swift 三處、彼此不一致的 if-else。
//!
//! # 為什麼要一張表
//!
//! 同一則訊息，桌面回 realign、iPhone 直接套用、Rust 靜默改寫本地 epoch——三個畫面
//! 都寫著「已同步」，分歧卻沒有任何一端看得見（對抗審查 `427c806` 的第三處差異）。
//! 決策順序本身就是安全邊界：先判連線世代、再判身分、再判格式，最後才輪到 revision。
//!
//! # 輸入的前提（typed boundary）
//!
//! 進到這裡的東西**已經**通過 envelope 檢查：`messageType == state`、`revision` 與
//! `sessionEpoch` 是非負整數、payload 大小／深度合法。錯誤格式與超大訊息由 boundary 擋下
//! （`interaction_aip::Envelope::parse`／`validate`），不是這張表的工作；boundary 擋下來的
//! 東西若來自一則權威回覆，呼叫端要記成一次 realign 失敗（見 [`RealignBudget`]）。
//!
//! # 表（第一個命中即決定）
//!
//! | # | 條件 | 決策 |
//! |---|---|---|
//! | 0 | `arrived_on_generation` 不是現行連線／請求世代 | [`ReceiveDecision::IgnoreStaleConnection`] |
//! | 1 | incoming 有 sessionId、local 有狀態且 sessionId **已知**、且兩者不同 | [`ReceiveDecision::RejectIdentity`] |
//! | 2 | snapshot 缺 hash 或缺 state（patch 缺 baseRevision） | [`ReceiveDecision::RejectInvalid`] |
//! | 3 | snapshot、`reason == "session-reset"`、epoch 不同（或 local 無狀態） | [`ReceiveDecision::Reset`] |
//! | 4 | snapshot、local 無狀態 | [`ReceiveDecision::Apply`]（bootstrap） |
//! | 5 | snapshot、epoch 不同、無 reset 宣告 | [`RealignReason::EpochChanged`] |
//! | 6 | snapshot、同 epoch、`reason == "recovery"`、revision 較舊 | [`ReceiveDecision::Recover`] |
//! | 7 | snapshot、同 epoch、revision 較舊 | [`ReceiveDecision::IgnoreStale`] |
//! | 8 | snapshot、同 epoch、revision 相同 | [`ReceiveDecision::AlreadyApplied`]（hash 不同 → realign） |
//! | 9 | snapshot、同 epoch、revision 較新 | [`ReceiveDecision::Apply`]（hash 不符 → realign） |
//! | 10 | patch、local 無狀態 | [`RealignReason::NoLocal`] |
//! | 11 | patch、epoch 不同 | [`RealignReason::EpochChanged`] |
//! | 12 | patch、revision ≤ local | ignore-stale／already-applied |
//! | 13 | patch、`baseRevision` != local revision | [`RealignReason::BaseMismatch`] |
//! | 14 | merge 後 hash 不符 | [`RealignReason::HashMismatch`] |
//! | 15 | 其餘 | [`ReceiveDecision::Apply`] |
//!
//! # 這張表**不**做的事
//!
//! - 不碰 I/O、不記時間、不持有狀態：套用與稽核都由呼叫端做（[`advance`] 只是把
//!   「套用之後本地會變成什麼」也寫成純函式，方便三端對答案）。
//! - 不自己算 hash：`computed_hash` 是呼叫端算出來的（snapshot ＝對收到的 `state`；
//!   patch ＝merge 之後的結果）。`None` 代表**這個呼叫端沒有核對**——AIP §6 要求真正的
//!   接收端要算，這裡不會替它假裝算過，也不會把「沒算」升級成錯誤。

use crate::{REASON_RECOVERY, REASON_SESSION_RESET};

/// 接收端上限：一則 resume 回覆最多幾則 patch。權威值在
/// [`interaction_aip::limits::MAX_RESUME_PATCHES`]（＝事件日誌環大小），發布在 golden schema
/// 的 `limits` 表，TypeScript／Swift 由 codegen 讀同一個數字。
pub const MAX_RESUME_PATCHES: usize = interaction_aip::limits::MAX_RESUME_PATCHES;
/// 接收端上限：連續幾次未能 apply 就是 unrecoverable。權威值在
/// [`interaction_aip::limits::MAX_REALIGN_ATTEMPTS`]。
pub const MAX_REALIGN_ATTEMPTS: u32 = interaction_aip::limits::MAX_REALIGN_ATTEMPTS;

/// 本地那份權威狀態副本的**摘要**（沒有 state 本身：這張表只看得到中繼資料）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverView {
    /// 本地有沒有一份可用的狀態（false ＝ 還沒 bootstrap）。
    pub has_state: bool,
    /// 本地記得的 session id（`None` ＝ 不知道，就不宣稱不符）。
    pub session_id: Option<String>,
    pub epoch: u64,
    pub revision: u64,
    /// 本地那份 state 的 canonical hash（本地自己算的；`None` ＝ 沒算過）。
    pub state_hash: Option<String>,
    /// 現行連線／請求世代。訊息帶著別的世代就是舊連線的遲到品。
    pub connection_generation: u64,
}

impl ReceiverView {
    /// 還沒收過任何權威狀態的接收端。
    pub fn empty(connection_generation: u64) -> Self {
        Self {
            has_state: false,
            session_id: None,
            epoch: 0,
            revision: 0,
            state_hash: None,
            connection_generation,
        }
    }
}

/// `state` 訊息的兩種形狀。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncomingKind {
    Snapshot,
    Patch,
}

impl IncomingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            IncomingKind::Snapshot => "snapshot",
            IncomingKind::Patch => "patch",
        }
    }

    /// wire 上的 `payload.kind`（未知字串 ＝ 不是這張表能處理的訊息）。
    pub fn parse(kind: &str) -> Option<Self> {
        match kind {
            "snapshot" => Some(IncomingKind::Snapshot),
            "patch" => Some(IncomingKind::Patch),
            _ => None,
        }
    }
}

/// 一則**已經通過 typed boundary** 的 `state` 訊息，攤平成決策需要的欄位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingState {
    pub kind: IncomingKind,
    /// envelope 的 `sessionId`。
    pub session_id: Option<String>,
    /// `payload.sessionEpoch`。
    pub epoch: u64,
    /// `payload.revision`。
    pub revision: u64,
    /// `baseRevision`（patch 必填）。
    pub base_revision: Option<u64>,
    /// `payload.reason`（`session-reset`／`recovery`；未知值視同沒有 reason）。
    pub reason: Option<String>,
    /// `payload.hash`（snapshot 必填）。
    pub hash: Option<String>,
    /// 呼叫端**自己算出來**的 hash：snapshot ＝對收到的 `state`；patch ＝merge 之後的結果。
    /// `None` ＝ 這個呼叫端沒有核對（不代表核對過了）。
    pub computed_hash: Option<String>,
    /// snapshot 的 payload 真的帶了 `state`。
    pub state_present: bool,
    /// 這則訊息是在哪個連線／請求世代上收到的。
    pub arrived_on_generation: u64,
    /// 它是不是我們自己要來的權威回覆（HTTP GET／resume response），而不是推播。
    pub via_authoritative_reply: bool,
}

/// realign 的原因（穩定字串；三端與 fixture 共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealignReason {
    /// 本地沒有狀態，補丁沒有東西可以套上去。
    NoLocal,
    /// epoch 不同，兩份狀態沒有可比的順序。
    EpochChanged,
    /// `baseRevision` 接不上本地 revision。
    BaseMismatch,
    /// 套用後算出來的 hash 與 host 宣告的不同。
    HashMismatch,
    /// resume 回覆的 patch 數量超過 [`MAX_RESUME_PATCHES`]（**不**靜默截斷）。
    ResumeTooLong,
}

impl RealignReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RealignReason::NoLocal => "no-local",
            RealignReason::EpochChanged => "epoch-changed",
            RealignReason::BaseMismatch => "base-mismatch",
            RealignReason::HashMismatch => "hash-mismatch",
            RealignReason::ResumeTooLong => "resume-too-long",
        }
    }
}

/// 一則 `state` 訊息對本地副本的意義。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveDecision {
    /// 舊連線／舊請求世代的遲到品：丟掉並計數（**先於**一切 epoch 判斷）。
    IgnoreStaleConnection,
    /// 別的 session 的狀態：不套用、稽核 `aip.identity-mismatch`、**不**realign。
    RejectIdentity,
    /// 不是一則能用的 state 訊息（snapshot 缺 hash／state；patch 缺 baseRevision）。
    RejectInvalid,
    /// host 明說 session 被重建：丟掉本地狀態，採用新的 epoch／revision。
    Reset,
    /// 套用。
    Apply,
    /// 接不上：不套用，改要求重新對齊（送 `character.session.resume`）。
    Realign { reason: RealignReason },
    /// 同一個 session 真的倒退過（host 從較舊快照還原）：套用並退回 host 的 revision，
    /// 稽核 `aip.state-recovered`。
    Recover,
    /// 落後：忽略（稽核 `aip.state-rollback-ignored`）。
    IgnoreStale,
    /// 重播：已經套用過，什麼都不做。
    AlreadyApplied,
}

impl ReceiveDecision {
    /// 穩定字串（fixture 與稽核共用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            ReceiveDecision::IgnoreStaleConnection => "ignore-stale-connection",
            ReceiveDecision::RejectIdentity => "reject-identity",
            ReceiveDecision::RejectInvalid => "reject-invalid",
            ReceiveDecision::Reset => "reset",
            ReceiveDecision::Apply => "apply",
            ReceiveDecision::Realign { .. } => "realign",
            ReceiveDecision::Recover => "recover",
            ReceiveDecision::IgnoreStale => "ignore-stale",
            ReceiveDecision::AlreadyApplied => "already-applied",
        }
    }

    /// 這個決策會讓本地採用 incoming 的狀態嗎？
    pub fn adopts_state(&self) -> bool {
        matches!(
            self,
            ReceiveDecision::Apply | ReceiveDecision::Reset | ReceiveDecision::Recover
        )
    }

    /// 這個決策需要呼叫端再發一次請求嗎？
    pub fn realign_reason(&self) -> Option<RealignReason> {
        match self {
            ReceiveDecision::Realign { reason } => Some(*reason),
            _ => None,
        }
    }
}

/// 兩個**都知道**的 hash 不同才算不符。有一邊不知道就是「沒得核對」，
/// 誠實地不核對（不假裝核對過，也不把沒核對升級成錯誤）。
fn hashes_disagree(a: Option<&str>, b: Option<&str>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x != y)
}

fn reason_is(incoming: &IncomingState, expected: &str) -> bool {
    incoming.reason.as_deref() == Some(expected)
}

/// 接收端決策表（模組文件的那張表，逐條照抄）。
pub fn decide_receive(view: &ReceiverView, incoming: &IncomingState) -> ReceiveDecision {
    // 0. 舊連線／舊請求世代的遲到品。這是「上一條連線送出的 reset 現在才到」的唯一防線：
    //    它宣告的 epoch 一定與本地不同，任何 epoch 判斷都會被它騙過去。
    if incoming.arrived_on_generation != view.connection_generation {
        return ReceiveDecision::IgnoreStaleConnection;
    }
    // 1. 身分：別的 session 的狀態不是「比較舊」，是**不相干**——不套用也不 realign。
    //    只在本地**知道**自己的 sessionId 時才比對：本地有狀態但身分未知（例如由不帶
    //    `sessionId` 的 resume snapshot payload bootstrap 出來的那一份）不算不符。
    //    把「未知」當成不符是 fail-closed 的地雷：reject-identity 不 realign，之後每一則
    //    帶 sessionId 的訊息都會被擋掉，那台裝置永遠凍在舊狀態且沒有任何出路。
    //    未知的身分由 [`advance`] 在套用時記下 incoming 的 sessionId 補齊，下一則就有得比。
    if let (true, Some(known), Some(claimed)) = (
        view.has_state,
        view.session_id.as_deref(),
        incoming.session_id.as_deref(),
    ) {
        if known != claimed {
            return ReceiveDecision::RejectIdentity;
        }
    }
    match incoming.kind {
        IncomingKind::Snapshot => decide_snapshot(view, incoming),
        IncomingKind::Patch => decide_patch(view, incoming),
    }
}

fn decide_snapshot(view: &ReceiverView, incoming: &IncomingState) -> ReceiveDecision {
    // 2. AIP 1.0 的 snapshot 必帶 hash 與 state；沒有 legacy profile。
    if incoming.hash.is_none() || !incoming.state_present {
        return ReceiveDecision::RejectInvalid;
    }
    // 套用之前一律核對：算出來的與宣告的不同就不採用（reset／bootstrap 也一樣）。
    let unverified = hashes_disagree(incoming.hash.as_deref(), incoming.computed_hash.as_deref());
    let adopt = |decision: ReceiveDecision| {
        if unverified {
            ReceiveDecision::Realign {
                reason: RealignReason::HashMismatch,
            }
        } else {
            decision
        }
    };
    // 3. host 明說重建了 session。epoch 相同的 `session-reset` **不算**（AIP §7 第 4 步是
    //    「epoch 不同」）：host 重灌後 epoch 可能比本地記得的小，所以是「不同」不是「更大」。
    if reason_is(incoming, REASON_SESSION_RESET)
        && (!view.has_state || incoming.epoch != view.epoch)
    {
        return adopt(ReceiveDecision::Reset);
    }
    // 4. bootstrap：本地什麼都沒有，第一份權威狀態直接收下。
    if !view.has_state {
        return adopt(ReceiveDecision::Apply);
    }
    // 5. epoch 不同又沒有重建宣告：兩份狀態沒有可比的順序，不猜。
    //    host 對 epoch 不同的 resume 一律回 `session-reset` snapshot，所以一次就收斂。
    if incoming.epoch != view.epoch {
        return ReceiveDecision::Realign {
            reason: RealignReason::EpochChanged,
        };
    }
    // 6. 同一個 session 真的倒退過：host 明說 `recovery` 才採納（成員自己宣稱超前不算證據）。
    if reason_is(incoming, REASON_RECOVERY) && incoming.revision < view.revision {
        return adopt(ReceiveDecision::Recover);
    }
    match incoming.revision.cmp(&view.revision) {
        // 7. 落後：忽略。權威回覆也一樣——「最新的 HTTP 回覆」不代表同一個 incarnation
        //    的回退是合法的；真的倒退過的 host 會說 `recovery`。
        std::cmp::Ordering::Less => ReceiveDecision::IgnoreStale,
        // 8. 重播：什麼都不做。除非它宣告的 hash 與本地算出來的不同——那就是同一個
        //    revision 有兩份不同的狀態，只能重新對齊。
        std::cmp::Ordering::Equal => {
            if hashes_disagree(incoming.hash.as_deref(), view.state_hash.as_deref()) {
                ReceiveDecision::Realign {
                    reason: RealignReason::HashMismatch,
                }
            } else {
                ReceiveDecision::AlreadyApplied
            }
        }
        // 9. 較新：核對過就套用。
        std::cmp::Ordering::Greater => adopt(ReceiveDecision::Apply),
    }
}

fn decide_patch(view: &ReceiverView, incoming: &IncomingState) -> ReceiveDecision {
    // 2（patch 版）：typed boundary 已經擋掉缺 baseRevision 的 patch，這裡是第二道。
    let Some(base_revision) = incoming.base_revision else {
        return ReceiveDecision::RejectInvalid;
    };
    // 10. 補丁不是完整狀態：沒有本地副本就沒有東西可以套上去。
    if !view.has_state {
        return ReceiveDecision::Realign {
            reason: RealignReason::NoLocal,
        };
    }
    // 11. epoch 不同 → realign（三端統一；以前 Rust 的 patch 分支完全不看 epoch，
    //     只靠 `baseRevision` 恰巧不符去擋）。
    if incoming.epoch != view.epoch {
        return ReceiveDecision::Realign {
            reason: RealignReason::EpochChanged,
        };
    }
    // 12. 落後／重播。
    match incoming.revision.cmp(&view.revision) {
        std::cmp::Ordering::Less => return ReceiveDecision::IgnoreStale,
        std::cmp::Ordering::Equal => return ReceiveDecision::AlreadyApplied,
        std::cmp::Ordering::Greater => {}
    }
    // 13. 接不上前一個 revision。
    if base_revision != view.revision {
        return ReceiveDecision::Realign {
            reason: RealignReason::BaseMismatch,
        };
    }
    // 14. merge 之後的 hash 與 host 宣告的不同。
    if hashes_disagree(incoming.hash.as_deref(), incoming.computed_hash.as_deref()) {
        return ReceiveDecision::Realign {
            reason: RealignReason::HashMismatch,
        };
    }
    // 15. 其餘：套用。
    ReceiveDecision::Apply
}

/// 決策套用之後，本地那份摘要會變成什麼（純函式；不採用狀態的決策原樣回傳）。
///
/// 套用時**記下 incoming 的 sessionId**：本地身分未知（規則 1 因此不比對）的那一份，
/// 收下第一則帶 sessionId 的權威狀態之後就有身分可比，之後別的 session 立刻被規則 1 擋下。
pub fn advance(
    view: &ReceiverView,
    incoming: &IncomingState,
    decision: &ReceiveDecision,
) -> ReceiverView {
    if !decision.adopts_state() {
        return view.clone();
    }
    ReceiverView {
        has_state: true,
        session_id: incoming
            .session_id
            .clone()
            .or_else(|| view.session_id.clone()),
        epoch: incoming.epoch,
        revision: incoming.revision,
        // 本地重算的優先；沒算過就只能記下 host 宣告的那一個。
        state_hash: incoming
            .computed_hash
            .clone()
            .or_else(|| incoming.hash.clone()),
        connection_generation: view.connection_generation,
    }
}

/// 一則 `character.session.resume` 回覆逐則處理過後的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeBatch {
    /// 逐則決策（中止之後不再有）。
    pub decisions: Vec<ReceiveDecision>,
    /// 處理完之後的本地摘要。
    pub view: ReceiverView,
    /// 中止在第幾則（`None` ＝ 整批走完）。
    pub stopped_at: Option<usize>,
    /// 中止的那一則決策。
    pub halted: Option<ReceiveDecision>,
    /// 真的套用了幾則。
    pub applied: usize,
    /// 良性跳過幾則（已套用／落後）。
    pub skipped: usize,
}

impl ResumeBatch {
    /// 整批的結論：中止就是中止的那一則，否則是最後一則決策（空批＝已對齊）。
    pub fn outcome(&self) -> ReceiveDecision {
        self.halted.clone().unwrap_or_else(|| {
            self.decisions
                .last()
                .cloned()
                .unwrap_or(ReceiveDecision::AlreadyApplied)
        })
    }
}

/// resume 回覆的逐則規則：
///
/// - 數量超過 [`MAX_RESUME_PATCHES`] → 整批不處理，直接 realign（**不**靜默截斷成
///   「我以為我追上了」）。
/// - `already-applied`／`ignore-stale` 是良性的舊項（host 回放的範圍本來就可能與本地重疊）：
///   跳過，**不**中止。
/// - 第一個 realign／reject 就中止：後面的補丁都建立在沒套用的那一則之上。
pub fn decide_resume_batch(view: &ReceiverView, items: &[IncomingState]) -> ResumeBatch {
    if items.len() > MAX_RESUME_PATCHES {
        return ResumeBatch {
            decisions: Vec::new(),
            view: view.clone(),
            stopped_at: Some(0),
            halted: Some(ReceiveDecision::Realign {
                reason: RealignReason::ResumeTooLong,
            }),
            applied: 0,
            skipped: 0,
        };
    }
    let mut current = view.clone();
    let mut decisions = Vec::with_capacity(items.len());
    let mut applied = 0usize;
    let mut skipped = 0usize;
    for (index, item) in items.iter().enumerate() {
        let decision = decide_receive(&current, item);
        decisions.push(decision.clone());
        match decision {
            ReceiveDecision::Apply | ReceiveDecision::Reset | ReceiveDecision::Recover => {
                current = advance(&current, item, &decision);
                applied += 1;
            }
            ReceiveDecision::AlreadyApplied | ReceiveDecision::IgnoreStale => {
                skipped += 1;
            }
            other => {
                return ResumeBatch {
                    decisions,
                    view: current,
                    stopped_at: Some(index),
                    halted: Some(other),
                    applied,
                    skipped,
                };
            }
        }
    }
    ResumeBatch {
        decisions,
        view: current,
        stopped_at: None,
        halted: None,
        applied,
        skipped,
    }
}

/// 有界的 realign 預算：連續 [`MAX_REALIGN_ATTEMPTS`] 次未能 apply 就是 unrecoverable。
///
/// realign 的效果是「再要一次權威讀取」；host 一直給對不上的東西時，沒有上限就是一個
/// 打不完的請求迴圈。達上限要照實說「狀態未知」——不是繼續轉圈圈，也不是假裝同步。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RealignBudget {
    attempts: u32,
}

impl RealignBudget {
    pub const fn new() -> Self {
        Self { attempts: 0 }
    }

    /// 記下一次決策的結果（純函式：回傳新的預算，不改自己）。
    ///
    /// `boundary_rejected_authoritative_reply` ＝ 這一則是**權威回覆**卻被 typed boundary
    /// 或身分／格式檢查擋下（`reject-invalid`）：對方回答了、但答案沒用，算一次失敗。
    /// 推播（SSE／WebSocket）上的垃圾不算——它不是我們要來的答案，不會讓對齊卡住。
    pub fn observe(self, decision: &ReceiveDecision, via_authoritative_reply: bool) -> Self {
        match decision {
            ReceiveDecision::Apply | ReceiveDecision::Reset | ReceiveDecision::Recover => {
                Self::new()
            }
            ReceiveDecision::Realign { .. } => Self {
                attempts: self.attempts.saturating_add(1),
            },
            ReceiveDecision::RejectInvalid if via_authoritative_reply => Self {
                attempts: self.attempts.saturating_add(1),
            },
            _ => self,
        }
    }

    /// 連續未能 apply 的次數。
    pub fn attempts(self) -> u32 {
        self.attempts
    }

    /// 已經到上限：狀態是**未知**，畫面照實說，不再自動重試。
    pub fn is_unrecoverable(self) -> bool {
        self.attempts >= MAX_REALIGN_ATTEMPTS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ReceiverView {
        ReceiverView {
            has_state: true,
            session_id: Some("session.home".into()),
            epoch: 3,
            revision: 10,
            state_hash: Some("h10".into()),
            connection_generation: 1,
        }
    }

    fn snapshot(revision: u64, epoch: u64) -> IncomingState {
        IncomingState {
            kind: IncomingKind::Snapshot,
            session_id: Some("session.home".into()),
            epoch,
            revision,
            base_revision: None,
            reason: None,
            hash: Some(format!("h{revision}")),
            computed_hash: Some(format!("h{revision}")),
            state_present: true,
            arrived_on_generation: 1,
            via_authoritative_reply: false,
        }
    }

    #[test]
    fn the_generation_check_comes_before_everything_else() {
        let mut reset = snapshot(1, 99);
        reset.reason = Some(REASON_SESSION_RESET.into());
        reset.session_id = Some("session.elsewhere".into());
        reset.hash = None;
        reset.arrived_on_generation = 0;
        assert_eq!(
            decide_receive(&view(), &reset),
            ReceiveDecision::IgnoreStaleConnection
        );
    }

    #[test]
    fn identity_comes_before_format() {
        let mut broken = snapshot(11, 3);
        broken.session_id = Some("session.elsewhere".into());
        broken.hash = None;
        assert_eq!(
            decide_receive(&view(), &broken),
            ReceiveDecision::RejectIdentity
        );
    }

    #[test]
    fn a_snapshot_whose_hash_does_not_match_its_state_is_never_adopted() {
        for reason in [None, Some(REASON_SESSION_RESET), Some(REASON_RECOVERY)] {
            let mut bad = snapshot(
                11,
                if reason == Some(REASON_SESSION_RESET) {
                    4
                } else {
                    3
                },
            );
            bad.reason = reason.map(str::to_string);
            bad.computed_hash = Some("something-else".into());
            assert_eq!(
                decide_receive(&view(), &bad),
                ReceiveDecision::Realign {
                    reason: RealignReason::HashMismatch
                },
                "reason={reason:?}"
            );
        }
    }

    #[test]
    fn a_bootstrap_snapshot_is_applied_and_advances_the_view() {
        let empty = ReceiverView::empty(1);
        let snap = snapshot(42, 7);
        let decision = decide_receive(&empty, &snap);
        assert_eq!(decision, ReceiveDecision::Apply);
        let next = advance(&empty, &snap, &decision);
        assert!(next.has_state);
        assert_eq!(next.revision, 42);
        assert_eq!(next.epoch, 7);
        assert_eq!(next.connection_generation, 1);
    }

    /// 規則 1 只在**本地知道自己的 sessionId** 時比對。本地有狀態但身分未知
    /// （例如由不帶 `sessionId` 的 resume snapshot payload bootstrap 出來的那一份）
    /// 不算「不符」——否則之後每一則帶 sessionId 的 SSE 都會被 reject-identity
    /// 永久凍結，而且 reject-identity 不 realign，沒有任何出路。
    #[test]
    fn a_local_copy_whose_session_id_is_unknown_is_not_a_mismatch() {
        let mut unknown = view();
        unknown.session_id = None;
        let incoming = snapshot(11, 3);
        assert_eq!(decide_receive(&unknown, &incoming), ReceiveDecision::Apply);
    }

    /// 套用時把 incoming 的 sessionId 記下來：下一則就有身分可比，凍結不會再長回來。
    #[test]
    fn applying_records_the_incoming_session_id_when_the_local_one_is_unknown() {
        let mut unknown = view();
        unknown.session_id = None;
        let incoming = snapshot(11, 3);
        let decision = decide_receive(&unknown, &incoming);
        let next = advance(&unknown, &incoming, &decision);
        assert_eq!(next.session_id.as_deref(), Some("session.home"));
        // 記下之後，別的 session 立刻被擋下來。
        let mut other = snapshot(12, 3);
        other.session_id = Some("session.elsewhere".into());
        assert_eq!(
            decide_receive(&next, &other),
            ReceiveDecision::RejectIdentity
        );
    }

    /// patch 也一樣：resume snapshot bootstrap（沒有 sessionId）之後的第一則 SSE patch
    /// 接得上就套用，不得因為身分未知而被凍結。
    #[test]
    fn a_patch_onto_a_local_copy_without_a_known_session_id_still_applies() {
        let mut unknown = view();
        unknown.session_id = None;
        let mut incoming = snapshot(11, 3);
        incoming.kind = IncomingKind::Patch;
        incoming.base_revision = Some(10);
        incoming.state_present = false;
        assert_eq!(decide_receive(&unknown, &incoming), ReceiveDecision::Apply);
        let next = advance(&unknown, &incoming, &decide_receive(&unknown, &incoming));
        assert_eq!(next.session_id.as_deref(), Some("session.home"));
    }

    /// 放寬只發生在「本地不知道」那一格：本地知道就照樣 reject（即使那則訊息本來
    /// 就會被規則 7 忽略——身分先於 revision）。
    #[test]
    fn a_known_local_session_id_still_rejects_a_mismatch() {
        let mut older_from_elsewhere = snapshot(9, 3);
        older_from_elsewhere.session_id = Some("session.elsewhere".into());
        assert_eq!(
            decide_receive(&view(), &older_from_elsewhere),
            ReceiveDecision::RejectIdentity
        );
    }

    #[test]
    fn an_oversized_resume_reply_realigns_instead_of_truncating() {
        let items: Vec<IncomingState> = (0..MAX_RESUME_PATCHES + 1)
            .map(|i| {
                let mut p = snapshot(11 + i as u64, 3);
                p.kind = IncomingKind::Patch;
                p.base_revision = Some(10 + i as u64);
                p
            })
            .collect();
        let batch = decide_resume_batch(&view(), &items);
        assert_eq!(batch.applied, 0);
        assert_eq!(
            batch.halted,
            Some(ReceiveDecision::Realign {
                reason: RealignReason::ResumeTooLong
            })
        );
        assert_eq!(batch.view, view(), "整批不處理就不得動到本地狀態");
    }

    #[test]
    fn a_rejected_authoritative_reply_counts_as_one_failed_realign() {
        let budget = RealignBudget::new().observe(&ReceiveDecision::RejectInvalid, true);
        assert_eq!(budget.attempts(), 1);
        // 推播上的垃圾不算：它不是我們要來的答案。
        let budget = RealignBudget::new().observe(&ReceiveDecision::RejectInvalid, false);
        assert_eq!(budget.attempts(), 0);
    }
}
