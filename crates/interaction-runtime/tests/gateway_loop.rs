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

/// `INTERACT_AI_*_BIN` / `FAKE_*` 是**程序級** env：同一個測試 binary 裡的
/// 執行緒共用它們，兩個測試同時改就會互相汙染（一個測試的 hang 模式會讓
/// 另一個測試的 fixture 掛住）。所有會碰 env 的 gateway 測試都先取這把鎖，
/// 彼此序列化。
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn fixture_path() -> String {
    format!(
        "{}/tests/fixtures/fake_claude.sh",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn codex_fixture_path() -> String {
    format!(
        "{}/tests/fixtures/fake_codex.sh",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// 情境選擇放在 workdir 裡的 `fake-mode` 檔（fixture 優先讀它）：這樣一個
/// 測試驅動的永遠是**自己那個**子程序，不受其他測試的 env 影響。
fn scenario_workdir(mode: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("fake-mode"), mode).unwrap();
    dir
}

/// 一個 session 依序發出的 `agent.session.state` taxonomy（小樞演出的來源）。
fn session_states(rt: &Runtime, session_id: &str) -> Vec<String> {
    rt.events
        .recent(2000)
        .into_iter()
        .filter(|e| e.event_type == EventType::AgentSessionState)
        .filter(|e| e.payload.get("agentSessionId").and_then(|v| v.as_str()) == Some(session_id))
        .filter_map(|e| {
            e.payload
                .get("state")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .collect()
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

fn codex_input(
    label: &str,
    workdir: &std::path::Path,
) -> interaction_runtime::agents::CreateAgentSession {
    interaction_runtime::agents::CreateAgentSession {
        provider_id: None,
        agent_id: "codex".into(),
        label: Some(label.into()),
        ttl_minutes: Some(10),
        data_scope: vec![],
        tool_scope: vec![],
        consent_scope: vec![],
        max_cost: None,
        max_messages: Some(10),
        delegation: None,
        workdir: Some(workdir.to_string_lossy().into_owned()),
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
    let _env = ENV_LOCK.lock().await;
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
        // → 呼叫端拿到明確的「未送達」錯誤（不是靜默 Ok），訊息留在信箱、
        // 沒有 delivered 戳記；而且「沒送到」≠「任務失敗」：上一輪的聲稱
        // 仍然成立，session 不得被改寫成 Failed。
        let err = rt4
            .mailbox_send(
                &sid4,
                MailboxDirection::ToSession,
                "task",
                BTreeMap::from([("task".to_string(), json!("第二個任務"))]),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
        assert!(err.to_string().contains("未送達"), "{err}");
        assert_eq!(
            rt4.get_agent_session(&sid4).await.unwrap().state,
            AgentSessionState::ClaimedCompleted,
            "an undelivered message is not evidence that the agent failed"
        );
        let msg2 = rt4
            .mailbox_peek(&sid4, MailboxDirection::ToSession)
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.body.get("task") == Some(&json!("第二個任務")))
            .expect("the undelivered message stays queued in the mailbox");
        assert!(msg2.delivered_at.is_none());
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
        // 收尾：close 負責收尾並殺掉子程序樹（測試沒開 watchdog，事件泵又
        // 持有 runtime clone，單靠 drop 不會清理；殘影會干擾後續 section 的
        // pgid attribution）。
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

/// regression（誠實階梯）：子程序結束而沒有回報任何結果，曾被記成
/// `failed`。沒觀察到成功不能說成功，沒觀察到錯誤也不能說失敗——
/// 那是 **unknown**。只有 connector 真的看得到的錯誤（非零 exit）才是失敗。
#[tokio::test]
async fn process_that_ends_without_a_claim_is_unknown_and_only_a_real_error_is_failed() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;

    // (a) exit 0，一個結果都沒回報 ⇒ unknown。
    let quiet = scenario_workdir("silent");
    let mut input = claude_input("安靜退出", None);
    input.workdir = Some(quiet.path().to_string_lossy().into_owned());
    let sid = rt
        .create_agent_session(input)
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();
    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .map(|r| r.state == AgentSessionState::Unknown)
                .unwrap_or(false)
        },
        "unknown state after a silent exit",
    )
    .await;
    let record = rt.get_agent_session(&sid).await.unwrap();
    assert!(!record.state.is_open(), "unknown is terminal");
    let states = session_states(&rt, &sid);
    assert!(
        states.iter().any(|s| s == "unknown"),
        "taxonomy must say unknown: {states:?}"
    );
    assert!(
        !states.iter().any(|s| s == "failed"),
        "a silent exit is not evidence of failure: {states:?}"
    );
    assert!(
        !states.iter().any(|s| s == "claimed-completed"),
        "and certainly not evidence of success: {states:?}"
    );

    // (b) 非零 exit＋stderr：這是**觀察得到**的錯誤 ⇒ failed，且 detail
    //     保留 exit code 與 stderr，讓人類看得到憑據。
    let boom = scenario_workdir("crash");
    let mut input = claude_input("崩潰退出", None);
    input.workdir = Some(boom.path().to_string_lossy().into_owned());
    let sid2 = rt
        .create_agent_session(input)
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();
    wait_for(
        async || {
            rt.get_agent_session(&sid2)
                .await
                .map(|r| r.state == AgentSessionState::Failed)
                .unwrap_or(false)
        },
        "failed state after a non-zero exit",
    )
    .await;
    let states2 = session_states(&rt, &sid2);
    assert!(
        states2.iter().any(|s| s == "failed"),
        "observable error must be failed: {states2:?}"
    );
    assert!(
        !states2.iter().any(|s| s == "unknown"),
        "a proven error must not be downgraded to unknown: {states2:?}"
    );
}

/// regression（誠實階梯）：`system/init` 曾直接被翻成 TaskAccepted，
/// 讓「working」跑在「fetched」前面——角色在任務還沒送進去時就演工作中。
/// 正確順序：created → fetched（真的寫進 stdin）→ working（agent 真的動了）。
#[tokio::test]
async fn working_never_precedes_the_task_actually_reaching_the_agent() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;
    let slow = scenario_workdir("slow");
    let mut input = claude_input("慢慢做", None);
    input.workdir = Some(slow.path().to_string_lossy().into_owned());
    let sid = rt
        .create_agent_session(input)
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();

    // 子程序起來後（init 已送出），在送任務之前不得出現任何工作狀態。
    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .ok()
                .and_then(|r| r.provider_session_id)
                .is_some()
        },
        "provider session id from init",
    )
    .await;
    let before = session_states(&rt, &sid);
    assert_eq!(
        before,
        vec!["created".to_string()],
        "init alone must not imply fetched/working: {before:?}"
    );

    rt.mailbox_send(
        &sid,
        MailboxDirection::ToSession,
        "task",
        BTreeMap::from([("task".to_string(), json!("看一下 repo"))]),
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

    let states = session_states(&rt, &sid);
    let index = |name: &str| {
        states
            .iter()
            .position(|s| s == name)
            .unwrap_or_else(|| panic!("missing {name} in {states:?}"))
    };
    assert!(
        index("created") < index("fetched"),
        "created → fetched: {states:?}"
    );
    assert!(
        index("fetched") < index("working"),
        "fetched → working: {states:?}"
    );
    assert!(
        index("working") < index("claimed-completed"),
        "working → claimed-completed: {states:?}"
    );
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
}

/// regression（誠實階梯）：訊息「排進佇列」不是送達。子程序不再讀 stdin 時
/// 寫入會失敗——此時絕不可蓋 delivered 戳記，也絕不可發 `fetched`。
#[tokio::test]
async fn a_write_that_fails_is_never_reported_as_delivered_or_fetched() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;
    let deaf = scenario_workdir("deaf");
    let mut input = claude_input("不讀 stdin", None);
    input.workdir = Some(deaf.path().to_string_lossy().into_owned());
    let sid = rt
        .create_agent_session(input)
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();
    // 等子程序真的關掉 stdin（否則寫入還會被 pipe buffer 吃下去）。
    let closed_marker = deaf.path().join("fake-stdin-closed");
    wait_for(async || closed_marker.exists(), "fixture closed its stdin").await;

    let message = rt
        .mailbox_send(
            &sid,
            MailboxDirection::ToSession,
            "task",
            BTreeMap::from([("task".to_string(), json!("這一則永遠送不進去"))]),
            None,
        )
        .await
        .unwrap();
    let queued = rt
        .mailbox_peek(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    let queued = queued
        .iter()
        .find(|m| m.message_id == message.message_id)
        .expect("message stays in the mailbox");
    assert!(
        queued.delivered_at.is_none(),
        "a failed write must not stamp delivered"
    );
    let states = session_states(&rt, &sid);
    assert!(
        !states.iter().any(|s| s == "fetched"),
        "no fetched without a real delivery: {states:?}"
    );
    assert!(
        states.iter().any(|s| s == "failed"),
        "the undeliverable task is reported honestly: {states:?}"
    );

    let pid = read_pid(&deaf.path().join("fake-pid"));
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    if pid > 0 {
        wait_for(async || !pid_alive(pid), "deaf fixture killed").await;
    }
}

/// 續開既有 provider thread：thread id 要真的傳給 connector，而權限旗標
/// **重新上鎖**——舊 session 的寫入權不會跟著 thread id 一起被繼承。
#[tokio::test]
async fn resume_passes_the_provider_thread_and_relocks_permission_flags() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;
    let dir = scenario_workdir("default");
    let mut input = claude_input("續開", None);
    input.workdir = Some(dir.path().to_string_lossy().into_owned());
    input.resume_provider_session_id = Some("fake-thread-9".into());
    let record = rt.create_agent_session(input).await.unwrap();
    let sid = record.session_id.as_str().to_string();

    let argv_file = dir.path().join("fake-argv");
    wait_for(async || argv_file.exists(), "fixture argv log").await;
    let argv = std::fs::read_to_string(&argv_file).unwrap();
    assert!(
        argv.contains("--resume fake-thread-9"),
        "resume must reach the connector: {argv}"
    );
    // 沒有要求寫入 ⇒ 續開仍是唯讀 Plan 模式，不繼承任何放寬。
    assert!(
        argv.contains("--permission-mode plan"),
        "resume must re-lock to plan: {argv}"
    );
    assert!(!argv.contains("acceptEdits"), "{argv}");
    assert!(!argv.contains("dangerously"), "{argv}");
    assert!(!record.allow_write, "resume never inherits write access");
    assert!(record.consent_scope.is_empty());
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
}

/// 裁決要回寫 mailbox。介面讀得到的只有 mailbox：沒有 `approval-resolved`
/// 這筆，「已被看門狗自動拒絕」跟「還在等你決定」在畫面上完全一樣，核可
/// 按鈕會一直掛著，按下去只會拿到 NotFound。
#[tokio::test]
async fn a_watchdog_auto_deny_is_written_back_to_the_mailbox() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CODEX_BIN", codex_fixture_path());
    let (_g, rt) = runtime().await;
    rt.set_approval_ttl_secs(0);
    let dir = tempfile::tempdir().unwrap();
    let sid = rt
        .create_agent_session(codex_input("看門狗回寫", dir.path()))
        .await
        .expect("fake codex app-server attaches")
        .session_id
        .as_str()
        .to_string();
    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .map(|r| r.state == AgentSessionState::WaitingForConsent)
                .unwrap_or(false)
        },
        "waiting-for-consent",
    )
    .await;

    rt.gateway_sweep().await;

    let inbox = rt
        .mailbox_peek(&sid, MailboxDirection::FromSession)
        .await
        .unwrap();
    let resolved = inbox
        .iter()
        .find(|m| m.kind == "approval-resolved")
        .expect("the auto-deny reaches the mailbox the UI actually reads");
    assert_eq!(resolved.body["requestId"], json!("9001"));
    assert_eq!(resolved.body["decision"], json!("denied"));
    assert_eq!(
        resolved.body["by"],
        json!("watchdog"),
        "a timeout is NOT a human decision and must not read as one"
    );
    assert_eq!(resolved.body["approved"], json!(false));
    assert_eq!(resolved.body["deliveredToAgent"], json!(true));
    assert!(resolved.body["summary"]
        .as_str()
        .unwrap_or_default()
        .contains("rm -rf"));
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}

