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
    try_connect(port, fingerprint).await.expect("wss connect")
}

/// 同 [`connect`]，但把失敗交給呼叫端判斷（連線名額用完時伺服器會直接丟掉
/// 連線，交握就不會成功——那是預期行為，不是測試錯誤）。
async fn try_connect(port: u16, fingerprint: &str) -> Result<Ws, String> {
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
    .map_err(|e| e.to_string())
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

/// Runtime 專屬真相狀態（`verified-success` 綠勾與 `emergency` 緊急停止中）
/// 只能由 runtime 的人工驗證／緊急停止路徑直送；絕不能從 message 推導、
/// 也不能由 plan 的 `extra.state` 指定——否則兩個安全狀態都會變成謊言。
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

    // `emergency` 同理：手機上的「緊急停止中」只能由真正的緊急停止產生，
    // AI 不得冒充（也不得從 message 推導成 emergency）。
    let from_message = ActionParameters {
        message: Some("emergency".into()),
        ..Default::default()
    };
    let (_, p) = map_wire_params("iphone.character", &from_message).unwrap();
    assert_eq!(p["state"], "idle", "message 不得被升級成 emergency：{p}");
    let explicit_emergency = ActionParameters {
        extra: Some(json!({"state": "emergency"})),
        ..Default::default()
    };
    let err = map_wire_params("iphone.character", &explicit_emergency)
        .expect_err("emergency must never come from a plan");
    assert!(err.contains("emergency-stop only"), "{err}");

    // 白名單外的狀態一律拒絕。
    let bogus = ActionParameters {
        extra: Some(json!({"state":"totally-done"})),
        ..Default::default()
    };
    assert!(map_wire_params("iphone.character", &bogus).is_err());
}

/// 伺服器端的參數守門必須和 iOS App 的驗證一致（甚至更嚴）：超長／型別錯／
/// 色碼錯在這裡就要被拒絕，不是等手機回 `bad-params`——那時 policy 已經授權、
/// 動作也已經送出去了。而且不代為截斷使用者的文字。
#[test]
fn map_wire_params_refuses_what_the_app_would_refuse() {
    use interaction_core::ActionParameters;
    use interaction_runtime::mobile::{
        NOTIFY_BODY_MAX_CHARS, NOTIFY_TITLE_MAX_CHARS, TTS_MAX_CHARS,
    };

    let with_message = |m: String| ActionParameters {
        message: Some(m),
        ..Default::default()
    };
    let with_extra = |v: Value| ActionParameters {
        extra: Some(v),
        ..Default::default()
    };

    // tts：剛好 200 字可以，201 字誠實拒絕（訊息要說出實際字數與上限）。
    let (name, p) = map_wire_params("iphone.tts", &with_message("字".repeat(TTS_MAX_CHARS)))
        .expect("200 chars is fine");
    app_validate(name, &p).expect("tts at the limit");
    let err = map_wire_params("iphone.tts", &with_message("字".repeat(TTS_MAX_CHARS + 1)))
        .expect_err("201 chars must be refused server-side");
    assert!(err.contains("201") && err.contains("200"), "{err}");
    // 型別錯：extra.text 不是字串。
    assert!(map_wire_params("iphone.tts", &with_extra(json!({"text": 12345}))).is_err());
    assert!(map_wire_params("iphone.tts", &with_extra(json!({"text": ""}))).is_err());

    // notify：title／body 必須是字串，且有伺服器端長度上限。
    assert!(map_wire_params(
        "iphone.notify",
        &with_extra(json!({"title": 5, "body": "x"}))
    )
    .is_err());
    assert!(map_wire_params("iphone.notify", &with_extra(json!({"body": true}))).is_err());
    assert!(map_wire_params(
        "iphone.notify",
        &with_extra(json!({"title": "標".repeat(NOTIFY_TITLE_MAX_CHARS + 1), "body": "x"})),
    )
    .is_err());
    assert!(map_wire_params(
        "iphone.notify",
        &with_extra(json!({"body": "文".repeat(NOTIFY_BODY_MAX_CHARS + 1)})),
    )
    .is_err());

    // flash：色碼必須是 6 位十六進位（App 的 parseHexColor 也是這條規則）。
    assert!(map_wire_params("iphone.flash", &with_extra(json!({"color": "red"}))).is_err());
    assert!(map_wire_params("iphone.flash", &with_extra(json!({"color": 16711680}))).is_err());
    let (name, p) = map_wire_params("iphone.flash", &with_extra(json!({"color": "ffb347"})))
        .expect("bare hex is fine");
    app_validate(name, &p).expect("flash hex without #");

    // 屬性檢查：只要映射成功，模擬 iPhone（＝App 規則）就必須接受。
    for (actuator, params) in [
        ("iphone.haptic", ActionParameters::default()),
        ("iphone.notify", with_message("嗨".into())),
        ("iphone.tts", with_message("嗨".into())),
        ("iphone.torch", ActionParameters::default()),
        ("iphone.flash", ActionParameters::default()),
        ("iphone.character", ActionParameters::default()),
    ] {
        if let Ok((name, p)) = map_wire_params(actuator, &params) {
            app_validate(name, &p)
                .unwrap_or_else(|e| panic!("{actuator} 映射出 App 會拒絕的參數：{e} / {p}"));
        }
    }
}

/// accept 錯誤的處置是純函式：暫時性錯誤退避重試（伺服器不能因為一次
/// accept 失敗就悄悄消失），監聽 socket 真的不可用或連續錯太多次才停。
#[test]
fn accept_errors_are_retried_until_they_are_clearly_hopeless() {
    use interaction_runtime::mobile::{accept_error_action, AcceptErrorAction};
    use std::io::{Error, ErrorKind};

    let transient = Error::new(ErrorKind::ConnectionAborted, "peer went away");
    assert!(matches!(
        accept_error_action(&transient, 0),
        AcceptErrorAction::RetryAfter(_)
    ));
    // 檔案描述子用盡在 Rust 沒有穩定的 ErrorKind（Uncategorized）——預設必須
    // 當成暫時性，否則一次 EMFILE 就殺掉整個 iPhone 伺服器。
    let emfile = Error::from_raw_os_error(24);
    assert!(matches!(
        accept_error_action(&emfile, 3),
        AcceptErrorAction::RetryAfter(_)
    ));
    // 退避有界且遞增。
    let first = accept_error_action(&transient, 0);
    let later = accept_error_action(&transient, 5);
    match (first, later) {
        (AcceptErrorAction::RetryAfter(a), AcceptErrorAction::RetryAfter(b)) => {
            assert!(a < b, "{a:?} 應該比 {b:?} 短");
            assert!(b <= Duration::from_secs(1), "退避上限 1 秒：{b:?}");
        }
        other => panic!("{other:?}"),
    }
    // 監聽 socket 本身不可用：不要無止境重試，誠實停下來。
    let fatal = Error::new(ErrorKind::PermissionDenied, "no");
    assert_eq!(accept_error_action(&fatal, 0), AcceptErrorAction::Stop);
    // 連續錯太多次也停（不是無界迴圈）。
    assert_eq!(
        accept_error_action(&transient, 999),
        AcceptErrorAction::Stop
    );
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

    for (id, _, _) in MOBILE_ACTUATORS.iter() {
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
    let err = rt.mobile_ble_scan(500, None).await.unwrap_err();
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

    let err = rt.mobile_ble_scan(500, None).await.unwrap_err();
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

    let payload = rt.emergency_stop("test", None).await.unwrap();
    let stop_all = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("stop-all reached the iPhone")
        .expect("stop-all payload");
    assert_eq!(
        stop_all["sensors"],
        json!(true),
        "緊急停止的 stop-all 必須要求手機連感測一起停：{stop_all}"
    );
    // estop 的回傳／事件／audit 都要逐台說出感測結果（手機有回 ack ⇒ stopped）。
    assert_eq!(payload["sensors"]["stopped"], json!(true), "{payload}");
    assert_eq!(payload["sensors"]["uncertain"], json!(false), "{payload}");
    assert_eq!(
        payload["sensors"]["devices"][0]["outcome"],
        json!("stopped"),
        "{payload}"
    );
    let estop_audit = rt
        .store
        .audit_tail(300)
        .unwrap()
        .into_iter()
        .rfind(|a| a["kind"] == json!("mobile.estop-stop-sensors"))
        .expect("mobile.estop-stop-sensors audit");
    assert_eq!(
        estop_audit["detail"]["devices"][0]["outcome"],
        json!("stopped"),
        "audit 要記每台結果，而不是只記有沒有排進佇列：{estop_audit}"
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

// ---------------------------------------------------------------------------
// v0.5 產品化：「停止所有感測」必須真的傳到 iPhone 並等確認
// （對抗審查 safety-invariants-034／035、mobile-server-040／045／047）
// ---------------------------------------------------------------------------

/// 事件流裡 `sensor.*` 事件（指定型別）針對 iPhone 麥克風的筆數。
fn iphone_sensor_events(rt: &Runtime, kind: interaction_core::EventType) -> Vec<Value> {
    rt.events
        .recent(300)
        .into_iter()
        .filter(|e| e.event_type == kind)
        .filter(|e| e.payload["sensor"] == json!("iphone.mic-level"))
        .map(|e| e.payload)
        .collect()
}

/// audit 尾端第一筆指定 kind。
fn last_audit(rt: &Runtime, kind: &str) -> Option<Value> {
    rt.store
        .audit_tail(300)
        .unwrap_or_default()
        .into_iter()
        .rfind(|a| a["kind"] == json!(kind))
}

/// 讓模擬 iPhone 在收到 stop-all 後照 iOS 的行為回覆：`ack{stopAll:true}`
/// ＋一則 `status{micLevel:false}`。回傳看到的 stop-all 訊息。
fn spawn_phone_confirming_stop_all(
    mut ws: Ws,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::UnboundedReceiver<Value>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let handle = tokio::spawn(async move {
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(8), ws.next()).await
        {
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v["type"] == "stop-all" {
                let _ = tx.send(v);
                send_json(&mut ws, json!({"type":"ack","stopAll":true})).await;
                send_phone_status(&mut ws, false).await;
            }
        }
    });
    (handle, rx)
}

/// 「停止所有感測」必須真的送到 iPhone，並等到手機確認才敢說停了。
#[tokio::test(flavor = "multi_thread")]
async fn stop_all_sensors_reaches_iphone_and_waits_for_confirmation() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;

    let (phone, mut rx) = spawn_phone_confirming_stop_all(ws);
    let report = rt.stop_all_sensors("test").await.expect("stop all sensors");
    let report = serde_json::to_value(&report).unwrap();

    // (a) 手機真的收到 stop-all，而且是「連感測一起停」。
    let stop_all = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("stop-all reached the iPhone")
        .expect("stop-all payload");
    assert_eq!(stop_all["sensors"], json!(true), "{stop_all}");

    // (b) 報告誠實逐台列出結果。
    assert_eq!(report["stopped"], json!(true), "{report}");
    assert_eq!(report["uncertain"], json!(false), "{report}");
    assert_eq!(report["local"]["microphone"], json!("idle"), "{report}");
    let device = &report["devices"][0];
    assert_eq!(device["deviceId"], json!(device_id), "{report}");
    assert_eq!(device["outcome"], json!("stopped"), "{report}");
    assert!(
        matches!(device["via"].as_str(), Some("ack") | Some("status")),
        "確認來源要說清楚：{report}"
    );

    // (c) status 立刻清空，且事件流有 sensor.stopped（帶 deviceId）。
    wait_for_iphone_mic_sensor(&rt, false).await;
    let stopped = iphone_sensor_events(&rt, interaction_core::EventType::SensorStopped);
    assert!(
        stopped.iter().any(|p| p["deviceId"] == json!(device_id)),
        "手機麥克風停止必須進事件流：{stopped:?}"
    );

    // (d) audit 記整份報告（不再是空的 detail）。
    let audit = last_audit(&rt, "sensor.stopped-all").expect("sensor.stopped-all audit");
    assert_eq!(audit["detail"]["devices"][0]["outcome"], json!("stopped"));
    assert_eq!(audit["detail"]["local"]["microphone"], json!("idle"));

    // (e) 高風險受器強制停用：要再串流必須人類重新啟用。
    assert!(
        rt.registry.receptor(&mic).await.is_err(),
        "停止所有感測之後 iphone.mic-level 必須是 disabled"
    );
    phone.abort();
}

/// 手機沒回覆＝結果未知：有界回來、誠實標 uncertain、畫面上不得消失。
#[tokio::test(flavor = "multi_thread")]
async fn stop_all_sensors_reports_unknown_when_iphone_does_not_reply() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;

    // 手機收得到但完全不回（App 當掉／背景被殺）。
    let (tx, mut saw_stop_all) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let mut ws = {
        let (probe_tx, probe_rx) = tokio::sync::oneshot::channel::<Ws>();
        let phone = tokio::spawn(async move {
            while let Ok(Some(Ok(msg))) =
                tokio::time::timeout(Duration::from_secs(8), ws.next()).await
            {
                let Message::Text(text) = msg else { continue };
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if v["type"] == "stop-all" {
                    let _ = tx.send(v);
                    break;
                }
            }
            let _ = probe_tx.send(ws);
        });
        let started = std::time::Instant::now();
        let report = rt.stop_all_sensors("test").await.expect("stop all sensors");
        let report = serde_json::to_value(&report).unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "等待必須有界（2 秒預算）：{:?}",
            started.elapsed()
        );
        assert_eq!(report["stopped"], json!(false), "{report}");
        assert_eq!(report["uncertain"], json!(true), "{report}");
        assert_eq!(
            report["devices"][0]["outcome"],
            json!("unknown"),
            "{report}"
        );
        assert!(report["devices"][0]["via"].is_null(), "{report}");

        // 未確認的感測不得從畫面上消失（消失＝宣稱已停）。
        let status = rt.status().await;
        let entry = status["activeSensors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["kind"] == "iphone.mic-level")
            .unwrap_or_else(|| panic!("結果未知時仍要列出：{}", status["activeSensors"]))
            .clone();
        assert_eq!(entry["state"], json!("stop-unknown"), "{entry}");
        assert!(
            entry["purpose"]
                .as_str()
                .unwrap_or_default()
                .contains("未知"),
            "{entry}"
        );

        // 事件與 audit 都要說「未知」。
        let uncertain = iphone_sensor_events(&rt, interaction_core::EventType::SensorStopUncertain);
        assert!(
            uncertain
                .iter()
                .any(|p| p["deviceId"] == json!(device_id) && p["outcome"] == json!("unknown")),
            "{uncertain:?}"
        );
        let audit = last_audit(&rt, "sensor.stopped-all").expect("audit");
        assert_eq!(audit["detail"]["devices"][0]["outcome"], json!("unknown"));

        let _ = tokio::time::timeout(Duration::from_secs(3), saw_stop_all.recv())
            .await
            .expect("stop-all 要真的送到手機");
        phone.await.expect("phone task");
        probe_rx.await.expect("ws back")
    };

    // 手機終於回報停了 → 清空並補一則 sensor.stopped。
    send_phone_status(&mut ws, false).await;
    wait_for_iphone_mic_sensor(&rt, false).await;
    let stopped = iphone_sensor_events(&rt, interaction_core::EventType::SensorStopped);
    assert!(
        stopped.iter().any(|p| p["deviceId"] == json!(device_id)),
        "{stopped:?}"
    );
}

