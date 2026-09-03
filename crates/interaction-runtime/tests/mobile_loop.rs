//! iPhone Mobile Provider 閉環測試（程序內【模擬 iPhone】——明確標示：
//! 這是模擬器等級的程序內驗收，不是真機；真機驗收需實體 iPhone＋Xcode，本環境無）。
//!
//! 覆蓋：TLS 指紋釘選連線、配對 challenge-response（對/錯配對碼）、token 重連、
//! 撤銷「立即」斷線、未認證 peer 不能解除 pending act、Bonjour 服務名長度、
//! 心跳 idle timeout、斷線強制停用高風險受器、facts 白名單過濾與丟棄計數、
//! 六個動器的 wire 參數與 iOS App 驗證規則一致、estop 只廣播一次 stop-all、
//! autostart 只在還有配對裝置時開埠、BLE 掃描逾時不洩漏 pending。

use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
#[allow(unused_imports)]
use interaction_core::Actuator as _;
use interaction_runtime::mobile::{
    filter_mobile_facts, map_wire_params, mdns_service_label, MDNS_SERVICE_SHORT,
    MDNS_SERVICE_TYPE, MOBILE_ACTUATORS,
};
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

type HmacSha256 = Hmac<Sha256>;

async fn runtime() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
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
    .unwrap()
}

/// TLS 驗證器：只釘 SHA-256 指紋（模擬 iPhone 端 TOFU pin）。
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

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(port: u16, fingerprint: &str) -> Ws {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinVerifier {
            fingerprint: fingerprint.to_string(),
            provider: rustls::crypto::ring::default_provider(),
        }))
        .with_no_client_auth();
    let (ws, _) = tokio_tungstenite::connect_async_tls_with_config(
        format!("wss://127.0.0.1:{port}/"),
        None,
        false,
        Some(tokio_tungstenite::Connector::Rustls(Arc::new(config))),
    )
    .await
    .expect("wss connect");
    ws
}

async fn recv_json(ws: &mut Ws) -> Value {
    recv_json_within(ws, Duration::from_secs(5)).await
}

async fn recv_json_within(ws: &mut Ws, budget: Duration) -> Value {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, ws.next())
            .await
            .expect("reply in time")
            .expect("stream open")
            .expect("frame ok")
        {
            Message::Text(text) => return serde_json::from_str(&text).expect("json"),
            _ => continue,
        }
    }
}

async fn send_json(ws: &mut Ws, v: Value) {
    ws.send(Message::Text(v.to_string())).await.expect("send");
}