/// 人類裁決同樣回寫，而且標明是「人」決定的（by=human）——UI 靠這個欄位
/// 分辨「你已核可」與「看門狗替你拒絕了」。
#[tokio::test]
async fn a_human_decision_is_written_back_to_the_mailbox_as_human() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CODEX_BIN", codex_fixture_path());
    let (_g, rt) = runtime().await;
    let dir = tempfile::tempdir().unwrap();
    let sid = rt
        .create_agent_session(codex_input("人類裁決", dir.path()))
        .await
        .expect("fake codex app-server attaches")
        .session_id
        .as_str()
        .to_string();
    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .map(|r| r.state == AgentSessionState::WaitingForConsent)
                .unwrap_or(false)
        },
        "waiting-for-consent",
    )
    .await;

    let out = rt
        .gateway_resolve_approval(&sid, "9001", true)
        .await
        .unwrap();
    assert_eq!(out["by"], json!("human"));
    assert_eq!(out["deliveredToAgent"], json!(true));

    let inbox = rt
        .mailbox_peek(&sid, MailboxDirection::FromSession)
        .await
        .unwrap();
    let resolved = inbox
        .iter()
        .find(|m| m.kind == "approval-resolved")
        .expect("the human decision reaches the mailbox");
    assert_eq!(resolved.body["by"], json!("human"));
    assert_eq!(resolved.body["decision"], json!("approved"));
    assert_eq!(resolved.body["deliveredToAgent"], json!(true));
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}