/// 手機麥克風的開始／停止必須恰好在「變化」時各發一次事件
/// （30 秒心跳 status 不得洗版事件流）。
#[tokio::test(flavor = "multi_thread")]
async fn iphone_mic_status_changes_emit_sensor_events() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();

    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;
    let started = iphone_sensor_events(&rt, interaction_core::EventType::SensorStarted);
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0]["deviceId"], json!(device_id));

    // 心跳：同樣的 true 再送兩次 → 不再發事件。
    send_phone_status(&mut ws, true).await;
    send_phone_status(&mut ws, true).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        iphone_sensor_events(&rt, interaction_core::EventType::SensorStarted).len(),
        1,
        "心跳不得洗版事件流"
    );

    // 關掉 → 恰好一則 sensor.stopped。
    send_phone_status(&mut ws, false).await;
    wait_for_iphone_mic_sensor(&rt, false).await;
    assert_eq!(
        iphone_sensor_events(&rt, interaction_core::EventType::SensorStopped).len(),
        1,
        "停止也只發一次"
    );
    send_phone_status(&mut ws, false).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        iphone_sensor_events(&rt, interaction_core::EventType::SensorStopped).len(),
        1
    );
}

/// 感測不靜默：只要觀察真的在流進來，即使手機沒送過 status，
/// `activeSensors` 也不得是空的。
#[tokio::test(flavor = "multi_thread")]
async fn ingested_mic_observations_alone_make_the_sensor_visible() {
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

    // 完全沒有 status 訊息，只有觀察。
    send_json(
        &mut ws,
        json!({"type":"observation","receptor":"iphone.mic-level","facts":{"level":0.3}}),
    )
    .await;
    let status = wait_for_iphone_mic_sensor(&rt, true).await;
    let entry = status["activeSensors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "iphone.mic-level")
        .unwrap()
        .clone();
    assert_eq!(entry["state"], json!("active"), "{entry}");
}

/// 只停「這一台」：沒連線＝誠實 unreachable；連線中＝送出去並等確認。
#[tokio::test(flavor = "multi_thread")]
async fn mobile_sensors_stop_targets_one_phone_and_is_honest_when_unreachable() {
    let (_tmp, rt) = runtime().await;
    let (device_id, token, ws) = pair(&rt).await;
    let port = rt.mobile_status().await.unwrap()["port"].as_u64().unwrap() as u16;
    let fingerprint = rt.mobile_status().await.unwrap()["fingerprint"]
        .as_str()
        .unwrap()
        .to_string();

    // 沒配對過的裝置 → NotFound。
    let err = rt.mobile_sensors_stop("iphone-nope").await.unwrap_err();
    assert!(matches!(err, interaction_core::DomainError::NotFound(_)));

    // 斷線 → 誠實 unreachable（沒有任何東西被停），且留 audit。
    drop(ws);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let out = rt.mobile_sensors_stop(&device_id).await.unwrap();
    assert_eq!(out["connected"], json!(false), "{out}");
    assert_eq!(out["requested"], json!(false), "{out}");
    assert_eq!(out["outcome"], json!("unreachable"), "{out}");
    assert!(
        last_audit(&rt, "mobile.sensors-stop-not-delivered").is_some(),
        "沒送到也要留痕"
    );

    // 重新連線 → 送到並確認。
    let mut ws = connect(port, &fingerprint).await;
    send_json(
        &mut ws,
        json!({"type":"auth","deviceId":device_id,"token":token}),
    )
    .await;
    assert_eq!(recv_json(&mut ws).await["type"], "auth-ok");
    let (phone, mut rx) = spawn_phone_confirming_stop_all(ws);
    let out = rt.mobile_sensors_stop(&device_id).await.unwrap();
    assert_eq!(out["connected"], json!(true), "{out}");
    assert_eq!(out["requested"], json!(true), "{out}");
    assert_eq!(out["outcome"], json!("stopped"), "{out}");
    assert!(out["waitedMs"].is_u64(), "{out}");
    let stop_all = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("stop-all reached the iPhone")
        .expect("payload");
    assert_eq!(stop_all["sensors"], json!(true));
    let audit = last_audit(&rt, "mobile.sensors-stop").expect("audit");
    assert_eq!(audit["detail"]["outcome"], json!("stopped"));
    phone.abort();
}

/// 「測試這台手機」：ok 只代表 socket 有回答；沒連線／estop 都誠實拒絕。
#[tokio::test(flavor = "multi_thread")]
async fn mobile_test_pings_the_phone_and_never_claims_more_than_a_pong() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, ws) = pair(&rt).await;

    // 未配對 → NotFound。
    let err = rt.mobile_test("iphone-nope").await.unwrap_err();
    assert!(matches!(err, interaction_core::DomainError::NotFound(_)));

    // 連線中：tungstenite 自動回 Pong → ok（但只代表連線有回應）。
    let phone = tokio::spawn(async move {
        let mut ws = ws;
        while let Ok(Some(Ok(_))) = tokio::time::timeout(Duration::from_secs(8), ws.next()).await {}
        ws
    });
    let out = rt.mobile_test(&device_id).await.unwrap();
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["connected"], json!(true), "{out}");
    assert!(out["latencyMs"].is_u64(), "{out}");
    assert!(
        out["note"].as_str().unwrap_or_default().contains("不代表"),
        "不得宣稱 App 功能正常：{out}"
    );
    assert!(last_audit(&rt, "mobile.test").is_some());

    // 緊急停止中：什麼都不送。
    rt.emergency_stop("test", None).await.unwrap();
    let err = rt.mobile_test(&device_id).await.unwrap_err();
    assert!(
        matches!(err, interaction_core::DomainError::PolicyBlocked(_)),
        "{err:?}"
    );
    phone.abort();

    // 斷線 → not-connected（不是 ok）。
    tokio::time::sleep(Duration::from_millis(300)).await;
    rt.clear_emergency_stop("test").await.unwrap();
    let out = rt.mobile_test(&device_id).await.unwrap();
    assert_eq!(out["ok"], json!(false), "{out}");
    assert_eq!(out["connected"], json!(false), "{out}");
    assert_eq!(out["reason"], json!("not-connected"), "{out}");
}

/// 手機斷線：在途 act 立刻以「結果未知」收場，pending 不留殘骸。
#[tokio::test(flavor = "multi_thread")]
async fn pending_acts_end_when_the_phone_disconnects() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, ws) = pair(&rt).await;
    let actuator = enabled_actuator(&rt, "iphone.haptic").await;

    // 手機收得到但不回 ack，然後斷線。
    let phone = tokio::spawn(async move {
        let mut ws = ws;
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(6), ws.next()).await
        {
            if let Message::Text(text) = msg {
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if v["type"] == "act" {
                    break;
                }
            }
        }
        drop(ws);
    });

    let started = std::time::Instant::now();
    let receipt = actuator
        .execute(test_action("iphone.haptic"))
        .await
        .expect("receipt");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "斷線後不必等滿 4 秒 ack 逾時：{:?}",
        started.elapsed()
    );
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed,
        "{receipt:?}"
    );
    assert_eq!(receipt.driver_response["outcomeUnknown"], json!(true));
    assert_eq!(
        rt.mobile_status().await.unwrap()["pendingActs"],
        json!(0),
        "斷線必須清掉那台手機的 pending"
    );
    phone.await.expect("phone task");
}

/// 等待端 future 被丟棄（HTTP client 斷線／CLI 中斷）也不得洩漏 pending。
#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_waiting_future_clears_the_pending_act() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, ws) = pair(&rt).await;
    let phone = tokio::spawn(async move {
        let mut ws = ws;
        // 收得到但永遠不回。
        while let Ok(Some(Ok(_))) = tokio::time::timeout(Duration::from_secs(8), ws.next()).await {}
    });

    let scan = rt.mobile_ble_scan(4_000, None);
    // 等 pending 登記好，然後丟掉整個 future（等於呼叫端中斷）。
    let dropped = tokio::time::timeout(Duration::from_millis(400), scan).await;
    assert!(dropped.is_err(), "這裡本來就不該在 400ms 內完成");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        rt.mobile_status().await.unwrap()["pendingActs"],
        json!(0),
        "等待端消失後 pending 不得殘留"
    );
    phone.abort();
}

/// 手機回 err（含與 stop-all 競態的 stopped）＝綠勾沒上去：誠實回 Err。
#[tokio::test(flavor = "multi_thread")]
async fn mobile_present_verified_fails_when_the_phone_refuses() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, ws) = pair(&rt).await;
    let phone = tokio::spawn(async move {
        let mut ws = ws;
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(6), ws.next()).await
        {
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v["type"] == "act" {
                send_json(
                    &mut ws,
                    json!({"type":"err","id":v["id"],"reason":"bad-state"}),
                )
                .await;
            }
        }
    });

    let err = rt
        .mobile_present_verified("agent-session-1")
        .await
        .expect_err("手機拒絕就不能算成功");
    assert!(err.to_string().contains("bad-state"), "{err}");
    let audit = last_audit(&rt, "mobile.present-verified").expect("audit");
    assert_eq!(audit["detail"]["reply"], json!("err"));
    phone.abort();
}

