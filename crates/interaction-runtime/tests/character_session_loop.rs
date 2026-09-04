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

/// 收到第一則符合 `message_type` 的 aip envelope（中途的廣播 patch 直接跳過）。
async fn recv_aip_of(ws: &mut Ws, message_type: &str, budget: Duration) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let envelope = recv_aip_within(ws, left).await?;
        if envelope["messageType"] == json!(message_type) {
            return Some(envelope);
        }
    }
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

/// Session 的 presence timeout（`SessionConfig::default()`；contract §12.7）。
const PRESENCE_TIMEOUT_MS: i64 = 45_000;

/// 等某個 async 條件成立（有界輪詢）。
async fn wait_for_async<F, Fut>(mut predicate: F, budget: Duration) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if predicate().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
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

// ---------------------------------------------------------------------------
// 1.5 存活證明：只送互動、不送 heartbeat 的手機不得被逾時踢出 session
// ---------------------------------------------------------------------------

/// 45 秒的 presence timeout 從**最後一則已驗證的訊息**算起，不是從協商算起。
/// 用 `tick_at` 注入時間：從協商起算已經超時，從最後一則 touch 起算還沒。
#[tokio::test]
async fn a_phone_that_only_touches_is_never_timed_out_of_the_session() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;

    let joined_at = Utc::now();
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    // 拉開協商與 touch 的距離，讓「從協商算起」與「從 touch 算起」分得開。
    tokio::time::sleep(Duration::from_millis(120)).await;
    send_json(
        &mut ws,
        aip(touch_envelope(&device_id, "fx-alive-1", "tap")),
    )
    .await;
    let frames = collect_aip(&mut ws, 3, Duration::from_secs(5)).await;
    assert_eq!(
        find(&frames, "result").expect("result")["payload"]["status"],
        "applied"
    );
    let touched_at = Utc::now();

    let timeout = ChronoDuration::milliseconds(PRESENCE_TIMEOUT_MS);
    let now = joined_at + timeout + ChronoDuration::milliseconds(20);
    assert!(
        now < touched_at + timeout,
        "測試前提：從最後一則 touch 算起還沒逾時"
    );
    rt.character_session_tick_at(now).await;

    let diagnostics = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    let members = diagnostics["members"].as_array().expect("members");
    let phone = members
        .iter()
        .find(|m| m["party"]["id"] == json!(device_id))
        .unwrap_or_else(|| panic!("只送 touch 的手機被踢出了 session：{diagnostics}"));
    assert_eq!(phone["presence"], "online", "互動就是存活證明");

    // 而且它還送得出互動（被踢掉之後會變成 not-a-member）。
    // tick 造成的廣播 patch 會排在前面，所以只挑 result 那一則。
    send_json(
        &mut ws,
        aip(touch_envelope(&device_id, "fx-alive-2", "pat")),
    )
    .await;
    let result = recv_aip_of(&mut ws, "result", Duration::from_secs(5))
        .await
        .expect("second result");
    assert_eq!(result["payload"]["status"], "applied", "{result}");
}

/// 舊協定的 `status` 心跳（iOS App 目前送的就是它）也是存活證明：已經協商過的手機
/// 不得因為「沒送 AIP heartbeat」就被清出成員名單。
#[tokio::test]
async fn a_negotiated_phone_that_only_sends_legacy_status_stays_a_member() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;

    let joined_at = Utc::now();
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    tokio::time::sleep(Duration::from_millis(120)).await;
    send_json(
        &mut ws,
        json!({"type":"status","sensors":{"micLevel": false},"appVersion":"fixture-heartbeat"}),
    )
    .await;
    // status 沒有回覆：等它出現在裝置清單裡，才確定 Runtime 真的處理過了。
    assert!(
        wait_for_async(
            || {
                let rt = rt.clone();
                let device_id = device_id.clone();
                async move {
                    rt.mobile_status()
                        .await
                        .ok()
                        .and_then(|status| {
                            let devices = status["devices"].as_array()?.clone();
                            Some(devices.iter().any(|d| {
                                d["deviceId"] == json!(device_id)
                                    && d["status"]["appVersion"] == json!("fixture-heartbeat")
                            }))
                        })
                        .unwrap_or(false)
                }
            },
            Duration::from_secs(3),
        )
        .await,
        "舊協定 status 沒有被處理"
    );
    let reported_at = Utc::now();

    let timeout = ChronoDuration::milliseconds(PRESENCE_TIMEOUT_MS);
    let now = joined_at + timeout + ChronoDuration::milliseconds(20);
    assert!(now < reported_at + timeout, "測試前提：status 之後還沒逾時");
    rt.character_session_tick_at(now).await;

    let diagnostics = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    let members = diagnostics["members"].as_array().expect("members");
    assert!(
        members
            .iter()
            .any(|m| m["party"]["id"] == json!(device_id) && m["presence"] == json!("online")),
        "已協商的手機送舊 status 也算存活：{diagnostics}"
    );
}

