//! 主動式對話政策（spec §6）——由 Rust **確定性**強制，不靠 prompt。
//!
//! 五種模式：off／necessary／natural（預設）／lively／custom。
//! 頻率限制（每小時上限、最短間隔、相近合併、未回覆不追問、勿擾延後）
//! 只約束「非安全」訊息；安全類（等待確認、失敗、未知、感測、重要異常）
//! 只做去重，**永不**被一般頻率壓制。
//!
//! 主動發話權 ≠ 主動行動權：本模組只管「說話」；任何行動仍走 governor。

use interaction_core::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const PROACTIVE_META_KEY: &str = "proactive_dialogue";

/// 訊息類別（決定模式閘門與是否受頻率限制）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProactiveClass {
    /// 必要安全與權限提示：等待確認／失敗／未知／感測／重要異常。
    Safety,
    /// 任務進度。
    TaskProgress,
    /// 任務完成通知。
    Completion,
    /// 低頻情境建議。
    Suggestion,
    /// 問候。
    Greeting,
    /// 輕量陪伴。
    Companionship,
    /// 世界觀小事件。
    WorldEvent,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ProactiveMode {
    /// 不主動說話（只保留必要安全與權限提示）。
    Off,
    /// 只有必要類。
    Necessary,
    /// 建議預設：必要＋任務進度／完成＋低頻建議。
    #[default]
    Natural,
    /// 加問候、陪伴、世界觀小事件（不增加任何權限）。
    Lively,
    /// 個別開關。
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomClasses {
    pub task_progress: bool,
    pub completion: bool,
    pub suggestion: bool,
    pub greeting: bool,
    pub companionship: bool,
    pub world_event: bool,
}

impl Default for CustomClasses {
    fn default() -> Self {
        Self {
            task_progress: true,
            completion: true,
            suggestion: true,
            greeting: false,
            companionship: false,
            world_event: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ProactiveDialogueConfig {
    pub mode: ProactiveMode,
    pub custom: CustomClasses,
    /// 每小時最多幾則非安全主動訊息（spec 建議 3）。
    pub max_per_hour: u32,
    /// 一般訊息最短間隔（分鐘，spec 建議 10–15）。
    pub min_interval_minutes: u32,
    /// 相近事件合併窗（秒）。
    pub merge_window_seconds: u32,
    /// 沒有回覆時不再追問。
    pub no_follow_up: bool,
    /// 勿擾時段延後非必要訊息。
    pub dnd_defer: bool,
    /// 生成式主動對話的每日預算（Phase 3 connector 用；0 = 不允許）。
    pub daily_generative_sessions: u32,
    pub daily_generative_cost_usd: f64,
}

impl Default for ProactiveDialogueConfig {
    fn default() -> Self {
        Self {
            mode: ProactiveMode::default(),
            custom: CustomClasses::default(),
            max_per_hour: 3,
            min_interval_minutes: 12,
            merge_window_seconds: 30,
            no_follow_up: true,
            dnd_defer: true,
            daily_generative_sessions: 8,
            daily_generative_cost_usd: 1.0,
        }
    }
}

/// 執行期狀態（持久化到 meta，重啟後限制連續）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProactiveDialogueState {
    pub config: ProactiveDialogueConfig,
    /// 最近非安全發話時間（bounded，用於每小時上限與最短間隔）。
    pub recent_sends: Vec<Timestamp>,
    /// 去重鍵 → 上次發送時間（安全與一般皆去重）。
    pub dedup: Vec<(String, Timestamp)>,
    /// 最後一則主動訊息是否已獲使用者回應（未回應 → 不追問）。
    pub last_answered: bool,
    /// 使用者要求安靜到此時間（今天安靜一點／一小時內不要說話）。
    pub quiet_until: Option<Timestamp>,
    /// 今日生成式對話用量（日期字串, 次數, 費用）。
    pub generative_today: Option<(String, u32, f64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "decision")]
pub enum ProactiveDecision {
    Allowed,
    /// 被模式／頻率／勿擾壓下（含原因；勿擾屬延後語意）。
    Suppressed {
        reason: String,
    },
}

const DEDUP_WINDOW_SECS: i64 = 600; // 同一事件 10 分鐘內只提醒一次
const MAX_TRACKED: usize = 64;

impl ProactiveDialogueState {
    /// 確定性決策：class 是否允許現在發話。允許時**登記**本次發話。
    /// `dnd_active`＝policy 勿擾時段目前生效（由呼叫端以本地時間判定）；
    /// 安全類不受勿擾延後，一般類在 `dnd_defer` 開啟時延後。
    pub fn gate(
        &mut self,
        class: ProactiveClass,
        dedup_key: &str,
        dnd_active: bool,
        now: Timestamp,
    ) -> ProactiveDecision {
        self.prune(now);
        // 去重（安全與一般都適用；安全可突破頻率但不可疲勞轟炸）。
        if !dedup_key.is_empty() {
            if let Some((_, at)) = self.dedup.iter().find(|(k, _)| k == dedup_key) {
                if now.signed_duration_since(*at).num_seconds() < DEDUP_WINDOW_SECS {
                    return ProactiveDecision::Suppressed {
                        reason: format!("同一事件 {} 分鐘內只提醒一次", DEDUP_WINDOW_SECS / 60),
                    };
                }
            }
        }
        if class == ProactiveClass::Safety {
            self.remember(dedup_key, now);
            return ProactiveDecision::Allowed;
        }
        // 模式閘門。
        if !self.class_enabled(class) {
            return ProactiveDecision::Suppressed {
                reason: format!("目前模式（{:?}）不主動發送此類訊息", self.config.mode),
            };
        }
        // 使用者要求安靜。
        if let Some(until) = self.quiet_until {
            if now < until {
                return ProactiveDecision::Suppressed {
                    reason: "使用者要求暫時安靜".into(),
                };
            }
        }
        // 勿擾時段（policy.quietHours）：延後非必要訊息。不登記發送與去重
        // ——屬延後語意，窗結束後同一 dedupKey 仍可提醒，而非丟棄。
        if dnd_active && self.config.dnd_defer {
            return ProactiveDecision::Suppressed {
                reason: "勿擾時段，非必要訊息延後".into(),
            };
        }
        // 未回覆不追問。
        if self.config.no_follow_up && !self.last_answered && !self.recent_sends.is_empty() {
            return ProactiveDecision::Suppressed {
                reason: "上一則主動訊息尚未獲回應，不追問".into(),
            };
        }
        // 相近事件合併窗：窗內任何已發訊息 → 合併（不再發）。
        if let Some(last) = self.recent_sends.last() {
            let since = now.signed_duration_since(*last).num_seconds();
            if since < self.config.merge_window_seconds as i64 {
                return ProactiveDecision::Suppressed {
                    reason: "與前一則訊息過近，已合併".into(),
                };
            }
            // 最短間隔。
            if since < (self.config.min_interval_minutes as i64) * 60 {
                return ProactiveDecision::Suppressed {
                    reason: format!("一般訊息最短間隔 {} 分鐘", self.config.min_interval_minutes),
                };
            }
        }
        // 每小時上限。
        let hour_ago = now - chrono::Duration::hours(1);
        let this_hour = self.recent_sends.iter().filter(|t| **t > hour_ago).count();
        if this_hour >= self.config.max_per_hour as usize {
            return ProactiveDecision::Suppressed {
                reason: format!("已達每小時 {} 則上限", self.config.max_per_hour),
            };
        }
        self.recent_sends.push(now);
        if self.recent_sends.len() > MAX_TRACKED {
            self.recent_sends.remove(0);
        }
        self.last_answered = false;
        self.remember(dedup_key, now);
        ProactiveDecision::Allowed
    }

    fn class_enabled(&self, class: ProactiveClass) -> bool {
        use ProactiveClass as C;
        use ProactiveMode as M;
        match self.config.mode {
            M::Off | M::Necessary => false, // Safety 已在前面放行
            M::Natural => matches!(class, C::TaskProgress | C::Completion | C::Suggestion),
            M::Lively => true,
            M::Custom => match class {
                C::Safety => true,
                C::TaskProgress => self.config.custom.task_progress,
                C::Completion => self.config.custom.completion,
                C::Suggestion => self.config.custom.suggestion,
                C::Greeting => self.config.custom.greeting,
                C::Companionship => self.config.custom.companionship,
                C::WorldEvent => self.config.custom.world_event,
            },
        }
    }

    fn remember(&mut self, key: &str, now: Timestamp) {
        if key.is_empty() {
            return;
        }
        self.dedup.retain(|(k, _)| k != key);
        self.dedup.push((key.to_string(), now));
        if self.dedup.len() > MAX_TRACKED {
            self.dedup.remove(0);
        }
    }

    fn prune(&mut self, now: Timestamp) {
        let day_ago = now - chrono::Duration::hours(24);
        self.recent_sends.retain(|t| *t > day_ago);
        self.dedup
            .retain(|(_, t)| now.signed_duration_since(*t).num_seconds() < DEDUP_WINDOW_SECS * 2);
    }

    /// 使用者互動（回覆／點擊氣泡）→ 解除「不追問」。
    pub fn note_user_reply(&mut self) {
        self.last_answered = true;
    }

    pub fn quiet_for(&mut self, now: Timestamp, minutes: i64) {
        self.quiet_until = Some(now + chrono::Duration::minutes(minutes));
    }

    pub fn status(&self, now: Timestamp) -> serde_json::Value {
        let hour_ago = now - chrono::Duration::hours(1);
        json!({
            "config": self.config,
            "sentThisHour": self.recent_sends.iter().filter(|t| **t > hour_ago).count(),
            "quietUntil": self.quiet_until,
            "lastAnswered": self.last_answered,
        })
    }
}

/// 從 plan metadata 解析類別（recipes 可宣告 proactiveClass；預設 suggestion）。
pub fn class_from_metadata(
    meta: &std::collections::BTreeMap<String, serde_json::Value>,
) -> ProactiveClass {
    match meta.get("proactiveClass").and_then(|v| v.as_str()) {
        Some("safety") => ProactiveClass::Safety,
        Some("task-progress") => ProactiveClass::TaskProgress,
        Some("completion") => ProactiveClass::Completion,
        Some("greeting") => ProactiveClass::Greeting,
        Some("companionship") => ProactiveClass::Companionship,
        Some("world-event") => ProactiveClass::WorldEvent,
        _ => ProactiveClass::Suggestion,
    }
}

/// 只有這些頻道上的自主動作屬於「主動說話」。
pub fn is_dialogue_channel(channel: &str) -> bool {
    matches!(
        channel,
        "desktop-pet" | "notification" | "conversation" | "audio"
    )
}

// ---------------------------------------------------------------------------
// Runtime 介面。
// ---------------------------------------------------------------------------

use crate::runtime::Runtime;

impl Runtime {
    pub async fn proactive_dialogue_status(&self) -> serde_json::Value {
        self.proactive_dialogue
            .read()
            .await
            .status(chrono::Utc::now())
    }

    /// 更新設定（部分欄位 merge）。
    pub async fn proactive_dialogue_configure(
        &self,
        patch: serde_json::Value,
    ) -> interaction_core::DomainResult<serde_json::Value> {
        let mut guard = self.proactive_dialogue.write().await;
        let mut cfg = serde_json::to_value(&guard.config).unwrap_or_default();
        if let (Some(obj), Some(p)) = (cfg.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
        let parsed: ProactiveDialogueConfig = serde_json::from_value(cfg).map_err(|e| {
            interaction_core::DomainError::Validation(format!("invalid config: {e}"))
        })?;
        let prior = std::mem::replace(&mut guard.config, parsed);
        if let Err(e) = self.persist_proactive(&guard) {
            // 存檔失敗不得謊稱已儲存（誠實階梯）：還原記憶體設定並回報錯誤。
            guard.config = prior;
            return Err(e);
        }
        let snapshot = guard.status(chrono::Utc::now());
        self.events.publish(interaction_core::RuntimeEvent::new(
            interaction_core::EventType::PolicyChanged,
            chrono::Utc::now(),
            serde_json::json!({"scope": "proactive-dialogue"}),
        ));
        Ok(snapshot)
    }

    /// 使用者要求安靜（今天安靜一點=到今日結束；一小時=60）。
    pub async fn proactive_dialogue_quiet(&self, minutes: i64) -> serde_json::Value {
        let mut guard = self.proactive_dialogue.write().await;
        guard.quiet_for(chrono::Utc::now(), minutes.clamp(1, 24 * 60));
        let mut snapshot = guard.status(chrono::Utc::now());
        if let Err(e) = self.persist_proactive(&guard) {
            // 回傳型別固定為 snapshot（HTTP／Tauri 直接轉發），以欄位誠實
            // 回報存檔失敗：安靜請求本次生效，但重啟後不保證延續。
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert("persistError".into(), json!(e.to_string()));
            }
        }
        snapshot
    }

    /// 確定性閘門＋持久化（供 executor 與未來的 agent 主動對話使用）。
    /// 勿擾時段在此以 policy.quietHours（本地時間）判定後傳入 gate：
    /// governor 的 quiet-hours 只壓制侵擾頻道（conversation 不在清單內），
    /// 對話類的勿擾延後必須由本閘門確定性強制。
    pub async fn proactive_dialogue_gate(
        &self,
        class: ProactiveClass,
        dedup_key: &str,
    ) -> ProactiveDecision {
        let dnd_active = {
            let policy = self.policy().await;
            let local = chrono::Local::now().time();
            policy
                .quiet_hours
                .iter()
                .any(|w| crate::runtime::quiet_window_active(&w.start, &w.end, local))
        };
        let mut guard = self.proactive_dialogue.write().await;
        let decision = guard.gate(class, dedup_key, dnd_active, chrono::Utc::now());
        // persist 失敗已於 persist_proactive 記 log；決策以記憶體狀態為準。
        let _ = self.persist_proactive(&guard);
        decision
    }

    /// 使用者有真實互動 → 解除「未回覆不追問」。
    pub(crate) async fn proactive_note_reply(&self) {
        let mut guard = self.proactive_dialogue.write().await;
        guard.note_user_reply();
        // persist 失敗已於 persist_proactive 記 log；決策以記憶體狀態為準。
        let _ = self.persist_proactive(&guard);
    }

    /// 持久化不得靜默失敗（誠實階梯）：失敗一律記 log 並回傳錯誤，讓
    /// 使用者面向的路徑（configure／quiet）誠實回報；若失敗後即重啟，
    /// 頻率計數會歸零，log 是唯一痕跡。
    fn persist_proactive(
        &self,
        state: &ProactiveDialogueState,
    ) -> interaction_core::DomainResult<()> {
        let result = serde_json::to_string(state)
            .map_err(|e| {
                interaction_core::DomainError::Internal(format!(
                    "serialize proactive-dialogue state: {e}"
                ))
            })
            .and_then(|body| self.store.set_meta(PROACTIVE_META_KEY, &body));
        if let Err(e) = &result {
            tracing::warn!(
                error = %e,
                "failed to persist proactive-dialogue state; rate limits may reset on restart"
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn state(mode: ProactiveMode) -> ProactiveDialogueState {
        let mut s = ProactiveDialogueState::default();
        s.config.mode = mode;
        s.last_answered = true;
        s
    }

    #[test]
    fn off_mode_allows_only_safety() {
        let mut s = state(ProactiveMode::Off);
        let now = Utc::now();
        assert_eq!(
            s.gate(ProactiveClass::Safety, "e1", false, now),
            ProactiveDecision::Allowed
        );
        assert!(matches!(
            s.gate(ProactiveClass::Suggestion, "e2", false, now),
            ProactiveDecision::Suppressed { .. }
        ));
        assert!(matches!(
            s.gate(ProactiveClass::Greeting, "e3", false, now),
            ProactiveDecision::Suppressed { .. }
        ));
    }

    #[test]
    fn natural_mode_excludes_greetings_lively_includes() {
        let now = Utc::now();
        let mut natural = state(ProactiveMode::Natural);
        assert!(matches!(
            natural.gate(ProactiveClass::Greeting, "g", false, now),
            ProactiveDecision::Suppressed { .. }
        ));
        let mut lively = state(ProactiveMode::Lively);
        assert_eq!(
            lively.gate(ProactiveClass::Greeting, "g", false, now),
            ProactiveDecision::Allowed
        );
    }

    #[test]
    fn hourly_cap_and_min_interval_are_hard() {
        let mut s = state(ProactiveMode::Lively);
        let base = Utc::now();
        // 間隔 13 分鐘發滿 3 則。
        for i in 0..3 {
            s.last_answered = true;
            let t = base + chrono::Duration::minutes(13 * i);
            assert_eq!(
                s.gate(ProactiveClass::Suggestion, &format!("k{i}"), false, t),
                ProactiveDecision::Allowed
            );
        }
        // 第 4 則在同一小時內 → 壓下。
        s.last_answered = true;
        let t4 = base + chrono::Duration::minutes(40);
        assert!(matches!(
            s.gate(ProactiveClass::Suggestion, "k4", false, t4),
            ProactiveDecision::Suppressed { .. }
        ));
        // 最短間隔：距上一則 5 分鐘 → 壓下（即使小時額度已回復也一樣邏輯）。
        let mut s2 = state(ProactiveMode::Lively);
        s2.last_answered = true;
        assert_eq!(
            s2.gate(ProactiveClass::Suggestion, "a", false, base),
            ProactiveDecision::Allowed
        );
        s2.last_answered = true;
        assert!(matches!(
            s2.gate(
                ProactiveClass::Suggestion,
                "b",
                false,
                base + chrono::Duration::minutes(5)
            ),
            ProactiveDecision::Suppressed { .. }
        ));
    }

    #[test]
    fn merge_window_and_no_follow_up() {
        let mut s = state(ProactiveMode::Lively);
        let base = Utc::now();
        assert_eq!(
            s.gate(ProactiveClass::Completion, "c1", false, base),
            ProactiveDecision::Allowed
        );
        // 20 秒內相近事件 → 合併。
        assert!(matches!(
            s.gate(
                ProactiveClass::Completion,
                "c2",
                false,
                base + chrono::Duration::seconds(20)
            ),
            ProactiveDecision::Suppressed { .. }
        ));
        // 未回覆 → 不追問（即使過了間隔）。
        assert!(!s.last_answered);
        assert!(matches!(
            s.gate(
                ProactiveClass::Suggestion,
                "c3",
                false,
                base + chrono::Duration::minutes(20)
            ),
            ProactiveDecision::Suppressed { .. }
        ));
        // 使用者回覆後恢復。
        s.note_user_reply();
        assert_eq!(
            s.gate(
                ProactiveClass::Suggestion,
                "c4",
                false,
                base + chrono::Duration::minutes(21)
            ),
            ProactiveDecision::Allowed
        );
    }

    #[test]
    fn safety_dedups_but_is_never_rate_limited() {
        let mut s = state(ProactiveMode::Off);
        let base = Utc::now();
        // 連續多個不同安全事件全部放行（頻率不適用）。
        for i in 0..10 {
            assert_eq!(
                s.gate(ProactiveClass::Safety, &format!("s{i}"), false, base),
                ProactiveDecision::Allowed
            );
        }
        // 同一鍵 10 分鐘內去重。
        assert!(matches!(
            s.gate(
                ProactiveClass::Safety,
                "s1",
                false,
                base + chrono::Duration::seconds(30)
            ),
            ProactiveDecision::Suppressed { .. }
        ));
        // 超過窗後可再提醒。
        assert_eq!(
            s.gate(
                ProactiveClass::Safety,
                "s1",
                false,
                base + chrono::Duration::minutes(11)
            ),
            ProactiveDecision::Allowed
        );
    }

    #[test]
    fn quiet_until_defers_non_essential_only() {
        let mut s = state(ProactiveMode::Lively);
        let now = Utc::now();
        s.quiet_for(now, 60);
        assert!(matches!(
            s.gate(
                ProactiveClass::Greeting,
                "g",
                false,
                now + chrono::Duration::minutes(5)
            ),
            ProactiveDecision::Suppressed { .. }
        ));
        // 安全提示不受安靜請求影響。
        assert_eq!(
            s.gate(
                ProactiveClass::Safety,
                "sfe",
                false,
                now + chrono::Duration::minutes(5)
            ),
            ProactiveDecision::Allowed
        );
        // 安靜期滿恢復。
        s.note_user_reply();
        assert_eq!(
            s.gate(
                ProactiveClass::Greeting,
                "g2",
                false,
                now + chrono::Duration::minutes(65)
            ),
            ProactiveDecision::Allowed
        );
    }

    #[test]
    fn dnd_defers_non_essential_but_never_safety() {
        let mut s = state(ProactiveMode::Lively);
        let now = Utc::now();
        // 勿擾生效＋dndDefer 預設開啟 → 一般訊息延後。
        assert_eq!(
            s.gate(ProactiveClass::Suggestion, "d1", true, now),
            ProactiveDecision::Suppressed {
                reason: "勿擾時段，非必要訊息延後".into()
            }
        );
        // 延後語意：勿擾壓下不登記去重與頻率，窗結束後同一事件仍可提醒。
        assert_eq!(
            s.gate(ProactiveClass::Suggestion, "d1", false, now),
            ProactiveDecision::Allowed
        );
        // 安全類不受勿擾延後（只去重，永不被頻率或勿擾壓制）。
        assert_eq!(
            s.gate(ProactiveClass::Safety, "d2", true, now),
            ProactiveDecision::Allowed
        );
    }

    #[test]
    fn dnd_defer_disabled_delivers_during_quiet_window() {
        let mut s = state(ProactiveMode::Lively);
        s.config.dnd_defer = false;
        let now = Utc::now();
        assert_eq!(
            s.gate(ProactiveClass::Greeting, "g", true, now),
            ProactiveDecision::Allowed
        );
    }
}
