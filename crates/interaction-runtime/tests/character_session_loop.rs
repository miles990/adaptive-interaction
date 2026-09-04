//! AIP Character Session 的 Runtime 閉環（**模擬 iPhone（fixture）**：程序內以真 wss
//! transport 驅動的假手機，不是真機驗收）。
//!
//! 覆蓋：capability 協商→snapshot→touch→state patch→Behavior Intent（手機端 command＋
//! 桌面端 CPP play）→ `iphone.touch` observation 只落一次；人工驗證的慶祝只送手機、桌面
//! 不雙播；緊急停止凍結互動；斷線 presence→重連 resume；日誌溢位→snapshot；epoch 不同→
//! session-reset；過期／重複／偽造身分／超大／未知型別／未知 name／越權／跨 session／
//! 速率上限；以及 `INTERACT_AI_CHARACTER_SESSION=0` 的回退路徑。

use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use interaction_character::{CharacterManifest, Negotiate};
use interaction_core::{EventType, ObservationQuery, ReceptorId, RuntimeEvent};
use interaction_runtime::character::{CharacterHelloInput, DESKTOP_INSTANCE_ID};
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

type HmacSha256 = Hmac<Sha256>;
type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// `INTERACT_AI_CHARACTER_SESSION` 是行程層設定：整個檔案序列化，避免平行測試互相干擾。
/// 用 async-aware 的鎖，因為每個測試都要持有它跨越 `.await`。
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().await
}

async fn runtime() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().expect("tempdir");
    let rt = runtime_at(dir.path()).await;
    (dir, rt)
}

async fn runtime_at(home: &std::path::Path) -> Runtime {
    Runtime::start(RuntimeOptions {
        home: Some(home.to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .expect("runtime starts")
}

// ---------------------------------------------------------------------------
// 模擬 iPhone（fixture）：TLS 指紋釘選＋HMAC 配對（與 mobile_loop.rs 同一套手法）
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct PinVerifier {
    fingerprint: String,
    provider: rustls::crypto::CryptoProvider,
}

impl rustls::client::danger::ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fp = format!("{:x}", Sha256::digest(end_entity.as_ref()));
        if fp == self.fingerprint {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("fingerprint mismatch".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

async fn connect(port: u16, fingerprint: &str) -> Ws {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinVerifier {
            fingerprint: fingerprint.to_string(),
            provider: rustls::crypto::ring::default_provider(),
        }))
        .with_no_client_auth();
    tokio_tungstenite::connect_async_tls_with_config(
        format!("wss://127.0.0.1:{port}/"),
        None,
        false,
        Some(tokio_tungstenite::Connector::Rustls(Arc::new(config))),
    )
    .await
    .map(|(ws, _)| ws)
    .expect("wss connect")
}

async fn send_json(ws: &mut Ws, value: Value) {
    ws.send(Message::Text(value.to_string()))
        .await
        .expect("send");
}

async fn recv_json_within(ws: &mut Ws, budget: Duration) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        match tokio::time::timeout(left, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                return Some(serde_json::from_str(&text).unwrap_or(Value::Null))
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(_))) | Ok(None) => return None,
            Err(_) => return None,
        }
    }
}

/// 收下一則 `aip` frame 的 envelope（其他 frame 一律跳過）。
async fn recv_aip(ws: &mut Ws) -> Value {
    recv_aip_within(ws, Duration::from_secs(5))
        .await
        .expect("aip frame in time")
}

async fn recv_aip_within(ws: &mut Ws, budget: Duration) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        let value = recv_json_within(ws, left).await?;
        if value["type"] == json!("aip") {
            return Some(value["envelope"].clone());
        }
    }
}

/// 收 `count` 則 aip envelope（順序不保證，呼叫端自己找）。
async fn collect_aip(ws: &mut Ws, count: usize, budget: Duration) -> Vec<Value> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + budget;
    while out.len() < count {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match recv_aip_within(ws, left).await {
            Some(envelope) => out.push(envelope),
            None => break,
        }
    }
    out
}

fn find<'a>(frames: &'a [Value], message_type: &str) -> Option<&'a Value> {
    frames
        .iter()
        .find(|e| e["messageType"] == json!(message_type))
}

