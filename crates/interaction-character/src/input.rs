//! §6 Input events：13 種事件、Envelope 與 Gateway 強制的正規化／節流／隱私規則。
//!
//! 不保存原始游標軌跡、不送 AI；payload 不含絕對螢幕座標；`file-dropped` 只帶 metadata 與短效 grant；
//! `observer`／`notification-only` 的輸入永不轉送；沒有任何 event kind 能表達 human verification。

use crate::intent::PrivacyClass;
use crate::lifecycle::CharacterRole;
use crate::{char_len, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// 13 種 input event kind（wire 名稱帶 `character.` 前綴）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum InputEventKind {
    #[serde(rename = "character.clicked")]
    Clicked,
    #[serde(rename = "character.double-clicked")]
    DoubleClicked,
    #[serde(rename = "character.hover-entered")]
    HoverEntered,
    #[serde(rename = "character.hover-left")]
    HoverLeft,
    #[serde(rename = "character.drag-started")]
    DragStarted,
    #[serde(rename = "character.dragged")]
    Dragged,
    #[serde(rename = "character.dropped")]
    Dropped,
    #[serde(rename = "character.text-submitted")]
    TextSubmitted,
    #[serde(rename = "character.file-dropped")]
    FileDropped,
    #[serde(rename = "character.toy-thrown")]
    ToyThrown,
    #[serde(rename = "character.action-requested")]
    ActionRequested,
    #[serde(rename = "character.dismissed")]
    Dismissed,
    #[serde(rename = "character.visibility-changed")]
    VisibilityChanged,
}

impl InputEventKind {
    pub const ALL: [InputEventKind; 13] = [
        InputEventKind::Clicked,
        InputEventKind::DoubleClicked,
        InputEventKind::HoverEntered,
        InputEventKind::HoverLeft,
        InputEventKind::DragStarted,
        InputEventKind::Dragged,
        InputEventKind::Dropped,
        InputEventKind::TextSubmitted,
        InputEventKind::FileDropped,
        InputEventKind::ToyThrown,
        InputEventKind::ActionRequested,
        InputEventKind::Dismissed,
        InputEventKind::VisibilityChanged,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            InputEventKind::Clicked => "character.clicked",
            InputEventKind::DoubleClicked => "character.double-clicked",
            InputEventKind::HoverEntered => "character.hover-entered",
            InputEventKind::HoverLeft => "character.hover-left",
            InputEventKind::DragStarted => "character.drag-started",
            InputEventKind::Dragged => "character.dragged",
            InputEventKind::Dropped => "character.dropped",
            InputEventKind::TextSubmitted => "character.text-submitted",
            InputEventKind::FileDropped => "character.file-dropped",
            InputEventKind::ToyThrown => "character.toy-thrown",
            InputEventKind::ActionRequested => "character.action-requested",
            InputEventKind::Dismissed => "character.dismissed",
            InputEventKind::VisibilityChanged => "character.visibility-changed",
        }
    }

    /// 佇列滿時不丟的事件：使用者要角色消失／可見性變化／明確的動作請求。
    pub fn is_safety(&self) -> bool {
        matches!(
            self,
            InputEventKind::Dismissed
                | InputEventKind::VisibilityChanged
                | InputEventKind::ActionRequested
        )
    }
}

/// §6 事件 envelope。**沒有** `truthState`／verification 欄位。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CharacterInputEvent {
    pub protocol_version: String,
    pub event_id: String,
    pub character_instance_id: String,
    /// 連線世代；舊世代事件丟棄（記 audit）。
    pub generation: u64,
    pub timestamp: Timestamp,
    pub kind: InputEventKind,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub payload: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub privacy_class: PrivacyClass,
}

/// 正規化限制（§6）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InputLimits {
    /// `hover-*` ≤ 4/s。
    pub hover_min_interval_ms: i64,
    /// `dragged` 合併為 ≤ 10/s。
    pub drag_min_interval_ms: i64,
    /// 拖曳座標量化網格（px，視窗相對）。
    pub drag_grid_px: f64,
    /// `pointerProximity` ≤ 1/30 s。
    pub proximity_min_interval_ms: i64,
    /// 佇列上限；滿了丟最舊的非安全事件。
    pub queue_cap: usize,
    /// `text-submitted` ≤ 2000 字。
    pub max_text_chars: usize,
    /// file-drop grant ≤ 10 分鐘。
    pub max_grant_ttl_ms: i64,
}

impl Default for InputLimits {
    fn default() -> Self {
        InputLimits {
            hover_min_interval_ms: 250,
            drag_min_interval_ms: 100,
            drag_grid_px: 8.0,
            proximity_min_interval_ms: 30_000,
            queue_cap: 64,
            max_text_chars: 2000,
            max_grant_ttl_ms: 600_000,
        }
    }
}

