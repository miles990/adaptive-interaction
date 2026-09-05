//! §3 語意狀態（`SemanticState`）與它的值物件。
//!
//! JSON 形狀是 `docs/aip/character-session.md` §3 的權威來源：`SemanticState` 的 serde 輸出
//! **就是** snapshot 的 `state`，`state_hash` 直接對它取 canonical SHA-256。
//!
//! 實作註記（見 crate 文件「與契約的落差」）：
//! - `Party` 在 `attention.id` 與 `lastInteraction.source` 兩處以 `"<kind>:<id>"` 字串出現（§3 範例
//!   `"device:iphone-…"`），在 `members[].party` 以物件出現（§3 範例同樣是物件）。
//! - 值為「無」的選填鍵一律**省略**，不寫 `null`：RFC 7396 的 `null` 代表刪除鍵，host 若寫 `null`
//!   而接收端刪除鍵，兩邊 canonical hash 會不一致。

use interaction_aip::{
    IntentSupport, MemberRole, NegotiatedCapabilities, Party, PartyKind, Timestamp,
};
use interaction_character::TruthState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// §3 `mood.kind` 詞彙（7）。
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MoodKind {
    #[default]
    Neutral,
    Happy,
    Playful,
    Proud,
    Tired,
    Alert,
    Down,
}

impl MoodKind {
    pub const ALL: [MoodKind; 7] = [
        MoodKind::Neutral,
        MoodKind::Happy,
        MoodKind::Playful,
        MoodKind::Proud,
        MoodKind::Tired,
        MoodKind::Alert,
        MoodKind::Down,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            MoodKind::Neutral => "neutral",
            MoodKind::Happy => "happy",
            MoodKind::Playful => "playful",
            MoodKind::Proud => "proud",
            MoodKind::Tired => "tired",
            MoodKind::Alert => "alert",
            MoodKind::Down => "down",
        }
    }
}

/// §3 `activity` 詞彙（7）。
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Activity {
    #[default]
    Idle,
    Reacting,
    Working,
    Waiting,
    Celebrating,
    Resting,
    Frozen,
}

impl Activity {
    pub const ALL: [Activity; 7] = [
        Activity::Idle,
        Activity::Reacting,
        Activity::Working,
        Activity::Waiting,
        Activity::Celebrating,
        Activity::Resting,
        Activity::Frozen,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Activity::Idle => "idle",
            Activity::Reacting => "reacting",
            Activity::Working => "working",
            Activity::Waiting => "waiting",
            Activity::Celebrating => "celebrating",
            Activity::Resting => "resting",
            Activity::Frozen => "frozen",
        }
    }
}

/// Session membership 的 presence（§2 由 Session Host 擁有）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Presence {
    Online,
    Reconnecting,
    Offline,
}

impl Presence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Presence::Online => "online",
            Presence::Reconnecting => "reconnecting",
            Presence::Offline => "offline",
        }
    }
}

/// §3 `mood`。`intensity` 恆為 0..=1 且四捨五入到 3 位小數（保證 canonical JSON 穩定）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Mood {
    pub kind: MoodKind,
    pub intensity: f64,
}

impl Mood {
    pub fn new(kind: MoodKind, intensity: f64) -> Self {
        Self {
            kind,
            intensity: clamp_unit(intensity),
        }
    }
}

/// 把任意 f64 夾進 0..=1 並四捨五入到 3 位小數；非有限值退回 0.0。
///
/// **負零正規化**：`f64` 有兩個零，`(-0.0).clamp(0.0, 1.0)` 與 `(-0.0 * 1000.0).round() / 1000.0`
/// 都原樣回 `-0.0`，而 serde_json 會把它寫成字面 `-0.0`。`-0.0` 與 `0.0` 語意相同、canonical
/// 文字不同，state hash 就跟著不同（AIP §6 的 hash 是對序列化文字取的），所以這裡把兩個零
/// 收斂成 `0.0`——host 永遠不產生 `-0.0`，接收端也就不必猜哪一個才算數。
pub fn clamp_unit(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let clamped = value.clamp(0.0, 1.0);
    let rounded = (clamped * 1000.0).round() / 1000.0;
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

/// §3 `truth`。**只有** Runtime 的真相事件能改；Session 只轉錄，不推論。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TruthView {
    pub state: TruthState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

impl Default for TruthView {
    fn default() -> Self {
        Self {
            state: TruthState::None,
            correlation_id: None,
        }
    }
}

/// §3 `attention`。序列化為 internally tagged：`{"kind":"none"}`、
/// `{"kind":"member","id":"device:…"}`、`{"kind":"task","correlationId":"…"}`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Attention {
    #[default]
    None,
    Member {
        #[serde(rename = "id", with = "party_ref")]
        #[schemars(with = "String")]
        party: Party,
    },
    Task {
        #[serde(rename = "correlationId")]
        correlation_id: String,
    },
}

/// §3 `lastInteraction`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LastInteraction {
    pub name: String,
    pub kind: String,
    #[serde(with = "party_ref")]
    #[schemars(with = "String")]
    pub source: Party,
    pub at: Timestamp,
}