fn hmac_hex(code: &str, nonce: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(code.as_bytes()).expect("hmac key");
    mac.update(nonce.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 完整配對一台模擬 iPhone（fixture）→ (deviceId, deviceToken, ws)。
async fn pair(rt: &Runtime) -> (String, String, Ws) {
    let session = rt.mobile_pairing_begin().await.expect("pairing session");
    let port = session["port"].as_u64().expect("port") as u16;
    let fp = session["fingerprint"]
        .as_str()
        .expect("fingerprint")
        .to_string();
    let code = session["code"].as_str().expect("code").to_string();
    let mut ws = connect(port, &fp).await;
    send_json(
        &mut ws,
        json!({"type":"pair-request","deviceName":"模擬 iPhone（fixture）","model":"iPhone12,1"}),
    )
    .await;
    let challenge = recv_json_within(&mut ws, Duration::from_secs(5))
        .await
        .expect("pair-challenge");
    assert_eq!(challenge["type"], "pair-challenge", "{challenge}");
    let nonce = challenge["nonce"].as_str().expect("nonce").to_string();
    send_json(
        &mut ws,
        json!({"type":"pair-response","hmac": hmac_hex(&code, &nonce)}),
    )
    .await;
    let paired = recv_json_within(&mut ws, Duration::from_secs(5))
        .await
        .expect("paired");
    assert_eq!(paired["type"], "paired", "{paired}");
    (
        paired["deviceId"].as_str().expect("deviceId").to_string(),
        paired["deviceToken"].as_str().expect("token").to_string(),
        ws,
    )
}

/// 用配對 token 重新連線（撤銷後會拿到 auth-fail）。
async fn reconnect(rt: &Runtime, device_id: &str, token: &str) -> Result<Ws, String> {
    let status = rt.mobile_status().await.expect("mobile status");
    let port = status["port"].as_u64().expect("port") as u16;
    let fp = status["fingerprint"]
        .as_str()
        .expect("fingerprint")
        .to_string();
    let mut ws = connect(port, &fp).await;
    send_json(
        &mut ws,
        json!({"type":"auth","deviceId": device_id, "token": token}),
    )
    .await;
    match recv_json_within(&mut ws, Duration::from_secs(5)).await {
        Some(reply) if reply["type"] == json!("auth-ok") => Ok(ws),
        Some(reply) => Err(reply["type"].as_str().unwrap_or("unknown").to_string()),
        None => Err("closed".into()),
    }
}

// ---------------------------------------------------------------------------
// AIP frame 組裝（模擬 iPhone（fixture）端）
// ---------------------------------------------------------------------------

fn aip(envelope: Value) -> Value {
    json!({"type":"aip","envelope": envelope})
}

fn base_envelope(message_type: &str, name: &str, device_id: &str, message_id: &str) -> Value {
    json!({
        "specVersion": "aip/1.0",
        "messageId": message_id,
        "messageType": message_type,
        "name": name,
        "source": {"kind":"device","id": device_id},
        "sessionId": "session.home",
        "occurredAt": Utc::now().to_rfc3339(),
        "payload": {},
    })
}

fn capability_envelope(device_id: &str) -> Value {
    let mut envelope = base_envelope(
        "capability",
        "character.session.capability",
        device_id,
        &format!("fx-cap-{}", Utc::now().timestamp_millis()),
    );
    envelope["payload"] = json!({
        "specVersions": ["aip/1.0"],
        "role": "remote-renderer",
        "profiles": ["character-session"],
        "syncClasses": ["semantic"],
        "intents": ["react-happily-to-touch", "celebrate", "idle"],
        "inputs": ["character.interaction.touch"],
        "features": {"haptic": true, "reducedMotion": false},
        "limits": {"maxMessageBytes": 65536}
    });
    envelope
}

fn touch_envelope(device_id: &str, message_id: &str, kind: &str) -> Value {
    let mut envelope = base_envelope(
        "event",
        "character.interaction.touch",
        device_id,
        message_id,
    );
    envelope["expiresAt"] = json!((Utc::now() + ChronoDuration::milliseconds(5_000)).to_rfc3339());
    envelope["payload"] = json!({"kind": kind, "intensity": 0.6});
    envelope
}

fn resume_query(
    device_id: &str,
    message_id: &str,
    revision: u64,
    sequence: u64,
    epoch: u64,
) -> Value {
    let mut envelope = base_envelope("query", "character.session.resume", device_id, message_id);
    envelope["target"] = json!({"kind":"session","id":"session.home"});
    envelope["payload"] = json!({
        "lastRevision": revision,
        "lastSequence": sequence,
        "sessionEpoch": epoch,
    });
    envelope
}

// ---------------------------------------------------------------------------
// 桌面視窗（可信 host surface）
// ---------------------------------------------------------------------------

fn desktop_manifest() -> CharacterManifest {
    serde_json::from_value(json!({
        "schemaVersion": "1.0",
        "characterId": "ref-shape",
        "displayName": { "zh-TW": "參考形狀", "en": "Ref Shape" },
        "author": "adaptive-interaction",
        "version": "1.0.0",
        "adapterKind": "in-process",
        "entrypoint": { "kind": "builtin", "id": "shape" },
        "assets": [],
        "capabilities": {
            "visual.presence": { "supported": true },
            "visual.expression": {
                "supported": true,
                "variants": ["idle", "notice", "play", "rest", "work", "think",
                             "acknowledge", "wait", "greet", "sleep", "success", "emergency"]
            },
            "visual.textBubble": { "supported": true }
        },
        "inputCapabilities": { "input.click": { "supported": true } },
        "channels": ["expression", "bubble"],
        "states": ["idle"],
        "intents": [
            "idle", "notice", "acknowledge", "think", "work", "wait", "ask", "request-consent",
            "blocked", "unknown", "claim-completed", "verified-success", "failed", "cancelled",
            "offline", "emergency", "greet", "play", "rest", "sleep"
        ],
        "variants": [],
        "locales": ["zh-TW", "en"],
        "securityRequirements": {
            "network": false, "executable": false, "fileAccess": "none",
            "audioOutput": false, "microphone": false, "camera": false
        },
        "compatibility": { "protocol": "1.x", "runtime": ">=0.5.0" }
    }))
    .expect("desktop manifest parses")
}

async fn hello(rt: &Runtime) -> Value {
    let manifest = desktop_manifest();
    rt.character_hello(CharacterHelloInput {
        instance_id: None,
        role: None,
        negotiate: Negotiate::from_manifest(&manifest, 1),
        manifest,
        visible: true,
        pack_id: None,
        behavior_state: None,
        reduced_motion: false,
    })
    .await
    .expect("hello accepted")
}

fn character_intents(rt: &Runtime) -> Vec<RuntimeEvent> {
    rt.events
        .recent(500)
        .into_iter()
        .filter(|e| e.event_type == EventType::CharacterIntent)
        .collect()
}

/// 等某個條件成立（有界輪詢；spawn 出去的投影不必用 sleep 猜時間）。
async fn wait_for(mut predicate: impl FnMut() -> bool, budget: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if predicate() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ---------------------------------------------------------------------------
// 1. capability → snapshot → touch → patch＋intent＋observation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fixture_iphone_touch_updates_state_and_reaches_both_renderers() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;

    // capability：host 回 negotiated＋snapshot 兩則 aip frame。
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let frames = collect_aip(&mut ws, 3, Duration::from_secs(5)).await;
    let negotiated = find(&frames, "capability").expect("negotiated capability");
    assert_eq!(negotiated["payload"]["specVersion"], "aip/1.0");
    assert_eq!(negotiated["payload"]["role"], "remote-renderer");
    assert_eq!(
        negotiated["payload"]["intents"]["react-happily-to-touch"],
        "exact"
    );
    assert_eq!(negotiated["payload"]["intents"]["settle"], "unsupported");
    let snapshot = find(&frames, "state").expect("snapshot state");
    assert_eq!(snapshot["payload"]["kind"], "snapshot");
    let revision0 = snapshot["payload"]["revision"].as_u64().expect("revision");

    // touch：一則 result{applied}＋一則 state patch＋一則 command（Behavior Intent）。
    send_json(
        &mut ws,
        aip(touch_envelope(&device_id, "fx-touch-1", "tap")),
    )
    .await;
    let frames = collect_aip(&mut ws, 3, Duration::from_secs(5)).await;
    let result = find(&frames, "result").expect("result envelope");
    assert_eq!(result["payload"]["status"], "applied", "{result}");
    assert_eq!(result["causationId"], "fx-touch-1");
    let patch = find(&frames, "state").expect("state patch");
    assert_eq!(patch["payload"]["kind"], "patch");
    assert_eq!(patch["baseRevision"].as_u64(), Some(revision0));
    assert_eq!(patch["payload"]["revision"].as_u64(), Some(revision0 + 1));
    assert_eq!(patch["payload"]["patch"]["activity"], "reacting");
    let command = find(&frames, "command").expect("behavior command");
    assert_eq!(command["name"], "character.behavior.request");
    assert_eq!(command["payload"]["intent"], "react-happily-to-touch");
    assert!(command["expiresAt"].is_string(), "intent 必須帶 deadline");

    // 桌面 renderer：CPP play（truthState none、priority 40、variant 同名）。
    assert!(
        wait_for(
            || character_intents(&rt)
                .iter()
                .any(|e| e.payload["envelope"]["intent"] == json!("play")),
            Duration::from_secs(3),
        )
        .await,
        "桌面角色必須收到 CPP play：{:?}",
        character_intents(&rt)
    );
    let play = character_intents(&rt)
        .into_iter()
        .find(|e| e.payload["envelope"]["intent"] == json!("play"))
        .expect("play intent");
    assert_eq!(play.payload["envelope"]["truthState"], "none");
    assert_eq!(play.payload["envelope"]["priority"], 40);
    assert_eq!(
        play.payload["envelope"]["presentationHints"]["variant"],
        "react-happily-to-touch"
    );
    assert_eq!(
        play.payload["targets"],
        json!([DESKTOP_INSTANCE_ID]),
        "session 投影只送給已連線的桌面 instance"
    );

    // recipe 相容：同一個 touch 只落成一筆 `iphone.touch` observation。
    assert!(
        wait_for(
            || rt
                .store
                .query_observations(&ObservationQuery {
                    receptor_id: Some(ReceptorId::new("iphone.touch")),
                    ..Default::default()
                })
                .map(|o| o.len() == 1)
                .unwrap_or(false),
            Duration::from_secs(3),
        )
        .await,
        "AIP touch 必須落成剛好一筆 iphone.touch observation"
    );
    let observed = rt
        .store
        .query_observations(&ObservationQuery {
            receptor_id: Some(ReceptorId::new("iphone.touch")),
            ..Default::default()
        })
        .expect("observations");
    assert_eq!(observed[0].facts.get("kind"), Some(&json!("tap")));
}

// ---------------------------------------------------------------------------
// 2. 重複 messageId → accepted{duplicate}，不重套用
// ---------------------------------------------------------------------------

#[tokio::test]
async fn duplicate_touch_is_accepted_once_and_never_reapplied() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    send_json(&mut ws, aip(touch_envelope(&device_id, "fx-dup", "tap"))).await;
    let first = collect_aip(&mut ws, 3, Duration::from_secs(5)).await;
    let revision = find(&first, "state").expect("patch")["payload"]["revision"]
        .as_u64()
        .expect("revision");

    send_json(&mut ws, aip(touch_envelope(&device_id, "fx-dup", "tap"))).await;
    let second = collect_aip(&mut ws, 1, Duration::from_secs(3)).await;
    let result = find(&second, "result").expect("duplicate result");
    assert_eq!(result["payload"]["status"], "accepted");
    assert_eq!(result["payload"]["duplicate"], true);
    assert_eq!(
        rt.character_session_peek().expect("snapshot").revision,
        revision,
        "重複的 messageId 不得再推進 revision"
    );
}

// ---------------------------------------------------------------------------
// 3. 過期 touch → expired（不套用）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expired_touch_is_never_applied() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;
    let before = rt.character_session_peek().expect("snapshot").revision;

    let mut envelope = touch_envelope(&device_id, "fx-expired", "tap");
    envelope["expiresAt"] = json!((Utc::now() - ChronoDuration::seconds(1)).to_rfc3339());
    send_json(&mut ws, aip(envelope)).await;
    let frames = collect_aip(&mut ws, 1, Duration::from_secs(3)).await;
    let result = find(&frames, "result").expect("expired result");
    assert_eq!(result["payload"]["status"], "expired");
    assert_eq!(
        rt.character_session_peek().expect("snapshot").revision,
        before
    );
}