/// 丟棄原因。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum InputDropReason {
    /// `observer`／`notification-only` 不送輸入。
    RoleFiltered,
    QueueFull,
    /// payload 含絕對螢幕座標鍵。
    AbsoluteCoordinates,
    /// payload 含原始檔案路徑／URI。
    RawPath,
    TextTooLong,
    /// grant 不是單一檔案範圍。
    GrantScope,
    ProtocolVersion,
    StaleGeneration,
    UnknownInstance,
    InvalidPayload {
        field: String,
    },
    /// 角色（manifest／協商結果）沒宣告這個 event kind 對應的輸入能力：宣告即契約，
    /// 沒宣告的輸入不進佇列、不算預算、不得變成 `companion.*` 觀察。
    CapabilityNotDeclared {
        requires: String,
    },
}

/// §6 event kind → 角色必須在 `manifest.inputCapabilities`（協商後為 `negotiated.inputCapabilities`）
/// 宣告的能力 id。連接頁對使用者說的「可以接收：…」直接由 manifest 產生，所以宣告就是契約：
/// 沒宣告的種類不得進入輸入佇列，也不得變成 `companion.*` 觀察或 file-drop grant
/// （對抗審查 character-protocol-040）。`hover-left`／`toy-thrown`／`dismissed`／
/// `visibility-changed` 本來就只留稽核、不產生觀察，因此不設閘。
pub fn required_input_capability(kind: InputEventKind) -> Option<&'static str> {
    match kind {
        InputEventKind::Clicked
        | InputEventKind::DoubleClicked
        | InputEventKind::ActionRequested => Some("input.click"),
        InputEventKind::HoverEntered => Some("input.hover"),
        InputEventKind::DragStarted | InputEventKind::Dragged => Some("input.drag"),
        InputEventKind::Dropped => Some("input.drop"),
        InputEventKind::TextSubmitted => Some("input.text"),
        InputEventKind::FileDropped => Some("input.fileDrop"),
        InputEventKind::HoverLeft
        | InputEventKind::ToyThrown
        | InputEventKind::Dismissed
        | InputEventKind::VisibilityChanged => None,
    }
}

/// 正規化決定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "decision", rename_all = "kebab-case")]
pub enum InputDecision {
    Queued,
    /// 併入佇列中最後一筆 `dragged`。
    Merged,
    Throttled,
    Dropped(InputDropReason),
}

impl InputDecision {
    pub fn is_queued(&self) -> bool {
        matches!(self, InputDecision::Queued | InputDecision::Merged)
    }
}

/// `action-requested` 轉成的 receptor observation（仍經 Runtime policy／consent）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QuickActionRequest {
    /// 固定 `companion.quick-action`。
    pub receptor: String,
    pub action: String,
    pub character_instance_id: String,
    pub event_id: String,
}

/// 統計（只計數，不存內容）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct InputStats {
    pub queued: u64,
    pub merged: u64,
    pub throttled: u64,
    pub dropped: u64,
}

const FORBIDDEN_COORD_KEYS: [&str; 10] = [
    "screenX",
    "screenY",
    "pageX",
    "pageY",
    "clientX",
    "clientY",
    "absoluteX",
    "absoluteY",
    "absX",
    "absY",
];
const FORBIDDEN_PATH_KEYS: [&str; 7] = [
    "path", "paths", "uri", "url", "filePath", "fullPath", "file",
];
const FILE_DROP_KEYS: [&str; 6] = [
    "name",
    "mediaType",
    "bytes",
    "readableScope",
    "grantId",
    "expiresAt",
];
/// `file-dropped` 的第二種 wire 形狀：TS Gateway（`apps/interaction-desktop/
/// src/character/gateway.ts` 的 `fileGrants`）把一次拖放送成
/// `{ files: [ {name, mediaType, bytes, readableScope, grantId, expiresAt}, … ] }`。
/// README §6 的扁平單檔形狀與這個列表形狀都收，規則相同：只有 metadata＋短效 grant。
const FILE_DROP_LIST_KEY: &str = "files";
/// 一次拖放最多幾個檔案（與 TS `LIMITS.fileDropMaxFiles` 一致）。
pub const FILE_DROP_MAX_FILES: usize = 16;

/// 有狀態、確定性的正規化器（時間由呼叫端以 millis 注入）。
#[derive(Debug, Clone)]
pub struct InputNormalizer {
    limits: InputLimits,
    role: CharacterRole,
    queue: VecDeque<CharacterInputEvent>,
    last_hover_ms: Option<i64>,
    last_drag_ms: Option<i64>,
    last_proximity_ms: Option<i64>,
    stats: InputStats,
}