fn hmac_hex(code: &str, nonce: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(code.as_bytes()).unwrap();
    mac.update(nonce.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 完整配對 → 回傳 (deviceId, deviceToken, ws)。
async fn pair(rt: &Runtime) -> (String, String, Ws) {
    let session = rt.mobile_pairing_begin().await.unwrap();
    let port = session["port"].as_u64().unwrap() as u16;
    let fp = session["fingerprint"].as_str().unwrap().to_string();
    let code = session["code"].as_str().unwrap().to_string();
    let mut ws = connect(port, &fp).await;
    send_json(
        &mut ws,
        json!({"type":"pair-request","deviceName":"測試 iPhone","model":"iPhone15,2"}),
    )
    .await;
    let challenge = recv_json(&mut ws).await;
    assert_eq!(challenge["type"], "pair-challenge");
    let nonce = challenge["nonce"].as_str().unwrap();
    send_json(
        &mut ws,
        json!({"type":"pair-response","hmac": hmac_hex(&code, nonce)}),
    )
    .await;
    let paired = recv_json(&mut ws).await;
    assert_eq!(paired["type"], "paired", "{paired}");
    (
        paired["deviceId"].as_str().unwrap().to_string(),
        paired["deviceToken"].as_str().unwrap().to_string(),
        ws,
    )
}

// ---------------------------------------------------------------------------
// 模擬 iPhone 的 act 參數驗證：與 iOS App
// `apps/interaction-ios/InteractionCompanion/Services/ActuatorCenter.swift`
// 逐條對齊。缺欄位／超界一律 `bad-params`（App 也是這樣回）。
// ---------------------------------------------------------------------------

fn app_validate(name: &str, p: &Value) -> Result<(), String> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let i = |k: &str| p.get(k).and_then(Value::as_i64);
    match name {
        "haptic.pulse" => {
            let style = s("style").ok_or("bad-params:style")?;
            if !["light", "medium", "heavy", "purr", "heartbeat"].contains(&style.as_str()) {
                return Err("bad-params:style".into());
            }
            let count = i("count").ok_or("bad-params:count")?;
            if !(1..=5).contains(&count) {
                return Err("bad-params:count".into());
            }
            Ok(())
        }
        "notify.show" => {
            s("title").ok_or("bad-params:title/body")?;
            s("body").ok_or("bad-params:title/body")?;
            Ok(())
        }
        "tts.speak" => {
            let text = s("text").ok_or("bad-params:text")?;
            if text.is_empty() {
                return Err("bad-params:text".into());
            }
            if text.chars().count() > 200 {
                return Err("text-too-long".into());
            }
            Ok(())
        }
        "screen.flash" => {
            let color = s("color").ok_or("bad-params:color")?;
            let hex = color.strip_prefix('#').unwrap_or(&color);
            if hex.len() != 6 || u32::from_str_radix(hex, 16).is_err() {
                return Err("bad-params:color".into());
            }
            let d = i("durationMs").ok_or("bad-params:durationMs")?;
            if !(1..=1500).contains(&d) {
                return Err("bad-params:durationMs".into());
            }
            Ok(())
        }
        "torch.set" => {
            let on = p
                .get("on")
                .and_then(Value::as_bool)
                .ok_or("bad-params:on")?;
            if on {
                let d = i("durationMs").ok_or("bad-params:durationMs")?;
                if !(1..=5000).contains(&d) {
                    return Err("bad-params:durationMs".into());
                }
            }
            Ok(())
        }
        "character.present" => {
            let state = s("state").ok_or("bad-state")?;
            if ![
                "idle",
                "working",
                "waiting",
                "verified-success",
                "failed",
                "unknown",
                "emergency",
            ]
            .contains(&state.as_str())
            {
                return Err("bad-state".into());
            }
            Ok(())
        }
        other => Err(format!("unknown-act:{other}")),
    }
}

fn test_action(actuator: &str) -> interaction_core::BoundedAction {
    test_action_with(actuator, None)
}

fn test_action_with(actuator: &str, message: Option<&str>) -> interaction_core::BoundedAction {
    use interaction_core::*;
    let now = chrono::Utc::now();
    BoundedAction {
        action_id: ActionId::new(format!("act-{}", uuid::Uuid::new_v4())),
        plan_id: PlanId::new("plan-1"),
        session_id: SessionId::new("sess-1"),
        actuator_id: ActuatorId::new(actuator),
        intent: "test".into(),
        risk_class: RiskClass::BoundedSideEffect,
        requested: ActionParameters::default(),
        effective: ActionParameters {
            magnitude: Some(0.5),
            duration_ms: Some(300),
            message: message.map(str::to_string),
            ..Default::default()
        },
        policy_decisions: vec![],
        expires_at: now + chrono::Duration::minutes(1),
        issued_at: now,
        correlation_id: CorrelationId::new("c1"),
        metadata: Default::default(),
        schema_version: "1.0".into(),
    }
}

async fn enabled_actuator(rt: &Runtime, id: &str) -> Arc<dyn interaction_core::Actuator> {
    let aid = interaction_core::ActuatorId::new(id);
    rt.registry
        .set_actuator_enabled(&aid, true)
        .await
        .unwrap_or_else(|e| panic!("enable {id}: {e}"));
    rt.registry
        .actuator(&aid)
        .await
        .unwrap_or_else(|e| panic!("{id} registered: {e}"))
}

// ---------------------------------------------------------------------------
// 純函式回歸（無網路）
// ---------------------------------------------------------------------------

/// RFC 6763 §7.2：service name label ≤15 bytes，否則 mdns-sd 直接拒絕註冊。
#[test]
fn mdns_service_label_fits_rfc6763_limit() {
    let label = mdns_service_label(MDNS_SERVICE_TYPE);
    assert_eq!(label, "interact-ai");
    assert!(
        label.len() <= 15,
        "mDNS service label `{label}` = {} bytes > 15",
        label.len()
    );
    // iOS `NSBonjourServices` 用的短型別要與完整型別同源。
    assert_eq!(MDNS_SERVICE_SHORT, "_interact-ai._tcp");
    assert!(MDNS_SERVICE_TYPE.starts_with(MDNS_SERVICE_SHORT));
    assert!(mdns_service_label("_x._udp.local.").len() <= 15);
}

/// facts 白名單＝manifest `provides`；白名單外的鍵一律剝除，白名單外的
/// receptor 回 None（＝丟棄）。
#[test]
fn filter_mobile_facts_keeps_only_manifest_provides() {
    let motion = filter_mobile_facts(
        "iphone.motion",
        &json!({"event":"lifted","x":1.0,"y":2.0,"z":3.0,"raw":[1,2,3]}),
    )
    .expect("iphone.motion is whitelisted");
    assert_eq!(
        motion.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["event"]
    );
    assert_eq!(motion["event"], json!("lifted"));

    // 白名單外的 receptor：None（呼叫端會計入 droppedObservations）。
    assert!(filter_mobile_facts("iphone.raw-trajectory", &json!({"x":1})).is_none());
    assert!(filter_mobile_facts("companion.click", &json!({"kind":"tap"})).is_none());

    // 白名單內但沒有任何可接受的鍵：Some(空) —— 同樣丟棄，不寫入空觀察。
    assert!(filter_mobile_facts("iphone.battery", &json!({"imei":"x"}))
        .expect("battery whitelisted")
        .is_empty());

    // 高風險受器只收 level，其他一律剝除。
    let mic = filter_mobile_facts("iphone.mic-level", &json!({"level":0.4,"pcm":"..."}))
        .expect("mic whitelisted");
    assert_eq!(
        mic.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["level"]
    );
}

/// wire 參數映射必須落在 iOS App 的驗證範圍內（六個動器的預設輸出）。
#[test]
fn map_wire_params_defaults_are_accepted_by_the_app_rules() {
    use interaction_core::ActionParameters;
    let bare = ActionParameters::default();
    let with_message = |m: &str| ActionParameters {
        message: Some(m.to_string()),
        ..Default::default()
    };

    let (name, p) = map_wire_params("iphone.haptic", &bare).unwrap();
    assert_eq!(name, "haptic.pulse");
    assert_eq!(p["style"], "medium");
    assert_eq!(p["count"], 1);
    app_validate(name, &p).expect("haptic defaults");

    // notify/tts 沒有文字就誠實拒絕（絕不替使用者編句子）。
    assert!(map_wire_params("iphone.notify", &bare).is_err());
    assert!(map_wire_params("iphone.tts", &bare).is_err());

    let (name, p) = map_wire_params("iphone.notify", &with_message("有事找你")).unwrap();
    assert_eq!(name, "notify.show");
    // 標題不再寫死任何角色：未協商角色前用中立的「角色」，協商後跟 manifest 走。
    assert_eq!(p["title"], "角色");
    let (_, titled) = interaction_runtime::mobile::map_wire_params_titled(
        "iphone.notify",
        &with_message("有事找你"),
        Some("小樞"),
    )
    .unwrap();
    assert_eq!(titled["title"], "小樞");
    assert_eq!(p["body"], "有事找你");
    app_validate(name, &p).expect("notify defaults");

    let (name, p) = map_wire_params("iphone.tts", &with_message("測試朗讀")).unwrap();
    assert_eq!(name, "tts.speak");
    assert_eq!(p["text"], "測試朗讀");
    app_validate(name, &p).expect("tts defaults");

    let (name, p) = map_wire_params("iphone.torch", &bare).unwrap();
    assert_eq!(name, "torch.set");
    assert_eq!(p["on"], true);
    assert_eq!(p["durationMs"], 1000);
    app_validate(name, &p).expect("torch defaults");

    let (name, p) = map_wire_params("iphone.flash", &bare).unwrap();
    assert_eq!(name, "screen.flash");
    assert_eq!(p["color"], "#FFB347");
    assert_eq!(p["durationMs"], 400);
    app_validate(name, &p).expect("flash defaults");

    let (name, p) = map_wire_params("iphone.character", &bare).unwrap();
    assert_eq!(name, "character.present");
    assert_eq!(p["state"], "idle");
    app_validate(name, &p).expect("character defaults");

    // 上限 clamp：torch ≤5000ms、flash ≤1500ms（App 超界會回 bad-params）。
    let long = ActionParameters {
        duration_ms: Some(60_000),
        ..Default::default()
    };
    let (name, p) = map_wire_params("iphone.torch", &long).unwrap();
    assert_eq!(p["durationMs"], 5000);
    app_validate(name, &p).expect("torch clamp");
    let (name, p) = map_wire_params("iphone.flash", &long).unwrap();
    assert_eq!(p["durationMs"], 1500);
    app_validate(name, &p).expect("flash clamp");

    // count clamp 1..=5。
    let many = ActionParameters {
        extra: Some(json!({"count": 99})),
        ..Default::default()
    };
    let (name, p) = map_wire_params("iphone.haptic", &many).unwrap();
    assert_eq!(p["count"], 5);
    app_validate(name, &p).expect("haptic count clamp");

    let unknown = map_wire_params("iphone.laser", &bare);
    assert!(unknown.is_err(), "unknown actuator must be refused");
}

/// `verified-success` 只能由呼叫端明確帶入（human verified 流程）；
/// 絕不能從 message 推導出來——否則綠勾號會變成謊言。
#[test]
fn character_present_never_infers_verified_success() {
    use interaction_core::ActionParameters;
    let from_message = ActionParameters {
        message: Some("verified-success".into()),
        ..Default::default()
    };
    let (_, p) = map_wire_params("iphone.character", &from_message).unwrap();
    assert_eq!(
        p["state"], "idle",
        "message 不得被升級成 verified-success：{p}"
    );

    // 其他白名單狀態可以從 message 推導。
    let working = ActionParameters {
        message: Some("working".into()),
        ..Default::default()
    };
    let (_, p) = map_wire_params("iphone.character", &working).unwrap();
    assert_eq!(p["state"], "working");

    // 明確帶入也不行：綠勾只能由 runtime 的人工驗證路徑直送
    // （`mobile_present_verified`），任何 plan／agent 參數一律拒絕。
    let explicit = ActionParameters {
        extra: Some(json!({"state":"verified-success"})),
        ..Default::default()
    };
    let err = map_wire_params("iphone.character", &explicit)
        .expect_err("verified-success must never come from a plan");
    assert!(err.contains("human-verification only"), "{err}");

    // 白名單外的狀態一律拒絕。
    let bogus = ActionParameters {
        extra: Some(json!({"state":"totally-done"})),
        ..Default::default()
    };
    assert!(map_wire_params("iphone.character", &bogus).is_err());
}

// ---------------------------------------------------------------------------
// 閉環回歸（程序內模擬 iPhone）
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn pairing_observation_act_and_estop_loop() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;

    // Provider 註冊且 Available。
    let providers = rt.list_providers().await;
    let text = serde_json::to_string(&providers).unwrap_or_default();
    assert!(
        text.contains(&format!("provider.mobile.{device_id}")),
        "mobile provider registered: {text}"
    );

    // 語意觀察 → ingest（原始軌跡永不出現）。
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.motion","facts":{"event":"lifted"}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // 白名單外 receptor 被丟棄（不報錯、不擴大資料面）。
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.raw-trajectory","facts":{"x":1}}),
    )
    .await;

    // 動器：consent-gated 預設 disabled——人類啟用後才可用（誠實三段授權）。
    let actuator = enabled_actuator(&rt, "iphone.haptic").await;
    let phone = tokio::spawn(async move {
        // 模擬 iPhone：收 act → ack；收 stop-all → ack。
        let mut saw_stop_all = false;
        loop {
            match tokio::time::timeout(Duration::from_secs(8), ws.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                    match v["type"].as_str() {
                        Some("act") => {
                            send_json(
                                &mut ws,
                                json!({"type":"ack","id":v["id"],"applied":{"style":"medium","count":1}}),
                            )
                            .await;
                        }
                        Some("stop-all") => {
                            saw_stop_all = true;
                            send_json(&mut ws, json!({"type":"ack","stopAll":true})).await;
                            return saw_stop_all;
                        }
                        _ => {}
                    }
                }
                _ => return saw_stop_all,
            }
        }
    });

    let receipt = actuator
        .execute(test_action("iphone.haptic"))
        .await
        .expect("execute");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Acknowledged,
        "iPhone ack ⇒ acknowledged（絕非 completed/verified）：{receipt:?}"
    );
    assert_eq!(receipt.driver_response["deviceApplied"]["style"], "medium");

    // estop → stop-all 傳到手機。
    rt.emergency_stop("test", None).await.unwrap();
    let saw_stop_all = phone.await.unwrap();
    assert!(saw_stop_all, "estop 必須把 stop-all 送到 iPhone");
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_pairing_code_is_refused_and_session_burned() {
    let (_tmp, rt) = runtime().await;
    let session = rt.mobile_pairing_begin().await.unwrap();
    let port = session["port"].as_u64().unwrap() as u16;
    let fp = session["fingerprint"].as_str().unwrap().to_string();
    let mut ws = connect(port, &fp).await;
    send_json(
        &mut ws,
        json!({"type":"pair-request","deviceName":"壞手機","model":"x"}),
    )
    .await;
    let challenge = recv_json(&mut ws).await;
    let nonce = challenge["nonce"].as_str().unwrap();
    send_json(
        &mut ws,
        json!({"type":"pair-response","hmac": hmac_hex("000000", nonce)}),
    )
    .await;
    let fail = recv_json(&mut ws).await;
    assert_eq!(fail["type"], "pair-fail");
    // 配對期已作廢：同一 code 不能再試（防暴力）。
    send_json(
        &mut ws,
        json!({"type":"pair-request","deviceName":"壞手機","model":"x"}),
    )
    .await;
    let fail2 = recv_json(&mut ws).await;
    assert_eq!(fail2["type"], "pair-fail", "{fail2}");
    // 沒有裝置被登錄。
    let status = rt.mobile_status().await.unwrap();
    assert_eq!(status["devices"].as_array().unwrap().len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn token_reconnect_works_until_revoked() {
    let (_tmp, rt) = runtime().await;
    let (device_id, token, ws) = pair(&rt).await;
    drop(ws); // 斷線
    tokio::time::sleep(Duration::from_millis(300)).await;

    let status = rt.mobile_status().await.unwrap();
    let port = status["port"].as_u64().unwrap() as u16;
    let fp = status["fingerprint"].as_str().unwrap().to_string();

    // token 重連 → auth-ok。
    let mut ws2 = connect(port, &fp).await;
    send_json(
        &mut ws2,
        json!({"type":"auth","deviceId":device_id,"token":token}),
    )
    .await;
    let ok = recv_json(&mut ws2).await;
    assert_eq!(ok["type"], "auth-ok", "{ok}");
    drop(ws2);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 撤銷 → 再連 auth-fail。
    rt.mobile_revoke(&device_id).await.unwrap();
    let mut ws3 = connect(port, &fp).await;
    send_json(
        &mut ws3,
        json!({"type":"auth","deviceId":device_id,"token":token}),
    )
    .await;
    let fail = recv_json(&mut ws3).await;
    assert_eq!(fail["type"], "auth-fail", "{fail}");
}

/// 清單 1：撤銷必須「立即」切斷現有連線（原缺陷：連線仍 ESTABLISHED，
/// App 要到下一次重連才知道）。撤銷後的訊息一律不得被 ingest；
/// provider 停在 Revoked，斷線收尾不得再轉 Disconnected（非法轉移）。
#[tokio::test(flavor = "multi_thread")]
async fn revoke_disconnects_live_connection_immediately() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    assert!(rt.mobile.any_connected().await);

    let started = std::time::Instant::now();
    rt.mobile_revoke(&device_id).await.unwrap();

    // <2s 內收到 auth-fail(revoked)（或伺服器直接關閉連線）。
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                    if v["type"] == "auth-fail" {
                        assert_eq!(v["reason"], "revoked", "{v}");
                        return true;
                    }
                }
                Some(Ok(Message::Close(_))) | None => return true,
                Some(Err(_)) => return true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(
        closed.is_ok(),
        "撤銷後 2 秒內必須收到 auth-fail/close，實際等了 {:?}",
        started.elapsed()
    );

    // 撤銷之後推送的觀察不得被 ingest（連線已收尾）。
    let _ = ws
        .send(Message::Text(
            json!({"type":"observation","receptor":"iphone.motion","facts":{"event":"lifted"}})
                .to_string(),
        ))
        .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let observed = rt
        .store
        .query_observations(&interaction_core::ObservationQuery {
            receptor_id: Some(interaction_core::ReceptorId::new("iphone.motion")),
            ..Default::default()
        })
        .unwrap();
    assert!(
        observed.is_empty(),
        "撤銷後的連線不得再 ingest 任何觀察：{observed:?}"
    );

    // provider 停在 Revoked（斷線收尾不得覆寫成 Disconnected）。
    let pid = interaction_core::ProviderId::new(format!("provider.mobile.{device_id}"));
    let provider = rt.get_provider(&pid).await.expect("provider still listed");
    assert_eq!(
        provider.state,
        interaction_core::ProviderState::Revoked,
        "撤銷後不得再轉 Disconnected（非法轉移）"
    );

    // 裝置清單也清空。
    let status = rt.mobile_status().await.unwrap();
    assert_eq!(status["devices"].as_array().unwrap().len(), 0);
}