// ---------------------------------------------------------------------------
// 4. 身分／越權／未知：偽造 source、task.verified、跨 session、未知 name／型別、超大
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forged_identity_and_out_of_scope_frames_are_refused() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    // 偽造 source.id → error{identity-mismatch}＋稽核。
    let mut forged = touch_envelope(&device_id, "fx-forged-id", "tap");
    forged["source"] = json!({"kind":"device","id":"iphone-someone-else"});
    send_json(&mut ws, aip(forged)).await;
    let envelope = recv_aip(&mut ws).await;
    assert_eq!(envelope["messageType"], "error", "{envelope}");
    assert_eq!(envelope["payload"]["code"], "identity-mismatch");
    assert_eq!(envelope["causationId"], "fx-forged-id");

    // 偽造 source.kind（冒充 runtime）→ 一樣拒絕，不執行。
    let mut forged_kind = touch_envelope(&device_id, "fx-forged-kind", "tap");
    forged_kind["source"] = json!({"kind":"runtime","id":"runtime"});
    send_json(&mut ws, aip(forged_kind)).await;
    let envelope = recv_aip(&mut ws).await;
    assert_eq!(envelope["payload"]["code"], "identity-mismatch");

    // device 送 task.verified → scope-denied（verified 只能由 Runtime 產生）。
    let mut verified = base_envelope("event", "task.verified", &device_id, "fx-verified");
    verified["payload"] = json!({"correlationId":"as-1"});
    send_json(&mut ws, aip(verified)).await;
    let envelope = recv_aip(&mut ws).await;
    assert_eq!(envelope["messageType"], "result");
    assert_eq!(envelope["payload"]["status"], "rejected");
    assert_eq!(envelope["payload"]["code"], "scope-denied");

    // 跨 session 注入 → not-a-member。
    let mut cross = touch_envelope(&device_id, "fx-cross", "tap");
    cross["sessionId"] = json!("session.someone-else");
    send_json(&mut ws, aip(cross)).await;
    let envelope = recv_aip(&mut ws).await;
    assert_eq!(envelope["payload"]["code"], "not-a-member");

    // 未知 name → unknown-name。
    let mut unknown_name = base_envelope(
        "event",
        "character.interaction.teleport",
        &device_id,
        "fx-unknown-name",
    );
    unknown_name["expiresAt"] = json!((Utc::now() + ChronoDuration::seconds(5)).to_rfc3339());
    send_json(&mut ws, aip(unknown_name)).await;
    let envelope = recv_aip(&mut ws).await;
    assert!(
        envelope["payload"]["code"] == json!("unknown-name")
            || envelope["payload"]["code"] == json!("scope-denied"),
        "未宣告的 event name 不得執行：{envelope}"
    );

    // 未知 message type → error{unsupported-message-type}，不執行。
    let mut unknown_type = base_envelope(
        "teleport",
        "character.interaction.touch",
        &device_id,
        "fx-unknown-type",
    );
    unknown_type["payload"] = json!({"kind":"tap"});
    send_json(&mut ws, aip(unknown_type)).await;
    let envelope = recv_aip(&mut ws).await;
    assert_eq!(envelope["messageType"], "error");
    assert_eq!(envelope["payload"]["code"], "unsupported-message-type");

    // 超大 envelope → message-too-large。
    let mut oversized = touch_envelope(&device_id, "fx-oversized", "tap");
    oversized["payload"] = json!({"kind":"tap","filler": "x".repeat(70_000)});
    send_json(&mut ws, aip(oversized)).await;
    let envelope = recv_aip(&mut ws).await;
    assert_eq!(envelope["messageType"], "error");
    assert_eq!(envelope["payload"]["code"], "message-too-large");

    // 稽核留痕（不含 token／路徑）。
    let audit = rt.store.audit_tail(200).expect("audit");
    assert!(
        audit
            .iter()
            .any(|row| row["kind"] == json!("aip.identity-mismatch")),
        "identity-mismatch 必須留稽核"
    );
}

