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
        // 工作目錄是使用者的專案資料夾，不是 runtime 自己的家（那裡面有
        // 人類 capability token；見 workdir_never_exposes_the_runtime_state_dir）。
        let project = home.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let mut missing = claude_input("缺少授權", None);
        missing.allow_write = true;
        missing.workdir = Some(project.to_string_lossy().into_owned());
        let err = rt.create_agent_session(missing).await.unwrap_err();
        assert!(matches!(err, DomainError::ConsentRequired(_)), "{err:?}");

        let mut writable = claude_input("限權寫入", None);
        writable.allow_write = true;
        writable.workdir = Some(project.to_string_lossy().into_owned());
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
    let argv_file = dir.path().join("fake-argv");
    // 先有一次由本 runtime 授權過的工作：續開一定要有紀錄可比對
    // （沒有紀錄的 thread id 一律拒絕，見
    // `resume_without_a_recorded_grant_is_refused`）。
    let mut first = claude_input("先開一次", None);
    first.workdir = Some(dir.path().to_string_lossy().into_owned());
    let first_sid = rt
        .create_agent_session(first)
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();
    wait_for(
        async || {
            rt.get_agent_session(&first_sid)
                .await
                .map(|r| r.provider_session_id.as_deref() == Some("fake-123"))
                .unwrap_or(false)
        },
        "provider session id",
    )
    .await;
    rt.close_agent_session(&first_sid, None, "closed")
        .await
        .unwrap();
    std::fs::remove_file(&argv_file).unwrap();

    let mut input = claude_input("續開", None);
    input.workdir = Some(dir.path().to_string_lossy().into_owned());
    input.resume_provider_session_id = Some("fake-123".into());
    let record = rt.create_agent_session(input).await.unwrap();
    let sid = record.session_id.as_str().to_string();

    wait_for(async || argv_file.exists(), "fixture argv log").await;
    let argv = std::fs::read_to_string(&argv_file).unwrap();
    assert!(
        argv.contains("--resume fake-123"),
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
    //     續開的對象必須是本 runtime 授權過的紀錄（沒有紀錄一律拒絕，見
    //     `resume_without_a_recorded_grant_is_refused`），所以先真的開一次。
    let dir = tempfile::tempdir().unwrap();
    let first = rt
        .create_agent_session(codex_input("先開一次", dir.path()))
        .await
        .unwrap();
    assert_eq!(first.provider_session_id.as_deref(), Some("fake-thread-1"));
    rt.close_agent_session(first.session_id.as_str(), None, "closed")
        .await
        .unwrap();
    std::fs::remove_file(dir.path().join("fake-thread-start")).unwrap();

    let mut input = codex_input("續開唯讀", dir.path());
    input.resume_provider_session_id = Some("fake-thread-1".into());
    let record = rt.create_agent_session(input).await.unwrap();
    let sid = record.session_id.as_str().to_string();

    let resume_file = dir.path().join("fake-thread-resume");
    wait_for(async || resume_file.exists(), "thread/resume params log").await;
    let raw = std::fs::read_to_string(&resume_file).unwrap();
    let sent: serde_json::Value = serde_json::from_str(raw.trim()).expect("resume line is JSON");
    assert_eq!(sent["method"], json!("thread/resume"));
    let params = &sent["params"];
    assert_eq!(params["threadId"], json!("fake-thread-1"));
    assert_eq!(
        params["sandbox"],
        json!("read-only"),
        "resume must re-lock the sandbox: {params}"
    );
    assert_eq!(params["approvalPolicy"], json!("untrusted"), "{params}");
    // 送出去的 cwd 是**正規化後**的絕對路徑（symlink／`..` 都解掉）：那才是
    // 子程序真的被掛上去、也是記錄下來供續開比對的那一個目錄。
    assert_eq!(
        params["cwd"].as_str().unwrap_or_default(),
        dir.path().canonicalize().unwrap().to_string_lossy(),
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
    // 同樣先有一次「本來就被授權可寫」的工作：續開不得憑空取得寫入權，
    // 所以比對的對象必須自己就是寫入型 session。它得留著（consent 隨
    // session 結束而消滅——已關閉的工作階段永遠無法再帶回寫入授權，見
    // `resume_cannot_widen_scope` 的 sneaky 分支）。
    let mut first_write = codex_input("先開一次可寫", dir2.path());
    first_write.allow_write = true;
    first_write.tool_scope = vec!["workspace.write".into()];
    first_write.consent_scope = vec!["agent-session:workspace-write".into()];
    let first_write = rt.create_agent_session(first_write).await.unwrap();
    assert_eq!(
        first_write.provider_session_id.as_deref(),
        Some("fake-thread-1")
    );

    let mut write_input = codex_input("續開可寫", dir2.path());
    write_input.resume_provider_session_id = Some("fake-thread-1".into());
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
    rt.close_agent_session(first_write.session_id.as_str(), None, "closed")
        .await
        .unwrap();

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

/// regression（agent-honesty：終局狀態不得被觀察管線吃掉）：`report_agent_session`
/// 曾把 `ingest("agent.session", …).await?` 排在 `emit_agent_session_state` 前面。
/// `agent.session` push receptor 只要不可用（被停用／不存在——例如中斷的同時感測層
/// 被關掉），`?` 就提早返回，**事件完全不發**：SSE 從中斷前的序號重放只會停在
/// `working`，每一個即時畫面都停在「處理中」直到重新載入，而後端其實已經
/// cancelled。state 已經落地就必須發得出去；同樣的形狀也會讓 failed／unknown／
/// timed-out 一起靜默。
#[tokio::test]
async fn a_cancelled_session_still_emits_its_state_event_when_ingest_is_unavailable() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CODEX_BIN", codex_fixture_path());
    let (_g, rt) = runtime().await;
    let dir = scenario_workdir("turns");
    let sid = rt
        .create_agent_session(codex_input("中斷時觀察管線壞掉", dir.path()))
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
    assert!(
        session_states(&rt, &sid).iter().any(|s| s == "working"),
        "working 是中斷前使用者看到的最後一個狀態"
    );

    // 觀察管線壞掉：session-as-receptor 這條路徑從現在起會失敗。
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("agent.session"), false)
        .await
        .unwrap();
    // 確認前提成立：這時候 report 真的會回 Err（測試沒有把 bug 條件擺錯）。
    assert!(
        rt.report_agent_session(&sid, "progress", json!({}))
            .await
            .is_err(),
        "停用 agent.session receptor 之後 ingest 必須真的失敗"
    );

    rt.gateway_interrupt(&sid).await.unwrap();
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

    // 真相不靠 receptor：狀態事件照發，畫面才有機會自己更新。
    let states = session_states(&rt, &sid);
    assert!(
        states.iter().any(|s| s == "cancelled"),
        "cancelled 落地了卻沒有發出 agent.session.state：即時畫面會停在 working：{states:?}"
    );
    assert_eq!(
        states.last().map(String::as_str),
        Some("cancelled"),
        "cancelled 必須是使用者看到的最後一個狀態：{states:?}"
    );
    std::env::remove_var("INTERACT_AI_CODEX_BIN");
}

