//! Deterministic natural-language recipe summaries. Derived ONLY from the
//! structured recipe — never free text, never AI — so the summary can never
//! drift from what the recipe actually does.

use crate::{ActuationMode, AiAssistMode, AiUnavailableBehavior, FusionMode, Recipe};
use interaction_core::MessageMode;

fn is_zh(locale: &str) -> bool {
    locale.split(['-', '_']).next().unwrap_or("") == "zh"
}

/// Produce a one-paragraph human summary. `resolve_name` maps a technical
/// receptor/actuator id to its display name (fall back to the id itself).
pub fn summarize(recipe: &Recipe, locale: &str, resolve_name: &dyn Fn(&str) -> String) -> String {
    if is_zh(locale) {
        summarize_zh(recipe, resolve_name)
    } else {
        summarize_en(recipe, resolve_name)
    }
}

fn join_names(ids: &[String], resolve: &dyn Fn(&str) -> String, sep: &str) -> String {
    ids.iter().map(|s| resolve(s)).collect::<Vec<_>>().join(sep)
}

fn summarize_zh(recipe: &Recipe, resolve: &dyn Fn(&str) -> String) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Trigger.
    let receptors: Vec<String> = recipe
        .trigger
        .steps
        .iter()
        .map(|s| s.receptor.clone())
        .collect();
    let trigger_names = join_names(&receptors, resolve, "、");
    let trigger = match recipe.trigger.mode {
        FusionMode::Single => format!("當「{trigger_names}」有新事件"),
        FusionMode::All => format!("當「{trigger_names}」全部成立"),
        FusionMode::Any => format!("當「{trigger_names}」任一成立"),
        FusionMode::Quorum => format!(
            "當「{trigger_names}」中至少 {} 項成立",
            recipe.trigger.quorum.unwrap_or(1)
        ),
        FusionMode::Weighted => format!("當「{trigger_names}」的加權訊號超過門檻"),
        FusionMode::Sequence => format!("當「{trigger_names}」依序發生"),
    };
    let within = recipe
        .trigger
        .within
        .as_deref()
        .map(|w| format!("（{w} 內）"))
        .unwrap_or_default();
    parts.push(format!("{trigger}{within}時"));

    // Actuation.
    let candidates = join_names(&recipe.actuation.candidates, resolve, "、");
    let act = match recipe.actuation.mode {
        ActuationMode::Single => format!("系統會使用「{candidates}」回應"),
        ActuationMode::Parallel => format!("系統會同時使用「{candidates}」回應"),
        ActuationMode::Sequence => format!("系統會依序使用「{candidates}」回應"),
        ActuationMode::Fallback => {
            let first = recipe
                .actuation
                .candidates
                .first()
                .map(|c| resolve(c))
                .unwrap_or_default();
            format!("系統會優先使用「{first}」，失敗時改用後備方式")
        }
        ActuationMode::Adaptive => {
            format!("系統會從「{candidates}」中挑選最不打擾的方式回應")
        }
        ActuationMode::Redundant => format!("系統會用多個通道（{candidates}）確保送達"),
    };
    parts.push(act);

    // No-action / silence.
    if recipe.decision.allow_no_action {
        parts.push("如果沒有合適的方式，會選擇不打擾".into());
    }
    if recipe.message.mode == MessageMode::None {
        parts.push("這個配方不顯示文字".into());
    }

    // AI involvement.
    match recipe.ai.as_ref().map(|a| a.mode) {
        None | Some(AiAssistMode::Never) => {
            parts.push("整個過程由本機規則處理，不需要 AI".into());
        }
        Some(AiAssistMode::WhenUncertain) => {
            let fallback = match recipe.ai.as_ref().map(|a| a.on_unavailable) {
                Some(AiUnavailableBehavior::NoAction) => "如果 AI 沒有回應，這次就不介入",
                _ => "如果 AI 沒有回應，會改用本機規則處理",
            };
            parts.push(format!(
                "只有在訊號模糊、無法確定時才會請 AI 協助；{fallback}"
            ));
        }
        Some(AiAssistMode::GenerateText) => {
            parts.push("AI 只負責產生文字內容，用哪種方式回應仍由本機規則決定".into());
        }
        Some(AiAssistMode::Interpret) => {
            parts.push("AI 可以協助解讀觀察到的狀態，但執行仍受安全規則限制".into());
        }
        Some(AiAssistMode::ChooseChannel) => {
            parts.push("AI 可以協助挑選回應方式，但仍受安全規則限制".into());
        }
        Some(AiAssistMode::DraftOnly) => {
            parts.push("AI 只能草擬調整建議，必須由你確認才會生效".into());
        }
    }

    // Limits.
    if let Some(cooldown) = &recipe.limits.cooldown {
        parts.push(format!("同一提醒至少間隔 {cooldown}"));
    }
    if !recipe.consent.required.is_empty() {
        parts.push("執行前需要對應的使用授權".into());
    }

    let mut s = parts.join("。");
    s.push('。');
    s
}

