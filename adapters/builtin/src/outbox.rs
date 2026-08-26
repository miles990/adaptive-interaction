//! Shared bounded outbox: where conversation / web-ui messages land so the
//! CLI, HTTP API and desktop UI can render them.

use interaction_core::{ActionId, Timestamp};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const OUTBOX_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxMessage {
    pub channel: String,
    pub intent: String,
    /// `None` = deliberate silence (still recorded for the timeline).
    pub text: Option<String>,
    pub action_id: ActionId,
    pub at: Timestamp,
}

#[derive(Clone, Default)]
pub struct Outbox {
    inner: Arc<Mutex<VecDeque<OutboxMessage>>>,
}

impl Outbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, message: OutboxMessage) {
        let mut q = self.inner.lock().expect("outbox lock");
        if q.len() >= OUTBOX_CAPACITY {
            q.pop_front();
        }
        q.push_back(message);
    }

    pub fn recent(&self, limit: usize) -> Vec<OutboxMessage> {
        let q = self.inner.lock().expect("outbox lock");
        let skip = q.len().saturating_sub(limit);
        q.iter().skip(skip).cloned().collect()
    }
}