/// Codex 續開（`thread/resume`）必須**重新上鎖**：cwd／approvalPolicy／
/// sandbox 跟 `thread/start` 一樣重送一次。只送 `threadId` 等於讓 provider
/// 端沿用舊 thread 的權限——舊 session 若曾是 workspace-write，寫入權就跟著
/// thread id 復活了。這裡直接讀 fixture 記下的 resume 參數。
#[tokio::test]
async fn codex_resume_resends_cwd_and_sandbox_instead_of_inheriting_them() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CODEX_BIN", codex_fixture_path());
    let (_g, rt) = runtime().await;

    // (1) 唯讀續開：resume 參數必須帶著 read-only 的重新上鎖。
    let dir = tempfile::tempdir().unwrap();
    let mut input = codex_input("續開唯讀", dir.path());
    input.resume_provider_session_id = Some("fake-thread-9".into());
    let record = rt.create_agent_session(input).await.unwrap();
    let sid = record.session_id.as_str().to_string();

    let resume_file = dir.path().join("fake-thread-resume");
    wait_for(async || resume_file.exists(), "thread/resume params log").await;
    let raw = std::fs::read_to_string(&resume_file).unwrap();
    let sent: serde_json::Value = serde_json::from_str(raw.trim()).expect("resume line is JSON");
    assert_eq!(sent["method"], json!("thread/resume"));
    let params = &sent["params"];
    assert_eq!(params["threadId"], json!("fake-thread-9"));
    assert_eq!(
        params["sandbox"],
        json!("read-only"),
        "resume must re-lock the sandbox: {params}"
    );
    assert_eq!(params["approvalPolicy"], json!("untrusted"), "{params}");
    assert_eq!(
        params["cwd"].as_str().unwrap_or_default(),
        dir.path().to_string_lossy(),
        "resume must re-scope the working directory: {params}"
    );
    assert!(
        !raw.contains("workspace-write") && !raw.contains("danger-full-access"),
        "a read-only resume must never send a writable sandbox: {raw}"
    );
    // 沒有走 thread/start（否則就不是續開了），而且 provider thread 有換上。
    assert!(!dir.path().join("fake-thread-start").exists());
    assert!(!record.allow_write, "resume never inherits write access");
    rt.close_agent_session(&sid, None, "closed").await.unwrap();

    // (2) 這次 SessionSpec 明確要寫入 ⇒ resume 送 workspace-write。
    // 也就是說 sandbox 完全由**本次**授權決定，與舊 thread 無關。
    let dir2 = tempfile::tempdir().unwrap();
    let mut write_input = codex_input("續開可寫", dir2.path());
    write_input.resume_provider_session_id = Some("fake-thread-9".into());
    write_input.allow_write = true;
    write_input.tool_scope = vec!["workspace.write".into()];
    write_input.consent_scope = vec!["agent-session:workspace-write".into()];
    let record2 = rt.create_agent_session(write_input).await.unwrap();
    let sid2 = record2.session_id.as_str().to_string();
    let resume_file2 = dir2.path().join("fake-thread-resume");
    wait_for(async || resume_file2.exists(), "write resume params log").await;
    let sent2: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&resume_file2).unwrap().trim()).unwrap();
    assert_eq!(sent2["params"]["sandbox"], json!("workspace-write"));
    rt.close_agent_session(&sid2, None, "closed").await.unwrap();

    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}