// ---------------------------------------------------------------------------
// 1.6 成員回報 observed → host 的 pending intent 結清（不再等 TTL）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_observed_report_settles_the_intent_instead_of_expiring_it() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    send_json(&mut ws, aip(touch_envelope(&device_id, "fx-obs-1", "tap"))).await;
    let frames = collect_aip(&mut ws, 3, Duration::from_secs(5)).await;
    let command = find(&frames, "command").expect("behavior command");
    let command_id = command["messageId"]
        .as_str()
        .expect("messageId")
        .to_string();

    // 手機誠實回報：它真的演了（observed ≠ verified）。
    let mut report = base_envelope(
        "result",
        "character.behavior.request",
        &device_id,
        "fx-obs-report",
    );
    report["causationId"] = json!(command_id);
    report["payload"] = json!({"status": "observed"});
    send_json(&mut ws, aip(report)).await;

    assert!(
        wait_for(
            || rt
                .character_session_diagnostics_value()
                .map(|d| d["counters"]["intents.observed"] == json!(1))
                .unwrap_or(false),
            Duration::from_secs(3),
        )
        .await,
        "observed 回報必須結清 intent：{:?}",
        rt.character_session_diagnostics_value()
    );

    // TTL 過去之後不得再稽核成過期（沒有人在等它了）。
    rt.character_session_tick_at(Utc::now() + ChronoDuration::seconds(120))
        .await;
    let diagnostics = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    assert_eq!(
        diagnostics["counters"]["intents.expired"],
        Value::Null,
        "已回報的 intent 不得再算成過期：{diagnostics}"
    );
    let audits = rt.store.audit_tail(200).expect("audit tail");
    assert!(
        !audits
            .iter()
            .any(|a| a["kind"] == json!("character.session.intent-expired")),
        "已結清的 intent 不得留下過期稽核"
    );
    assert!(
        audits
            .iter()
            .any(|a| a["kind"] == json!("character.session.intent-settled")),
        "結清必須留下稽核"
    );
}

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

    // 斷線：presence → reconnecting（成員保留；逾時後才由 tick 轉 offline，再久才清）。
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
                            && entry["presence"] == json!("reconnecting")))
                    .unwrap_or(false))
                .unwrap_or(false),
            Duration::from_secs(3),
        )
        .await,
        "斷線必須誠實反映成 presence reconnecting：{:?}",
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

// ---------------------------------------------------------------------------
// 15. §8 安全管線第 1 關對**非成員的第一則 capability** 一樣生效
//     （對抗審查 session-integrity-059／identity-binding-008／006）
// ---------------------------------------------------------------------------

/// 這則回覆本身是不是一則合法的 AIP envelope？
/// 契約 §1／§11 對**送出去**的訊息一樣有效：回一則自己都不接受的 envelope，
/// 接收端只會再拒絕一次。
fn reply_is_a_valid_envelope(frame: &Value) -> bool {
    serde_json::from_value::<interaction_aip::Envelope>(frame.clone())
        .map(|envelope| envelope.validate().is_ok())
        .unwrap_or(false)
}

