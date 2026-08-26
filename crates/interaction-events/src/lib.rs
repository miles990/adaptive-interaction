//! Bounded event bus: tokio broadcast for live subscribers plus a bounded
//! in-memory ring buffer so SSE clients can resume from `Last-Event-ID`.
//! Slow subscribers lag (broadcast semantics) instead of blocking the runtime.

use interaction_core::{EventType, RuntimeEvent};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// Default capacity for both the live channel and the replay buffer.
pub const DEFAULT_CAPACITY: usize = 1024;

#[derive(Clone)]
pub struct EventBus {
    inner: Arc<EventBusInner>,
}

struct EventBusInner {
    sender: broadcast::Sender<RuntimeEvent>,
    ring: Mutex<VecDeque<RuntimeEvent>>,
    capacity: usize,
    sequence: AtomicU64,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(16));
        Self {
            inner: Arc::new(EventBusInner {
                sender,
                ring: Mutex::new(VecDeque::with_capacity(capacity)),
                capacity,
                sequence: AtomicU64::new(1),
            }),
        }
    }

    /// Publish an event; assigns the sequence number and never blocks.
    pub fn publish(&self, mut event: RuntimeEvent) -> RuntimeEvent {
        event.sequence = self.inner.sequence.fetch_add(1, Ordering::SeqCst);
        {
            let mut ring = self.inner.ring.lock().expect("event ring poisoned");
            if ring.len() >= self.inner.capacity {
                ring.pop_front();
            }
            ring.push_back(event.clone());
        }
        // Errors just mean "no live subscribers" — replay buffer still has it.
        let _ = self.inner.sender.send(event.clone());
        tracing::debug!(
            event_type = event.event_type.as_str(),
            seq = event.sequence,
            "event published"
        );
        event
    }

    /// Subscribe to live events.
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.inner.sender.subscribe()
    }

    /// Events with sequence strictly greater than `after` still held in the buffer.
    pub fn replay_after(&self, after: u64) -> Vec<RuntimeEvent> {
        let ring = self.inner.ring.lock().expect("event ring poisoned");
        ring.iter()
            .filter(|e| e.sequence > after)
            .cloned()
            .collect()
    }

    /// Most recent events (newest last), up to `limit`.
    pub fn recent(&self, limit: usize) -> Vec<RuntimeEvent> {
        let ring = self.inner.ring.lock().expect("event ring poisoned");
        let skip = ring.len().saturating_sub(limit);
        ring.iter().skip(skip).cloned().collect()
    }

    pub fn last_sequence(&self) -> u64 {
        self.inner.sequence.load(Ordering::SeqCst).saturating_sub(1)
    }

    /// Convenience: build + publish in one call.
    pub fn emit(&self, event_type: EventType, payload: serde_json::Value) -> RuntimeEvent {
        self.publish(RuntimeEvent::new(event_type, chrono::Utc::now(), payload))
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn publish_and_subscribe() {
        let bus = EventBus::new(8);
        let mut rx = bus.subscribe();
        bus.emit(EventType::SessionStarted, json!({"x": 1}));
        let evt = rx.recv().await.unwrap();
        assert_eq!(evt.event_type, EventType::SessionStarted);
        assert_eq!(evt.sequence, 1);
    }

    #[test]
    fn ring_buffer_is_bounded_and_replays() {
        let bus = EventBus::new(4);
        for i in 0..10 {
            bus.emit(EventType::PlanCreated, json!({ "i": i }));
        }
        // Only the last 4 are retained.
        let replay = bus.replay_after(0);
        assert_eq!(replay.len(), 4);
        assert_eq!(replay.first().unwrap().sequence, 7);
        let after8 = bus.replay_after(8);
        assert_eq!(after8.len(), 2);
        assert_eq!(bus.last_sequence(), 10);
    }
}
