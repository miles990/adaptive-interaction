//! 角色記憶分層（spec §10／§15）：小樞的記憶由 Adaptive Interaction 管理，
//! 不綁定任何 Agent 的 thread／session——換 Agent 人格與記憶不變。
//!
//! 誠實與隱私規則（資料模型層強制）：
//! - fact／inference／preference／know-how 分開標示，永不混淆。
//! - 不存在使用者不可刪除的永久記憶：「永久」只是 until-deleted。
//! - Secret／Token／Consent 永不入記憶（validate 拒絕明顯樣態；上層另有閘）。
//! - 保存期限三態：expiresAt（到期停用並刪）、reviewAfter（過期變 stale，
//!   使用前需重新確認）、until-deleted（直到使用者刪除或被版本取代）。

use crate::{MemoryId, Timestamp, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};

/// 記憶分層（spec §10 的層級）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryLayer {
    /// Persona／Character Core：角色核心人格（版本化；改動需使用者確認）。
    PersonaCore,
    /// 角色自身的經歷記憶（熟悉度、里程碑；只影響呈現）。
    CharacterMemory,
    /// 關於使用者的記憶（偏好、糾正；敏感內容預設不保存）。
    UserMemory,
    /// 世界觀設定（資料性、可版本化）。
    WorldKnowledge,
    /// 領域知識（概念、規則、術語）。
    DomainKnowledge,
    /// 領域 Know-how（怎麼做得好、失敗模式、驗證方式）。
    DomainKnowHow,
    /// 可執行的結構化工作流程。
    Skill,
    /// 任務記憶（目標、決策、結果；90 天後摘要歸檔）。
    TaskMemory,
    /// 目前對話暫存（session 結束或 24h 清除）。
    SessionContext,
    /// Agent Handoff（bounded 摘要，30 天）。
    AgentHandoff,
}

/// 內容的認識論身分：事實≠推論≠偏好≠方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryKind {
    Fact,
    Inference,
    Preference,
    KnowHow,
    /// 候選：等待複審才能升為正式內容（agent 寫入的預設歸宿）。
    Candidate,
}

