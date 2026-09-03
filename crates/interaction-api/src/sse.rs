//! SSE event stream with `Last-Event-ID` resume against the bounded replay
//! buffer. Slow clients lag (broadcast semantics) instead of back-pressuring
//! the runtime.

use crate::{ApiState, AuthContext, AuthPrincipal};
use axum::extract::{Extension, State};
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

fn event_allowed(auth: &AuthContext, event: &RuntimeEvent) -> bool {
    if matches!(&auth.principal, AuthPrincipal::Human) {
        return true;
    }
    // The legacy Agent token has no knowledge/memory/presentation/sensor
    // authority. SSE must obey the same boundary as REST instead of becoming
    // a side channel for payloads rejected by those route families.
    // Character Protocol 事件（intent／receipt／instance／system-text）只給
    // 可信 host（human）；外部 adapter 走自己的 WebSocket，不開 SSE。
    !matches!(
        event.event_type,
        interaction_core::EventType::KnowledgeUpdated
            | interaction_core::EventType::ConsentChanged
            | interaction_core::EventType::PolicyChanged
            | interaction_core::EventType::SensorStarted
            | interaction_core::EventType::SensorStopped
            | interaction_core::EventType::PresentationCommand
            | interaction_core::EventType::PresentationState
            | interaction_core::EventType::AiAssistRequested
            | interaction_core::EventType::AiAssistResolved
            | interaction_core::EventType::CharacterIntent
            | interaction_core::EventType::CharacterReceipt
            | interaction_core::EventType::CharacterInstance
            | interaction_core::EventType::CharacterSystemText
    )
}

pub async fn events(
    State(state): State<ApiState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let last_seen: u64 = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    // Subscribe before taking the replay snapshot. Any event published in the
    // boundary window is then present in either replay or live; sequence
    // filtering removes duplicates.
    let receiver = state.runtime.events.subscribe();
    let replay = state
        .runtime
        .events
        .replay_after(last_seen)
        .into_iter()
        .filter(|event| event_allowed(&auth, event))
        .collect::<Vec<_>>();
    let replayed_until = replay.last().map(|e| e.sequence).unwrap_or(last_seen);

    let replay_stream = futures::stream::iter(replay.into_iter().map(|e| Ok(to_sse_event(&e))));
    let live_auth = auth.clone();
    let live_stream = BroadcastStream::new(receiver).filter_map(move |item| {
        let auth = live_auth.clone();
        async move {
            match item {
                Ok(event) if event.sequence > replayed_until && event_allowed(&auth, &event) => {
                    Some(Ok(to_sse_event(&event)))
                }
                Ok(_) => None,
                Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "sse client lagged; events skipped");
                    None
                }
            }
        }
    });

    Sse::new(replay_stream.chain(live_stream)).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use interaction_core::EventType;
    use serde_json::json;

    #[test]
    fn legacy_agent_sse_cannot_read_knowledge_or_sensor_payloads() {
        let auth = AuthContext {
            principal: AuthPrincipal::LegacyAgent,
        };
        for event_type in [EventType::KnowledgeUpdated, EventType::SensorStarted] {
            let event = RuntimeEvent::new(event_type, Utc::now(), json!({"secret": "no"}));
            assert!(!event_allowed(&auth, &event));
        }
        let action = RuntimeEvent::new(EventType::ActionFailed, Utc::now(), json!({}));
        assert!(event_allowed(&auth, &action));
    }

    #[test]
    fn character_events_are_human_only_on_sse() {
        let human = AuthContext {
            principal: AuthPrincipal::Human,
        };
        let adapter = AuthContext {
            principal: AuthPrincipal::CharacterAdapter {
                adapter_id: "adp-1".into(),
            },
        };
        let agent = AuthContext {
            principal: AuthPrincipal::LegacyAgent,
        };
        for event_type in [
            EventType::CharacterIntent,
            EventType::CharacterReceipt,
            EventType::CharacterInstance,
            EventType::CharacterSystemText,
        ] {
            let event = RuntimeEvent::new(event_type, Utc::now(), json!({}));
            assert!(event_allowed(&human, &event));
            assert!(
                !event_allowed(&agent, &event),
                "{event_type:?} hidden from agent"
            );
            assert!(
                !event_allowed(&adapter, &event),
                "{event_type:?} hidden from adapter"
            );
        }
    }
}