/// regression（F-…-agent-honesty-022）：緊急停止**不是**一次新的模型回合。
/// 舊版先把一則 `cancel` 送進信箱，而 `to-session` 的信箱訊息一律會被轉送
/// 進 agent 子程序的 stdin——停止這個動作因此自己對外開了一輪計費呼叫、
/// 發出 `fetched`（角色演成「工作中」）、還吃掉一格訊息預算，而且要等
/// stdin 寫入逾時之後才輪得到殺程序。現在只留 runtime 自己寫的稽核註記。
#[tokio::test]
async fn estop_does_not_send_a_user_turn() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;
    let dir = scenario_workdir("default");
    let mut input = claude_input("緊急停止不得開新回合", None);
    input.workdir = Some(dir.path().to_string_lossy().into_owned());
    let record = rt.create_agent_session(input).await.unwrap();
    let sid = record.session_id.as_str().to_string();

    let pid_path = dir.path().join("fake-pid");
    let stdin_log = dir.path().join("fake-input");
    let mut pid = 0;
    wait_for(
        async || {
            pid = read_pid(&pid_path);
            pid > 0
        },
        "fixture pid file",
    )
    .await;
    assert!(pid_alive(pid), "fixture alive before estop");

    rt.mailbox_send(
        &sid,
        MailboxDirection::ToSession,
        "task",
        BTreeMap::from([("task".to_string(), json!("看一下這個 repo"))]),
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

    let before_lines = std::fs::read_to_string(&stdin_log).unwrap();
    assert_eq!(
        before_lines.lines().count(),
        1,
        "只有那一則任務進過 agent 的 stdin：{before_lines}"
    );
    let before_states = session_states(&rt, &sid);
    let before_spent = rt
        .get_agent_session(&sid)
        .await
        .unwrap()
        .budget
        .spent_messages;

    rt.emergency_stop("test", None).await.unwrap();

    // 1) agent 子程序沒有被開第二個回合。
    let after_lines = std::fs::read_to_string(&stdin_log).unwrap();
    assert_eq!(
        after_lines.lines().count(),
        1,
        "緊急停止不得再送任何東西進 agent 的 stdin：{after_lines}"
    );
    assert!(
        !after_lines.contains("emergency stop"),
        "停止指令不得變成使用者發言：{after_lines}"
    );

    // 2) taxonomy：停止之後不得再多出 fetched／working（角色不得演成工作中）。
    let states = session_states(&rt, &sid);
    let fetched = |v: &[String]| v.iter().filter(|s| s.as_str() == "fetched").count();
    assert_eq!(
        fetched(&states),
        fetched(&before_states),
        "estop 不得再發 fetched：{states:?}"
    );
    assert!(
        !states[before_states.len()..]
            .iter()
            .any(|s| s == "working" || s == "fetched"),
        "停止之後只該有 cancelled：{states:?}"
    );
    assert_eq!(states.last().map(String::as_str), Some("cancelled"));

    // 3) 信箱：沒有偽裝成任務的 cancel；只有 runtime 自己寫的稽核註記。
    let to_session = rt
        .mailbox_peek(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    assert!(
        !to_session.iter().any(|m| m.kind == "cancel"),
        "緊急停止不得在 to-session 留下一則要送給 agent 的取消任務"
    );
    let note = rt
        .mailbox_peek(&sid, MailboxDirection::FromSession)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.kind == "emergency-stop")
        .expect("runtime 自己寫的緊急停止註記留在信箱裡供稽核");
    assert_eq!(note.body.get("by"), Some(&json!("runtime")));
    assert_eq!(note.body.get("deliveredToAgent"), Some(&json!(false)));
    assert!(
        note.delivered_at.is_none(),
        "runtime 的註記從來沒有送進 agent，不得蓋送達戳記"
    );

    // 4) 停止本身不花掉 session 的訊息預算。
    let stopped = rt.get_agent_session(&sid).await.unwrap();
    assert_eq!(stopped.budget.spent_messages, before_spent);
    assert_eq!(stopped.state, AgentSessionState::Cancelled);

    // 5) 停止之後不得再開新的一輪。
    let err = rt
        .mailbox_send(
            &sid,
            MailboxDirection::ToSession,
            "task",
            BTreeMap::from([("task".to_string(), json!("再來一輪"))]),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Conflict(_)), "{err:?}");

    wait_for(async || !pid_alive(pid), "estop kills the subprocess tree").await;
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// stdin 已經塞爆、agent 又不再讀的 session：對它寫入會一路卡到逾時。
/// 把管線灌滿，讓「送訊息給 agent」變成一件會卡住的事。
async fn fill_agent_stdin_until_it_blocks(rt: &Runtime, sid: &str) {
    let big = "x".repeat(4_000);
    for _ in 0..40u32 {
        if let Err(e) = rt
            .mailbox_send(
                sid,
                MailboxDirection::ToSession,
                "task",
                BTreeMap::from([("task".to_string(), json!(big))]),
                None,
            )
            .await
        {
            assert!(
                e.to_string().contains("無回應"),
                "前提是 stdin 被塞爆而寫入卡住，不是別的錯：{e}"
            );
            return;
        }
    }
    panic!("agent 的 stdin 沒有塞爆，這個測試的前提不成立");
}

