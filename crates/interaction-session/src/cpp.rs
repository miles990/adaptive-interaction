//! §5 Behavior Intent → Character Presentation Protocol（CPP）投影。
//!
//! 這裡**只**產生語意（intent／variant／parameters／priority）；`truthState`、`messageId`、
//! `characterInstanceId`、`expiresAt` 由 Runtime 端的 `CppRendererAdapter` 補上——
//! 角色呈現層沒有權限主權，adapter 不能改寫安全文字、不能偽造 `verified`。
//!
//! `celebrate` **不投影**：桌面已由既有 Runtime 真相投影送 `verified-success`（受保護行為，不雙播）。

use interaction_character::{priority_floor, CharacterIntent};
use serde_json::{json, Value};

use crate::types::{
    BehaviorIntent, INTENT_CELEBRATE, INTENT_IDLE, INTENT_REACT_HAPPILY_TO_TOUCH, INTENT_SETTLE,
};

/// Runtime 非安全投影的 requested priority（`docs/character-protocol/README.md` §4.3：非安全 runtime 投影 40）。
pub const RUNTIME_PROJECTION_PRIORITY: u8 = 40;

/// Behavior Intent 投影到 CPP 的結果。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CppProjection {
    pub intent: CharacterIntent,
    pub variant: String,
    pub parameters: Value,
    pub priority: u8,
}

/// §5 投影表。無對應者回 `None`（renderer 端自行降級，不猜、不雙播）。
pub fn behavior_to_cpp(behavior: &BehaviorIntent) -> Option<CppProjection> {
    let intent = match behavior.intent.as_str() {
        INTENT_REACT_HAPPILY_TO_TOUCH => CharacterIntent::Play,
        INTENT_SETTLE => CharacterIntent::Rest,
        INTENT_IDLE => CharacterIntent::Idle,
        // celebrate 由既有 Runtime 真相投影負責（verified-success），這裡不重複產生。
        INTENT_CELEBRATE => return None,
        _ => return None,
    };
    let intensity = crate::state::clamp_unit(behavior.intensity);
    Some(CppProjection {
        intent,
        variant: behavior.intent.clone(),
        parameters: json!({"intensity": intensity}),
        // priority = max(requested, floor)：投影出來的三個 intent 的 floor 都是 0。
        priority: RUNTIME_PROJECTION_PRIORITY.max(priority_floor(intent)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IntentOrigin;
    use chrono::{TimeZone, Utc};

    fn behavior(name: &str) -> BehaviorIntent {
        BehaviorIntent {
            intent: name.to_string(),
            intensity: 0.5,
            interruptible: true,
            origin: IntentOrigin::Interaction,
            hints: serde_json::Map::new(),
            correlation_id: "flow_1".to_string(),
            expires_at: Utc
                .with_ymd_and_hms(2026, 9, 4, 12, 30, 10)
                .single()
                .expect("fixed timestamp"),
        }
    }

    #[test]
    fn projection_never_emits_a_safety_intent() {
        for name in [INTENT_REACT_HAPPILY_TO_TOUCH, INTENT_SETTLE, INTENT_IDLE] {
            let projected = behavior_to_cpp(&behavior(name)).expect("projected");
            assert!(
                !projected.intent.is_safety(),
                "{name} 投影成安全 intent 會讓 adapter 取得它不該有的權限"
            );
            assert!(projected.intent.ai_allowed());
        }
    }
}