// ---------------------------------------------------------------------------
// 5. 人工驗證：手機收到 celebrate，桌面不雙播
// ---------------------------------------------------------------------------

#[tokio::test]
async fn human_verified_celebrates_on_the_phone_without_double_playing_on_the_desktop() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    let session = rt
        .create_agent_session(
            serde_json::from_value(json!({
                "agentId": "agent.coder",
                "label": "測試工作",
                "ttlMinutes": 30,
                "dataScope": [],
                "toolScope": [],
            }))
            .expect("create input"),
        )
        .await
        .expect("agent session");
    let session_id = session.session_id.as_str().to_string();
    rt.report_agent_session(&session_id, "claimed-completed", json!({}))
        .await
        .expect("claim");
    rt.verify_agent_session(&session_id, Some("我看過輸出了".into()))
        .await
        .expect("verify");

    let frames = collect_aip(&mut ws, 4, Duration::from_secs(5)).await;
    let command = frames
        .iter()
        .find(|e| e["payload"]["intent"] == json!("celebrate"))
        .unwrap_or_else(|| panic!("手機必須收到 celebrate：{frames:?}"));
    assert_eq!(command["messageType"], "command");
    assert!(frames.iter().any(|e| e["messageType"] == json!("state")
        && e["payload"]["patch"]["truth"]["state"] == json!("verified")));

    // 桌面：既有真相投影送 verified-success；session 的 celebrate 不得再投一則。
    let intents = character_intents(&rt);
    let verified: Vec<&RuntimeEvent> = intents
        .iter()
        .filter(|e| e.payload["envelope"]["truthState"] == json!("verified"))
        .collect();
    assert_eq!(verified.len(), 1, "verified 投影只能有一則：{intents:?}");
    assert!(
        !intents
            .iter()
            .any(|e| e.payload["envelope"]["presentationHints"]["variant"] == json!("celebrate")),
        "celebrate 不投影到 CPP（桌面已由 verified-success 表達）"
    );
}

