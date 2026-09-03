//! §7 Command 回執（`CommandReceipt`）與合法順序。
//!
//! 誠實階梯：`accepted ≠ started ≠ completed`；`completed` 只代表呈現演完了；
//! `acknowledged` 代表「收到但這個 adapter 不會回報 completion」，之後只能變 `uncertain`。
//! 這個型別**沒有** `truthState`／`verified` 欄位：adapter 在型別層就無法偽造。

use crate::capability::Resolution;
use crate::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 回執狀態（10）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ReceiptStatus {
    /// Gateway 收進佇列。**不是**完成。
    Accepted,
    /// adapter 收到，但不會回報 completion；Gateway 之後記成 `uncertain`。
    Acknowledged,
    /// adapter 已排程。
    Scheduled,
    /// adapter 已開始演出。
    Started,
    /// 終態：呈現演完了（≠ 外部工作 verified）。
    Completed,
    /// 終態：被取消（含 preempted／queue-full／merged）。
    Cancelled,
    /// 終態：`expiresAt` 已過、未播。
    Expired,
    /// 終態：協商結果 `unsupported`。
    Unsupported,
    /// 終態：adapter 回報失敗。
    Failed,
    /// 終態：結果未知（crash／斷線／acknowledged 逾時／watchdog）。
    Uncertain,
}

impl ReceiptStatus {
    pub const ALL: [ReceiptStatus; 10] = [
        ReceiptStatus::Accepted,
        ReceiptStatus::Acknowledged,
        ReceiptStatus::Scheduled,
        ReceiptStatus::Started,
        ReceiptStatus::Completed,
        ReceiptStatus::Cancelled,
        ReceiptStatus::Expired,
        ReceiptStatus::Unsupported,
        ReceiptStatus::Failed,
        ReceiptStatus::Uncertain,
    ];

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ReceiptStatus::Completed
                | ReceiptStatus::Cancelled
                | ReceiptStatus::Expired
                | ReceiptStatus::Unsupported
                | ReceiptStatus::Failed
                | ReceiptStatus::Uncertain
        )
    }

    /// `self → next` 是否合法（§7）。同狀態重送視為冪等，由呼叫端處理，這裡回 `false`。
    pub fn can_transition_to(&self, next: ReceiptStatus) -> bool {
        can_transition(*self, next)
    }
}

/// §7 合法順序：
/// `accepted → (scheduled) → started → completed | cancelled | failed`；
/// `accepted → expired | unsupported`；`accepted → acknowledged → uncertain`；
/// 任何非終態都可 `uncertain`／`cancelled`／`failed`；`acknowledged` **永不** `completed`。
pub fn can_transition(from: ReceiptStatus, to: ReceiptStatus) -> bool {
    use ReceiptStatus::*;
    if from.is_terminal() || from == to {
        return false;
    }
    match (from, to) {
        (Accepted, Acknowledged)
        | (Accepted, Scheduled)
        | (Accepted, Started)
        | (Accepted, Expired)
        | (Accepted, Unsupported) => true,
        (Scheduled, Started) | (Scheduled, Expired) => true,
        (Started, Completed) => true,
        (Acknowledged, Expired) => true,
        // 任何非終態都可以取消、失敗或變成未知。
        (_, Cancelled) | (_, Failed) | (_, Uncertain) => true,
        _ => false,
    }
}

/// `acknowledged` 之後多久沒有進一步消息就記成 `uncertain`（在 durationHint 之外的寬限）。
pub const ACK_GRACE_MS: i64 = 5_000;

/// `acknowledged` 何時該被 sweep 記成 `uncertain`：`ackedAt + durationHint + ACK_GRACE_MS`。
pub fn ack_uncertain_deadline(acked_at: Timestamp, duration_hint_ms: u64) -> Timestamp {
    let ms = i64::try_from(duration_hint_ms).unwrap_or(i64::MAX / 4);
    acked_at + chrono::Duration::milliseconds(ms.saturating_add(ACK_GRACE_MS))
}