/// regression（F-…-agent-honesty-022，延遲面）：緊急停止不得因為「先禮貌
/// 地通知每個 agent」而把終止排成一列。兩個卡死（stdin 塞爆、不再讀）的
/// session 曾經各要等一次 5 秒的寫入逾時才輪到殺程序，總共 ~10 秒；期間
/// 撤銷同意、EmergencyStop 事件與角色的緊急投影全都被拖著。現在每個
/// session 的收尾有界（2 秒）且彼此併行。
#[tokio::test]
async fn estop_terminates_sessions_concurrently() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;

    let mut dirs = Vec::new();
    let mut sids = Vec::new();
    let mut pids = Vec::new();
    for label in ["卡死一號", "卡死二號"] {
        let dir = scenario_workdir("hang");
        let mut input = claude_input(label, None);
        input.workdir = Some(dir.path().to_string_lossy().into_owned());
        input.max_messages = Some(60);
        let sid = rt
            .create_agent_session(input)
            .await
            .unwrap()
            .session_id
            .as_str()
            .to_string();
        let pid_path = dir.path().join("fake-pid");
        let mut pid = 0;
        wait_for(
            async || {
                pid = read_pid(&pid_path);
                pid > 0
            },
            "hung fixture pid file",
        )
        .await;
        sids.push(sid);
        pids.push(pid);
        dirs.push(dir);
    }

    // 兩個 session 同時灌爆 stdin，設定本身才不會被序列化。
    tokio::join!(
        fill_agent_stdin_until_it_blocks(&rt, &sids[0]),
        fill_agent_stdin_until_it_blocks(&rt, &sids[1]),
    );

    let started = std::time::Instant::now();
    rt.emergency_stop("test", None).await.unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(4),
        "兩個卡死的 session 不得把緊急停止排成一列（每個 session 上限 2 秒）：實際 {elapsed:?}"
    );

    for (sid, pid) in sids.iter().zip(pids.iter().copied()) {
        let stopped = rt.get_agent_session(sid).await.unwrap();
        assert_eq!(stopped.state, AgentSessionState::Cancelled, "{stopped:?}");
        wait_for(async || !pid_alive(pid), "hung subprocess tree killed").await;
    }
    assert_eq!(rt.open_agent_sessions().await, 0);
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 安全不變量：續開既有的 provider 對話**不得**放寬任何範圍。thread id 是
/// provider 端的東西，不是憑證：工作目錄、寫入權、tool／consent scope 與
/// session 能力一律由**這一次**的授權決定，絕不從上一個 session 繼承。
#[tokio::test]
async fn resume_cannot_widen_scope() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;

    // (1) 先有一個「可寫、掛在寬工作目錄、帶著領域授權」的 session。
    let wide = scenario_workdir("default");
    let mut writable = claude_input("可寫的前一輪", None);
    writable.workdir = Some(wide.path().to_string_lossy().into_owned());
    writable.allow_write = true;
    writable.tool_scope = vec!["workspace.write".into()];
    writable.consent_scope = vec!["agent-session:workspace-write".into()];
    writable.data_scope = vec!["domain:home-automation".into()];
    let first = rt.create_agent_session(writable).await.unwrap();
    let first_sid = first.session_id.as_str().to_string();
    let wide_argv = wide.path().join("fake-argv");
    wait_for(async || wide_argv.exists(), "writable argv log").await;
    assert!(
        std::fs::read_to_string(&wide_argv)
            .unwrap()
            .contains("acceptEdits"),
        "前一輪真的是可寫的，續開才有東西可繼承"
    );
    // 續開要比對的是上一次**實際**掛上子程序的工作目錄；等它真的落到記錄裡
    // （fixture 的 init 事件同時帶回 provider session id），比對才是確定性的。
    wait_for(
        async || {
            rt.get_agent_session(&first_sid)
                .await
                .map(|r| r.provider_session_id.as_deref() == Some("fake-123"))
                .unwrap_or(false)
        },
        "provider session id",
    )
    .await;
    rt.close_agent_session(&first_sid, None, "closed")
        .await
        .unwrap();
    std::fs::remove_file(&wide_argv).unwrap();
    std::fs::remove_file(wide.path().join("fake-pid")).unwrap();

    // (2) 續開同一個 provider 對話，但這一次什麼都沒授權。
    //     資料夾必須是**同一個**：換資料夾就是換範圍（見
    //     `resume_must_stay_in_the_recorded_workdir`），所以這裡只驗
    //     「同一個資料夾裡，權限旗標與 scope 一律重新上鎖」。
    let mut resumed = claude_input("續開不得放寬", None);
    resumed.workdir = Some(wide.path().to_string_lossy().into_owned());
    resumed.resume_provider_session_id = Some("fake-123".into());
    let second = rt.create_agent_session(resumed).await.unwrap();
    let sid = second.session_id.as_str().to_string();

    wait_for(async || wide_argv.exists(), "resumed argv log").await;
    let argv = std::fs::read_to_string(&wide_argv).unwrap();
    assert!(argv.contains("--resume fake-123"), "{argv}");
    assert!(
        argv.contains("--permission-mode plan"),
        "續開一律重新上鎖成唯讀：{argv}"
    );
    assert!(
        !argv.contains("acceptEdits") && !argv.contains("dangerously"),
        "續開不得繼承上一輪的寫入放寬：{argv}"
    );
    assert!(wide.path().join("fake-pid").exists());
    assert!(!second.allow_write, "續開不得繼承寫入權");
    assert!(second.tool_scope.is_empty(), "{:?}", second.tool_scope);
    assert!(
        second.consent_scope.is_empty(),
        "{:?}",
        second.consent_scope
    );
    assert!(second.data_scope.is_empty(), "{:?}", second.data_scope);

    // (3) session 能力（工具／領域）同樣不得繼承。
    let token = rt.issue_agent_session_capability(&sid).await.unwrap();
    let capability = rt
        .agent_session_capability(&token)
        .await
        .expect("續開的 session 有自己的能力");
    assert!(capability.tool_scope.is_empty());
    assert!(capability.domains.is_empty());
    assert!(!capability.allows_tool("workspace.write"));
    assert!(!capability.allows_domain(&["home-automation".to_string()]));

    // (4) 想在續開時把寫入權要回來，仍要走完整的人類同意（資料夾相同，
    //     被擋下的就只有「寫入權」這一個維度）。
    let mut sneaky = claude_input("續開偷渡寫入", None);
    sneaky.workdir = Some(wide.path().to_string_lossy().into_owned());
    sneaky.resume_provider_session_id = Some("fake-123".into());
    sneaky.allow_write = true;
    let err = rt.create_agent_session(sneaky).await.unwrap_err();
    assert!(matches!(err, DomainError::ConsentRequired(_)), "{err:?}");

    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 回歸（agent-honesty-022）：runtime 自己的狀態資料夾裡有人類 capability
