//! Negotiated transport profile `aip.applied/1`. A receipt is a peer's report,
//! never independent verification of a renderer or physical effect.
//!
//! The two production consumers (DeviceLink and MobileBridge) supply the
//! authenticated connection generation. This bounded tracker performs no I/O.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;

/// Process-local monotonic clock; no wall-clock change can extend a receipt TTL.
pub fn monotonic_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

pub const APPLIED_PROFILE: &str = "aip.applied/1";
pub const MAX_PENDING_APPLIED: usize = 32;
pub const APPLIED_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateAppliedReceipt {
    pub profile: String,
    pub token: String,
    pub generation: u64,
    pub session_id: String,
    pub epoch: u64,
    pub revision: u64,
    pub hash: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StateDelivery {
    pub negotiated: bool,
    pub progress: &'static str,
    pub outstanding: usize,
    pub expired: u64,
    pub overflow: u64,
    pub rejected: u64,
    pub cancelled: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent: Option<StateAppliedReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<StateAppliedReceipt>,
}

struct Pending {
    receipt: StateAppliedReceipt,
    deadline_ms: u64,
}

#[derive(Default)]
pub struct StateAppliedTracker {
    generation: Option<u64>,
    negotiated: bool,
    pending: VecDeque<Pending>,
    sent: Option<StateAppliedReceipt>,
    applied: Option<StateAppliedReceipt>,
    progress: &'static str,
    expired: u64,
    overflow: u64,
    rejected: u64,
    cancelled: u64,
}

impl StateAppliedTracker {
    pub fn negotiate(&mut self, generation: u64, enabled: bool) {
        if self.generation != Some(generation) || self.negotiated != enabled {
            self.cancel();
        }
        self.generation = Some(generation);
        self.negotiated = enabled;
    }

    /// Cancelling also invalidates a previously applied claim. A new transfer
    /// gets a new random token even if the transport generation is unchanged.
    pub fn cancel(&mut self) {
        self.cancelled = self.cancelled.saturating_add(self.pending.len() as u64);
        self.pending.clear();
        self.sent = None;
        self.applied = None;
        self.progress = "cancelled";
    }

    pub fn invalidate(&mut self) {
        self.cancel();
        self.negotiated = false;
        self.generation = None;
    }

    pub fn expire(&mut self, now_ms: u64) {
        let before = self.pending.len();
        self.pending.retain(|p| p.deadline_ms > now_ms);
        self.expired = self
            .expired
            .saturating_add((before - self.pending.len()) as u64);
        if before > self.pending.len() && self.pending.is_empty() {
            self.progress = if self
                .applied
                .as_ref()
                .zip(self.sent.as_ref())
                .is_some_and(|(applied, sent)| advances(applied, sent))
            {
                "peer-applied"
            } else {
                "timed-out"
            };
        }
    }

    /// The context is added only after both sides have opted into this profile.
    /// The caller measures the final encoded envelope and framing afterwards.
    pub fn prepare(
        &mut self,
        envelope: &mut Value,
        generation: u64,
        now_ms: u64,
    ) -> Option<String> {
        if !self.negotiated || self.generation != Some(generation) {
            return None;
        }
        self.expire(now_ms);
        let body = envelope.get("payload")?;
        let item = match body.get("kind")?.as_str()? {
            "snapshot" | "patch" => body,
            "patches" => body.get("patches")?.as_array()?.last()?,
            _ => return None,
        };
        if !matches!(envelope.get("messageType")?.as_str()?, "state" | "response") {
            return None;
        }
        let receipt = StateAppliedReceipt {
            profile: APPLIED_PROFILE.into(),
            token: format!(
                "{}{}",
                crate::protocol::new_nonce(),
                crate::protocol::new_nonce()
            ),
            generation,
            session_id: envelope.get("sessionId")?.as_str()?.to_string(),
            epoch: item.get("sessionEpoch")?.as_u64()?,
            revision: item.get("revision")?.as_u64()?,
            hash: item.get("hash")?.as_str()?.to_string(),
            message_id: envelope.get("messageId")?.as_str()?.to_string(),
        };
        // Only trusted host state envelopes enter this path, but keep retained
        // records independently bounded if a future caller gets that wrong.
        if receipt.session_id.len() > 128
            || receipt.message_id.len() > 128
            || receipt.hash.len() != 64
            || !receipt.hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
        if self.pending.len() == MAX_PENDING_APPLIED {
            self.pending.pop_front();
            self.overflow = self.overflow.saturating_add(1);
        }
        let token = receipt.token.clone();
        envelope
            .as_object_mut()?
            .insert("stateApplied".into(), serde_json::to_value(&receipt).ok()?);
        self.pending.push_back(Pending {
            receipt,
            deadline_ms: now_ms.saturating_add(APPLIED_TIMEOUT_MS),
        });
        self.progress = "sending";
        Some(token)
    }

    /// `queued` (mobile) and `written` (device link) are deliberately distinct.
    pub fn note_sent(&mut self, token: &str, progress: &'static str) {
        if let Some(p) = self.pending.iter().find(|p| p.receipt.token == token) {
            let incoming = &p.receipt;
            if self.sent.as_ref().is_none_or(|old| advances(incoming, old)) {
                self.sent = Some(incoming.clone());
                self.progress = progress;
            }
        }
    }

    pub fn failed(&mut self, token: &str) {
        self.pending.retain(|p| p.receipt.token != token);
        self.progress = "send-failed";
    }

    /// The transport must first check current connection, member and admission.
    /// Equal tuple and random token are necessary; a bare revision is never an ack.
    pub fn acknowledge(
        &mut self,
        receipt: &StateAppliedReceipt,
        generation: u64,
        now_ms: u64,
    ) -> bool {
        self.expire(now_ms);
        let index = self.pending.iter().position(|p| &p.receipt == receipt);
        if !self.negotiated
            || self.generation != Some(generation)
            || receipt.generation != generation
            || index.is_none()
        {
            self.rejected = self.rejected.saturating_add(1);
            return false;
        }
        if let Some(index) = index {
            self.pending.remove(index);
        }
        if self
            .applied
            .as_ref()
            .is_none_or(|old| advances(receipt, old))
        {
            self.applied = Some(receipt.clone());
            self.progress = "peer-applied";
        }
        true
    }

    pub fn status(&mut self, generation: u64, now_ms: u64) -> StateDelivery {
        if self.generation != Some(generation) {
            self.invalidate();
        }
        self.expire(now_ms);
        StateDelivery {
            negotiated: self.negotiated,
            progress: if self.negotiated {
                self.progress
            } else {
                "unconfirmed"
            },
            outstanding: self.pending.len(),
            expired: self.expired,
            overflow: self.overflow,
            rejected: self.rejected,
            cancelled: self.cancelled,
            sent: self.sent.clone(),
            applied: self.applied.clone(),
        }
    }
}

fn advances(incoming: &StateAppliedReceipt, old: &StateAppliedReceipt) -> bool {
    incoming.session_id == old.session_id
        && (incoming.epoch > old.epoch
            || (incoming.epoch == old.epoch && incoming.revision >= old.revision))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(revision: u64) -> Value {
        json!({"messageType":"state","messageId":format!("host-{revision}"),"sessionId":"session.home",
            "payload":{"kind":"snapshot","sessionEpoch":2,"revision":revision,"hash":"a".repeat(64),"state":{}}})
    }
    fn prepare(t: &mut StateAppliedTracker, revision: u64, now: u64) -> StateAppliedReceipt {
        let mut envelope = state(revision);
        let token = t.prepare(&mut envelope, 7, now).unwrap();
        t.note_sent(&token, "written");
        serde_json::from_value(envelope["stateApplied"].clone()).unwrap()
    }

    #[test]
    fn writes_are_not_applied_and_legacy_peers_receive_no_extension() {
        let mut t = StateAppliedTracker::default();
        let mut e = state(1);
        assert!(t.prepare(&mut e, 7, 0).is_none());
        assert!(e.get("stateApplied").is_none());
        t.negotiate(7, true);
        let r = prepare(&mut t, 1, 0);
        let s = t.status(7, 1);
        assert_eq!(s.progress, "written");
        assert!(s.applied.is_none());
        assert!(t.acknowledge(&r, 7, 2));
        assert_eq!(t.status(7, 2).applied.unwrap().revision, 1);
    }

    #[test]
    fn every_binding_field_must_match_an_outstanding_sent_state() {
        let mut t = StateAppliedTracker::default();
        t.negotiate(7, true);
        let good = prepare(&mut t, 3, 0);
        for key in [
            "profile",
            "token",
            "sessionId",
            "hash",
            "messageId",
            "generation",
            "epoch",
            "revision",
        ] {
            let mut raw = serde_json::to_value(&good).unwrap();
            raw[key] = if raw[key].is_string() {
                json!("forged")
            } else {
                json!(900)
            };
            let wrong = serde_json::from_value(raw).unwrap();
            assert!(!t.acknowledge(&wrong, 7, 1), "{key}");
        }
        assert!(t.acknowledge(&good, 7, 2));
        assert!(
            !t.acknowledge(&good, 7, 3),
            "replay is not an outstanding transfer"
        );
    }

    #[test]
    fn delayed_valid_receipts_make_progress_without_rolling_back_newer_claims() {
        let mut t = StateAppliedTracker::default();
        t.negotiate(7, true);
        let a = prepare(&mut t, 1, 0);
        let b = prepare(&mut t, 2, 1);
        assert!(t.acknowledge(&a, 7, 2));
        assert_eq!(t.status(7, 2).applied.unwrap().revision, 1);
        let c = prepare(&mut t, 3, 3);
        assert!(t.acknowledge(&c, 7, 4));
        assert!(t.acknowledge(&b, 7, 5));
        assert_eq!(t.status(7, 5).applied.unwrap().revision, 3);
    }

    #[test]
    fn timeout_capacity_generation_and_stop_are_bounded_and_invalidate_old_receipts() {
        let mut t = StateAppliedTracker::default();
        t.negotiate(7, true);
        let first = prepare(&mut t, 0, 0);
        for rev in 1..40 {
            prepare(&mut t, rev, 0);
        }
        assert_eq!(t.status(7, 1).outstanding, MAX_PENDING_APPLIED);
        assert_eq!(t.status(7, 1).overflow, 8);
        assert!(!t.acknowledge(&first, 7, 1));
        let late = prepare(&mut t, 41, 1);
        assert!(!t.acknowledge(&late, 7, APPLIED_TIMEOUT_MS + 1));
        assert_eq!(t.status(7, APPLIED_TIMEOUT_MS + 1).outstanding, 0);
        let cancelled = prepare(&mut t, 42, 10_000);
        t.cancel();
        assert!(!t.acknowledge(&cancelled, 7, 10_001));
        let previous = prepare(&mut t, 43, 10_002);
        t.negotiate(8, true);
        assert!(!t.acknowledge(&previous, 8, 10_003));
        assert!(t.status(8, 10_003).applied.is_none());
    }
}