fn session_member_ids(rt: &Runtime) -> Vec<String> {
    rt.character_session_peek()
        .expect("snapshot")
        .state
        .get("members")
        .and_then(Value::as_array)
        .map(|members| {
            members
                .iter()
                .filter_map(|m| m["party"]["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 一台已配對但**還不是成員**的手機，第一則 capability 的 payload 超過 §11 的
/// 32 KiB 上限（整包仍在 64 KiB 的訊息上限內，wss 送得出去）。
///
/// 舊行為：非成員分支直接 `serde_json::from_value` 後 join，從未呼叫
/// `Envelope::validate()`——payload 上限、巢狀深度、字串長度全部不生效，裝置照樣
/// 變成成員。現在必須回 `error{payload-too-large}` 且不得入會。
#[tokio::test]
async fn an_oversized_first_capability_frame_is_refused_before_it_can_join() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;

    let mut envelope = capability_envelope(&device_id);
    // 每則字串都在 §11 的 2000 字元以內：先觸發的一定是 payload 位元組上限。
    let padding: Vec<Value> = (0..48)
        .map(|i| json!(format!("character.interaction.{i}{}", "a".repeat(800))))
        .collect();
    envelope["payload"]["inputs"] = json!(padding);
    let bytes = serde_json::to_vec(&envelope).expect("envelope encodes");
    assert!(
        bytes.len() < interaction_aip::limits::MAX_MESSAGE_BYTES,
        "測試前提：整包訊息仍在 64 KiB 內（真的送得上線），實際 {}",
        bytes.len()
    );
    assert!(
        serde_json::to_vec(&envelope["payload"])
            .expect("payload")
            .len()
            > interaction_aip::limits::MAX_PAYLOAD_BYTES,
        "測試前提：payload 超過 32 KiB"
    );

    send_json(&mut ws, aip(envelope)).await;
    let frames = collect_aip(&mut ws, 1, Duration::from_secs(5)).await;
    let error = find(&frames, "error").unwrap_or_else(|| panic!("必須回 error：{frames:?}"));
    assert_eq!(
        error["payload"]["code"],
        json!("payload-too-large"),
        "{error}"
    );
    assert!(reply_is_a_valid_envelope(error), "{error}");
    assert!(
        !session_member_ids(&rt).contains(&device_id),
        "被 §8 第 1 關擋下的 frame 不得讓裝置入會：{:?}",
        session_member_ids(&rt)
    );
}

/// 跨 session 注入：capability 的 `sessionId` 指向別的 session。
/// 舊行為：非成員的 join 路徑完全不比對 `sessionId`，裝置照樣入會。
#[tokio::test]
async fn a_first_capability_frame_for_another_session_cannot_join() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;

    let mut envelope = capability_envelope(&device_id);
    envelope["sessionId"] = json!("session.somewhere-else");
    send_json(&mut ws, aip(envelope)).await;

    let frames = collect_aip(&mut ws, 1, Duration::from_secs(5)).await;
    let error = find(&frames, "error").unwrap_or_else(|| panic!("必須回 error：{frames:?}"));
    assert_eq!(error["payload"]["code"], json!("not-a-member"), "{error}");
    assert!(reply_is_a_valid_envelope(error), "{error}");
    assert!(
        !session_member_ids(&rt).contains(&device_id),
        "別的 session 的 frame 不得讓裝置入會"
    );
}

/// 身分不符的稽核與回覆都不得回顯未驗證、無長度上限的攻擊者字串。
///
/// 舊行為：`source.kind`（untagged 的任意字串）與 `name` 原封不動寫進 audit
/// （audit 不截斷、不過期），未消毒的 `messageId` 被當成 `causationId` 送回去——
/// 那則回覆自己過不了 `Envelope::validate()`。
#[tokio::test]
async fn a_forged_frame_never_echoes_unbounded_attacker_text() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    let marker = "z".repeat(50_000);
    let mut envelope = touch_envelope(&device_id, "fx-forged", "tap");
    // 宣稱一個超長的 kind（PartyKind::Unknown 保留原字串）與一個違法的 messageId
    // （超過 128 字元、含空白）。
    envelope["source"] = json!({"kind": marker.clone(), "id": device_id});
    envelope["messageId"] = json!(format!("id with spaces {}", "m".repeat(400)));
    let bytes = serde_json::to_vec(&envelope).expect("encodes");
    assert!(
        bytes.len() < interaction_aip::limits::MAX_MESSAGE_BYTES,
        "測試前提：這是一則真的送得上線的 frame（{} bytes）",
        bytes.len()
    );

    send_json(&mut ws, aip(envelope)).await;
    let frames = collect_aip(&mut ws, 1, Duration::from_secs(5)).await;
    let reply = frames.first().unwrap_or_else(|| panic!("必須有回覆"));
    assert!(
        reply_is_a_valid_envelope(reply),
        "回送的 envelope 自己必須合法（causationId 不得回顯違法 id）：{reply}"
    );
    assert!(
        !reply.to_string().contains(&marker),
        "回覆不得回顯輸入內容：{reply}"
    );

    let audits = rt.store.audit_tail(200).expect("audit tail");
    for row in &audits {
        let text = row.to_string();
        assert!(
            !text.contains(&marker),
            "稽核不得寫進無界的攻擊者字串：{}",
            &text[..text.len().min(200)]
        );
        assert!(
            text.len() < 4_000,
            "單筆稽核不得無界成長（{} bytes）",
            text.len()
        );
    }
}

// ---------------------------------------------------------------------------
// 16. presence 逾時的成員要先「暫時離線」一段時間才被清除
//     （對抗審查 reconnect-recovery-046；契約 §11）
// ---------------------------------------------------------------------------

/// 殭屍連線（socket 還在、手機不再送任何訊息）：host 自己的 presence 逾時先到。
/// 舊行為：`tick` 把成員標成 Offline 之後，同一輪就用**同一個門檻**把它 leave 掉，
/// §11 的「iPhone 暫時離線」一個 tick 都活不過，UI 從「已連接」直接跳到「沒有裝置」。
#[tokio::test]
async fn a_timed_out_member_is_offline_before_it_is_evicted() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;
    let joined_at = Utc::now();

    let timeout = ChronoDuration::milliseconds(PRESENCE_TIMEOUT_MS);
    // 剛過 presence 逾時：成員必須還在名單上，而且看得見是 offline。
    rt.character_session_tick_at(joined_at + timeout + ChronoDuration::milliseconds(20))
        .await;
    let diagnostics = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    let member = diagnostics["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|m| m["party"]["id"] == json!(device_id))
        .unwrap_or_else(|| panic!("逾時的成員不得在同一個 tick 就消失：{diagnostics}"))
        .clone();
    assert_eq!(
        member["presence"],
        json!("offline"),
        "契約 §11 的「iPhone 暫時離線」必須看得到：{diagnostics}"
    );

    // 再久一點才是幽靈成員：清除門檻比 presence 逾時晚。
    rt.character_session_tick_at(joined_at + timeout * 2 + ChronoDuration::milliseconds(20))
        .await;
    let diagnostics = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    assert!(
        !diagnostics["members"]
            .as_array()
            .expect("members")
            .iter()
            .any(|m| m["party"]["id"] == json!(device_id)),
        "離線太久的成員最後仍必須被清掉（幽靈成員＝假的「已連接」）：{diagnostics}"
    );
}

// ---------------------------------------------------------------------------
// 17. 同一台手機的新連線取代舊連線：session 成員不得被誤標離線
//     （對抗審查 reconnect-recovery-047）
// ---------------------------------------------------------------------------

/// 舊連線還活著的時候，同一台手機用同一組 token 再連一次（iOS 15 s 退避上限 <
/// 桌面 45 s idle timeout，這是常態）。舊 handler 判成 superseded：不得把
/// character session 的成員標成 offline，也不得把它踢出成員名單——手機其實還在線上。
#[tokio::test]
async fn a_superseded_transport_does_not_mark_the_session_member_offline() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, token, ws) = pair(&rt).await;
    let mut ws = ws;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;
    assert!(session_member_ids(&rt).contains(&device_id));

    // 舊 socket 不關：新連線直接取代它。
    let _ws2 = reconnect(&rt, &device_id, &token)
        .await
        .expect("同一台手機可以再連一次");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let diagnostics = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    let member = diagnostics["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|m| m["party"]["id"] == json!(device_id))
        .unwrap_or_else(|| panic!("被取代的連線不得讓成員消失：{diagnostics}"))
        .clone();
    assert_eq!(
        member["presence"],
        json!("online"),
        "換一條 socket 不是離線：{diagnostics}"
    );
    drop(ws);
}

// ---------------------------------------------------------------------------
// 18. host 進度落後成員時，回的 snapshot 必須說清楚這是重新開始
//     （對抗審查 pairing-migration-002／reconnect-recovery-041）
// ---------------------------------------------------------------------------

/// 成員記得的 revision 比 host 大（host 被重建過／還原了更舊的快照）。
/// 舊行為：epoch 相同就回一則**沒有 reason** 的普通 snapshot，revision 比成員小；
/// 接收端依 AIP §6 的防重播規則把它當 rollback 忽略，畫面卻還顯示「已同步」。
#[tokio::test]
async fn a_snapshot_that_moves_a_member_backwards_says_it_is_a_session_reset() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    let party = interaction_aip::Party::device(&device_id);
    let snapshot = rt.character_session_peek().expect("snapshot");
    let ahead = snapshot.revision + 200;
    let resume = rt
        .character_session_resume(&party, ahead, snapshot.sequence, snapshot.epoch)
        .await
        .expect("session enabled");
    let payload = rt.character_session_resume_value(&party, resume).await;

    assert_eq!(
        payload["reason"],
        json!("session-reset"),
        "host 倒退回去的 snapshot 必須是明說的重新開始，不能長得像重播攻擊：{payload}"
    );
    let revision = payload["revision"].as_u64().expect("revision");
    assert!(
        revision < ahead,
        "測試前提：host 的 revision 真的比成員記得的小（{revision} < {ahead}）"
    );
}