/// §3 `members[]`：共享狀態裡的成員投影。
///
/// **不含**協商結果的細節（`NegotiatedCapabilities` 是 host 私有），只投影一件所有人都需要
/// 知道的事實：這個成員把哪些 Behavior Intent 協商成 `unsupported`。沒有它，桌面與 iPhone
/// 只能顯示「能力核對中」——契約 §11 的「部分能力目前不可用」永遠不會被觸發。
/// 沒有任何不支援的 intent 時是**空陣列**（不是缺鍵），這樣接收端不必區分「都支援」與「不知道」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberView {
    pub party: Party,
    pub role: MemberRole,
    pub presence: Presence,
    pub last_seen_at: Timestamp,
    /// 協商為 `unsupported` 的 intent 名（排序穩定：來自 `BTreeMap`）。
    /// `default` 是為了讓這個欄位出現之前寫下的持久化 snapshot 仍然還原得回來。
    #[serde(default)]
    pub unsupported_intents: Vec<String>,
}

/// Host 私有的成員紀錄（含協商結果）。不進 `SemanticState`。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub party: Party,
    pub role: MemberRole,
    pub presence: Presence,
    pub last_seen_at: Timestamp,
    pub negotiated: NegotiatedCapabilities,
}

impl Member {
    pub fn view(&self) -> MemberView {
        MemberView {
            party: self.party.clone(),
            role: self.role,
            presence: self.presence,
            last_seen_at: self.last_seen_at,
            unsupported_intents: self
                .negotiated
                .intents
                .iter()
                .filter(|(_, support)| **support == IntentSupport::Unsupported)
                .map(|(name, _)| name.clone())
                .collect(),
        }
    }
}

/// §3 權威語意狀態。欄位是 `pub(crate)`：**只有** [`crate::CharacterSession`] 能改
/// （§2「`SemanticState` 只有 `CharacterSession::apply` 能改」），對外只有 getter 與 `Serialize`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticState {
    pub(crate) character_id: String,
    pub(crate) mood: Mood,
    pub(crate) activity: Activity,
    pub(crate) attention: Attention,
    pub(crate) truth: TruthView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_interaction: Option<LastInteraction>,
    pub(crate) members: Vec<MemberView>,
    pub(crate) reduced_motion: bool,
}

impl SemanticState {
    /// 全新 session 的初始狀態：neutral／idle／attention none／truth none／無成員。
    pub fn new(character_id: impl Into<String>) -> Self {
        Self {
            character_id: character_id.into(),
            mood: Mood::default(),
            activity: Activity::Idle,
            attention: Attention::None,
            truth: TruthView::default(),
            last_interaction: None,
            members: Vec::new(),
            reduced_motion: false,
        }
    }

    pub fn character_id(&self) -> &str {
        &self.character_id
    }
    pub fn mood(&self) -> &Mood {
        &self.mood
    }
    pub fn activity(&self) -> Activity {
        self.activity
    }
    pub fn attention(&self) -> &Attention {
        &self.attention
    }
    pub fn truth(&self) -> &TruthView {
        &self.truth
    }
    pub fn last_interaction(&self) -> Option<&LastInteraction> {
        self.last_interaction.as_ref()
    }
    pub fn members(&self) -> &[MemberView] {
        &self.members
    }
    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    /// 不可信來源（snapshot 檔案、patch 後的 JSON）是否違反不變量。違反就拒絕，不「幫忙修正」。
    ///
    /// `is_sign_negative` 這一條擋的是 `-0.0`：它落在 `0.0..=1.0` 之內（`-0.0 == 0.0`），
    /// 但 host 的產生路徑（[`clamp_unit`]）永遠寫不出它。放行就等於把一個本實作寫不出來的
    /// canonical 字面當成權威狀態廣播出去；「幫忙修正」成 `0.0` 則會讓還原後的狀態與
    /// snapshot 的 hash 對不上。兩條都不誠實，所以拒絕。
    pub(crate) fn violates_limits(&self, max_members: usize) -> bool {
        !self.mood.intensity.is_finite()
            || self.mood.intensity.is_sign_negative()
            || !(0.0..=1.0).contains(&self.mood.intensity)
            || self.members.len() > max_members
    }
}

/// `"<kind>:<id>"` 形式的 Party 參照（§3 範例 `"device:iphone-…"`）。
pub fn format_party(party: &Party) -> String {
    format!("{}:{}", party_kind_str(&party.kind), party.id)
}

/// 解析 `"<kind>:<id>"`；格式不符回 `None`（不猜）。
pub fn parse_party(value: &str) -> Option<Party> {
    let (kind, id) = value.split_once(':')?;
    if kind.is_empty() || id.is_empty() {
        return None;
    }
    let kind: PartyKind =
        serde_json::from_value(serde_json::Value::String(kind.to_string())).ok()?;
    Some(Party::new(kind, id))
}

fn party_kind_str(kind: &PartyKind) -> String {
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "unknown".to_string(),
    }
}

