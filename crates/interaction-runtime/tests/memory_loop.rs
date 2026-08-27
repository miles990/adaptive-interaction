//! 記憶層垂直閉環：分層 CRUD、actor 降權、保存期限三態、到期清除、
//! Context Bundle 確定性選擇（stale/敏感/不可見/候選排除）、
//! handoff 落地、secret 拒收、匯出。

use chrono::Utc;
use interaction_core::*;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;

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

fn item(layer: MemoryLayer, kind: MemoryKind, title: &str, actor: MemoryActor) -> MemoryItem {
    new_memory_item(
        layer,
        kind,
        title,
        format!("{title} 內容"),
        actor,
        Utc::now(),
    )
}

#[tokio::test]
async fn crud_layers_and_status() {
    let (_g, rt) = runtime().await;
    let created = rt
        .memory_create(item(
            MemoryLayer::UserMemory,
            MemoryKind::Preference,
            "喜歡深色主題",
            MemoryActor::Human,
        ))
        .await
        .unwrap();
    // 讀回 + 更新 + 列表。
    let got = rt.memory_get(created.memory_id.as_str()).await.unwrap();
    assert_eq!(got.title, "喜歡深色主題");
    rt.memory_update(created.memory_id.as_str(), json!({"title": "偏好深色主題"}))
        .await
        .unwrap();
    let listed = rt.memory_list(Some("user-memory"), 10).await.unwrap();
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["items"][0]["title"], "偏好深色主題");
    assert_eq!(listed["items"][0]["status"], "active");
    // 刪除：不存在使用者不可刪的記憶。
    assert!(rt.memory_delete(created.memory_id.as_str()).await.unwrap());
    assert!(rt.memory_get(created.memory_id.as_str()).await.is_err());
}

#[tokio::test]
async fn agent_writes_are_demoted_and_secrets_rejected() {
    let (_g, rt) = runtime().await;
    // agent 宣稱 fact → inference。
    let mut m = item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Fact,
        "agent 宣稱",
        MemoryActor::Agent("codex".into()),
    );
    m.retention = RetentionPolicy::default();
    let created = rt.memory_create(m).await.unwrap();
    assert_eq!(created.kind, MemoryKind::Inference);
    // agent 想寫長期使用者記憶 → candidate＋複查。
    let mut m = item(
        MemoryLayer::UserMemory,
        MemoryKind::Preference,
        "agent 猜的偏好",
        MemoryActor::Agent("claude-code".into()),
    );
    m.retention = RetentionPolicy::default();
    let created = rt.memory_create(m).await.unwrap();
    assert_eq!(created.kind, MemoryKind::Candidate);
    assert!(created.retention.review_after.is_some());
    // secret 樣態拒收。
    let mut m = item(
        MemoryLayer::UserMemory,
        MemoryKind::Fact,
        "危險",
        MemoryActor::Human,
    );
    m.content = "api_key = sk-abc".into();
    assert!(rt.memory_create(m).await.is_err());
}

#[tokio::test]
async fn expired_memories_are_pruned_by_sweep() {
    let (_g, rt) = runtime().await;
    let mut m = item(
        MemoryLayer::SessionContext,
        MemoryKind::Fact,
        "暫存",
        MemoryActor::Human,
    );
    m.retention.expires_at = Some(Utc::now() - chrono::Duration::minutes(1));
    // 直接持久化（繞過 create 的 now 重設）。
    rt.memory_create(m).await.unwrap();
    // create 重設了 created_at 但保留 retention → 已過期。
    let listed = rt.memory_list(None, 100).await.unwrap();
    assert_eq!(listed["items"][0]["status"], "expired");
    rt.sweep_memory().await;
    let listed = rt.memory_list(None, 100).await.unwrap();
    assert_eq!(listed["count"], 0, "到期記憶被清除");
}

