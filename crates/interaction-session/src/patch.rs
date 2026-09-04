//! AIP §6：RFC 7396 merge patch、canonical state hash、接收端的 revision 規則。
//!
//! 這三個都是純函式：Rust host、TS 桌面與 Swift App 對同一個 state JSON 得到同一個 hash 與同一個決策。

use interaction_aip::{Envelope, MessageType};
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateDecision {
    /// 可以套用；套用後本地 revision 變成這個值。
    Apply { revision: u64 },
    /// host 明確重建了 session（`payload.reason:"session-reset"` 且 epoch 更大）：丟棄本地狀態。
    Reset { revision: u64 },
    /// `baseRevision` 與本地不符：**不得**套用，改送 `character.session.resume`。
    Resume,
    /// 忽略（稽核 `aip.state-rollback-ignored`）。
    Ignore { reason: IgnoreReason },
    /// 不是合法的 state 訊息（缺 revision／kind 未知／patch 缺 baseRevision）。
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

/// 不追蹤 epoch 的簡化版本（等同 `accept_state_with_epoch(local_revision, 0, envelope)`）。
pub fn accept_state(local_revision: u64, envelope: &Envelope) -> StateDecision {
    accept_state_with_epoch(local_revision, 0, envelope)
}

/// AIP §6 完整規則：rollback 防護、`session-reset` 例外、patch 續接。
pub fn accept_state_with_epoch(
    local_revision: u64,
    local_epoch: u64,
    envelope: &Envelope,
) -> StateDecision {
    if envelope.message_type != MessageType::State {
        return StateDecision::Invalid;
    }
    let Some(revision) = envelope.payload.get("revision").and_then(Value::as_u64) else {
        return StateDecision::Invalid;
    };
    let kind = envelope
        .payload
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "snapshot" => {
            let reset = envelope.payload.get("reason").and_then(Value::as_str)
                == Some(crate::REASON_SESSION_RESET);
            let epoch = envelope
                .payload
                .get("sessionEpoch")
                .and_then(Value::as_u64)
                .unwrap_or(local_epoch);
            if reset && epoch > local_epoch {
                return StateDecision::Reset { revision };
            }
            stale_or(revision, local_revision, StateDecision::Apply { revision })
        }
        "patch" => {
            let Some(base_revision) = envelope.base_revision else {
                return StateDecision::Invalid;
            };
            stale_or(
                revision,
                local_revision,
                if base_revision == local_revision {
                    StateDecision::Apply { revision }
                } else {
                    StateDecision::Resume
                },
            )
        }
        _ => StateDecision::Invalid,
    }
}

fn stale_or(revision: u64, local_revision: u64, fresh: StateDecision) -> StateDecision {
    match revision.cmp(&local_revision) {
        std::cmp::Ordering::Greater => fresh,
        std::cmp::Ordering::Equal => StateDecision::Ignore {
            reason: IgnoreReason::AlreadyApplied,
        },
        std::cmp::Ordering::Less => StateDecision::Ignore {
            reason: IgnoreReason::Rollback,
        },
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