// ---------------------------------------------------------------------------
// 19. capability 宣告的名稱筆數有界（對抗審查 identity-binding-008）
// ---------------------------------------------------------------------------

/// 協商結果（含 `unsupported`）會留在成員紀錄裡。宣告的名稱不夾住筆數的話，
/// 一則合法大小的 payload 就能塞進上千筆。真實 renderer 只宣告個位數。
#[tokio::test]
async fn a_capability_announcement_cannot_declare_unbounded_names() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;

    let announcement = interaction_aip::CapabilityAnnouncement {
        spec_versions: vec!["aip/1.0".to_string()],
        role: Some(interaction_aip::MemberRole::RemoteRenderer),
        profiles: vec!["character-session".to_string()],
        sync_classes: vec![interaction_aip::SyncClass::Semantic],
        intents: Vec::new(),
        inputs: (0..500)
            .map(|i| format!("character.interaction.x{i}"))
            .collect(),
        features: serde_json::Map::new(),
        limits: None,
        extra: serde_json::Map::new(),
    };
    let party = interaction_aip::Party::device("iphone-fixture-flood");
    assert!(
        rt.character_session_join(party.clone(), &announcement)
            .await
            .is_err(),
        "無界的宣告必須被拒（不得默默存下上百筆 unsupported）"
    );
    assert!(
        !session_member_ids(&rt).contains(&"iphone-fixture-flood".to_string()),
        "被拒的宣告不得讓 party 入會"
    );
}

