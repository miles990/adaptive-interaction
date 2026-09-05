//! AIP §6：RFC 7396 merge patch、canonical state hash、接收端的 revision 規則。
//!
//! 這三個都是純函式：Rust host、TS 桌面與 Swift App 對同一個 state JSON 得到同一個 hash 與同一個決策。

use interaction_aip::{Envelope, MessageType};

use crate::receive::{decide_receive, IncomingKind, IncomingState, ReceiveDecision, ReceiverView};
use serde_json::{Map, Value};

/// RFC 7396 JSON Merge Patch：`null` 代表刪除鍵，物件遞迴合併，其他型別整體取代。
pub fn apply_patch(base: &Value, patch: &Value) -> Value {
    let Value::Object(patch_map) = patch else {
        return patch.clone();
    };
    let mut out: Map<String, Value> = match base {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    for (key, value) in patch_map {
        if value.is_null() {
            out.remove(key);
        } else {
            let current = out.get(key).cloned().unwrap_or(Value::Null);
            out.insert(key.clone(), apply_patch(&current, value));
        }
    }
    Value::Object(out)
}

/// `old` → `new` 的最小 RFC 7396 patch。對不含 `null` 值的文件保證
/// `apply_patch(old, merge_diff(old, new)) == new`（[`crate::state::SemanticState`] 永不寫 `null`）。
pub fn merge_diff(old: &Value, new: &Value) -> Value {
    let (Value::Object(old_map), Value::Object(new_map)) = (old, new) else {
        return new.clone();
    };
    let mut patch = Map::new();
    for (key, new_value) in new_map {
        match old_map.get(key) {
            Some(old_value) if old_value == new_value => {}
            Some(old_value) if old_value.is_object() && new_value.is_object() => {
                let nested = merge_diff(old_value, new_value);
                if !matches!(&nested, Value::Object(m) if m.is_empty()) {
                    patch.insert(key.clone(), nested);
                }
            }
            _ => {
                patch.insert(key.clone(), new_value.clone());
            }
        }
    }
    for key in old_map.keys() {
        if !new_map.contains_key(key) {
            patch.insert(key.clone(), Value::Null);
        }
    }
    Value::Object(patch)
}

/// Canonical JSON（鍵排序、無空白）的 SHA-256 hex。與 `interaction_aip::canonical_hash` 同一個實作。
pub fn state_hash(state: &Value) -> String {
    interaction_aip::canonical_hash(state)
}

/// 接收端收到 `state` 訊息時的決策（AIP §6 rollback 防護與 patch 續接規則）。
///
/// 這是**中繼資料形狀**的舊介面。完整的接收端決策表（連線世代、身分、hash 核對、
/// recovery、有界 realign）在 [`crate::receive`]；這裡的每一個變體都只是那張表的投影。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateDecision {
    /// 可以套用；套用後本地 revision 變成這個值。
    Apply { revision: u64 },
    /// host 明確重建了 session（`payload.reason:"session-reset"` 且 epoch 與本地不同）：丟棄本地狀態。
    Reset { revision: u64 },
    /// host 明說**同一個 session** 真的倒退過（`payload.reason:"recovery"`）：套用並退回
    /// host 的 revision，稽核 `aip.state-recovered`。
    Recover { revision: u64 },
    /// 接不上（`baseRevision` 不符、epoch 不同卻沒有重建宣告、hash 不符）：**不得**套用，
    /// 改送 `character.session.resume`。
    Resume,
    /// 忽略（稽核 `aip.state-rollback-ignored`）。
    Ignore { reason: IgnoreReason },
    /// 不是合法的 state 訊息（缺 revision／kind 未知／patch 缺 baseRevision／snapshot 缺 hash 或 state）。
    Invalid,
}

/// 忽略的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    /// revision 比本地舊。
    Rollback,
    /// revision 等於本地，已經套用過。
    AlreadyApplied,
}

/// **不追蹤 epoch** 的簡化版本：以訊息自己宣告的 `sessionEpoch` 當成本地 epoch。
///
/// 所以 §7 的 `session-reset`／`epoch-changed` 規則對它永遠不成立——它只回答
/// 「這個 revision 續不續得上」。真正的接收端要用 [`accept_state_with_epoch`]，
/// 或（連線世代、hash 核對、recovery 都要）直接用 [`crate::receive::decide_receive`]。
pub fn accept_state(local_revision: u64, envelope: &Envelope) -> StateDecision {
    decide(local_revision, None, envelope)
}