fn summarize_en(recipe: &Recipe, resolve: &dyn Fn(&str) -> String) -> String {
    let receptors: Vec<String> = recipe
        .trigger
        .steps
        .iter()
        .map(|s| s.receptor.clone())
        .collect();
    let trigger_names = join_names(&receptors, resolve, ", ");
    let mut parts: Vec<String> = Vec::new();
    let trigger = match recipe.trigger.mode {
        FusionMode::Single => format!("When {trigger_names} reports an event"),
        FusionMode::All => format!("When all of {trigger_names} match"),
        FusionMode::Any => format!("When any of {trigger_names} matches"),
        FusionMode::Quorum => format!(
            "When at least {} of {trigger_names} match",
            recipe.trigger.quorum.unwrap_or(1)
        ),
        FusionMode::Weighted => {
            format!("When the weighted signal from {trigger_names} crosses the threshold")
        }
        FusionMode::Sequence => format!("When {trigger_names} happen in order"),
    };
    parts.push(trigger);
    let candidates = join_names(&recipe.actuation.candidates, resolve, ", ");
    parts.push(match recipe.actuation.mode {
        ActuationMode::Adaptive => {
            format!("the system picks the least intrusive of {candidates}")
        }
        ActuationMode::Fallback => {
            format!("the system prefers the first of {candidates}, falling back on failure")
        }
        _ => format!("the system responds via {candidates}"),
    });
    if recipe.decision.allow_no_action {
        parts.push("staying silent is allowed when nothing fits".into());
    }
    match recipe.ai.as_ref().map(|a| a.mode) {
        None | Some(AiAssistMode::Never) => {
            parts.push("local rules handle everything; no AI involved".into())
        }
        Some(AiAssistMode::WhenUncertain) => parts.push(
            "AI is only consulted when the signals are ambiguous; a deterministic fallback applies if it does not answer".into(),
        ),
        Some(m) => parts.push(format!("AI involvement: {m:?}")),
    }
    let mut s = parts.join("; ");
    s.push('.');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_and_validate;

    const RECIPE: &str = r#"
id: t
name: 測試
trigger:
  mode: single
  steps:
    - receptor: task.lifecycle
decision:
  objective: x
actuation:
  mode: adaptive
  candidates: [conversation, local-notification]
limits:
  cooldown: 15m
"#;

    #[test]
    fn summary_reflects_structure_not_free_text() {
        let recipe = parse_and_validate(RECIPE).unwrap();
        let resolve = |id: &str| -> String {
            match id {
                "task.lifecycle" => "任務狀態".into(),
                "conversation" => "對話訊息".into(),
                "local-notification" => "桌面通知".into(),
                other => other.into(),
            }
        };
        let s = summarize(&recipe, "zh-TW", &resolve);
        assert!(s.contains("任務狀態"), "{s}");
        assert!(s.contains("最不打擾"), "{s}");
        assert!(s.contains("不需要 AI"), "{s}");
        assert!(s.contains("15m"), "{s}");
        // Structure change → summary change.
        let with_ai =
            RECIPE.to_string() + "\nai:\n  mode: when-uncertain\n  onUnavailable: no-action\n";
        let recipe2 = parse_and_validate(&with_ai).unwrap();
        let s2 = summarize(&recipe2, "zh-TW", &resolve);
        assert!(s2.contains("才會請 AI 協助"), "{s2}");
        assert!(s2.contains("這次就不介入"), "{s2}");
    }

    #[test]
    fn english_summary_available() {
        let recipe = parse_and_validate(RECIPE).unwrap();
        let resolve = |id: &str| -> String { id.into() };
        let s = summarize(&recipe, "en", &resolve);
        assert!(s.contains("least intrusive"), "{s}");
        assert!(s.contains("no AI involved"), "{s}");
    }
}