/// Approval 對稱性：`claude -p` 沒有互動核可管道，所以 runtime 絕不會為
/// claude session 掛出一個沒有人能裁決的 waiting-consent；對它裁決一律
/// 誠實回 NotFound（而不是假裝送出了一個核可）。
#[tokio::test]
async fn claude_sessions_never_show_a_consent_prompt_nobody_can_answer() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;
    let dir = scenario_workdir("default");
    let mut input = claude_input("無核可管道", None);
    input.workdir = Some(dir.path().to_string_lossy().into_owned());
    let sid = rt
        .create_agent_session(input)
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();

    let err = rt
        .gateway_resolve_approval(&sid, "made-up-request", true)
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::NotFound(_)), "{err:?}");
    assert!(
        !session_states(&rt, &sid)
            .iter()
            .any(|s| s == "waiting-consent"),
        "claude sessions must never fabricate a consent prompt"
    );
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
}

/// 無人裁決的 approval 必須**逾時自動拒絕**（絕不替人類同意），而且拒絕
/// 要真的送到 agent 子程序，不是只寫在 log 裡。
#[tokio::test]
async fn an_unanswered_approval_is_denied_by_the_watchdog_and_the_deny_reaches_the_agent() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CODEX_BIN", codex_fixture_path());
    let (_g, rt) = runtime().await;
    // TTL 只能被調短（上限仍是 APPROVAL_TTL_SECS）。
    assert_eq!(rt.set_approval_ttl_secs(0), 0);
    assert_eq!(
        rt.set_approval_ttl_secs(99_999),
        interaction_runtime::gateway::APPROVAL_TTL_SECS,
        "TTL is a ceiling: it can be shortened, never extended"
    );
    rt.set_approval_ttl_secs(0);

    let dir = tempfile::tempdir().unwrap();
    let record = rt
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: None,
            agent_id: "codex".into(),
            label: Some("等待核可".into()),
            ttl_minutes: Some(10),
            data_scope: vec![],
            tool_scope: vec![],
            consent_scope: vec![],
            max_cost: None,
            max_messages: Some(10),
            delegation: None,
            workdir: Some(dir.path().to_string_lossy().into_owned()),
            resume_provider_session_id: None,
            allow_write: false,
        })
        .await
        .expect("fake codex app-server attaches");
    let sid = record.session_id.as_str().to_string();

    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .map(|r| r.state == AgentSessionState::WaitingForConsent)
                .unwrap_or(false)
        },
        "waiting-for-consent",
    )
    .await;
    // 請求描述有進信箱（人類要知道自己在裁決什麼）。
    let inbox = rt
        .mailbox_peek(&sid, MailboxDirection::FromSession)
        .await
        .unwrap();
    let request = inbox
        .iter()
        .find(|m| m.kind == "approval-request")
        .expect("approval request reaches the mailbox");
    assert!(request
        .body
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("rm -rf"));

    // watchdog 掃描：逾時 → 自動拒絕。
    rt.gateway_sweep().await;
    let decision_file = dir.path().join("fake-approval-decision");
    wait_for(
        async || {
            std::fs::read_to_string(&decision_file)
                .map(|body| body.contains("reject"))
                .unwrap_or(false)
        },
        "deny actually delivered to the agent subprocess",
    )
    .await;
    let decision = std::fs::read_to_string(&decision_file).unwrap();
    assert!(
        !decision.contains("accept"),
        "the runtime must never approve on a human's behalf: {decision}"
    );
    // 再次裁決同一個請求 → 已經不存在（不得重複核可）。
    assert!(rt
        .gateway_resolve_approval(&sid, "9001", true)
        .await
        .is_err());

    // 人類（或逾時規則）到底拒絕了「什麼」必須留在紀錄裡：只留一個
    // request id 的稽核等於沒有稽核。
    let observations = rt
        .observe_stored(&ObservationQuery {
            receptor_id: Some(ReceptorId::new("agent.session")),
            limit: Some(100),
            ..Default::default()
        })
        .await
        .unwrap();
    let denied = observations
        .iter()
        .find(|o| {
            o.inferences
                .get("report")
                .and_then(|r| r.get("approvalAutoDenied"))
                .is_some()
        })
        .expect("the auto-deny is recorded as an observation");
    assert!(
        denied.inferences["report"]["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("rm -rf"),
        "the record must say WHAT was auto-denied: {:?}",
        denied.inferences["report"]
    );
    assert_eq!(
        denied.inferences["report"]["delivered"],
        json!(true),
        "deciding to deny and the deny arriving are two different facts"
    );

    // 送達證明：writer 真的寫進 stdin＋flush 之後才會有 fetched。
    rt.mailbox_send(
        &sid,
        MailboxDirection::ToSession,
        "task",
        BTreeMap::from([("task".to_string(), json!("繼續"))]),
        None,
    )
    .await
    .unwrap();
    assert!(
        session_states(&rt, &sid).iter().any(|s| s == "fetched"),
        "an acknowledged stdin write is what earns 'fetched': {:?}",
        session_states(&rt, &sid)
    );

    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}

