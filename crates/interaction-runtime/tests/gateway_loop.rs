//! Agent Gateway 垂直閉環（fake agent 子程序，真 spawn／真 pipe／真 kill）：
//! 建立→送任務→事件→聲稱完成（claim≠verified）→成本入預算→關閉殺程序；
//! 不可用 agent 誠實拒絕；estop 終止子程序樹。
//!
//! 全部情境放同一個 test fn：INTERACT_AI_CLAUDE_BIN 是程序級 env，
//! 平行測試執行緒共用，分開會互踩。

use interaction_core::*;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;

fn fixture_path() -> String {
    format!(
        "{}/tests/fixtures/fake_claude.sh",
        env!("CARGO_MANIFEST_DIR")
    )
}

async fn runtime() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    (dir, rt)
}

async fn wait_for<F>(mut f: F, what: &str)
where
    F: AsyncFnMut() -> bool,
{
    for _ in 0..100 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timeout waiting for {what}");
}

fn pid_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[tokio::test]
async fn gateway_full_loop_with_fake_agent() {
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());

    // ---- 1) 不可用 agent → 建立誠實失敗，不留半掛 session ----
    {
        std::env::set_var("INTERACT_AI_CODEX_BIN", "/nonexistent/codex-bin");
        let (_g, rt) = runtime().await;
        let err = rt
            .create_agent_session(interaction_runtime::agents::CreateAgentSession {
                provider_id: None,
                agent_id: "codex".into(),
                label: None,
                ttl_minutes: Some(10),
                data_scope: vec![],
                tool_scope: vec![],
                consent_scope: vec![],
                max_cost: None,
                max_messages: None,
                delegation: None,
                workdir: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Unavailable(_)), "{err:?}");
        assert_eq!(rt.open_agent_sessions().await, 0, "沒有殘留 session");
    }

    // ---- 2) 完整閉環：建立 → 任務 → 聲稱完成 → 成本 → 結果信箱 ----
    let (_g, rt) = runtime().await;
    let record = rt
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: None,
            agent_id: "claude-code".into(),
            label: Some("測試任務".into()),
            ttl_minutes: Some(10),
            data_scope: vec![],
            tool_scope: vec![],
            consent_scope: vec![],
            max_cost: Some(1.0),
            max_messages: Some(10),
            delegation: None,
            workdir: None,
        })
        .await
        .expect("fake agent attaches");
    let sid = record.session_id.as_str().to_string();

    // provider session id 由 init 事件回填。
    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .ok()
                .and_then(|r| r.provider_session_id)
                .as_deref()
                == Some("fake-123")
        },
        "provider session id fake-123",
    )
    .await;

    // 送任務 → gateway 即時轉送（delivered 戳記）→ fake 回聲稱完成。
    let msg = rt
        .mailbox_send(
            &sid,
            MailboxDirection::ToSession,
            "task",
            BTreeMap::from([("task".to_string(), json!("幫我看一下這個 repo"))]),
            None,
        )
        .await
        .unwrap();
    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .map(|r| r.state == AgentSessionState::ClaimedCompleted)
                .unwrap_or(false)
        },
        "claimed-completed",
    )
    .await;

    let after = rt.get_agent_session(&sid).await.unwrap();
    // 聲稱完成是 OPEN 狀態（不是 terminal）；成本已入預算。
    assert!(after.state.is_open());
    assert!((after.budget.spent_cost - 0.01).abs() < 1e-9);

    // ToSession 任務有 delivered 戳記（轉送即送達）。
    let to_session = rt
        .mailbox_peek(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    let delivered = to_session
        .iter()
        .find(|m| m.message_id == msg.message_id)
        .unwrap();
    assert!(delivered.delivered_at.is_some());

    // FromSession 有結果訊息（帶成本）。
    let results = rt
        .mailbox_peek(&sid, MailboxDirection::FromSession)
        .await
        .unwrap();
    let result_msg = results
        .iter()
        .find(|m| m.kind == "result")
        .expect("result in mailbox");
    assert_eq!(
        result_msg.body.get("summary").and_then(|v| v.as_str()),
        Some("完成了（這是聲稱）")
    );

    // 關閉 → 子程序被殺、record 關閉。
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    let closed = rt.get_agent_session(&sid).await.unwrap();
    assert_eq!(closed.state, AgentSessionState::Closed);

    // ---- 3) estop 終止子程序樹（hang 模式＋pid 檔驗證） ----
    let pid_file = tempfile::NamedTempFile::new().unwrap();
    std::env::set_var("FAKE_MODE", "hang");
    std::env::set_var("FAKE_PID_FILE", pid_file.path());
    let record = rt
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: None,
            agent_id: "claude-code".into(),
            label: Some("會掛住的任務".into()),
            ttl_minutes: Some(10),
            data_scope: vec![],
            tool_scope: vec![],
            consent_scope: vec![],
            max_cost: None,
            max_messages: None,
            delegation: None,
            workdir: None,
        })
        .await
        .unwrap();
    let sid2 = record.session_id.as_str().to_string();
    // 等 fixture 寫入自己的 pid。
    let mut pid: i32 = 0;
    wait_for(
        async || {
            pid = std::fs::read_to_string(pid_file.path())
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            pid > 0
        },
        "fixture pid file",
    )
    .await;
    assert!(pid_alive(pid), "fixture alive before estop");

    rt.emergency_stop("test", None).await.unwrap();
    let stopped = rt.get_agent_session(&sid2).await.unwrap();
    assert!(!stopped.state.is_open(), "estop closes the session");
    wait_for(async || !pid_alive(pid), "subprocess tree killed").await;

    std::env::remove_var("FAKE_MODE");
    std::env::remove_var("FAKE_PID_FILE");
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}