#[tokio::test]
async fn far_future_horizon_cannot_escape_candidate_demotion() {
    let (_g, rt) = runtime().await;
    // 建立面：agent 給 100 年 reviewAfter 的使用者記憶 → 仍降候選＋30 天複查。
    let mut m = item(
        MemoryLayer::UserMemory,
        MemoryKind::Preference,
        "百年偏好",
        MemoryActor::Agent("codex".into()),
    );
    m.retention = RetentionPolicy {
        review_after: Some(Utc::now() + chrono::Duration::days(36500)),
        ..Default::default()
    };
    let created = rt.memory_create(m).await.unwrap();
    assert_eq!(
        created.kind,
        MemoryKind::Candidate,
        "遠期 horizon 一樣是長期"
    );
    assert!(
        created.retention.review_after.unwrap() <= Utc::now() + chrono::Duration::days(31),
        "reviewAfter 壓回 30 天上限"
    );
    // 更新面：PATCH kind／retention 不得解除降權（同一把 flat token 的側門）。
    let patched = rt
        .memory_update(
            created.memory_id.as_str(),
            json!({
                "kind": "preference",
                "retention": {"reviewAfter": Utc::now() + chrono::Duration::days(36500)}
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        patched.kind,
        MemoryKind::Candidate,
        "PATCH 不能解除候選降權"
    );
    assert!(patched.retention.review_after.unwrap() <= Utc::now() + chrono::Duration::days(31));
    // 其他層：agent 供給的 horizon 壓回層級預設（domain-knowledge 180 天）。
    let mut m = item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Inference,
        "百年知識",
        MemoryActor::Agent("codex".into()),
    );
    m.retention = RetentionPolicy {
        review_after: Some(Utc::now() + chrono::Duration::days(36500)),
        ..Default::default()
    };
    let created = rt.memory_create(m).await.unwrap();
    assert!(created.retention.review_after.unwrap() <= Utc::now() + chrono::Duration::days(181));
}

#[tokio::test]
async fn secrets_in_tags_and_provenance_are_rejected() {
    let (_g, rt) = runtime().await;
    let mut m = item(
        MemoryLayer::UserMemory,
        MemoryKind::Fact,
        "標籤夾帶",
        MemoryActor::Human,
    );
    m.tags = vec!["api_key=sk-abc".into()];
    assert!(rt.memory_create(m).await.is_err(), "tag 夾帶憑證樣態拒收");
    let mut m = item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Fact,
        "來源夾帶",
        MemoryActor::Human,
    );
    m.provenance = vec!["https://user:hunter2@internal.example/repo".into()];
    assert!(
        rt.memory_create(m).await.is_err(),
        "provenance 憑證 URL 拒收"
    );
    // 乾淨的 tag 與來源不受影響。
    let mut m = item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Fact,
        "乾淨",
        MemoryActor::Human,
    );
    m.tags = vec!["rust".into()];
    m.provenance = vec!["https://example.com/doc".into()];
    assert!(rt.memory_create(m).await.is_ok());
}

#[tokio::test]
async fn expired_memories_are_refused_before_sweep() {
    let (_g, rt) = runtime().await;
    let mut m = item(
        MemoryLayer::UserMemory,
        MemoryKind::Preference,
        "已到期",
        MemoryActor::Human,
    );
    m.retention.expires_at = Some(Utc::now() - chrono::Duration::minutes(1));
    let created = rt.memory_create(m).await.unwrap();
    let id = created.memory_id.as_str().to_string();
    // sweep 前 get 就視同已刪（與 sweep 後一致）：過期資料不得當有效供應。
    assert!(matches!(
        rt.memory_get(&id).await,
        Err(DomainError::NotFound(_))
    ));
    // PATCH 延長 expiresAt 不得讓過期項復活。
    let future = Utc::now() + chrono::Duration::days(7);
    assert!(
        rt.memory_update(&id, json!({"retention": {"expiresAt": future}}))
            .await
            .is_err(),
        "過期項不可經 PATCH 復活"
    );
    // 匯出仍包含（資料主權）但標記 expired，不冒充有效。
    let export = rt.memory_export().await.unwrap();
    assert_eq!(export["items"][0]["status"], "expired");
    // 刪除仍然合法（過期後唯一有效操作）。
    assert!(rt.memory_delete(&id).await.unwrap());
}

#[tokio::test]
async fn clear_session_context_clears_beyond_storage_page_limit() {
    let (_g, rt) = runtime().await;
    // 直接寫 store 造 >1000 筆（storage 單次列表上限），避免逐筆 create
    // 的 audit 開銷拖慢測試。
    let now = Utc::now();
    for i in 0..1005u32 {
        let m = new_memory_item(
            MemoryLayer::SessionContext,
            MemoryKind::Fact,
            format!("暫存 {i}"),
            "x",
            MemoryActor::Human,
            now,
        );
        let body = serde_json::to_string(&m).unwrap();
        rt.store
            .save_memory(
                m.memory_id.as_str(),
                "session-context",
                "fact",
                None,
                None,
                &body,
            )
            .unwrap();
    }
    let cleared = rt.memory_clear_session_context().await.unwrap();
    assert_eq!(cleared, 1005, "一次呼叫清完全部，不受 1000 分頁上限影響");
    let listed = rt.memory_list(Some("session-context"), 10).await.unwrap();
    assert_eq!(listed["count"], 0);
}