fn codex_exec_fixture_path() -> String {
    format!(
        "{}/tests/fixtures/fake_codex_exec.sh",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn task(text: &str) -> BTreeMap<String, serde_json::Value> {
    BTreeMap::from([("task".to_string(), json!(text))])
}

/// regression（agent-honesty）：「本輪已有結局」曾是 session 層級、只設不清
/// 的旗標——第一輪聲稱完成後，第二輪子程序死掉不會發 unknown／failed，
/// session 對著一個已死的程序永遠「工作中」。結局是每一輪各自的事實：
/// 第二輪 exit≠0 必須是 failed、exit 0 無結果必須是 unknown；人工驗證只
/// 綁第一個 claim，新任務一送達就失效；死掉之後再送任務必須被明確拒絕。
#[tokio::test]
async fn a_second_turn_that_dies_is_not_covered_by_the_first_turns_claim() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;

    for (mode, expected, taxonomy) in [
        ("claim-then-crash", AgentSessionState::Failed, "failed"),
        ("claim-then-silent", AgentSessionState::Unknown, "unknown"),
    ] {
        let dir = scenario_workdir(mode);
        let mut input = claude_input(mode, None);
        input.workdir = Some(dir.path().to_string_lossy().into_owned());
        let sid = rt
            .create_agent_session(input)
            .await
            .unwrap()
            .session_id
            .as_str()
            .to_string();

        // 第一輪：健康的聲稱完成。
        rt.mailbox_send(
            &sid,
            MailboxDirection::ToSession,
            "task",
            task("第一輪"),
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
            &format!("{mode}: first-turn claim"),
        )
        .await;
        // 人工驗證第一個 claim：驗證紀錄指向這個 claim id。
        let verified = rt
            .verify_agent_session(&sid, Some("第一輪看過了".into()))
            .await
            .unwrap();
        let first_claim = verified
            .claim_id
            .clone()
            .expect("a claimed-completed record carries a claim id");
        assert_eq!(
            verified
                .human_verified
                .as_ref()
                .and_then(|v| v.claim_id.clone()),
            Some(first_claim.clone()),
            "{mode}: the verification is bound to the claim it confirmed"
        );

        // 第二輪：任務真的送進子程序 ⇒ 舊的驗證只屬於上一個 claim。
        rt.mailbox_send(
            &sid,
            MailboxDirection::ToSession,
            "task",
            task("第二輪"),
            None,
        )
        .await
        .unwrap();
        let after = rt.get_agent_session(&sid).await.unwrap();
        assert!(
            after.human_verified.is_none(),
            "{mode}: a newly delivered task invalidates the old verification"
        );
        assert_eq!(
            after.claim_id,
            Some(first_claim.clone()),
            "{mode}: the claim id only changes on a new claim"
        );

        // 子程序死在第二輪：第一輪的聲稱不能替它擔保。
        wait_for(
            async || {
                rt.get_agent_session(&sid)
                    .await
                    .map(|r| r.state == expected)
                    .unwrap_or(false)
            },
            &format!("{mode}: honest outcome after the second turn died"),
        )
        .await;
        let record = rt.get_agent_session(&sid).await.unwrap();
        assert!(!record.state.is_open(), "{mode}: the outcome is terminal");
        assert!(record.human_verified.is_none());
        let states = session_states(&rt, &sid);
        assert_eq!(
            states.last().map(String::as_str),
            Some(taxonomy),
            "{mode}: the character must not keep performing 'working': {states:?}"
        );
        assert_eq!(
            states.iter().filter(|s| *s == "claimed-completed").count(),
            1,
            "{mode}: only the first turn claimed anything: {states:?}"
        );
        if expected == AgentSessionState::Failed {
            assert!(
                !states.iter().any(|s| s == "unknown"),
                "{mode}: exit 3 is an observable error, not unknown: {states:?}"
            );
        } else {
            assert!(
                !states.iter().any(|s| s == "failed"),
                "{mode}: exit 0 without a result is not evidence of failure: {states:?}"
            );
        }

        // 死掉之後再送：明確拒絕（不得靜默 Ok 留下一則永遠 undelivered 的訊息）。
        let err = rt
            .mailbox_send(
                &sid,
                MailboxDirection::ToSession,
                "task",
                task("第三輪"),
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Conflict(_)), "{mode}: {err:?}");

        // 關閉只收尾，不把 failed／unknown 改寫成 closed。
        let closed = rt.close_agent_session(&sid, None, "closed").await.unwrap();
        assert_eq!(closed.state, expected, "{mode}: close keeps the outcome");
        assert!(closed.closed_at.is_some());
    }
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 聲稱完成後子程序自行結束（exit 0）：聲稱仍然成立（不是 unknown、不是
/// failed），但之後再送任務必須明確回錯——事件泵收攤後沒有人會送這則訊息，
/// 靜默 Ok 會留下一則永遠「等待送達」的訊息，也不會有任何事件說明為什麼。
#[tokio::test]
async fn after_the_process_exits_a_new_task_is_refused_instead_of_silently_queued() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;
    let dir = scenario_workdir("claim-then-exit");
    let mut input = claude_input("做完就走", None);
    input.workdir = Some(dir.path().to_string_lossy().into_owned());
    let sid = rt
        .create_agent_session(input)
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();
    assert!(rt.gateway_session_attached(&sid));

    rt.mailbox_send(
        &sid,
        MailboxDirection::ToSession,
        "task",
        task("第一輪"),
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
        "claim",
    )
    .await;
    // 子程序結束、事件泵收攤——聲稱不因此變成 unknown。
    wait_for(
        async || !rt.gateway_session_attached(&sid),
        "event pump finished after the process exited",
    )
    .await;
    let record = rt.get_agent_session(&sid).await.unwrap();
    assert_eq!(record.state, AgentSessionState::ClaimedCompleted);
    let states = session_states(&rt, &sid);
    assert!(
        !states.iter().any(|s| s == "unknown" || s == "failed"),
        "a claim that was actually made stands: {states:?}"
    );

    // 再送：沒有人能送了 ⇒ 明確回錯，狀態不動。
    let err = rt
        .mailbox_send(
            &sid,
            MailboxDirection::ToSession,
            "task",
            task("再交代一句"),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Unavailable(_)), "{err:?}");
    assert!(err.to_string().contains("未送達"), "{err}");
    assert_eq!(
        rt.get_agent_session(&sid).await.unwrap().state,
        AgentSessionState::ClaimedCompleted
    );
    let queued = rt
        .mailbox_peek(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    let ghost = queued
        .iter()
        .find(|m| m.body.get("task") == Some(&json!("再交代一句")))
        .expect("the message is visible in the mailbox");
    assert!(ghost.delivered_at.is_none(), "never stamped delivered");

    let closed = rt.close_agent_session(&sid, None, "closed").await.unwrap();
    assert_eq!(closed.state, AgentSessionState::Closed);
    assert_eq!(
        closed.detail.as_deref(),
        Some("closed (was ClaimedCompleted)")
    );
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// regression（agent-honesty）：codex exec fallback 上一輪還在跑時的第二則
/// 訊息回 `GatewayError::Busy`，runtime 曾把它翻成 `failed`——Failed 是
/// terminal，看門狗接著殺掉還在正常工作的子程序，真正的結局也再也記不進來。
/// 「訊息未送達」≠「任務失敗」：呼叫端拿到明確錯誤、訊息留在信箱沒有
/// delivered 戳記、狀態不動、子程序活著、第一輪的聲稱照常被記錄。
#[tokio::test]
async fn an_undelivered_message_during_a_running_exec_turn_is_not_an_agent_failure() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CODEX_BIN", codex_exec_fixture_path());
    let (_g, rt) = runtime().await;
    let dir = tempfile::tempdir().unwrap();
    let mut input = codex_input("exec 忙碌中", dir.path());
    input.max_messages = Some(50);
    let sid = rt
        .create_agent_session(input)
        .await
        .expect("fake codex without app-server attaches through the exec fallback")
        .session_id
        .as_str()
        .to_string();

    // 第一則：真的 spawn 一個 exec turn（fixture 睡 1.5 秒才聲稱完成）。
    rt.mailbox_send(
        &sid,
        MailboxDirection::ToSession,
        "task",
        task("第一則"),
        None,
    )
    .await
    .unwrap();
    let pid_file = dir.path().join("fake-pid");
    let mut pid = 0;
    wait_for(
        async || {
            pid = read_pid(&pid_file);
            pid > 0 && pid_alive(pid)
        },
        "exec turn running",
    )
    .await;

    // 第一輪還在跑：第二則必須是「未送達」，不是「任務失敗」。
    let err = rt
        .mailbox_send(
            &sid,
            MailboxDirection::ToSession,
            "task",
            task("第二則"),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Conflict(_)), "{err:?}");
    assert!(err.to_string().contains("未送達"), "{err}");
    let record = rt.get_agent_session(&sid).await.unwrap();
    assert!(
        record.state.is_open(),
        "the agent is working normally: {:?}",
        record.state
    );
    assert_ne!(record.state, AgentSessionState::Failed);
    let states = session_states(&rt, &sid);
    assert!(
        !states.iter().any(|s| s == "failed"),
        "an undelivered message is not an agent failure: {states:?}"
    );
    let queued = rt
        .mailbox_peek(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    assert!(queued
        .iter()
        .find(|m| m.body.get("task") == Some(&json!("第一則")))
        .unwrap()
        .delivered_at
        .is_some());
    assert!(queued
        .iter()
        .find(|m| m.body.get("task") == Some(&json!("第二則")))
        .unwrap()
        .delivered_at
        .is_none());

    // 看門狗掃描不得殺掉還在工作的子程序（record 仍 open）。
    rt.gateway_sweep().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        pid_alive(pid),
        "the running exec turn must survive the sweep"
    );

    // 第一輪的結局照常被記錄。
    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .map(|r| r.state == AgentSessionState::ClaimedCompleted)
                .unwrap_or(false)
        },
        "first turn's claim still lands",
    )
    .await;
    // 第一輪結束（子程序收割、busy 清除）後可以再送——有界重試。
    let mut delivered = false;
    for _ in 0..40 {
        match rt
            .mailbox_send(
                &sid,
                MailboxDirection::ToSession,
                "task",
                task("第三則"),
                None,
            )
            .await
        {
            Ok(_) => {
                delivered = true;
                break;
            }
            Err(DomainError::Conflict(_)) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => panic!("unexpected: {e:?}"),
        }
    }
    assert!(
        delivered,
        "a new turn can start once the previous one ended"
    );
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}

