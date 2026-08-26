//! SSE event stream with `Last-Event-ID` resume against the bounded replay
//! buffer. Slow clients lag (broadcast semantics) instead of back-pressuring
//! the runtime.

use crate::ApiState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{Stream, StreamExt};
use interaction_core::RuntimeEvent;
use std::convert::Infallible;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

fn to_sse_event(event: &RuntimeEvent) -> Event {
    Event::default()
        .id(event.sequence.to_string())
        .event(event.event_type.as_str())
        .data(serde_json::to_string(event).unwrap_or_else(|_| "{}".into()))
}

pub async fn events(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let last_seen: u64 = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let replay = state.runtime.events.replay_after(last_seen);
    let replayed_until = replay.last().map(|e| e.sequence).unwrap_or(last_seen);
    let receiver = state.runtime.events.subscribe();

    let replay_stream = futures::stream::iter(replay.into_iter().map(|e| Ok(to_sse_event(&e))));
    let live_stream = BroadcastStream::new(receiver).filter_map(move |item| async move {
        match item {
            Ok(event) if event.sequence > replayed_until => Some(Ok(to_sse_event(&event))),
            Ok(_) => None,
            Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "sse client lagged; events skipped");
                None
            }
        }
    });

    Sse::new(replay_stream.chain(live_stream)).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}