// ---------------------------------------------------------------------------
// 20. 重播同一則 query 不得再跑一次 resume／snapshot（identity-binding-009）
// ---------------------------------------------------------------------------

/// §8 第 12 關的去重對**每一種** message type 都成立，`query` 不例外。
/// 舊行為：去重命中回的是 `accepted{duplicate:true}` 且 `error` 為 None，於是
/// `character_session_device_query` 的短路守衛不會觸發，resume／snapshot 被再執行一次——
/// 多消耗一個 sequence、把 `resumes`／`snapshots` 計數器灌大，而且對方拿到的不是
/// `accepted{duplicate:true}` 而是一份全新的 response。
#[tokio::test]
async fn a_replayed_resume_query_is_deduped_instead_of_re_executed() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let frames = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;
    let snapshot = find(&frames, "state").expect("snapshot");
    let revision = snapshot["payload"]["revision"].as_u64().expect("revision");
    let sequence = snapshot["payload"]["sequence"].as_u64().unwrap_or(0);
    let epoch = snapshot["payload"]["sessionEpoch"].as_u64().expect("epoch");

    let query = aip(resume_query(
        &device_id,
        "fx-resume-replay",
        revision,
        sequence,
        epoch,
    ));
    send_json(&mut ws, query.clone()).await;
    let response = recv_aip_of(&mut ws, "response", Duration::from_secs(5))
        .await
        .expect("第一次 resume 要有 response");
    assert_eq!(response["causationId"], "fx-resume-replay");

    let after_first = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    let resumes = after_first["counters"]["resumes"].as_u64().unwrap_or(0);
    let snapshots = after_first["counters"]["snapshots"].as_u64().unwrap_or(0);
    let seq_after_first = after_first["sequence"].as_u64().expect("sequence");

    // 重播：同一個 messageId 再送一次。
    send_json(&mut ws, query).await;
    let result = recv_aip_of(&mut ws, "result", Duration::from_secs(5))
        .await
        .expect("重播必須回 result，不是再跑一次 resume");
    assert_eq!(result["payload"]["status"], json!("accepted"), "{result}");
    assert_eq!(result["payload"]["duplicate"], json!(true), "{result}");
    assert!(
        reply_is_a_valid_envelope(&result),
        "回覆本身要是合法 envelope：{result}"
    );

    let after_replay = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    assert_eq!(
        after_replay["counters"]["resumes"].as_u64().unwrap_or(0),
        resumes,
        "重播不得再灌一次 resumes：{after_replay}"
    );
    assert_eq!(
        after_replay["counters"]["snapshots"].as_u64().unwrap_or(0),
        snapshots,
        "重播不得再灌一次 snapshots：{after_replay}"
    );
    assert_eq!(
        after_replay["sequence"].as_u64().expect("sequence"),
        seq_after_first,
        "重播不得消耗 sequence（其他成員會看到假的跳號）：{after_replay}"
    );
    rt.shutdown().await;
}

