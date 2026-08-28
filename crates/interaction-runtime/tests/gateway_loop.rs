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

fn claude_input(
    label: &str,
    max_cost: Option<f64>,
) -> interaction_runtime::agents::CreateAgentSession {
    interaction_runtime::agents::CreateAgentSession {
        provider_id: None,
        agent_id: "claude-code".into(),
        label: Some(label.into()),
        ttl_minutes: Some(10),
        data_scope: vec![],
        tool_scope: vec![],
        consent_scope: vec![],
        max_cost,
        max_messages: Some(10),
        delegation: None,
        workdir: None,
        resume_provider_session_id: None,
        allow_write: false,
    }
}

fn read_pid(path: &std::path::Path) -> i32 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
async fn gateway_full_loop_with_fake_agent() {
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());

    // ---- 0) 使用者停用的 connector 是 Runtime 規則，不是 UI 假開關。 ----
    {
        let (_g, rt) = runtime().await;
        rt.update_ui_preferences(serde_json::json!({"disabledAgents": ["claude-code"]}))
            .await
            .unwrap();
        let err = rt
            .create_agent_session(claude_input("已停用", None))
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    }

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
                resume_provider_session_id: None,
                allow_write: false,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Unavailable(_)), "{err:?}");
        assert_eq!(rt.open_agent_sessions().await, 0, "沒有殘留 session");
    }

    // ---- 1b) 寫入 session 必須是 gateway agent＋明確工作目錄＋雙重 scope；
    //          建立後 access mode 寫進 record，不能只存在 UI request。 ----
    {
        let (home, rt) = runtime().await;
        let mut missing = claude_input("缺少授權", None);
        missing.allow_write = true;
        missing.workdir = Some(home.path().to_string_lossy().into_owned());
        let err = rt.create_agent_session(missing).await.unwrap_err();
        assert!(matches!(err, DomainError::ConsentRequired(_)), "{err:?}");

        let mut writable = claude_input("限權寫入", None);
        writable.allow_write = true;
        writable.workdir = Some(home.path().to_string_lossy().into_owned());
        writable.tool_scope = vec!["workspace.write".into()];
        writable.consent_scope = vec!["agent-session:workspace-write".into()];
        let record = rt.create_agent_session(writable).await.unwrap();
        assert!(record.allow_write);
        assert_eq!(record.tool_scope, ["workspace.write"]);
        assert_eq!(record.consent_scope, ["agent-session:workspace-write"]);
        rt.close_agent_session(record.session_id.as_str(), None, "closed")
            .await
            .unwrap();
    }

    // ---- 2) 完整閉環：建立 → 任務 → 聲稱完成 → 成本 → 結果信箱 ----
    let input_file = tempfile::NamedTempFile::new().unwrap();
    let env_status_file = tempfile::NamedTempFile::new().unwrap();
    std::env::set_var("FAKE_INPUT_FILE", input_file.path());
    std::env::set_var("FAKE_ENV_STATUS_FILE", env_status_file.path());
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
            resume_provider_session_id: None,
            allow_write: false,
        })
        .await
        .expect("fake agent attaches");
    assert_eq!(
        record.provider_id.as_str(),
        "provider.ai-agent.claude-code",
        "gateway sessions must inherit the concrete connector provider when the caller omits providerId"
    );
    let sid = record.session_id.as_str().to_string();
    wait_for(
        async || {
            std::fs::read_to_string(env_status_file.path())
                .map(|body| body.trim() == "scoped-session-capability-present")
                .unwrap_or(false)
        },
        "scoped session capability in provider environment",
    )
    .await;

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
            std::fs::read_to_string(input_file.path())
                .map(|body| body.contains("contextBundle") && body.contains("agentId"))
                .unwrap_or(false)
        },
        "actual context bundle delivered to provider stdin",
    )
    .await;
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
    std::env::remove_var("FAKE_INPUT_FILE");
    std::env::remove_var("FAKE_ENV_STATUS_FILE");

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
            resume_provider_session_id: None,
            allow_write: false,
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

    // ---- 4) GET/fetch 對 gateway session 是純觀看：不得替子程序蓋
    //         delivered／推進 receipt（觀看≠送達） ----
    {
        let pid_file4 = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("FAKE_PID_FILE", pid_file4.path());
        let (_g4, rt4) = runtime().await;
        let record = rt4
            .create_agent_session(claude_input("觀看不等於送達", Some(0.005)))
            .await
            .unwrap();
        let sid4 = record.session_id.as_str().to_string();
        let mut pid4 = 0;
        wait_for(
            async || {
                pid4 = read_pid(pid_file4.path());
                pid4 > 0
            },
            "fixture pid (observer fetch)",
        )
        .await;
        // 第一個任務：真實轉送成功 → delivered 由 gateway_deliver 蓋。
        let msg1 = rt4
            .mailbox_send(
                &sid4,
                MailboxDirection::ToSession,
                "task",
                BTreeMap::from([("task".to_string(), json!("第一個任務"))]),
                None,
            )
            .await
            .unwrap();
        wait_for(
            async || {
                rt4.get_agent_session(&sid4)
                    .await
                    .map(|r| r.state == AgentSessionState::ClaimedCompleted)
                    .unwrap_or(false)
            },
            "cost booked via claimed-completed",
        )
        .await;
        // 第二個任務：成本預算已爆（0.01 ≥ 0.005），gateway 誠實拒絕轉送
        // → 訊息留在信箱、沒有 delivered 戳記。
        let msg2 = rt4
            .mailbox_send(
                &sid4,
                MailboxDirection::ToSession,
                "task",
                BTreeMap::from([("task".to_string(), json!("第二個任務"))]),
                None,
            )
            .await
            .unwrap();
        // 觀察者讀信箱（GET messages 的底層路徑）：不得蓋章。
        let seen = rt4
            .mailbox_fetch(&sid4, MailboxDirection::ToSession)
            .await
            .unwrap();
        let m1 = seen
            .iter()
            .find(|m| m.message_id == msg1.message_id)
            .unwrap();
        assert!(m1.delivered_at.is_some(), "真實轉送過的任務保有 delivered");
        let m2 = seen
            .iter()
            .find(|m| m.message_id == msg2.message_id)
            .unwrap();
        assert!(
            m2.delivered_at.is_none(),
            "觀看不等於送達：fetch 不得替 gateway session 蓋 delivered"
        );
        // 持久狀態再驗一次（peek 不改變任何東西）。
        let peeked = rt4
            .mailbox_peek(&sid4, MailboxDirection::ToSession)
            .await
            .unwrap();
        assert!(peeked
            .iter()
            .find(|m| m.message_id == msg2.message_id)
            .unwrap()
            .delivered_at
            .is_none());
        // 收尾：Failed 是任務結局不是 session 終局，close 負責收尾並殺掉
        // 子程序樹（測試沒開 watchdog，事件泵又持有 runtime clone，單靠
        // drop 不會清理；殘影會干擾後續 section 的 pgid attribution）。
        rt4.close_agent_session(&sid4, None, "closed")
            .await
            .unwrap();
        wait_for(async || !pid_alive(pid4), "observer-fetch fixture killed").await;
        std::env::remove_var("FAKE_PID_FILE");
    }

    // ---- 5) estop×create 確定性屏障：緊急停止絕不留下 open session
    //         或存活的子程序（TOCTOU regression） ----
    {
        let pid_file5 = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("FAKE_MODE", "hang");
        std::env::set_var("FAKE_PID_FILE", pid_file5.path());
        let (_g5, rt5) = runtime().await;
        let creator = {
            let rt = rt5.clone();
            tokio::spawn(async move {
                rt.create_agent_session(claude_input("estop 競態", None))
                    .await
            })
        };
        // 讓 create 進行到 attach 途中（discover＋spawn 需要數十 ms）。
        tokio::time::sleep(Duration::from_millis(10)).await;
        rt5.emergency_stop("test", None).await.unwrap();
        let created = creator.await.unwrap();
        // 不論交錯順序：create 要嘛被拒絕，要嘛其 session 已被 estop 收掉。
        if let Ok(rec) = created {
            let now = rt5
                .get_agent_session(rec.session_id.as_str())
                .await
                .unwrap();
            assert!(!now.state.is_open(), "estop 後不得留下 open session");
        }
        assert_eq!(rt5.open_agent_sessions().await, 0);
        // 子程序若已 spawn（pid 檔已寫）必須死透——順帶避免殘影干擾
        // 後續 section 的 pgid attribution。
        let pid5 = read_pid(pid_file5.path());
        if pid5 > 0 {
            wait_for(
                async || !pid_alive(pid5),
                "estop-race subprocess tree killed",
            )
            .await;
        }
        std::env::remove_var("FAKE_MODE");
        std::env::remove_var("FAKE_PID_FILE");
    }

    // ---- 6a) 正常 shutdown：inline 依 pgid 記錄整樹終結，不留孤兒 ----
    {
        let pid_file6 = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("FAKE_MODE", "hang");
        std::env::set_var("FAKE_PID_FILE", pid_file6.path());
        let (_g6, rt6) = runtime().await;
        rt6.create_agent_session(claude_input("關機孤兒", None))
            .await
            .unwrap();
        let mut pid6 = 0;
        wait_for(
            async || {
                pid6 = read_pid(pid_file6.path());
                pid6 > 0
            },
            "fixture pid (shutdown)",
        )
        .await;
        assert!(pid_alive(pid6), "fixture alive before shutdown");
        rt6.shutdown().await;
        wait_for(async || !pid_alive(pid6), "shutdown kills subprocess tree").await;
        std::env::remove_var("FAKE_MODE");
        std::env::remove_var("FAKE_PID_FILE");
    }

    // ---- 6b) 崩潰（不走 shutdown）：重啟時 restore 依 pgid 記錄 reap
    //          孤兒子程序樹，record 標 Expired 並註明 reaped ----
    {
        let home = tempfile::tempdir().unwrap();
        let pid_file7 = tempfile::NamedTempFile::new().unwrap();
        std::env::set_var("FAKE_MODE", "hang");
        std::env::set_var("FAKE_PID_FILE", pid_file7.path());
        let rt7 = Runtime::start(RuntimeOptions {
            home: Some(home.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        let sid7 = rt7
            .create_agent_session(claude_input("崩潰孤兒", None))
            .await
            .unwrap()
            .session_id
            .as_str()
            .to_string();
        let mut pid7 = 0;
        wait_for(
            async || {
                pid7 = read_pid(pid_file7.path());
                pid7 > 0
            },
            "fixture pid (crash)",
        )
        .await;
        assert!(pid_alive(pid7), "fixture alive before simulated crash");
        // 模擬崩潰：不 shutdown、不 drop（drop 會觸發 kill_on_drop，
        // 真正的崩潰不會有這種好事）。
        std::mem::forget(rt7);
        let rt8 = Runtime::start(RuntimeOptions {
            home: Some(home.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        wait_for(async || !pid_alive(pid7), "restart reaps orphan subprocess").await;
        let rec = rt8.get_agent_session(&sid7).await.unwrap();
        // 注意：這裡用 mem::forget 模擬崩潰，被遺忘的 runtime 事件泵其實
        // 還活著，可能在 restore 讀 record 前搶先把它標成 Failed（真實
        // 崩潰不會有這回事）。不變量是：session 非 open、孤兒已被 reap。
        assert!(
            !rec.state.is_open(),
            "restored session must not be open: {:?}",
            rec.state
        );
        if rec.state == AgentSessionState::Expired {
            assert!(
                rec.detail.as_deref().unwrap_or("").contains("reaped"),
                "detail 應註明孤兒已被 reap：{:?}",
                rec.detail
            );
        }
        std::env::remove_var("FAKE_MODE");
        std::env::remove_var("FAKE_PID_FILE");
    }

    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}