/// 清單 2：ack/err 必須「已認證」且來自 act 的目標手機。未認證 peer 送 ack
/// 不能解除 pending —— 動作結果誠實停在 UNKNOWN（不謊報 acknowledged）。
#[tokio::test(flavor = "multi_thread")]
async fn unauthenticated_peer_cannot_resolve_pending_act() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut phone) = pair(&rt).await;
    let status = rt.mobile_status().await.unwrap();
    let port = status["port"].as_u64().unwrap() as u16;
    let fp = status["fingerprint"].as_str().unwrap().to_string();

    // 第二條連線：完全未認證（沒有 pair 也沒有 auth）。
    let mut impostor = connect(port, &fp).await;

    let actuator = enabled_actuator(&rt, "iphone.haptic").await;
    let action = test_action("iphone.haptic");
    let action_id = action.action_id.as_str().to_string();
    let exec = tokio::spawn(async move { actuator.execute(action).await });

    // 真手機收到 act 但故意不回；由未認證 peer 冒充回 ack。
    let act = recv_json_within(&mut phone, Duration::from_secs(3)).await;
    assert_eq!(act["type"], "act");
    assert_eq!(act["id"].as_str().unwrap(), action_id);
    send_json(
        &mut impostor,
        json!({"type":"ack","id":action_id,"applied":{"style":"medium"}}),
    )
    .await;

    let receipt = exec.await.unwrap().expect("execute returns a receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Dispatched,
        "冒充的 ack 不得讓動作變成 acknowledged：{receipt:?}"
    );
    assert_eq!(receipt.driver_response["ackTimeout"], json!(true));
    // pending 不外洩。
    let status = rt.mobile_status().await.unwrap();
    assert_eq!(status["pendingActs"], 0);
}

