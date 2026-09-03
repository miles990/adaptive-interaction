//! 內嵌模式的 Character API 轉接：Tauri IPC → `interaction_runtime::Runtime`。
//!
//! Runtime 端的 CPP 方法由 `crates/interaction-runtime/src/character.rs`（Phase 2-D）
//! 提供；這裡只把與 `/v1/character/*` 完全相同的 JSON body 轉成那些方法的型別，
//! 不含任何協定邏輯（協定在 `interaction-character`）。External 模式不經此處：
//! `lib.rs` 的 `Backend` 直接打 HTTP 路由，兩條路徑的輸入輸出形狀一致。
//!
//! 誠實原則：解析失敗或 Runtime 回錯一律 `Err`（訊息 ≤ 200 字、不回顯 manifest），
//! 不假裝成功、不回假的 negotiated／instances。

use interaction_character::{
    CharacterInputEvent, CharacterManifest, CharacterRole, CommandReceipt, Negotiate,
};
use interaction_runtime::character::CharacterHelloInput;
use interaction_runtime::Runtime;
use serde_json::Value;

/// 錯誤訊息上限（CPP README §2.1：不得回顯超過 200 字）。
const MAX_ERROR_CHARS: usize = 200;

fn short_error(prefix: &str, error: impl std::fmt::Display) -> String {
    let text = format!("{prefix}: {error}");
    if text.chars().count() <= MAX_ERROR_CHARS {
        text
    } else {
        let mut out: String = text.chars().take(MAX_ERROR_CHARS).collect();
        out.push('…');
        out
    }
}

fn field<T: serde::de::DeserializeOwned>(body: &Value, key: &str) -> Result<T, String> {
    let value = body
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing field `{key}`"))?;
    serde_json::from_value(value).map_err(|e| short_error(key, e))
}

fn optional<T: serde::de::DeserializeOwned>(body: &Value, key: &str) -> Result<Option<T>, String> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| short_error(key, e)),
    }
}

/// `POST /v1/character/hello` 的內嵌對應。body 與 HTTP 路由完全相同：
/// `{instanceId?, role?, manifest, negotiate, visible, packId?, behaviorState?, reducedMotion?}`
/// → `{instanceId, generation, negotiated}`。
pub async fn hello(rt: &Runtime, body: Value) -> Result<Value, String> {
    let manifest: CharacterManifest = field(&body, "manifest")?;
    let negotiate: Negotiate = field(&body, "negotiate")?;
    let role: Option<CharacterRole> = optional(&body, "role")?;
    let input = CharacterHelloInput {
        instance_id: optional(&body, "instanceId")?,
        role,
        manifest,
        negotiate,
        visible: body
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        pack_id: optional(&body, "packId")?,
        behavior_state: body.get("behaviorState").cloned().filter(|v| !v.is_null()),
        reduced_motion: body
            .get("reducedMotion")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    rt.character_hello(input)
        .await
        .map_err(|e| short_error("character hello", e))
}

/// `POST /v1/character/receipts`：`{instanceId, receipt}` → `{accepted, status?}`。
pub async fn receipt(rt: &Runtime, instance_id: &str, receipt: Value) -> Result<Value, String> {
    let receipt: CommandReceipt =
        serde_json::from_value(receipt).map_err(|e| short_error("receipt", e))?;
    rt.character_receipt(instance_id, receipt)
        .await
        .map_err(|e| short_error("character receipt", e))
}

/// `POST /v1/character/events`：`{instanceId, event}` → `{decision, reason?}`。
pub async fn event(rt: &Runtime, instance_id: &str, event: Value) -> Result<Value, String> {
    let event: CharacterInputEvent =
        serde_json::from_value(event).map_err(|e| short_error("event", e))?;
    rt.character_event(instance_id, event)
        .await
        .map_err(|e| short_error("character event", e))
}

/// `GET /v1/character/instances` → `{instances: [...]}`。
pub async fn instances(rt: &Runtime) -> Result<Value, String> {
    Ok(rt.character_instances())
}

/// `GET /v1/character/manifest` → 目前桌面角色 manifest；尚未 hello 時 `Err`（同 HTTP 404）。
pub async fn manifest(rt: &Runtime) -> Result<Value, String> {
    let manifest = rt
        .character_manifest()
        .ok_or_else(|| "no active desktop character (hello not received yet)".to_string())?;
    serde_json::to_value(manifest).map_err(|e| short_error("manifest", e))
}

/// `GET /v1/character/adapters` → `{adapters: [...]}`（不含 token）。
pub async fn adapters(rt: &Runtime) -> Result<Value, String> {
    Ok(rt.character_adapters())
}

/// `DELETE /v1/character/adapters/{id}` → `{adapterId, revoked, disconnected}`。
pub async fn adapter_revoke(rt: &Runtime, adapter_id: &str) -> Result<Value, String> {
    rt.character_adapter_revoke(adapter_id)
        .await
        .map_err(|e| short_error("character adapter revoke", e))
}