/// E2E／CI 需要「真 daemon 但不對區網廣播」：環境開關關閉時只綁 127.0.0.1，
/// 且 status.bonjour 誠實說明原因（不得假裝 advertised）。
#[tokio::test(flavor = "multi_thread")]
async fn serve_mode_can_disable_bonjour_via_env() {
    use interaction_runtime::mobile::mobile_advertise_enabled;

    // 預設允許（真 daemon 才會真的廣播；測試模式另外關）。
    std::env::remove_var("INTERACT_AI_MOBILE_ADVERTISE");
    assert!(mobile_advertise_enabled());

    std::env::set_var("INTERACT_AI_MOBILE_ADVERTISE", "0");
    assert!(!mobile_advertise_enabled());
    let (_tmp, rt) = runtime().await;
    let status = rt.mobile_ensure_started().await.expect("server starts");
    std::env::remove_var("INTERACT_AI_MOBILE_ADVERTISE");

    assert_eq!(status["bonjour"]["advertised"], json!(false), "{status}");
    assert_eq!(status["bonjour"]["bindIp"], json!("127.0.0.1"), "{status}");
    let error = status["bonjour"]["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("INTERACT_AI_MOBILE_ADVERTISE"),
        "關閉原因要說人話：{error}"
    );
    // loopback 仍然可用（配對／連線不受影響）。
    let (_device_id, _token, _ws) = pair(&rt).await;
}

// ---------------------------------------------------------------------------
// 緊急停止的真相投影（`emergency` 只能由 Runtime 產生，而且一定要送到手機）
// ---------------------------------------------------------------------------

/// 讓模擬 iPhone 自動回 ack（act 與 stop-all 都回），並把看到的 act 交給測試。
fn spawn_phone_acking_acts(mut ws: Ws) -> (tokio::task::JoinHandle<()>, ActRx) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let handle = tokio::spawn(async move {
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(15), ws.next()).await
        {
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            match v["type"].as_str() {
                Some("act") => {
                    let _ = tx.send(v.clone());
                    send_json(
                        &mut ws,
                        json!({"type":"ack","id":v["id"],"applied":v["params"]}),
                    )
                    .await;
                }
                Some("stop-all") => {
                    send_json(&mut ws, json!({"type":"ack","stopAll":true})).await;
                }
                _ => {}
            }
        }
    });
    (handle, rx)
}

type ActRx = tokio::sync::mpsc::UnboundedReceiver<Value>;

/// 真正的緊急停止必須投影到每一台已連線手機（`character.present emergency`），
/// 而且逐台結果要誠實記在 estop payload 與 audit 裡。
#[tokio::test(flavor = "multi_thread")]
async fn estop_projects_emergency_to_connected_phone() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, ws) = pair(&rt).await;
    let (phone, mut acts) = spawn_phone_acking_acts(ws);

    let payload = rt.emergency_stop("test", None).await.unwrap();

    let act = tokio::time::timeout(Duration::from_secs(3), acts.recv())
        .await
        .expect("emergency presentation reached the phone")
        .expect("act");
    assert_eq!(act["name"], "character.present", "{act}");
    assert_eq!(act["params"]["state"], "emergency", "{act}");
    assert_eq!(
        act["params"]["source"], "runtime-estop",
        "來源必須是 runtime，不是 plan：{act}"
    );
    assert_eq!(
        payload["characterEmergency"][0]["deviceId"],
        json!(device_id),
        "{payload}"
    );
    assert_eq!(
        payload["characterEmergency"][0]["outcome"],
        json!("acknowledged"),
        "estop payload 要逐台說出投影結果：{payload}"
    );
    let audit = last_audit(&rt, "mobile.character-emergency").expect("audit");
    assert_eq!(audit["detail"]["state"], json!("emergency"), "{audit}");
    assert_eq!(
        audit["detail"]["devices"][0]["outcome"],
        json!("acknowledged"),
        "{audit}"
    );
    phone.abort();
}

/// 解除緊急停止（只有人類做得到）之後，手機不能停在「緊急停止中」。
#[tokio::test(flavor = "multi_thread")]
async fn clearing_the_emergency_stop_tells_the_phone_it_is_over() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, ws) = pair(&rt).await;
    let (phone, mut acts) = spawn_phone_acking_acts(ws);

    rt.emergency_stop("test", None).await.unwrap();
    let engaged = tokio::time::timeout(Duration::from_secs(3), acts.recv())
        .await
        .expect("emergency reached the phone")
        .expect("act");
    assert_eq!(engaged["params"]["state"], "emergency", "{engaged}");

    rt.clear_emergency_stop("test").await.unwrap();
    let cleared = tokio::time::timeout(Duration::from_secs(3), acts.recv())
        .await
        .expect("clear reached the phone")
        .expect("act");
    assert_eq!(cleared["name"], "character.present", "{cleared}");
    assert_eq!(cleared["params"]["state"], "idle", "{cleared}");
    assert_eq!(cleared["params"]["source"], "runtime-estop", "{cleared}");
    phone.abort();
}

/// 有界收集接下來的訊息（收滿 `want` 則或逾時就回目前收到的，不 panic）。
/// 給「必須同時收到兩則、順序不重要」的斷言用。
async fn collect_json_within(ws: &mut Ws, budget: Duration, want: usize) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + budget;
    let mut out = Vec::new();
    while out.len() < want {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    out.push(v);
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    out
}

/// 緊急停止期間才連上（或重連）的手機：一連上就要**被要求停止感測**
/// （stop-all { sensors:true, reason:emergency }），而且要看到緊急狀態——
/// 只換一個角色文字標籤不會關掉手機的麥克風／位置／BLE 閘道。
#[tokio::test(flavor = "multi_thread")]
async fn phone_connecting_during_estop_receives_emergency() {
    let (_tmp, rt) = runtime().await;
    let (device_id, token, ws) = pair(&rt).await;
    let status = rt.mobile_status().await.unwrap();
    let port = status["port"].as_u64().unwrap() as u16;
    let fp = status["fingerprint"].as_str().unwrap().to_string();
    drop(ws);
    for _ in 0..60 {
        if !rt.mobile.any_connected().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // 沒有任何手機連線時的 estop：誠實地什麼都沒投影。
    let payload = rt.emergency_stop("test", None).await.unwrap();
    assert_eq!(payload["characterEmergency"], json!([]), "{payload}");

    let mut ws = connect(port, &fp).await;
    send_json(
        &mut ws,
        json!({"type":"auth","deviceId":device_id,"token":token}),
    )
    .await;
    assert_eq!(recv_json(&mut ws).await["type"], "auth-ok");
    // 兩件事都要發生（順序不重要）：真的停止感測的要求，以及角色狀態投影。
    let msgs = collect_json_within(&mut ws, Duration::from_secs(8), 2).await;
    let stop_all = msgs
        .iter()
        .find(|m| m["type"] == "stop-all")
        .unwrap_or_else(|| panic!("緊急停止中連上的手機必須被要求停止感測（stop-all）：{msgs:?}"));
    assert_eq!(
        stop_all["sensors"],
        json!(true),
        "estop 中重連的手機必須連感測一起停：{stop_all}"
    );
    assert_eq!(
        stop_all["reason"],
        json!("emergency"),
        "手機要顯示「因桌面緊急停止而停用」：{stop_all}"
    );
    let act = msgs
        .iter()
        .find(|m| m["type"] == "act")
        .unwrap_or_else(|| panic!("緊急狀態也要投影到手機：{msgs:?}"));
    assert_eq!(act["name"], "character.present", "{act}");
    assert_eq!(act["params"]["state"], "emergency", "{act}");
    assert_eq!(act["params"]["source"], "runtime-estop", "{act}");
    // 逐台結果要進 audit（送出去 ≠ 手機停了：沒回覆就是 unknown）。
    let audit = last_audit(&rt, "mobile.estop-stop-sensors")
        .unwrap_or_else(|| panic!("重連時的停止感測要留 audit"));
    assert_eq!(audit["detail"]["deviceId"], json!(device_id), "{audit}");
    assert_eq!(
        audit["detail"]["devices"][0]["outcome"],
        json!("unknown"),
        "手機沒確認就必須是 unknown（不得謊稱停了）：{audit}"
    );
    // 高風險受器維持 disabled：重連不得讓它自動恢復。
    assert!(
        rt.registry
            .receptor(&interaction_core::ReceptorId::new("iphone.mic-level"))
            .await
            .is_err(),
        "estop 中重連之後 iphone.mic-level 仍須是 disabled"
    );
}

// ---------------------------------------------------------------------------
// 資源上限（未認證 peer 不得吃光連線／記憶體／處理時間）
// ---------------------------------------------------------------------------

/// 連線名額用完就在 accept 當下拒絕（不是先接進來再說）；連線關閉後名額歸還。
#[tokio::test(flavor = "multi_thread")]
async fn too_many_connections_are_refused_at_accept_and_the_slots_come_back() {
    use interaction_runtime::mobile::MOBILE_MAX_CONNS;

    let (_tmp, rt) = runtime().await;
    // 這個測試要的是「名額」，不是認證死線。
    rt.mobile.set_auth_timeout(Duration::from_secs(60));
    let session = rt.mobile_pairing_begin().await.unwrap();
    let port = session["port"].as_u64().unwrap() as u16;
    let fp = session["fingerprint"].as_str().unwrap().to_string();

    let mut held = Vec::new();
    for i in 0..MOBILE_MAX_CONNS {
        held.push(
            try_connect(port, &fp)
                .await
                .unwrap_or_else(|e| panic!("第 {i} 條連線應該在上限之內：{e}")),
        );
    }
    assert!(
        try_connect(port, &fp).await.is_err(),
        "超過連線上限的 peer 必須被拒絕"
    );
    let heartbeat = rt.mobile_status().await.unwrap()["heartbeat"].clone();
    assert_eq!(heartbeat["maxConnections"], json!(MOBILE_MAX_CONNS));
    assert!(
        heartbeat["refusedConnections"].as_u64().unwrap_or(0) >= 1,
        "被拒絕的次數要看得見：{heartbeat}"
    );

    drop(held);
    let mut recovered = false;
    for _ in 0..60 {
        if try_connect(port, &fp).await.is_ok() {
            recovered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(recovered, "連線關閉之後名額必須釋放");
}

/// 未認證連線有絕對死線：送 Ping／未知訊息「續命」也沒用。
#[tokio::test(flavor = "multi_thread")]
async fn an_unauthenticated_peer_is_closed_when_its_deadline_passes() {
    let (_tmp, rt) = runtime().await;
    rt.mobile.set_auth_timeout(Duration::from_millis(500));
    let session = rt.mobile_pairing_begin().await.unwrap();
    let port = session["port"].as_u64().unwrap() as u16;
    let fp = session["fingerprint"].as_str().unwrap().to_string();
    let mut ws = connect(port, &fp).await;

    // 每 200ms 送一次心跳（遠低於速率上限）：證明關閉的原因是認證死線，
    // 不是閒置、也不是速率限制。
    let started = std::time::Instant::now();
    let mut closed = false;
    while started.elapsed() < Duration::from_secs(4) && !closed {
        let _ = ws
            .send(Message::Text(json!({"type":"keep-alive"}).to_string()))
            .await;
        let until = tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            let left = until.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                break;
            }
            match tokio::time::timeout(left, ws.next()).await {
                Err(_) => break,
                Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => {
                    closed = true;
                    break;
                }
                Ok(Some(Ok(_))) => continue,
            }
        }
    }
    assert!(
        closed,
        "未認證連線必須在死線到期時關閉（心跳不能續命）：{:?}",
        started.elapsed()
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "關閉必須發生在死線附近，而不是閒置逾時：{:?}",
        started.elapsed()
    );
    let mut audited = None;
    for _ in 0..40 {
        audited = last_audit(&rt, "mobile.unauthenticated-timeout");
        if audited.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let audited = audited.expect("關閉未認證連線要留痕");
    assert!(
        audited["detail"]["afterMs"].as_u64().unwrap_or(0) >= 1,
        "{audited}"
    );
    // 伺服器本身沒事（單一 peer 的死線不影響服務）。
    assert_eq!(rt.mobile_status().await.unwrap()["started"], json!(true));
}

/// 單一連線的入站速率上限：狂灌訊息會被關掉並留 audit（不是讓 runtime
/// 把處理時間全花在一個 peer 身上）。
#[tokio::test(flavor = "multi_thread")]
async fn a_flooding_connection_is_rate_limited_and_audited() {
    use interaction_runtime::mobile::MOBILE_MAX_INBOUND_PER_SEC;

    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;
    for _ in 0..(MOBILE_MAX_INBOUND_PER_SEC * 4) {
        if ws
            .send(Message::Text(json!({"type":"noop"}).to_string()))
            .await
            .is_err()
        {
            break;
        }
    }
    let mut gone = false;
    for _ in 0..80 {
        if !rt.mobile.any_connected().await {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone, "超過入站速率上限的連線必須被關掉");
    let audit = last_audit(&rt, "mobile.rate-limited").expect("速率限制要留痕");
    assert_eq!(
        audit["detail"]["limitPerSec"],
        json!(MOBILE_MAX_INBOUND_PER_SEC),
        "{audit}"
    );
    assert_eq!(
        rt.mobile_status().await.unwrap()["started"],
        json!(true),
        "限制單一連線不得拖垮伺服器"
    );
}

/// 超大訊息不得被整則讀進記憶體再解析：連線直接收掉，伺服器照常運作。
#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_message_closes_the_connection() {
    use interaction_runtime::mobile::MOBILE_WS_MAX_MESSAGE_BYTES;

    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;
    assert!(rt.mobile.any_connected().await);

    let _ = ws
        .send(Message::Text("x".repeat(MOBILE_WS_MAX_MESSAGE_BYTES * 2)))
        .await;
    let mut gone = false;
    for _ in 0..80 {
        if !rt.mobile.any_connected().await {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone, "超過訊息上限的連線必須被收掉");
    assert_eq!(
        rt.mobile_status().await.unwrap()["started"],
        json!(true),
        "一條濫用連線不得拖垮整個伺服器"
    );
}

// ---------------------------------------------------------------------------
// accept 迴圈的存活與誠實
// ---------------------------------------------------------------------------

/// 暫時性的 accept 錯誤不得讓 iPhone 伺服器悄悄消失：退避重試之後照常配對。
#[tokio::test(flavor = "multi_thread")]
async fn a_transient_accept_error_does_not_kill_the_mobile_server() {
    let (_tmp, rt) = runtime().await;
    // 故障注入：接下來三次 accept 直接走錯誤分支。
    rt.mobile.inject_accept_errors(3);
    let status = rt.mobile_ensure_started().await.expect("server starts");
    assert_eq!(status["started"], json!(true));

    // 迴圈活著才配對得起來。
    let (_device_id, _token, _ws) = pair(&rt).await;
    let after = rt.mobile_status().await.unwrap();
    assert_eq!(after["started"], json!(true), "{after}");
    assert!(after["port"].as_u64().is_some(), "{after}");
}

/// accept 迴圈真的停了：status 必須說 started:false（不是繼續假裝在跑），
/// Bonjour 要撤掉並說明原因，而且下一次啟動可以乾淨重綁。
#[tokio::test(flavor = "multi_thread")]
async fn an_accept_loop_that_stops_is_reported_as_stopped() {
    let (_tmp, rt) = runtime().await;
    let status = rt.mobile_ensure_started().await.expect("server starts");
    let port = status["port"].as_u64().unwrap() as u16;

    rt.mobile_note_accept_loop_stopped(port, "simulated fatal accept error")
        .await;

    let stopped = rt.mobile_status().await.unwrap();
    assert_eq!(stopped["started"], json!(false), "{stopped}");
    assert_eq!(stopped["port"], Value::Null, "{stopped}");
    assert_eq!(stopped["bonjour"]["advertised"], json!(false), "{stopped}");
    let why = stopped["bonjour"]["error"].as_str().unwrap_or_default();
    assert!(why.contains("simulated fatal accept error"), "{stopped}");
    let audit = last_audit(&rt, "mobile.server-stopped").expect("停下來要留痕");
    assert_eq!(audit["detail"]["port"], json!(port), "{audit}");

    let restarted = rt.mobile_ensure_started().await.expect("clean restart");
    assert_eq!(restarted["started"], json!(true), "{restarted}");
    assert!(restarted["port"].as_u64().is_some(), "{restarted}");
}

// ---------------------------------------------------------------------------
// 多台 iPhone：動作必須指定目標，收據要記真正送到哪一台
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn with_two_phones_an_act_must_name_its_target_and_the_receipt_records_it() {
    let (_tmp, rt) = runtime().await;
    let (id_a, _token_a, ws_a) = pair(&rt).await;
    let (id_b, _token_b, ws_b) = pair(&rt).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, Value)>();
    let mut phones = Vec::new();
    for (id, ws) in [(id_a.clone(), ws_a), (id_b.clone(), ws_b)] {
        let tx = tx.clone();
        phones.push(tokio::spawn(async move {
            let mut ws = ws;
            while let Ok(Some(Ok(msg))) =
                tokio::time::timeout(Duration::from_secs(15), ws.next()).await
            {
                let Message::Text(text) = msg else { continue };
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if v["type"] == "act" {
                    let _ = tx.send((id.clone(), v.clone()));
                    send_json(
                        &mut ws,
                        json!({"type":"ack","id":v["id"],"applied":v["params"]}),
                    )
                    .await;
                }
            }
        }));
    }
    drop(tx);

    let actuator = enabled_actuator(&rt, "iphone.haptic").await;

    // (1) 兩台在線又沒指定目標：拒絕，而且要說出有哪些可選——絕不偷偷挑一台。
    let receipt = actuator
        .execute(test_action("iphone.haptic"))
        .await
        .expect("receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed,
        "{receipt:?}"
    );
    let why = serde_json::to_string(&receipt.errors).unwrap_or_default();
    assert!(
        why.contains(&id_a) && why.contains(&id_b) && why.contains("deviceId"),
        "錯誤要列出連線中的手機並說怎麼指定：{why}"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(400), rx.recv())
            .await
            .is_err(),
        "被拒絕的動作不得送到任何一台手機"
    );

    // (2) 指定目標：只送那一台，收據記真正的那一台。
    let mut targeted = test_action("iphone.haptic");
    targeted.effective.extra = Some(json!({"deviceId": id_b}));
    let receipt = actuator.execute(targeted).await.expect("receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Acknowledged,
        "{receipt:?}"
    );
    assert_eq!(receipt.driver_response["deviceId"], json!(id_b));
    assert_eq!(
        receipt.driver_response["deviceName"],
        json!("測試 iPhone"),
        "{receipt:?}"
    );
    let (delivered_to, act) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("act delivered")
        .expect("act");
    assert_eq!(delivered_to, id_b, "動作必須送到指定的那一台");
    assert!(
        act["params"].get("deviceId").is_none(),
        "deviceId 是路由參數，不該混進 wire 參數：{act}"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(400), rx.recv())
            .await
            .is_err(),
        "另一台不得收到同一個動作"
    );

    // (3) 指定一台沒連線的：誠實失敗，兩台都收不到。
    let mut nowhere = test_action("iphone.haptic");
    nowhere.effective.extra = Some(json!({"deviceId": "iphone-nope"}));
    let receipt = actuator.execute(nowhere).await.expect("receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Failed,
        "{receipt:?}"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(400), rx.recv())
            .await
            .is_err(),
        "指定不存在的手機不得改送別台"
    );

    // 每台的能力說明都要誠實：這六項是共用的，不是這一台專屬。
    let desc = rt
        .get_provider(&interaction_core::ProviderId::new(format!(
            "provider.mobile.{id_a}"
        )))
        .await
        .unwrap();
    let detail = desc.detail.clone().unwrap_or_default();
    assert!(
        detail.contains("共用") && detail.contains("deviceId"),
        "{detail}"
    );

    for phone in phones {
        phone.abort();
    }
}

