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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const CURRENT_SCHEMA: i64 = 8;

pub struct Store {
    conn: Mutex<Connection>,
    /// Number of [`Store::transaction`] scopes opened so far. Test-only
    /// observability (a caller that must write once atomically can assert it
    /// did not silently regress to per-statement autocommit); never read by
    /// production code.
    txn_count: AtomicU64,
    /// Fault-injection seam for [`Store::transaction`]: inert unless a test
    /// arms it with [`Store::force_next_transaction_error`]. It is a plain
    /// field rather than a `#[cfg(test)]` item because the tests that need it
    /// are integration tests in *other* crates, which link this crate without
    /// `cfg(test)`. Nothing in the HTTP/CLI surface can reach it.
    forced_txn_error: Mutex<Option<String>>,
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
            txn_count: AtomicU64::new(0),
            forced_txn_error: Mutex::new(None),
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
        if version < 2 {
            // AI-assisted capability descriptions, bound to the manifest hash
            // they were written against; stale hashes are ignored on read.
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS ai_descriptions (
                    kind          TEXT NOT NULL,
                    capability_id TEXT NOT NULL,
                    locale        TEXT NOT NULL,
                    manifest_hash TEXT NOT NULL,
                    text          TEXT NOT NULL,
                    created_at    TEXT NOT NULL,
                    PRIMARY KEY (kind, capability_id, locale)
                );
                "#,
            )
            .map_err(map_err)?;
        }
        if version < 3 {
            // Capability providers (devices/services/agents/sessions) and
            // agent-session records. JSON documents keyed by id; identity and
            // lifecycle live in the domain model, storage only persists.
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS providers (
                    id         TEXT PRIMARY KEY,
                    body       TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS agent_sessions (
                    id         TEXT PRIMARY KEY,
                    body       TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                "#,
            )
            .map_err(map_err)?;
        }
        if version < 4 {
            // v0.4 記憶層：typed 查詢欄位＋JSON 本體。到期清除與層級查詢
            // 走欄位索引，不掃 JSON。
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS memory_items (
                    id           TEXT PRIMARY KEY,
                    layer        TEXT NOT NULL,
                    kind         TEXT NOT NULL,
                    expires_at   TEXT,
                    review_after TEXT,
                    updated_at   TEXT NOT NULL,
                    body         TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_memory_layer ON memory_items(layer, updated_at);
                CREATE INDEX IF NOT EXISTS idx_memory_expiry ON memory_items(expires_at);
                "#,
            )
            .map_err(map_err)?;
        }
        if version < 5 {
            // v0.4 知識系統：內容定址素材中繼資料＋版本化知識圖譜＋FTS5 全文。
            // blob 本體在檔案系統（CAS）；這裡只有中繼資料與圖。
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS assets (
                    hash       TEXT PRIMARY KEY,
                    media_type TEXT NOT NULL,
                    size       INTEGER NOT NULL,
                    added_at   TEXT NOT NULL,
                    body       TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS knowledge_nodes (
                    id         TEXT PRIMARY KEY,
                    node_type  TEXT NOT NULL,
                    status     TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    body       TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_kn_status ON knowledge_nodes(status, updated_at);
                CREATE TABLE IF NOT EXISTS knowledge_edges (
                    id       TEXT PRIMARY KEY,
                    from_id  TEXT NOT NULL,
                    to_id    TEXT NOT NULL,
                    relation TEXT NOT NULL,
                    status   TEXT NOT NULL,
                    body     TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_ke_from ON knowledge_edges(from_id);
                CREATE INDEX IF NOT EXISTS idx_ke_to ON knowledge_edges(to_id);
                CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_fts
                    USING fts5(node_id UNINDEXED, title, content);
                "#,
            )
            .map_err(map_err)?;
        }
        if version < 6 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS knowledge_receipts (
                    id         TEXT PRIMARY KEY,
                    created_at TEXT NOT NULL,
                    body       TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_kr_time ON knowledge_receipts(created_at);
                "#,
            )
            .map_err(map_err)?;
        }
        if version < 7 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS asset_derivatives (
                    id          TEXT PRIMARY KEY,
                    parent_hash TEXT NOT NULL,
                    kind        TEXT NOT NULL,
                    status      TEXT NOT NULL,
                    created_at  TEXT NOT NULL,
                    body        TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_asset_derivatives_parent
                    ON asset_derivatives(parent_hash, created_at);
                "#,
            )
            .map_err(map_err)?;
        }
        if version < 8 {
            // v8：Character Presentation Protocol 外部 adapter 登記
            // （token 只存 sha256；撤銷旗標隨 body 一起持久化，重啟後仍生效）。
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS character_adapters (
                    id         TEXT PRIMARY KEY,
                    body       TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                "#,
            )
            .map_err(map_err)?;
        }
        conn.pragma_update(None, "user_version", CURRENT_SCHEMA)
            .map_err(map_err)?;
        Ok(())
    }

    // ---- AI-assisted descriptions ----

    /// Store an AI-assisted description for a capability. `manifest_hash` is
    /// the hash of the manifest the text was written against.
    pub fn set_ai_description(
        &self,
        kind: &str,
        capability_id: &str,
        locale: &str,
        manifest_hash: &str,
        text: &str,
    ) -> DomainResult<()> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO ai_descriptions(kind, capability_id, locale, manifest_hash, text, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(kind, capability_id, locale)
             DO UPDATE SET manifest_hash = ?4, text = ?5, created_at = ?6",
            params![
                kind,
                capability_id,
                locale,
                manifest_hash,
                text,
                Utc::now().to_rfc3339()
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    /// Fetch an AI description only when it still matches `current_hash`;
    /// a stale description (manifest changed since) is treated as absent.
    pub fn ai_description(
        &self,
        kind: &str,
        capability_id: &str,
        locale: &str,
        current_hash: &str,
    ) -> DomainResult<Option<String>> {
        let conn = self.conn.lock().expect("store lock");
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT manifest_hash, text FROM ai_descriptions
                 WHERE kind = ?1 AND capability_id = ?2 AND locale = ?3",
                params![kind, capability_id, locale],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(map_err)?;
        Ok(row.and_then(|(hash, text)| (hash == current_hash).then_some(text)))
    }

    pub fn delete_ai_description(
        &self,
        kind: &str,
        capability_id: &str,
        locale: &str,
    ) -> DomainResult<bool> {
        let conn = self.conn.lock().expect("store lock");
        let n = conn
            .execute(
                "DELETE FROM ai_descriptions
                 WHERE kind = ?1 AND capability_id = ?2 AND locale = ?3",
                params![kind, capability_id, locale],
            )
            .map_err(map_err)?;
        Ok(n > 0)
    }

    // ---- meta ----

    // ---- JSON document tables (providers / agent sessions) ----

    fn doc_upsert(&self, table: &str, id: &str, body: &str) -> DomainResult<()> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            &format!(
                "INSERT INTO {table}(id, body, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at"
            ),
            params![id, body, chrono::Utc::now().to_rfc3339()],
        )
        .map_err(map_err)?;
        Ok(())
    }

    fn doc_all(&self, table: &str) -> DomainResult<Vec<String>> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(&format!("SELECT body FROM {table} ORDER BY id"))
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(map_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_err)
    }

    fn doc_delete(&self, table: &str, id: &str) -> DomainResult<()> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])
            .map_err(map_err)?;
        Ok(())
    }

    pub fn save_provider(&self, id: &str, body: &str) -> DomainResult<()> {
        self.doc_upsert("providers", id, body)
    }

    pub fn all_providers(&self) -> DomainResult<Vec<String>> {
        self.doc_all("providers")
    }

    pub fn delete_provider(&self, id: &str) -> DomainResult<()> {
        self.doc_delete("providers", id)
    }

    // ---- Character Presentation Protocol：外部 adapter 登記（v8）----

    pub fn save_character_adapter(&self, id: &str, body: &str) -> DomainResult<()> {
        self.doc_upsert("character_adapters", id, body)
    }

    pub fn all_character_adapters(&self) -> DomainResult<Vec<String>> {
        self.doc_all("character_adapters")
    }

    pub fn delete_character_adapter(&self, id: &str) -> DomainResult<()> {
        self.doc_delete("character_adapters", id)
    }

    pub fn save_agent_session(&self, id: &str, body: &str) -> DomainResult<()> {
        self.doc_upsert("agent_sessions", id, body)
    }

    pub fn all_agent_sessions(&self) -> DomainResult<Vec<String>> {
        self.doc_all("agent_sessions")
    }

    pub fn delete_agent_session(&self, id: &str) -> DomainResult<()> {
        self.doc_delete("agent_sessions", id)
    }

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

    // ---- transactions ----

    /// Run several writes as ONE atomic unit: the closure's writes commit
    /// together or not at all. Used where a half-written set of rows would be
    /// a lie about what the user asked for (the first-run wizard's commit).
    ///
    /// The closure gets a [`StoreTxn`] and must use *only* its writers: the
    /// connection mutex is held for the whole scope, so calling a `Store`
    /// method from inside would deadlock.
    ///
    /// Any `Err` from the closure rolls the transaction back and is returned
    /// unchanged; a rollback failure is reported instead of being swallowed.
    pub fn transaction<T, F>(&self, f: F) -> DomainResult<T>
    where
        F: FnOnce(&StoreTxn<'_>) -> DomainResult<T>,
    {
        self.txn_count.fetch_add(1, Ordering::SeqCst);
        let mut conn = self.conn.lock().expect("store lock");
        let tx = conn.transaction().map_err(map_err)?;
        let scoped = StoreTxn { tx };
        let value = match f(&scoped) {
            Ok(v) => v,
            Err(e) => {
                scoped.tx.rollback().map_err(map_err)?;
                return Err(e);
            }
        };
        // Armed faults fire here, after the writes and before the commit —
        // the shape a real commit failure has, so the caller's compensation
        // path is exercised for real.
        let forced = self.forced_txn_error.lock().expect("store lock").take();
        if let Some(message) = forced {
            scoped.tx.rollback().map_err(map_err)?;
            return Err(DomainError::Storage(message));
        }
        scoped.tx.commit().map_err(map_err)?;
        Ok(value)
    }

    /// Test seam: make the next [`Store::transaction`] fail at commit time.
    /// Inert until armed; consumed by the next transaction.
    #[doc(hidden)]
    pub fn force_next_transaction_error(&self, message: &str) {
        *self.forced_txn_error.lock().expect("store lock") = Some(message.to_string());
    }

    /// Test seam: how many [`Store::transaction`] scopes have been opened.
    #[doc(hidden)]
    pub fn transaction_count(&self) -> u64 {
        self.txn_count.load(Ordering::SeqCst)
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

    /// Preserve late driver/verifier evidence without ever changing the
    /// already-terminal status. This is used when a watchdog or emergency
    /// stop wins the race against an in-flight driver.
    pub fn merge_terminal_receipt_evidence(
        &self,
        attempted: &ActionReceipt,
        reason: &str,
    ) -> DomainResult<ActionReceipt> {
        let conn = self.conn.lock().expect("store lock");
        let json: String = conn
            .query_row(
                "SELECT json FROM receipts WHERE action_id = ?1",
                params![attempted.action_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_err)?
            .ok_or_else(|| DomainError::NotFound(format!("action {}", attempted.action_id)))?;
        let mut stored: ActionReceipt = serde_json::from_str(&json).map_err(map_json)?;
        if !stored.is_terminal() {
            return Err(DomainError::Conflict(format!(
                "action {} is not terminal",
                attempted.action_id
            )));
        }
        for (key, value) in &attempted.driver_response {
            stored.driver_response.insert(key.clone(), value.clone());
        }
        for error in &attempted.errors {
            if !stored.errors.contains(error) {
                stored.errors.push(error.clone());
            }
        }
        if stored.verification.is_none() {
            stored.verification.clone_from(&attempted.verification);
        }
        stored.push_error(
            "late_evidence",
            format!("terminal status preserved while merging late evidence: {reason}"),
            Utc::now(),
        );
        let merged = serde_json::to_string(&stored).map_err(map_json)?;
        conn.execute(
            "UPDATE receipts SET json = ?2, updated_at = ?3 WHERE action_id = ?1",
            params![attempted.action_id.as_str(), merged, ts_to_str(Utc::now())],
        )
        .map_err(map_err)?;
        Ok(stored)
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

    /// Receipts in the given statuses, newest first (used by the human inbox
    /// to find every open decision item).
    ///
    /// `receipts(None, n)` only returns the newest `n` rows regardless of
    /// status, so an older receipt that still needs a human decision (a
    /// terminal `uncertain`/`blocked` row — sticky, with no ack/dismiss path)
    /// silently drops out of the badge once `n` newer receipts exist. This
    /// query goes straight at the status index instead. `limit` is the caller's
    /// honest scan bound: when the result length equals `limit` there may be
    /// more, and the caller MUST report the count as inexact rather than
    /// claiming "nothing pending".
    pub fn receipts_with_status(
        &self,
        statuses: &[&str],
        limit: u32,
    ) -> DomainResult<Vec<ActionReceipt>> {
        if statuses.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.clamp(1, 5000);
        let placeholders = vec!["?"; statuses.len()].join(",");
        let sql = format!(
            "SELECT json FROM receipts WHERE status IN ({placeholders}) ORDER BY created_at DESC LIMIT ?"
        );
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = statuses
            .iter()
            .map(|s| Box::new(s.to_string()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        args.push(Box::new(limit));
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(params_ref.as_slice(), |r| r.get::<_, String>(0))
            .map_err(map_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row.map_err(map_err)?).map_err(map_json)?);
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

// ---------------------------------------------------------------------------
// v4：記憶層。
// ---------------------------------------------------------------------------

impl Store {
    pub fn save_memory(
        &self,
        id: &str,
        layer: &str,
        kind: &str,
        expires_at: Option<&str>,
        review_after: Option<&str>,
        body: &str,
    ) -> Result<(), DomainError> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO memory_items (id, layer, kind, expires_at, review_after, updated_at, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET layer=?2, kind=?3, expires_at=?4, review_after=?5, updated_at=?6, body=?7",
            rusqlite::params![id, layer, kind, expires_at, review_after, ts_to_str(chrono::Utc::now()), body],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn get_memory(&self, id: &str) -> Result<Option<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare("SELECT body FROM memory_items WHERE id = ?1")
            .map_err(map_err)?;
        let mut rows = stmt.query([id]).map_err(map_err)?;
        match rows.next().map_err(map_err)? {
            Some(row) => Ok(Some(row.get(0).map_err(map_err)?)),
            None => Ok(None),
        }
    }

    pub fn delete_memory(&self, id: &str) -> Result<bool, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let n = conn
            .execute("DELETE FROM memory_items WHERE id = ?1", [id])
            .map_err(map_err)?;
        Ok(n > 0)
    }

    /// 列出（layer 過濾可選；updated_at 新→舊；bounded）。
    pub fn list_memory(&self, layer: Option<&str>, limit: u32) -> Result<Vec<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let limit = limit.clamp(1, 1000);
        let mut out = Vec::new();
        match layer {
            Some(l) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT body FROM memory_items WHERE layer = ?1 ORDER BY updated_at DESC LIMIT ?2",
                    )
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map(rusqlite::params![l, limit], |r| r.get::<_, String>(0))
                    .map_err(map_err)?;
                for r in rows {
                    out.push(r.map_err(map_err)?);
                }
            }
            None => {
                let mut stmt = conn
                    .prepare("SELECT body FROM memory_items ORDER BY updated_at DESC LIMIT ?1")
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map([limit], |r| r.get::<_, String>(0))
                    .map_err(map_err)?;
                for r in rows {
                    out.push(r.map_err(map_err)?);
                }
            }
        }
        Ok(out)
    }

    /// 資料庫裡實際的記憶筆數（layer 過濾可選）。
    ///
    /// `list_memory` 只回一頁，呼叫端無法分辨「剛好裝滿一頁」與「後面還有」；
    /// 匯出／掃描要判斷有沒有被截斷，必須拿真值來比，不能用「這頁滿了」去猜——
    /// 猜出來的截斷警告在剛好等於上限時是誤報。
    pub fn count_memory(&self, layer: Option<&str>) -> Result<u32, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let n: i64 = match layer {
            Some(l) => conn
                .query_row(
                    "SELECT COUNT(*) FROM memory_items WHERE layer = ?1",
                    [l],
                    |r| r.get(0),
                )
                .map_err(map_err)?,
            None => conn
                .query_row("SELECT COUNT(*) FROM memory_items", [], |r| r.get(0))
                .map_err(map_err)?,
        };
        Ok(n.clamp(0, u32::MAX as i64) as u32)
    }

    /// 依 delete_with_parent（隨父素材刪除）找出**所有**衍生記憶 id。
    /// 全表 json_extract 掃描——刪素材是罕見的人類動作，級聯完整性
    /// 優先於速度；不設 recency 窗或上限（recency 窗會讓舊衍生物
    /// 靜默逃過級聯）。
    pub fn list_memory_ids_by_delete_parent(
        &self,
        parent_hash: &str,
    ) -> Result<Vec<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT id FROM memory_items
                 WHERE json_extract(body, '$.retention.deleteWithParent') = ?1",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([parent_hash], |r| r.get::<_, String>(0))
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }

    /// 刪除已過期記憶（expiresAt 到期＝停止使用並刪除）。回傳刪除數。
    pub fn prune_expired_memory(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<u32, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let n = conn
            .execute(
                "DELETE FROM memory_items WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                [ts_to_str(now)],
            )
            .map_err(map_err)?;
        Ok(n as u32)
    }
}