/// token（`state/api-token`, 0600）。它**永遠不得**成為 agent 的工作目錄——
/// 子程序的 env 早就把 token 拿掉了，用檔案系統把同一把 token 送回去是同一
/// 個威脅換一條管道。沒指定資料夾時要落在專屬的空資料夾，不是 runtime home。
#[tokio::test]
async fn workdir_never_exposes_the_runtime_state_dir() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (home, rt) = runtime().await;

    // (1) 明確指定 runtime home（或任何包含 state/ 的上層資料夾）＝誠實拒絕。
    let mut into_home = claude_input("把家目錄當工作區", None);
    into_home.workdir = Some(home.path().to_string_lossy().into_owned());
    let err = rt.create_agent_session(into_home).await.unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    assert!(format!("{err}").contains("狀態資料夾"), "{err}");
    assert_eq!(rt.open_agent_sessions().await, 0, "沒有殘留 session");

    // (2) 沒指定資料夾（純對話）也不得退回 runtime home。
    //     在 runtime home 放一個 fixture 情境檔：子程序只要 cwd 真的是
    //     runtime home 就會讀到它並留下 fake-argv／fake-pid。
    std::fs::write(home.path().join("fake-mode"), "crash").unwrap();
    let record = rt
        .create_agent_session(claude_input("純對話", None))
        .await
        .expect("沒有資料夾的對話 session 仍可建立");
    let sid = record.session_id.as_str().to_string();
    assert!(
        !home.path().join("fake-argv").exists() && !home.path().join("fake-pid").exists(),
        "子程序的工作目錄不得是 runtime home（那裡有人類 token）"
    );
    assert!(
        home.path().join("state").is_dir(),
        "前提：狀態資料夾（正式環境的 api-token 就放這裡）真的在 runtime home 底下"
    );
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 回歸（agent-honesty-022）：「接續上次」不得放寬上一次的上限。省略欄位
/// **不是**沿用——那會落到 runtime 預設（120 分鐘、沒有金額上限），比上次寬。
#[tokio::test]
async fn resume_cannot_widen_time_or_cost_limits() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;

    // 上一次：30 分鐘、US$0.5、限定一個資料夾。
    let first_dir = scenario_workdir("default");
    let mut first = claude_input("原本的工作", Some(0.5));
    first.ttl_minutes = Some(30);
    first.workdir = Some(first_dir.path().to_string_lossy().into_owned());
    first.data_scope = vec![format!("workspace:{}", first_dir.path().display())];
    let original = rt.create_agent_session(first).await.unwrap();
    let first_sid = original.session_id.as_str().to_string();
    // fixture 的 init 事件回報 provider session id；接續就是靠它找回上一次。
    wait_for(
        async || {
            rt.get_agent_session(&first_sid)
                .await
                .map(|r| r.provider_session_id.is_some())
                .unwrap_or(false)
        },
        "provider session id",
    )
    .await;
    let provider_sid = rt
        .get_agent_session(&first_sid)
        .await
        .unwrap()
        .provider_session_id
        .unwrap();
    rt.close_agent_session(&first_sid, None, "closed")
        .await
        .unwrap();

    let resume_dir = scenario_workdir("default");
    let base_resume = |label: &str| {
        let mut input = claude_input(label, None);
        input.ttl_minutes = None;
        input.max_cost = None;
        input.max_messages = None;
        input.workdir = Some(resume_dir.path().to_string_lossy().into_owned());
        input.resume_provider_session_id = Some(provider_sid.clone());
        input
    };

    // (1) 桌面舊行為：什麼上限都不帶＝時間從 30 分鐘變 120 分鐘、費用上限消失。
    let err = rt
        .create_agent_session(base_resume("放寬時間"))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    assert!(format!("{err}").contains("時間上限"), "{err}");

    // (2) 時間帶對了，但費用上限沒帶＝沒有金額上限，一樣是放寬。
    let mut no_cost = base_resume("放寬費用");
    no_cost.ttl_minutes = Some(30);
    no_cost.max_messages = Some(10);
    let err = rt.create_agent_session(no_cost).await.unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    assert!(format!("{err}").contains("費用上限"), "{err}");

    // (3) 資料範圍也不得長出上次沒有的項目。
    let mut wider_scope = base_resume("放寬資料範圍");
    wider_scope.ttl_minutes = Some(30);
    wider_scope.max_cost = Some(0.5);
    wider_scope.max_messages = Some(10);
    wider_scope.data_scope = vec!["domain:health".into()];
    let err = rt.create_agent_session(wider_scope).await.unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    assert!(format!("{err}").contains("資料範圍"), "{err}");

    // (4) 上限都帶對了，`dataScope` 也照抄上次那個資料夾的標籤——但真正
    //     送出去的 `workdir` 是**另一個**資料夾。宣告與事實不一致：字面上
    //     什麼都沒放寬，實際上整個工作範圍換了一棵檔案樹。
    //     （v0.5.1 之前這個案例是 `.unwrap()` 成功的——既有測試把漏洞
    //     斷言成了預期行為。）
    let mut lying = base_resume("宣告與事實不一致");
    lying.ttl_minutes = Some(30);
    lying.max_cost = Some(0.5);
    lying.max_messages = Some(10);
    lying.data_scope = vec![format!("workspace:{}", first_dir.path().display())];
    let err = rt.create_agent_session(lying).await.unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    assert!(format!("{err}").contains("工作目錄"), "{err}");
    assert!(
        !resume_dir.path().join("fake-pid").exists(),
        "被拒絕的續開不得真的把子程序掛到另一個資料夾"
    );

    // (5) 誠實地沿用上次的上限**與上次那個資料夾**＝可以接續（仍然唯讀）。
    let mut faithful = base_resume("誠實接續");
    faithful.ttl_minutes = Some(30);
    faithful.max_cost = Some(0.5);
    faithful.max_messages = Some(10);
    faithful.workdir = Some(first_dir.path().to_string_lossy().into_owned());
    faithful.data_scope = vec![format!("workspace:{}", first_dir.path().display())];
    let resumed = rt.create_agent_session(faithful).await.unwrap();
    assert!(!resumed.allow_write);
    assert_eq!(resumed.budget.max_cost, 0.5);
    assert_eq!(resumed.budget.max_duration_ms, 30 * 60_000);
    assert_eq!(
        resumed.resolved_workdir.as_deref(),
        Some(
            first_dir
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    rt.close_agent_session(resumed.session_id.as_str(), None, "closed")
        .await
        .unwrap();
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 回歸（agent-honesty-024）：委派給 gateway session 時，訊息在 executor 還
/// 沒 merge driver receipt 之前就已經真的寫進子程序。receipt 必須推進到
/// acknowledged 並發出 `action.acknowledged`，不能永遠停在 dispatched。
#[tokio::test]
async fn delegated_gateway_receipt_reaches_acknowledged() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;
    let dir = scenario_workdir("default");
    let mut input = claude_input("委派目標", None);
    input.workdir = Some(dir.path().to_string_lossy().into_owned());
    let record = rt.create_agent_session(input).await.unwrap();
    let sid = record.session_id.as_str().to_string();

    rt.registry
        .set_actuator_enabled(&ActuatorId::new("agent.delegate"), true)
        .await
        .unwrap();
    rt.start_session(Some("human".into()), None, vec!["channel:agent".into()])
        .await
        .unwrap();
    let mut intent = SemanticIntent::new("delegate-work");
    intent.payload = Some(json!({"sessionId": sid, "task": "檢查這個專案"}));
    intent.preferred_channels = vec!["agent".into()];
    let plan = rt
        .create_plan(
            intent,
            vec!["agent.delegate".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let receipts = rt
        .execute_plan(
            &plan.plan_id,
            interaction_policy::ActionSource::ExplicitRequest,
            false,
        )
        .await
        .unwrap();
    let action_id = receipts[0].action_id.clone();
    assert_eq!(
        receipts[0].current_status,
        ActionStatus::Acknowledged,
        "訊息真的寫進 agent 子程序了：receipt 不得停在 dispatched"
    );
    let stored = rt.get_action(&action_id).unwrap();
    assert_eq!(stored.current_status, ActionStatus::Acknowledged);
    assert!(rt
        .events
        .recent(2000)
        .into_iter()
        .any(|e| e.event_type == EventType::ActionAcknowledged
            && e.payload.get("actionId").and_then(|v| v.as_str()) == Some(action_id.as_str())));

    // 送達戳記也要真的落在信箱訊息上（誠實：有戳記才敢說已送達）。
    let messages = rt
        .mailbox_peek(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    assert!(messages.iter().any(|m| m.delivered_at.is_some()));

    // 回歸（agent-honesty-025）：`mailbox_send` 的**回傳值**也要帶戳記——
    // 呼叫端（HTTP／Tauri／介面）只看得到這一份，沒有戳記就不得說「已送達」。
    let mut body = BTreeMap::new();
    body.insert("task".to_string(), json!("再交代一句"));
    let sent = rt
        .mailbox_send(&sid, MailboxDirection::ToSession, "task", body, None)
        .await
        .unwrap();
    assert!(
        sent.delivered_at.is_some(),
        "真的寫進子程序了，回傳的訊息必須帶送達戳記"
    );

    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 回歸（v0.5.1 / 已知限制 #18）：「接續上次」不得換工作目錄。
///
/// 上一次實際掛上子程序的資料夾是這個 session **真正的**授權範圍；它必須被
/// 記錄下來（`resolvedWorkdir`），續開時才有東西可比。宣告的 `dataScope`
/// 只是人話標籤，換一個 workdir 卻不動 scope，字面上「什麼都沒放寬」，
/// 實際上子程序被掛到另一棵檔案樹——那是換範圍，不是接續。
///
/// 這裡同時涵蓋 `A/../B`：字串前綴看起來還在 A 底下，canonicalize 之後其實
/// 是隔壁的 B。比對一律以正規化後的絕對路徑為準。
#[tokio::test]
async fn resume_must_stay_in_the_recorded_workdir() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;

    let parent = tempfile::tempdir().unwrap();
    let dir_a = parent.path().join("A");
    let dir_b = parent.path().join("B");
    for dir in [&dir_a, &dir_b] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("fake-mode"), "default").unwrap();
    }

    let mut first = claude_input("原本的工作", None);
    first.workdir = Some(dir_a.to_string_lossy().into_owned());
    let original = rt.create_agent_session(first).await.unwrap();
    let first_sid = original.session_id.as_str().to_string();
    wait_for(
        async || {
            rt.get_agent_session(&first_sid)
                .await
                .map(|r| r.provider_session_id.is_some())
                .unwrap_or(false)
        },
        "provider session id",
    )
    .await;
    let stored = rt.get_agent_session(&first_sid).await.unwrap();
    let provider_sid = stored.provider_session_id.clone().unwrap();
    // 真正掛上去的資料夾要留下記錄（正規化後的絕對路徑），否則續開時
    // 根本無從比對。
    assert_eq!(
        stored.resolved_workdir.as_deref(),
        Some(dir_a.canonicalize().unwrap().to_string_lossy().as_ref()),
        "實際掛上子程序的工作目錄必須寫進記錄"
    );
    rt.close_agent_session(&first_sid, None, "closed")
        .await
        .unwrap();

    // (1) `A/../B`：正規化之後是隔壁的資料夾＝換範圍，誠實拒絕。
    let mut traversal = claude_input("穿越到隔壁", None);
    traversal.workdir = Some(format!("{}/../B", dir_a.display()));
    traversal.resume_provider_session_id = Some(provider_sid.clone());
    let err = rt.create_agent_session(traversal).await.unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    assert!(format!("{err}").contains("工作目錄"), "{err}");
    assert!(
        !dir_b.join("fake-argv").exists() && !dir_b.join("fake-pid").exists(),
        "被拒絕的續開不得真的把子程序掛到另一個資料夾"
    );

    // (2) 完全不帶資料夾也是換範圍：後端會自己挑一個 scratch 目錄，
    //     那不是上一次那一個。
    let mut omitted = claude_input("不帶資料夾", None);
    omitted.resume_provider_session_id = Some(provider_sid.clone());
    let err = rt.create_agent_session(omitted).await.unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    assert!(format!("{err}").contains("工作目錄"), "{err}");

    // (3) 誠實地帶同一個資料夾（寫法不同、正規化後相同）才能接續。
    let mut faithful = claude_input("誠實接續", None);
    faithful.workdir = Some(format!("{}/./", dir_a.display()));
    faithful.resume_provider_session_id = Some(provider_sid);
    let resumed = rt.create_agent_session(faithful).await.unwrap();
    assert_eq!(
        resumed.resolved_workdir.as_deref(),
        Some(dir_a.canonicalize().unwrap().to_string_lossy().as_ref())
    );
    rt.close_agent_session(resumed.session_id.as_str(), None, "closed")
        .await
        .unwrap();
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 回歸（agent-honesty-022）：symlink 不得繞過「工作資料夾不是狀態資料夾」
/// 這道防線——比對走的是正規化後的路徑，所以一個名字人畜無害、指向
/// runtime `state/` 的連結一樣被擋。
///
/// 這條防線走 `DomainError::Validation`（建立時就拒絕），與續開的
/// `PolicyBlocked`（換工作目錄，見 `resume_must_stay_in_the_recorded_workdir`）
/// 是不同輸入觸發的兩道防線，彼此不遮蔽。
///
/// 這裡驗的是「workdir 是（或包含）state/」；「workdir 位在 state/ **底下**」由
/// `a_workdir_inside_the_runtime_state_dir_is_refused` 涵蓋（runtime 自己的
/// proactive 候選工作區因此搬到 `agent-workspaces/proactive`，不再住在 state/ 裡）。
#[tokio::test]
async fn a_symlink_cannot_smuggle_the_runtime_state_dir_in_as_a_workdir() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (home, rt) = runtime().await;

    let state = home.path().join("state");
    assert!(state.is_dir(), "前提：狀態資料夾真的在 runtime home 底下");
    let elsewhere = tempfile::tempdir().unwrap();
    let link = elsewhere.path().join("looks-innocent");
    std::os::unix::fs::symlink(&state, &link).unwrap();

    let mut input = claude_input("用連結躲進狀態資料夾", None);
    input.workdir = Some(link.to_string_lossy().into_owned());
    let err = rt.create_agent_session(input).await.unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    assert!(format!("{err}").contains("狀態資料夾"), "{err}");
    assert!(
        !state.join("fake-pid").exists() && !state.join("fake-argv").exists(),
        "子程序不得真的在狀態資料夾裡跑起來"
    );
    assert_eq!(rt.open_agent_sessions().await, 0, "沒有殘留 session");
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 反方向的同一道防線：workdir **位於** runtime 的 state/ 底下也不行——
/// 子程序只要往上走一層就能讀到 human `api-token`。之前 `resolve_gateway_workdir`
/// 只擋「是或包含 state/」，`state/agent-bait` 這種路徑會被接受。
#[tokio::test]
async fn a_workdir_inside_the_runtime_state_dir_is_refused() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (home, rt) = runtime().await;

    let state = home.path().join("state");
    assert!(state.is_dir(), "前提：狀態資料夾真的在 runtime home 底下");
    let bait = state.join("agent-bait");
    std::fs::create_dir_all(&bait).unwrap();

    let mut input = claude_input("躲進狀態資料夾底下", None);
    input.workdir = Some(bait.to_string_lossy().into_owned());
    let err = rt.create_agent_session(input).await.unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    assert!(format!("{err}").contains("狀態資料夾"), "{err}");
    assert!(
        !bait.join("fake-pid").exists() && !bait.join("fake-argv").exists(),
        "子程序不得真的在狀態資料夾底下跑起來"
    );
    assert_eq!(rt.open_agent_sessions().await, 0, "沒有殘留 session");
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 回歸（agent-honesty-022）：`toolScope: ["conversation.generate"]` 是
/// intent-only（「不讀檔、不使用工具」）的宣告。在 claude 它是確定性的
/// `--tools ""`（一個工具都不給）；codex 的 app-server（`thread/start`／
/// `thread/resume`）與 exec fallback 都沒有等價旗標——sandbox 擋得住寫入，
/// 擋不住讀檔與 shell，限制只剩 prompt 文字。安全由 Rust 確定性強制、
/// 不靠 prompt：codex 不得收下一個它執行不了的限制，必須誠實拒絕。
#[tokio::test]
async fn codex_refuses_intent_only_sessions_it_cannot_enforce() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CODEX_BIN", codex_fixture_path());
    let (_g, rt) = runtime().await;

    let dir = tempfile::tempdir().unwrap();
    let mut intent_only = codex_input("intent-only", dir.path());
    intent_only.tool_scope = vec!["conversation.generate".into()];
    let err = rt.create_agent_session(intent_only).await.unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    assert!(format!("{err}").contains("codex"), "{err}");
    assert_eq!(rt.open_agent_sessions().await, 0, "沒有半掛 session");
    assert!(
        !dir.path().join("fake-thread-start").exists() && !dir.path().join("fake-pid").exists(),
        "被拒絕的 session 不得真的起 codex 子程序"
    );
    std::env::remove_var("INTERACT_AI_CODEX_BIN");

    // 同一個宣告在 claude 上仍然成立，而且真的送出「零工具」旗標。
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let cdir = scenario_workdir("default");
    let mut claude = claude_input("intent-only", None);
    claude.workdir = Some(cdir.path().to_string_lossy().into_owned());
    claude.tool_scope = vec!["conversation.generate".into()];
    let record = rt.create_agent_session(claude).await.unwrap();
    let argv_file = cdir.path().join("fake-argv");
    wait_for(async || argv_file.exists(), "fixture argv log").await;
    let argv = std::fs::read_to_string(&argv_file).unwrap();
    assert!(argv.contains("--tools"), "{argv}");
    assert!(
        !argv.contains("Read,Glob,Grep"),
        "intent-only 不得拿到任何工具：{argv}"
    );
    rt.close_agent_session(record.session_id.as_str(), None, "closed")
        .await
        .unwrap();
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 回歸（agent-honesty-023）：續開一個 intent-only（零工具）的工作階段時，
/// 桌面／CLI 送的是空 `toolScope`。空集合在字面上是子集，卻讓 provider 的
/// 實際工具集從「一個都沒有」變成「可讀檔／glob／grep」——宣告更窄、實權
/// 更寬，正是「不得放寬」要擋的事。
#[tokio::test]
async fn resuming_an_intent_only_session_may_not_widen_the_real_tool_set() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (_g, rt) = runtime().await;
    let dir = scenario_workdir("default");
    let argv_file = dir.path().join("fake-argv");

    let mut first = claude_input("intent-only", None);
    first.workdir = Some(dir.path().to_string_lossy().into_owned());
    first.tool_scope = vec!["conversation.generate".into()];
    let first_record = rt.create_agent_session(first).await.unwrap();
    let first_sid = first_record.session_id.as_str().to_string();
    wait_for(
        async || {
            rt.get_agent_session(&first_sid)
                .await
                .map(|r| r.provider_session_id.as_deref() == Some("fake-123"))
                .unwrap_or(false)
        },
        "provider session id",
    )
    .await;
    assert!(
        !std::fs::read_to_string(&argv_file)
            .unwrap()
            .contains("Read,Glob,Grep"),
        "第一輪真的是零工具，續開才有東西可放寬"
    );
    rt.close_agent_session(&first_sid, None, "closed")
        .await
        .unwrap();
    std::fs::remove_file(&argv_file).unwrap();

    // (1) 空 toolScope 的續開＝把零工具換成可讀檔，必須拒絕。
    let mut widened = claude_input("續開放寬工具", None);
    widened.workdir = Some(dir.path().to_string_lossy().into_owned());
    widened.resume_provider_session_id = Some("fake-123".into());
    widened.tool_scope = vec![];
    let err = rt.create_agent_session(widened).await.unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    assert!(format!("{err}").contains("工具"), "{err}");
    assert!(!argv_file.exists(), "被拒絕的續開不得真的起子程序");

    // (2) 誠實地帶著同一個 intent-only 宣告續開才行，而且仍然是零工具。
    let mut faithful = claude_input("誠實續開", None);
    faithful.workdir = Some(dir.path().to_string_lossy().into_owned());
    faithful.resume_provider_session_id = Some("fake-123".into());
    faithful.tool_scope = vec!["conversation.generate".into()];
    let resumed = rt.create_agent_session(faithful).await.unwrap();
    wait_for(async || argv_file.exists(), "resumed argv log").await;
    let argv = std::fs::read_to_string(&argv_file).unwrap();
    assert!(argv.contains("--resume fake-123"), "{argv}");
    assert!(!argv.contains("Read,Glob,Grep"), "續開仍然是零工具：{argv}");
    rt.close_agent_session(resumed.session_id.as_str(), None, "closed")
        .await
        .unwrap();
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}

/// 回歸（agent-honesty-025）：續開時「找不到上一次的授權紀錄」比「找得到但
/// 缺工作目錄」更不確定——不確定就拒絕，不得整段跳過比對後落到 runtime
/// 預設（ttl 120 分鐘、沒有金額上限）並掛上任意資料夾。
#[tokio::test]
async fn resume_without_a_recorded_grant_is_refused() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var("INTERACT_AI_CLAUDE_BIN", fixture_path());
    let (home, rt) = runtime().await;

    // (1) 本 runtime 從來沒有發過的 provider thread id（例如使用者自己在
    //     終端跑 claude 拿到的）：我們不知道上次授權了什麼 ⇒ 拒絕。
    let dir = scenario_workdir("default");
    let mut unknown = claude_input("外部 thread", None);
    unknown.workdir = Some(dir.path().to_string_lossy().into_owned());
    unknown.resume_provider_session_id = Some("thread-from-somewhere-else".into());
    let err = rt.create_agent_session(unknown).await.unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    assert!(
        !dir.path().join("fake-argv").exists() && !dir.path().join("fake-pid").exists(),
        "被拒絕的續開不得真的起子程序"
    );
    assert_eq!(rt.open_agent_sessions().await, 0, "沒有半掛 session");

    // (2) 升級前建立的舊紀錄沒有 resolvedWorkdir，這次也沒帶資料夾：
    //     (None, None) 一樣無從證明沒換資料夾 ⇒ 拒絕。
    let seed = rt
        .create_agent_session({
            let mut input = claude_input("做一份模板", None);
            input.workdir = Some(dir.path().to_string_lossy().into_owned());
            input
        })
        .await
        .unwrap();
    let seed_sid = seed.session_id.as_str().to_string();
    wait_for(
        async || {
            rt.get_agent_session(&seed_sid)
                .await
                .map(|r| r.provider_session_id.is_some())
                .unwrap_or(false)
        },
        "provider session id",
    )
    .await;
    rt.close_agent_session(&seed_sid, None, "closed")
        .await
        .unwrap();
    let template = rt
        .store
        .all_agent_sessions()
        .unwrap()
        .into_iter()
        .next()
        .expect("seed record persisted");
    let mut legacy: serde_json::Value = serde_json::from_str(&template).unwrap();
    legacy["sessionId"] = json!("legacy-session-1");
    legacy["providerSessionId"] = json!("legacy-thread-1");
    legacy.as_object_mut().unwrap().remove("resolvedWorkdir");
    rt.store
        .save_agent_session("legacy-session-1", &legacy.to_string())
        .unwrap();

    // 重啟：restore 把舊紀錄載回記憶體，續開才找得到它。
    let rt2 = Runtime::start(RuntimeOptions {
        home: Some(home.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    let legacy_dir = scenario_workdir("default");
    let mut legacy_resume = claude_input("續開舊紀錄", None);
    legacy_resume.resume_provider_session_id = Some("legacy-thread-1".into());
    legacy_resume.workdir = None;
    let err = rt2.create_agent_session(legacy_resume).await.unwrap_err();
    assert!(matches!(err, DomainError::PolicyBlocked(_)), "{err:?}");
    assert!(format!("{err}").contains("工作目錄"), "{err}");
    assert!(
        !legacy_dir.path().join("fake-argv").exists(),
        "被拒絕的續開不得真的起子程序"
    );
    std::env::remove_var("INTERACT_AI_CLAUDE_BIN");
}
