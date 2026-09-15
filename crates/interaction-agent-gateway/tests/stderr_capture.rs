//! D16 回歸：agent 子程序的 stderr 以前被丟掉（codex app-server）或被留成
//! 未脫敏、無統計的 2000 字（claude／codex exec）。
//!
//! 這裡用一個真的會噴 ~5 MB stderr 的假 agent 子程序，證明：
//! 1. 流程不卡（reader 一路讀到 EOF，管線不塞）。
//! 2. `StderrCaptured` 在 `SessionClosed` **之前**送出，帶著誠實的統計。
//! 3. 記憶體與事件裡的 tail 都有界（truncated=true）且**已脫敏**。
//! 4. stderr 的內容不改變結局判定（exit 0＋agent 已聲稱 ⇒ 仍是聲稱完成）。

#![cfg(unix)]

use interaction_agent_gateway::claude::ClaudeConnector;
use interaction_agent_gateway::diagnostics::EVENT_TAIL_MAX_CHARS;
use interaction_agent_gateway::{AgentConnector, GatewayEvent, SessionSpec};
use std::time::Duration;

fn fixture() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fake_claude_stderr_flood.sh"
    )
    .to_string()
}

/// 收齊事件直到 `SessionClosed`；每一次等待都有界，卡住就是測試失敗。
async fn collect_until_closed(
    events: &mut tokio::sync::mpsc::Receiver<GatewayEvent>,
) -> Vec<GatewayEvent> {
    let mut seen = Vec::new();
    for _ in 0..64 {
        let event = tokio::time::timeout(Duration::from_secs(20), events.recv())
            .await
            .expect("子程序噴 5 MB stderr 不得讓事件流卡住")
            .expect("channel closed before SessionClosed");
        let closed = matches!(event, GatewayEvent::SessionClosed { .. });
        seen.push(event);
        if closed {
            return seen;
        }
    }
    panic!("SessionClosed never arrived: {seen:?}");
}

#[tokio::test]
async fn a_flooding_subprocess_yields_a_bounded_redacted_stderr_capture() {
    let dir = tempfile::tempdir().expect("tempdir");
    let connector = ClaudeConnector::with_binary(fixture());
    let mut handle = connector
        .start_session(SessionSpec::read_only_in(dir.path().to_path_buf()))
        .await
        .expect("session starts");
    let mut events = handle.take_events().expect("events");
    let seen = collect_until_closed(&mut events).await;

    let captured_at = seen
        .iter()
        .position(|e| matches!(e, GatewayEvent::StderrCaptured { .. }))
        .unwrap_or_else(|| panic!("no StderrCaptured event: {seen:?}"));
    let closed_at = seen
        .iter()
        .position(|e| matches!(e, GatewayEvent::SessionClosed { .. }))
        .expect("SessionClosed");
    assert!(
        captured_at < closed_at,
        "StderrCaptured must precede SessionClosed: {seen:?}"
    );

    let GatewayEvent::StderrCaptured {
        tail,
        lines_seen,
        bytes_seen,
        lines_dropped,
        truncated,
    } = &seen[captured_at]
    else {
        unreachable!()
    };

    // 統計誠實：5000 行雜訊＋2 行 secret，丟棄量數得出來。
    assert!(*lines_seen >= 5_000, "lines_seen={lines_seen}");
    assert!(*bytes_seen >= 5_000_000, "bytes_seen={bytes_seen}");
    assert!(*lines_dropped > 0, "lines_dropped={lines_dropped}");
    assert!(*truncated, "a 5 MB flood must be disclosed as truncated");
    assert!(
        lines_dropped < lines_seen,
        "dropped everything? {lines_dropped}/{lines_seen}"
    );

    // 有界：事件裡的 tail 不會把 5 MB 帶出去。
    assert!(
        tail.chars().count() <= EVENT_TAIL_MAX_CHARS,
        "tail is {} chars",
        tail.chars().count()
    );

    // 脫敏：原文 secret 不得出現，脫敏後的形狀要在。
    assert!(
        !tail.contains("abc123def456"),
        "leaked bearer token: {tail}"
    );
    assert!(!tail.contains("/Users/someone"), "leaked home path: {tail}");
    assert!(!tail.contains("someone"), "leaked local user name: {tail}");
    assert!(tail.contains("Bearer [redacted]"), "{tail}");
    assert!(tail.contains("~/secret"), "{tail}");

    // 誠實階梯：stderr 再吵也不改結局。agent 自己聲稱完成、exit 0 ⇒
    // 仍然只有一個 claimed-completed，沒有被 stderr 翻成 failed。
    assert!(
        seen.iter()
            .any(|e| matches!(e, GatewayEvent::TaskClaimedCompleted { .. })),
        "{seen:?}"
    );
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, GatewayEvent::TaskFailed { .. })),
        "noisy stderr must never be turned into a business failure: {seen:?}"
    );

    // SessionClosed 的 detail 引用 stderr 時一樣是脫敏後的。
    let GatewayEvent::SessionClosed { detail, .. } = &seen[closed_at] else {
        unreachable!()
    };
    let detail = detail.as_deref().unwrap_or_default();
    assert!(detail.contains("exit 0"), "{detail}");
    assert!(!detail.contains("abc123def456"), "{detail}");
    assert!(!detail.contains("/Users/someone"), "{detail}");

    handle.kill().await.expect("kill");
}
