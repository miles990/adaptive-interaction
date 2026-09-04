//! 小樞內建角色的 CPP conformance（核心 `tests/conformance.rs` 把 `builtin:shu-rig` 列為 deferred，
//! 由這裡覆蓋）。
//!
//! 同一套驗收：manifest 通過驗證（host 注入白名單）→ 20 個 intent 全部有解析結果 →
//! 安全 intent 永不 `unsupported` → 沒有任何 intent 經由 `verified-success` 呈現 →
//! `emergency` 的 priority floor 仍是 100。
//!
//! manifest 以 `include_str!` 在編譯期讀入（與 Tauri 讀 `index.json` 的做法一致），
//! 檔案改了就要重編、不會靜默漂移。

use chrono::{TimeZone, Utc};
use interaction_character::*;
use interaction_character_shu::ShuRigPack;

const BUNDLED: [(&str, &str); 3] = [
    (
        "shu-maid",
        include_str!("../../../apps/interaction-desktop/public/characters/shu-maid/manifest.json"),
    ),
    (
        "shu-maid-dusk",
        include_str!(
            "../../../apps/interaction-desktop/public/characters/shu-maid-dusk/manifest.json"
        ),
    ),
    (
        "shu-maid-sakura",
        include_str!(
            "../../../apps/interaction-desktop/public/characters/shu-maid-sakura/manifest.json"
        ),
    ),
];

fn t(secs: i64) -> Timestamp {
    Utc.timestamp_opt(1_800_000_000 + secs, 0)
        .single()
        .unwrap_or_default()
}

fn host_limits() -> ValidationLimits {
    ValidationLimits {
        builtin_whitelist: vec![ShuRigPack::ENTRYPOINT_ID.to_string()],
        ..ValidationLimits::default()
    }
}

#[test]
fn bundled_rig_characters_conform() {
    for (id, text) in BUNDLED {
        let manifest: CharacterManifest = serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("{id}: 不是合法的 CPP manifest：{e}"));
        assert_eq!(manifest.character_id, id);
        assert_eq!(
            manifest.entrypoint,
            Entrypoint::Builtin {
                id: ShuRigPack::ENTRYPOINT_ID.into()
            },
            "{id}: 內建 rig 角色的 entrypoint"
        );

        // 1. host 注入白名單後通過驗證；沒注入就不通過（核心不預設任何 builtin id）。
        let bytes = text.len();
        validate_manifest(bytes, &manifest, &host_limits())
            .unwrap_or_else(|e| panic!("{id}: manifest 驗證失敗：{e}"));
        assert!(
            validate_manifest(bytes, &manifest, &ValidationLimits::default()).is_err(),
            "{id}: 沒有 host 注入時不得通過"
        );

        // 2. 20 個 intent 全部有解析結果。
        let mut gw = Gateway::default();
        let instance = gw.register_instance(manifest.clone(), CharacterRole::PrimaryCompanion);
        let (negotiated, _) = gw
            .on_negotiate(&instance, Negotiate::from_manifest(&manifest, 1), t(0))
            .unwrap_or_else(|e| panic!("{id}: 協商失敗：{e}"));
        assert_eq!(negotiated.resolutions.len(), 20, "{id}: 20 個 intent");

        for intent in CharacterIntent::ALL {
            let r = negotiated
                .resolutions
                .get(&intent)
                .unwrap_or_else(|| panic!("{id}: {intent} 沒有解析結果"));
            // 3. 安全 intent 永不遺失，也不得經由非安全 intent 呈現。
            if intent.is_safety() {
                assert_ne!(
                    r.resolution,
                    Resolution::Unsupported,
                    "{id}: 安全 intent {intent} 不得 unsupported"
                );
                if let Some(via_intent) = r.via_intent {
                    assert!(
                        via_intent.is_safety(),
                        "{id}: 安全 intent {intent} 不得經由 {via_intent} 呈現"
                    );
                }
            }
            // 4. claimed ≠ verified：沒有任何 intent 經由 verified-success 呈現。
            assert_ne!(
                r.via_intent,
                Some(CharacterIntent::VerifiedSuccess),
                "{id}: {intent} 不得經由 verified-success 呈現"
            );
        }

        // 5. emergency floor 仍是 100，且必定送達（adapter 或 system.text）。
        let envelope = IntentEnvelope::from_runtime(
            "shu-conformance-emergency",
            instance.as_str(),
            Some("shu-conformance".into()),
            CharacterIntent::Emergency,
            TruthState::Emergency,
            0,
            t(1),
            t(61),
        );
        assert_eq!(envelope.priority, 100, "{id}: emergency floor");
        let out = gw.dispatch(&instance, envelope, t(1));
        let delivered = out.iter().any(|o| match o {
            GatewayOutput::Send {
                message: WireMessage::Intent { envelope },
                ..
            } => envelope.intent == CharacterIntent::Emergency,
            GatewayOutput::SystemText { intent, .. } => *intent == CharacterIntent::Emergency,
            _ => false,
        });
        assert!(delivered, "{id}: emergency 不得遺失");
    }
}
