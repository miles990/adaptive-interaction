//! Embedded SQLite storage.
//!
//! Human-editable configuration stays in files (File=Truth); this store holds
//! high-frequency runtime state: receipts, plans, sessions, observation
//! metadata and the audit trail. All methods are synchronous and cheap; the
//! runtime wraps calls appropriately.

use chrono::{DateTime, Utc};
use interaction_core::{
    ActionId, ActionReceipt, DomainError, DomainResult, Observation, ObservationQuery, Plan,
    PlanId, Session, SessionId,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

const CURRENT_SCHEMA: i64 = 1;

pub struct Store {
    conn: Mutex<Connection>,
}

fn ts_to_str(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn map_err(e: rusqlite::Error) -> DomainError {
    DomainError::Storage(e.to_string())
}

fn map_json(e: serde_json::Error) -> DomainError {
    DomainError::Storage(format!("json: {e}"))
}

impl Store {
    pub fn open(path: &Path) -> DomainResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DomainError::Storage(format!("create {parent:?}: {e}")))?;
        }
        let conn = Connection::open(path).map_err(map_err)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> DomainResult<Self> {
        let conn = Connection::open_in_memory().map_err(map_err)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> DomainResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(map_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(map_err)?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> DomainResult<()> {
        let conn = self.conn.lock().expect("store lock");
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(map_err)?;
        if version >= CURRENT_SCHEMA {
            return Ok(());
        }
        if version < 1 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS receipts (
                    action_id   TEXT PRIMARY KEY,
                    plan_id     TEXT NOT NULL,
                    session_id  TEXT NOT NULL,
                    actuator_id TEXT NOT NULL,
                    channel     TEXT NOT NULL DEFAULT '',
                    intent      TEXT NOT NULL,
                    status      TEXT NOT NULL,
                    json        TEXT NOT NULL,
                    created_at  TEXT NOT NULL,
                    updated_at  TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_receipts_session ON receipts(session_id);
                CREATE INDEX IF NOT EXISTS idx_receipts_actuator_time ON receipts(actuator_id, created_at);
                CREATE INDEX IF NOT EXISTS idx_receipts_status ON receipts(status);

                CREATE TABLE IF NOT EXISTS plans (
                    plan_id    TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    status     TEXT NOT NULL,
                    json       TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS sessions (
                    session_id TEXT PRIMARY KEY,
                    state      TEXT NOT NULL,
                    json       TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS observations (
                    observation_id TEXT PRIMARY KEY,
                    receptor_id    TEXT NOT NULL,
                    session_id     TEXT,
                    at             TEXT NOT NULL,
                    json           TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_obs_receptor_time ON observations(receptor_id, at);

                CREATE TABLE IF NOT EXISTS audit (
                    id     INTEGER PRIMARY KEY AUTOINCREMENT,
                    at     TEXT NOT NULL,
                    kind   TEXT NOT NULL,
                    actor  TEXT NOT NULL,
                    detail TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS meta (
                    key   TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                "#,
            )
            .map_err(map_err)?;
        }
        conn.pragma_update(None, "user_version", CURRENT_SCHEMA)
            .map_err(map_err)?;
        Ok(())
    }

    // ---- meta ----

    pub fn set_meta(&self, key: &str, value: &str) -> DomainResult<()> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> DomainResult<Option<String>> {
        let conn = self.conn.lock().expect("store lock");
        conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(map_err)
    }

    // ---- receipts ----

    const TERMINAL_STATUSES: &'static str =
        "'completed','blocked','failed','uncertain','cancelled','expired','stopped'";

    /// Upsert a receipt. Terminal states are STICKY: once a stored receipt is
    /// terminal (e.g. `stopped` written by the emergency-stop sweep, `expired`
    /// written by the watchdog), a concurrent in-flight executor can no longer
    /// overwrite or resurrect it. Returns `true` when the write was applied,
    /// `false` when it was refused because the stored receipt is terminal.
    pub fn upsert_receipt(&self, receipt: &ActionReceipt, channel: &str) -> DomainResult<bool> {
        let json = serde_json::to_string(receipt).map_err(map_json)?;
        let created = receipt
            .timestamps
            .first()
            .map(|(_, t)| ts_to_str(*t))
            .unwrap_or_else(|| ts_to_str(Utc::now()));
        let updated = receipt
            .timestamps
            .last()
            .map(|(_, t)| ts_to_str(*t))
            .unwrap_or_else(|| created.clone());
        let status = serde_json::to_string(&receipt.current_status)
            .map_err(map_json)?
            .trim_matches('"')
            .to_string();
        let conn = self.conn.lock().expect("store lock");
        let sql = format!(
            "INSERT INTO receipts(action_id, plan_id, session_id, actuator_id, channel, intent, status, json, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(action_id) DO UPDATE SET
                status = excluded.status, json = excluded.json, updated_at = excluded.updated_at,
                channel = CASE WHEN excluded.channel != '' THEN excluded.channel ELSE receipts.channel END
             WHERE receipts.status NOT IN ({})",
            Self::TERMINAL_STATUSES
        );
        conn.execute(
            &sql,
            params![
                receipt.action_id.as_str(),
                receipt.plan_id.as_str(),
                receipt.session_id.as_str(),
                receipt.actuator_id.as_str(),
                channel,
                receipt.intent,
                status,
                json,
                created,
                updated,
            ],
        )
        .map_err(map_err)?;
        // `changes()` is 0 when the conflict-update was suppressed by the
        // terminal guard — the caller's copy is stale.
        let applied = conn.changes() > 0;
        if !applied {
            tracing::debug!(
                action_id = receipt.action_id.as_str(),
                attempted = %status,
                "receipt write refused: stored receipt already terminal"
            );
        }
        Ok(applied)
    }

    pub fn receipt(&self, action_id: &ActionId) -> DomainResult<ActionReceipt> {
        let conn = self.conn.lock().expect("store lock");
        let json: String = conn
            .query_row(
                "SELECT json FROM receipts WHERE action_id = ?1",
                params![action_id.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?
            .ok_or_else(|| DomainError::NotFound(format!("action {action_id}")))?;
        serde_json::from_str(&json).map_err(map_json)
    }

    pub fn receipts(
        &self,
        session_id: Option<&SessionId>,
        limit: u32,
    ) -> DomainResult<Vec<ActionReceipt>> {
        let conn = self.conn.lock().expect("store lock");
        let mut out = Vec::new();
        match session_id {
            Some(sid) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT json FROM receipts WHERE session_id = ?1 ORDER BY created_at DESC LIMIT ?2",
                    )
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map(params![sid.as_str(), limit], |r| r.get::<_, String>(0))
                    .map_err(map_err)?;
                for row in rows {
                    out.push(serde_json::from_str(&row.map_err(map_err)?).map_err(map_json)?);
                }
            }
            None => {
                let mut stmt = conn
                    .prepare("SELECT json FROM receipts ORDER BY created_at DESC LIMIT ?1")
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map(params![limit], |r| r.get::<_, String>(0))
                    .map_err(map_err)?;
                for row in rows {
                    out.push(serde_json::from_str(&row.map_err(map_err)?).map_err(map_json)?);
                }
            }
        }
        Ok(out)
    }

    /// Non-terminal receipts (used by emergency stop and crash recovery).
    pub fn open_receipts(&self) -> DomainResult<Vec<ActionReceipt>> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT json FROM receipts WHERE status NOT IN
                 ('completed','blocked','failed','uncertain','cancelled','expired','stopped')",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(map_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row.map_err(map_err)?).map_err(map_json)?);
        }
        Ok(out)
    }

    /// Usage counters for the governor.
    pub fn actuator_usage(
        &self,
        actuator_id: &str,
        now: DateTime<Utc>,
    ) -> DomainResult<(u32, Option<DateTime<Utc>>)> {
        let hour_ago = ts_to_str(now - chrono::Duration::hours(1));
        let conn = self.conn.lock().expect("store lock");
        let fired: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM receipts
                 WHERE actuator_id = ?1 AND created_at >= ?2
                   AND status NOT IN ('blocked')",
                params![actuator_id, hour_ago],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        let last: Option<String> = conn
            .query_row(
                "SELECT MAX(created_at) FROM receipts
                 WHERE actuator_id = ?1 AND status NOT IN ('blocked')",
                params![actuator_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?
            .flatten();
        let last_ts = last
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|t| t.with_timezone(&Utc));
        Ok((fired, last_ts))
    }

    /// Sum of effective durations (ms) on a channel within a session.
    pub fn channel_usage_ms(&self, session_id: &SessionId, channel: &str) -> DomainResult<u64> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT json FROM receipts WHERE session_id = ?1 AND channel = ?2
                 AND status NOT IN ('blocked','cancelled','expired')",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![session_id.as_str(), channel], |r| {
                r.get::<_, String>(0)
            })
            .map_err(map_err)?;
        let mut total: u64 = 0;
        for row in rows {
            let receipt: ActionReceipt =
                serde_json::from_str(&row.map_err(map_err)?).map_err(map_json)?;
            total = total.saturating_add(
                receipt
                    .effective_bounded_parameters
                    .duration_ms
                    .unwrap_or(0),
            );
        }
        Ok(total)
    }

    pub fn scheduled_action_count(&self) -> DomainResult<u32> {
        let conn = self.conn.lock().expect("store lock");
        conn.query_row(
            "SELECT COUNT(*) FROM receipts WHERE status IN ('authorized','accepted','dispatched')",
            [],
            |r| r.get(0),
        )
        .map_err(map_err)
    }

    // ---- plans ----

    pub fn upsert_plan(&self, plan: &Plan) -> DomainResult<()> {
        let json = serde_json::to_string(plan).map_err(map_json)?;
        let status = serde_json::to_string(&plan.status)
            .map_err(map_json)?
            .trim_matches('"')
            .to_string();
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO plans(plan_id, session_id, status, json, created_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(plan_id) DO UPDATE SET status = excluded.status, json = excluded.json",
            params![
                plan.plan_id.as_str(),
                plan.session_id.as_str(),
                status,
                json,
                ts_to_str(plan.created_at),
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn plan(&self, plan_id: &PlanId) -> DomainResult<Plan> {
        let conn = self.conn.lock().expect("store lock");
        let json: String = conn
            .query_row(
                "SELECT json FROM plans WHERE plan_id = ?1",
                params![plan_id.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?
            .ok_or_else(|| DomainError::NotFound(format!("plan {plan_id}")))?;
        serde_json::from_str(&json).map_err(map_json)
    }

    // ---- sessions ----

    pub fn upsert_session(&self, session: &Session) -> DomainResult<()> {
        let json = serde_json::to_string(session).map_err(map_json)?;
        let state = serde_json::to_string(&session.state)
            .map_err(map_json)?
            .trim_matches('"')
            .to_string();
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO sessions(session_id, state, json, started_at, updated_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(session_id) DO UPDATE SET
                state = excluded.state, json = excluded.json, updated_at = excluded.updated_at",
            params![
                session.session_id.as_str(),
                state,
                json,
                ts_to_str(session.started_at),
                ts_to_str(Utc::now()),
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn session(&self, session_id: &SessionId) -> DomainResult<Session> {
        let conn = self.conn.lock().expect("store lock");
        let json: String = conn
            .query_row(
                "SELECT json FROM sessions WHERE session_id = ?1",
                params![session_id.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?
            .ok_or_else(|| DomainError::NotFound(format!("session {session_id}")))?;
        serde_json::from_str(&json).map_err(map_json)
    }

    pub fn latest_active_session(&self) -> DomainResult<Option<Session>> {
        let conn = self.conn.lock().expect("store lock");
        let json: Option<String> = conn
            .query_row(
                "SELECT json FROM sessions WHERE state = 'active' ORDER BY started_at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?;
        json.map(|j| serde_json::from_str(&j).map_err(map_json))
            .transpose()
    }

    // ---- observations ----

    pub fn insert_observation(&self, obs: &Observation) -> DomainResult<()> {
        let json = serde_json::to_string(obs).map_err(map_json)?;
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT OR REPLACE INTO observations(observation_id, receptor_id, session_id, at, json)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                obs.observation_id.as_str(),
                obs.receptor_id.as_str(),
                obs.session_id.as_ref().map(|s| s.as_str()),
                ts_to_str(obs.timestamp),
                json,
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn query_observations(&self, query: &ObservationQuery) -> DomainResult<Vec<Observation>> {
        let limit = query.limit.unwrap_or(50).min(500);
        let conn = self.conn.lock().expect("store lock");
        let mut sql = String::from("SELECT json FROM observations WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(r) = &query.receptor_id {
            sql.push_str(" AND receptor_id = ?");
            args.push(Box::new(r.as_str().to_string()));
        }
        if let Some(s) = &query.session_id {
            sql.push_str(" AND session_id = ?");
            args.push(Box::new(s.as_str().to_string()));
        }
        if let Some(since) = query.since {
            sql.push_str(" AND at >= ?");
            args.push(Box::new(ts_to_str(since)));
        }
        sql.push_str(" ORDER BY at DESC LIMIT ?");
        args.push(Box::new(limit));
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(params_ref.as_slice(), |r| r.get::<_, String>(0))
            .map_err(map_err)?;
        let mut out: Vec<Observation> = Vec::new();
        for row in rows {
            let obs: Observation =
                serde_json::from_str(&row.map_err(map_err)?).map_err(map_json)?;
            out.push(obs);
        }
        // Post-filter by freshness/confidence (JSON fields).
        let now = Utc::now();
        if let Some(max_age) = query.max_age_ms {
            out.retain(|o| !o.is_stale(now, max_age));
        }
        if let Some(minc) = query.min_confidence {
            out.retain(|o| o.confidence >= minc);
        }
        Ok(out)
    }

    /// Delete observations older than the retention window (privacy).
    pub fn prune_observations(&self, older_than: DateTime<Utc>) -> DomainResult<u32> {
        let conn = self.conn.lock().expect("store lock");
        let n = conn
            .execute(
                "DELETE FROM observations WHERE at < ?1",
                params![ts_to_str(older_than)],
            )
            .map_err(map_err)?;
        Ok(n as u32)
    }

    // ---- audit ----

    pub fn audit(&self, kind: &str, actor: &str, detail: &serde_json::Value) -> DomainResult<()> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO audit(at, kind, actor, detail) VALUES (?1,?2,?3,?4)",
            params![ts_to_str(Utc::now()), kind, actor, detail.to_string()],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn audit_tail(&self, limit: u32) -> DomainResult<Vec<serde_json::Value>> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare("SELECT at, kind, actor, detail FROM audit ORDER BY id DESC LIMIT ?1")
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![limit], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(map_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (at, kind, actor, detail) = row.map_err(map_err)?;
            out.push(serde_json::json!({
                "at": at,
                "kind": kind,
                "actor": actor,
                "detail": serde_json::from_str::<serde_json::Value>(&detail).unwrap_or(serde_json::Value::String(detail)),
            }));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interaction_core::*;

    fn receipt_for(actuator: &str, session: &SessionId, status: ActionStatus) -> ActionReceipt {
        let now = Utc::now();
        let action = BoundedAction {
            action_id: ActionId::generate(),
            plan_id: PlanId::generate(),
            session_id: session.clone(),
            actuator_id: ActuatorId::new(actuator),
            intent: "test".into(),
            risk_class: RiskClass::Low,
            requested: ActionParameters {
                duration_ms: Some(1000),
                ..Default::default()
            },
            effective: ActionParameters {
                duration_ms: Some(800),
                ..Default::default()
            },
            policy_decisions: vec![],
            expires_at: now + chrono::Duration::seconds(30),
            issued_at: now,
            correlation_id: CorrelationId::generate(),
            metadata: Default::default(),
            schema_version: SCHEMA_VERSION.into(),
        };
        let mut receipt = ActionReceipt::for_action(&action, now);
        if status != ActionStatus::Authorized {
            // walk forward legally
            let _ = receipt.transition(ActionStatus::Accepted, now);
            if status != ActionStatus::Accepted {
                let _ = receipt.transition(status, now);
            }
        }
        receipt
    }

    #[test]
    fn receipt_roundtrip_and_usage() {
        let store = Store::open_in_memory().unwrap();
        let session = SessionId::generate();
        let r1 = receipt_for("conversation", &session, ActionStatus::Accepted);
        store.upsert_receipt(&r1, "conversation").unwrap();
        let loaded = store.receipt(&r1.action_id).unwrap();
        assert_eq!(loaded.action_id, r1.action_id);
        assert_eq!(loaded.current_status, ActionStatus::Accepted);

        let (fired, last) = store.actuator_usage("conversation", Utc::now()).unwrap();
        assert_eq!(fired, 1);
        assert!(last.is_some());

        let used = store.channel_usage_ms(&session, "conversation").unwrap();
        assert_eq!(used, 800);

        assert_eq!(store.scheduled_action_count().unwrap(), 1);
        assert_eq!(store.open_receipts().unwrap().len(), 1);
    }

    #[test]
    fn terminal_receipts_are_sticky() {
        let store = Store::open_in_memory().unwrap();
        let session = SessionId::generate();
        let mut receipt = receipt_for("mock", &session, ActionStatus::Accepted);
        assert!(store.upsert_receipt(&receipt, "haptic").unwrap());

        // Emergency-stop sweep marks it stopped.
        let mut stopped = receipt.clone();
        stopped
            .transition(ActionStatus::Stopped, Utc::now())
            .unwrap();
        assert!(store.upsert_receipt(&stopped, "").unwrap());

        // A racing executor with a stale copy tries to advance it — refused.
        receipt
            .transition(ActionStatus::Dispatched, Utc::now())
            .unwrap();
        receipt
            .transition(ActionStatus::Acknowledged, Utc::now())
            .unwrap();
        assert!(!store.upsert_receipt(&receipt, "haptic").unwrap());
        assert_eq!(
            store.receipt(&receipt.action_id).unwrap().current_status,
            ActionStatus::Stopped,
            "terminal state must survive concurrent overwrite attempts"
        );
    }

    #[test]
    fn session_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut s = Session::new(Utc::now(), Some("test".into()), None);
        store.upsert_session(&s).unwrap();
        assert!(store.latest_active_session().unwrap().is_some());
        s.stop(Utc::now());
        store.upsert_session(&s).unwrap();
        assert!(store.latest_active_session().unwrap().is_none());
    }

    #[test]
    fn observation_query_filters() {
        let store = Store::open_in_memory().unwrap();
        let now = Utc::now();
        let fresh = Observation::now(ReceptorId::new("a"), "t", now).with_fact("x", 1);
        let old = Observation::now(ReceptorId::new("a"), "t", now - chrono::Duration::hours(2));
        store.insert_observation(&fresh).unwrap();
        store.insert_observation(&old).unwrap();
        let all = store
            .query_observations(&ObservationQuery {
                receptor_id: Some(ReceptorId::new("a")),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(all.len(), 2);
        let recent = store
            .query_observations(&ObservationQuery {
                receptor_id: Some(ReceptorId::new("a")),
                max_age_ms: Some(60_000),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(recent.len(), 1);
        let pruned = store
            .prune_observations(now - chrono::Duration::hours(1))
            .unwrap();
        assert_eq!(pruned, 1);
    }

    #[test]
    fn audit_trail() {
        let store = Store::open_in_memory().unwrap();
        store
            .audit("emergency.stop", "cli", &serde_json::json!({"why": "test"}))
            .unwrap();
        let tail = store.audit_tail(10).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0]["kind"], "emergency.stop");
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/interaction.db");
        {
            let store = Store::open(&path).unwrap();
            store.set_meta("clean_shutdown", "false").unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.get_meta("clean_shutdown").unwrap().as_deref(),
            Some("false")
        );
    }
}