mod party_ref {
    use super::{format_party, parse_party};
    use interaction_aip::Party;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(party: &Party, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format_party(party))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Party, D::Error> {
        let raw = String::deserialize(deserializer)?;
        parse_party(&raw)
            .ok_or_else(|| serde::de::Error::custom("party reference must be <kind>:<id>"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn t0() -> Timestamp {
        Utc.with_ymd_and_hms(2026, 9, 4, 12, 30, 0)
            .single()
            .expect("fixed timestamp")
    }

    #[test]
    fn party_reference_round_trips() {
        for party in [
            Party::device("iphone-1"),
            Party::renderer("desktop"),
            Party::human_surface("desktop"),
            Party::runtime(),
        ] {
            let text = format_party(&party);
            assert_eq!(parse_party(&text), Some(party));
        }
        assert_eq!(parse_party("nope"), None);
        assert_eq!(parse_party("device:"), None);
        assert_eq!(parse_party(":id"), None);
    }

    #[test]
    fn state_json_matches_the_documented_shape() {
        let mut state = SemanticState::new("ref-shape");
        state.mood = Mood::new(MoodKind::Happy, 0.45);
        state.activity = Activity::Reacting;
        state.attention = Attention::Member {
            party: Party::device("iphone-1"),
        };
        state.last_interaction = Some(LastInteraction {
            name: "character.interaction.touch".into(),
            kind: "tap".into(),
            source: Party::device("iphone-1"),
            at: t0(),
        });
        state.members.push(MemberView {
            party: Party::device("iphone-1"),
            role: MemberRole::RemoteRenderer,
            presence: Presence::Online,
            last_seen_at: t0(),
            unsupported_intents: Vec::new(),
        });
        let value = serde_json::to_value(&state).expect("serialize");
        assert_eq!(value["characterId"], json!("ref-shape"));
        assert_eq!(value["mood"], json!({"kind": "happy", "intensity": 0.45}));
        assert_eq!(value["activity"], json!("reacting"));
        assert_eq!(
            value["attention"],
            json!({"kind": "member", "id": "device:iphone-1"})
        );
        assert_eq!(
            value["truth"],
            json!({"state": "none"}),
            "None 的鍵省略而非 null"
        );
        assert_eq!(value["lastInteraction"]["source"], json!("device:iphone-1"));
        assert_eq!(
            value["members"][0]["party"],
            json!({"kind": "device", "id": "iphone-1"})
        );
        assert_eq!(value["reducedMotion"], json!(false));
        let back: SemanticState = serde_json::from_value(value).expect("round trip");
        assert_eq!(back, state);
    }

    #[test]
    fn intensity_is_clamped_and_stable() {
        assert_eq!(clamp_unit(-1.0), 0.0);
        assert_eq!(clamp_unit(9.0), 1.0);
        assert_eq!(clamp_unit(f64::NAN), 0.0);
        assert_eq!(clamp_unit(0.4567), 0.457);
        let m = Mood::new(MoodKind::Playful, 0.123456);
        let text = serde_json::to_string(&m).expect("serialize");
        let back: Mood = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(
            serde_json::to_string(&back).expect("re-serialize"),
            text,
            "canonical JSON 必須在 round-trip 後完全一致"
        );
    }

    #[test]
    fn attention_variants_serialize_as_documented() {
        assert_eq!(
            serde_json::to_value(Attention::None).expect("serialize"),
            json!({"kind": "none"})
        );
        assert_eq!(
            serde_json::to_value(Attention::Task {
                correlation_id: "c1".into()
            })
            .expect("serialize"),
            json!({"kind": "task", "correlationId": "c1"})
        );
    }

    /// `violates_limits` 是 crate 私有的守門員（`CharacterSession::restore`／patch 套用都走它），
    /// 這裡直接釘住它對兩個零的判斷：`0.0` 放行、`-0.0` 拒絕。
    #[test]
    fn violates_limits_rejects_a_sign_negative_zero_intensity() {
        let mut state = SemanticState::new("ref-shape");
        state.mood = Mood {
            kind: MoodKind::Neutral,
            intensity: 0.0,
        };
        assert!(!state.violates_limits(16));
        // serde 反序列化不走 `Mood::new`，所以不可信來源真的送得進 `-0.0`。
        state.mood = Mood {
            kind: MoodKind::Neutral,
            intensity: -0.0,
        };
        assert!(
            state.violates_limits(16),
            "-0.0 落在 0.0..=1.0 之內，但 host 寫不出它：必須拒絕"
        );
    }

    #[test]
    fn clamp_unit_never_returns_a_sign_negative_zero() {
        for input in [-0.0_f64, -1.0, -0.0004, f64::NEG_INFINITY, f64::NAN] {
            let out = clamp_unit(input);
            assert_eq!(out, 0.0);
            assert!(
                !out.is_sign_negative(),
                "clamp_unit({input}) 回了 sign-negative 的零"
            );
        }
        assert_eq!(clamp_unit(0.1234), 0.123);
        assert_eq!(clamp_unit(2.0), 1.0);
    }
}
