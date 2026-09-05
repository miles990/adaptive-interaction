//! AIP 1.0 接收端決策表（`interaction_session::receive`）的行為測試。
//!
//! 這裡釘住的是**表本身**：一則已經通過 typed boundary 的 `state` 訊息，對一份本地副本
//! 到底是 apply／reset／recover／realign／ignore 還是 reject。跨語言 fixture 由
//! `receive_decision_fixtures.rs` 產生、`receive_decisions_from_json.rs` 獨立消費。

use interaction_session::receive::{
    decide_receive, decide_resume_batch, IncomingKind, IncomingState, RealignBudget, RealignReason,
    ReceiveDecision, ReceiverView,
};
use interaction_session::state_hash;
use serde_json::json;

const SESSION: &str = "session.home";

fn local(revision: u64, epoch: u64) -> ReceiverView {
    ReceiverView {
        has_state: true,
        session_id: Some(SESSION.to_string()),
        epoch,
        revision,
        state_hash: None,
        connection_generation: 7,
    }
}

fn snapshot(revision: u64, epoch: u64) -> IncomingState {
    let state = json!({"activity": "idle"});
    IncomingState {
        kind: IncomingKind::Snapshot,
        session_id: Some(SESSION.to_string()),
        epoch,
        revision,
        base_revision: None,
        reason: None,
        hash: Some(state_hash(&state)),
        computed_hash: Some(state_hash(&state)),
        state_present: true,
        arrived_on_generation: 7,
        via_authoritative_reply: false,
    }
}

fn patch(revision: u64, base: u64, epoch: u64) -> IncomingState {
    IncomingState {
        kind: IncomingKind::Patch,
        session_id: Some(SESSION.to_string()),
        epoch,
        revision,
        base_revision: Some(base),
        reason: None,
        hash: None,
        computed_hash: None,
        state_present: false,
        arrived_on_generation: 7,
        via_authoritative_reply: false,
    }
}

/// 規則 0 先於一切 epoch 判斷：舊連線世代上遲到的 `session-reset` 是唯一防線。
#[test]
fn a_reset_that_arrives_on_a_dead_connection_generation_is_ignored_first() {
    let mut stale = snapshot(1, 9);
    stale.reason = Some(interaction_session::REASON_SESSION_RESET.to_string());
    stale.arrived_on_generation = 6; // 上一條連線
    assert_eq!(
        decide_receive(&local(30, 5), &stale),
        ReceiveDecision::IgnoreStaleConnection
    );
}

/// 規則 1：sessionId 不符一律不套用，也不 realign（realign 只會再要一次別人的 session）。
#[test]
fn a_message_from_another_session_is_rejected_not_realigned() {
    let mut other = snapshot(31, 5);
    other.session_id = Some("session.someone-else".to_string());
    assert_eq!(
        decide_receive(&local(30, 5), &other),
        ReceiveDecision::RejectIdentity
    );
}

/// 規則 2：AIP 1.0 的 snapshot 必帶 hash 與 state，沒有 legacy profile。
#[test]
fn a_snapshot_without_a_hash_or_state_is_invalid() {
    let mut no_hash = snapshot(31, 5);
    no_hash.hash = None;
    assert_eq!(
        decide_receive(&local(30, 5), &no_hash),
        ReceiveDecision::RejectInvalid
    );
    let mut no_state = snapshot(31, 5);
    no_state.state_present = false;
    assert_eq!(
        decide_receive(&local(30, 5), &no_state),
        ReceiveDecision::RejectInvalid
    );
}

/// 規則 3／5：epoch 不同時，只有 host 明說 `session-reset` 才丟掉本地狀態；
/// 沒有宣告就 realign（三端統一：以前 Rust 直接套用並靜默改寫本地 epoch）。
#[test]
fn a_different_epoch_needs_an_explicit_reset_otherwise_realign() {
    let mut reset = snapshot(1, 1);
    reset.reason = Some(interaction_session::REASON_SESSION_RESET.to_string());
    assert_eq!(
        decide_receive(&local(30, 5), &reset),
        ReceiveDecision::Reset
    );

    let plain = snapshot(1, 1);
    assert_eq!(
        decide_receive(&local(30, 5), &plain),
        ReceiveDecision::Realign {
            reason: RealignReason::EpochChanged
        }
    );
}

/// 規則 6：同 epoch、host 明說 `recovery` 的較舊 snapshot 要被採納（host 真的倒退過）。
/// 沒有這個宣告的較舊 snapshot 仍是 ignore-stale。
#[test]
fn a_recovery_snapshot_moves_the_receiver_back_but_a_plain_one_does_not() {
    let mut recovery = snapshot(9, 5);
    recovery.reason = Some(interaction_session::REASON_RECOVERY.to_string());
    recovery.via_authoritative_reply = true;
    assert_eq!(
        decide_receive(&local(30, 5), &recovery),
        ReceiveDecision::Recover
    );

    let mut plain = snapshot(9, 5);
    plain.via_authoritative_reply = true;
    assert_eq!(
        decide_receive(&local(30, 5), &plain),
        ReceiveDecision::IgnoreStale,
        "最新的 HTTP 回覆不代表同一個 incarnation 的回退是合法的"
    );
}

/// 規則 11：patch 的 epoch 與本地不同 → realign（以前 Rust 完全不看 epoch）。
#[test]
fn a_patch_from_another_epoch_realigns() {
    assert_eq!(
        decide_receive(&local(30, 5), &patch(31, 30, 6)),
        ReceiveDecision::Realign {
            reason: RealignReason::EpochChanged
        }
    );
}

/// resume 回覆：良性的舊項（已套用／落後）跳過不中止，第一個 realign 才中止。
#[test]
fn a_resume_reply_skips_benign_items_and_stops_at_the_first_realign() {
    let items = vec![
        patch(29, 28, 5), // 已經套用過
        patch(30, 29, 5), // 已經套用過
        patch(31, 30, 5), // 新的
        patch(33, 32, 5), // 斷了一格
        patch(34, 33, 5), // 中止之後不再處理
    ];
    let batch = decide_resume_batch(&local(30, 5), &items);
    assert_eq!(batch.view.revision, 31);
    assert_eq!(batch.stopped_at, Some(3));
    assert_eq!(
        batch.halted,
        Some(ReceiveDecision::Realign {
            reason: RealignReason::BaseMismatch
        })
    );
    assert_eq!(batch.decisions.len(), 4, "中止之後不得再處理剩下的");
}

/// realign 有界：連續 3 次未能 apply → unrecoverable；一次 apply 清零。
#[test]
fn the_realign_budget_is_bounded_and_reset_by_a_successful_apply() {
    let mut budget = RealignBudget::new();
    for _ in 0..2 {
        budget = budget.observe(
            &ReceiveDecision::Realign {
                reason: RealignReason::HashMismatch,
            },
            true,
        );
        assert!(!budget.is_unrecoverable());
    }
    budget = budget.observe(
        &ReceiveDecision::Realign {
            reason: RealignReason::HashMismatch,
        },
        true,
    );
    assert!(budget.is_unrecoverable());
    budget = budget.observe(&ReceiveDecision::Apply, false);
    assert_eq!(budget.attempts(), 0);
    assert!(!budget.is_unrecoverable());
}