// ---------------------------------------------------------------------------
// TLS 私鑰權限與公開身分指紋
// ---------------------------------------------------------------------------

/// 私鑰從建立的第一刻起就只有擁有者可讀寫；被放寬過的舊鑰在下次啟動時修回來
/// （修不回來就拒絕啟動，不會安靜地用一把全機可讀的私鑰）。
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn the_tls_private_key_is_owner_only_and_loose_permissions_are_repaired() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_path_buf();
    let rt = runtime_at(&home).await;
    rt.mobile_ensure_started().await.expect("server starts");

    let key = home.join("state").join("mobile-key.der");
    let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "私鑰必須是 0600（實際 {mode:o}）");
    assert!(
        !home.join("state").join("mobile-key.der.tmp").exists(),
        "暫存檔不得留在 state 目錄"
    );

    rt.shutdown_token.cancel();
    drop(rt);
    // 有人（或舊版本）把它放寬了。
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    let rt2 = runtime_at(&home).await;
    rt2.mobile_ensure_started()
        .await
        .expect("loose key is repaired, not silently used");
    let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "載入時必須把權限收回來（實際 {mode:o}）");
}

/// 對外的裝置身分指紋不得等於認證比對用的驗證值，也不得能當成憑據使用。
#[tokio::test(flavor = "multi_thread")]
async fn the_public_device_fingerprint_is_not_the_token_verifier() {
    let (_tmp, rt) = runtime().await;
    let (device_id, token, ws) = pair(&rt).await;
    let status = rt.mobile_status().await.unwrap();
    let port = status["port"].as_u64().unwrap() as u16;
    let tls_fp = status["fingerprint"].as_str().unwrap().to_string();

    let pid = interaction_core::ProviderId::new(format!("provider.mobile.{device_id}"));
    let fingerprint = rt
        .get_provider(&pid)
        .await
        .unwrap()
        .identity
        .fingerprint
        .expect("paired device has a public fingerprint");
    let verifier = format!("{:x}", Sha256::digest(token.as_bytes()));
    assert_eq!(fingerprint.len(), 64);
    assert_ne!(
        fingerprint, verifier,
        "公開的身分指紋不得直接是認證比對用的雜湊"
    );

    drop(ws);
    for _ in 0..60 {
        if !rt.mobile.any_connected().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // 指紋不能當憑據重放。
    let mut ws = connect(port, &tls_fp).await;
    send_json(
        &mut ws,
        json!({"type":"auth","deviceId":device_id,"token":fingerprint}),
    )
    .await;
    assert_eq!(recv_json(&mut ws).await["type"], "auth-fail");
    drop(ws);

    // 真正的 token 仍然可用，而且指紋跨連線穩定（衍生是決定性的）。
    let mut ws = connect(port, &tls_fp).await;
    send_json(
        &mut ws,
        json!({"type":"auth","deviceId":device_id,"token":token}),
    )
    .await;
    assert_eq!(recv_json(&mut ws).await["type"], "auth-ok");
    let again = rt
        .get_provider(&pid)
        .await
        .unwrap()
        .identity
        .fingerprint
        .expect("fingerprint");
    assert_eq!(again, fingerprint, "身分指紋必須跨連線穩定");
}

// ---------------------------------------------------------------------------
// stop-all 的 wire `reason`：使用者按的 vs 緊急停止（iOS 端據此顯示停用說明）
// ---------------------------------------------------------------------------

/// 純函式：只有緊急停止那條路徑是 `emergency`；不認得的 reason 保守地
/// 也當成 emergency（顯示比較嚴格的那一句，不會把緊急停止講成使用者操作）。
#[test]
fn stop_all_wire_reason_only_calls_the_estop_path_emergency() {
    use interaction_runtime::mobile::{
        stop_all_wire_reason, STOP_REASON_EMERGENCY, STOP_REASON_USER,
    };
    assert_eq!(stop_all_wire_reason("stop-all-sensors"), STOP_REASON_USER);
    assert_eq!(
        stop_all_wire_reason("emergency-stop"),
        STOP_REASON_EMERGENCY
    );
    assert_eq!(stop_all_wire_reason("who-knows"), STOP_REASON_EMERGENCY);
}

/// 使用者按「停止所有感測」（POST /v1/sensors/stop）→ wire 上 `reason:"user"`；
/// 手機不得顯示成「因桌面緊急停止而停用」。
#[tokio::test(flavor = "multi_thread")]
async fn stop_all_sensors_tells_the_phone_the_user_asked() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;

    let (phone, mut rx) = spawn_phone_confirming_stop_all(ws);
    rt.stop_all_sensors("test").await.expect("stop all sensors");
    let stop_all = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("stop-all reached the iPhone")
        .expect("stop-all payload");
    assert_eq!(stop_all["sensors"], json!(true), "{stop_all}");
    assert_eq!(
        stop_all["reason"],
        json!("user"),
        "使用者按的停止不得在手機上顯示成緊急停止：{stop_all}"
    );
    phone.abort();
}

/// 每機的「停止這台手機的感測」也是使用者發起的 → `reason:"user"`。
#[tokio::test(flavor = "multi_thread")]
async fn per_device_sensors_stop_tells_the_phone_the_user_asked() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, ws) = pair(&rt).await;
    let (phone, mut rx) = spawn_phone_confirming_stop_all(ws);
    rt.mobile_sensors_stop(&device_id)
        .await
        .expect("per-device stop");
    let stop_all = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("stop-all reached the iPhone")
        .expect("stop-all payload");
    assert_eq!(stop_all["reason"], json!("user"), "{stop_all}");
    phone.abort();
}

