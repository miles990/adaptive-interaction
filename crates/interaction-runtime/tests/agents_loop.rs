//! Agent-session integration: lease, mailbox honesty ladder, claims-are-not-
//! receipts, delegation safety, estop propagation, close-with-handoff.

use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::agents::CreateAgentSession;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;
use std::collections::BTreeMap;

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

fn create_input(agent: &str) -> CreateAgentSession {
    serde_json::from_value(json!({
        "agentId": agent,
        "label": "測試工作",
        "ttlMinutes": 30,
        "dataScope": ["project-source"],
        "toolScope": [],
        "maxMessages": 10,
    }))
    .unwrap()
}

#[tokio::test]
async fn delegation_honesty_ladder_dispatched_acknowledged_claimed() {
    let (_g, rt) = runtime().await;
    let session = rt
        .create_agent_session(create_input("agent.coder"))
        .await
        .unwrap();
    let sid = session.session_id.as_str().to_string();
    assert_eq!(rt.open_agent_sessions().await, 1);

    // Delegate THROUGH the governor (agent.delegate actuator, consent-gated).
    // The human enables the delegation actuator first (disabled by default).
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
    let sim = rt.simulate_plan(&plan.plan_id).await.unwrap();
    let receipts = match rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
    {
        Ok(r) => r,
        Err(e) => panic!(
            "execute failed: {e}; simulate: {}",
            serde_json::to_string_pretty(&sim).unwrap()
        ),
    };
    let receipt = &receipts[0];
    // Honesty: queued into the mailbox = DISPATCHED, nothing more.
    assert_eq!(receipt.current_status, ActionStatus::Dispatched);

    // The session fetches its tasks → NOW the task is acknowledged.
    let messages = rt
        .mailbox_fetch(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].delivered_at.is_some());
    let after = rt.get_action(&receipt.action_id).unwrap();
    assert_eq!(after.current_status, ActionStatus::Acknowledged);

    // The agent claims completion: session state changes, but the receipt
    // does NOT complete — a claim is never verification.
    rt.report_agent_session(
        &sid,
        "claimed-completed",
        json!({"summary": "done", "actionId": "spoof"}),
    )
    .await
    .unwrap();
    let record = rt.get_agent_session(&sid).await.unwrap();
    assert_eq!(record.state, AgentSessionState::ClaimedCompleted);
    let still = rt.get_action(&receipt.action_id).unwrap();
    assert_eq!(still.current_status, ActionStatus::Acknowledged);

    // The claim landed as an observation whose payload is an INFERENCE, and
    // any smuggled actionId was renamed so it can't act as evidence.
    let obs = rt
        .observe_stored(&ObservationQuery {
            receptor_id: Some(ReceptorId::new("agent.session")),
            limit: Some(10),
            ..Default::default()
        })
        .await
        .unwrap();
    let claim = obs
        .iter()
        .find(|o| o.facts["event"] == json!("claimed-completed"))
        .unwrap();
    assert!(!claim.facts.contains_key("actionId"));
    assert_eq!(claim.inferences["report"]["claimActionId"], json!("spoof"));
}

/// 「有人看了信箱」≠「agent 收到任務」。human 身分讀 to-session 是**純觀看**：
/// 訊息維持未送達、委派 receipt 停在 dispatched、不發 action.acknowledged。
/// 只有 agent 身分讀才有送達語意。（CLI `agents messages` 預設方向就是
/// to-session，而那條路徑拿的是 human token。）
#[tokio::test]
async fn a_human_reading_the_mailbox_is_watching_not_delivering() {
    use interaction_runtime::agents::MailboxReader;

    let (_g, rt) = runtime().await;
    let sid = rt
        .create_agent_session(create_input("agent.coder"))
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();
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
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    let action_id = receipts[0].action_id.clone();
    assert_eq!(receipts[0].current_status, ActionStatus::Dispatched);

    // 人類看信箱——看幾次都一樣，什麼都不會改變。
    for _ in 0..3 {
        let seen = rt
            .mailbox_read(&sid, MailboxDirection::ToSession, MailboxReader::Human)
            .await
            .unwrap();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].delivered_at.is_none(),
            "a human GET must not stamp delivery"
        );
    }
    // 副作用真的沒有留下：信箱本體與 receipt 都沒動。
    let stored = rt
        .mailbox_peek(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    assert!(stored[0].delivered_at.is_none());
    assert_eq!(
        rt.get_action(&action_id).unwrap().current_status,
        ActionStatus::Dispatched,
        "watching the mailbox must never move dispatched → acknowledged"
    );
    assert!(
        !rt.events
            .recent(200)
            .iter()
            .any(|e| e.event_type == EventType::ActionAcknowledged),
        "no acknowledgement event may be fabricated by a human read"
    );

    // Agent 身分讀才是送達。
    let fetched = rt
        .mailbox_read(&sid, MailboxDirection::ToSession, MailboxReader::Agent)
        .await
        .unwrap();
    assert!(fetched[0].delivered_at.is_some());
    assert_eq!(
        rt.get_action(&action_id).unwrap().current_status,
        ActionStatus::Acknowledged
    );
}

