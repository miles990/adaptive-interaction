//! 測試共用 fixture：三種 manifest 範例（第三方 rig 風格／sprite／純文字）、時間與 envelope 工具。
//!
//! v0.6.0 起核心不再認識任何具名角色：這裡的 rig 風格 manifest 只是**第三方 manifest 範例**
//! （欄位齊全、能力最多的那種），核心並不知道它的 entrypoint id 代表什麼。
//! builtin 白名單一律由 host 注入（[`TEST_BUILTIN_WHITELIST`]），核心預設是空的。
#![allow(dead_code)]

use chrono::{TimeZone, Utc};
use interaction_character::*;
use std::path::PathBuf;

/// 確定性時間軸：`t(0)` 為固定基準，`t(n)` = 基準 + n 秒。
pub fn t(secs: i64) -> Timestamp {
    Utc.timestamp_opt(1_800_000_000 + secs, 0)
        .single()
        .unwrap_or_default()
}

pub fn packs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/interaction-desktop/public/packs")
}

pub fn read_pack(name: &str) -> serde_json::Value {
    let path = packs_dir().join(name).join("manifest.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// 測試 host 注入的 builtin 白名單（桌面 host 的實際白名單見 runtime／Tauri）。
pub const TEST_BUILTIN_WHITELIST: [&str; 4] = ["shu-rig", "sprite", "text", "shape"];

/// §2 範例風格的完整第三方 manifest。含一個 namespaced custom 能力與一個未知 canonical 前綴能力。
pub const SHU_MAID_JSON: &str = r#"{
  "schemaVersion": "1.0",
  "characterId": "shu-maid",
  "displayName": { "zh-TW": "小樞", "en": "Shu" },
  "author": "adaptive-interaction",
  "description": { "zh-TW": "執行期參數化分層 rig。" },
  "version": "3.0.0",
  "adapterKind": "in-process",
  "entrypoint": { "kind": "builtin", "id": "shu-rig" },
  "assets": [ { "id": "preview", "path": "preview.png", "mediaType": "image/png", "bytes": 12345,
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000" } ],
  "capabilities": {
    "visual.presence":   { "supported": true, "reducedMotionBehavior": "static" },
    "visual.pose":       { "supported": true, "reducedMotionBehavior": "reduced", "interruptible": true },
    "visual.expression": { "supported": true, "variants": ["idle", "notice", "thinking", "curious"],
                           "reducedMotionBehavior": "reduced", "durationRange": { "minMs": 200, "maxMs": 60000 } },
    "visual.gaze":       { "supported": true },
    "visual.locomotion": { "supported": true, "reducedMotionBehavior": "static" },
    "visual.overlay":    { "supported": true },
    "visual.particles":  { "supported": true, "reducedMotionBehavior": "disabled" },
    "visual.prop":       { "supported": true },
    "visual.textBubble": { "supported": true, "reducedMotionBehavior": "unchanged" },
    "audio.speech":      { "supported": true, "requiresAudio": true },
    "audio.effect":      { "supported": true, "requiresAudio": true },
    "multiCharacter":    { "supported": true },
    "scene":             { "supported": true },
    "rollCall":          { "supported": true },
    "gameplay.toys":     { "supported": true, "maxConcurrent": 4 },
    "gameplay.autonomy": { "supported": true },
    "com.example.character.wings": { "supported": true },
    "visual.wings":      { "supported": true }
  },
  "inputCapabilities": {
    "input.click": { "supported": true }, "input.hover": { "supported": true },
    "input.drag": { "supported": true }, "input.drop": { "supported": true },
    "input.pointerProximity": { "supported": true }, "input.text": { "supported": true },
    "input.fileDrop": { "supported": true }
  },
  "channels": ["transform", "locomotion", "pose", "expression", "gaze", "speech", "bubble", "audio",
               "prop", "overlay", "particle", "scene", "com.example.character.wings"],
  "states": ["idle", "working"],
  "intents": ["idle", "notice", "acknowledge", "think", "work", "wait", "ask", "request-consent",
              "blocked", "unknown", "claim-completed", "verified-success", "failed", "cancelled",
              "offline", "emergency", "greet", "play", "rest", "sleep"],
  "variants": [ { "id": "maid-classic", "displayName": { "zh-TW": "經典" } },
                { "id": "maid-dusk", "displayName": { "zh-TW": "暮色" } },
                { "id": "maid-sakura", "displayName": { "zh-TW": "櫻" } } ],
  "locales": ["zh-TW", "en"],
  "pronouns": { "zh-TW": "她", "en": "she" },
  "preferencesSchema": { "type": "object", "properties": {
      "palette": { "type": "string", "enum": ["maid-classic", "maid-dusk", "maid-sakura"] },
      "volume": { "type": "number", "minimum": 0, "maximum": 1 },
      "autonomy": { "type": "boolean", "default": false },
      "toys": { "type": "integer", "minimum": 0, "maximum": 8 } } },
  "securityRequirements": { "network": false, "executable": false, "fileAccess": "none",
                            "audioOutput": true, "microphone": false, "camera": false },
  "resourceLimits": { "maxAssetBytes": 8388608, "maxConcurrentCommands": 4, "maxQueue": 32, "maxFps": 60 },
  "fallbacks": { "capabilities": { "visual.expression": ["visual.pose", "visual.textBubble"],
                                   "visual.particles": ["visual.expression"] },
                 "intents": { "play": "notice", "sleep": "rest" } },
  "compatibility": { "protocol": "1.x", "runtime": ">=0.5.0" },
  "futureField": { "anything": true },
  "x-vendor": { "note": "vendor extension is preserved and not reported" }
}"#;

/// 最小文字角色（§12 `text`）：證明協定不依賴 rig。
pub const TEXT_JSON: &str = r#"{
  "schemaVersion": "1.0",
  "characterId": "text",
  "displayName": { "zh-TW": "文字角色", "en": "Text" },
  "version": "1.0.0",
  "adapterKind": "in-process",
  "entrypoint": { "kind": "builtin", "id": "text" },
  "capabilities": {
    "visual.presence":   { "supported": true, "reducedMotionBehavior": "unchanged" },
    "visual.textBubble": { "supported": true, "reducedMotionBehavior": "unchanged" },
    "audio.effect":      { "supported": true, "requiresAudio": true }
  },
  "inputCapabilities": { "input.click": { "supported": true }, "input.text": { "supported": true } },
  "channels": ["transform", "bubble", "audio"],
  "intents": ["idle", "notice", "acknowledge", "think", "work", "wait", "ask", "request-consent",
              "blocked", "unknown", "claim-completed", "verified-success", "failed", "cancelled",
              "offline", "emergency", "greet", "play", "rest", "sleep"],
  "locales": ["zh-TW", "en"],
  "fallbacks": { "capabilities": { "visual.expression": ["visual.textBubble"],
                                   "visual.pose": ["visual.textBubble"],
                                   "gameplay.toys": ["visual.textBubble"] } },
  "compatibility": { "protocol": "1.x" }
}"#;

pub fn parse_json(json: &str) -> CharacterManifest {
    serde_json::from_str(json).expect("fixture manifest parses")
}

/// 能力最完整的第三方 manifest 範例。
pub fn reference_manifest() -> CharacterManifest {
    parse_json(SHU_MAID_JSON)
}

/// [`reference_manifest`] 的既有名稱（測試沿用）。
pub fn shu_manifest() -> CharacterManifest {
    reference_manifest()
}

pub fn text_manifest() -> CharacterManifest {
    parse_json(TEXT_JSON)
}

/// 核心 registry（只有通用 sprite migrator）。
pub fn core_registry() -> MigrationRegistry {
    MigrationRegistry::with_core_migrators()
}

/// 由真實舊 pack（`shu-standard`，character-pack 1.0）遷移出的 sprite manifest。
pub fn sprite_manifest() -> CharacterManifest {
    migrate_pack_to_manifest(&read_pack("shu-standard"), &core_registry())
        .expect("shu-standard migrates")
}

/// host 注入白名單後的驗證（核心預設白名單是空的）。
pub fn validate(manifest: &CharacterManifest) -> Result<ManifestReport, ManifestError> {
    validate_with_whitelist(manifest, &TEST_BUILTIN_WHITELIST)
}

pub fn test_limits(whitelist: &[&str]) -> ValidationLimits {
    ValidationLimits {
        builtin_whitelist: whitelist.iter().map(|s| s.to_string()).collect(),
        ..ValidationLimits::default()
    }
}

pub fn validate_with_whitelist(
    manifest: &CharacterManifest,
    whitelist: &[&str],
) -> Result<ManifestReport, ManifestError> {
    let bytes = serde_json::to_vec(manifest).unwrap_or_default().len();
    validate_manifest(bytes, manifest, &test_limits(whitelist))
}

pub fn hello(instance: &str, reduced_motion: bool) -> Hello {
    Hello {
        protocol_version: PROTOCOL_VERSION.into(),
        runtime_version: "0.5.0".into(),
        character_instance_id: instance.into(),
        role: CharacterRole::PrimaryCompanion,
        locale: "zh-TW".into(),
        reduced_motion,
        requires: CharacterIntent::ALL.to_vec(),
        limits: HelloLimits::default(),
    }
}

/// Runtime 建構的 envelope（priority 由 floor 夾住；TTL 60 s）。
pub fn envelope(
    instance: &InstanceId,
    message_id: &str,
    intent: CharacterIntent,
    truth: TruthState,
    priority: u8,
    now: Timestamp,
) -> IntentEnvelope {
    IntentEnvelope::from_runtime(
        message_id,
        instance.as_str(),
        Some("corr-1".into()),
        intent,
        truth,
        priority,
        now,
        now + chrono::Duration::seconds(60),
    )
}

/// 註冊＋協商（照 manifest 全數提供）。
pub fn connect(gw: &mut Gateway, manifest: &CharacterManifest, role: CharacterRole) -> InstanceId {
    let id = gw.register_instance(manifest.clone(), role);
    let offer = Negotiate::from_manifest(manifest, 1);
    gw.on_negotiate(&id, offer, t(0))
        .expect("negotiation succeeds");
    id
}

pub fn receipts(out: &[GatewayOutput]) -> Vec<&CommandReceipt> {
    out.iter()
        .filter_map(|o| match o {
            GatewayOutput::Receipt(r) => Some(r),
            _ => None,
        })
        .collect()
}

pub fn sends(out: &[GatewayOutput]) -> Vec<&WireMessage> {
    out.iter()
        .filter_map(|o| match o {
            GatewayOutput::Send { message, .. } => Some(message),
            _ => None,
        })
        .collect()
}

pub fn audits(out: &[GatewayOutput]) -> Vec<&str> {
    out.iter()
        .filter_map(|o| match o {
            GatewayOutput::Audit(a) => Some(a.as_str()),
            _ => None,
        })
        .collect()
}

pub fn system_texts(out: &[GatewayOutput]) -> usize {
    out.iter()
        .filter(|o| matches!(o, GatewayOutput::SystemText { .. }))
        .count()
}

pub fn sent_intents(out: &[GatewayOutput]) -> Vec<String> {
    sends(out)
        .iter()
        .filter_map(|m| match m {
            WireMessage::Intent { envelope } => Some(envelope.message_id.clone()),
            _ => None,
        })
        .collect()
}

pub fn sent_cancels(out: &[GatewayOutput]) -> Vec<(String, Option<String>)> {
    sends(out)
        .iter()
        .filter_map(|m| match m {
            WireMessage::Cancel { message_id, reason } => {
                Some((message_id.clone(), reason.clone()))
            }
            _ => None,
        })
        .collect()
}

/// adapter 回執（帶目前 generation）。
pub fn adapter_receipt(
    gw: &mut Gateway,
    id: &InstanceId,
    message_id: &str,
    status: ReceiptStatus,
    now: Timestamp,
) -> Vec<GatewayOutput> {
    let generation = gw.generation(id).unwrap_or(0);
    gw.on_receipt(
        id,
        CommandReceipt::new(message_id, id.as_str(), generation, status, now),
        now,
    )
}
