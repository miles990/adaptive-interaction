//! Sessions: the consent and budget boundary for everything the runtime does.

use crate::{Consent, ConsentScope, SessionId, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    Active,
    Stopped,
    Expired,
}

/// What `Session::consume_one_shot` actually did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentConsumption {
    /// No active consent matched any of the scopes offered.
    NotFound,
    /// An unlimited (TTL-only) consent matched; nothing to spend.
    Unlimited,
    /// One use was debited from a bounded consent.
    Consumed { scope: ConsentScope, remaining: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub session_id: SessionId,
    pub state: SessionState,
    pub started_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<Timestamp>,
    #[serde(default)]
    pub consents: Vec<Consent>,
    /// Cumulative usage per channel (active ms) for budget enforcement.
    #[serde(default)]
    pub channel_usage_ms: BTreeMap<String, u64>,
    /// Cumulative monetary spend in USD.
    #[serde(default)]
    pub monetary_spent: f64,
    /// Free-form label ("cli", "api", "desktop", agent name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub schema_version: String,
}

impl Session {
    pub fn new(now: Timestamp, label: Option<String>, ttl_ms: Option<u64>) -> Self {
        Self {
            session_id: SessionId::generate(),
            state: SessionState::Active,
            started_at: now,
            expires_at: ttl_ms.map(|ms| now + chrono::Duration::milliseconds(ms as i64)),
            stopped_at: None,
            consents: Vec::new(),
            channel_usage_ms: BTreeMap::new(),
            monetary_spent: 0.0,
            label,
            schema_version: crate::SCHEMA_VERSION.to_string(),
        }
    }

    pub fn is_active(&self, now: Timestamp) -> bool {
        self.state == SessionState::Active && self.expires_at.map(|e| now <= e).unwrap_or(true)
    }

    pub fn has_consent(&self, scope: &ConsentScope, now: Timestamp) -> bool {
        self.consents
            .iter()
            .any(|c| &c.scope == scope && c.is_active(now))
    }

    /// Granted and neither revoked nor expired, ignoring the use counter.
    /// Only the pre-dispatch gate should need this (see `Consent::still_granted`).
    pub fn has_consent_ignoring_uses(&self, scope: &ConsentScope, now: Timestamp) -> bool {
        self.consents
            .iter()
            .any(|c| &c.scope == scope && c.still_granted(now))
    }

    pub fn grant(&mut self, scope: ConsentScope, now: Timestamp, expires_at: Option<Timestamp>) {
        self.grant_with_uses(scope, now, expires_at, None);
    }

    /// `max_uses = Some(1)` is the real "only this once": the first authorized
    /// dispatch spends it. `None` keeps the unlimited-within-TTL behaviour.
    pub fn grant_with_uses(
        &mut self,
        scope: ConsentScope,
        now: Timestamp,
        expires_at: Option<Timestamp>,
        max_uses: Option<u32>,
    ) {
        // Revoke duplicates first so the latest grant wins.
        self.revoke(&scope, now);
        self.consents.push(Consent {
            scope,
            granted_at: now,
            expires_at,
            revoked_at: None,
            max_uses,
            remaining_uses: max_uses,
        });
    }

    /// Spend one use of whichever consent authorizes this action.
    ///
    /// `scopes` is in the Governor's own priority order (actuator before
    /// channel), so the counter that gets debited is the one the authorization
    /// actually leaned on. Consuming is deliberately NOT refundable: consent is
    /// exercised the instant the Governor says "authorized"; unlike money there
    /// is no later moment where we learn it "wasn't really used".
    pub fn consume_one_shot(
        &mut self,
        scopes: &[ConsentScope],
        now: Timestamp,
    ) -> ConsentConsumption {
        for scope in scopes {
            let Some(consent) = self
                .consents
                .iter_mut()
                .find(|c| &c.scope == scope && c.is_active(now))
            else {
                continue;
            };
            let Some(remaining) = consent.remaining_uses else {
                return ConsentConsumption::Unlimited;
            };
            let left = remaining.saturating_sub(1);
            consent.remaining_uses = Some(left);
            return ConsentConsumption::Consumed {
                scope: scope.clone(),
                remaining: left,
            };
        }
        ConsentConsumption::NotFound
    }

    pub fn revoke(&mut self, scope: &ConsentScope, now: Timestamp) -> bool {
        let mut hit = false;
        for c in self.consents.iter_mut() {
            if &c.scope == scope && c.revoked_at.is_none() {
                c.revoked_at = Some(now);
                hit = true;
            }
        }
        hit
    }