// ---------------------------------------------------------------------------
// 21. 重啟時仍在生效的緊急停止要補投進 session（session-integrity-056 後半）
// ---------------------------------------------------------------------------

/// estop 旗標是 latched、跨重啟保留的；但 Character Session 的 `truth` 只來自
/// `RuntimeFact::Emergency`，而持久化快照是**有間隔的**（預設 32 個 revision 或 60 s），
/// 完全可能早於那次緊急停止。啟動時不補投，session 就會以「沒有 emergency」的狀態復活，
/// 互動事件重新被接受——AI 不可解除 emergency stop，重啟也不行。
#[tokio::test]
async fn an_engaged_emergency_stop_is_replayed_into_the_session_on_startup() {
    let _guard = env_lock().await;
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let rt = runtime_at(dir.path()).await;
        hello(&rt).await;
        rt.emergency_stop("user", Some("test".into()))
            .await
            .expect("emergency stop");
        let snapshot = rt.character_session_peek().expect("snapshot");
        assert_eq!(snapshot.state["truth"]["state"], json!("emergency"));
        rt.shutdown().await;
    }

    // 快照早於緊急停止（持久化有間隔）：這裡直接拿掉那份快照來代表「快照證明不了 emergency」。
    let file = dir.path().join("state").join("character-session.json");
    if file.exists() {
        std::fs::remove_file(&file).expect("remove snapshot");
    }

    let rt = runtime_at(dir.path()).await;
    let restored = rt.character_session_peek().expect("snapshot");
    assert_eq!(
        restored.state["truth"]["state"],
        json!("emergency"),
        "重啟後仍在生效的緊急停止必須重新出現在 session 裡：{}",
        restored.state
    );
    assert_eq!(
        restored.state["activity"],
        json!("frozen"),
        "{}",
        restored.state
    );
    rt.shutdown().await;
}