/// 清單 4：心跳／閒置逾時。手機停止讀取（半開連線）→ idle 後伺服器斷開，
/// 狀態、provider 與動器健康度全部誠實轉為未連線／offline。
#[tokio::test(flavor = "multi_thread")]
async fn idle_connection_times_out_and_capabilities_go_offline() {
    let (_tmp, rt) = runtime().await;
    // 測試用短心跳（新連線起算）。
    rt.mobile
        .set_timeouts(Duration::from_millis(150), Duration::from_millis(600));
    let (device_id, _token, ws) = pair(&rt).await;
    let status = rt.mobile_status().await.unwrap();
    assert_eq!(status["heartbeat"]["idleTimeoutMs"], 600);
    assert_eq!(status["devices"][0]["connected"], true);

    // 手機「還活著但不再回應」：持有 socket，完全不讀（Ping 得不到 Pong）。
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    loop {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if !rt.mobile.any_connected().await {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "idle timeout 沒有斷開半開連線"
        );
    }
    drop(ws);

    let status = rt.mobile_status().await.unwrap();
    assert_eq!(status["devices"][0]["connected"], false, "{status}");
    let pid = interaction_core::ProviderId::new(format!("provider.mobile.{device_id}"));
    let provider = rt.get_provider(&pid).await.unwrap();
    assert_eq!(
        provider.state,
        interaction_core::ProviderState::Disconnected
    );

    let actuator = rt
        .registry
        .actuator_any(&interaction_core::ActuatorId::new("iphone.haptic"))
        .await
        .unwrap();
    assert_eq!(
        actuator.status().await.status,
        interaction_core::HealthStatus::Offline,
        "斷線後動器健康度必須誠實 offline"
    );
}

/// 清單 5：斷線 → 桌面端強制停用高風險受器（`iphone.mic-level`），
/// 重連後仍是 disabled（不自動恢復）。
#[tokio::test(flavor = "multi_thread")]
async fn high_risk_receptor_forced_off_on_disconnect_and_stays_off() {
    let (_tmp, rt) = runtime().await;
    let (device_id, token, ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");

    // 高風險受器預設 disabled（requires_consent）；人類啟用後才可用。
    assert!(rt.registry.receptor(&mic).await.is_err());
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    assert!(rt.registry.receptor(&mic).await.is_ok());

    drop(ws);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        rt.registry.receptor(&mic).await.is_err(),
        "斷線後高風險受器必須被強制停用"
    );

    // 重連：能力回來，但麥克風不自動恢復。
    let status = rt.mobile_status().await.unwrap();
    let port = status["port"].as_u64().unwrap() as u16;
    let fp = status["fingerprint"].as_str().unwrap().to_string();
    let mut ws2 = connect(port, &fp).await;
    send_json(
        &mut ws2,
        json!({"type":"auth","deviceId":device_id,"token":token}),
    )
    .await;
    assert_eq!(recv_json(&mut ws2).await["type"], "auth-ok");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(rt.mobile.any_connected().await);
    assert!(
        rt.registry.receptor(&mic).await.is_err(),
        "重連不得自動恢復高風險受器"
    );
}