/// 誰建立的（權限規則依此判斷，不信任自稱）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "id")]
pub enum MemoryActor {
    Human,
    Agent(String),
    Runtime,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct RetentionPolicy {
    /// 到期後停止使用並依政策刪除。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Timestamp>,
    /// 過此時間仍保留但標記 stale，使用前重新確認。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_after: Option<Timestamp>,
    /// 隨父素材刪除（多模態衍生物）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_with_parent: Option<String>,
}

/// 依 spec §15 預設表給各層預設保存政策。
pub fn default_retention(layer: MemoryLayer, now: Timestamp) -> RetentionPolicy {
    let days = |d: i64| Some(now + chrono::Duration::days(d));
    let hours = |h: i64| Some(now + chrono::Duration::hours(h));
    match layer {
        MemoryLayer::SessionContext => RetentionPolicy {
            expires_at: hours(24),
            ..Default::default()
        },
        MemoryLayer::AgentHandoff => RetentionPolicy {
            expires_at: days(30),
            ..Default::default()
        },
        MemoryLayer::TaskMemory => RetentionPolicy {
            review_after: days(90),
            ..Default::default()
        },
        MemoryLayer::UserMemory => RetentionPolicy {
            // 臨時偏好 30 天複查；明確長期偏好由呼叫端改為 until-deleted。
            review_after: days(30),
            ..Default::default()
        },
        MemoryLayer::PersonaCore | MemoryLayer::WorldKnowledge | MemoryLayer::Skill => {
            RetentionPolicy::default() // until-deleted（版本化）
        }
        MemoryLayer::CharacterMemory => RetentionPolicy {
            review_after: days(180),
            ..Default::default()
        },
        MemoryLayer::DomainKnowledge | MemoryLayer::DomainKnowHow => RetentionPolicy {
            review_after: days(180),
            ..Default::default()
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryStatus {
    Active,
    /// 過了 reviewAfter：仍在，但使用前要重新確認（不入 Context Bundle）。
    Stale,
    /// 過了 expiresAt：停止使用，等待清除。
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemoryItem {
    pub memory_id: MemoryId,
    pub layer: MemoryLayer,
    pub kind: MemoryKind,
    pub title: String,
    pub content: String,
    pub created_by: MemoryActor,
    /// 來源指引（observation id／action id／URL——可稽核）。
    #[serde(default)]
    pub provenance: Vec<String>,
    /// 0..1（inference 的可信度；fact 由建立者背書）。
    pub confidence: f64,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 允許看到此記憶的 agent id；空 = 依層級預設規則。
    #[serde(default)]
    pub agent_visibility: Vec<String>,
    /// 明確禁止的 agent id（優先於 visibility）。
    #[serde(default)]
    pub agent_denylist: Vec<String>,
    pub retention: RetentionPolicy,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<Timestamp>,
    pub schema_version: String,
}

impl MemoryItem {
    pub fn status(&self, now: Timestamp) -> MemoryStatus {
        if let Some(exp) = self.retention.expires_at {
            if now >= exp {
                return MemoryStatus::Expired;
            }
        }
        if let Some(review) = self.retention.review_after {
            if now >= review {
                return MemoryStatus::Stale;
            }
        }
        MemoryStatus::Active
    }

    /// agent 是否可見（denylist 優先；visibility 空＝敏感層預設不可見）。
    pub fn visible_to_agent(&self, agent_id: &str) -> bool {
        if self.agent_denylist.iter().any(|a| a == agent_id) {
            return false;
        }
        if !self.agent_visibility.is_empty() {
            return self.agent_visibility.iter().any(|a| a == agent_id);
        }
        // 預設規則：使用者記憶與 session 暫存不自動給 agent；
        // 知識／know-how／skill／世界觀預設可見。
        !matches!(
            self.layer,
            MemoryLayer::UserMemory | MemoryLayer::SessionContext | MemoryLayer::PersonaCore
        )
    }
}

pub const MAX_MEMORY_CONTENT_BYTES: usize = 8 * 1024;
pub const MAX_MEMORY_TITLE_CHARS: usize = 120;
pub const MAX_MEMORY_TAGS: usize = 16;

/// 明顯的憑證樣態：記憶層絕不保存 secret/token/consent。
/// （最後防線；上層 UI／API 另有引導。誠實限制：樣態比對非完美偵測。）
fn looks_like_secret(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "secret://",
        "bearer ",
        "api-token",
        "apikey",
        "api_key",
        "-----begin",
        "password:",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

/// 來源 URL 的 userinfo 夾帶密碼（https://user:pass@host）：與 secret 樣態
/// 同級拒收——provenance 常是 URL，這是憑證入庫的典型側門。
fn url_carries_credentials(text: &str) -> bool {
    let Some((_, rest)) = text.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    match authority.rsplit_once('@') {
        Some((userinfo, _host)) => userinfo.contains(':'),
        None => false,
    }
}

pub fn validate_memory_item(item: &MemoryItem) -> Result<(), String> {
    if item.title.trim().is_empty() || item.title.chars().count() > MAX_MEMORY_TITLE_CHARS {
        return Err(format!("title 必須為 1..{MAX_MEMORY_TITLE_CHARS} 字"));
    }
    if item.content.len() > MAX_MEMORY_CONTENT_BYTES {
        return Err(format!("content 超過 {MAX_MEMORY_CONTENT_BYTES} bytes"));
    }
    if item.tags.len() > MAX_MEMORY_TAGS {
        return Err(format!("tags 最多 {MAX_MEMORY_TAGS} 個"));
    }
    if !(0.0..=1.0).contains(&item.confidence) {
        return Err("confidence 必須在 0..1".into());
    }
    // secret 掃描涵蓋所有自由文字欄位：tags 與 provenance 一樣能夾帶憑證，
    // 只掃 content／title 會留下未掃的側門。
    let secret_present = looks_like_secret(&item.content)
        || looks_like_secret(&item.title)
        || item.tags.iter().any(|t| looks_like_secret(t))
        || item
            .provenance
            .iter()
            .any(|p| looks_like_secret(p) || url_carries_credentials(p));
    if secret_present {
        return Err("記憶不得包含 secret/token/credential 樣態內容".into());
    }
    Ok(())
}

/// 建立／更新規則（呼叫端在寫入前執行；PATCH 也要重套，否則補丁成為
/// 解除降權的側門）：
/// - agent 建立的使用者長期記憶不允許——「長期」看 horizon 不看欄位有無：
///   最早檢查點晚於 30 天上限即視同長期（給 100 年的 reviewAfter 繞不過），
///   降為 30 天複查的 Candidate，等使用者確認（spec §13.4）。
/// - agent 建立的 fact 一律降為 inference／candidate（agent 聲稱≠事實）。
/// - agent 供給的 reviewAfter／expiresAt 不得晚於層級預設 horizon（spec §15
///   預設表；有預設值的維度取 min、沒給值視同要求超長一樣壓回預設）。
///   until-deleted 層（world／skill）依規格表無自動期限，不另設。
pub fn apply_actor_rules(item: &mut MemoryItem, now: Timestamp) {
    if matches!(item.created_by, MemoryActor::Agent(_)) {
        if item.kind == MemoryKind::Fact {
            item.kind = MemoryKind::Inference;
        }
        let review_cap = now + chrono::Duration::days(30);
        let earliest_checkpoint = [item.retention.expires_at, item.retention.review_after]
            .into_iter()
            .flatten()
            .min();
        let wants_long_term = match earliest_checkpoint {
            Some(t) => t > review_cap,
            None => true,
        };
        if item.layer == MemoryLayer::UserMemory && wants_long_term {
            item.kind = MemoryKind::Candidate;
            item.retention.review_after = Some(review_cap);
        }
        // 人格核心：agent 絕不能直接改——一律候選＋複查。
        if item.layer == MemoryLayer::PersonaCore {
            item.kind = MemoryKind::Candidate;
            item.retention.review_after = Some(review_cap);
        }
        // 層級預設是 agent 保存 horizon 的天花板：agent 不能替自己的寫入
        // 爭取比預設更長的複查／到期週期。
        let defaults = default_retention(item.layer, now);
        if let Some(cap) = defaults.review_after {
            item.retention.review_after =
                Some(item.retention.review_after.map_or(cap, |t| t.min(cap)));
        }
        if let Some(cap) = defaults.expires_at {
            item.retention.expires_at = Some(item.retention.expires_at.map_or(cap, |t| t.min(cap)));
        }
    }
}

/// 建構輔助。
pub fn new_memory_item(
    layer: MemoryLayer,
    kind: MemoryKind,
    title: impl Into<String>,
    content: impl Into<String>,
    created_by: MemoryActor,
    now: Timestamp,
) -> MemoryItem {
    MemoryItem {
        memory_id: MemoryId::generate(),
        layer,
        kind,
        title: title.into(),
        content: content.into(),
        created_by,
        provenance: vec![],
        confidence: 1.0,
        tags: vec![],
        agent_visibility: vec![],
        agent_denylist: vec![],
        retention: default_retention(layer, now),
        created_at: now,
        updated_at: now,
        last_used_at: None,
        schema_version: SCHEMA_VERSION.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn retention_three_states_derive_status() {
        let now = Utc::now();
        let mut item = new_memory_item(
            MemoryLayer::TaskMemory,
            MemoryKind::Fact,
            "任務",
            "內容",
            MemoryActor::Human,
            now,
        );
        assert_eq!(item.status(now), MemoryStatus::Active);
        item.retention.review_after = Some(now - chrono::Duration::days(1));
        assert_eq!(item.status(now), MemoryStatus::Stale);
        item.retention.expires_at = Some(now - chrono::Duration::hours(1));
        assert_eq!(item.status(now), MemoryStatus::Expired);
    }

    #[test]
    fn defaults_follow_the_spec_table() {
        let now = Utc::now();
        assert!(default_retention(MemoryLayer::SessionContext, now)
            .expires_at
            .is_some());
        assert!(default_retention(MemoryLayer::AgentHandoff, now)
            .expires_at
            .is_some());
        assert!(default_retention(MemoryLayer::TaskMemory, now)
            .review_after
            .is_some());
        // until-deleted：無自動期限，但可刪（不存在不可刪的永久記憶）。
        let persona = default_retention(MemoryLayer::PersonaCore, now);
        assert!(persona.expires_at.is_none() && persona.review_after.is_none());
    }

    #[test]
    fn secrets_never_enter_memory() {
        let now = Utc::now();
        let mut item = new_memory_item(
            MemoryLayer::UserMemory,
            MemoryKind::Fact,
            "token",
            "Bearer abc123",
            MemoryActor::Human,
            now,
        );
        assert!(validate_memory_item(&item).is_err());
        item.content = "secret://device-key".into();
        assert!(validate_memory_item(&item).is_err());
        item.content = "喜歡深色主題".into();
        assert!(validate_memory_item(&item).is_ok());
    }

    #[test]
    fn agent_rules_demote_claims_and_long_term_user_memory() {
        let now = Utc::now();
        // agent 的 fact → inference。
        let mut item = new_memory_item(
            MemoryLayer::DomainKnowledge,
            MemoryKind::Fact,
            "宣稱",
            "x",
            MemoryActor::Agent("codex".into()),
            now,
        );
        apply_actor_rules(&mut item, now);
        assert_eq!(item.kind, MemoryKind::Inference);
        // agent 想寫使用者長期記憶 → candidate＋30 天複查。
        let mut item = new_memory_item(
            MemoryLayer::UserMemory,
            MemoryKind::Preference,
            "偏好",
            "x",
            MemoryActor::Agent("claude-code".into()),
            now,
        );
        item.retention = RetentionPolicy::default(); // 長期
        apply_actor_rules(&mut item, now);
        assert_eq!(item.kind, MemoryKind::Candidate);
        assert!(item.retention.review_after.is_some());
        // 人格核心：一律候選。
        let mut item = new_memory_item(
            MemoryLayer::PersonaCore,
            MemoryKind::Fact,
            "人格",
            "x",
            MemoryActor::Agent("codex".into()),
            now,
        );
        apply_actor_rules(&mut item, now);
        assert_eq!(item.kind, MemoryKind::Candidate);
    }

    #[test]
    fn far_future_horizon_counts_as_long_term() {
        let now = Utc::now();
        // 遠期 reviewAfter：形式上「有檢查點」，實質是長期——仍要降候選。
        let mut item = new_memory_item(
            MemoryLayer::UserMemory,
            MemoryKind::Preference,
            "百年偏好",
            "x",
            MemoryActor::Agent("codex".into()),
            now,
        );
        item.retention = RetentionPolicy {
            review_after: Some(now + chrono::Duration::days(36500)),
            ..Default::default()
        };
        apply_actor_rules(&mut item, now);
        assert_eq!(item.kind, MemoryKind::Candidate);
        assert!(item.retention.review_after.unwrap() <= now + chrono::Duration::days(30));
        // 遠期 expiresAt 同樣視為長期。
        let mut item = new_memory_item(
            MemoryLayer::UserMemory,
            MemoryKind::Preference,
            "百年偏好",
            "x",
            MemoryActor::Agent("codex".into()),
            now,
        );
        item.retention = RetentionPolicy {
            expires_at: Some(now + chrono::Duration::days(36500)),
            ..Default::default()
        };
        apply_actor_rules(&mut item, now);
        assert_eq!(item.kind, MemoryKind::Candidate);
        assert!(item.retention.review_after.unwrap() <= now + chrono::Duration::days(30));
        // 30 天內的檢查點不算長期（既有行為不變）。
        let mut item = new_memory_item(
            MemoryLayer::UserMemory,
            MemoryKind::Preference,
            "短期偏好",
            "x",
            MemoryActor::Agent("codex".into()),
            now,
        );
        item.retention = RetentionPolicy {
            review_after: Some(now + chrono::Duration::days(10)),
            ..Default::default()
        };
        apply_actor_rules(&mut item, now);
        assert_eq!(item.kind, MemoryKind::Preference);
    }

    #[test]
    fn agent_horizons_clamped_to_layer_defaults() {
        let now = Utc::now();
        // domain-knowledge：agent 給 100 年 → 壓回預設 180 天。
        let mut item = new_memory_item(
            MemoryLayer::DomainKnowledge,
            MemoryKind::Inference,
            "知識",
            "x",
            MemoryActor::Agent("codex".into()),
            now,
        );
        item.retention = RetentionPolicy {
            review_after: Some(now + chrono::Duration::days(36500)),
            ..Default::default()
        };
        apply_actor_rules(&mut item, now);
        assert_eq!(
            item.retention.review_after,
            Some(now + chrono::Duration::days(180))
        );
        // session-context：明確給 {}（until-deleted）也壓回 24h 到期。
        let mut item = new_memory_item(
            MemoryLayer::SessionContext,
            MemoryKind::Fact,
            "暫存",
            "x",
            MemoryActor::Agent("codex".into()),
            now,
        );
        item.retention = RetentionPolicy::default();
        apply_actor_rules(&mut item, now);
        assert_eq!(
            item.retention.expires_at,
            Some(now + chrono::Duration::hours(24))
        );
        // 人類寫入不受 horizon 限制。
        let mut item = new_memory_item(
            MemoryLayer::DomainKnowledge,
            MemoryKind::Fact,
            "人寫",
            "x",
            MemoryActor::Human,
            now,
        );
        item.retention = RetentionPolicy {
            review_after: Some(now + chrono::Duration::days(36500)),
            ..Default::default()
        };
        apply_actor_rules(&mut item, now);
        assert_eq!(
            item.retention.review_after,
            Some(now + chrono::Duration::days(36500))
        );
    }

    #[test]
    fn secrets_in_tags_and_provenance_rejected() {
        let now = Utc::now();
        let mut item = new_memory_item(
            MemoryLayer::UserMemory,
            MemoryKind::Fact,
            "標籤",
            "普通內容",
            MemoryActor::Human,
            now,
        );
        item.tags = vec!["api_key".into()];
        assert!(validate_memory_item(&item).is_err(), "tag 夾帶憑證樣態");
        item.tags = vec!["rust".into()];
        item.provenance = vec!["Bearer abc123".into()];
        assert!(
            validate_memory_item(&item).is_err(),
            "provenance 夾帶憑證樣態"
        );
        item.provenance = vec!["https://user:token@example.com/repo".into()];
        assert!(
            validate_memory_item(&item).is_err(),
            "provenance URL userinfo 帶密碼"
        );
        item.provenance = vec!["https://example.com/doc".into()];
        assert!(validate_memory_item(&item).is_ok(), "乾淨來源不受影響");
    }

    #[test]
    fn visibility_denylist_wins_and_sensitive_layers_default_hidden() {
        let now = Utc::now();
        let mut item = new_memory_item(
            MemoryLayer::DomainKnowHow,
            MemoryKind::KnowHow,
            "方法",
            "x",
            MemoryActor::Human,
            now,
        );
        assert!(item.visible_to_agent("codex"));
        item.agent_denylist = vec!["codex".into()];
        assert!(!item.visible_to_agent("codex"));
        let user = new_memory_item(
            MemoryLayer::UserMemory,
            MemoryKind::Preference,
            "私人",
            "x",
            MemoryActor::Human,
            now,
        );
        assert!(!user.visible_to_agent("codex"), "使用者記憶預設不給 agent");
    }
}
