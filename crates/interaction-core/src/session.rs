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
        self.state == SessionState::Active
            && self.expires_at.map(|e| now <= e).unwrap_or(true)
    }

    pub fn has_consent(&self, scope: &ConsentScope, now: Timestamp) -> bool {
        self.consents
            .iter()
            .any(|c| &c.scope == scope && c.is_active(now))
    }

    pub fn grant(&mut self, scope: ConsentScope, now: Timestamp, expires_at: Option<Timestamp>) {
        // Revoke duplicates first so the latest grant wins.
        self.revoke(&scope, now);
        self.consents.push(Consent { scope, granted_at: now, expires_at, revoked_at: None });
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
        s.grant(scope.clone(), now, Some(now + chrono::Duration::seconds(10)));
        assert!(s.has_consent(&scope, now));
        assert!(!s.has_consent(&scope, now + chrono::Duration::seconds(11)));
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