// ---------------------------------------------------------------------------
// v5：知識系統（素材中繼資料、圖譜、FTS5）。
// ---------------------------------------------------------------------------

impl Store {
    /// 素材中繼資料 write-once：同 hash 再寫直接拒絕（AI 不可覆寫來源）。
    pub fn insert_asset(
        &self,
        hash: &str,
        media_type: &str,
        size: u64,
        body: &str,
    ) -> Result<bool, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let n = conn
            .execute(
                "INSERT OR IGNORE INTO assets (hash, media_type, size, added_at, body)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    hash,
                    media_type,
                    size as i64,
                    ts_to_str(chrono::Utc::now()),
                    body
                ],
            )
            .map_err(map_err)?;
        Ok(n > 0)
    }

    pub fn get_asset(&self, hash: &str) -> Result<Option<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare("SELECT body FROM assets WHERE hash = ?1")
            .map_err(map_err)?;
        let mut rows = stmt.query([hash]).map_err(map_err)?;
        match rows.next().map_err(map_err)? {
            Some(row) => Ok(Some(row.get(0).map_err(map_err)?)),
            None => Ok(None),
        }
    }

    pub fn list_assets(&self, limit: u32) -> Result<Vec<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare("SELECT body FROM assets ORDER BY added_at DESC LIMIT ?1")
            .map_err(map_err)?;
        let rows = stmt
            .query_map([limit.clamp(1, 1000)], |r| r.get::<_, String>(0))
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }

    pub fn delete_asset(&self, hash: &str) -> Result<bool, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let n = conn
            .execute("DELETE FROM assets WHERE hash = ?1", [hash])
            .map_err(map_err)?;
        Ok(n > 0)
    }

    pub fn save_asset_derivative(
        &self,
        id: &str,
        parent_hash: &str,
        kind: &str,
        status: &str,
        body: &str,
    ) -> Result<(), DomainError> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT OR REPLACE INTO asset_derivatives
             (id, parent_hash, kind, status, created_at, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                id,
                parent_hash,
                kind,
                status,
                ts_to_str(chrono::Utc::now()),
                body
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn list_asset_derivatives(&self, parent_hash: &str) -> Result<Vec<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT body FROM asset_derivatives
                 WHERE parent_hash = ?1 ORDER BY created_at, id",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([parent_hash], |row| row.get::<_, String>(0))
            .map_err(map_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(map_err)?);
        }
        Ok(out)
    }

    pub fn delete_asset_derivatives(&self, parent_hash: &str) -> Result<u32, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let count = conn
            .execute(
                "DELETE FROM asset_derivatives WHERE parent_hash = ?1",
                [parent_hash],
            )
            .map_err(map_err)?;
        Ok(count as u32)
    }

    pub fn count_asset_derivative_output_references(
        &self,
        output_hash: &str,
    ) -> Result<u32, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM asset_derivatives
                 WHERE json_extract(body, '$.outputHash') = ?1",
                [output_hash],
                |row| row.get(0),
            )
            .map_err(map_err)?;
        Ok(count.max(0) as u32)
    }

    pub fn save_knowledge_node(
        &self,
        id: &str,
        node_type: &str,
        status: &str,
        title: &str,
        content: &str,
        body: &str,
    ) -> Result<(), DomainError> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO knowledge_nodes (id, node_type, status, updated_at, body)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET node_type=?2, status=?3, updated_at=?4, body=?5",
            rusqlite::params![id, node_type, status, ts_to_str(chrono::Utc::now()), body],
        )
        .map_err(map_err)?;
        // FTS 同步：先刪舊列再插新列。
        conn.execute("DELETE FROM knowledge_fts WHERE node_id = ?1", [id])
            .map_err(map_err)?;
        conn.execute(
            "INSERT INTO knowledge_fts (node_id, title, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, title, content],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn get_knowledge_node(&self, id: &str) -> Result<Option<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare("SELECT body FROM knowledge_nodes WHERE id = ?1")
            .map_err(map_err)?;
        let mut rows = stmt.query([id]).map_err(map_err)?;
        match rows.next().map_err(map_err)? {
            Some(row) => Ok(Some(row.get(0).map_err(map_err)?)),
            None => Ok(None),
        }
    }

    pub fn list_knowledge_nodes(
        &self,
        status: Option<&str>,
        limit: u32,
    ) -> Result<Vec<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let limit = limit.clamp(1, 1000);
        let mut out = Vec::new();
        match status {
            Some(st) => {
                let mut stmt = conn
                    .prepare("SELECT body FROM knowledge_nodes WHERE status = ?1 ORDER BY updated_at DESC LIMIT ?2")
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map(rusqlite::params![st, limit], |r| r.get::<_, String>(0))
                    .map_err(map_err)?;
                for r in rows {
                    out.push(r.map_err(map_err)?);
                }
            }
            None => {
                let mut stmt = conn
                    .prepare("SELECT body FROM knowledge_nodes ORDER BY updated_at DESC LIMIT ?1")
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map([limit], |r| r.get::<_, String>(0))
                    .map_err(map_err)?;
                for r in rows {
                    out.push(r.map_err(map_err)?);
                }
            }
        }
        Ok(out)
    }

    /// keyset 分頁列出知識節點（id 升冪；回傳 (id, body)）。
    /// 供全量掃描（freshness sweep／向量索引重建）逐頁掃完——
    /// list_knowledge_nodes 的單次上限會把超過 1000 節點的圖譜靜默截斷。
    /// keyset 以不變的 id 為游標，掃描中改 status/updated_at 不影響進度。
    pub fn list_knowledge_nodes_page(
        &self,
        status: Option<&str>,
        after_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<(String, String)>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let limit = limit.clamp(1, 1000);
        let after = after_id.unwrap_or("");
        let mut out = Vec::new();
        match status {
            Some(st) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, body FROM knowledge_nodes
                         WHERE status = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
                    )
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map(rusqlite::params![st, after, limit], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(map_err)?;
                for r in rows {
                    out.push(r.map_err(map_err)?);
                }
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, body FROM knowledge_nodes
                         WHERE id > ?1 ORDER BY id LIMIT ?2",
                    )
                    .map_err(map_err)?;
                let rows = stmt
                    .query_map(rusqlite::params![after, limit], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(map_err)?;
                for r in rows {
                    out.push(r.map_err(map_err)?);
                }
            }
        }
        Ok(out)
    }

    /// FTS5 全文搜尋 → (node_id, bm25 分數，越小越好)。
    pub fn search_knowledge(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<(String, f64)>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT node_id, bm25(knowledge_fts) FROM knowledge_fts
                 WHERE knowledge_fts MATCH ?1 ORDER BY bm25(knowledge_fts) LIMIT ?2",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map(rusqlite::params![query, limit.clamp(1, 100)], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }

    pub fn save_knowledge_edge(
        &self,
        id: &str,
        from_id: &str,
        to_id: &str,
        relation: &str,
        status: &str,
        body: &str,
    ) -> Result<(), DomainError> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO knowledge_edges (id, from_id, to_id, relation, status, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET relation=?4, status=?5, body=?6",
            rusqlite::params![id, from_id, to_id, relation, status, body],
        )
        .map_err(map_err)?;
        Ok(())
    }

    /// 某節點的相鄰邊（兩個方向）。
    pub fn edges_touching(&self, node_id: &str, limit: u32) -> Result<Vec<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare("SELECT body FROM knowledge_edges WHERE from_id = ?1 OR to_id = ?1 LIMIT ?2")
            .map_err(map_err)?;
        let rows = stmt
            .query_map(rusqlite::params![node_id, limit.clamp(1, 500)], |r| {
                r.get::<_, String>(0)
            })
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }

    /// keyset 分頁列出某節點的相鄰邊（id 升冪；回傳 (id, body)）。
    /// 供衝突檢查逐頁掃完——edges_touching 的單次上限會漏看超出的邊。
    pub fn edges_touching_page(
        &self,
        node_id: &str,
        after_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<(String, String)>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let after = after_id.unwrap_or("");
        let mut stmt = conn
            .prepare(
                "SELECT id, body FROM knowledge_edges
                 WHERE (from_id = ?1 OR to_id = ?1) AND id > ?2 ORDER BY id LIMIT ?3",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map(
                rusqlite::params![node_id, after, limit.clamp(1, 500)],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }

    /// 引用某素材的知識節點 id（刪除影響預覽＋dispute 級聯）。
    /// 精確比對 evidence[].assetHash——不做 `LIKE %hash%` 子字串比對
    /// （內文順帶提到 hash 不算引用；萬用字元不可膨脹結果），且不設
    /// 上限：刪除級聯必須涵蓋**每一個**引用節點，截斷會讓 Active 知識
    /// 靜默保留懸空證據。
    pub fn nodes_referencing_asset(&self, hash: &str) -> Result<Vec<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT id FROM knowledge_nodes WHERE EXISTS (
                     SELECT 1 FROM json_each(knowledge_nodes.body, '$.evidence') je
                     WHERE json_extract(je.value, '$.assetHash') = ?1)",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([hash], |r| r.get::<_, String>(0))
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }
}