impl InputNormalizer {
    pub fn new(role: CharacterRole, limits: InputLimits) -> Self {
        InputNormalizer {
            limits,
            role,
            queue: VecDeque::with_capacity(limits.queue_cap.min(64)),
            last_hover_ms: None,
            last_drag_ms: None,
            last_proximity_ms: None,
            stats: InputStats::default(),
        }
    }

    pub fn role(&self) -> CharacterRole {
        self.role
    }

    pub fn stats(&self) -> InputStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// 取出全部已正規化事件（FIFO）。
    pub fn drain(&mut self) -> Vec<CharacterInputEvent> {
        self.queue.drain(..).collect()
    }

    fn quantize(&self, v: f64) -> f64 {
        let grid = self.limits.drag_grid_px.max(1.0);
        (v / grid).round() * grid
    }

    fn number(payload: &BTreeMap<String, serde_json::Value>, key: &str) -> Option<f64> {
        payload
            .get(key)
            .and_then(|v| v.as_f64())
            .filter(|n| n.is_finite())
    }

    fn drop(&mut self, reason: InputDropReason) -> InputDecision {
        self.stats.dropped += 1;
        InputDecision::Dropped(reason)
    }

    /// 一個檔案的 metadata＋短效 grant 正規化。扁平單檔形狀（README §6）與
    /// `files[]` 內的每一筆共用同一套規則：只認 `FILE_DROP_KEYS`、不收任何
    /// 路徑／絕對座標鍵、名字不得含路徑分隔符、scope 只能是單一檔案、grant 到期
    /// 一律夾到 `max_grant_ttl_ms`。輸出固定六個鍵。
    fn normalize_file_grant<'a>(
        &self,
        fields: impl Iterator<Item = (&'a String, &'a serde_json::Value)>,
        now_ms: i64,
    ) -> Result<BTreeMap<String, serde_json::Value>, InputDropReason> {
        let fields: BTreeMap<&str, &serde_json::Value> =
            fields.map(|(k, v)| (k.as_str(), v)).collect();
        if fields.keys().any(|k| FORBIDDEN_COORD_KEYS.contains(k)) {
            return Err(InputDropReason::AbsoluteCoordinates);
        }
        if fields.keys().any(|k| FORBIDDEN_PATH_KEYS.contains(k)) {
            return Err(InputDropReason::RawPath);
        }
        if let Some(extra) = fields.keys().find(|k| !FILE_DROP_KEYS.contains(k)) {
            return Err(InputDropReason::InvalidPayload {
                field: crate::truncate_for_echo(extra),
            });
        }
        let name = fields
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if name.is_empty()
            || char_len(name) > 255
            || name.contains('/')
            || name.contains('\\')
            || name.chars().any(|c| c.is_control())
        {
            return Err(InputDropReason::RawPath);
        }
        let media_type = fields
            .get("mediaType")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if media_type.is_empty() || !media_type.contains('/') || char_len(media_type) > 128 {
            return Err(InputDropReason::InvalidPayload {
                field: "mediaType".into(),
            });
        }
        let Some(bytes) = fields.get("bytes").and_then(|v| v.as_u64()) else {
            return Err(InputDropReason::InvalidPayload {
                field: "bytes".into(),
            });
        };
        if fields.get("readableScope").and_then(|v| v.as_str()) != Some("file") {
            return Err(InputDropReason::GrantScope);
        }
        let grant_id = fields
            .get("grantId")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if grant_id.is_empty() || char_len(grant_id) > 128 {
            return Err(InputDropReason::InvalidPayload {
                field: "grantId".into(),
            });
        }
        let max_expiry = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
            now_ms.saturating_add(self.limits.max_grant_ttl_ms),
        );
        let Some(max_expiry) = max_expiry else {
            return Err(InputDropReason::InvalidPayload {
                field: "expiresAt".into(),
            });
        };
        let requested = fields
            .get("expiresAt")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&chrono::Utc));
        let Some(requested) = requested else {
            return Err(InputDropReason::InvalidPayload {
                field: "expiresAt".into(),
            });
        };
        let expires_at = requested.min(max_expiry);
        let mut clean = BTreeMap::new();
        clean.insert("name".to_string(), serde_json::json!(name));
        clean.insert("mediaType".to_string(), serde_json::json!(media_type));
        clean.insert("bytes".to_string(), serde_json::json!(bytes));
        clean.insert("readableScope".to_string(), serde_json::json!("file"));
        clean.insert("grantId".to_string(), serde_json::json!(grant_id));
        clean.insert(
            "expiresAt".to_string(),
            serde_json::json!(expires_at.to_rfc3339()),
        );
        Ok(clean)
    }

    /// 正規化並排入佇列。
    pub fn push(&mut self, event: CharacterInputEvent, now_ms: i64) -> InputDecision {
        if !self.role.accepts_input() {
            return self.drop(InputDropReason::RoleFiltered);
        }
        if event
            .payload
            .keys()
            .any(|k| FORBIDDEN_COORD_KEYS.contains(&k.as_str()))
        {
            return self.drop(InputDropReason::AbsoluteCoordinates);
        }
        if event
            .payload
            .keys()
            .any(|k| FORBIDDEN_PATH_KEYS.contains(&k.as_str()))
        {
            return self.drop(InputDropReason::RawPath);
        }
        let mut event = event;
        let mut clean: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        match event.kind {
            InputEventKind::HoverEntered | InputEventKind::HoverLeft => {
                let proximity = Self::number(&event.payload, "proximity");
                let (last, interval) = if proximity.is_some() {
                    (
                        self.last_proximity_ms,
                        self.limits.proximity_min_interval_ms,
                    )
                } else {
                    (self.last_hover_ms, self.limits.hover_min_interval_ms)
                };
                if let Some(last) = last {
                    if now_ms - last < interval {
                        self.stats.throttled += 1;
                        return InputDecision::Throttled;
                    }
                }
                if let Some(p) = proximity {
                    self.last_proximity_ms = Some(now_ms);
                    clean.insert("proximity".into(), serde_json::json!(p.clamp(0.0, 1.0)));
                } else {
                    self.last_hover_ms = Some(now_ms);
                }
            }
            InputEventKind::Dragged => {
                let (Some(x), Some(y)) = (
                    Self::number(&event.payload, "x"),
                    Self::number(&event.payload, "y"),
                ) else {
                    return self.drop(InputDropReason::InvalidPayload {
                        field: "x/y".into(),
                    });
                };
                clean.insert("x".into(), serde_json::json!(self.quantize(x)));
                clean.insert("y".into(), serde_json::json!(self.quantize(y)));
                if let Some(last) = self.last_drag_ms {
                    if now_ms - last < self.limits.drag_min_interval_ms {
                        if let Some(prev) = self
                            .queue
                            .iter_mut()
                            .rev()
                            .find(|e| e.kind == InputEventKind::Dragged)
                        {
                            prev.payload = clean;
                            prev.timestamp = event.timestamp;
                            prev.event_id = event.event_id;
                            self.stats.merged += 1;
                            return InputDecision::Merged;
                        }
                        self.stats.throttled += 1;
                        return InputDecision::Throttled;
                    }
                }
                self.last_drag_ms = Some(now_ms);
            }
            InputEventKind::Clicked
            | InputEventKind::DoubleClicked
            | InputEventKind::DragStarted
            | InputEventKind::Dropped => {
                if let (Some(x), Some(y)) = (
                    Self::number(&event.payload, "x"),
                    Self::number(&event.payload, "y"),
                ) {
                    clean.insert("x".into(), serde_json::json!(self.quantize(x)));
                    clean.insert("y".into(), serde_json::json!(self.quantize(y)));
                }
            }
            InputEventKind::TextSubmitted => {
                let Some(text) = event.payload.get("text").and_then(|v| v.as_str()) else {
                    return self.drop(InputDropReason::InvalidPayload {
                        field: "text".into(),
                    });
                };
                if char_len(text) > self.limits.max_text_chars {
                    return self.drop(InputDropReason::TextTooLong);
                }
                clean.insert("text".into(), serde_json::json!(text));
                event.privacy_class = event.privacy_class.max(PrivacyClass::Personal);
            }
            InputEventKind::FileDropped => {
                if let Some(extra) = event.payload.keys().find(|k| {
                    !FILE_DROP_KEYS.contains(&k.as_str()) && k.as_str() != FILE_DROP_LIST_KEY
                }) {
                    return self.drop(InputDropReason::InvalidPayload {
                        field: crate::truncate_for_echo(extra),
                    });
                }
                if event.payload.contains_key(FILE_DROP_LIST_KEY) {
                    // 列表形狀（TS Gateway）：不得同時帶扁平鍵——兩個真相來源會互相矛盾。
                    if event.payload.len() != 1 {
                        return self.drop(InputDropReason::InvalidPayload {
                            field: FILE_DROP_LIST_KEY.into(),
                        });
                    }
                    let list = event
                        .payload
                        .get(FILE_DROP_LIST_KEY)
                        .and_then(|v| v.as_array())
                        .filter(|list| !list.is_empty() && list.len() <= FILE_DROP_MAX_FILES);
                    let Some(list) = list else {
                        return self.drop(InputDropReason::InvalidPayload {
                            field: FILE_DROP_LIST_KEY.into(),
                        });
                    };
                    let mut files = Vec::with_capacity(list.len());
                    for entry in list {
                        let Some(obj) = entry.as_object() else {
                            return self.drop(InputDropReason::InvalidPayload {
                                field: FILE_DROP_LIST_KEY.into(),
                            });
                        };
                        match self.normalize_file_grant(obj.iter(), now_ms) {
                            Ok(file) => files.push(file),
                            Err(reason) => return self.drop(reason),
                        }
                    }
                    // 第一個檔案同時以 README §6 的扁平鍵輸出：既有消費端（Runtime 的
                    // `companion.drag-drop` 觀察）讀的是扁平鍵；`files` 保留全部檔案。
                    if let Some(first) = files.first() {
                        clean.extend(first.iter().map(|(k, v)| (k.clone(), v.clone())));
                    }
                    clean.insert(
                        FILE_DROP_LIST_KEY.into(),
                        serde_json::Value::Array(
                            files
                                .into_iter()
                                .map(|file| serde_json::Value::Object(file.into_iter().collect()))
                                .collect(),
                        ),
                    );
                } else {
                    match self.normalize_file_grant(event.payload.iter(), now_ms) {
                        Ok(file) => clean.extend(file),
                        Err(reason) => return self.drop(reason),
                    }
                }
                event.privacy_class = event.privacy_class.max(PrivacyClass::Personal);
            }
            InputEventKind::ToyThrown => {
                if let Some(toy) = event.payload.get("toyId").and_then(|v| v.as_str()) {
                    if char_len(toy) > 64 {
                        return self.drop(InputDropReason::InvalidPayload {
                            field: "toyId".into(),
                        });
                    }
                    clean.insert("toyId".into(), serde_json::json!(toy));
                }
                for key in ["x", "y"] {
                    if let Some(v) = Self::number(&event.payload, key) {
                        clean.insert(key.into(), serde_json::json!(self.quantize(v)));
                    }
                }
                for key in ["vx", "vy"] {
                    if let Some(v) = Self::number(&event.payload, key) {
                        clean.insert(key.into(), serde_json::json!(v.clamp(-10_000.0, 10_000.0)));
                    }
                }
            }
            InputEventKind::ActionRequested => {
                let action = event
                    .payload
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if !crate::manifest::is_valid_character_id(action) {
                    return self.drop(InputDropReason::InvalidPayload {
                        field: "action".into(),
                    });
                }
                clean.insert("action".into(), serde_json::json!(action));
            }
            InputEventKind::Dismissed => {}
            InputEventKind::VisibilityChanged => {
                let Some(visible) = event.payload.get("visible").and_then(|v| v.as_bool()) else {
                    return self.drop(InputDropReason::InvalidPayload {
                        field: "visible".into(),
                    });
                };
                clean.insert("visible".into(), serde_json::json!(visible));
            }
        }
        event.payload = clean;

        if self.queue.len() >= self.limits.queue_cap {
            if let Some(idx) = self.queue.iter().position(|e| !e.kind.is_safety()) {
                self.queue.remove(idx);
                self.stats.dropped += 1;
            } else {
                return self.drop(InputDropReason::QueueFull);
            }
        }
        self.queue.push_back(event);
        self.stats.queued += 1;
        InputDecision::Queued
    }
}

