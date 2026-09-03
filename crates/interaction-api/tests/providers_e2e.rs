//! Provider 證據等級的 HTTP 投影（protocol-conformance-030）。
//!
//! 「已測試」證據裡的 `pairingUnverified`（裝置說它不需要配對 ⇒ spec 配的那組
//! 配對碼從未被任何一方比對過）必須一路走到 API JSON：CLI `providers show` 是
//! 直接印這份 JSON，桌面 UI 的六階階梯也是讀它。少了這一段，「從未驗證過的
//! 配對」在 CLI／UI 上會跟真配對完全無法區分。
//!
//! 這裡測的是 providers → API（含跨重啟的落地 JSON 相容性）。
//! executor → providers 那一段由
//! `crates/interaction-runtime/tests/providers_loop.rs` 的
//! `a_device_that_never_verified_the_pairing_code_downgrades_the_provider_evidence`
//! 用假傳輸的真 DeviceLink 覆蓋。

use interaction_core::{
    ProviderDescriptor, ProviderId, ProviderIdentity, ProviderKind, ProviderState, TrustLevel,
};
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};

const TOKEN: &str = "test-token-0123456789";

async fn start_runtime(home: &std::path::Path) -> Runtime {
    Runtime::start(RuntimeOptions {
        home: Some(home.to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap()
}

fn descriptor(id: &ProviderId, detail: Value) -> ProviderDescriptor {
    ProviderDescriptor {
        identity: ProviderIdentity {
            id: id.clone(),
            kind: ProviderKind::Device,
            display_name: "ESP32 桌面裝置".into(),
            trust_level: TrustLevel::Paired,
            origin: "declarative".into(),
            version: "1".into(),
            fingerprint: None,
            human: None,
        },
        // Installed：宣告式裝置的正常停靠狀態（重啟不會被降級改寫 detail）。
        state: ProviderState::Installed,
        receptors: vec![],
        actuators: vec!["esp32-desk.vibe".into()],
        tool_operations: vec![],
        paired_at: None,
        last_seen: Some(chrono::Utc::now()),
        detail: Some(detail.to_string()),
    }
}

/// 落地一筆「已測試」證據，重啟後由 API 讀回來（走 runtime 的
/// `split_provider_detail` → `with_tested` → HTTP JSON 這條真實路徑）。
async fn tested_from_api(home: &std::path::Path, id: &ProviderId, detail: Value) -> Value {
    {
        let rt = start_runtime(home).await;
        rt.store
            .save_provider(
                id.as_str(),
                &serde_json::to_string(&descriptor(id, detail)).unwrap(),
            )
            .unwrap();
        rt.shutdown().await;
    }
    let rt = start_runtime(home).await;
    let (addr, _handle) = interaction_api::serve(rt.clone(), "127.0.0.1", 0, TOKEN.into())
        .await
        .unwrap();
    let body: Value = reqwest::Client::new()
        .get(format!("http://{addr}/v1/providers/{}", id.as_str()))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let detail = body["detail"].as_str().expect("detail json string");
    serde_json::from_str::<Value>(detail).expect("detail parses")["tested"].clone()
}

#[tokio::test]
async fn provider_detail_tells_the_api_that_the_pairing_code_was_never_verified() {
    let dir = tempfile::tempdir().unwrap();
    let id = ProviderId::new("provider.adapter.esp32-desk");
    let tested = tested_from_api(
        dir.path(),
        &id,
        json!({
            "tested": {
                "at": "2026-09-03T02:00:00Z",
                "how": "handshake",
                "ok": true,
                "note": "裝置報上身分，但配對碼未經比對，身分證據僅為裝置自報的 deviceId：回應方式 esp32-desk.vibe 已回覆收到（acknowledged，不代表已完成）",
                "pairingUnverified": true,
            }
        }),
    )
    .await;

    assert_eq!(tested["ok"], Value::Bool(true));
    assert_eq!(tested["how"], Value::from("handshake"));
    assert_eq!(
        tested["pairingUnverified"],
        Value::Bool(true),
        "API 必須把「配對碼未經比對」帶出去，否則 CLI／UI 分不出真假配對：{tested}"
    );
}

#[tokio::test]
async fn a_verified_record_never_grows_a_pairing_warning_out_of_nowhere() {
    let dir = tempfile::tempdir().unwrap();
    let id = ProviderId::new("provider.adapter.esp32-desk");
    // 舊版寫下的 JSON 完全沒有這個鍵：不得因此被當成「未驗證」，也不得
    // 憑空長出旗標（向後相容：缺席＝false）。
    let tested = tested_from_api(
        dir.path(),
        &id,
        json!({
            "tested": {
                "at": "2026-09-03T02:00:00Z",
                "how": "handshake",
                "ok": true,
                "note": "裝置報上身分並完成配對：回應方式 esp32-desk.vibe 已回覆收到（acknowledged，不代表已完成）",
            }
        }),
    )
    .await;

    assert_eq!(tested["ok"], Value::Bool(true));
    assert!(
        tested.get("pairingUnverified").is_none(),
        "沒有旗標的舊記錄不得被改寫：{tested}"
    );
}