/// AIP §6／§7 的接收規則，逐條委派給 [`crate::receive::decide_receive`]。
///
/// 這個簽名帶不進兩樣東西，所以它們在這裡一定不成立，呼叫端要自己補：
///
/// 1. **連線世代**（規則 0）：舊連線遲到的 `session-reset` 只有呼叫端分得出來。
/// 2. **hash 核對**（規則 9／14）：這裡沒有 state，也就算不出 canonical hash；
///    `computed_hash` 是 `None` ＝「這個呼叫端沒有核對」，不是「核對過了」。
///
/// 身分（規則 1）同理：本地 session id 未知時不宣稱不符。
pub fn accept_state_with_epoch(
    local_revision: u64,
    local_epoch: u64,
    envelope: &Envelope,
) -> StateDecision {
    decide(local_revision, Some(local_epoch), envelope)
}

fn decide(local_revision: u64, local_epoch: Option<u64>, envelope: &Envelope) -> StateDecision {
    if envelope.message_type != MessageType::State {
        return StateDecision::Invalid;
    }
    let Some(revision) = envelope.payload.get("revision").and_then(Value::as_u64) else {
        return StateDecision::Invalid;
    };
    let Some(kind) = envelope
        .payload
        .get("kind")
        .and_then(Value::as_str)
        .and_then(IncomingKind::parse)
    else {
        return StateDecision::Invalid;
    };
    // `sessionEpoch` 缺席時沿用本地 epoch：typed boundary 保證真實訊息帶得有，
    // 這裡只是不讓「沒宣告」變成「宣告了 0」。
    let declared_epoch = envelope.payload.get("sessionEpoch").and_then(Value::as_u64);
    let epoch = declared_epoch.or(local_epoch).unwrap_or_default();
    let incoming = IncomingState {
        kind,
        session_id: envelope.session_id.clone(),
        epoch,
        revision,
        base_revision: envelope.base_revision,
        reason: envelope
            .payload
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        hash: envelope
            .payload
            .get("hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        computed_hash: None,
        state_present: envelope.payload.get("state").is_some(),
        arrived_on_generation: 0,
        via_authoritative_reply: false,
    };
    let view = ReceiverView {
        has_state: true,
        // 這個簽名沒有身分輸入：拿訊息自己的 sessionId 當本地值，規則 1 就不會誤判。
        session_id: incoming.session_id.clone(),
        epoch: local_epoch.unwrap_or(incoming.epoch),
        revision: local_revision,
        state_hash: None,
        connection_generation: 0,
    };
    match decide_receive(&view, &incoming) {
        ReceiveDecision::Apply => StateDecision::Apply { revision },
        ReceiveDecision::Reset => StateDecision::Reset { revision },
        ReceiveDecision::Recover => StateDecision::Recover { revision },
        ReceiveDecision::Realign { .. } => StateDecision::Resume,
        ReceiveDecision::IgnoreStale => StateDecision::Ignore {
            reason: IgnoreReason::Rollback,
        },
        ReceiveDecision::AlreadyApplied => StateDecision::Ignore {
            reason: IgnoreReason::AlreadyApplied,
        },
        // 規則 0／1 在這個簽名下不可能成立（世代相同、身分取自訊息本身）。
        ReceiveDecision::RejectInvalid
        | ReceiveDecision::IgnoreStaleConnection
        | ReceiveDecision::RejectIdentity => StateDecision::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_diff_is_minimal() {
        let old = json!({"mood": {"kind": "neutral", "intensity": 0.0}, "activity": "idle"});
        let new = json!({"mood": {"kind": "happy", "intensity": 0.0}, "activity": "idle"});
        assert_eq!(merge_diff(&old, &new), json!({"mood": {"kind": "happy"}}));
    }

    #[test]
    fn hash_changes_with_content() {
        let a = json!({"activity": "idle"});
        let b = json!({"activity": "reacting"});
        assert_ne!(state_hash(&a), state_hash(&b));
    }
}