/// `action-requested` → `companion.quick-action` receptor observation（只是請求，仍經 policy／consent）。
pub fn quick_action_request(event: &CharacterInputEvent) -> Option<QuickActionRequest> {
    if event.kind != InputEventKind::ActionRequested {
        return None;
    }
    let action = event.payload.get("action")?.as_str()?;
    Some(QuickActionRequest {
        receptor: "companion.quick-action".to_string(),
        action: action.to_string(),
        character_instance_id: event.character_instance_id.clone(),
        event_id: event.event_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: InputEventKind, payload: serde_json::Value) -> CharacterInputEvent {
        CharacterInputEvent {
            protocol_version: crate::PROTOCOL_VERSION.into(),
            event_id: format!("e-{}", kind.as_str()),
            character_instance_id: "inst".into(),
            generation: 1,
            timestamp: Timestamp::default(),
            kind,
            payload: payload
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default(),
            privacy_class: PrivacyClass::Internal,
        }
    }

    #[test]
    fn kinds_serialize_with_prefix() {
        for kind in InputEventKind::ALL {
            let json = serde_json::to_string(&kind).unwrap_or_default();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            assert!(kind.as_str().starts_with("character."));
        }
    }

    #[test]
    fn hover_throttled_to_4_per_second() {
        let mut n = InputNormalizer::new(CharacterRole::PrimaryCompanion, InputLimits::default());
        let mut accepted = 0;
        for i in 0..20 {
            if n.push(
                ev(InputEventKind::HoverEntered, serde_json::json!({})),
                i * 50,
            ) == InputDecision::Queued
            {
                accepted += 1;
            }
        }
        // 1 s 內 20 筆（每 50 ms 一筆）→ 最多 4 筆。
        assert_eq!(accepted, 4);
    }

    #[test]
    fn proximity_throttled_to_one_per_30s() {
        let mut n = InputNormalizer::new(CharacterRole::PrimaryCompanion, InputLimits::default());
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::HoverEntered,
                    serde_json::json!({"proximity": 0.4})
                ),
                0
            ),
            InputDecision::Queued
        );
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::HoverEntered,
                    serde_json::json!({"proximity": 0.5})
                ),
                29_999
            ),
            InputDecision::Throttled
        );
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::HoverEntered,
                    serde_json::json!({"proximity": 0.5})
                ),
                30_000
            ),
            InputDecision::Queued
        );
    }

    #[test]
    fn dragged_is_merged_and_quantized() {
        let mut n = InputNormalizer::new(CharacterRole::PrimaryCompanion, InputLimits::default());
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::Dragged,
                    serde_json::json!({"x": 13.0, "y": 21.0})
                ),
                0
            ),
            InputDecision::Queued
        );
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::Dragged,
                    serde_json::json!({"x": 30.0, "y": 44.0})
                ),
                50
            ),
            InputDecision::Merged
        );
        let drained = n.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].payload["x"], 32.0);
        // 44 / 8 = 5.5 → 遠離零捨入 → 48。
        assert_eq!(drained[0].payload["y"], 48.0);
        // 佇列已空、又太快 → throttled（不是 merged）。
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::Dragged,
                    serde_json::json!({"x": 1.0, "y": 1.0})
                ),
                60
            ),
            InputDecision::Throttled
        );
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::Dragged,
                    serde_json::json!({"x": 1.0, "y": 1.0})
                ),
                100
            ),
            InputDecision::Queued
        );
    }

    #[test]
    fn absolute_coordinates_and_raw_paths_rejected() {
        let mut n = InputNormalizer::new(CharacterRole::PrimaryCompanion, InputLimits::default());
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::Clicked,
                    serde_json::json!({"x": 1, "y": 2, "screenX": 900})
                ),
                0
            ),
            InputDecision::Dropped(InputDropReason::AbsoluteCoordinates)
        );
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::FileDropped,
                    serde_json::json!({"path": "/Users/x/secret.txt"})
                ),
                0
            ),
            InputDecision::Dropped(InputDropReason::RawPath)
        );
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::FileDropped,
                    serde_json::json!({"name": "../x.txt", "mediaType": "text/plain", "bytes": 1,
                        "readableScope": "file", "grantId": "g", "expiresAt": "2026-09-02T12:00:00Z"})
                ),
                0
            ),
            InputDecision::Dropped(InputDropReason::RawPath)
        );
    }

    #[test]
    fn text_limit_and_privacy() {
        let mut n = InputNormalizer::new(CharacterRole::PrimaryCompanion, InputLimits::default());
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::TextSubmitted,
                    serde_json::json!({"text": "字".repeat(2001)})
                ),
                0
            ),
            InputDecision::Dropped(InputDropReason::TextTooLong)
        );
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::TextSubmitted,
                    serde_json::json!({"text": "字".repeat(2000), "junk": 1})
                ),
                0
            ),
            InputDecision::Queued
        );
        let e = n.drain().remove(0);
        assert_eq!(e.privacy_class, PrivacyClass::Personal);
        assert!(!e.payload.contains_key("junk"));
    }

    #[test]
    fn file_drop_is_metadata_only_with_short_grant() {
        let mut n = InputNormalizer::new(CharacterRole::PrimaryCompanion, InputLimits::default());
        let now_ms = 1_000_000;
        let decision = n.push(
            ev(
                InputEventKind::FileDropped,
                serde_json::json!({"name": "report.pdf", "mediaType": "application/pdf", "bytes": 1234,
                    "readableScope": "file", "grantId": "grant-1", "expiresAt": "2030-01-01T00:00:00Z"}),
            ),
            now_ms,
        );
        assert_eq!(decision, InputDecision::Queued);
        let e = n.drain().remove(0);
        let expires = chrono::DateTime::parse_from_rfc3339(
            e.payload["expiresAt"].as_str().unwrap_or_default(),
        )
        .map(|d| d.timestamp_millis())
        .unwrap_or_default();
        assert_eq!(expires, now_ms + 600_000);
        assert_eq!(e.payload.len(), 6);
        // 整個檔案系統的 scope 拒絕。
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::FileDropped,
                    serde_json::json!({"name": "a.txt", "mediaType": "text/plain", "bytes": 1,
                        "readableScope": "filesystem", "grantId": "g", "expiresAt": "2030-01-01T00:00:00Z"})
                ),
                now_ms
            ),
            InputDecision::Dropped(InputDropReason::GrantScope)
        );
    }

    /// Regression (character-protocol-028): the TS Gateway sends
    /// `character.file-dropped` as `{ files: [grant, …] }` (see
    /// `apps/interaction-desktop/src/test/character-gateway.test.ts`); the
    /// normalizer used to drop that whole shape as `invalid-payload{files}`,
    /// so desktop drag-and-drop under the v0.5 daemon always failed.
    #[test]
    fn file_drop_accepts_ts_gateway_files_shape() {
        let mut n = InputNormalizer::new(CharacterRole::PrimaryCompanion, InputLimits::default());
        let now_ms = 1_000_000;
        // Exactly what the TS Gateway emits for a 2-file drop (names already
        // basenamed, unknown media types defaulted, bytes clamped).
        let ts_payload = serde_json::json!({
            "files": [
                { "name": "secret.pdf", "mediaType": "application/pdf", "bytes": 1234,
                  "readableScope": "file", "grantId": "grant-1", "expiresAt": "2030-01-01T00:00:00Z" },
                { "name": "y.png", "mediaType": "application/octet-stream", "bytes": 0,
                  "readableScope": "file", "grantId": "grant-2", "expiresAt": "2030-01-01T00:00:00Z" }
            ]
        });
        assert_eq!(
            n.push(ev(InputEventKind::FileDropped, ts_payload), now_ms),
            InputDecision::Queued
        );
        let e = n.drain().remove(0);
        assert_eq!(e.privacy_class, PrivacyClass::Personal);
        // README §6 flat keys carry the first file (the runtime's
        // `companion.drag-drop` observation reads these) …
        assert_eq!(e.payload["name"], "secret.pdf");
        assert_eq!(e.payload["grantId"], "grant-1");
        assert_eq!(e.payload["readableScope"], "file");
        // … and `files` keeps every file, each normalized to the six keys with
        // the grant expiry clamped to max_grant_ttl_ms (10 min), never 2030.
        let files = e.payload["files"].as_array().expect("files array");
        assert_eq!(files.len(), 2);
        for (file, name) in files.iter().zip(["secret.pdf", "y.png"]) {
            let obj = file.as_object().expect("file object");
            assert_eq!(obj.len(), 6);
            assert_eq!(obj["name"], name);
            let expires =
                chrono::DateTime::parse_from_rfc3339(obj["expiresAt"].as_str().unwrap_or_default())
                    .map(|d| d.timestamp_millis())
                    .unwrap_or_default();
            assert_eq!(expires, now_ms + 600_000);
        }
        assert_eq!(e.payload.len(), 7);
        assert!(!serde_json::to_string(&e.payload)
            .unwrap_or_default()
            .contains("/Users/"));
    }

    /// The list shape gets the same metadata-only rules as the flat shape:
    /// path keys, whole-filesystem scope, unknown keys, empty/oversized lists,
    /// non-object entries and mixing both shapes are all rejected.
    #[test]
    fn file_drop_files_shape_keeps_metadata_only_rules() {
        let mut n = InputNormalizer::new(CharacterRole::PrimaryCompanion, InputLimits::default());
        let now_ms = 1_000_000;
        let good = serde_json::json!({ "name": "a.txt", "mediaType": "text/plain", "bytes": 1,
            "readableScope": "file", "grantId": "g", "expiresAt": "2030-01-01T00:00:00Z" });
        let with = |patch: serde_json::Value| {
            let mut f = good.clone();
            for (k, v) in patch.as_object().expect("patch object") {
                f[k] = v.clone();
            }
            f
        };
        let push = |n: &mut InputNormalizer, payload: serde_json::Value| {
            n.push(ev(InputEventKind::FileDropped, payload), now_ms)
        };
        // A raw path inside an entry is still a raw path.
        assert_eq!(
            push(
                &mut n,
                serde_json::json!({ "files": [with(serde_json::json!({"path": "/Users/x/a.txt"}))] })
            ),
            InputDecision::Dropped(InputDropReason::RawPath)
        );
        assert_eq!(
            push(
                &mut n,
                serde_json::json!({ "files": [with(serde_json::json!({"name": "dir/a.txt"}))] })
            ),
            InputDecision::Dropped(InputDropReason::RawPath)
        );
        // Whole-filesystem scope in the second entry rejects the whole drop.
        assert_eq!(
            push(
                &mut n,
                serde_json::json!({ "files": [good.clone(), with(serde_json::json!({"readableScope": "filesystem"}))] })
            ),
            InputDecision::Dropped(InputDropReason::GrantScope)
        );
        // Unknown keys inside an entry.
        assert_eq!(
            push(
                &mut n,
                serde_json::json!({ "files": [with(serde_json::json!({"contents": "…"}))] })
            ),
            InputDecision::Dropped(InputDropReason::InvalidPayload {
                field: "contents".into()
            })
        );
        // Absolute coordinates inside an entry.
        assert_eq!(
            push(
                &mut n,
                serde_json::json!({ "files": [with(serde_json::json!({"screenX": 1}))] })
            ),
            InputDecision::Dropped(InputDropReason::AbsoluteCoordinates)
        );
        // Empty list, non-array, non-object entry, and more than FILE_DROP_MAX_FILES.
        let invalid_files = InputDecision::Dropped(InputDropReason::InvalidPayload {
            field: "files".into(),
        });
        assert_eq!(
            push(&mut n, serde_json::json!({ "files": [] })),
            invalid_files
        );
        assert_eq!(
            push(&mut n, serde_json::json!({ "files": "a.txt" })),
            invalid_files
        );
        assert_eq!(
            push(&mut n, serde_json::json!({ "files": ["a.txt"] })),
            invalid_files
        );
        let too_many: Vec<serde_json::Value> =
            (0..=FILE_DROP_MAX_FILES).map(|_| good.clone()).collect();
        assert_eq!(
            push(&mut n, serde_json::json!({ "files": too_many })),
            invalid_files
        );
        // Mixing the flat shape with the list shape is ambiguous → rejected.
        let mut mixed = good.clone();
        mixed["files"] = serde_json::json!([good.clone()]);
        assert_eq!(push(&mut n, mixed), invalid_files);
        // Exactly FILE_DROP_MAX_FILES is fine.
        let max: Vec<serde_json::Value> = (0..FILE_DROP_MAX_FILES).map(|_| good.clone()).collect();
        assert_eq!(
            push(&mut n, serde_json::json!({ "files": max })),
            InputDecision::Queued
        );
        assert_eq!(
            n.drain().remove(0).payload["files"]
                .as_array()
                .map(Vec::len),
            Some(FILE_DROP_MAX_FILES)
        );
        // The flat README §6 shape is unchanged: six keys, no `files`.
        assert_eq!(push(&mut n, good.clone()), InputDecision::Queued);
        let flat = n.drain().remove(0);
        assert_eq!(flat.payload.len(), 6);
        assert!(!flat.payload.contains_key("files"));
    }

    #[test]
    fn queue_cap_drops_oldest_non_safety() {
        let mut n = InputNormalizer::new(CharacterRole::PrimaryCompanion, InputLimits::default());
        assert_eq!(
            n.push(ev(InputEventKind::Dismissed, serde_json::json!({})), 0),
            InputDecision::Queued
        );
        for i in 0..64 {
            let mut e = ev(InputEventKind::Clicked, serde_json::json!({"x": i, "y": 0}));
            e.event_id = format!("c{i}");
            assert_eq!(n.push(e, 1000 + i), InputDecision::Queued);
        }
        assert_eq!(n.len(), 64);
        let drained = n.drain();
        assert_eq!(drained[0].kind, InputEventKind::Dismissed);
        assert_eq!(drained[1].event_id, "c1");
        // 全是安全事件時，新事件被拒絕而不是無界成長。
        for _ in 0..64 {
            assert_eq!(
                n.push(ev(InputEventKind::Dismissed, serde_json::json!({})), 0),
                InputDecision::Queued
            );
        }
        assert_eq!(
            n.push(ev(InputEventKind::Dismissed, serde_json::json!({})), 0),
            InputDecision::Dropped(InputDropReason::QueueFull)
        );
        assert_eq!(n.len(), 64);
    }

    #[test]
    fn observer_and_notification_only_never_forward() {
        for role in [CharacterRole::Observer, CharacterRole::NotificationOnly] {
            let mut n = InputNormalizer::new(role, InputLimits::default());
            assert_eq!(
                n.push(ev(InputEventKind::Clicked, serde_json::json!({})), 0),
                InputDecision::Dropped(InputDropReason::RoleFiltered)
            );
            assert_eq!(
                n.push(ev(InputEventKind::Dismissed, serde_json::json!({})), 0),
                InputDecision::Dropped(InputDropReason::RoleFiltered)
            );
            assert!(n.is_empty());
        }
    }

    #[test]
    fn action_requested_becomes_quick_action_request() {
        let mut n = InputNormalizer::new(CharacterRole::Familiar, InputLimits::default());
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::ActionRequested,
                    serde_json::json!({"action": "open settings"})
                ),
                0
            ),
            InputDecision::Dropped(InputDropReason::InvalidPayload {
                field: "action".into()
            })
        );
        assert_eq!(
            n.push(
                ev(
                    InputEventKind::ActionRequested,
                    serde_json::json!({"action": "open-settings"})
                ),
                0
            ),
            InputDecision::Queued
        );
        let e = n.drain().remove(0);
        let req = quick_action_request(&e).expect("quick action");
        assert_eq!(req.receptor, "companion.quick-action");
        assert_eq!(req.action, "open-settings");
    }
}