/// 清單 3＋6：facts 依 manifest `provides` 過濾；白名單外／無可接受鍵的觀察
/// 計入 `droppedObservations`；`iphone.mic-level` 宣告 retention:none 不落地；
/// status 誠實顯示 Bonjour 註冊結果。
#[tokio::test(flavor = "multi_thread")]
async fn observations_are_filtered_and_drops_are_counted() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;

    // 原始軌跡鍵一律剝除，只留 manifest provides。
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.motion",
               "facts":{"event":"lifted","x":1.0,"y":2.0,"z":3.0}}),
    )
    .await;
    // 白名單外 receptor → dropped。
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.raw-trajectory","facts":{"x":1}}),
    )
    .await;
    // 白名單內但沒有任何可接受的鍵 → dropped。
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.battery","facts":{"imei":"x"}}),
    )
    .await;
    // 高風險受器：即使被啟用（且有 consent）也不落地（manifest retention: none）。
    rt.registry
        .set_receptor_enabled(&interaction_core::ReceptorId::new("iphone.mic-level"), true)
        .await
        .unwrap();
    rt.start_session(
        Some("human".into()),
        None,
        vec!["receptor:iphone.mic-level".into()],
    )
    .await
    .unwrap();
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.mic-level","facts":{"level":0.4,"pcm":"..."}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(600)).await;

    let motion = rt
        .store
        .query_observations(&interaction_core::ObservationQuery {
            receptor_id: Some(interaction_core::ReceptorId::new("iphone.motion")),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(motion.len(), 1, "{motion:?}");
    assert_eq!(
        motion[0]
            .facts
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["event"],
        "原始軌跡鍵必須被剝除：{:?}",
        motion[0].facts
    );

    let mic = rt
        .store
        .query_observations(&interaction_core::ObservationQuery {
            receptor_id: Some(interaction_core::ReceptorId::new("iphone.mic-level")),
            ..Default::default()
        })
        .unwrap();
    assert!(mic.is_empty(), "retention:none 的環境音量不得落地：{mic:?}");

    let status = rt.mobile_status().await.unwrap();
    assert_eq!(
        status["droppedObservations"], 2,
        "白名單外 receptor 與無可接受鍵各記一次：{status}"
    );
    // Bonjour 狀態誠實可見（本機 CI 可能註冊失敗，重點是看得到）。
    let bonjour = &status["bonjour"];
    assert_eq!(bonjour["service"], MDNS_SERVICE_SHORT);
    assert!(bonjour["advertised"].is_boolean(), "{bonjour}");
    if bonjour["advertised"] == json!(false) {
        assert!(!bonjour["error"].is_null(), "註冊失敗要說明原因：{bonjour}");
    }
}

/// 清單 7：六個動器帶預設參數都要被「像 App 一樣驗證參數」的模擬 iPhone 接受。
#[tokio::test(flavor = "multi_thread")]
async fn six_actuators_pass_app_grade_parameter_validation() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    let phone = tokio::spawn(async move {
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(15), ws.next()).await
        {
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v["type"] != "act" {
                continue;
            }
            let name = v["name"].as_str().unwrap_or_default().to_string();
            match app_validate(&name, &v["params"]) {
                Ok(()) => {
                    let _ = tx.send((name, "ack".into()));
                    send_json(
                        &mut ws,
                        json!({"type":"ack","id":v["id"],"applied":v["params"]}),
                    )
                    .await;
                }
                Err(reason) => {
                    let _ = tx.send((name, reason.clone()));
                    send_json(&mut ws, json!({"type":"err","id":v["id"],"reason":reason})).await;
                }
            }
        }
    });

    for (id, _, _) in MOBILE_ACTUATORS {
        let id: &str = id;
        // notify/tts 需要真正的文字（runtime 不替使用者編句子）。
        let message = match id {
            "iphone.notify" => Some("有事找你"),
            "iphone.tts" => Some("測試朗讀"),
            _ => None,
        };
        let actuator = enabled_actuator(&rt, id).await;
        let receipt = actuator
            .execute(test_action_with(id, message))
            .await
            .unwrap_or_else(|e| panic!("{id} execute: {e:?}"));
        let (wire, verdict) = rx.recv().await.expect("phone saw the act");
        assert_eq!(verdict, "ack", "{id} → {wire}: App 端拒絕參數（{verdict}）");
        assert_eq!(
            receipt.current_status,
            interaction_core::ActionStatus::Acknowledged,
            "{id} 應停在 acknowledged（≠ completed/verified）：{receipt:?}"
        );
    }
    phone.abort();
}

/// 清單 8：estop 在去重窗內只廣播「一則」stop-all，並讓在途 act 立刻收場；
/// 沒有任何手機連線時誠實回 Err（runtime 的 stoppedActuators 才不會灌水）。
#[tokio::test(flavor = "multi_thread")]
async fn estop_broadcasts_one_stop_all_and_ends_inflight_acts() {
    let (_tmp, rt) = runtime().await;
    // 動器要先註冊（開埠成功才註冊能力）才拿得到實例。
    rt.mobile_ensure_started().await.unwrap();

    // 沒有手機 → 誠實 Err（沒有東西被停）。
    let actuator = enabled_actuator(&rt, "iphone.haptic").await;
    assert!(
        actuator.emergency_stop().await.is_err(),
        "沒有 iPhone 連線時 stop-all 必須誠實失敗"
    );

    let (_device_id, _token, mut ws) = pair(&rt).await;
    let stop_alls = Arc::new(AtomicUsize::new(0));
    let seen = stop_alls.clone();
    let phone = tokio::spawn(async move {
        // 故意「不回 ack」：在途 act 只能靠 estop 收場。
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(8), ws.next()).await
        {
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v["type"] == "stop-all" {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    let exec_actuator = actuator.clone();
    let started = std::time::Instant::now();
    let exec =
        tokio::spawn(async move { exec_actuator.execute(test_action("iphone.haptic")).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(rt.mobile_status().await.unwrap()["pendingActs"], 1);

    rt.emergency_stop("test", None).await.unwrap();
    let receipt = exec.await.unwrap().expect("receipt");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "在途 act 必須立即收場（不是等 ack 逾時）：{:?}",
        started.elapsed()
    );
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed,
        "{receipt:?}"
    );
    assert_eq!(receipt.driver_response["outcomeUnknown"], json!(true));
    assert_eq!(rt.mobile_status().await.unwrap()["pendingActs"], 0);

    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        stop_alls.load(Ordering::SeqCst),
        1,
        "六個 mobile 動器只能共用一則 stop-all（500ms 去重）"
    );
    phone.abort();
}

/// 清單 9：`started` 只在 bind 成功後設；autostart 只在還有配對裝置時開埠。
/// 撤銷唯一裝置後重建 runtime → 不開網路服務。
#[tokio::test(flavor = "multi_thread")]
async fn autostart_is_skipped_after_the_last_device_is_revoked() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_path_buf();
    let rt = runtime_at(&home).await;
    assert_eq!(
        rt.mobile_status().await.unwrap()["started"],
        json!(false),
        "沒有配對裝置時不得自動開埠"
    );

    let (device_id, _token, ws) = pair(&rt).await;
    assert_eq!(rt.mobile_status().await.unwrap()["started"], json!(true));
    drop(ws);
    rt.mobile_revoke(&device_id).await.unwrap();
    rt.shutdown_token.cancel();
    drop(rt);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let rt2 = runtime_at(&home).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let status = rt2.mobile_status().await.unwrap();
    assert_eq!(
        status["started"],
        json!(false),
        "撤銷最後一台裝置後重啟不得再開埠：{status}"
    );
    assert!(status["port"].is_null(), "{status}");
    rt2.shutdown_token.cancel();
}

/// 清單 10：BLE 掃描沒有回覆＝結果未知（誠實 Err），且 pending 一定被移除。
#[tokio::test(flavor = "multi_thread")]
async fn ble_scan_timeout_is_honest_and_clears_pending() {
    let (_tmp, rt) = runtime().await;

    // 沒有手機 → 誠實 Unavailable，不假裝掃到 0 台。
    let err = rt.mobile_ble_scan(500).await.unwrap_err();
    assert!(err.to_string().contains("no iPhone connected"), "{err}");

    let (_device_id, _token, mut ws) = pair(&rt).await;
    let phone = tokio::spawn(async move {
        // 手機收得到但故意不回（模擬完全沒回應；BLE 閘道關閉的 App 會回 err）。
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(6), ws.next()).await
        {
            if let Message::Text(text) = msg {
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if v["type"] == "ble.scan" {
                    return true;
                }
            }
        }
        false
    });

    let err = rt.mobile_ble_scan(500).await.unwrap_err();
    assert!(err.to_string().contains("outcome unknown"), "{err}");
    assert_eq!(
        rt.mobile_status().await.unwrap()["pendingActs"],
        0,
        "逾時必須移除 pending（否則洩漏）"
    );
    assert!(phone.await.unwrap_or(false), "ble.scan 應該真的送到手機");
}