// ---------------------------------------------------------------------------
// 6. 緊急停止：state emergency＋互動被拒
// ---------------------------------------------------------------------------

#[tokio::test]
async fn emergency_stop_freezes_the_character_and_refuses_interaction() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    rt.emergency_stop("user", Some("測試".into()))
        .await
        .expect("estop");
    assert!(
        wait_for(
            || rt
                .character_session_peek()
                .map(|s| s.state["truth"]["state"] == json!("emergency"))
                .unwrap_or(false),
            Duration::from_secs(3),
        )
        .await,
        "緊急停止必須進入 session 真相"
    );
    let snapshot = rt.character_session_peek().expect("snapshot");
    assert_eq!(snapshot.state["activity"], json!("frozen"));

    send_json(
        &mut ws,
        aip(touch_envelope(&device_id, "fx-estop-touch", "tap")),
    )
    .await;
    let frames = collect_aip(&mut ws, 4, Duration::from_secs(4)).await;
    let refusal = frames
        .iter()
        .find(|e| e["causationId"] == json!("fx-estop-touch"))
        .unwrap_or_else(|| panic!("必須回一則拒絕：{frames:?}"));
    assert_eq!(refusal["payload"]["status"], "rejected");
    assert_eq!(refusal["payload"]["code"], "scope-denied");
}