    pub fn stop(&mut self, now: Timestamp) {
        self.state = SessionState::Stopped;
        self.stopped_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_grant_and_revoke() {
        let now = chrono::Utc::now();
        let mut s = Session::new(now, None, None);
        let scope = ConsentScope::Channel("haptic".into());
        assert!(!s.has_consent(&scope, now));
        s.grant(scope.clone(), now, None);
        assert!(s.has_consent(&scope, now));
        assert!(s.revoke(&scope, now));
        assert!(!s.has_consent(&scope, now));
    }

    #[test]
    fn consent_expiry() {
        let now = chrono::Utc::now();
        let mut s = Session::new(now, None, None);
        let scope = ConsentScope::Actuator("mock.actuator".into());
        s.grant(
            scope.clone(),
            now,
            Some(now + chrono::Duration::seconds(10)),
        );
        assert!(s.has_consent(&scope, now));
        assert!(!s.has_consent(&scope, now + chrono::Duration::seconds(11)));
    }

    #[test]
    fn consent_max_uses_exhausts_after_first_consume() {
        let now = chrono::Utc::now();
        let mut s = Session::new(now, None, None);
        let scope = ConsentScope::Actuator("mock.actuator".into());
        s.grant_with_uses(scope.clone(), now, None, Some(1));
        assert!(s.has_consent(&scope, now));

        let outcome = s.consume_one_shot(std::slice::from_ref(&scope), now);
        assert_eq!(
            outcome,
            ConsentConsumption::Consumed {
                scope: scope.clone(),
                remaining: 0
            }
        );
        assert!(!s.has_consent(&scope, now), "用過一次即失效");
        assert_eq!(s.consents[0].remaining_uses, Some(0));
        assert_eq!(s.consents[0].max_uses, Some(1));
        // 用完之後再消耗一次不會「借」到下一次。
        assert_eq!(
            s.consume_one_shot(std::slice::from_ref(&scope), now),
            ConsentConsumption::NotFound
        );
        assert_eq!(s.consents[0].remaining_uses, Some(0));
    }

    #[test]
    fn consent_consume_prefers_the_actuator_scope_over_the_channel_scope() {
        let now = chrono::Utc::now();
        let mut s = Session::new(now, None, None);
        let actuator = ConsentScope::Actuator("mock.actuator".into());
        let channel = ConsentScope::Channel("haptic".into());
        s.grant_with_uses(actuator.clone(), now, None, Some(1));
        s.grant_with_uses(channel.clone(), now, None, Some(1));

        // Governor 的優先序是動器先於頻道；被扣的必須是同一筆。
        let outcome = s.consume_one_shot(&[actuator.clone(), channel.clone()], now);
        assert_eq!(
            outcome,
            ConsentConsumption::Consumed {
                scope: actuator.clone(),
                remaining: 0
            }
        );
        assert!(!s.has_consent(&actuator, now));
        assert!(s.has_consent(&channel, now), "頻道那筆不該被連坐扣掉");
    }

    #[test]
    fn consent_max_uses_none_keeps_unlimited_behaviour() {
        let now = chrono::Utc::now();
        let mut s = Session::new(now, None, None);
        let scope = ConsentScope::Channel("haptic".into());
        s.grant(scope.clone(), now, None);
        for _ in 0..5 {
            assert_eq!(
                s.consume_one_shot(std::slice::from_ref(&scope), now),
                ConsentConsumption::Unlimited
            );
            assert!(s.has_consent(&scope, now));
        }
        assert_eq!(s.consents[0].remaining_uses, None);
    }

    #[test]
    fn consent_deserializes_old_json_without_the_use_counters() {
        // 舊 sessions blob（v0.5.0 之前）沒有 maxUses/remainingUses：讀回來
        // 必須是「不限次」，不是「零次」——否則升級會把既有授權全部鎖死。
        let raw = r#"{
            "sessionId": "session-1",
            "state": "active",
            "startedAt": "2026-01-01T00:00:00Z",
            "consents": [
                {
                    "scope": {"kind": "channel", "id": "haptic"},
                    "grantedAt": "2026-01-01T00:00:00Z"
                }
            ],
            "channelUsageMs": {},
            "monetarySpent": 0.0,
            "schemaVersion": "1.0"
        }"#;
        let session: Session = serde_json::from_str(raw).expect("old blob must still parse");
        let scope = ConsentScope::Channel("haptic".into());
        assert_eq!(session.consents[0].max_uses, None);
        assert_eq!(session.consents[0].remaining_uses, None);
        assert!(session.has_consent(&scope, chrono::Utc::now()));
    }

    #[test]
    fn revoked_one_shot_consent_is_not_still_granted() {
        let now = chrono::Utc::now();
        let mut s = Session::new(now, None, None);
        let scope = ConsentScope::Actuator("mock.actuator".into());
        s.grant_with_uses(scope.clone(), now, None, Some(1));
        s.consume_one_shot(std::slice::from_ref(&scope), now);
        // 用完但沒撤銷：派工前的閘門看得到它仍然「被授予過」。
        assert!(s.has_consent_ignoring_uses(&scope, now));
        // 真的撤銷之後，連派工前的閘門也擋。
        assert!(s.revoke(&scope, now));
        assert!(!s.has_consent_ignoring_uses(&scope, now));
    }

    #[test]
    fn stopped_session_inactive() {
        let now = chrono::Utc::now();
        let mut s = Session::new(now, None, None);
        assert!(s.is_active(now));
        s.stop(now);
        assert!(!s.is_active(now));
    }
}
