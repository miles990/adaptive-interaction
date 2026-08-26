//! Default interaction text catalog + adaptive selection.
//!
//! Texts are concise, neutral, not infantilized, and can be overridden by user
//! templates or replaced by AI-generated text. Silence is a legitimate choice.

use interaction_core::{MessageMode, MessageStrategy};
use std::collections::VecDeque;
use std::sync::Mutex;

/// Built-in catalog: intent -> (zh-Hant candidates, en candidates).
fn catalog(intent: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match intent {
        "presence" => (&["在。", "我在這裡。"], &["Here.", "Standing by."]),
        "task-start" => (&["開始處理。", "著手進行。"], &["Starting.", "On it."]),
        "progress" => (
            &["進行中，一切正常。", "持續推進。"],
            &["In progress.", "Moving along."],
        ),
        "discovery" => (
            &["發現一個值得注意的點。"],
            &["Found something worth noting."],
        ),
        "success" => (
            &["完成了。", "這一段順利收尾。"],
            &["Done.", "Wrapped up cleanly."],
        ),
        "celebration" => (
            &["完成了，所有檢查都已通過。", "順利完成。"],
            &["Done — all checks passed.", "Completed successfully."],
        ),
        "warning" => (
            &["注意：偵測到需要留意的狀況。"],
            &["Warning: a condition needs your attention."],
        ),
        "failure" => (
            &["失敗了，已停止。詳細原因記錄在案。"],
            &["Failed and stopped; details logged."],
        ),
        "recovery" => (&["已恢復，繼續進行。"], &["Recovered; continuing."]),
        "confirmation-required" => (
            &["需要你的確認才能繼續。"],
            &["Your confirmation is required to continue."],
        ),
        "stopped" => (&["已停止。"], &["Stopped."]),
        "emergency-stop" => (
            &["緊急停止已執行，所有輸出已中止。"],
            &["Emergency stop executed; all outputs halted."],
        ),
        "calm" => (&["一切平穩。"], &["All steady."]),
        "tension" => (
            &["情況緊湊，保持專注。"],
            &["Things are tense; staying focused."],
        ),
        "assistance" => (
            &["需要幫忙的話，我可以接手一部分。"],
            &["I can take over part of this if useful."],
        ),
        "acknowledge" => (&["收到。", "了解。"], &["Acknowledged.", "Got it."]),
        _ => (&[], &[]),
    }
}

/// Anti-repetition memory (recently used texts).
pub struct TextSelector {
    recent: Mutex<VecDeque<String>>,
    capacity: usize,
}

impl Default for TextSelector {
    fn default() -> Self {
        Self {
            recent: Mutex::new(VecDeque::new()),
            capacity: 16,
        }
    }
}

impl TextSelector {
    /// Select a message per strategy. `Ok(None)` = deliberate silence.
    /// `ai_text` is the AI-provided suggestion used by `AiGenerated` (and as a
    /// fallback candidate for `Adaptive`).
    pub fn select(
        &self,
        strategy: &MessageStrategy,
        intent: &str,
        ai_text: Option<&str>,
    ) -> Option<String> {
        match strategy.mode {
            MessageMode::None => None,
            MessageMode::AiGenerated => ai_text.map(|s| s.to_string()).or_else(|| {
                if strategy.allow_silence {
                    None
                } else {
                    self.pick_from_catalog(strategy, intent)
                }
            }),
            MessageMode::Fixed => strategy.templates.first().cloned(),
            MessageMode::Random | MessageMode::Adaptive => {
                let mut candidates: Vec<String> = strategy.templates.clone();
                if candidates.is_empty() {
                    candidates = self.catalog_candidates(strategy, intent);
                }
                if let Some(ai) = ai_text {
                    candidates.push(ai.to_string());
                }
                if candidates.is_empty() {
                    return if strategy.allow_silence {
                        None
                    } else {
                        Some(intent.to_string())
                    };
                }
                let chosen = if strategy.mode == MessageMode::Adaptive {
                    self.pick_least_recent(&candidates)
                } else {
                    // Deterministic pseudo-random pick: hash of recent length +
                    // candidate count (kept simple; runtime jitter handles chance).
                    let idx = (self.recent.lock().expect("recent lock").len() + candidates.len())
                        % candidates.len();
                    candidates[idx].clone()
                };
                self.remember(&chosen);
                Some(chosen)
            }
        }
    }

    fn catalog_candidates(&self, strategy: &MessageStrategy, intent: &str) -> Vec<String> {
        let mut intents: Vec<&str> = strategy.intents.iter().map(|s| s.as_str()).collect();
        if intents.is_empty() {
            intents.push(intent);
        }
        let want_en = strategy
            .language
            .as_deref()
            .map(|l| l.starts_with("en"))
            .unwrap_or(false);
        let mut out = Vec::new();
        for i in intents {
            let (zh, en) = catalog(i);
            let list = if want_en { en } else { zh };
            out.extend(list.iter().map(|s| s.to_string()));
        }
        out
    }

    fn pick_from_catalog(&self, strategy: &MessageStrategy, intent: &str) -> Option<String> {
        let c = self.catalog_candidates(strategy, intent);
        if c.is_empty() {
            None
        } else {
            let chosen = self.pick_least_recent(&c);
            self.remember(&chosen);
            Some(chosen)
        }
    }

    fn pick_least_recent(&self, candidates: &[String]) -> String {
        let recent = self.recent.lock().expect("recent lock");
        candidates
            .iter()
            .min_by_key(|c| {
                recent
                    .iter()
                    .rev()
                    .position(|r| r == *c)
                    .map(|pos| (recent.len() - pos) as i64)
                    .unwrap_or(i64::MAX)
                    // Larger = longer ago (or never) → min_by_key with negation
                    .checked_neg()
                    .unwrap_or(i64::MIN)
            })
            .cloned()
            .unwrap_or_else(|| candidates[0].clone())
    }

    fn remember(&self, text: &str) {
        let mut recent = self.recent.lock().expect("recent lock");
        if recent.len() >= self.capacity {
            recent.pop_front();
        }
        recent.push_back(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_mode_is_silent() {
        let s = TextSelector::default();
        let strategy = MessageStrategy {
            mode: MessageMode::None,
            ..Default::default()
        };
        assert_eq!(s.select(&strategy, "success", None), None);
    }

    #[test]
    fn adaptive_avoids_repetition() {
        let s = TextSelector::default();
        let strategy = MessageStrategy {
            mode: MessageMode::Adaptive,
            templates: vec!["A".into(), "B".into()],
            ..Default::default()
        };
        let first = s.select(&strategy, "success", None).unwrap();
        let second = s.select(&strategy, "success", None).unwrap();
        assert_ne!(first, second, "adaptive selection should rotate candidates");
    }

    #[test]
    fn catalog_covers_all_default_intents() {
        for intent in interaction_core::DEFAULT_MESSAGE_INTENTS {
            let (zh, en) = catalog(intent);
            assert!(
                !zh.is_empty() && !en.is_empty(),
                "missing catalog texts for {intent}"
            );
        }
    }

    #[test]
    fn ai_generated_uses_ai_text() {
        let s = TextSelector::default();
        let strategy = MessageStrategy {
            mode: MessageMode::AiGenerated,
            allow_silence: true,
            ..Default::default()
        };
        assert_eq!(
            s.select(&strategy, "success", Some("custom")),
            Some("custom".into())
        );
        assert_eq!(s.select(&strategy, "success", None), None);
    }
}