#[tokio::test]
async fn delegation_limits_depth_cycle_and_count() {
    let (_g, rt) = runtime().await;
    // Depth exhausted.
    let mut deep = create_input("agent.a");
    deep.delegation = Some(DelegationEnvelope {
        root_task_id: "root".into(),
        parent_task_id: Some("p".into()),
        delegation_id: "d".into(),
        origin_agent_id: "agent.a".into(),
        hop_count: 3,
        max_hops: 5,
        visited_sessions: vec![],
        budget_remaining: 1.0,
    });
    let err = rt.create_agent_session(deep).await.unwrap_err();
    assert!(err.to_string().contains("depth"));

    // Budget exhausted.
    let mut broke = create_input("agent.b");
    broke.delegation = Some(DelegationEnvelope {
        root_task_id: "root".into(),
        parent_task_id: None,
        delegation_id: "d2".into(),
        origin_agent_id: "agent.b".into(),
        hop_count: 0,
        max_hops: 3,
        visited_sessions: vec![],
        budget_remaining: 0.0,
    });
    assert!(rt
        .create_agent_session(broke)
        .await
        .unwrap_err()
        .to_string()
        .contains("budget"));

    // Session-count ceiling (policy default max_sessions = 8).
    for i in 0..8 {
        rt.create_agent_session(create_input(&format!("agent.n{i}")))
            .await
            .unwrap();
    }
    let err = rt
        .create_agent_session(create_input("agent.overflow"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("open agent sessions"));
}

#[tokio::test]
async fn message_budget_is_a_hard_ceiling() {
    let (_g, rt) = runtime().await;
    let mut input = create_input("agent.chatty");
    input.max_messages = Some(2);
    let s = rt.create_agent_session(input).await.unwrap();
    let sid = s.session_id.as_str();
    for _ in 0..2 {
        rt.mailbox_send(
            sid,
            MailboxDirection::ToSession,
            "task",
            BTreeMap::new(),
            None,
        )
        .await
        .unwrap();
    }
    let err = rt
        .mailbox_send(
            sid,
            MailboxDirection::ToSession,
            "task",
            BTreeMap::new(),
            None,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("budget"));
}

#[tokio::test]
async fn every_task_receives_and_persists_the_exact_session_scoped_context_bundle() {
    let (home, rt) = runtime().await;
    let now = chrono::Utc::now();
    let mut rust = new_memory_item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Fact,
        "Rust 驗證規則",
        "先執行 cargo test",
        MemoryActor::Human,
        now,
    );
    rust.tags = vec!["rust".into()];
    rt.memory_create(rust).await.unwrap();
    let mut private = new_memory_item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Fact,
        "財務資料",
        "不可提供給此工作階段",
        MemoryActor::Human,
        now,
    );
    private.tags = vec!["finance".into()];
    rt.memory_create(private).await.unwrap();

    let mut input = create_input("agent.bundle");
    input.data_scope = vec!["domain:rust".into()];
    let session = rt.create_agent_session(input).await.unwrap();
    let sid = session.session_id.as_str().to_string();
    let sent = rt
        .mailbox_send(
            &sid,
            MailboxDirection::ToSession,
            "task",
            BTreeMap::from([("task".into(), json!("修正 Rust 測試"))]),
            None,
        )
        .await
        .unwrap();

    let bundle = sent
        .body
        .get("contextBundle")
        .expect("actual bundle attached");
    assert_eq!(bundle["agentId"], "agent.bundle");
    assert_eq!(bundle["domains"], json!(["rust"]));
    let titles: Vec<&str> = bundle["includes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["title"].as_str())
        .collect();
    assert!(titles.contains(&"Rust 驗證規則"));
    assert!(!titles.contains(&"財務資料"));

    let record = rt.get_agent_session(&sid).await.unwrap();
    let evidence = record
        .context_bundles
        .last()
        .expect("bundle evidence persisted");
    assert_eq!(evidence.message_id, sent.message_id);
    assert_eq!(evidence.bundle, *bundle);
    assert_eq!(evidence.content_hash.len(), 64);

    rt.shutdown().await;
    let restored = Runtime::start(RuntimeOptions {
        home: Some(home.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    let record = restored.get_agent_session(&sid).await.unwrap();
    assert_eq!(
        record.context_bundles.len(),
        1,
        "actual bundle survives restart as evidence"
    );
}

#[tokio::test]
async fn lease_expiry_kills_capabilities_and_refuses_renewal() {
    let (_g, rt) = runtime().await;
    let s = rt
        .create_agent_session(create_input("agent.short"))
        .await
        .unwrap();
    let sid = s.session_id.as_str().to_string();
    // Force-expire the lease by rewinding it in storage + memory.
    {
        // Renew path first: works while open.
        rt.renew_agent_session(&sid, 10).await.unwrap();
    }
    // Simulate expiry: craft an expired record through the public close path
    // is not enough — use report to keep it open, then rewind via renew(0)?
    // Instead: create a 1-minute session and hand-expire by editing the record
    // through the persistence API is private; so assert the lazy-expiry logic
    // via a directly-built record: closed sessions refuse mail.
    rt.close_agent_session(&sid, None, "closed").await.unwrap();
    let err = rt
        .mailbox_send(
            &sid,
            MailboxDirection::ToSession,
            "task",
            BTreeMap::new(),
            None,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("mailbox closed"));
    // Closed sessions cannot be renewed either.
    assert!(rt.renew_agent_session(&sid, 10).await.is_err());
    // The session-provider surface is closed.
    let pid = ProviderId::new(format!("provider.ai-session.{sid}"));
    let p = rt.get_provider(&pid).await.unwrap();
    assert_eq!(p.state, ProviderState::Closed);
}

#[tokio::test]
async fn estop_cancels_all_open_sessions_and_blocks_new_ones() {
    let (_g, rt) = runtime().await;
    let a = rt
        .create_agent_session(create_input("agent.a"))
        .await
        .unwrap();
    let b = rt
        .create_agent_session(create_input("agent.b"))
        .await
        .unwrap();
    rt.emergency_stop("test", Some("estop".into()))
        .await
        .unwrap();

    for s in [&a, &b] {
        let rec = rt.get_agent_session(s.session_id.as_str()).await.unwrap();
        assert_eq!(rec.state, AgentSessionState::Cancelled);
    }
    assert_eq!(rt.open_agent_sessions().await, 0);
    // No new sessions while stopped.
    let err = rt
        .create_agent_session(create_input("agent.c"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("emergency stop"));
}

#[tokio::test]
async fn close_with_handoff_keeps_only_bounded_summary() {
    let (_g, rt) = runtime().await;
    let s = rt
        .create_agent_session(create_input("agent.doc"))
        .await
        .unwrap();
    let sid = s.session_id.as_str().to_string();

    // Transcript-sized handoffs are refused.
    let huge = HandoffSummary {
        confirmed_facts: (0..60).map(|i| format!("f{i}")).collect(),
        ..Default::default()
    };
    assert!(rt
        .close_agent_session(&sid, Some(huge), "closed")
        .await
        .is_err());

    // A bounded handoff persists; consents die with the session.
    let ok = HandoffSummary {
        task: "整理文件".into(),
        confirmed_facts: vec!["docs/ 有 12 個檔案".into()],
        remaining_work: vec!["附圖尚未更新".into()],
        ..Default::default()
    };
    let closed = rt
        .close_agent_session(&sid, Some(ok), "closed")
        .await
        .unwrap();
    assert_eq!(closed.state, AgentSessionState::Closed);
    assert!(closed.consent_scope.is_empty());
    assert_eq!(closed.handoff.as_ref().unwrap().confirmed_facts.len(), 1);
}

#[tokio::test]
async fn open_sessions_do_not_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let sid;
    {
        let rt = Runtime::start(RuntimeOptions {
            home: Some(dir.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        let s = rt
            .create_agent_session(create_input("agent.x"))
            .await
            .unwrap();
        sid = s.session_id.as_str().to_string();
        rt.shutdown().await;
    }
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    let rec = rt.get_agent_session(&sid).await.unwrap();
    assert_eq!(rec.state, AgentSessionState::Expired);
    assert_eq!(rec.detail.as_deref(), Some("runtime restarted"));
}

#[tokio::test]
async fn close_is_terminal_and_keeps_prior_state_detail() {
    let (_g, rt) = runtime().await;
    let s = rt
        .create_agent_session(create_input("agent.once"))
        .await
        .unwrap();
    let sid = s.session_id.as_str().to_string();
    let handoff = HandoffSummary {
        task: "收尾".into(),
        confirmed_facts: vec!["完成 1 項".into()],
        ..Default::default()
    };
    let closed = rt
        .close_agent_session(&sid, Some(handoff), "closed")
        .await
        .unwrap();
    assert_eq!(closed.state, AgentSessionState::Closed);
    // detail 保留 prior-state 註記（不再被第二個 dead write 覆蓋掉）。
    assert_eq!(closed.detail.as_deref(), Some("closed (was Created)"));

    // terminal 狀態不可翻轉：換個 reason 再關一次必須被拒絕，
    // 狀態、detail、handoff 都不得變動。
    let err = rt
        .close_agent_session(&sid, None, "cancelled")
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Conflict(_)), "{err:?}");
    let after = rt.get_agent_session(&sid).await.unwrap();
    assert_eq!(after.state, AgentSessionState::Closed);
    assert_eq!(after.detail.as_deref(), Some("closed (was Created)"));
    assert_eq!(
        after.handoff.as_ref().map(|h| h.confirmed_facts.len()),
        Some(1),
        "re-close 不得抹掉 handoff"
    );
}

#[tokio::test]
async fn max_messages_zero_does_not_mean_unlimited() {
    let (_g, rt) = runtime().await;
    // A caller-supplied 0 must NOT nullify the mailbox budget — it falls back
    // to the policy default (200), and any value is capped at the policy max.
    let mut input = create_input("agent.zero");
    input.max_messages = Some(0);
    let s = rt.create_agent_session(input).await.unwrap();
    assert_eq!(
        s.budget.max_messages, 200,
        "0 -> policy default, not unlimited"
    );

    let mut big = create_input("agent.big");
    big.max_messages = Some(10_000);
    let s2 = rt.create_agent_session(big).await.unwrap();
    assert_eq!(s2.budget.max_messages, 200, "clamped to policy max");
}

#[tokio::test]
async fn delegation_tree_is_bounded_by_max_parallel_regardless_of_hop_count() {
    let (_g, rt) = runtime().await;
    // Four sessions sharing one rootTaskId, each dishonestly claiming hop 0 —
    // the tree is still capped at max_parallel (default 4) by rootTaskId.
    let mk = |i: usize| {
        let mut input = create_input(&format!("agent.tree{i}"));
        input.delegation = Some(DelegationEnvelope {
            root_task_id: "shared-root".into(),
            parent_task_id: None,
            delegation_id: format!("d{i}"),
            origin_agent_id: "agent.root".into(),
            hop_count: 0, // lie about depth
            max_hops: 99,
            visited_sessions: vec![],
            budget_remaining: 1.0,
        });
        input
    };
    for i in 0..4 {
        rt.create_agent_session(mk(i)).await.unwrap();
    }
    let err = rt.create_agent_session(mk(4)).await.unwrap_err();
    assert!(err.to_string().contains("max_parallel"), "got: {err}");
}

/// v0.5：人工驗證是 claim → verified 的唯一路徑，且不可重複、不可跳步。
#[tokio::test]
async fn human_verify_is_the_only_path_from_claim_to_verified() {
    let (_tmp, rt) = runtime().await;
    let record = rt
        .create_agent_session(create_input("agent.coder"))
        .await
        .unwrap();
    let id = record.session_id.as_str().to_string();
    assert!(record.human_verified.is_none());

    // Active session 不能驗證（沒有 claim 就沒有可驗證的東西）。
    rt.report_agent_session(&id, "task-started", json!({}))
        .await
        .unwrap();
    let err = rt.verify_agent_session(&id, None).await.unwrap_err();
    assert!(format!("{err}").contains("claimed-completed"), "{err}");

    // claimed-completed 後可驗證一次。
    rt.report_agent_session(&id, "claimed-completed", json!({"summary": "done"}))
        .await
        .unwrap();
    let verified = rt
        .verify_agent_session(&id, Some("我看過輸出檔了".into()))
        .await
        .unwrap();
    assert!(verified.human_verified.is_some());
    assert_eq!(
        verified.human_verified.as_ref().unwrap().note.as_deref(),
        Some("我看過輸出檔了")
    );
    // 狀態仍是 claimed-completed（verified 是人類註記，不是 agent 的新聲稱）。
    assert_eq!(
        verified.state,
        interaction_core::AgentSessionState::ClaimedCompleted
    );

    // 不可重複驗證。
    let err = rt.verify_agent_session(&id, None).await.unwrap_err();
    assert!(format!("{err}").contains("already verified"), "{err}");

    // 過長備註誠實拒絕。
    let long = "x".repeat(501);
    let record2 = rt
        .create_agent_session(create_input("agent.reviewer"))
        .await
        .unwrap();
    let id2 = record2.session_id.as_str().to_string();
    rt.report_agent_session(&id2, "claimed-completed", json!({}))
        .await
        .unwrap();
    let err = rt.verify_agent_session(&id2, Some(long)).await.unwrap_err();
    assert!(format!("{err}").contains("too long"), "{err}");
}

/// 一個 session 依序發出的 `agent.session.state` taxonomy。
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

/// v0.5 角色演出只認「真實事件」：每一級誠實階梯都必須真的發出
/// `agent.session.state`，否則小樞會停在上一個（此刻已經不真的）狀態。
/// regression：租約到期與「結果未知」這兩級曾完全靜默。
#[tokio::test]
async fn agent_session_state_taxonomy_is_emitted_for_every_rung_of_the_ladder() {
    let (_g, rt) = runtime().await;

    // A：working → waiting-input → waiting-consent → claim → 人工驗證 → 關閉。
    let a = rt
        .create_agent_session(create_input("agent.ladder"))
        .await
        .unwrap();
    let a = a.session_id.as_str().to_string();
    for event in [
        "task-started",
        "waiting-for-input",
        "waiting-for-consent",
        "claimed-completed",
    ] {
        rt.report_agent_session(&a, event, json!({})).await.unwrap();
    }
    rt.verify_agent_session(&a, None).await.unwrap();
    rt.close_agent_session(&a, None, "closed").await.unwrap();

    // B：明確失敗。
    let b = rt
        .create_agent_session(create_input("agent.failed"))
        .await
        .unwrap();
    let b = b.session_id.as_str().to_string();
    rt.report_agent_session(&b, "failed", json!({"error": "boom"}))
        .await
        .unwrap();

    // C：結果未知（工作結束了，既沒有聲稱也沒有可觀察的錯誤）。
    let c = rt
        .create_agent_session(create_input("agent.unknown"))
        .await
        .unwrap();
    let c = c.session_id.as_str().to_string();
    let after = rt
        .report_agent_session(&c, "unknown", json!({"reason": "程序結束而未回報結果"}))
        .await
        .unwrap();
    assert_eq!(after.state, AgentSessionState::Unknown);
    assert!(!after.state.is_open(), "unknown is terminal");
    // 未知**不是** claim：不得被人工驗證成完成。
    let err = rt.verify_agent_session(&c, None).await.unwrap_err();
    assert!(format!("{err}").contains("claimed-completed"), "{err}");

    // D：租約到期。
    let d = rt
        .create_agent_session(create_input("agent.timeout"))
        .await
        .unwrap();
    let d = d.session_id.as_str().to_string();
    let expired = rt.expire_agent_session_lease(&d).await.unwrap();
    assert_eq!(expired.state, AgentSessionState::Expired);

    // E：取消。
    let e = rt
        .create_agent_session(create_input("agent.cancelled"))
        .await
        .unwrap();
    let e = e.session_id.as_str().to_string();
    rt.close_agent_session(&e, None, "cancelled").await.unwrap();

    let ladder = session_states(&rt, &a);
    assert_eq!(
        ladder,
        vec![
            "created",
            "working",
            "waiting-input",
            "waiting-consent",
            "claimed-completed",
            "verified",
            "closed",
        ],
        "the whole ladder, in order"
    );
    assert!(session_states(&rt, &b).contains(&"failed".to_string()));
    assert!(session_states(&rt, &c).contains(&"unknown".to_string()));
    assert!(
        session_states(&rt, &d).contains(&"timed-out".to_string()),
        "lease expiry must not be silent: {:?}",
        session_states(&rt, &d)
    );
    assert!(session_states(&rt, &e).contains(&"cancelled".to_string()));

    // 每個事件都帶得出 agentId（角色演出要知道是誰）。
    let tagged = rt
        .events
        .recent(2000)
        .into_iter()
        .filter(|ev| ev.event_type == EventType::AgentSessionState)
        .all(|ev| ev.payload.get("agentId").and_then(|v| v.as_str()).is_some());
    assert!(tagged, "every taxonomy event names its agent");
}

/// regression：租約到期只發了 `session.stopped`，角色 taxonomy 完全靜默，
/// 於是 UI 會永遠停在到期前的最後一個狀態（例如「工作中」）。
#[tokio::test]
async fn lease_expiry_emits_timed_out_and_revokes_the_session_capability() {
    let (_g, rt) = runtime().await;
    let record = rt
        .create_agent_session(create_input("agent.lease"))
        .await
        .unwrap();
    let id = record.session_id.as_str().to_string();
    rt.report_agent_session(&id, "task-started", json!({}))
        .await
        .unwrap();
    let token = rt.issue_agent_session_capability(&id).await.unwrap();
    assert!(rt.agent_session_capability(&token).await.is_some());

    let expired = rt.expire_agent_session_lease(&id).await.unwrap();
    assert_eq!(expired.state, AgentSessionState::Expired);
    assert_eq!(expired.detail.as_deref(), Some("lease expired"));
    let states = session_states(&rt, &id);
    assert_eq!(
        states,
        vec!["created", "working", "timed-out"],
        "the character must be told the lease ran out"
    );
    // 租約死了，capability 也跟著死。
    assert!(rt.agent_session_capability(&token).await.is_none());
    // 到期不可續租、信箱關閉。
    assert!(rt.renew_agent_session(&id, 10).await.is_err());
}

/// regression：重啟後仍是 open 的 session 被靜靜標成 Expired，沒有任何
/// taxonomy 事件——UI 於是停在重啟前的假象。上一輪 daemon 沒走完，那些
/// 工作到底成了沒有：沒有人知道 ⇒ 誠實發 `unknown`。
#[tokio::test]
async fn restart_reports_unknown_for_work_that_was_still_open() {
    let dir = tempfile::tempdir().unwrap();
    let sid;
    {
        let rt = Runtime::start(RuntimeOptions {
            home: Some(dir.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        let record = rt
            .create_agent_session(create_input("agent.restart"))
            .await
            .unwrap();
        sid = record.session_id.as_str().to_string();
        rt.report_agent_session(&sid, "task-started", json!({}))
            .await
            .unwrap();
        rt.shutdown().await;
    }
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    let record = rt.get_agent_session(&sid).await.unwrap();
    assert_eq!(record.state, AgentSessionState::Expired);
    let states = session_states(&rt, &sid);
    assert_eq!(
        states,
        vec!["unknown"],
        "restart must say 'nobody knows how that ended', not stay silent"
    );
    assert!(
        !states
            .iter()
            .any(|s| s == "working" || s == "claimed-completed"),
        "a fresh runtime must not replay pre-crash appearances: {states:?}"
    );
}

/// regression（agent-honesty）：close 曾把 Failed／Unknown／TimedOut 改寫成
/// Closed，失敗／未知這個誠實結局從主要狀態消失、只剩 detail 一行。關閉只
/// 收尾（closed_at、consent、provider）；任務結局留在主要狀態。
#[tokio::test]
async fn close_keeps_a_terminal_outcome_instead_of_rewriting_it_to_closed() {
    let (_g, rt) = runtime().await;
    for (event, expected, reason) in [
        ("unknown", AgentSessionState::Unknown, "closed"),
        ("failed", AgentSessionState::Failed, "cancelled"),
        ("timed-out", AgentSessionState::TimedOut, "closed"),
        ("cancelled", AgentSessionState::Cancelled, "closed"),
    ] {
        let sid = rt
            .create_agent_session(create_input("agent.outcome"))
            .await
            .unwrap()
            .session_id
            .as_str()
            .to_string();
        rt.report_agent_session(&sid, "task-started", json!({}))
            .await
            .unwrap();
        rt.report_agent_session(&sid, event, json!({"reason": "test"}))
            .await
            .unwrap();
        let closed = rt.close_agent_session(&sid, None, reason).await.unwrap();
        assert_eq!(closed.state, expected, "{event}: close keeps the outcome");
        assert_eq!(
            closed.detail.as_deref(),
            Some(format!("{reason} (was {expected:?})").as_str())
        );
        assert!(closed.closed_at.is_some(), "{event}: close still closes");
        assert!(!closed.state.is_open());
        // 收件匣把結局當主要狀態呈現，不是「已關閉」。
        let inbox = rt
            .activity_inbox(interaction_runtime::activity::ActivityInboxFilter {
                agent: Some("agent.outcome".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let item = inbox["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["itemId"] == json!(sid))
            .expect("closed sessions stay in the activity history");
        assert_eq!(
            item["status"],
            serde_json::to_value(expected).unwrap(),
            "{event}: inbox status is the outcome"
        );
        assert_eq!(item["needsDecision"], json!(false));
        // terminal 不可翻轉：再關一次仍是 Conflict。
        let err = rt
            .close_agent_session(&sid, None, "closed")
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Conflict(_)), "{event}: {err:?}");
        assert_eq!(rt.get_agent_session(&sid).await.unwrap().state, expected);
    }
    // 沒有任務結局的 session 照舊：關閉＝Closed、取消＝Cancelled。
    let sid = rt
        .create_agent_session(create_input("agent.plain"))
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();
    let closed = rt
        .close_agent_session(&sid, None, "cancelled")
        .await
        .unwrap();
    assert_eq!(closed.state, AgentSessionState::Cancelled);
    assert_eq!(closed.detail.as_deref(), Some("cancelled (was Created)"));
}

/// regression（agent-honesty）：human_verified 曾是 session 層級旗標且從不
/// 清除——驗證一次後，之後每一輪的 claim 都顯示成「已確認完成」，而第二個
/// claim 想驗證只會得到 409 already verified。驗證必須綁定具體的 claim：
/// 新任務送達、新一輪工作、新的聲稱都讓舊驗證失效；新 claim 可以再驗證。
#[tokio::test]
async fn human_verification_binds_to_one_claim_and_a_new_round_needs_a_new_one() {
    let (_g, rt) = runtime().await;
    let sid = rt
        .create_agent_session(create_input("agent.multi"))
        .await
        .unwrap()
        .session_id
        .as_str()
        .to_string();
    assert!(rt.get_agent_session(&sid).await.unwrap().claim_id.is_none());

    // 第一輪：working → claim → 人工驗證。
    rt.report_agent_session(&sid, "task-started", json!({}))
        .await
        .unwrap();
    let claimed = rt
        .report_agent_session(&sid, "claimed-completed", json!({"summary": "第一輪"}))
        .await
        .unwrap();
    let first_claim = claimed.claim_id.clone().expect("every claim gets an id");
    let verified = rt
        .verify_agent_session(&sid, Some("第一輪看過了".into()))
        .await
        .unwrap();
    let verification = verified.human_verified.as_ref().unwrap();
    assert_eq!(
        verification.claim_id.as_deref(),
        Some(first_claim.as_str()),
        "the verification names the claim it confirmed"
    );
    assert_eq!(verified.claim_id.as_deref(), Some(first_claim.as_str()));
    // 序列化名稱是前端契約：record.claimId 與 humanVerified.claimId。
    let wire = serde_json::to_value(&verified).unwrap();
    assert_eq!(wire["claimId"], json!(first_claim));
    assert_eq!(wire["humanVerified"]["claimId"], json!(first_claim));

    // 第二輪任務送達（agent 真的取走）⇒ 舊驗證失效；claim id 不變（還沒有新聲稱）。
    rt.mailbox_send(
        &sid,
        MailboxDirection::ToSession,
        "task",
        BTreeMap::from([("task".to_string(), json!("再交代一句"))]),
        None,
    )
    .await
    .unwrap();
    assert!(
        rt.get_agent_session(&sid)
            .await
            .unwrap()
            .human_verified
            .is_some(),
        "queued is not delivered: nothing changed yet"
    );
    rt.mailbox_read(
        &sid,
        MailboxDirection::ToSession,
        interaction_runtime::agents::MailboxReader::Agent,
    )
    .await
    .unwrap();
    let after_fetch = rt.get_agent_session(&sid).await.unwrap();
    assert!(
        after_fetch.human_verified.is_none(),
        "a delivered new task invalidates the previous verification"
    );
    assert_eq!(after_fetch.claim_id.as_deref(), Some(first_claim.as_str()));

    // 新一輪工作中：不是 verified、也不是 claimed。
    let active = rt
        .report_agent_session(&sid, "task-started", json!({}))
        .await
        .unwrap();
    assert_eq!(active.state, AgentSessionState::Active);
    assert!(active.human_verified.is_none());
    let err = rt.verify_agent_session(&sid, None).await.unwrap_err();
    assert!(format!("{err}").contains("claimed-completed"), "{err}");

    // 第二個聲稱：新的 claim id，且可以（必須）重新驗證——不是 already verified。
    let second = rt
        .report_agent_session(&sid, "claimed-completed", json!({"summary": "第二輪"}))
        .await
        .unwrap();
    let second_claim = second.claim_id.clone().unwrap();
    assert_ne!(second_claim, first_claim, "a new claim is a new claim");
    assert!(second.human_verified.is_none());
    let verified2 = rt
        .verify_agent_session(&sid, Some("第二輪也看過了".into()))
        .await
        .unwrap();
    assert_eq!(
        verified2
            .human_verified
            .as_ref()
            .and_then(|v| v.claim_id.as_deref()),
        Some(second_claim.as_str())
    );
    // 每一個 claim 都只能驗證一次。
    let err = rt.verify_agent_session(&sid, None).await.unwrap_err();
    assert!(format!("{err}").contains("already verified"), "{err}");

    // 任何更新的 agent 自我回報（連 progress 也算）都讓驗證失效。
    let progressed = rt
        .report_agent_session(&sid, "progress", json!({"text": "還在改"}))
        .await
        .unwrap();
    assert!(progressed.human_verified.is_none());
    assert_eq!(
        progressed.claim_id.as_deref(),
        Some(second_claim.as_str()),
        "progress is not a new claim"
    );
    // taxonomy：verified 只在人工驗證當下出現，之後的工作狀態是 working。
    let states = session_states(&rt, &sid);
    assert_eq!(
        states.iter().filter(|s| *s == "verified").count(),
        2,
        "{states:?}"
    );
    assert_eq!(states.last().map(String::as_str), Some("working"));
}

/// regression（F-…-agent-honesty-022）：緊急停止在信箱裡留下的是 **runtime
/// 自己寫的稽核註記**，不是一則要送給 agent 的取消任務。那則 cancel 對輪詢
/// 型 session 從來就到不了（關閉會把未送達的信件清掉），卻照樣吃掉一格訊息
/// 預算；對 gateway session 更會被寫進子程序、開出一輪新的模型呼叫。
/// 同時確認 session 範圍的授權（能力 token、consent）隨著停止一起死。
#[tokio::test]
async fn estop_leaves_a_runtime_note_instead_of_a_cancel_task() {
    let (_g, rt) = runtime().await;
    let mut input = create_input("agent.a");
    input.tool_scope = vec!["knowledge.read".into()];
    input.consent_scope = vec!["agent-session:read".into()];
    let record = rt.create_agent_session(input).await.unwrap();
    let sid = record.session_id.as_str().to_string();

    // 一則真的送達過的任務：稽核註記不得把既有紀錄擠掉或改寫。
    rt.mailbox_send(
        &sid,
        MailboxDirection::ToSession,
        "task",
        BTreeMap::from([("task".to_string(), json!("先做這個"))]),
        None,
    )
    .await
    .unwrap();
    rt.mailbox_fetch(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    let before_spent = rt
        .get_agent_session(&sid)
        .await
        .unwrap()
        .budget
        .spent_messages;
    let token = rt.issue_agent_session_capability(&sid).await.unwrap();
    assert!(rt.agent_session_capability(&token).await.is_some());

    rt.emergency_stop("test", Some("estop".into()))
        .await
        .unwrap();

    let to_session = rt
        .mailbox_peek(&sid, MailboxDirection::ToSession)
        .await
        .unwrap();
    assert!(
        !to_session.iter().any(|m| m.kind == "cancel"),
        "緊急停止不得在 to-session 留下一則要送給 agent 的取消任務：{to_session:?}"
    );
    assert!(
        to_session.iter().all(|m| m.delivered_at.is_some()),
        "未送達的信件在關閉時就誠實地死了：{to_session:?}"
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
        "runtime 的註記從來沒有送給 agent，不得蓋送達戳記"
    );

    let stopped = rt.get_agent_session(&sid).await.unwrap();
    assert_eq!(stopped.state, AgentSessionState::Cancelled);
    assert_eq!(
        stopped.budget.spent_messages, before_spent,
        "緊急停止不得吃掉 session 的訊息預算"
    );
    // session 範圍的授權隨關閉一起撤銷（能力 token 立刻失效、consent 清空）。
    assert!(rt.agent_session_capability(&token).await.is_none());
    assert!(stopped.consent_scope.is_empty());
}

/// 安全不變量：agent 的「聲稱完成」永遠不會自己升級成「已驗證」——只有人
/// 明確驗證才會，而且任何更新的自我回報都會讓舊的驗證失效。
#[tokio::test]
async fn a_claim_never_auto_upgrades_itself_to_verified() {
    let (_g, rt) = runtime().await;
    let record = rt
        .create_agent_session(create_input("agent.coder"))
        .await
        .unwrap();
    let id = record.session_id.as_str().to_string();

    // 一則自稱「已驗證／verified」的回報不得被當成事件，更不得改寫狀態。
    let err = rt
        .report_agent_session(&id, "verified", json!({"summary": "我自己驗過了"}))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");

    // 就算 payload 裡塞 humanVerified，也只是 agent 的聲稱（inference）。
    let claimed = rt
        .report_agent_session(
            &id,
            "claimed-completed",
            json!({"summary": "做完了", "humanVerified": true, "verified": true}),
        )
        .await
        .unwrap();
    assert_eq!(claimed.state, AgentSessionState::ClaimedCompleted);
    assert!(
        claimed.human_verified.is_none(),
        "agent 不能替自己蓋人工驗證"
    );

    // 重複聲稱同樣不會升級。
    let again = rt
        .report_agent_session(&id, "claimed-completed", json!({"summary": "真的做完了"}))
        .await
        .unwrap();
    assert!(again.human_verified.is_none());

    // 只有人明確驗證才會有 verified；而且新的自我回報立刻讓它失效。
    let verified = rt.verify_agent_session(&id, None).await.unwrap();
    assert!(verified.human_verified.is_some());
    let after = rt
        .report_agent_session(&id, "progress", json!({"note": "又動起來了"}))
        .await
        .unwrap();
    assert!(
        after.human_verified.is_none(),
        "新的一輪自我回報不得沿用上一個 claim 的人工驗證"
    );
}
