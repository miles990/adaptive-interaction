//! §4 語意事件目錄的 Director：**純函式**，把一則語意事件或一個 Runtime 真相事實
//! 轉成「對 `SemanticState` 的 RFC 7396 patch」＋「要發出的 Behavior Intent」。
//!
//! Director 不碰 revision／sequence／membership／安全檢查，那些全在 [`crate::CharacterSession`]。
//!
//! 實作註記：Director 產生的 patch **一律寫滿整個子物件**（不存在的選填鍵寫 `null`），
//! 這樣 RFC 7396 的合併語意才會真的把舊值蓋掉；對外廣播的 patch 由 Session 以
//! `merge_diff(舊狀態, 新狀態)` 重新算，因此不會把這些 `null` 送上線。

use chrono::Duration;
use interaction_aip::{Party, Timestamp};
use interaction_character::TruthState;
use serde_json::{json, Map, Value};

use crate::state::{clamp_unit, format_party, SemanticState};
use crate::types::{
    BehaviorIntent, IntentOrigin, RuntimeFact, SessionConfig, EVENT_DISMISS, EVENT_TOUCH,
    INTENT_CELEBRATE, INTENT_REACT_HAPPILY_TO_TOUCH, INTENT_SETTLE,
};

/// 已通過安全管線、可交給 Director 的互動事件。
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionEvent {
    /// `character.interaction.touch` 或 `character.interaction.dismiss`。
    pub name: String,
    /// `payload.kind`（touch：tap／longpress／pat／stroke；dismiss 無意義，慣例填 `dismiss`）。
    pub kind: String,
    /// `payload.intensity`；`None` 或非有限值 → 0.5。
    pub intensity: Option<f64>,
    /// 已綁定的來源身分。
    pub source: Party,
    /// 事件的 correlationId（沒有就用 messageId）。
    pub correlation_id: String,
    /// Host 時鐘的處理時間（不是 `occurredAt`）。
    pub at: Timestamp,
}

/// §4 觸摸種類。未知種類不猜（回 `None`，Session 回 `rejected{schema-invalid}`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchKind {
    Tap,
    LongPress,
    Pat,
    Stroke,
}

impl TouchKind {
    pub const ALL: [TouchKind; 4] = [
        TouchKind::Tap,
        TouchKind::LongPress,
        TouchKind::Pat,
        TouchKind::Stroke,
    ];

    pub fn parse(value: &str) -> Option<TouchKind> {
        match value {
            "tap" => Some(TouchKind::Tap),
            "longpress" => Some(TouchKind::LongPress),
            "pat" => Some(TouchKind::Pat),
            "stroke" => Some(TouchKind::Stroke),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TouchKind::Tap => "tap",
            TouchKind::LongPress => "longpress",
            TouchKind::Pat => "pat",
            TouchKind::Stroke => "stroke",
        }
    }

    /// §4：longpress／pat／stroke → playful，tap → happy。
    fn mood(&self) -> &'static str {
        match self {
            TouchKind::Tap => "happy",
            TouchKind::LongPress | TouchKind::Pat | TouchKind::Stroke => "playful",
        }
    }
}

/// 預設互動強度（payload 沒帶 `intensity` 時）。
pub const DEFAULT_INTERACTION_INTENSITY: f64 = 0.5;
/// `settle` 的固定強度。
const SETTLE_INTENSITY: f64 = 0.3;
/// `celebrate` 的固定強度。
const CELEBRATE_INTENSITY: f64 = 0.8;

/// §4：把互動事件套成 patch＋Behavior Intent。未知 name 回 `None`（Session 回 `unknown-name`）。
pub fn react(
    state: &SemanticState,
    event: &InteractionEvent,
    config: &SessionConfig,
    now: Timestamp,
) -> Option<(Value, Vec<BehaviorIntent>)> {
    // emergency 中不產生任何演出（Session 會更早就 `rejected{scope-denied}`；這裡是第二道）。
    if state.truth().state == TruthState::Emergency {
        return None;
    }
    let expires_at = now + Duration::milliseconds(config.intent_ttl_ms);
    let intensity = event
        .intensity
        .filter(|v| v.is_finite())
        .map(clamp_unit)
        .unwrap_or(DEFAULT_INTERACTION_INTENSITY);
    let last_interaction = json!({
        "name": event.name,
        "kind": event.kind,
        "source": format_party(&event.source),
        "at": event.at,
    });
    match event.name.as_str() {
        EVENT_TOUCH => {
            let kind = TouchKind::parse(&event.kind)?;
            let patch = json!({
                "mood": {"kind": kind.mood(), "intensity": intensity},
                "activity": "reacting",
                "attention": {
                    "kind": "member",
                    "id": format_party(&event.source),
                    "correlationId": Value::Null,
                },
                "lastInteraction": last_interaction,
            });
            let mut hints = Map::new();
            hints.insert("haptic".into(), Value::String("light".into()));
            hints.insert("touchKind".into(), Value::String(kind.as_str().to_string()));
            Some((
                patch,
                vec![BehaviorIntent {
                    intent: INTENT_REACT_HAPPILY_TO_TOUCH.to_string(),
                    intensity,
                    interruptible: true,
                    origin: IntentOrigin::Interaction,
                    hints,
                    correlation_id: event.correlation_id.clone(),
                    expires_at,
                }],
            ))
        }
        EVENT_DISMISS => {
            let patch = json!({
                "activity": "resting",
                "attention": {"kind": "none", "id": Value::Null, "correlationId": Value::Null},
                "lastInteraction": last_interaction,
            });
            Some((
                patch,
                vec![BehaviorIntent {
                    intent: INTENT_SETTLE.to_string(),
                    intensity: SETTLE_INTENSITY,
                    interruptible: true,
                    origin: IntentOrigin::Interaction,
                    hints: Map::new(),
                    correlation_id: event.correlation_id.clone(),
                    expires_at,
                }],
            ))
        }
        _ => None,
    }
}

