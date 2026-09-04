//! Ports（`docs/aip/architecture-boundaries.md` §2）：Session 對外的穩定介面。
//!
//! 每個 port 只有 1–3 個方法；可選能力一律靠 AIP capability 協商表達，不用 optional method。
//! 這裡**沒有** I/O：`Clock` 注入時間，`SessionStore` 由 adapter 實作。附的
//! [`MemoryStore`]／[`FixedClock`] 是測試用實作，容量有界。

use std::collections::BTreeMap;
use std::sync::Mutex;

use interaction_aip::{bind_identity, IdentityDecision, Party, Timestamp};

use crate::types::Snapshot;

/// Port 錯誤。不含路徑、token 或輸入回顯（AIP §5）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    #[error("session store is unavailable")]
    Unavailable,
    #[error("session store rejected the snapshot")]
    Rejected,
    #[error("stored session data is corrupt")]
    Corrupt,
}

/// 時間來源。Session 本身不讀時鐘；host 用這個把時間注入 use case 層。
pub trait Clock {
    fn now(&self) -> Timestamp;
}

/// Snapshot 持久化（§6：每 N 個 revision 或每 60 s 存一次）。
pub trait SessionStore {
    fn save(&self, snapshot: &Snapshot) -> Result<(), PortError>;
    fn load(&self, session_id: &str) -> Result<Option<Snapshot>, PortError>;
}

/// Transport 身分 vs `source` 宣稱（AIP §5）。不符一律拒絕，**不得**修正後執行。
pub trait IdentityVerifier {
    fn verify(&self, transport_identity: &Party, claimed: &Party) -> IdentityDecision;
}

/// `consentGrantId` 是否有效。AI／adapter／裝置**不能**授予 consent。
///
/// **1.0 沒有接進 [`crate::CharacterSession::gate`]，而且刻意如此。** `consentGrantId` 只出現在
/// host→裝置、需要授權的 `command` 上；成員送進來的 inbound 訊息本來就沒有理由帶 grant，
/// 所以 gate 對「帶 grant 的 inbound 訊息」一律 `rejected{scope-denied}`——不需要問任何驗證器，
/// 也不會有「驗證器說 yes 就放行」的路徑（fail-closed 比可設定更安全）。
///
/// 這個 port 留給 host 端的**外送**方向（Consent Service 發出的 grant 在送 command 之前自我檢查）
/// 與 1.1 以後的 `approval-*` 流程。它是純分類函式，不是 session 安全管線的一環：讀契約時
/// 不要把它當成「host 會驗成員帶來的 grant」（capability-consent-055）。
pub trait ConsentVerifier {
    fn is_valid(&self, grant_id: &str, now: Timestamp) -> bool;
}

/// Renderer port：能呈現哪些 Behavior Intent。
pub trait RendererPort {
    fn party(&self) -> Party;
    fn intents(&self) -> &[String];
}

/// Device port：能產生哪些 event name。
pub trait DevicePort {
    fn party(&self) -> Party;
    fn inputs(&self) -> &[String];
}

/// 直接比對綁定身分的預設驗證器（`bind_identity`）。
#[derive(Debug, Clone, Copy, Default)]
pub struct StrictIdentityVerifier;

impl IdentityVerifier for StrictIdentityVerifier {
    fn verify(&self, transport_identity: &Party, claimed: &Party) -> IdentityDecision {
        bind_identity(transport_identity, claimed)
    }
}

/// 一律不承認 grant 的 consent 驗證器（fail-closed 預設）。
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyAllConsent;

impl ConsentVerifier for DenyAllConsent {
    fn is_valid(&self, _grant_id: &str, _now: Timestamp) -> bool {
        false
    }
}

/// 測試用固定時鐘（可推進）。
#[derive(Debug)]
pub struct FixedClock {
    now: Mutex<Timestamp>,
}

impl FixedClock {
    pub fn new(now: Timestamp) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    /// 推進時鐘；鎖中毒時保持原值（不 panic）。
    pub fn advance_ms(&self, millis: i64) {
        if let Ok(mut guard) = self.now.lock() {
            *guard += chrono::Duration::milliseconds(millis);
        }
    }

    pub fn set(&self, now: Timestamp) {
        if let Ok(mut guard) = self.now.lock() {
            *guard = now;
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.now
            .lock()
            .map(|guard| *guard)
            .unwrap_or_else(|poisoned| *poisoned.into_inner())
    }
}

/// 記憶體 SessionStore：容量有界（預設 8 個 session），滿了拒絕新 session 而不是無限成長。
#[derive(Debug)]
pub struct MemoryStore {
    cap: usize,
    entries: Mutex<BTreeMap<String, Snapshot>>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new(8)
    }
}

impl MemoryStore {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.clamp(1, 64),
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SessionStore for MemoryStore {
    fn save(&self, snapshot: &Snapshot) -> Result<(), PortError> {
        let mut entries = self.entries.lock().map_err(|_| PortError::Unavailable)?;
        if !entries.contains_key(&snapshot.session_id) && entries.len() >= self.cap {
            return Err(PortError::Rejected);
        }
        entries.insert(snapshot.session_id.clone(), snapshot.clone());
        Ok(())
    }

    fn load(&self, session_id: &str) -> Result<Option<Snapshot>, PortError> {
        let entries = self.entries.lock().map_err(|_| PortError::Unavailable)?;
        Ok(entries.get(session_id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn t0() -> Timestamp {
        Utc.with_ymd_and_hms(2026, 9, 4, 12, 30, 0)
            .single()
            .expect("fixed timestamp")
    }

    fn snapshot(id: &str) -> Snapshot {
        Snapshot {
            session_id: id.to_string(),
            epoch: 1,
            revision: 1,
            sequence: 0,
            state: json!({}),
            hash: crate::state_hash(&json!({})),
            at: t0(),
        }
    }

    #[test]
    fn memory_store_is_bounded() {
        let store = MemoryStore::new(2);
        store.save(&snapshot("a")).expect("first");
        store.save(&snapshot("b")).expect("second");
        assert_eq!(store.save(&snapshot("c")), Err(PortError::Rejected));
        // 覆寫既有 session 不受容量限制。
        store.save(&snapshot("a")).expect("overwrite");
        assert_eq!(store.len(), 2);
        assert_eq!(store.load("a").expect("load").map(|s| s.epoch), Some(1));
        assert_eq!(store.load("zz").expect("load"), None);
    }

    #[test]
    fn fixed_clock_advances() {
        let clock = FixedClock::new(t0());
        assert_eq!(clock.now(), t0());
        clock.advance_ms(1_500);
        assert_eq!(clock.now(), t0() + chrono::Duration::milliseconds(1_500));
    }

    #[test]
    fn strict_identity_verifier_rejects_mismatches() {
        let v = StrictIdentityVerifier;
        assert_eq!(
            v.verify(&Party::device("a"), &Party::device("a")),
            IdentityDecision::Accept
        );
        assert!(matches!(
            v.verify(&Party::device("a"), &Party::device("b")),
            IdentityDecision::Reject { .. }
        ));
        assert!(!DenyAllConsent.is_valid("grant_1", t0()));
    }
}
