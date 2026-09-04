//! 【模擬 iPhone（fixture）】——給瀏覽器 E2E 用的程序外假手機。
//!
//! 這**不是**真機驗收：它只是把 `crates/interaction-runtime/tests/mobile_loop.rs`
//! 裡那個程序內模擬手機（釘指紋的 TLS 驗證器＋HMAC 配對＋status 回報）搬成一支
//! 可執行檔，讓 Playwright 可以在真 daemon 上重現「iPhone 已連線／已斷線／
//! 權限被拒／停止感測沒回應」這些狀態。任何用到它的截圖與文件都必須標示
//! 「模擬 iPhone（fixture）」，不得寫成 iPhone 真機驗收。
//!
//! 用法：
//! ```text
//! fake_iphone --port P --fingerprint FP --code C \
//!   [--name '模擬 iPhone（fixture）'] [--model iPhone12,1] [--auto-ack-stop-all]
//! ```
//! 啟動後先完成配對，然後在 stdout 印出一行 `{"deviceId":…,"deviceToken":…}`，
//! 之後每收到一則值得觀察的訊息就印一行 `{"event":…}`（JSON Lines）。
//! stdin 讀 JSON 指令，一行一則：
//! `{"op":"status","micLevel":true,"permissions":{"microphone":"denied"}}`／
//! `{"op":"disconnect"}`／`{"op":"reconnect"}`／`{"op":"ack-stop-all"}`／`{"op":"quit"}`。
//!
//! AIP Character Session（`docs/aip/README.md` §9.1）的 op：
//! `{"op":"aip-capability"}`（協商：intents／inputs／role remote-renderer）、
//! `{"op":"aip-touch","kind":"tap","expiresInMs":5000,"messageId":"…"?,"source":{…}?}`
//! （`source` 可覆寫以測偽造身分）、
//! `{"op":"aip-resume","lastRevision":n,"lastSequence":n,"epoch":n}`、
//! `{"op":"aip-raw","frame":{…}}`（任意 frame：未知 type／超大／壞 JSON）。
//! 收到的每則 aip frame 印一行 `{"event":"aip","envelope":…}`，送出的印
//! `{"event":"aip-sent","messageType":…,"messageId":…}`。
//!
//! 誠實預設：收到 `stop-all` **不會**自動回 ack（除非 `--auto-ack-stop-all`），
//! 這樣「桌面說已停止 vs 手機沒回應＝結果不確定」那一條路徑才測得到。

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

type HmacSha256 = Hmac<Sha256>;
type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsWrite = SplitSink<Ws, Message>;
type WsRead = SplitStream<Ws>;

/// TLS 驗證器：只釘 SHA-256 指紋（模擬 iPhone 端的 TOFU pin）。
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

fn emit(value: Value) {
    println!("{value}");
    let _ = std::io::stdout().flush();
}

fn die(message: &str) -> ! {
    eprintln!("fake_iphone: {message}");
    std::process::exit(2);
}

struct Args {
    port: u16,
    fingerprint: String,
    code: String,
    name: String,
    model: String,
    auto_ack_stop_all: bool,
}

fn parse_args() -> Args {
    let mut port: Option<u16> = None;
    let mut fingerprint: Option<String> = None;
    let mut code: Option<String> = None;
    let mut name = "模擬 iPhone（fixture）".to_string();
    let mut model = "iPhone12,1".to_string();
    let mut auto_ack_stop_all = false;
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        let mut value = || {
            argv.next()
                .unwrap_or_else(|| die(&format!("{flag} 需要一個值")))
        };
        match flag.as_str() {
            "--port" => port = Some(value().parse().unwrap_or_else(|_| die("--port 不是埠號"))),
            "--fingerprint" => fingerprint = Some(value()),
            "--code" => code = Some(value()),
            "--name" => name = value(),
            "--model" => model = value(),
            "--auto-ack-stop-all" => auto_ack_stop_all = true,
            other => die(&format!("未知參數 {other}")),
        }
    }
    Args {
        port: port.unwrap_or_else(|| die("缺少 --port")),
        fingerprint: fingerprint.unwrap_or_else(|| die("缺少 --fingerprint")),
        code: code.unwrap_or_else(|| die("缺少 --code")),
        name,
        model,
        auto_ack_stop_all,
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
    match tokio_tungstenite::connect_async_tls_with_config(
        format!("wss://127.0.0.1:{port}/"),
        None,
        false,
        Some(tokio_tungstenite::Connector::Rustls(Arc::new(config))),
    )
    .await
    {
        Ok((ws, _)) => ws,
        Err(e) => die(&format!("連不上 wss://127.0.0.1:{port}/：{e}")),
    }
}

