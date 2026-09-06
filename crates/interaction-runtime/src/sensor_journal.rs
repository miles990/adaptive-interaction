//! Runtime-owned durable stop reminders. SQLite commits the bounded document and
//! its audit together; this is independent of Character Session snapshot formats.
use crate::sensor_source::{CaptureEvidence, SourceKey, UnresolvedStop, MAX_UNRESOLVED_STOPS};
use interaction_core::{DomainError, DomainResult};
use interaction_storage::Store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const SENSOR_JOURNAL_KEY: &str = "sensor_stop_journal";
const FORMAT: u32 = 1;
const MAX_BYTES: usize = 256 * 1024;
const MAX_GENERATION: u64 = (1 << 53) - 1;
// Independent monotonic reservation survives a failed reminder write. A stale
// human dismissal can never name a later process's unrelated source.
const GENERATION_KEY: &str = "sensor_source_generation_high_water";
const GENERATION_BLOCK: u64 = 1 << 20;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredStop {
    source_id: String,
    generation: u64,
    process_incarnation: String,
    sensors: Vec<String>,
    evidence: Vec<CaptureEvidence>,
    since: chrono::DateTime<chrono::Utc>,
    reason: String,
    confirmed_stopped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_label: Option<String>,
}

impl From<&UnresolvedStop> for StoredStop {
    fn from(r: &UnresolvedStop) -> Self {
        Self {
            source_id: r.source_id.clone(),
            generation: r.generation,
            process_incarnation: r.process_incarnation.clone(),
            sensors: r.sensors.clone(),
            evidence: r.evidence.clone(),
            since: r.since,
            reason: r.reason.clone(),
            confirmed_stopped: false,
            source_label: r.source_label.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    format: u32,
    high_water: u64,
    overflow: u64,
    entries: Vec<StoredStop>,
}

pub(crate) struct SensorJournal {
    pub entries: BTreeMap<SourceKey, UnresolvedStop>,
    pub incarnation: String,
    high_water: u64,
    next_generation: u64,
    overflow: u64,
    parked: bool,
    note: Option<&'static str>,
    recovery_unknown: bool,
}

impl SensorJournal {
    pub fn open(store: &Store, clean_shutdown: bool) -> DomainResult<Self> {
        let previous = store
            .get_meta(GENERATION_KEY)?
            .map(|raw| raw.parse::<u64>())
            .transpose()
            .map_err(|_| {
                DomainError::Storage("sensor source generation metadata is invalid".into())
            })?
            .unwrap_or(0);
        let reserved = previous
            .checked_add(GENERATION_BLOCK)
            .filter(|n| *n <= MAX_GENERATION)
            .ok_or_else(|| DomainError::Storage("sensor source generation exhausted".into()))?;
        store.transaction(|txn| txn.set_meta(GENERATION_KEY, &reserved.to_string()))?;
        let mut journal = Self {
            entries: BTreeMap::new(),
            incarnation: uuid::Uuid::new_v4().to_string(),
            high_water: reserved,
            next_generation: previous,
            overflow: 0,
            parked: false,
            note: None,
            recovery_unknown: false,
        };
        let raw = match store.get_meta(SENSOR_JOURNAL_KEY) {
            Ok(None) => return Ok(journal),
            Ok(Some(raw)) => raw,
            Err(_) => {
                journal.park("unreadable");
                return Ok(journal);
            }
        };
        // Unclean restart cannot establish whether a failed final write reached
        // storage. Surface this independently of the known reminder records.
        journal.recovery_unknown = !clean_shutdown;
        if raw.len() > MAX_BYTES {
            journal.park("corrupt");
            return Ok(journal);
        }
        let format = serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|value| value.get("format").and_then(Value::as_u64));
        if format.is_some_and(|value| value > u64::from(FORMAT)) {
            journal.park("future-format");
            return Ok(journal);
        }
        let Ok(doc) = serde_json::from_str::<Document>(&raw) else {
            journal.park("corrupt");
            return Ok(journal);
        };
        let valid = doc.format == FORMAT
            && doc.high_water <= previous
            && doc.entries.len() <= MAX_UNRESOLVED_STOPS
            && doc.entries.iter().all(|r| {
                !r.source_id.is_empty()
                    && r.source_id.len() <= 512
                    && r.generation > 0
                    && r.generation <= doc.high_water
                    && uuid::Uuid::parse_str(&r.process_incarnation).is_ok()
                    && !r.confirmed_stopped
                    && !r.sensors.is_empty()
                    && r.sensors.len() <= 64
                    && r.sensors.iter().all(|s| !s.is_empty() && s.len() <= 512)
                    && r.sensors
                        .iter()
                        .all(|sensor| r.evidence.iter().any(|scope| &scope.sensor == sensor))
                    && !r.evidence.is_empty()
                    && r.evidence.len() <= 64
                    && r.evidence.iter().all(|e| {
                        r.sensors.contains(&e.sensor) && !e.scope.is_empty() && e.scope.len() <= 512
                    })
                    && r.reason.len() <= 256
                    && r.source_label.as_ref().is_none_or(|s| s.len() <= 512)
            });
        if !valid {
            journal.park("corrupt");
            return Ok(journal);
        }
        journal.overflow = doc.overflow;
        for r in doc.entries {
            let key = (r.source_id.clone(), r.generation);
            let entry = UnresolvedStop {
                source_id: r.source_id,
                generation: r.generation,
                process_incarnation: r.process_incarnation,
                sensors: r.sensors,
                evidence: r.evidence,
                since: r.since,
                last_known: Vec::new(),
                source_label: r.source_label,
                reason: r.reason,
                confirmed_stopped: false,
            };
            if journal.entries.insert(key, entry).is_some() {
                journal.entries.clear();
                journal.park("corrupt");
                return Ok(journal);
            }
        }
        Ok(journal)
    }

    fn park(&mut self, note: &'static str) {
        self.parked = true;
        self.note = Some(note);
        self.recovery_unknown = true;
    }

    pub fn health(&self) -> Value {
        json!({"overflowCount": self.overflow, "storage": self.note.unwrap_or("durable"),
            "recoveryUnknown": self.recovery_unknown, "parked": self.parked})
    }

    pub fn next_generation(&mut self, store: &Store) -> DomainResult<u64> {
        // Reserved before Runtime starts. Reminder writes may fail or park
        // without allowing generation reuse or removing the source's stop path.
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .filter(|n| *n <= self.high_water)
            .ok_or_else(|| {
                DomainError::Storage(
                    "sensor generation reservation exhausted; restart required".into(),
                )
            })?;
        if !self.parked {
            let _ = self.persist(store, None);
        }
        Ok(self.next_generation)
    }

    /// Losing attribution cannot establish that a capture stopped. The bounded
    /// aggregate remains sticky just like entry overflow, including on restart.
    pub fn record_unattributed_capture(&mut self, store: &Store) {
        self.overflow = self.overflow.max(1);
        self.recovery_unknown = true;
        if !self.parked {
            let _ = self.persist(store, None);
        }
    }

    pub fn record(&mut self, store: &Store, mut entry: UnresolvedStop) {
        let key = (entry.source_id.clone(), entry.generation);
        if let Some(previous) = self.entries.get(&key) {
            entry.since = entry.since.min(previous.since);
            // A changing live list is not negative stop evidence. Preserve the
            // old scopes first when the bounded union overflows.
            let mut evidence = previous.evidence.clone();
            for scope in entry.evidence {
                if !evidence.contains(&scope) {
                    evidence.push(scope);
                }
            }
            entry.evidence = evidence;
            let mut last_known = previous.last_known.clone();
            for capture in entry.last_known {
                if !last_known
                    .iter()
                    .any(|old| old.kind == capture.kind && old.started_by == capture.started_by)
                {
                    last_known.push(capture);
                }
            }
            entry.last_known = last_known;
        }
        if entry.evidence.len() > 64
            || entry.evidence.iter().any(|e| {
                e.sensor.is_empty()
                    || e.sensor.len() > 512
                    || e.scope.is_empty()
                    || e.scope.len() > 512
            })
        {
            self.overflow = 1;
            entry.evidence.retain(|e| {
                !e.sensor.is_empty()
                    && e.sensor.len() <= 512
                    && !e.scope.is_empty()
                    && e.scope.len() <= 512
            });
            entry.evidence.truncate(64);
        }
        entry.sensors = entry.evidence.iter().map(|e| e.sensor.clone()).collect();
        entry.sensors.sort();
        entry.sensors.dedup();
        entry.last_known.truncate(64);
        if entry.evidence.is_empty() {
            let _ = self.persist(store, None);
            return;
        }
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_UNRESOLVED_STOPS {
            // Retain the existing exact reminders and aggregate additional
            // unknowns. They can never evaporate into a zero-problem state.
            self.overflow = 1;
            let _ = self.persist(store, Some(("sensor.unresolved-stop-overflow", "runtime", json!({"overflowCount": self.overflow, "sourceId": entry.source_id, "generation": entry.generation}))));
            return;
        }
        if let Some(previous) = self.entries.get(&key) {
            entry.since = entry.since.min(previous.since);
        }
        self.entries.insert(key, entry);
        let _ = self.persist(store, None);
    }

    pub fn resolve(&mut self, store: &Store, key: &SourceKey, evidence: &[CaptureEvidence]) {
        let Some(before) = self.entries.get(key).cloned() else {
            return;
        };
        if before.process_incarnation != self.incarnation {
            return;
        }
        let mut next = before.clone();
        next.evidence.retain(|scope| !evidence.contains(scope));
        next.sensors = next
            .evidence
            .iter()
            .map(|scope| scope.sensor.clone())
            .collect();
        next.sensors.sort();
        next.sensors.dedup();
        next.last_known
            .retain(|capture| next.sensors.contains(&capture.kind));
        if next.sensors.is_empty() {
            self.entries.remove(key);
        } else {
            self.entries.insert(key.clone(), next);
        }
        if self.persist(store, Some(("sensor.unresolved-stop-resolved", "runtime", json!({
            "sourceId": key.0, "generation": key.1, "processIncarnation": self.incarnation,
            "evidence": evidence, "confirmedStopped": true,
        })))).is_err() { self.entries.insert(key.clone(), before); }
    }

    pub fn dismiss(
        &mut self,
        store: &Store,
        key: &SourceKey,
        evidence: &[CaptureEvidence],
        actor: &str,
    ) -> DomainResult<Value> {
        let before = self.entries.get(key).cloned().ok_or_else(|| {
            DomainError::NotFound(format!(
                "no unresolved stop for {} (generation {})",
                key.0, key.1
            ))
        })?;
        let detail = json!({"dismissed": true, "sourceId": before.source_id,
            "generation": before.generation, "processIncarnation": before.process_incarnation,
            "sensors": evidence.iter().map(|e| &e.sensor).collect::<Vec<_>>(), "since": before.since, "confirmedStopped": false,
            "note": "human reviewed and dismissed the reminder; no source stop confirmation"});
        let mut retained = before.clone();
        retained.evidence.retain(|scope| !evidence.contains(scope));
        retained.sensors = retained
            .evidence
            .iter()
            .map(|scope| scope.sensor.clone())
            .collect();
        retained.sensors.sort();
        retained.sensors.dedup();
        retained
            .last_known
            .retain(|capture| retained.sensors.contains(&capture.kind));
        if retained.evidence.is_empty() {
            self.entries.remove(key);
        } else {
            self.entries.insert(key.clone(), retained);
        }
        if let Err(error) = self.persist(
            store,
            Some(("sensor.unresolved-stop-dismissed", actor, detail.clone())),
        ) {
            self.entries.insert(key.clone(), before);
            return Err(error);
        }
        Ok(detail)
    }

    pub fn flush(&mut self, store: &Store) -> DomainResult<()> {
        self.persist(store, None)
    }

    fn persist(&mut self, store: &Store, audit: Option<(&str, &str, Value)>) -> DomainResult<()> {
        if self.parked {
            return Err(DomainError::Storage(
                "sensor stop recovery needs attention".into(),
            ));
        }
        let doc = Document {
            format: FORMAT,
            high_water: self.high_water,
            overflow: self.overflow,
            entries: self.entries.values().map(StoredStop::from).collect(),
        };
        let raw = serde_json::to_string(&doc)
            .map_err(|_| DomainError::Storage("sensor journal encoding failed".into()))?;
        let result = if raw.len() > MAX_BYTES {
            Err(DomainError::Storage(
                "sensor journal exceeds bounded storage".into(),
            ))
        } else {
            store.transaction(|txn| {
                txn.set_meta(SENSOR_JOURNAL_KEY, &raw)?;
                if let Some((kind, actor, detail)) = &audit {
                    txn.audit(kind, actor, detail)?;
                }
                Ok(())
            })
        };
        match &result {
            Ok(_) => {
                self.note = None;
            }
            Err(_) => {
                self.note = Some("write-failed");
                self.recovery_unknown = true;
            }
        }
        result
    }
}