/// 緊急停止 → `reason:"emergency"`（手機顯示「因桌面緊急停止而停用」）。
#[tokio::test(flavor = "multi_thread")]
async fn emergency_stop_tells_the_phone_it_is_an_emergency() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, ws) = pair(&rt).await;
    let (phone, mut rx) = spawn_phone_confirming_stop_all(ws);
    rt.emergency_stop("test", None).await.expect("estop");
    let stop_all = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("stop-all reached the iPhone")
        .expect("stop-all payload");
    assert_eq!(stop_all["sensors"], json!(true), "{stop_all}");
    assert_eq!(
        stop_all["reason"],
        json!("emergency"),
        "緊急停止不得被手機顯示成使用者按的停止：{stop_all}"
    );
    phone.abort();
}

// ---------------------------------------------------------------------------
// F-043：「已測試」證據必須記在真正執行的那一台手機上
// ---------------------------------------------------------------------------

/// 兩台手機同時連線、動作指名第二台：`provider.mobile.<第二台>` 才拿到
/// 「已測試」證據，第一台（字典序可能在前）不得被記上它沒做過的事。
#[tokio::test(flavor = "multi_thread")]
async fn tested_evidence_lands_on_the_phone_that_actually_ran_the_action() {
    use interaction_core::SemanticIntent;
    use std::collections::BTreeMap;

    let (_tmp, rt) = runtime().await;
    let (id_1, _token_a, ws_a) = pair(&rt).await;
    let (id_2, _token_b, ws_b) = pair(&rt).await;
    assert_ne!(id_1, id_2);
    // 目標刻意挑「provider id 字典序在後」的那一台：舊實作用
    // `providers.list().find(...)`（字典序第一個）會把證據記到另一台，
    // 這個測試就會紅——不靠隨機的配對順序碰運氣。
    let (id_other, id_target) = if id_1 < id_2 {
        (id_1.clone(), id_2.clone())
    } else {
        (id_2.clone(), id_1.clone())
    };

    let mut phones = Vec::new();
    for ws in [ws_a, ws_b] {
        phones.push(tokio::spawn(async move {
            let mut ws = ws;
            while let Ok(Some(Ok(msg))) =
                tokio::time::timeout(Duration::from_secs(15), ws.next()).await
            {
                let Message::Text(text) = msg else { continue };
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if v["type"] == "act" {
                    send_json(
                        &mut ws,
                        json!({"type":"ack","id":v["id"],"applied":v["params"]}),
                    )
                    .await;
                }
            }
        }));
    }

    let _ = enabled_actuator(&rt, "iphone.haptic").await;
    // 測試 runtime 沒有 watchdog：健康快取要手動刷一次，計畫器才看得到
    // 「iPhone 已連線」（否則動器仍是 offline，計畫會被擋下）。
    rt.registry.refresh_health().await;
    // 政策仍是預設拒絕：人類明確允許這個動器與通道之後才可能執行。
    rt.update_policy(json!({
        "actuatorAllowlist": ["iphone.haptic"],
        "allowedChannels": ["conversation", "web-ui", "log", "haptic"],
    }))
    .await
    .unwrap();
    rt.start_session(
        Some("f043".into()),
        None,
        vec!["actuator:iphone.haptic".into()],
    )
    .await
    .unwrap();

    let mut intent = SemanticIntent::new("f043");
    intent.preferred_channels = vec!["haptic".into()];
    // `extra.deviceId` ＝ 指名第二台手機。
    intent.payload = Some(json!({"deviceId": id_target}));
    let plan = rt
        .create_plan(
            intent,
            vec!["iphone.haptic".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .expect("plan");
    let planned = serde_json::to_value(&plan).unwrap();
    let receipts = rt
        .execute_plan(
            &plan.plan_id,
            interaction_policy::ActionSource::ExplicitRequest,
            false,
        )
        .await
        .unwrap_or_else(|e| {
            panic!(
                "execute failed: {e}; plan: {}",
                serde_json::to_string_pretty(&planned).unwrap()
            )
        });
    let receipt = receipts.first().expect("one receipt");
    assert_eq!(
        receipt.current_status,
        interaction_core::ActionStatus::Acknowledged,
        "{receipt:?}"
    );
    assert_eq!(receipt.driver_response["deviceId"], json!(id_target));

    async fn tested_of(rt: &Runtime, id: &str) -> Option<Value> {
        let pid = interaction_core::ProviderId::new(format!("provider.mobile.{id}"));
        let desc = rt.get_provider(&pid).await.ok()?;
        let detail: Value = serde_json::from_str(desc.detail.as_deref()?).ok()?;
        detail.get("tested").cloned()
    }
    assert!(
        tested_of(&rt, &id_target).await.is_some(),
        "證據要記在真正執行的那一台（{id_target}）"
    );
    assert!(
        tested_of(&rt, &id_other).await.is_none(),
        "沒有執行過的那一台（{id_other}）不得被記上「已測試」"
    );

    for phone in phones {
        phone.abort();
    }
}

// ---------------------------------------------------------------------------
// BLE 掃描：可以指名目標手機
// ---------------------------------------------------------------------------

/// 兩台手機同時連線時：不指名＝誠實回 Err（列出可選的 id）；
/// 指名＝只有那一台收到 `ble.scan`，另一台完全沒被打擾。
#[tokio::test(flavor = "multi_thread")]
async fn ble_scan_can_name_its_target_phone() {
    let (_tmp, rt) = runtime().await;
    let (id_a, _token_a, ws_a) = pair(&rt).await;
    let (id_b, _token_b, ws_b) = pair(&rt).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, Value)>();
    let mut phones = Vec::new();
    for (id, ws) in [(id_a.clone(), ws_a), (id_b.clone(), ws_b)] {
        let tx = tx.clone();
        phones.push(tokio::spawn(async move {
            let mut ws = ws;
            while let Ok(Some(Ok(msg))) =
                tokio::time::timeout(Duration::from_secs(15), ws.next()).await
            {
                let Message::Text(text) = msg else { continue };
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if v["type"] == "ble.scan" {
                    let _ = tx.send((id.clone(), v.clone()));
                    send_json(
                        &mut ws,
                        json!({"type":"ble.result","id":v["id"],"peripherals":[]}),
                    )
                    .await;
                }
            }
        }));
    }
    drop(tx);

    // (1) 沒指名：兩台在線 → 誠實 Err，不偷偷挑一台。
    let err = rt
        .mobile_ble_scan(600, None)
        .await
        .expect_err("兩台在線又沒指名時必須拒絕");
    let text = err.to_string();
    assert!(
        text.contains(&id_a) && text.contains(&id_b) && text.contains("deviceId"),
        "錯誤要說出可選的手機與怎麼指定：{text}"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(400), rx.recv())
            .await
            .is_err(),
        "被拒絕的掃描不得送到任何一台手機"
    );

    // (2) 指名第二台：只有它收到。
    rt.mobile_ble_scan(600, Some(&id_b))
        .await
        .expect("targeted scan");
    let (delivered_to, _msg) = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("ble.scan delivered")
        .expect("ble.scan");
    assert_eq!(delivered_to, id_b, "掃描必須送到指名的那一台");
    assert!(
        tokio::time::timeout(Duration::from_millis(400), rx.recv())
            .await
            .is_err(),
        "另一台不得收到同一次掃描"
    );

    for phone in phones {
        phone.abort();
    }
}

// ---------------------------------------------------------------------------
// sensor.stop-uncertain 進統一收件匣（要人處理，不是純歷史）
// ---------------------------------------------------------------------------

/// 手機沒確認停止 → `sensor.stop-uncertain` 必須以「感測停止結果不確定：<手機名>」
/// 出現在收件匣的「待我決定」，而且徽章要數到它。
#[tokio::test(flavor = "multi_thread")]
async fn stop_uncertain_is_a_pending_decision_in_the_inbox() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;

    // 手機收到 stop-all 但完全不回覆（socket 還在）→ 結果未知。
    let silent = tokio::spawn(async move {
        while let Ok(Some(Ok(_))) = tokio::time::timeout(Duration::from_secs(8), ws.next()).await {}
    });
    let report = rt.stop_all_sensors("test").await.expect("stop all sensors");
    assert!(report.uncertain, "沒回覆就必須是 uncertain");

    let uncertain = iphone_sensor_events(&rt, interaction_core::EventType::SensorStopUncertain);
    assert!(
        uncertain.iter().any(|p| p["deviceId"] == json!(device_id)),
        "結果未知必須進事件流：{uncertain:?}"
    );

    let inbox = rt
        .activity_inbox(interaction_runtime::activity::ActivityInboxFilter {
            needs_decision: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    let items = inbox["items"].as_array().expect("items");
    let item = items
        .iter()
        .find(|i| i["status"] == json!("sensor.stop-uncertain"))
        .unwrap_or_else(|| panic!("stop-uncertain 必須進「待我決定」：{inbox}"));
    assert_eq!(item["kind"], json!("safety-event"), "{item}");
    assert_eq!(item["needsDecision"], json!(true), "{item}");
    let title = item["title"].as_str().unwrap_or_default();
    assert!(
        title.starts_with("感測停止結果不確定："),
        "標題要是人話：{title}"
    );
    assert!(
        title.contains("測試 iPhone"),
        "標題要點名是哪一台裝置：{title}"
    );
    assert!(
        !title.contains(&device_id),
        "一般模式標題不得外洩原始裝置 id：{title}"
    );
    assert!(
        inbox["pendingCount"].as_u64().unwrap_or(0) >= 1,
        "徽章要數到它：{inbox}"
    );
    silent.abort();
}

// ---------------------------------------------------------------------------
// 對抗審查修復回歸（mobile-server 061–066）
// ---------------------------------------------------------------------------

/// BLE 掃描是一次對外的無線探測：緊急停止生效時必須誠實拒絕，而且**什麼都不
/// 送到手機**（同檔風險更低的「測試這台手機」早就有這道閘）。
#[tokio::test(flavor = "multi_thread")]
async fn ble_scan_is_refused_while_the_emergency_stop_is_engaged() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;

    rt.emergency_stop("test", None).await.unwrap();
    let err = rt
        .mobile_ble_scan(800, None)
        .await
        .expect_err("緊急停止中不得掃描");
    let why = err.to_string();
    assert!(
        why.contains("emergency stop"),
        "拒絕理由要說是緊急停止：{why}"
    );

    // estop 自己會送 stop-all／character.present；不得有任何 ble.scan。
    let msgs = collect_json_within(&mut ws, Duration::from_millis(800), 8).await;
    assert!(
        !msgs.iter().any(|m| m["type"] == "ble.scan"),
        "緊急停止期間不得把 BLE 掃描送到手機：{msgs:?}"
    );
}