// ---------------------------------------------------------------------------
// v0.5 Phase 7 對抗審查第三輪：安全底線／不變量回歸
// ---------------------------------------------------------------------------

/// 讓模擬 iPhone 回報一次自身感測狀態（micLevel 由參數決定）。
async fn send_phone_status(ws: &mut Ws, mic_level: bool) {
    send_json(
        ws,
        json!({
            "type": "status",
            "sensors": {
                "motion": false,
                "battery": false,
                "micLevel": mic_level,
                "location": false,
                "bleGateway": false,
            },
            "permissions": {
                "microphone": "granted",
                "location": "notDetermined",
                "bluetooth": "notDetermined",
            },
        }),
    )
    .await;
}

/// `status.activeSensors` 是否含 iPhone 麥克風（有界輪詢，等 status 訊息落地）。
async fn wait_for_iphone_mic_sensor(rt: &Runtime, want: bool) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let status = rt.status().await;
        let listed = status["activeSensors"]
            .as_array()
            .map(|list| {
                list.iter()
                    .any(|s| s["kind"].as_str() == Some("iphone.mic-level"))
            })
            .unwrap_or(false);
        if listed == want || std::time::Instant::now() > deadline {
            assert_eq!(
                listed, want,
                "activeSensors 與預期不符（want={want}）：{}",
                status["activeSensors"]
            );
            return status;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 清單 2：感測不靜默——手機自報麥克風串流中時，桌面的 `status.activeSensors`
/// 必須看得到（tray／首頁／角色視窗都吃這個欄位）；斷線後必須消失。
#[tokio::test(flavor = "multi_thread")]
async fn iphone_microphone_shows_up_in_active_sensors() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");

    // 受器還沒啟用：即使手機說在串流也不算（受器 disabled ＝ 沒有授權的感測）。
    send_phone_status(&mut ws, true).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let status = rt.status().await;
    assert!(
        status["activeSensors"]
            .as_array()
            .map(|l| l.is_empty())
            .unwrap_or(false),
        "{status}"
    );

    // 人類啟用受器 → 三個條件同時成立 → 誠實顯示。
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    let status = wait_for_iphone_mic_sensor(&rt, true).await;
    let entry = status["activeSensors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "iphone.mic-level")
        .unwrap()
        .clone();
    assert!(
        entry["purpose"]
            .as_str()
            .unwrap_or_default()
            .contains("僅音量值"),
        "人話要說清楚只有音量：{entry}"
    );

    // 手機說停了 → 立刻消失。
    send_phone_status(&mut ws, false).await;
    wait_for_iphone_mic_sensor(&rt, false).await;

    // 再開，然後斷線 → 一樣必須消失（斷線也強制停用受器）。
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;
    drop(ws);
    wait_for_iphone_mic_sensor(&rt, false).await;
}

/// 清單 1：緊急停止必須連 iPhone 的感測一起停——不只是停動器。
/// (a) 手機收到的 stop-all 帶 `sensors:true`；(b) 桌面把 `iphone.mic-level`
/// 強制 disabled；(c) estop 期間再推來的音量觀察一律丟棄並計數。
#[tokio::test(flavor = "multi_thread")]
async fn emergency_stop_also_stops_iphone_sensing() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    rt.start_session(
        Some("human".into()),
        None,
        vec!["receptor:iphone.mic-level".into()],
    )
    .await
    .unwrap();
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;

    let dropped_before = rt.mobile.dropped_observations();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let phone = tokio::spawn(async move {
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(6), ws.next()).await
        {
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v["type"] == "stop-all" {
                let _ = tx.send(v);
                // 緊急停止後手機仍（錯誤地）推來音量：桌面端必須擋掉。
                send_json(
                    &mut ws,
                    json!({"type":"observation","receptor":"iphone.mic-level","facts":{"level":0.9}}),
                )
                .await;
                send_json(&mut ws, json!({"type":"ack","stopAll":true})).await;
            }
        }
    });

    rt.emergency_stop("test", None).await.unwrap();
    let stop_all = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("stop-all reached the iPhone")
        .expect("stop-all payload");
    assert_eq!(
        stop_all["sensors"],
        json!(true),
        "緊急停止的 stop-all 必須要求手機連感測一起停：{stop_all}"
    );

    // 桌面端強制停用高風險受器（重連／重啟都不自動恢復）。
    assert!(
        rt.registry.receptor(&mic).await.is_err(),
        "estop 後 iphone.mic-level 必須是 disabled"
    );
    wait_for_iphone_mic_sensor(&rt, false).await;

    // estop 期間推來的音量觀察被丟棄並計數（既不 ingest 也不發事件）。
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        rt.mobile.dropped_observations() > dropped_before,
        "estop 期間的高風險觀察必須被丟棄並計數"
    );
    let heard = rt
        .events
        .recent(200)
        .into_iter()
        .filter(|e| e.event_type == interaction_core::EventType::ReceptorObservation)
        .any(|e| e.payload["receptorId"] == json!("iphone.mic-level"));
    assert!(!heard, "estop 之後不得再有 iphone.mic-level 觀察事件");
    phone.abort();
}