// ---------------------------------------------------------------------------
// 7. 斷線 → presence offline → 重連 → resume 取回 patches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reconnecting_fixture_iphone_resumes_with_patches() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let frames = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;
    let snapshot = find(&frames, "state").expect("snapshot");
    let revision = snapshot["payload"]["revision"].as_u64().expect("revision");
    let sequence = snapshot["payload"]["sequence"].as_u64().unwrap_or(0);
    let epoch = snapshot["payload"]["sessionEpoch"].as_u64().expect("epoch");

    // 斷線：presence → offline（成員保留，等 tick 才清）。
    drop(ws);
    assert!(
        wait_for(
            || rt
                .character_session_peek()
                .map(|s| s.state["members"]
                    .as_array()
                    .map(|m| m
                        .iter()
                        .any(|entry| entry["party"]["kind"] == json!("device")
                            && entry["presence"] == json!("offline")))
                    .unwrap_or(false))
                .unwrap_or(false),
            Duration::from_secs(3),
        )
        .await,
        "斷線必須誠實反映成 presence offline：{:?}",
        rt.character_session_peek().map(|s| s.state.clone())
    );

    // 重連 → 重送 capability（協商結果不跨重啟）→ resume。
    let mut ws = reconnect(&rt, &device_id, &token).await.expect("reconnect");
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;
    send_json(
        &mut ws,
        aip(resume_query(
            &device_id,
            "fx-resume-1",
            revision,
            sequence,
            epoch,
        )),
    )
    .await;
    let response = loop {
        let envelope = recv_aip(&mut ws).await;
        if envelope["messageType"] == json!("response") {
            break envelope;
        }
    };
    assert_eq!(response["causationId"], "fx-resume-1");
    assert_eq!(response["payload"]["kind"], "patches", "{response}");
    let patches = response["payload"]["patches"]
        .as_array()
        .expect("patches array");
    assert!(!patches.is_empty(), "重連後必須補上錯過的 patch");
    assert!(patches[0]["revision"].as_u64().is_some(), "{response}");
    assert!(patches[0]["patch"].is_object(), "{response}");
    assert_eq!(patches[0]["baseRevision"].as_u64(), Some(revision));
}

// ---------------------------------------------------------------------------
// 8. epoch 不同 → session-reset snapshot
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stale_epoch_resume_gets_a_session_reset_snapshot() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    send_json(
        &mut ws,
        aip(resume_query(&device_id, "fx-resume-epoch", 1, 0, 999)),
    )
    .await;
    let response = loop {
        let envelope = recv_aip(&mut ws).await;
        if envelope["messageType"] == json!("response") {
            break envelope;
        }
    };
    assert_eq!(response["payload"]["kind"], "snapshot");
    assert_eq!(response["payload"]["reason"], "session-reset");
    assert!(response["payload"]["state"].is_object(), "{response}");
}