async fn send_json(write: &mut WsWrite, value: Value) {
    if let Err(e) = write.send(Message::Text(value.to_string())).await {
        emit(json!({"event":"send-failed","error": e.to_string()}));
    }
}

/// 讀到下一則 JSON 文字訊息（Ping／Pong／二進位一律跳過）。
async fn next_json(ws: &mut Ws) -> Option<Value> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(text))) => {
                return Some(serde_json::from_str(&text).unwrap_or(Value::Null))
            }
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return None,
        }
    }
}

fn hmac_hex(code: &str, nonce: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(code.as_bytes()).expect("hmac accepts any key length");
    mac.update(nonce.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 手機自報狀態（與 iOS App 的 `status` 訊息同形狀）。
fn status_message(mic_level: bool, permissions: &Value) -> Value {
    json!({
        "type": "status",
        "sensors": {
            "motion": false,
            "battery": false,
            "micLevel": mic_level,
            "location": false,
            "bleGateway": false,
        },
        "permissions": permissions,
    })
}

fn default_permissions() -> Value {
    json!({
        "microphone": "granted",
        "location": "notDetermined",
        "bluetooth": "notDetermined",
    })
}

enum Step {
    Continue,
    Disconnect(String),
    Reconnect,
    Quit,
}

struct Phone {
    args: Args,
    device_id: String,
    token: String,
    mic_level: bool,
    permissions: Value,
    /// 這台模擬 iPhone（fixture）送出的 AIP 訊息序號（messageId 唯一）。
    aip_seq: u64,
}

impl Phone {
    /// 完成一次配對（第一次連線）。
    async fn pair(args: Args) -> (Self, WsWrite, WsRead) {
        let mut ws = connect(args.port, &args.fingerprint).await;
        let request = json!({
            "type": "pair-request",
            "deviceName": args.name,
            "model": args.model,
        });
        if let Err(e) = ws.send(Message::Text(request.to_string())).await {
            die(&format!("送不出 pair-request：{e}"));
        }
        let challenge = next_json(&mut ws)
            .await
            .unwrap_or_else(|| die("配對期間連線就斷了（沒收到 pair-challenge）"));
        if challenge["type"] != "pair-challenge" {
            die(&format!("配對被拒：{challenge}"));
        }
        let nonce = challenge["nonce"].as_str().unwrap_or_default().to_string();
        let response = json!({"type":"pair-response","hmac": hmac_hex(&args.code, &nonce)});
        if let Err(e) = ws.send(Message::Text(response.to_string())).await {
            die(&format!("送不出 pair-response：{e}"));
        }
        let paired = next_json(&mut ws)
            .await
            .unwrap_or_else(|| die("配對期間連線就斷了（沒收到 paired）"));
        if paired["type"] != "paired" {
            die(&format!("配對失敗：{paired}"));
        }
        let device_id = paired["deviceId"].as_str().unwrap_or_default().to_string();
        let token = paired["deviceToken"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let phone = Phone {
            args,
            device_id,
            token,
            mic_level: false,
            permissions: default_permissions(),
            aip_seq: 0,
        };
        let (write, read) = ws.split();
        (phone, write, read)
    }

    /// 重新連線（用配對時拿到的 token）。被撤銷時會收到 `auth-fail`。
    async fn reconnect(&self) -> Option<(WsWrite, WsRead)> {
        let mut ws = connect(self.args.port, &self.args.fingerprint).await;
        let auth = json!({"type":"auth","deviceId": self.device_id, "token": self.token});
        if let Err(e) = ws.send(Message::Text(auth.to_string())).await {
            emit(json!({"event":"reconnect-failed","error": e.to_string()}));
            return None;
        }
        match next_json(&mut ws).await {
            Some(reply) if reply["type"] == "auth-ok" => {
                let (write, read) = ws.split();
                Some((write, read))
            }
            Some(reply) => {
                emit(json!({
                    "event": "auth-fail",
                    "reason": reply["reason"].as_str().unwrap_or("unknown"),
                }));
                None
            }
            None => {
                emit(json!({"event":"auth-fail","reason":"connection closed"}));
                None
            }
        }
    }

    /// 桌面送過來的一則訊息。回傳 false＝這條連線該收掉了。
    async fn handle_inbound(&mut self, write: &mut WsWrite, message: Value) {
        match message["type"].as_str() {
            Some("stop-all") => {
                let sensors = message["sensors"] == json!(true);
                emit(json!({"event":"stop-all","sensors": sensors}));
                // 預設不自動回覆：桌面必須誠實回報「結果不確定」。
                if self.args.auto_ack_stop_all {
                    self.ack_stop_all(write).await;
                }
            }
            Some("act") => {
                let name = message["name"].as_str().unwrap_or_default().to_string();
                let params = message["params"].clone();
                emit(json!({"event":"act","name": name, "params": params}));
                let applied = if name == "character.present" {
                    json!({"state": params["state"]})
                } else {
                    params
                };
                send_json(
                    write,
                    json!({"type":"ack","id": message["id"], "applied": applied}),
                )
                .await;
            }
            Some("aip") => {
                // 每一則 AIP envelope 都原封不動印出來（下游測試自己判斷）。
                emit(json!({"event":"aip","envelope": message["envelope"].clone()}));
            }
            Some("auth-fail") => {
                emit(json!({
                    "event": "auth-fail",
                    "reason": message["reason"].as_str().unwrap_or("unknown"),
                }));
            }
            Some(other) => emit(json!({"event":"message","type": other})),
            None => {}
        }
    }

    /// 照 iOS 的行為確認 stop-all：`ack{stopAll:true}` ＋一則 `status{micLevel:false}`。
    async fn ack_stop_all(&mut self, write: &mut WsWrite) {
        self.mic_level = false;
        send_json(write, json!({"type":"ack","stopAll":true})).await;
        send_json(write, status_message(false, &self.permissions)).await;
        emit(json!({"event":"ack-stop-all"}));
    }

    async fn send_status(&mut self, write: &mut WsWrite) {
        let message = status_message(self.mic_level, &self.permissions);
        send_json(write, message).await;
        emit(json!({"event":"status","micLevel": self.mic_level}));
    }

    /// 送一則 AIP envelope（包成 `{"type":"aip","envelope":…}`）。
    async fn send_aip(&mut self, write: &mut WsWrite, envelope: Value) {
        emit(json!({
            "event": "aip-sent",
            "messageType": envelope["messageType"].clone(),
            "messageId": envelope["messageId"].clone(),
        }));
        send_json(write, json!({"type":"aip","envelope": envelope})).await;
    }

    fn next_aip_id(&mut self, prefix: &str) -> String {
        self.aip_seq += 1;
        format!("fx-{prefix}-{}", self.aip_seq)
    }

    /// 這台模擬 iPhone（fixture）的宣稱身分（`source`）。
    fn claimed_source(&self) -> Value {
        json!({"kind": "device", "id": self.device_id})
    }

    fn base_envelope(&mut self, message_type: &str, name: &str, prefix: &str) -> Value {
        let message_id = self.next_aip_id(prefix);
        json!({
            "specVersion": "aip/1.0",
            "messageId": message_id,
            "messageType": message_type,
            "name": name,
            "source": self.claimed_source(),
            "sessionId": "session.home",
            "occurredAt": chrono::Utc::now().to_rfc3339(),
            "payload": {},
        })
    }

    /// stdin 來的一則指令。
    async fn handle_op(&mut self, write: &mut WsWrite, op: Value) -> Step {
        match op["op"].as_str() {
            Some("aip-capability") => {
                let mut envelope =
                    self.base_envelope("capability", "character.session.capability", "cap");
                envelope["payload"] = json!({
                    "specVersions": ["aip/1.0"],
                    "role": "remote-renderer",
                    "profiles": ["character-session"],
                    "syncClasses": ["semantic"],
                    "intents": ["react-happily-to-touch", "celebrate", "idle"],
                    "inputs": ["character.interaction.touch"],
                    "features": {"haptic": true, "reducedMotion": false},
                    "limits": {"maxMessageBytes": 65536},
                });
                self.send_aip(write, envelope).await;
                Step::Continue
            }
            Some("aip-touch") => {
                let kind = op["kind"].as_str().unwrap_or("tap").to_string();
                let ttl = op["expiresInMs"].as_i64().unwrap_or(5_000);
                let mut envelope =
                    self.base_envelope("event", "character.interaction.touch", "touch");
                if let Some(id) = op["messageId"].as_str() {
                    envelope["messageId"] = json!(id);
                }
                // 偽造身分測試：呼叫端可以覆寫 source（host 必須拒絕）。
                if op["source"].is_object() {
                    envelope["source"] = op["source"].clone();
                }
                envelope["expiresAt"] =
                    json!((chrono::Utc::now() + chrono::Duration::milliseconds(ttl)).to_rfc3339());
                envelope["payload"] = json!({"kind": kind});
                self.send_aip(write, envelope).await;
                Step::Continue
            }
            Some("aip-resume") => {
                let mut envelope =
                    self.base_envelope("query", "character.session.resume", "resume");
                envelope["target"] = json!({"kind": "session", "id": "session.home"});
                envelope["payload"] = json!({
                    "lastRevision": op["lastRevision"].as_u64().unwrap_or(0),
                    "lastSequence": op["lastSequence"].as_u64().unwrap_or(0),
                    "sessionEpoch": op["epoch"].as_u64().unwrap_or(0),
                });
                self.send_aip(write, envelope).await;
                Step::Continue
            }
            Some("aip-raw") => {
                self.send_aip(write, op["frame"].clone()).await;
                Step::Continue
            }
            Some("status") => {
                if let Some(mic) = op.get("micLevel").and_then(Value::as_bool) {
                    self.mic_level = mic;
                }
                if let Some(permissions) = op.get("permissions") {
                    if permissions.is_object() {
                        let mut merged = self.permissions.clone();
                        if let (Some(target), Some(source)) =
                            (merged.as_object_mut(), permissions.as_object())
                        {
                            for (key, value) in source {
                                target.insert(key.clone(), value.clone());
                            }
                        }
                        self.permissions = merged;
                    }
                }
                self.send_status(write).await;
                Step::Continue
            }
            Some("ack-stop-all") => {
                self.ack_stop_all(write).await;
                Step::Continue
            }
            Some("disconnect") => Step::Disconnect("requested".into()),
            Some("reconnect") => Step::Reconnect,
            Some("quit") => Step::Quit,
            other => {
                emit(json!({"event":"unknown-op","op": other}));
                Step::Continue
            }
        }
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args = parse_args();
    let (mut phone, write, read) = Phone::pair(args).await;
    emit(json!({"deviceId": phone.device_id, "deviceToken": phone.token}));
    let mut conn = Some((write, read));
    if let Some((write, _)) = conn.as_mut() {
        phone.send_status(write).await;
    }
    emit(json!({"event":"connected"}));

    // stdin 用一般執行緒讀（workspace 的 tokio 沒開 io-std feature），
    // EOF＝父程序收工，等同 quit。
    let (op_tx, mut op_rx) = mpsc::unbounded_channel::<Value>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match std::io::BufRead::read_line(&mut stdin.lock(), &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let op: Value = serde_json::from_str(trimmed).unwrap_or(Value::Null);
                    if op_tx.send(op).is_err() {
                        return;
                    }
                }
            }
        }
        let _ = op_tx.send(json!({"op":"quit"}));
    });

    loop {
        let step = if let Some((write, read)) = conn.as_mut() {
            tokio::select! {
                incoming = read.next() => match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                        phone.handle_inbound(write, value).await;
                        Step::Continue
                    }
                    Some(Ok(_)) => Step::Continue,
                    Some(Err(e)) => Step::Disconnect(e.to_string()),
                    None => Step::Disconnect("closed by desktop".into()),
                },
                op = op_rx.recv() => match op {
                    Some(op) => phone.handle_op(write, op).await,
                    None => Step::Quit,
                },
            }
        } else {
            match op_rx.recv().await {
                Some(op) if op["op"] == json!("reconnect") => Step::Reconnect,
                Some(op) if op["op"] == json!("quit") => Step::Quit,
                Some(op) => {
                    emit(json!({"event":"ignored-while-offline","op": op["op"]}));
                    Step::Continue
                }
                None => Step::Quit,
            }
        };

        match step {
            Step::Continue => {}
            Step::Disconnect(reason) => {
                if let Some((mut write, read)) = conn.take() {
                    let _ = write.close().await;
                    drop(read);
                }
                emit(json!({"event":"disconnected","reason": reason}));
            }
            Step::Reconnect => {
                if let Some((mut write, read)) = conn.take() {
                    let _ = write.close().await;
                    drop(read);
                }
                conn = phone.reconnect().await;
                if conn.is_some() {
                    if let Some((write, _)) = conn.as_mut() {
                        phone.send_status(write).await;
                    }
                    emit(json!({"event":"connected"}));
                }
            }
            Step::Quit => {
                if let Some((mut write, read)) = conn.take() {
                    let _ = write.close().await;
                    drop(read);
                }
                emit(json!({"event":"quit"}));
                break;
            }
        }
    }
}