/// §4：Runtime 真相事實 → patch＋Behavior Intent。Session **只轉錄真相，不推論**。
///
/// emergency 守衛（第二道；第一道在 [`crate::CharacterSession::submit_runtime`]）：緊急停止期間
/// `task.*` 的真相轉錄一律回 `None`。`task.state{truth:"unknown"}` 會把 `truth` 寫成 `unknown`、
/// `activity` 寫回非 frozen，等於讓一個不相關的工作解除 emergency 守衛
/// （CLAUDE.md：AI 不可解除 emergency stop）。只有 `runtime.emergency{engaged:false}` 能離開。
pub fn on_fact(
    state: &SemanticState,
    fact: &RuntimeFact,
    correlation: Option<&str>,
    config: &SessionConfig,
    now: Timestamp,
) -> Option<(Value, Vec<BehaviorIntent>)> {
    if state.truth().state == TruthState::Emergency
        && matches!(
            fact,
            RuntimeFact::TaskState { .. } | RuntimeFact::TaskVerified { .. }
        )
    {
        return None;
    }
    let expires_at = now + Duration::milliseconds(config.intent_ttl_ms);
    match fact {
        RuntimeFact::TaskState {
            truth,
            correlation_id,
        } => {
            let cid = correlation_id.as_deref().or(correlation);
            let mut patch = Map::new();
            patch.insert("truth".into(), truth_value(*truth, cid));
            patch.insert("attention".into(), attention_task(cid));
            // §4 activity 對照表；表外的 truth 只轉錄，不動 activity（不推論）。
            if let Some(activity) = activity_for(*truth) {
                patch.insert("activity".into(), Value::String(activity.to_string()));
            }
            if *truth == TruthState::Failed {
                patch.insert("mood".into(), json!({"kind": "down", "intensity": 0.4}));
            }
            Some((Value::Object(patch), Vec::new()))
        }
        RuntimeFact::TaskVerified { correlation_id } => {
            let cid = Some(correlation_id.as_str());
            let patch = json!({
                "truth": truth_value(TruthState::Verified, cid),
                "mood": {"kind": "proud", "intensity": CELEBRATE_INTENSITY},
                "activity": "celebrating",
                "attention": attention_task(cid),
            });
            Some((
                patch,
                vec![BehaviorIntent {
                    intent: INTENT_CELEBRATE.to_string(),
                    intensity: CELEBRATE_INTENSITY,
                    interruptible: false,
                    origin: IntentOrigin::Truth,
                    hints: Map::new(),
                    correlation_id: correlation.unwrap_or(correlation_id.as_str()).to_string(),
                    expires_at,
                }],
            ))
        }
        RuntimeFact::Emergency { engaged } => {
            let patch = if *engaged {
                json!({
                    "truth": truth_value(TruthState::Emergency, None),
                    "activity": "frozen",
                    "attention": attention_task(None),
                })
            } else {
                json!({
                    "truth": truth_value(TruthState::None, None),
                    "activity": "idle",
                    "attention": attention_task(None),
                })
            };
            Some((patch, Vec::new()))
        }
        RuntimeFact::ReducedMotion(enabled) => {
            Some((json!({"reducedMotion": *enabled}), Vec::new()))
        }
    }
}

/// `activity: reacting` 逾時後回到 idle 的 patch（[`crate::CharacterSession::tick`] 用）。
pub fn settle_to_idle() -> Value {
    json!({"activity": "idle"})
}

fn truth_value(truth: TruthState, correlation_id: Option<&str>) -> Value {
    json!({
        "state": truth,
        "correlationId": correlation_id.map(|c| Value::String(c.to_string())).unwrap_or(Value::Null),
    })
}

fn attention_task(correlation_id: Option<&str>) -> Value {
    match correlation_id {
        Some(cid) => json!({"kind": "task", "id": Value::Null, "correlationId": cid}),
        None => json!({"kind": "none", "id": Value::Null, "correlationId": Value::Null}),
    }
}

/// §4 truth → activity 對照。表外的值回 `None`（activity 不變）。
fn activity_for(truth: TruthState) -> Option<&'static str> {
    match truth {
        TruthState::Working | TruthState::Queued => Some("working"),
        TruthState::WaitingInput | TruthState::WaitingConsent => Some("waiting"),
        TruthState::None => Some("idle"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_kind_vocabulary_is_closed() {
        for kind in TouchKind::ALL {
            assert_eq!(TouchKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(TouchKind::parse("poke"), None);
        assert_eq!(TouchKind::parse("TAP"), None);
    }

    #[test]
    fn activity_mapping_is_not_a_guess() {
        assert_eq!(activity_for(TruthState::Working), Some("working"));
        assert_eq!(activity_for(TruthState::WaitingInput), Some("waiting"));
        assert_eq!(activity_for(TruthState::WaitingConsent), Some("waiting"));
        assert_eq!(activity_for(TruthState::None), Some("idle"));
        // 表外的真相不推論 activity。
        assert_eq!(activity_for(TruthState::Unknown), None);
        assert_eq!(activity_for(TruthState::Claimed), None);
        assert_eq!(activity_for(TruthState::Verified), None);
    }
}