/// regression（agent-honesty）：看門狗自動拒絕時，即使拒絕**沒送到** agent
/// 也曾回報 progress → 卡片變「工作中」，而 pending 登記已被刪掉，人類再按
/// 核可／拒絕只會拿到 NotFound——agent 其實仍卡在等核可。送不到就照實說：
/// 狀態留在 waiting-consent、請求留在登記中、delivered:false 進紀錄。
#[tokio::test]
async fn a_watchdog_deny_that_cannot_reach_the_agent_keeps_the_request_pending() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CODEX_BIN", codex_fixture_path());
    let (_g, rt) = runtime().await;
    rt.set_approval_ttl_secs(0);
    let dir = scenario_workdir("deaf-after-approval");
    let sid = rt
        .create_agent_session(codex_input("聾掉的 agent", dir.path()))
        .await
        .expect("fake codex app-server attaches")
        .session_id
        .as_str()
        .to_string();
    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .map(|r| r.state == AgentSessionState::WaitingForConsent)
                .unwrap_or(false)
        },
        "waiting-for-consent",
    )
    .await;
    let closed_marker = dir.path().join("fake-stdin-closed");
    wait_for(async || closed_marker.exists(), "fixture closed its stdin").await;

    rt.gateway_sweep().await;

    // 狀態不得翻成「工作中」。
    let record = rt.get_agent_session(&sid).await.unwrap();
    assert_eq!(
        record.state,
        AgentSessionState::WaitingForConsent,
        "the agent is still blocked on the approval"
    );
    let states = session_states(&rt, &sid);
    assert!(
        !states.iter().any(|s| s == "working"),
        "an undelivered deny must not be performed as progress: {states:?}"
    );
    // 裁決紀錄誠實：決定了拒絕、但沒送到、請求仍待裁決。
    let inbox = rt
        .mailbox_peek(&sid, MailboxDirection::FromSession)
        .await
        .unwrap();
    let watchdog: Vec<_> = inbox
        .iter()
        .filter(|m| m.kind == "approval-resolved" && m.body["by"] == json!("watchdog"))
        .collect();
    assert_eq!(watchdog.len(), 1, "{inbox:?}");
    assert_eq!(watchdog[0].body["approved"], json!(false));
    assert_eq!(watchdog[0].body["deliveredToAgent"], json!(false));
    assert_eq!(watchdog[0].body["stillPending"], json!(true));
    // 觀察紀錄同樣 delivered:false，且事件是 waiting-for-consent 而非 progress。
    let observations = rt
        .observe_stored(&ObservationQuery {
            receptor_id: Some(ReceptorId::new("agent.session")),
            limit: Some(100),
            ..Default::default()
        })
        .await
        .unwrap();
    let denied = observations
        .iter()
        .find(|o| {
            o.inferences
                .get("report")
                .and_then(|r| r.get("approvalAutoDenied"))
                .is_some()
        })
        .expect("the auto-deny attempt is recorded");
    assert_eq!(denied.inferences["report"]["delivered"], json!(false));
    assert_eq!(denied.facts["event"], json!("waiting-for-consent"));
    // deny 從未寫進 agent。
    assert!(!dir.path().join("fake-approval-decision").exists());

    // 人類仍可對同一個請求裁決：請求還在（不是 NotFound），只是這次也送不到。
    let err = rt
        .gateway_resolve_approval(&sid, "9001", false)
        .await
        .unwrap_err();
    assert!(
        !matches!(err, DomainError::NotFound(_)),
        "the request must stay decidable: {err:?}"
    );
    assert!(matches!(err, DomainError::Unavailable(_)), "{err:?}");
    assert_eq!(
        rt.get_agent_session(&sid).await.unwrap().state,
        AgentSessionState::WaitingForConsent
    );

    // 再掃一次：退避中，不重複灌同一句話。
    rt.gateway_sweep().await;
    let inbox = rt
        .mailbox_peek(&sid, MailboxDirection::FromSession)
        .await
        .unwrap();
    assert_eq!(
        inbox
            .iter()
            .filter(|m| m.kind == "approval-resolved" && m.body["by"] == json!("watchdog"))
            .count(),
        1,
        "backoff: the watchdog does not retry on every tick"
    );
    assert_eq!(
        rt.get_agent_session(&sid).await.unwrap().state,
        AgentSessionState::WaitingForConsent
    );

    let pid = read_pid(&dir.path().join("fake-pid"));
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    wait_for(async || !pid_alive(pid), "deaf fixture killed on close").await;
    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}