// ---------------------------------------------------------------------------
// 22. 協商結果要投影進 members[]（session-integrity-056c／§11「部分能力目前不可用」）
// ---------------------------------------------------------------------------

/// 桌面同步卡在 Runtime 沒有投影協商結果之前只能顯示「能力核對中」。
/// `members[].unsupportedIntents` 是那句話的唯一真實來源。
#[tokio::test]
async fn negotiated_unsupported_intents_reach_the_shared_state_and_diagnostics() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    // fixture 手機宣告 3 個 intent，沒宣告 `settle`。
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;

    let snapshot = rt.character_session_peek().expect("snapshot");
    let members = snapshot.state["members"].as_array().expect("members");
    let phone = members
        .iter()
        .find(|m| m["party"]["id"] == json!(device_id))
        .unwrap_or_else(|| panic!("成員應在名單上：{}", snapshot.state));
    assert_eq!(
        phone["unsupportedIntents"],
        json!(["settle"]),
        "沒宣告的 intent 要如實投影：{}",
        snapshot.state
    );

    let diagnostics = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    let entry = diagnostics["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|m| m["party"]["id"] == json!(device_id))
        .unwrap_or_else(|| panic!("diagnostics 也要看得到：{diagnostics}"))
        .clone();
    assert_eq!(entry["unsupportedIntents"], json!(["settle"]), "{entry}");
    // 桌面（host renderer）全部支援：空陣列，不是缺鍵。
    let desktop = members
        .iter()
        .find(|m| m["party"]["kind"] == json!("human-surface"))
        .unwrap_or_else(|| panic!("桌面成員應在名單上：{}", snapshot.state));
    assert_eq!(desktop["unsupportedIntents"], json!([]), "{desktop}");
    rt.shutdown().await;
}

// ---------------------------------------------------------------------------
// 23. 斷線先是「正在重新連線」，逾時之後才是「暫時離線」（reconnect-recovery-044）
// ---------------------------------------------------------------------------

/// `Presence::Reconnecting` 是 **Transport 事實**：socket 斷了、但這台裝置仍在配對狀態、
/// iOS 端正在退避重連。沒有生產者的話，契約 §11 的「iPhone 正在重新連線」與桌面
/// statusProjection 的 reconnecting 分支永遠不會被觸發（測試全綠卻是假覆蓋率）。
#[tokio::test]
async fn a_disconnected_member_is_reconnecting_before_it_is_offline() {
    let _guard = env_lock().await;
    let (_dir, rt) = runtime().await;
    hello(&rt).await;
    let (device_id, _token, ws) = pair(&rt).await;
    let mut ws = ws;
    send_json(&mut ws, aip(capability_envelope(&device_id))).await;
    let _ = collect_aip(&mut ws, 2, Duration::from_secs(5)).await;
    let joined_at = Utc::now();

    drop(ws);
    let presence_is = |rt: &Runtime, device_id: &str, want: &str| -> bool {
        rt.character_session_peek()
            .map(|s| {
                s.state["members"]
                    .as_array()
                    .map(|m| {
                        m.iter().any(|entry| {
                            entry["party"]["id"] == json!(device_id)
                                && entry["presence"] == json!(want)
                        })
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    };
    assert!(
        wait_for(
            || presence_is(&rt, &device_id, "reconnecting"),
            Duration::from_secs(3)
        )
        .await,
        "斷線的第一步是「正在重新連線」，不是「離線」：{:?}",
        rt.character_session_peek().map(|s| s.state.clone())
    );

    // 退避窗口過去仍然沒有聲音 → session tick 才把它轉成 offline。
    rt.character_session_tick_at(
        joined_at + ChronoDuration::milliseconds(PRESENCE_TIMEOUT_MS) + ChronoDuration::seconds(1),
    )
    .await;
    assert!(
        presence_is(&rt, &device_id, "offline"),
        "逾時之後才是「暫時離線」：{:?}",
        rt.character_session_peek().map(|s| s.state.clone())
    );
    rt.shutdown().await;
}
