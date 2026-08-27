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