// ---------------------------------------------------------------------------
// 9. 未認證連線送 aip → 忽略；已撤銷裝置重連 → auth-fail＋成員移除
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unauthenticated_and_revoked_devices_never_reach_the_session() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;
    assert_eq!(
        rt.character_session_diagnostics_value()
            .expect("diagnostics")["members"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "桌面 host surface＋模擬 iPhone（fixture）"
    );

    // 未認證的第二條連線：aip frame 不得進 session。
    let status = rt.mobile_status().await.expect("status");
    let port = status["port"].as_u64().expect("port") as u16;
    let fp = status["fingerprint"].as_str().expect("fp").to_string();
    let mut stranger = connect(port, &fp).await;
    send_json(
        &mut stranger,
        aip(touch_envelope("iphone-stranger", "fx-stranger", "tap")),
    )
    .await;
    assert!(
        recv_aip_within(&mut stranger, Duration::from_millis(600))
            .await
            .is_none(),
        "未認證連線不得得到任何 aip 回應"
    );

    // 撤銷 → 成員立即移除；重連拿到 auth-fail。
    rt.mobile_revoke(&device_id).await.expect("revoke");
    assert!(
        wait_for(
            || rt
                .character_session_diagnostics_value()
                .map(|d| d["members"].as_array().map(Vec::len) == Some(1))
                .unwrap_or(false),
            Duration::from_secs(3),
        )
        .await,
        "撤銷必須把裝置移出 session"
    );
    let err = reconnect(&rt, &device_id, &token).await.unwrap_err();
    assert_eq!(err, "auth-fail");
    let _ = ws.close(None).await;
}

// ---------------------------------------------------------------------------
// 10. 速率上限：session 端 token bucket（每個成員 30/s）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flooding_the_session_is_rate_limited() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    // wss 這一層自己有 30 msg/s 的上限（超過就關連線，見 mobile_loop.rs），所以
    // session 的 token bucket 用 host 入口直接灌（同一個成員、同一個 bucket）。
    let party = interaction_aip::Party::device(&device_id);
    let mut limited = None;
    for n in 0..60 {
        let envelope: interaction_aip::Envelope =
            serde_json::from_value(touch_envelope(&device_id, &format!("fx-flood-{n}"), "tap"))
                .expect("envelope parses");
        let submission = rt
            .character_session_submit(envelope, &party)
            .await
            .expect("session enabled");
        if submission.error == Some(interaction_aip::ErrorCode::RateLimited) {
            limited = Some(n);
            break;
        }
    }
    assert!(
        limited.is_some(),
        "超過每個成員每秒的上限必須誠實回 rate-limited"
    );

    // 重新協商走同一個 bucket：已配對的裝置不能用 capability 洪水把 revision
    // 與廣播打成無界成長。
    let capability: interaction_aip::Envelope =
        serde_json::from_value(capability_envelope(&device_id)).expect("envelope parses");
    let submission = rt
        .character_session_submit(capability, &party)
        .await
        .expect("session enabled");
    assert_eq!(
        submission.error,
        Some(interaction_aip::ErrorCode::RateLimited),
        "重新協商必須跟事件共用同一個速率上限"
    );
    let _ = ws.close(None).await;
}

// ---------------------------------------------------------------------------
// 11. 關閉開關（INTERACT_AI_CHARACTER_SESSION=0）：所有入口停用，v0.5.1 行為不變
// ---------------------------------------------------------------------------

#[tokio::test]
async fn disabled_flag_falls_back_to_the_v051_behaviour() {
    let _guard = env_lock().await;
    std::env::set_var("INTERACT_AI_CHARACTER_SESSION", "0");
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    assert!(!rt.character_session_enabled());
    assert!(rt.character_session_peek().is_err());
    assert!(rt.character_session_diagnostics_value().is_err());

    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let envelope = recv_aip(&mut ws).await;
    assert_eq!(envelope["messageType"], "error");
    assert_eq!(envelope["payload"]["code"], "unsupported-capability");

    // v0.5.1 路徑不變：`character.present` 動器仍然送得到手機（等 ack 由手機端負責，
    // 這裡只證明舊路徑沒有被 Session 取代）。
    let background = rt.clone();
    let sending =
        tokio::spawn(async move { background.mobile_present_verified("as-legacy").await });
    let act = loop {
        let frame = recv_json_within(&mut ws, Duration::from_secs(5))
            .await
            .expect("act frame");
        if frame["type"] == json!("act") {
            break frame;
        }
    };
    assert_eq!(act["name"], "character.present");
    assert_eq!(act["params"]["state"], "verified-success");
    sending.abort();
    std::env::remove_var("INTERACT_AI_CHARACTER_SESSION");
    let _ = ws.close(None).await;
}

