//! `GET /v1/character/ws?token=<adapter token>`：外部 Character Adapter 的 WebSocket
//! transport（README §8／§8.1）。
//!
//! - 只收 adapter token（query 參數；不是 Bearer）：human／agent token 一律 401，
//!   未知／已撤銷 token 401。
//! - 每連線：outbound 有界（Runtime 端 mpsc 32）、heartbeat 每 15 s、45 s 無任何
//!   inbound 視為斷線（pending → uncertain、generation+1）、rate limit 50/s 與
//!   方向檢查由 gateway 做、單則 ≤ 64 KB 由 `parse_wire` 強制（超過回 `error{too-large}`）。
//! - 撤銷／被新連線取代／runtime 關機 → close token 取消 → socket 關閉。
//! - 第一則 runtime → adapter 訊息一定是 `hello`。

use crate::{ApiError, ApiState};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use interaction_character::{encode_wire, DisconnectReason, Limits, WireMessage};
use interaction_runtime::character::{WsSession, WsStep, HEARTBEAT_INTERVAL_MS, IDLE_TIMEOUT_MS};
use interaction_runtime::Runtime;
use std::time::Duration;

/// socket 層的 frame 上限放寬到協定上限的兩倍：超過 64 KB 的訊息仍會被收下，
/// 再由 `parse_wire` 以協定錯誤 `too-large` 誠實回覆（而不是無聲斷線）。
const WS_MAX_FRAME_BYTES: usize = Limits::MAX_MESSAGE_BYTES * 2;
/// 關閉前把佇列裡剩下的 error／goodbye 送完（有界等待）。
const FLUSH_WAIT_MS: u64 = 50;
/// 單次 WebSocket 寫入的逾時（對抗審查 character-protocol-039）。
///
/// `tokio::select!` 選中 outbound 分支之後，分支主體裡的 `.await` 不再回到 select——
/// 對端若停止讀取（TCP 反壓），沒有逾時的 `sink.send()` 會把整個迴圈卡死：
/// `session.close.cancelled()`（撤銷／被取代／關機）與 idle deadline 都不再被輪詢，
/// runtime 端的 sweep 就算 `close.cancel()` 也叫不醒它。逾時＝這條連線沒在讀，
/// 當成 transport 關閉跳出迴圈，讓 pending 誠實落成 `uncertain`＋安全 intent 補 `system.text`。
const WRITE_TIMEOUT_MS: u64 = 5_000;

#[derive(serde::Deserialize, Default)]
pub struct WsQuery {
    #[serde(default)]
    pub token: Option<String>,
}

pub async fn character_ws(
    State(state): State<ApiState>,
    Query(query): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(token) = query.token.filter(|t| !t.trim().is_empty()) else {
        return ApiError::adapter_token_required().into_response();
    };
    if crate::constant_time_eq(token.as_bytes(), state.token.as_bytes())
        || crate::constant_time_eq(token.as_bytes(), state.agent_token.as_bytes())
    {
        // README §9：WebSocket 不接受 human／agent token。
        return ApiError::adapter_token_required().into_response();
    }
    let Some(adapter_id) = state.runtime.character_adapter_for_token(&token) else {
        return ApiError::unauthorized().into_response();
    };
    let session = match state.runtime.character_ws_attach(&adapter_id).await {
        Ok(session) => session,
        Err(err) => return ApiError::from(err).into_response(),
    };
    let runtime = state.runtime.clone();
    ws.max_message_size(WS_MAX_FRAME_BYTES)
        .max_frame_size(WS_MAX_FRAME_BYTES)
        .on_upgrade(move |socket| character_ws_loop(runtime, session, socket))
}

async fn send_wire<S>(sink: &mut S, message: &WireMessage) -> Result<(), ()>
where
    S: futures::Sink<Message> + Unpin,
{
    let bytes = encode_wire(message).map_err(|_| ())?;
    let text = String::from_utf8(bytes).map_err(|_| ())?;
    match tokio::time::timeout(
        Duration::from_millis(WRITE_TIMEOUT_MS),
        sink.send(Message::Text(text.into())),
    )
    .await
    {
        Ok(result) => result.map_err(|_| ()),
        Err(_) => {
            tracing::warn!(
                kind = message.kind(),
                "character ws write timed out; treating the peer as gone"
            );
            Err(())
        }
    }
}