/// regression（agent-honesty）：codex app-server 的 `turn/completed` 曾無條件
/// 翻成 claimed-completed——協定裡 turn 的每種結局（完成／被 turn/interrupt
/// 中斷／失敗）都只走 turn/completed，用 `turn.status` 區分。使用者按「中斷」
/// 後，session 必須是 cancelled，絕不可演成「Agent 說做完了」。
#[tokio::test]
async fn an_interrupted_codex_turn_is_cancelled_not_claimed_completed() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CODEX_BIN", codex_fixture_path());
    let (_g, rt) = runtime().await;
    let dir = scenario_workdir("turns");
    let sid = rt
        .create_agent_session(codex_input("可中斷的 turn", dir.path()))
        .await
        .expect("fake codex app-server attaches")
        .session_id
        .as_str()
        .to_string();

    rt.mailbox_send(
        &sid,
        MailboxDirection::ToSession,
        "task",
        task("開始"),
        None,
    )
    .await
    .unwrap();
    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .map(|r| r.state == AgentSessionState::Active)
                .unwrap_or(false)
        },
        "turn/started → working",
    )
    .await;

    let out = rt.gateway_interrupt(&sid).await.unwrap();
    assert_eq!(out["interrupted"], json!(true));
    let marker = dir.path().join("fake-turn-interrupt");
    wait_for(async || marker.exists(), "turn/interrupt reached the agent").await;
    assert!(std::fs::read_to_string(&marker)
        .unwrap()
        .contains("\"turnId\":\"turn-1\""));

    wait_for(
        async || {
            rt.get_agent_session(&sid)
                .await
                .map(|r| r.state == AgentSessionState::Cancelled)
                .unwrap_or(false)
        },
        "turn/completed(status=interrupted) → cancelled",
    )
    .await;
    let states = session_states(&rt, &sid);
    assert!(states.iter().any(|s| s == "cancelled"), "{states:?}");
    assert!(
        !states
            .iter()
            .any(|s| s == "claimed-completed" || s == "unknown" || s == "failed"),
        "an interrupted turn is neither a claim nor a failure: {states:?}"
    );
    let record = rt.get_agent_session(&sid).await.unwrap();
    assert!(!record.state.is_open());
    assert!(record.claim_id.is_none(), "nothing was ever claimed");

    // 關閉只收尾：cancelled 這個結局留在主要狀態。
    let closed = rt.close_agent_session(&sid, None, "closed").await.unwrap();
    assert_eq!(closed.state, AgentSessionState::Cancelled);
    assert_eq!(closed.detail.as_deref(), Some("closed (was Cancelled)"));
    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}