/// §7 `CommandReceipt`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandReceipt {
    pub message_id: String,
    pub character_instance_id: String,
    /// 連線世代；舊世代回執一律丟棄（記 audit）。
    pub generation: u64,
    pub status: ReceiptStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
    /// ≤ 200 字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// `cancelled{reason}`：preempted／queue-full／outbound-full／busy／merged／expired／safety-deduplicated…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// `accepted{duplicate:true}`：重複 messageId（環 256）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub duplicate: bool,
    /// `cancelled{alreadyTerminal:true}`：cancel 打到已終結的 command。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub already_terminal: bool,
    pub at: Timestamp,
}

/// `detail` 上限。
pub const MAX_DETAIL_CHARS: usize = 200;

impl CommandReceipt {
    pub fn new(
        message_id: impl Into<String>,
        character_instance_id: impl Into<String>,
        generation: u64,
        status: ReceiptStatus,
        at: Timestamp,
    ) -> Self {
        CommandReceipt {
            message_id: message_id.into(),
            character_instance_id: character_instance_id.into(),
            generation,
            status,
            resolution: None,
            detail: None,
            reason: None,
            duplicate: false,
            already_terminal: false,
            at,
        }
    }

    pub fn with_resolution(mut self, resolution: Resolution) -> Self {
        self.resolution = Some(resolution);
        self
    }

    /// 設定 detail（自動截到 200 字）。
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        let detail: String = detail.into();
        self.detail = Some(if crate::char_len(&detail) > MAX_DETAIL_CHARS {
            detail.chars().take(MAX_DETAIL_CHARS).collect()
        } else {
            detail
        });
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("illegal receipt transition {from:?} -> {to:?}")]
pub struct IllegalReceiptTransition {
    pub from: ReceiptStatus,
    pub to: ReceiptStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ReceiptStatus::*;

    #[test]
    fn happy_path_is_legal() {
        for pair in [Accepted, Scheduled, Started, Completed].windows(2) {
            assert!(
                can_transition(pair[0], pair[1]),
                "{:?}->{:?}",
                pair[0],
                pair[1]
            );
        }
        assert!(can_transition(Accepted, Started));
        assert!(can_transition(Accepted, Expired));
        assert!(can_transition(Accepted, Unsupported));
    }

    #[test]
    fn accepted_is_not_completed() {
        assert!(!can_transition(Accepted, Completed));
        assert!(!can_transition(Scheduled, Completed));
    }

    #[test]
    fn acknowledged_never_becomes_completed() {
        assert!(can_transition(Accepted, Acknowledged));
        assert!(!can_transition(Acknowledged, Completed));
        assert!(!can_transition(Acknowledged, Started));
        assert!(!can_transition(Acknowledged, Scheduled));
        assert!(can_transition(Acknowledged, Uncertain));
        assert!(can_transition(Acknowledged, Cancelled));
    }

    #[test]
    fn terminal_states_are_frozen() {
        for from in ReceiptStatus::ALL.iter().filter(|s| s.is_terminal()) {
            for to in ReceiptStatus::ALL {
                assert!(!can_transition(*from, to), "{from:?}->{to:?}");
            }
        }
    }

    #[test]
    fn receipt_type_cannot_carry_truth_state() {
        let json = r#"{"messageId":"m","characterInstanceId":"i","generation":1,
            "status":"completed","truthState":"verified","verified":true,
            "at":"2026-09-02T12:00:00Z"}"#;
        let receipt: CommandReceipt = serde_json::from_str(json).expect("extra keys ignored");
        let back = serde_json::to_value(&receipt).unwrap_or_default();
        assert!(back.get("truthState").is_none());
        assert!(back.get("verified").is_none());
        assert_eq!(receipt.status, Completed);
    }

    #[test]
    fn detail_is_truncated() {
        let r = CommandReceipt::new("m", "i", 1, Accepted, Timestamp::default())
            .with_detail("x".repeat(500));
        assert_eq!(r.detail.map(|d| d.chars().count()), Some(200));
    }
}