/// 兩台手機同時串流時，A 斷線會把**全域**的 `iphone.mic-level` 受器關掉——
/// 仍在串流的 B 不得因此從 `activeSensors` 無聲消失（消失＝宣稱它停了，
/// 但它的麥克風其實還在錄）。B 必須收到停止請求，並照實顯示「停止中／
/// 結果未知」直到它確認。
#[tokio::test(flavor = "multi_thread")]
async fn one_phone_disconnecting_never_silently_hides_another_streaming_phone() {
    let (_tmp, rt) = runtime().await;
    let (id_a, _token_a, mut ws_a) = pair(&rt).await;
    let (id_b, _token_b, mut ws_b) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();

    send_phone_status(&mut ws_a, true).await;
    send_phone_status(&mut ws_b, true).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let sensors = rt.status().await["activeSensors"].clone();
        let n = sensors.as_array().map(Vec::len).unwrap_or(0);
        if n >= 2 || std::time::Instant::now() > deadline {
            assert_eq!(n, 2, "兩台都在串流時都要看得見：{sensors}");
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // A 斷線（B 的麥克風還在錄）。全域受器被關掉之後，B 必須在 activeSensors
    // 上收斂成「停止中／結果未知」——而不是無聲消失。
    drop(ws_a);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let entry = loop {
        let sensors = rt.status().await["activeSensors"].clone();
        let list = sensors.as_array().cloned().unwrap_or_default();
        assert!(
            !list
                .iter()
                .any(|s| s["startedBy"] == json!(format!("iphone:{id_a}")))
                || std::time::Instant::now() < deadline,
            "A 應該要從 activeSensors 消失：{sensors}"
        );
        let b = list
            .iter()
            .find(|s| s["startedBy"] == json!(format!("iphone:{id_b}")))
            .cloned();
        match b {
            Some(entry) if entry["state"] != json!("active") => break entry,
            _ if std::time::Instant::now() > deadline => break Value::Null,
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    };
    let state = entry["state"].as_str().unwrap_or_default().to_string();
    let listed = rt.status().await["activeSensors"].clone();
    assert!(
        state == "stopping" || state == "stop-unknown",
        "A 斷線不得讓仍在串流的 B 從 activeSensors 無聲消失：\
         B 必須誠實標成停止中／結果未知（現在的清單：{listed}）"
    );

    // B 真的收到停止請求（不是只在畫面上改字）。
    let msgs = collect_json_within(&mut ws_b, Duration::from_secs(3), 4).await;
    let stop_all = msgs
        .iter()
        .find(|m| m["type"] == "stop-all")
        .unwrap_or_else(|| panic!("共用受器被關掉時 B 也要收到停止請求：{msgs:?}"));
    assert_eq!(stop_all["sensors"], json!(true), "{stop_all}");
    assert_eq!(
        stop_all["reason"],
        json!("user"),
        "這不是緊急停止：手機不得顯示成緊急停止：{stop_all}"
    );
    let audit = last_audit(&rt, "mobile.sensors-stop-shared-receptor-off")
        .unwrap_or_else(|| panic!("要留 audit 說明為什麼 B 也被要求停止"));
    assert_eq!(audit["detail"]["triggeredBy"], json!(id_a), "{audit}");
}

/// 撤銷一台「正在串流、而且有在途動作」的手機：撤銷路徑自己就要把收尾做完——
/// 補一則 `sensor.stopped`、在途 act 立刻收場（不必等滿 4 秒逾時）。
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_streaming_phone_ends_its_sensing_and_inflight_acts_at_once() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;

    // 手機收得到 act 但永遠不回覆（在途）。
    let silent = tokio::spawn(async move {
        while let Ok(Some(Ok(_))) = tokio::time::timeout(Duration::from_secs(8), ws.next()).await {}
    });
    let actuator = enabled_actuator(&rt, "iphone.haptic").await;
    let rt_act = rt.clone();
    let acting = tokio::spawn(async move {
        actuator
            .execute(test_action("iphone.haptic"))
            .await
            .expect("receipt")
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if rt_act.mobile_status().await.unwrap()["pendingActs"] == json!(1) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "act 應該已經在途了");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let started = std::time::Instant::now();
    rt.mobile_revoke(&device_id).await.unwrap();
    let receipt = tokio::time::timeout(Duration::from_secs(2), acting)
        .await
        .expect("撤銷之後在途 act 必須立刻收場，不能等滿 4 秒逾時")
        .expect("join");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "撤銷不得讓呼叫端空等到 ACT_TIMEOUT：{:?}",
        started.elapsed()
    );
    assert_ne!(
        receipt.current_status,
        interaction_core::ActionStatus::Acknowledged,
        "手機沒回覆就不得記成 acknowledged：{receipt:?}"
    );
    assert_eq!(
        rt.mobile_status().await.unwrap()["pendingActs"],
        json!(0),
        "撤銷後不得留下在途 act"
    );

    let stopped = iphone_sensor_events(&rt, interaction_core::EventType::SensorStopped);
    assert!(
        stopped
            .iter()
            .any(|p| p["deviceId"] == json!(device_id) && p["reason"] == json!("revoked")),
        "撤銷一台正在串流的手機必須補一則 sensor.stopped：{stopped:?}"
    );
    silent.abort();
}

/// iOS 把語意事件自己的時間戳放在訊息**頂層**的 `at`（Protocol.swift），
/// 不在 `facts` 裡。manifest 宣告 `provides: ["event","at"]` 就必須真的收得到，
/// 否則對外宣稱了一個永遠不會出現的欄位。
#[tokio::test(flavor = "multi_thread")]
async fn a_top_level_at_on_a_motion_observation_reaches_the_facts() {
    let (_tmp, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;
    // 與 iOS ProtocolTests.swift 釘住的 wire 形狀一致。
    send_json(
        &mut ws,
        json!({
            "type": "observation",
            "receptor": "iphone.motion",
            "facts": {"event": "lifted"},
            "at": "2026-08-28T10:00:00.000Z",
        }),
    )
    .await;

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let facts = loop {
        let found = rt
            .events
            .recent(200)
            .into_iter()
            .filter(|e| e.event_type == interaction_core::EventType::ReceptorObservation)
            .find(|e| e.payload["receptorId"] == json!("iphone.motion"))
            .map(|e| e.payload["facts"].clone());
        if let Some(facts) = found {
            break facts;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "motion 觀察應該要進事件流"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(facts["event"], json!("lifted"), "{facts}");
    assert_eq!(
        facts["at"],
        json!("2026-08-28T10:00:00.000Z"),
        "manifest 宣告收得到 `at`，就不能把手機送來的時間戳丟掉：{facts}"
    );
}

/// 認證失敗（未知裝置／錯 token／撤銷後仍在重連）必須留 audit ＋計數：
/// 撤銷有沒有生效、區網上有沒有人在猜 deviceId／token，使用者要看得見。
/// token 本身永遠不進 audit。
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_auth_is_audited_and_counted() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, _ws) = pair(&rt).await;
    let status = rt.mobile_status().await.unwrap();
    let port = status["port"].as_u64().unwrap() as u16;
    let fp = status["fingerprint"].as_str().unwrap().to_string();

    let mut ws = connect(port, &fp).await;
    send_json(
        &mut ws,
        json!({"type":"auth","deviceId":device_id,"token":"not-the-real-token"}),
    )
    .await;
    assert_eq!(recv_json(&mut ws).await["type"], "auth-fail");

    let audit = last_audit(&rt, "mobile.auth-failed")
        .unwrap_or_else(|| panic!("認證失敗必須留 audit（撤銷後的重連才看得見）"));
    assert_eq!(audit["detail"]["deviceId"], json!(device_id), "{audit}");
    assert_eq!(audit["detail"]["knownDevice"], json!(true), "{audit}");
    assert!(
        audit["detail"]["peer"].as_str().unwrap_or_default().len() > 3,
        "要記下是誰在敲門：{audit}"
    );
    assert!(
        !serde_json::to_string(&audit)
            .unwrap_or_default()
            .contains("not-the-real-token"),
        "audit 不得記下對方送來的 token：{audit}"
    );
    assert_eq!(
        rt.mobile_status().await.unwrap()["heartbeat"]["failedAuths"],
        json!(1),
        "status 要數得出來"
    );
}

// ---------------------------------------------------------------------------
// v0.5.1 對抗審查第三輪（0c845e0）：safety-invariants-056、mobile-server-059／060
// ---------------------------------------------------------------------------

/// safety-invariants-056：使用者在「連接與權限」按下 `iphone.mic-level` 的
/// 「停用」（＝PATCH /v1/receptors／Tauri set_receptor_enabled）之後，手機
/// **不會自己停**——桌面必須 (a) 請它停止感測，(b) 在它確認之前不得讓它從
/// `status.activeSensors` 消失（消失＝宣稱已停＝感測靜默）。
#[tokio::test(flavor = "multi_thread")]
async fn disabling_the_shared_mic_receptor_stops_the_phone_and_keeps_it_visible() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;

    // 模擬 iPhone：收得到 stop-all，但先不回覆（結果未知＝可能仍在錄）。
    let (tx, mut saw_stop_all) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let phone = tokio::spawn(async move {
        while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(8), ws.next()).await
        {
            let Message::Text(text) = msg else { continue };
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v["type"] == "stop-all" {
                let _ = tx.send(v);
            }
        }
    });

    // 這一行就是使用者按下「停用」時 runtime 唯一會發生的事。
    rt.registry.set_receptor_enabled(&mic, false).await.unwrap();

    // (a) 手機必須收到「連感測一起停」的請求，而且說明是使用者發起的。
    let stop_all = tokio::time::timeout(Duration::from_secs(3), saw_stop_all.recv())
        .await
        .expect("停用共享的高風險受器必須請手機停止擷取")
        .expect("stop-all payload");
    assert_eq!(stop_all["sensors"], json!(true), "{stop_all}");
    assert_eq!(stop_all["reason"], json!("user"), "{stop_all}");

    // (b) 手機還沒確認 → 仍要出現在 activeSensors，並誠實標「停止中／未知」。
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let entry = loop {
        let status = rt.status().await;
        let found = status["activeSensors"]
            .as_array()
            .and_then(|l| l.iter().find(|s| s["kind"] == "iphone.mic-level").cloned());
        match found {
            Some(entry) => break entry,
            None if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            None => panic!(
                "仍在擷取的 iPhone 不得從 activeSensors 無聲消失：{}",
                rt.status().await["activeSensors"]
            ),
        }
    };
    assert!(
        matches!(
            entry["state"].as_str(),
            Some("stopping") | Some("stop-unknown")
        ),
        "受器停用後手機未確認，狀態要說「停止中／結果未知」：{entry}"
    );
    assert!(
        entry["startedBy"]
            .as_str()
            .unwrap_or_default()
            .contains(&device_id),
        "要指得出是哪一台手機：{entry}"
    );

    // audit 留痕：使用者事後查得到「桌面確實請手機停了」（逐台結果，
    // 有界等待之後才落地，所以這裡也要有界輪詢）。
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let audit = loop {
        match last_audit(&rt, "mobile.sensors-stop-shared-receptor-off") {
            Some(audit) => break audit,
            None if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            None => panic!("停用共享受器要留 audit"),
        }
    };
    assert_eq!(audit["detail"]["devices"][0]["deviceId"], json!(device_id));
    assert_eq!(
        audit["detail"]["receptor"],
        json!("iphone.mic-level"),
        "{audit}"
    );
    phone.abort();
}

/// mobile-server-059：緊急停止時「有連線但一則都送不出去」——沒有任何一台
/// iPhone 收到 stop-all 時，六個 mobile 動器都不得回 Ok，`stoppedActuators`
/// 也不得把它們算成已停止（誠實階梯：排進佇列／送不出去 ≠ 已停止）。
#[tokio::test(flavor = "multi_thread")]
async fn estop_never_counts_mobile_actuators_when_no_phone_received_stop_all() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, _ws) = pair(&rt).await;
    // 連線還在（any_connected() 為真），但出站訊息一則都排不進去。
    rt.mobile.inject_outbound_failure(true);

    let total_actuators = rt.registry.all_actuator_instances().await.len();
    let payload = rt.emergency_stop("user", None).await.unwrap();

    // 感測面誠實：這台手機是 unreachable。
    assert_eq!(
        payload["sensors"]["devices"][0]["deviceId"],
        json!(device_id),
        "{payload}"
    );
    assert_eq!(
        payload["sensors"]["devices"][0]["outcome"],
        json!("unreachable"),
        "{payload}"
    );
    // 動器面不得自相矛盾：六個 iphone.* 動器一個都不能被算進去。
    let stopped = payload["stoppedActuators"].as_u64().unwrap();
    assert!(
        stopped as usize <= total_actuators - MOBILE_ACTUATORS.len(),
        "沒送出任何 stop-all 卻把 mobile 動器算成已停止：stoppedActuators={stopped}／全部 {total_actuators}"
    );

    // 去重窗不得替失敗的那一則「代簽」：同一輪內再問一次仍必須誠實 Err。
    for (id, _, _) in MOBILE_ACTUATORS.iter() {
        let id: &str = id;
        let actuator = rt
            .registry
            .actuator_any(&interaction_core::ActuatorId::new(id))
            .await
            .unwrap_or_else(|e| panic!("{id} registered: {e}"));
        assert!(
            actuator.emergency_stop().await.is_err(),
            "{id}: 沒有任何手機收到 stop-all 時不得回 Ok"
        );
    }
}