/// 清單 5：`iphone.mic-level` 宣告 requires_consent —— ingest 必須真的驗。
/// 沒有 session／沒有 consent／consent 被撤銷之後的觀察一律丟棄＋計數。
#[tokio::test(flavor = "multi_thread")]
async fn mic_level_observations_need_an_explicit_session_consent() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();

    let mic_events = |rt: &Runtime| {
        rt.events
            .recent(300)
            .into_iter()
            .filter(|e| e.event_type == interaction_core::EventType::ReceptorObservation)
            .filter(|e| e.payload["receptorId"] == json!("iphone.mic-level"))
            .count()
    };

    // 沒有 session ＝ 沒有 consent。
    let before = rt.mobile.dropped_observations();
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.mic-level","facts":{"level":0.2}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(rt.mobile.dropped_observations(), before + 1);
    assert_eq!(mic_events(&rt), 0, "沒有 consent 不得產生觀察");

    // 有 session 但沒有這個 receptor 的 consent：一樣丟棄。
    rt.start_session(Some("human".into()), None, vec![])
        .await
        .unwrap();
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.mic-level","facts":{"level":0.3}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(rt.mobile.dropped_observations(), before + 2);
    assert_eq!(mic_events(&rt), 0);

    // 明確授權後才收得到。
    rt.grant_consent("receptor:iphone.mic-level", None)
        .await
        .unwrap();
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.mic-level","facts":{"level":0.4}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        rt.mobile.dropped_observations(),
        before + 2,
        "有 consent 就不該再被丟棄"
    );
    assert_eq!(mic_events(&rt), 1, "有 consent 才有觀察");

    // 撤銷 consent → 之後的觀察立刻又被丟棄。
    rt.revoke_consent("receptor:iphone.mic-level")
        .await
        .unwrap();
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.mic-level","facts":{"level":0.5}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(rt.mobile.dropped_observations(), before + 3);
    assert_eq!(mic_events(&rt), 1, "撤銷後不得再有新觀察");

    // 一次性 audit 留痕。
    let audits = rt.store.audit_tail(200).unwrap();
    assert!(
        audits
            .iter()
            .any(|a| a["kind"] == "mobile.observation-without-consent"),
        "缺 consent 的丟棄要有 audit"
    );
}