// ---------------------------------------------------------------------------
// 12. 日誌溢位 → snapshot（不是錯誤）；sequence 跳號也不是錯誤
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resume_falls_back_to_a_snapshot_when_the_log_no_longer_covers_it() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let party = interaction_aip::Party::human_surface("desktop");
    let start = rt.character_session_peek().expect("snapshot").revision;
    let epoch = rt.character_session_peek().expect("snapshot").epoch;

    // 有界事件日誌是 512 筆；推超過它（Runtime 真相事實不受成員速率限制）。
    for n in 0..600 {
        rt.character_session_submit_runtime(
            interaction_session::RuntimeFact::ReducedMotion(n % 2 == 0),
            None,
        );
    }
    let now = rt.character_session_peek().expect("snapshot");
    assert!(
        now.revision > start + 512,
        "測試前提：revision 必須超過日誌容量（{} → {}）",
        start,
        now.revision
    );

    let resume = rt
        .character_session_resume(&party, start, 0, epoch)
        .await
        .expect("resume");
    let payload = rt.character_session_resume_value(&party, resume).await;
    assert_eq!(payload["kind"], "snapshot", "{payload}");
    assert!(payload["state"].is_object());

    // 對方宣稱看過的進度超前 host（sequence gap 的另一半）：只能用 snapshot 對齊，
    // 不得把它當成錯誤、也不得憑空補出不存在的 patch。
    let resume = rt
        .character_session_resume(&party, now.revision, now.sequence + 500, epoch)
        .await
        .expect("resume");
    let payload = rt.character_session_resume_value(&party, resume).await;
    assert_eq!(payload["kind"], "snapshot", "{payload}");

    // 已經對齊的成員：patches 是空的（不是錯誤，也不會逼出無限 resume）。
    let resume = rt
        .character_session_resume(&party, now.revision, 0, epoch)
        .await
        .expect("resume");
    let payload = rt.character_session_resume_value(&party, resume).await;
    assert_eq!(payload["kind"], "patches", "{payload}");
    assert_eq!(
        payload["patches"].as_array().map(Vec::len),
        Some(0),
        "sequence 落後但 revision 已對齊 → 沒有東西要補"
    );
}

// ---------------------------------------------------------------------------
// 13. 重啟續接：revision 不歸零、epoch 不變；還原後的成員必須重送 capability
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_restart_resumes_the_same_session_and_asks_members_to_renegotiate() {
    let _guard = env_lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let (device_id, revision, epoch) = {
        let rt = runtime_at(dir.path()).await;
        hello(&rt).await;
        let (device_id, _token, mut ws) = pair(&rt).await;
        send_json(&mut ws, aip(capability_envelope(&device_id))).await;
        let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;
        send_json(
            &mut ws,
            aip(touch_envelope(&device_id, "fx-restart", "tap")),
        )
        .await;
        let _ = collect_aip(&mut ws, 3, Duration::from_secs(5)).await;
        let snapshot = rt.character_session_peek().expect("snapshot");
        rt.shutdown().await;
        (device_id, snapshot.revision, snapshot.epoch)
    };

    // 持久化檔案只能是擁有者可讀寫（狀態裡有成員身分與互動紀錄）。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("state").join("character-session.json"))
            .expect("session file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "持久化檔案必須是 0600（實際 {mode:o}）");
    }

    let rt = runtime_at(dir.path()).await;
    let restored = rt.character_session_peek().expect("snapshot");
    assert_eq!(restored.epoch, epoch, "重啟不重建 session");
    assert!(
        restored.revision >= revision,
        "重啟後 revision 不得歸零（{revision} → {}）",
        restored.revision
    );

    // 還原的成員留在名單上，但沒有協商結果：必須重送 capability 才能再送 event。
    let party = interaction_aip::Party::device(&device_id);
    let envelope: interaction_aip::Envelope =
        serde_json::from_value(touch_envelope(&device_id, "fx-after-restart", "tap"))
            .expect("envelope parses");
    let submission = rt
        .character_session_submit(envelope, &party)
        .await
        .expect("session enabled");
    assert_eq!(
        submission.error,
        Some(interaction_aip::ErrorCode::ScopeDenied),
        "還原後未重新協商就送 event 必須被拒"
    );
    rt.shutdown().await;
}

// ---------------------------------------------------------------------------
// 14. 壞掉的持久化檔案：改名 .corrupt＋epoch+1，並誠實顯示在 diagnostics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unreadable_session_file_is_quarantined_and_never_silently_reused() {
    let _guard = env_lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).expect("state dir");
    let file = state.join("character-session.json");
    std::fs::write(&file, "{\"epoch\": 7, this is not json").expect("write corrupt file");

    let rt = runtime_at(dir.path()).await;
    let snapshot = rt.character_session_peek().expect("snapshot");
    assert_eq!(
        snapshot.epoch, 8,
        "壞檔案的 epoch 必須 +1（成員才會拿到 session-reset）"
    );
    assert!(
        state.join("character-session.json.corrupt").exists(),
        "壞檔案要留證據，不得靜默刪除"
    );
    let diagnostics = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    assert!(
        diagnostics["storeNote"].as_str().is_some(),
        "載入異常必須誠實顯示：{diagnostics}"
    );
    rt.shutdown().await;
}