/// mobile-server-060：配對清單讀不到／被截斷時，不得靜默演成「還沒有配對的
/// iPhone」。壞掉的檔案要留證據、要有 audit，status 要說「未知」。
#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_paired_device_list_is_reported_as_unknown_not_as_none() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime_at(dir.path()).await;
    let (device_id, _token, ws) = pair(&rt).await;
    drop(ws);
    let path = dir.path().join("state").join("mobile-devices.json");
    let text = std::fs::read_to_string(&path).expect("device list written");
    assert!(text.contains(&device_id), "{text}");

    // 模擬 ENOSPC／崩潰：檔案被截成半截（非原子寫入的典型結果）。
    std::fs::write(&path, &text[..text.len() / 2]).unwrap();

    let rt2 = runtime_at(dir.path()).await;
    rt2.mobile_ensure_started().await.expect("mobile server");
    let status = rt2.mobile_status().await.unwrap();
    assert_eq!(
        status["devices"].as_array().map(|d| d.len()),
        Some(0),
        "{status}"
    );
    assert_eq!(
        status["devicesUnknown"],
        json!(true),
        "讀不到配對清單不得假裝成「還沒有配對的 iPhone」：{status}"
    );
    assert!(
        !status["devicesError"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "要說得出讀不到的原因：{status}"
    );
    let audit = last_audit(&rt2, "mobile.devices-load-failed")
        .unwrap_or_else(|| panic!("配對清單讀不到必須留 audit"));
    assert!(
        audit["detail"]["error"].as_str().unwrap_or_default().len() > 3,
        "{audit}"
    );
    // 壞掉的檔案要留著（人可以救回來），不是被就地覆蓋。
    let quarantined = dir.path().join("state").join("mobile-devices.json.corrupt");
    assert!(quarantined.exists(), "壞掉的清單要保留成證據：{audit}");
    assert_eq!(
        std::fs::read_to_string(&quarantined).unwrap().len(),
        text.len() / 2
    );
}

/// AIP 相容性（`docs/aip/README.md` §9.1）：**沒有**送 `capability` 的舊 App
/// 永遠不會收到任何 `aip` frame——它只看得到 v1 線協定的訊息（這裡是
/// `stop-all`）。模擬 iPhone（fixture），不是真機驗收。
#[tokio::test]
async fn a_legacy_phone_that_never_negotiates_receives_no_aip_frames() {
    let (_g, rt) = runtime().await;
    let (_device_id, _token, mut ws) = pair(&rt).await;
    send_json(
        &mut ws,
        json!({"type":"status","sensors":{"micLevel":false}}),
    )
    .await;

    // Runtime 真相事實（緊急停止）：Session 廣播不得外洩給未協商的 App。
    rt.emergency_stop("user", None).await.expect("estop");

    let mut seen = Vec::new();
    for _ in 0..4 {
        match tokio::time::timeout(Duration::from_millis(800), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: Value = serde_json::from_str(&text).expect("json");
                seen.push(value["type"].as_str().unwrap_or_default().to_string());
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(
        !seen.iter().any(|kind| kind == "aip"),
        "未協商的舊 App 不得收到 aip frame：{seen:?}"
    );
    assert!(
        seen.iter().any(|kind| kind == "stop-all" || kind == "act"),
        "v1 線協定的既有路徑必須照舊：{seen:?}"
    );
}

// ---------------------------------------------------------------------------
// v0.6.0：mobile provider 自己宣告能力語意（核心不再認得 `iphone.*` 字面值）
// ---------------------------------------------------------------------------

/// 呈現面、高風險受器與人話種類名由這個 provider 自己宣告，而且必須跟它實際
/// 註冊的能力表一致——宣告漂掉的話，runtime 核心會把手機的角色呈現當成一般
/// 動作再投影一次，或漏掉「還可能在錄音」的誠實提示。
#[test]
fn the_mobile_provider_declares_its_own_presentation_surface_and_high_risk_receptors() {
    use interaction_runtime::mobile::{mobile_capability_declaration, MOBILE_RECEPTOR_SPECS};
    let decl = mobile_capability_declaration();

    assert_eq!(decl.class_label.as_deref(), Some("iPhone"));
    assert!(decl
        .presentation_surfaces
        .iter()
        .any(|s| s.matches("iphone.character")));
    for (id, _, _) in MOBILE_ACTUATORS.iter() {
        if *id == "iphone.character" {
            continue;
        }
        assert!(
            !decl.presentation_surfaces.iter().any(|s| s.matches(id)),
            "{id} 不是呈現面：它的收據必須照常投影成 action.*"
        );
    }
    let expected_high_risk: Vec<String> = MOBILE_RECEPTOR_SPECS
        .iter()
        .filter(|s| s.high_risk)
        .map(|s| s.id.to_string())
        .collect();
    assert_eq!(decl.high_risk_receptors, expected_high_risk);
    assert!(!expected_high_risk.is_empty());
    let expected_receptors: Vec<String> = MOBILE_RECEPTOR_SPECS
        .iter()
        .map(|s| s.id.to_string())
        .collect();
    assert_eq!(decl.receptors, expected_receptors);
}

/// 每一台手機的停止結果自己說得出「還可能在擷取的是哪個受器」，所以
/// sensors.rs 不需要知道任何 `iphone.*` 字面值。
#[test]
fn a_mobile_stop_outcome_names_the_receptors_its_provider_declared() {
    use interaction_runtime::mobile::{MobileStopOutcome, StopOutcome};
    use interaction_runtime::sensors::SensorStopOutcome;
    let unknown = MobileStopOutcome {
        device_id: "iphone-a1b2c3d4".into(),
        name: "測試 iPhone".into(),
        outcome: StopOutcome::Unknown,
        waited_ms: 3000,
        via: None,
    };
    assert_eq!(unknown.source_id(), "iphone-a1b2c3d4");
    assert_eq!(unknown.sensor_ids(), vec!["iphone.mic-level".to_string()]);
    assert_eq!(unknown.outcome_label(), "unknown");
    assert_eq!(unknown.waited_ms(), 3000);
    assert!(!unknown.confirmed_stopped());

    let stopped = MobileStopOutcome {
        outcome: StopOutcome::Stopped,
        ..unknown.clone()
    };
    assert!(stopped.confirmed_stopped());
}

/// 宣告有被 runtime 接上：開機（沒有配對任何裝置、伺服器也還沒起來）之後，
/// 核心就已經知道手機的角色呈現面與高風險受器。核心對能力 id 的理解不得
/// 依賴某個 provider 剛好在線上。
#[tokio::test]
async fn the_mobile_declaration_is_wired_into_the_runtime_at_startup() {
    let (_g, rt) = runtime().await;
    let decls = rt.capability_declarations();
    assert!(
        decls.is_presentation_surface("iphone.character"),
        "手機的角色呈現面要在開機時就宣告好：{:?}",
        decls.declaration_ids()
    );
    assert!(!decls.is_presentation_surface("iphone.haptic"));
    assert_eq!(
        decls.class_label_of_receptor("iphone.mic-level").as_deref(),
        Some("iPhone")
    );
    assert!(decls
        .high_risk_receptors()
        .iter()
        .any(|r| r == "iphone.mic-level"));
}

/// 撤銷**一支**手機不得動到 `provider.mobile` 這一族的能力宣告。
///
/// 這是刻意的設計，不是漏網：宣告說的是「`iphone.mic-level` 是高風險受器」這
/// 件事實，跟「現在有沒有一支手機在線上」無關。若撤銷最後一支手機就把宣告刪
/// 掉，`stop_all_sensors` 從此不知道那些受器是高風險，停止結果未知時就不會再
/// 誠實補「可能還在擷取」——撤銷反而讓系統變得更不誠實。
/// 移除宣告是 `retract_provider_capabilities` 的事（整族不再存在時才做）。
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_never_retracts_the_provider_family_declaration() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, ws) = pair(&rt).await;
    drop(ws);

    rt.mobile_revoke(&device_id).await.unwrap();
    assert_eq!(
        rt.mobile_status().await.unwrap()["devices"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "前提：最後一支手機已經被撤銷"
    );

    let decls = rt.capability_declarations();
    assert!(
        decls
            .declaration_ids()
            .iter()
            .any(|id| id == interaction_runtime::mobile::MOBILE_PROVIDER_DECLARATION_ID),
        "撤銷裝置不得刪掉整族宣告：{:?}",
        decls.declaration_ids()
    );
    assert!(
        decls
            .high_risk_receptors()
            .iter()
            .any(|r| r == "iphone.mic-level"),
        "高風險受器的語意不隨裝置消失"
    );
    assert!(decls.is_presentation_surface("iphone.character"));
    assert_eq!(
        decls.class_label_of_receptor("iphone.mic-level").as_deref(),
        Some("iPhone")
    );
}

// ---------------------------------------------------------------------------
// 被新連線取代（superseded）：對抗審查 pairing-migration-003／reconnect-recovery-047
// ---------------------------------------------------------------------------

/// 同一台手機用同一組 token 再開一條連線（不關掉舊的）。
/// iOS 的重連退避上限 15 s 遠小於桌面 45 s 的 idle timeout，所以「舊 handler 還沒
/// 逾時、新連線已經上來」是**常態**而不是邊界。
async fn reconnect_same_device(rt: &Runtime, device_id: &str, token: &str) -> Ws {
    let status = rt.mobile_status().await.unwrap();
    let port = status["port"].as_u64().unwrap() as u16;
    let fp = status["fingerprint"].as_str().unwrap().to_string();
    let mut ws = connect(port, &fp).await;
    send_json(
        &mut ws,
        json!({"type":"auth","deviceId": device_id, "token": token}),
    )
    .await;
    assert_eq!(recv_json(&mut ws).await["type"], "auth-ok");
    ws
}

/// 不變量「手機斷線 → 高風險受器由桌面端強制 disabled，重連不自動恢復」在
/// **superseded**（舊連線被新連線取代）這條收尾路徑上一樣要成立。
///
/// 舊行為：舊 handler 判成 superseded 就整段跳過強制停用，於是只要在 45 s 的
/// idle timeout 之前重連，`iphone.mic-level` 就一直是 enabled——人類不必重新啟用，
/// 手機單邊打開麥克風即可繼續推 mic-level；而且先前的 `sensor.started` 永遠等不到
/// 對應的 `sensor.stopped`（新連線的 `mic_since` 是 None，該手機已從
/// `status.activeSensors` 消失）。
#[tokio::test(flavor = "multi_thread")]
async fn a_superseded_connection_still_forces_the_high_risk_receptor_off() {
    let (_tmp, rt) = runtime().await;
    let (device_id, token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");

    // 人類啟用 → 手機自報串流中 → 這台手機真的在 activeSensors 裡。
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;
    let started_before =
        iphone_sensor_events(&rt, interaction_core::EventType::SensorStarted).len();
    assert!(started_before > 0, "前提：sensor.started 已經發出去了");

    // 舊 socket 完全不關：新連線直接取代它。
    let _ws2 = reconnect_same_device(&rt, &device_id, &token).await;

    assert!(
        rt.registry.receptor(&mic).await.is_err(),
        "被新連線取代＝那條有效連線消失，高風險受器必須強制停用（重連不自動恢復）"
    );
    let stopped = iphone_sensor_events(&rt, interaction_core::EventType::SensorStopped);
    assert!(
        stopped.iter().any(|p| p["reason"] == json!("superseded")),
        "先前的 sensor.started 必須有對應的結束事件：{stopped:?}"
    );
    let audit = last_audit(&rt, "mobile.high-risk-receptor-disabled")
        .expect("強制停用要留稽核（誰、為什麼）");
    assert_eq!(audit["detail"]["reason"], json!("superseded"), "{audit}");

    // 感測不靜默：受器停用後 status 也不得再顯示這台手機在串流。
    wait_for_iphone_mic_sensor(&rt, false).await;
    drop(ws);
}

/// superseded 的另一半：舊連線被靜默關閉（**不得**誤報成撤銷），而且 provider
/// 不轉 Disconnected——手機其實還在線上，只是換了一條 socket。
/// 這條分支在 v0.6.0 恢復矩陣 §2 第 11 條被列為「受保護但無測試」。
#[tokio::test(flavor = "multi_thread")]
async fn superseding_a_live_connection_is_silent_and_keeps_the_provider_available() {
    let (_tmp, rt) = runtime().await;
    let (device_id, token, mut ws) = pair(&rt).await;
    let pid = interaction_core::ProviderId::new(format!("provider.mobile.{device_id}"));
    assert_eq!(
        rt.get_provider(&pid).await.unwrap().state,
        interaction_core::ProviderState::Available
    );

    let ws2 = reconnect_same_device(&rt, &device_id, &token).await;

    // 舊連線：被關掉，但不得收到 `auth-fail`（那是撤銷的意思，iOS 會停止重連）。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, ws.next()).await {
            Err(_) => panic!("舊連線沒有被取代它的新連線關掉"),
            Ok(None) => break,
            Ok(Some(Err(_))) => break,
            Ok(Some(Ok(Message::Text(text)))) => {
                let frame: Value = serde_json::from_str(&text).expect("json");
                assert_ne!(
                    frame["type"],
                    json!("auth-fail"),
                    "被取代不是撤銷：不得回 auth-fail（{frame}）"
                );
            }
            Ok(Some(Ok(_))) => continue,
        }
    }

    // 新連線還活著；provider 不得被舊 handler 的收尾拖成 Disconnected。
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(rt.mobile.any_connected().await, "新連線必須還在");
    assert_eq!(
        rt.get_provider(&pid).await.unwrap().state,
        interaction_core::ProviderState::Available,
        "同一台手機換 socket 不是斷線"
    );
    let status = rt.mobile_status().await.unwrap();
    assert_eq!(status["devices"][0]["connected"], json!(true), "{status}");
    drop(ws2);
}