async fn character_ws_loop(runtime: Runtime, mut session: WsSession, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut heartbeat = tokio::time::interval(Duration::from_millis(HEARTBEAT_INTERVAL_MS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // 第一個 tick 立即觸發：跳過（hello 已經在佇列最前面）。
    heartbeat.tick().await;
    let idle = Duration::from_millis(IDLE_TIMEOUT_MS);
    let mut last_inbound = tokio::time::Instant::now();
    let mut reason = DisconnectReason::TransportClosed;
    loop {
        let idle_deadline = tokio::time::sleep_until(last_inbound + idle);
        tokio::select! {
            _ = session.close.cancelled() => {
                reason = DisconnectReason::Revoked;
                break;
            }
            _ = idle_deadline => {
                reason = DisconnectReason::HeartbeatTimeout;
                break;
            }
            outbound = session.rx.recv() => {
                match outbound {
                    Some(message) => {
                        let goodbye = matches!(message, WireMessage::Goodbye { .. });
                        if send_wire(&mut sink, &message).await.is_err() {
                            break;
                        }
                        if goodbye {
                            reason = DisconnectReason::Goodbye;
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = heartbeat.tick() => {
                let generation = runtime.character_generation(&session.instance_id);
                if send_wire(&mut sink, &WireMessage::Heartbeat { generation }).await.is_err() {
                    break;
                }
            }
            inbound = stream.next() => {
                last_inbound = tokio::time::Instant::now();
                let step = match inbound {
                    None => break,
                    Some(Err(_)) => break,
                    Some(Ok(Message::Text(text))) => {
                        runtime
                            .character_ws_message(&session.instance_id, session.conn_id, text.as_bytes())
                            .await
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        runtime
                            .character_ws_message(&session.instance_id, session.conn_id, &bytes)
                            .await
                    }
                    Some(Ok(Message::Close(_))) => break,
                    // ping／pong 由 axum 自動回應。
                    Some(Ok(_)) => WsStep::KeepOpen,
                };
                if step == WsStep::Close {
                    reason = DisconnectReason::Goodbye;
                    break;
                }
            }
        }
    }
    // 有界 flush：把 gateway 已排好的 error／goodbye 送出去再關。
    while let Ok(Some(message)) =
        tokio::time::timeout(Duration::from_millis(FLUSH_WAIT_MS), session.rx.recv()).await
    {
        if send_wire(&mut sink, &message).await.is_err() {
            break;
        }
    }
    runtime
        .character_ws_closed(&session.instance_id, session.conn_id, reason)
        .await;
    let _ = sink.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::Sink;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// 對端停止讀取（TCP 反壓）後的 sink：永遠 Pending，寫入不會完成。
    struct StalledSink;

    impl Sink<Message> for StalledSink {
        type Error = axum::Error;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn start_send(self: Pin<&mut Self>, _item: Message) -> Result<(), Self::Error> {
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
    }

    /// character-protocol-039：對端不讀時，寫入必須逾時放棄——否則 `tokio::select!` 的
    /// outbound 分支會一直卡在 `.await`，`session.close.cancelled()` 與 idle deadline
    /// 永遠不再被輪詢，撤銷／被取代／關機都關不掉這條連線。
    #[tokio::test]
    async fn a_peer_that_stops_reading_cannot_wedge_the_ws_loop_forever() {
        let mut sink = StalledSink;
        let outcome = tokio::time::timeout(
            Duration::from_millis(WRITE_TIMEOUT_MS + 2_000),
            send_wire(
                &mut sink,
                &WireMessage::Heartbeat {
                    generation: Some(1),
                },
            ),
        )
        .await;
        assert!(
            matches!(outcome, Ok(Err(()))),
            "WebSocket 寫入必須有逾時，逾時即視為 transport 關閉（讓 close token 叫得醒迴圈）"
        );
    }
}
