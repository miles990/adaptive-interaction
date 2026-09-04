//! §3 Outcome：成功狀態不是一個布林。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `result.payload.status` 的十二種值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Received,
    Accepted,
    Acknowledged,
    Applied,
    Observed,
    ClaimedCompleted,
    Verified,
    Rejected,
    Expired,
    CancelRequested,
    CancelConfirmed,
    Failed,
}

impl Outcome {
    pub const ALL: [Outcome; 12] = [
        Outcome::Received,
        Outcome::Accepted,
        Outcome::Acknowledged,
        Outcome::Applied,
        Outcome::Observed,
        Outcome::ClaimedCompleted,
        Outcome::Verified,
        Outcome::Rejected,
        Outcome::Expired,
        Outcome::CancelRequested,
        Outcome::CancelConfirmed,
        Outcome::Failed,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Received => "received",
            Outcome::Accepted => "accepted",
            Outcome::Acknowledged => "acknowledged",
            Outcome::Applied => "applied",
            Outcome::Observed => "observed",
            Outcome::ClaimedCompleted => "claimed-completed",
            Outcome::Verified => "verified",
            Outcome::Rejected => "rejected",
            Outcome::Expired => "expired",
            Outcome::CancelRequested => "cancel-requested",
            Outcome::CancelConfirmed => "cancel-confirmed",
            Outcome::Failed => "failed",
        }
    }

    /// 終態（之後不得再變）。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Outcome::Applied
                | Outcome::Observed
                | Outcome::Verified
                | Outcome::Rejected
                | Outcome::Expired
                | Outcome::CancelConfirmed
                | Outcome::Failed
        )
    }

    /// 只有 Runtime 的人類驗證路徑可以產生的值；來自 device／renderer 一律拒絕。
    pub fn is_runtime_only(&self) -> bool {
        matches!(self, Outcome::Verified)
    }

    /// `event` 被處理後的合法值。
    pub fn allowed_for_event(&self) -> bool {
        matches!(
            self,
            Outcome::Received
                | Outcome::Accepted
                | Outcome::Applied
                | Outcome::Rejected
                | Outcome::Expired
        )
    }

    /// `command`（Behavior Intent 等）被處理後的合法值。`observed` ≠ 工作 `verified`。
    pub fn allowed_for_command(&self) -> bool {
        matches!(
            self,
            Outcome::Received
                | Outcome::Accepted
                | Outcome::Acknowledged
                | Outcome::Observed
                | Outcome::Rejected
                | Outcome::Expired
                | Outcome::Failed
                | Outcome::CancelRequested
                | Outcome::CancelConfirmed
        )
    }

    /// `state` 被套用後的合法值。
    pub fn allowed_for_state(&self) -> bool {
        matches!(self, Outcome::Applied | Outcome::Rejected)
    }

    /// 合法遷移（誠實階梯：只能往前、終態黏住）。
    pub fn can_transition_to(&self, next: Outcome) -> bool {
        use Outcome::*;
        if self.is_terminal() {
            return false;
        }
        matches!(
            (self, next),
            (Received, Accepted)
                | (Received, Rejected)
                | (Received, Expired)
                | (Accepted, Acknowledged)
                | (Accepted, Applied)
                | (Accepted, Observed)
                | (Accepted, ClaimedCompleted)
                | (Accepted, Expired)
                | (Accepted, Failed)
                | (Accepted, CancelRequested)
                | (Accepted, CancelConfirmed)
                | (Acknowledged, Observed)
                | (Acknowledged, Failed)
                | (Acknowledged, Expired)
                | (Acknowledged, CancelRequested)
                | (Acknowledged, CancelConfirmed)
                | (ClaimedCompleted, Verified)
                | (ClaimedCompleted, Failed)
                | (CancelRequested, CancelConfirmed)
                | (CancelRequested, Failed)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_is_honest() {
        assert!(!Outcome::Acknowledged.can_transition_to(Outcome::Verified));
        assert!(!Outcome::Observed.can_transition_to(Outcome::Verified));
        assert!(!Outcome::ClaimedCompleted.is_terminal());
        assert!(Outcome::ClaimedCompleted.can_transition_to(Outcome::Verified));
        assert!(!Outcome::Applied.can_transition_to(Outcome::Accepted));
        assert!(Outcome::Verified.is_runtime_only());
        for o in Outcome::ALL {
            let s = serde_json::to_string(&o).unwrap();
            assert_eq!(s.trim_matches('"'), o.as_str());
        }
    }
}