/// 斷線收尾（reconnect-recovery-044）：`Presence::Reconnecting` 只給**已經協商過**的成員。
/// 從沒送過 AIP `capability` 的舊版 App 不是 session 成員，它斷線時不得憑空長出一個成員
/// （「有人正在重新連線」是一句需要證據的話）。已協商成員那一半在
/// `character_session_loop.rs::a_disconnected_member_is_reconnecting_before_it_is_offline`。
#[tokio::test]
async fn a_legacy_app_that_never_negotiated_creates_no_session_member_on_disconnect() {
    let (_dir, rt) = runtime().await;
    let (device_id, _token, ws) = pair(&rt).await;
    let members_before = rt
        .character_session_diagnostics_value()
        .expect("diagnostics")["members"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);

    drop(ws);
    tokio::time::sleep(Duration::from_millis(400)).await;

    let diagnostics = rt
        .character_session_diagnostics_value()
        .expect("diagnostics");
    let members = diagnostics["members"].as_array().expect("members");
    assert_eq!(
        members.len(),
        members_before,
        "沒協商過的裝置斷線不得改變成員名單：{diagnostics}"
    );
    assert!(
        !members.iter().any(|m| m["party"]["id"] == json!(device_id)),
        "沒協商過的裝置本來就不是成員：{diagnostics}"
    );
    rt.shutdown().await;
}

/// X2：**通用**的 provider 撤銷（`revoke_provider`，不是 mobile 專屬入口）也必須
/// 走同一條停止感測路徑：指名這一台請它停止擷取、有界等待、結果進稽核，
/// 並且把連線放掉。在此之前這條路只翻了受器旗標，能不能停到全靠背景 watcher
/// 撞上事件——那是競態，不是保證。
#[tokio::test(flavor = "multi_thread")]
async fn generic_provider_revoke_stops_a_streaming_phone_and_drops_its_connection() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    send_phone_status(&mut ws, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;

    let (phone, mut rx) = spawn_phone_confirming_stop_all(ws);
    let pid = interaction_runtime::mobile::mobile_provider_id(&device_id);
    let started = std::time::Instant::now();
    rt.revoke_provider(&pid).await.expect("generic revoke");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "撤銷的等待必須有界：{:?}",
        started.elapsed()
    );

    // (a) 手機真的收到「停止感測」，而且不是被說成緊急停止。
    let stop_all = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("stop-all reached the phone")
        .expect("stop-all payload");
    assert_eq!(stop_all["sensors"], json!(true), "{stop_all}");
    assert_eq!(
        stop_all["reason"], "user",
        "撤銷不是緊急停止，手機不得顯示緊急停止那一句：{stop_all}"
    );

    // (b) 逐台結果進 provider.revoked 的稽核（誠實：stopped／unknown／unreachable）。
    let audit = last_audit(&rt, "provider.revoked").expect("provider.revoked audit");
    let outcome = audit["detail"]["sensorStop"]["reports"][0]["outcome"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        ["stopped", "unknown", "unreachable"].contains(&outcome.as_str()),
        "停止結果要誠實逐台列出：{audit}"
    );
    assert_eq!(
        audit["detail"]["sensorStop"]["target"],
        json!(device_id),
        "只停這一台：{audit}"
    );

    // (c) 連線被放掉（撤銷不是「不再派工」而已）。
    assert!(
        !rt.mobile.any_connected().await,
        "撤銷之後不得留著一條還在串流的連線"
    );
    assert!(
        audit["detail"]["sensorStop"]["released"]["connectionClosed"] == json!(true),
        "稽核要說得出連線被放掉了：{audit}"
    );
    // (d) 高風險受器強制停用：重連不自動恢復。
    assert!(rt.registry.receptor(&mic).await.is_err());
    phone.abort();
}

// ---------------------------------------------------------------------------
// AIP 出站通道登記表（型別抹除）：iPhone 那一族
// ---------------------------------------------------------------------------

/// 一則最小合規的 capability envelope（讓這台手機成為 session 成員）。
fn iphone_capability_envelope(device_id: &str) -> Value {
    json!({
        "specVersion": "aip/1.0",
        "messageId": format!("fx-cap-{}", chrono::Utc::now().timestamp_millis()),
        "messageType": "capability",
        "name": "character.session.capability",
        "source": {"kind": "device", "id": device_id},
        "sessionId": "session.home",
        "occurredAt": chrono::Utc::now().to_rfc3339(),
        "payload": {
            "specVersions": ["aip/1.0"],
            "role": "remote-renderer",
            "profiles": ["character-session"],
            "syncClasses": ["semantic"],
            "intents": ["idle"],
            "inputs": ["character.interaction.touch"],
            "limits": {"maxMessageBytes": 65536},
        },
    })
}

fn member_identity_strength(rt: &Runtime, device_id: &str) -> Option<Value> {
    rt.character_session_diagnostics_value()
        .expect("diagnostics")["members"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|m| m["party"]["id"] == device_id)
        .and_then(|m| m.get("identityStrength").cloned())
}

/// 已認證的 iPhone 必須登記成一條型別抹除的出站通道，而且說得出自己的身分
/// 強度（host 端 sha256(token) 逐次驗證＝這一族最強的一種）。斷線就要移除：
/// 留著等於之後每一則廣播都往一條已經沒有的連線上送。
#[tokio::test(flavor = "multi_thread")]
async fn a_paired_iphone_registers_an_outbound_channel_with_its_identity_strength() {
    let (_tmp, rt) = runtime().await;
    let (device_id, _token, mut ws) = pair(&rt).await;
    assert!(
        rt.device_outbound_ids().contains(&device_id),
        "已認證的手機必須在出站表上：{:?}",
        rt.device_outbound_ids()
    );

    send_json(
        &mut ws,
        json!({"type":"aip","envelope": iphone_capability_envelope(&device_id)}),
    )
    .await;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while member_identity_strength(&rt, &device_id).is_none()
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        member_identity_strength(&rt, &device_id),
        Some(json!("paired-token")),
        "已配對 iPhone 的身分強度＝配對時交換的 per-device token"
    );

    let _ = ws.close(None).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while rt.device_outbound_ids().contains(&device_id) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !rt.device_outbound_ids().contains(&device_id),
        "斷線之後出站表不得還留著它：{:?}",
        rt.device_outbound_ids()
    );
}

// ---------------------------------------------------------------------------
// 家族共用能力：通用 provider 停用／撤銷只能停「這一台」
// （`iphone.*` 這一組受器／動器是所有已配對 iPhone 共用的同一份 registry 旗標，
//  每台手機的 descriptor 都塞同一組字面值。翻旗標＝把還沒被停用的手機一起關掉。）
// ---------------------------------------------------------------------------

/// 兩台手機都在串流時，用**通用**入口撤銷其中一台：另一台的共用受器／動器
/// 旗標不得被翻掉，它也不得從 `activeSensors` 消失（感測不靜默）。
#[tokio::test(flavor = "multi_thread")]
async fn generic_revoke_of_one_phone_keeps_the_shared_capabilities_of_the_other() {
    let (_tmp, rt) = runtime().await;
    let (id_a, _token_a, mut ws_a) = pair(&rt).await;
    let (id_b, _token_b, mut ws_b) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    let _haptic = enabled_actuator(&rt, "iphone.haptic").await;
    let _notify = enabled_actuator(&rt, "iphone.notify").await;
    let _character = enabled_actuator(&rt, "iphone.character").await;

    send_phone_status(&mut ws_a, true).await;
    send_phone_status(&mut ws_b, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;

    let (phone_a, mut rx_a) = spawn_phone_confirming_stop_all(ws_a);
    let pid_a = interaction_runtime::mobile::mobile_provider_id(&id_a);
    rt.revoke_provider(&pid_a).await.expect("generic revoke");

    // (a) X2 還在：被撤銷的那一台真的被指名要求停止。
    let stop_all = tokio::time::timeout(Duration::from_secs(3), rx_a.recv())
        .await
        .expect("stop-all reached phone A")
        .expect("stop-all payload");
    assert_eq!(stop_all["sensors"], json!(true), "{stop_all}");

    // (b) 共用受器旗標不得被翻掉：B 從來沒有被停用。
    assert!(
        rt.registry.receptor(&mic).await.is_ok(),
        "撤銷 A 不得關掉全 iPhone 共用的 iphone.mic-level（B 仍在串流）"
    );
    // (c) 共用動器同理。
    for aid in ["iphone.haptic", "iphone.notify", "iphone.character"] {
        assert!(
            rt.registry
                .actuator(&interaction_core::ActuatorId::new(aid))
                .await
                .is_ok(),
            "撤銷 A 不得關掉全 iPhone 共用的 {aid}"
        );
    }

    // (d) B 仍然在 activeSensors（消失＝宣稱它停了）。
    let status = rt.status().await;
    let listed = status["activeSensors"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .any(|s| s["startedBy"] == json!(format!("iphone:{id_b}")));
    assert!(
        listed,
        "B 仍在串流卻從 activeSensors 消失了：{}",
        status["activeSensors"]
    );

    // (e) 稽核說得出「為什麼沒關」，並指名還有誰宣告同一份能力。
    let kept = last_audit(&rt, "provider.capabilities-shared-kept")
        .expect("provider.capabilities-shared-kept audit");
    assert_eq!(
        kept["detail"]["providerId"],
        json!(pid_a.as_str()),
        "{kept}"
    );
    let shared_with = kept["detail"]["sharedWith"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        shared_with.contains(&json!(interaction_runtime::mobile::mobile_provider_id(
            &id_b
        )
        .as_str())),
        "稽核要指名共用這份能力的其他 provider：{kept}"
    );
    let receptors = kept["detail"]["receptors"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        receptors.contains(&json!("iphone.mic-level")),
        "稽核要列出被保留的受器：{kept}"
    );
    phone_a.abort();
}

/// 同一件事的另一條通用入口：`providers transition <id> --state disabled`。
#[tokio::test(flavor = "multi_thread")]
async fn generic_disable_of_one_phone_keeps_the_shared_capabilities_of_the_other() {
    let (_tmp, rt) = runtime().await;
    let (id_a, _token_a, ws_a) = pair(&rt).await;
    let (id_b, _token_b, mut ws_b) = pair(&rt).await;
    let mic = interaction_core::ReceptorId::new("iphone.mic-level");
    rt.registry.set_receptor_enabled(&mic, true).await.unwrap();
    let _haptic = enabled_actuator(&rt, "iphone.haptic").await;

    send_phone_status(&mut ws_b, true).await;
    wait_for_iphone_mic_sensor(&rt, true).await;
    let (phone_a, _rx_a) = spawn_phone_confirming_stop_all(ws_a);

    let pid_a = interaction_runtime::mobile::mobile_provider_id(&id_a);
    rt.transition_provider(&pid_a, interaction_core::ProviderState::Disabled)
        .await
        .expect("generic transition");

    assert!(
        rt.registry.receptor(&mic).await.is_ok(),
        "停用 A 不得關掉全 iPhone 共用的 iphone.mic-level（B 仍在串流）"
    );
    assert!(
        rt.registry
            .actuator(&interaction_core::ActuatorId::new("iphone.haptic"))
            .await
            .is_ok(),
        "停用 A 不得關掉全 iPhone 共用的 iphone.haptic"
    );
    let status = rt.status().await;
    let listed = status["activeSensors"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .any(|s| s["startedBy"] == json!(format!("iphone:{id_b}")));
    assert!(
        listed,
        "B 仍在串流卻從 activeSensors 消失了：{}",
        status["activeSensors"]
    );
    assert!(
        last_audit(&rt, "provider.capabilities-shared-kept").is_some(),
        "沒有關旗標這件事必須留稽核"
    );
    phone_a.abort();
}