#[tokio::test]
async fn context_bundle_is_deterministic_and_honest() {
    let (_g, rt) = runtime().await;
    // 1) 可入 bundle 的 know-how（domain 命中）。
    let mut kh = item(
        MemoryLayer::DomainKnowHow,
        MemoryKind::KnowHow,
        "測試前先跑 clippy",
        MemoryActor::Human,
    );
    kh.tags = vec!["rust".into()];
    rt.memory_create(kh).await.unwrap();
    // 2) stale 的知識 → 不入，列 needsReview。
    let mut stale = item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Fact,
        "舊知識",
        MemoryActor::Human,
    );
    stale.tags = vec!["rust".into()];
    stale.retention.review_after = Some(Utc::now() - chrono::Duration::days(1));
    let stale_created = rt.memory_create(stale).await.unwrap();
    // 3) 敏感 tag → 排除計數。
    let mut sens = item(
        MemoryLayer::TaskMemory,
        MemoryKind::Fact,
        "敏感任務",
        MemoryActor::Human,
    );
    sens.tags = vec!["sensitive".into()];
    rt.memory_create(sens).await.unwrap();
    // 4) 使用者記憶預設不可見 → 排除計數。
    rt.memory_create(item(
        MemoryLayer::UserMemory,
        MemoryKind::Preference,
        "私人偏好",
        MemoryActor::Human,
    ))
    .await
    .unwrap();
    // 5) denylist 排除。
    let mut denied = item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Fact,
        "不給 codex",
        MemoryActor::Human,
    );
    denied.tags = vec!["rust".into()];
    denied.agent_denylist = vec!["codex".into()];
    rt.memory_create(denied).await.unwrap();
    // 6) candidate 不入。
    let mut cand = item(
        MemoryLayer::DomainKnowledge,
        MemoryKind::Candidate,
        "未複審",
        MemoryActor::Human,
    );
    cand.tags = vec!["rust".into()];
    rt.memory_create(cand).await.unwrap();

    let bundle = rt
        .memory_context_bundle("修 bug", &["rust".to_string()], "codex")
        .await
        .unwrap();
    let includes = bundle["includes"].as_array().unwrap();
    let titles: Vec<&str> = includes
        .iter()
        .filter_map(|i| i["title"].as_str())
        .collect();
    assert!(titles.contains(&"測試前先跑 clippy"));
    assert!(!titles.contains(&"舊知識"), "stale 不入 bundle");
    assert!(!titles.contains(&"敏感任務"));
    assert!(!titles.contains(&"私人偏好"));
    assert!(!titles.contains(&"不給 codex"), "denylist 生效");
    assert!(!titles.contains(&"未複審"), "candidate 不入 bundle");
    assert!(bundle["excluded"]["needsReview"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == stale_created.memory_id.as_str()));
    assert!(bundle["excluded"]["sensitive"].as_u64().unwrap() >= 1);
    assert!(bundle["excluded"]["notVisibleToAgent"].as_u64().unwrap() >= 1);

    // 同輸入同輸出（確定性）。
    let again = rt
        .memory_context_bundle("修 bug", &["rust".to_string()], "codex")
        .await
        .unwrap();
    assert_eq!(bundle["includes"], again["includes"]);
}

#[tokio::test]
async fn handoff_lands_in_memory_with_30d_retention() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let record = rt
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: None,
            agent_id: "agent.writer".into(),
            label: Some("寫作".into()),
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
    let handoff = HandoffSummary {
        task: "寫一篇初稿".into(),
        confirmed_facts: vec!["完成初稿".into()],
        inferences: vec![],
        decisions: vec!["採用大綱 B".into()],
        artifacts: vec![],
        permissions: vec![],
        remaining_work: vec![],
        risks: vec![],
    };
    rt.close_agent_session(record.session_id.as_str(), Some(handoff), "closed")
        .await
        .unwrap();
    let listed = rt.memory_list(Some("agent-handoff"), 10).await.unwrap();
    assert_eq!(listed["count"], 1);
    let item = &listed["items"][0];
    assert_eq!(item["kind"], "inference", "handoff 是 agent 聲稱");
    assert!(item["retention"]["expiresAt"].is_string(), "30 天到期");
    assert!(item["provenance"][0]
        .as_str()
        .unwrap()
        .starts_with("agent-session:"));
}