/// 清單 3：誠實階梯——手機的綠勾只能由「人工驗證」送出。
/// plan／agent 路徑（含 `extra.state`）一律 Rejected；human verify 才推送。
#[tokio::test(flavor = "multi_thread")]
async fn verified_success_is_reachable_only_through_human_verification() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, ws) = pair(&rt).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let phone = tokio::spawn(async move {
        let mut ws = ws;
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(10), ws.next()).await
        {
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v["type"] == "act" {
                let _ = tx.send(v.clone());
                send_json(
                    &mut ws,
                    json!({"type":"ack","id":v["id"],"applied":v["params"]}),
                )
                .await;
            }
        }
    });

    // AI／plan 路徑：帶 verified-success 一律 Rejected，什麼都不上線。
    let actuator = enabled_actuator(&rt, "iphone.character").await;
    let mut action = test_action("iphone.character");
    action.effective.extra = Some(json!({"state": "verified-success"}));
    let err = actuator
        .execute(action)
        .await
        .expect_err("a plan must never request the green check");
    assert!(
        matches!(err, interaction_core::ActuatorError::Rejected(_)),
        "{err:?}"
    );

    // 一般狀態照走（證明拒絕的是「那個狀態」，不是整個動器）。
    let mut working = test_action("iphone.character");
    working.effective.extra = Some(json!({"state": "working"}));
    actuator.execute(working).await.expect("working is fine");
    let act = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("phone saw the act")
        .expect("act");
    assert_eq!(act["params"]["state"], "working");

    // human verify：claim → verified 才把綠勾送到手機。
    let session = rt
        .create_agent_session(
            serde_json::from_value(json!({
                "agentId": "agent.coder",
                "label": "驗證測試",
                "ttlMinutes": 30,
                "dataScope": [],
                "toolScope": [],
                "maxMessages": 5,
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let sid = session.session_id.as_str().to_string();
    rt.report_agent_session(&sid, "claimed-completed", json!({"summary": "done"}))
        .await
        .unwrap();
    rt.verify_agent_session(&sid, Some("我看過了".into()))
        .await
        .unwrap();
    let verified = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("verified act reached the phone")
        .expect("act");
    assert_eq!(verified["name"], "character.present");
    assert_eq!(verified["params"]["state"], "verified-success");
    assert_eq!(verified["params"]["source"], "human-verification");
    phone.abort();
}

/// 清單 4：L3 硬限制不得被 `extra` 繞過——durationMs 取 policy 有效值與
/// 裝置硬上限的較小值；style 不得比 magnitude 允許的更強；magnitude 被
/// clamp 成 0 時 extra.on 也點不亮手電筒。
#[test]
fn extra_parameters_can_never_widen_the_policy_bounds() {
    use interaction_core::ActionParameters;

    // policy 給 300ms，AI 想要 999999ms → 取 300（不是 5000 硬上限）。
    let long = ActionParameters {
        magnitude: Some(0.5),
        duration_ms: Some(300),
        extra: Some(json!({"durationMs": 999_999})),
        ..Default::default()
    };
    let (name, p) = map_wire_params("iphone.torch", &long).unwrap();
    assert_eq!(p["durationMs"], 300, "extra 不得放大 policy 上限：{p}");
    app_validate(name, &p).expect("torch clamp");
    let (name, p) = map_wire_params("iphone.flash", &long).unwrap();
    assert_eq!(p["durationMs"], 300);
    app_validate(name, &p).expect("flash clamp");

    // policy 沒給 duration：extra 可當預設，但仍受裝置硬上限。
    let no_policy = ActionParameters {
        extra: Some(json!({"durationMs": 999_999})),
        ..Default::default()
    };
    let (_, p) = map_wire_params("iphone.torch", &no_policy).unwrap();
    assert_eq!(p["durationMs"], 5_000);
    let (_, p) = map_wire_params("iphone.flash", &no_policy).unwrap();
    assert_eq!(p["durationMs"], 1_500);

    // style 不得比 magnitude 允許的更強。
    let weak_but_heavy = ActionParameters {
        magnitude: Some(0.2),
        extra: Some(json!({"style": "heavy"})),
        ..Default::default()
    };
    let (name, p) = map_wire_params("iphone.haptic", &weak_but_heavy).unwrap();
    assert_eq!(p["style"], "light", "magnitude 0.2 不得演成 heavy：{p}");
    app_validate(name, &p).expect("haptic style clamp");
    let medium_but_heartbeat = ActionParameters {
        magnitude: Some(0.5),
        extra: Some(json!({"style": "heartbeat"})),
        ..Default::default()
    };
    let (_, p) = map_wire_params("iphone.haptic", &medium_but_heartbeat).unwrap();
    assert_eq!(p["style"], "medium");
    // 允許範圍內的樣式照舊（purr 屬於最弱一階）。
    let purr = ActionParameters {
        magnitude: Some(0.2),
        extra: Some(json!({"style": "purr"})),
        ..Default::default()
    };
    let (_, p) = map_wire_params("iphone.haptic", &purr).unwrap();
    assert_eq!(p["style"], "purr");
    // 未知樣式一律拒絕（App 也會回 bad-params）。
    let bogus = ActionParameters {
        extra: Some(json!({"style": "nuclear"})),
        ..Default::default()
    };
    assert!(map_wire_params("iphone.haptic", &bogus).is_err());

    // magnitude 被 clamp 成 0 ＝ 不得點亮，extra.on 也不能推翻。
    let dark = ActionParameters {
        magnitude: Some(0.0),
        extra: Some(json!({"on": true})),
        ..Default::default()
    };
    let (name, p) = map_wire_params("iphone.torch", &dark).unwrap();
    assert_eq!(p["on"], false, "policy clamp 成 0 就不得點亮：{p}");
    app_validate(name, &p).expect("torch off");
}

/// 清單 6：撤銷寫檔失敗不得回報成功（否則重啟後 token 復活）。
#[tokio::test(flavor = "multi_thread")]
async fn a_revoke_that_cannot_be_persisted_fails_honestly() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime_at(dir.path()).await;
    let (device_id, token, ws) = pair(&rt).await;
    drop(ws);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 讓寫檔一定失敗：把裝置清單檔換成目錄。
    let path = dir.path().join("state").join("mobile-devices.json");
    std::fs::remove_file(&path).ok();
    std::fs::create_dir_all(&path).unwrap();

    let err = rt
        .mobile_revoke(&device_id)
        .await
        .expect_err("an unpersisted revoke must not report success");
    assert!(
        err.to_string().contains("still paired"),
        "要誠實說出「還沒被撤銷」：{err}"
    );

    // 記憶體表沒被清掉：token 仍可用（撤銷沒有假裝發生過）。
    let status = rt.mobile_status().await.unwrap();
    assert_eq!(status["devices"].as_array().unwrap().len(), 1, "{status}");
    let port = status["port"].as_u64().unwrap() as u16;
    let fp = status["fingerprint"].as_str().unwrap().to_string();
    let mut ws2 = connect(port, &fp).await;
    send_json(
        &mut ws2,
        json!({"type":"auth","deviceId":device_id,"token":token}),
    )
    .await;
    assert_eq!(recv_json(&mut ws2).await["type"], "auth-ok");
    let audits = rt.store.audit_tail(50).unwrap();
    assert!(audits.iter().any(|a| a["kind"] == "mobile.revoke-failed"));

    // 修好目錄後再撤銷一次：這次才是真的。
    drop(ws2);
    std::fs::remove_dir_all(&path).unwrap();
    rt.mobile_revoke(&device_id).await.expect("revoke persists");
    assert_eq!(
        rt.mobile_status().await.unwrap()["devices"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

/// 清單 8：配對期可被區網上任何未認證 peer 一次燒掉（防暴力設計不變），
/// 但使用者必須看得到——status 有 `pairingBurnedAt`，audit 有 peer 位址。
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_burns_the_pairing_window_is_visible_to_the_user() {
    let (_tmp, rt) = runtime().await;
    let session = rt.mobile_pairing_begin().await.unwrap();
    let port = session["port"].as_u64().unwrap() as u16;
    let fp = session["fingerprint"].as_str().unwrap().to_string();
    assert!(
        rt.mobile_status().await.unwrap()["pairingBurnedAt"].is_null(),
        "剛開始的配對期沒有被燒過"
    );

    let mut peer = connect(port, &fp).await;
    send_json(
        &mut peer,
        json!({"type":"pair-request","deviceName":"別人的手機","model":"x"}),
    )
    .await;
    let challenge = recv_json(&mut peer).await;
    let nonce = challenge["nonce"].as_str().unwrap();
    send_json(
        &mut peer,
        json!({"type":"pair-response","hmac": hmac_hex("000000", nonce)}),
    )
    .await;
    assert_eq!(recv_json(&mut peer).await["type"], "pair-fail");

    let status = rt.mobile_status().await.unwrap();
    assert!(
        status["pairingBurnedAt"].is_string(),
        "配對期被燒掉要在 status 看得到：{status}"
    );
    assert_eq!(status["pairingActive"], json!(false));
    let audits = rt.store.audit_tail(50).unwrap();
    let burned = audits
        .iter()
        .find(|a| a["kind"] == "mobile.pair-burned-by-peer")
        .expect("audit records who burned it");
    assert!(
        burned["detail"]["peer"]
            .as_str()
            .unwrap_or_default()
            .contains("127.0.0.1"),
        "{burned}"
    );

    // 使用者重新開一段配對期 → 提示歸零。
    rt.mobile_pairing_begin().await.unwrap();
    assert!(rt.mobile_status().await.unwrap()["pairingBurnedAt"].is_null());
}

/// 清單 7：`iphone.character` 走 desktop-pet 通道，但它是送到另一台實體
/// 裝置的外部副作用——結果未知必須留在「待我決定」（不得被角色演出的
/// 豁免一起靜音）。
#[tokio::test(flavor = "multi_thread")]
async fn iphone_character_uncertain_is_still_a_pending_decision() {
    use interaction_core::*;
    let (_tmp, rt) = runtime().await;
    rt.mobile_ensure_started().await.unwrap();

    let uncertain = |actuator: &str, action: &str| ActionReceipt {
        action_id: ActionId::new(action),
        plan_id: PlanId::new("plan-x"),
        session_id: SessionId::new("sess-x"),
        actuator_id: ActuatorId::new(actuator),
        intent: "角色狀態".into(),
        requested_parameters: ActionParameters::default(),
        effective_bounded_parameters: ActionParameters::default(),
        policy_decisions: vec![],
        current_status: ActionStatus::Uncertain,
        timestamps: vec![(ActionStatus::Uncertain, chrono::Utc::now())],
        errors: vec![],
        driver_response: std::collections::BTreeMap::new(),
        verification: None,
        expires_at: None,
        correlation_id: CorrelationId::new("corr-x"),
        schema_version: SCHEMA_VERSION.to_string(),
    };
    assert!(rt
        .store
        .upsert_receipt(&uncertain("iphone.character", "act-iphone-character"), "t")
        .unwrap());
    assert!(rt
        .store
        .upsert_receipt(
            &uncertain("companion.state.present", "act-companion-state"),
            "t"
        )
        .unwrap());

    let inbox = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter::default())
        .await
        .unwrap();
    let needs = |action_id: &str| -> bool {
        inbox["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["itemId"].as_str() == Some(action_id))
            .unwrap_or_else(|| panic!("{action_id} missing from the inbox"))["needsDecision"]
            .as_bool()
            .unwrap()
    };
    assert!(
        needs("act-iphone-character"),
        "送到 iPhone 的結果未知一定要人看見"
    );
    assert!(
        !needs("act-companion-state"),
        "桌面角色演出的結果未知不佔「待我決定」"
    );
}

/// Phase 7 回歸：測試模式（無 watchdog）不得把 Bonjour 服務記錄廣播到實體區網——
/// 模擬不得有外部副作用；status.bonjour 必須誠實說「disabled」而不是假裝 advertised。
#[tokio::test]
async fn test_mode_never_advertises_bonjour_on_the_lan() {
    let (_g, rt) = runtime().await;
    let status = rt
        .mobile_ensure_started()
        .await
        .expect("mobile server starts");
    assert_eq!(status["bonjour"]["advertised"], serde_json::json!(false));
    assert!(status["bonjour"]["error"]
        .as_str()
        .unwrap_or_default()
        .contains("test mode"));
    assert_eq!(
        status["bonjour"]["service"],
        serde_json::json!("_interact-ai._tcp")
    );
}