impl Store {
    pub fn save_knowledge_receipt(&self, id: &str, body: &str) -> Result<(), DomainError> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT OR REPLACE INTO knowledge_receipts (id, created_at, body) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, ts_to_str(chrono::Utc::now()), body],
        )
        .map_err(map_err)?;
        Ok(())
    }

    pub fn list_knowledge_receipts(&self, limit: u32) -> Result<Vec<String>, DomainError> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare("SELECT body FROM knowledge_receipts ORDER BY created_at DESC LIMIT ?1")
            .map_err(map_err)?;
        let rows = stmt
            .query_map([limit.clamp(1, 500)], |r| r.get::<_, String>(0))
            .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(map_err)?);
        }
        Ok(out)
    }
}

/// The write handle inside a [`Store::transaction`] scope. Deliberately tiny:
/// it exposes only the writers the atomic callers need, so a transaction can
/// never accidentally re-enter the connection mutex through a `Store` method.
pub struct StoreTxn<'a> {
    tx: rusqlite::Transaction<'a>,
}

impl StoreTxn<'_> {
    /// Same upsert as [`Store::set_meta`], scoped to this transaction.
    pub fn set_meta(&self, key: &str, value: &str) -> DomainResult<()> {
        self.tx
            .execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(map_err)?;
        Ok(())
    }

    /// Same append as [`Store::audit`], scoped to this transaction — the audit
    /// row commits with the change it describes or not at all.
    pub fn audit(&self, kind: &str, actor: &str, detail: &serde_json::Value) -> DomainResult<()> {
        self.tx
            .execute(
                "INSERT INTO audit(at, kind, actor, detail) VALUES (?1,?2,?3,?4)",
                params![ts_to_str(Utc::now()), kind, actor, detail.to_string()],
            )
            .map_err(map_err)?;
        Ok(())
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

    /// The inbox badge must not depend on a receipt still being in the recent-N
    /// window: an older `uncertain`/`blocked` receipt (sticky, no ack/dismiss
    /// path) has to be findable by status alone.
    #[test]
    fn receipts_with_status_finds_pending_rows_outside_the_recency_window() {
        let store = Store::open_in_memory().unwrap();
        let session = SessionId::generate();

        let mut old = receipt_for("mock", &session, ActionStatus::Uncertain);
        old.timestamps = vec![(
            ActionStatus::Uncertain,
            Utc::now() - chrono::Duration::hours(2),
        )];
        assert!(store.upsert_receipt(&old, "haptic").unwrap());
        for _ in 0..5 {
            let newer = receipt_for("conversation", &session, ActionStatus::Completed);
            assert!(store.upsert_receipt(&newer, "conversation").unwrap());
        }

        // The recency window no longer shows it...
        let window = store.receipts(None, 5).unwrap();
        assert_eq!(window.len(), 5);
        assert!(
            window.iter().all(|r| r.action_id != old.action_id),
            "the older pending receipt is pushed out of the recency window"
        );
        // ...but the status query still does.
        let pending = store
            .receipts_with_status(&["uncertain", "blocked"], 100)
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].action_id, old.action_id);

        // Other statuses are not swept in.
        assert!(store
            .receipts_with_status(&["blocked"], 100)
            .unwrap()
            .is_empty());
        assert!(store.receipts_with_status(&[], 100).unwrap().is_empty());

        // Hitting `limit` is the caller's signal that the count may be
        // incomplete — it must be reported, never silently rounded down.
        let mut blocked = receipt_for("mock", &session, ActionStatus::Authorized);
        blocked.current_status = ActionStatus::Blocked;
        blocked.timestamps = vec![(ActionStatus::Blocked, Utc::now())];
        assert!(store.upsert_receipt(&blocked, "haptic").unwrap());
        let capped = store
            .receipts_with_status(&["uncertain", "blocked"], 1)
            .unwrap();
        assert_eq!(capped.len(), 1, "limit honoured");
        assert_eq!(
            store
                .receipts_with_status(&["uncertain", "blocked"], 100)
                .unwrap()
                .len(),
            2,
            "both open decision items are there when the scan bound allows"
        );
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
    fn late_driver_evidence_is_kept_without_resurrecting_terminal_status() {
        let store = Store::open_in_memory().unwrap();
        let session = SessionId::generate();
        let receipt = receipt_for("mock", &session, ActionStatus::Accepted);
        store.upsert_receipt(&receipt, "haptic").unwrap();

        let mut stopped = receipt.clone();
        stopped
            .transition(ActionStatus::Stopped, Utc::now())
            .unwrap();
        store.upsert_receipt(&stopped, "").unwrap();

        let mut late = receipt;
        late.driver_response
            .insert("commandId".into(), serde_json::json!("driver-123"));
        late.push_error("driver_note", "command left driver", Utc::now());
        let merged = store
            .merge_terminal_receipt_evidence(&late, "watchdog won")
            .unwrap();

        assert_eq!(merged.current_status, ActionStatus::Stopped);
        assert_eq!(merged.driver_response["commandId"], "driver-123");
        assert!(merged
            .errors
            .iter()
            .any(|error| error.code == "late_evidence"));
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

    /// A transaction is all-or-nothing: a closure that fails half way must
    /// leave the store exactly as it was, and the error must reach the caller
    /// unchanged so it can compensate for whatever lives outside SQLite.
    #[test]
    fn transaction_commits_together_or_not_at_all() {
        let store = Store::open_in_memory().unwrap();
        let before = store.transaction_count();
        store
            .transaction(|tx| {
                tx.set_meta("a", "1")?;
                tx.set_meta("b", "2")?;
                tx.audit("t.ok", "test", &serde_json::json!({}))?;
                Ok(())
            })
            .unwrap();
        assert_eq!(store.get_meta("a").unwrap().as_deref(), Some("1"));
        assert_eq!(store.get_meta("b").unwrap().as_deref(), Some("2"));
        assert_eq!(store.transaction_count(), before + 1);

        let err = store
            .transaction(|tx| {
                tx.set_meta("a", "rolled-back")?;
                tx.set_meta("c", "3")?;
                Err::<(), _>(DomainError::Validation("nope".into()))
            })
            .expect_err("closure failed");
        assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
        assert_eq!(
            store.get_meta("a").unwrap().as_deref(),
            Some("1"),
            "a failed transaction must not leave a partial write behind"
        );
        assert_eq!(store.get_meta("c").unwrap(), None);
    }

    /// The fault-injection seam simulates a commit-time failure: the writes
    /// already made inside the scope must roll back too.
    #[test]
    fn forced_transaction_error_rolls_back() {
        let store = Store::open_in_memory().unwrap();
        store.set_meta("k", "old").unwrap();
        store.force_next_transaction_error("disk on fire");
        let err = store
            .transaction(|tx| tx.set_meta("k", "new"))
            .expect_err("forced failure");
        assert!(err.to_string().contains("disk on fire"), "{err}");
        assert_eq!(store.get_meta("k").unwrap().as_deref(), Some("old"));
        // Armed once, consumed once.
        store.transaction(|tx| tx.set_meta("k", "new")).unwrap();
        assert_eq!(store.get_meta("k").unwrap().as_deref(), Some("new"));
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

    #[test]
    fn asset_reference_matching_is_exact_and_unbounded() {
        let store = Store::open_in_memory().unwrap();
        let hash = "ab".repeat(32);
        // 205 個以 evidence.assetHash 真正引用素材的節點（超過舊 200 上限）。
        for i in 0..205 {
            let body = serde_json::json!({"evidence": [{"assetHash": hash}]}).to_string();
            store
                .save_knowledge_node(
                    &format!("kn-ref-{i:04}"),
                    "claim",
                    "active",
                    "t",
                    "c",
                    &body,
                )
                .unwrap();
        }
        // 內文順帶提到 hash 但 evidence 沒引用 → 不算引用。
        let prose =
            serde_json::json!({"evidence": [], "content": format!("提到 {hash} 而已")}).to_string();
        store
            .save_knowledge_node("kn-prose", "claim", "active", "t", "c", &prose)
            .unwrap();
        let refs = store.nodes_referencing_asset(&hash).unwrap();
        assert_eq!(refs.len(), 205, "每一個引用節點都要被找到，不得截斷");
        assert!(!refs.iter().any(|id| id == "kn-prose"), "子字串不算引用");
        // 萬用字元不可膨脹結果（精確比對）。
        assert!(store.nodes_referencing_asset("%").unwrap().is_empty());
    }

    #[test]
    fn delete_parent_lookup_scans_all_rows() {
        let store = Store::open_in_memory().unwrap();
        let hash = "cd".repeat(32);
        // 依附素材的衍生記憶先寫入（最舊——recency 窗會漏掉的位置）。
        let dep = serde_json::json!({"retention": {"deleteWithParent": hash}}).to_string();
        store
            .save_memory(
                "mem-dependent",
                "domain-knowledge",
                "inference",
                None,
                None,
                &dep,
            )
            .unwrap();
        // 1010 筆較新的無關記憶（超過舊 1000 recency 窗）。
        for i in 0..1010 {
            store
                .save_memory(
                    &format!("mem-filler-{i:04}"),
                    "session-context",
                    "fact",
                    None,
                    None,
                    r#"{"retention": {}}"#,
                )
                .unwrap();
        }
        let ids = store.list_memory_ids_by_delete_parent(&hash).unwrap();
        assert_eq!(ids, vec!["mem-dependent".to_string()]);
    }

    #[test]
    fn knowledge_node_pages_cover_all_rows() {
        let store = Store::open_in_memory().unwrap();
        for i in 0..1010 {
            store
                .save_knowledge_node(
                    &format!("kn-{i:04}"),
                    "claim",
                    if i % 2 == 0 { "active" } else { "candidate" },
                    "t",
                    "c",
                    "{}",
                )
                .unwrap();
        }
        // 全量分頁：總數不受單頁上限截斷、id 不重複。
        let mut seen = std::collections::BTreeSet::new();
        let mut after: Option<String> = None;
        loop {
            let page = store
                .list_knowledge_nodes_page(Some("active"), after.as_deref(), 400)
                .unwrap();
            let Some((last, _)) = page.last() else { break };
            after = Some(last.clone());
            let full = page.len() == 400;
            for (id, _) in page {
                assert!(seen.insert(id), "分頁不得重複");
            }
            if !full {
                break;
            }
        }
        assert_eq!(seen.len(), 505, "所有 active 節點都要被掃到");
    }

    /// `list_memory` 只能回「一頁」，呼叫端無法分辨「剛好裝滿」與「後面還有」。
    /// `count_memory` 給的是資料庫裡真正的筆數——匯出要用它判斷有沒有截斷。
    #[test]
    fn count_memory_matches_actual_row_count() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.count_memory(None).unwrap(), 0);
        assert_eq!(store.count_memory(Some("task-memory")).unwrap(), 0);

        for i in 0..7u32 {
            let layer = if i % 3 == 0 {
                "user-memory"
            } else {
                "task-memory"
            };
            store
                .save_memory(
                    &format!("mem-{i:04}"),
                    layer,
                    "fact",
                    None,
                    None,
                    &format!("{{\"memoryId\":\"mem-{i:04}\"}}"),
                )
                .unwrap();
        }
        assert_eq!(store.count_memory(None).unwrap(), 7);
        assert_eq!(store.count_memory(Some("user-memory")).unwrap(), 3);
        assert_eq!(store.count_memory(Some("task-memory")).unwrap(), 4);
        assert_eq!(store.count_memory(Some("no-such-layer")).unwrap(), 0);

        // 超過單頁上限時，count 仍是真值（不被 LIMIT 夾住）——這正是
        // 「剛好 1000 筆」與「1001 筆」得以分辨的依據。
        for i in 7..1002u32 {
            store
                .save_memory(
                    &format!("mem-{i:04}"),
                    "task-memory",
                    "fact",
                    None,
                    None,
                    "{}",
                )
                .unwrap();
        }
        assert_eq!(store.count_memory(None).unwrap(), 1002);
        assert_eq!(store.list_memory(None, 1000).unwrap().len(), 1000);
    }
}
