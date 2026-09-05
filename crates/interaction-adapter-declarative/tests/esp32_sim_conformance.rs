//! 線協定三方一致性：`scripts/esp32-serial-sim.py`【模擬器】↔ 參考韌體
//! `firmware/esp32-companion/esp32-companion.ino` ↔ 韌體 README。
//!
//! 明確標示：這裡跑的是 python pty **模擬器**，不是 ESP32 真機。它驗的是
//! （a）模擬器與韌體對同一條規則的行為一致（配對鎖定、單行上限、nonce
//! 重放、state 欄位、hello 欄位、可控感測面），（b）README 沒有與兩者矛盾，
//! （c）host 端 serial 傳輸對這套協定的守門（超長 cmd 不上線）。
//! 真機驗收仍為零——本檔案不能拿來宣稱「真板可用」。
#![cfg(unix)]

use interaction_adapter_declarative::protocol::{
    parse_device_msg, DeviceLink, DeviceMsg, LinkError, RawLink,
};
use interaction_adapter_declarative::serial::SerialRawLink;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn read_repo_file(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// 一個模擬器子程序＋它的工作目錄（pty 路徑檔、log、facts 檔）。
struct Sim {
    child: Child,
    dir: PathBuf,
    pty_path: String,
}

impl Sim {
    /// 送一則控制指令給模擬器的 stdin（`{"op":…}`，一行一則；仿
    /// `crates/interaction-runtime/examples/fake_iphone.rs`）。
    fn control(&mut self, op: Value) {
        let stdin = self.child.stdin.as_mut().expect("simulator stdin is piped");
        writeln!(stdin, "{op}").expect("write control op");
        stdin.flush().expect("flush control op");
    }
}

impl Sim {
    /// `extra` 追加到模擬器命令列（例如 `--pair-lockout-ms 1500`、`--facts-file …`）。
    fn spawn(pairing_code: &str, extra: &[String]) -> Option<Sim> {
        if !python3_available() {
            eprintln!("python3 unavailable; skipping simulator conformance test");
            return None;
        }
        let dir = std::env::temp_dir().join(format!(
            "esp32-sim-conf-{}-{}",
            std::process::id(),
            DIR_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let pty_file = dir.join("pty");
        let mut cmd = Command::new("python3");
        cmd.arg(repo_root().join("scripts/esp32-serial-sim.py"))
            .arg("--device-id")
            .arg("esp32-sim01")
            .arg("--pairing-code")
            .arg(pairing_code)
            .arg("--pty-path-file")
            .arg(&pty_file)
            .arg("--log")
            .arg(dir.join("sim.log"))
            .args(extra)
            .stdin(Stdio::piped())
            .stderr(Stdio::null());
        let child = cmd.spawn().expect("spawn simulator");
        // 啟動期限：python 模擬器在重負載（整個 workspace 測試並行）下曾超過 5 s 才發布 pty 路徑，
        // 讓這支測試偶發失敗（整合里程碑實跑一次）。這只是測試機具的等待上限，不是產品時序。
        let deadline = Instant::now() + Duration::from_secs(20);
        let pty_path = loop {
            if let Ok(text) = std::fs::read_to_string(&pty_file) {
                if !text.trim().is_empty() {
                    break text.trim().to_string();
                }
            }
            assert!(
                Instant::now() < deadline,
                "simulator never published its pty path"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        Some(Sim {
            child,
            dir,
            pty_path,
        })
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(self.dir.join("sim.log")).unwrap_or_default()
    }

    fn sigusr1(&self) {
        let status = Command::new("kill")
            .arg("-USR1")
            .arg(self.child.id().to_string())
            .status()
            .expect("kill -USR1");
        assert!(status.success(), "kill -USR1 failed");
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // 只刪自己建的那個唯一目錄（字面 temp 路徑，不含變數展開的家目錄）。
        if self.dir.starts_with(std::env::temp_dir()) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// 直接開 pty slave 的原始客戶端（讀執行緒→channel，等待有界）。
/// 用它是為了控制**每一個位元組**（nonce、超長行）——host 的 DeviceLink
/// 會自己產 nonce、SerialRawLink 會擋超長行，測「裝置端」規則時繞不過去。
struct RawClient {
    writer: std::fs::File,
    rx: mpsc::Receiver<String>,
}

impl RawClient {
    fn open(pty_path: &str) -> RawClient {
        let writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pty_path)
            .expect("open pty slave");
        let reader = writer.try_clone().expect("clone pty fd");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(reader).lines();
            while let Some(Ok(line)) = lines.next() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        RawClient { writer, rx }
    }

    fn send_raw(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .expect("write to pty");
        self.writer.flush().expect("flush pty");
    }

    fn send(&mut self, msg: Value) {
        self.send_raw(&msg.to_string());
    }

    /// 下一行（原文＋解析後），2 秒內沒有就是 None。
    fn recv(&self) -> Option<(String, Value)> {
        self.recv_within(Duration::from_secs(2))
    }

    fn recv_within(&self, timeout: Duration) -> Option<(String, Value)> {
        let raw = self.rx.recv_timeout(timeout).ok()?;
        let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        Some((raw, parsed))
    }

    /// 等一則符合條件的行（其餘忽略）。有界：超過 `timeout` 就 None。
    fn recv_matching(
        &self,
        mut want: impl FnMut(&Value) -> bool,
        timeout: Duration,
    ) -> Option<(String, Value)> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let line = self.recv_within(left)?;
            if want(&line.1) {
                return Some(line);
            }
        }
    }

    fn ask(&mut self, msg: Value) -> Value {
        self.ask_raw(msg).1
    }

    /// 送一則、等它的回覆。已配對的模擬器每 5 秒會主動推播 state（與韌體
    /// 一樣），那不是這則請求的回覆——除非請求本身就是 read。
    fn ask_raw(&mut self, msg: Value) -> (String, Value) {
        self.send(msg.clone());
        loop {
            let reply = self
                .recv()
                .unwrap_or_else(|| panic!("no reply within 2s to {msg}"));
            if reply.1["type"] == "state" && msg["type"] != "read" {
                continue;
            }
            return reply;
        }
    }

    fn pair(&mut self, code: &str) {
        let hello = self.ask(json!({"type": "who"}));
        assert_eq!(hello["type"], "hello", "{hello}");
        let ok = self.ask(json!({"type": "pair", "code": code}));
        assert_eq!(ok["type"], "pair-ok", "{ok}");
    }
}

/// JSON-ish 文字裡「第一層」的鍵（依出現順序）：用來從 README 的 state 範例
/// 與模擬器的原始輸出抽欄位順序，不受 serde_json Map 排序影響。
fn top_level_keys(json_like: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut current = String::new();
    let mut chars = json_like.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            if c == '"' {
                in_string = false;
                // 只有後面緊接 ':' 的字串才是鍵。
                let mut look = chars.clone();
                while let Some(&n) = look.peek() {
                    if n == ' ' {
                        look.next();
                    } else {
                        break;
                    }
                }
                if look.peek() == Some(&':') && depth == 1 {
                    keys.push(current.clone());
                }
                current.clear();
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }
    keys
}

/// 從 `.ino` 抽一個函式本體（從 `static void NAME(` 的**定義**到下一個行首 `}`）。
/// 檔案上半部有同名的函式原型（`static void sendHello(Link link);`），
/// 必須跳過：只有 `(` 之後先遇到 `{`（而不是 `;`）的才是定義。
fn firmware_function_body(source: &str, signature: &str) -> String {
    let mut from = 0;
    while let Some(i) = source[from..].find(signature) {
        let start = from + i;
        let rest = &source[start..];
        let is_definition = match (rest.find('{'), rest.find(';')) {
            (Some(brace), Some(semi)) => brace < semi,
            (Some(_), None) => true,
            _ => false,
        };
        if is_definition {
            let end = rest.find("\n}").map(|i| i + 2).unwrap_or(rest.len());
            return rest[..end].to_string();
        }
        from = start + signature.len();
    }
    panic!("firmware lacks a definition of {signature}");
}

/// `facts["key"]`／`doc["key"]` 這種 ArduinoJson 寫法的鍵（依出現順序、去重）。
fn bracket_keys(body: &str, object: &str) -> Vec<String> {
    let needle = format!("{object}[\"");
    let mut keys: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find(&needle) {
        rest = &rest[i + needle.len()..];
        if let Some(end) = rest.find("\"]") {
            let key = rest[..end].to_string();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

// ---------------------------------------------------------------------------
// protocol-conformance-014：state.facts 三方一致（含 buzzActive）
// ---------------------------------------------------------------------------

#[test]
fn state_facts_are_identical_across_firmware_simulator_and_readme() {
    let ino = read_repo_file("firmware/esp32-companion/esp32-companion.ino");
    let firmware = bracket_keys(
        &firmware_function_body(&ino, "static void buildState("),
        "facts",
    );
    assert!(
        firmware.contains(&"buzzActive".to_string()),
        "firmware buildState() must report buzzActive: {firmware:?}"
    );

    let readme = read_repo_file("firmware/esp32-companion/README.md");
    let state_row = readme
        .lines()
        .find(|l| l.contains("\"type\":\"state\"") && l.contains("\"facts\":{"))
        .expect("README protocol table has a state row");
    let facts_start = state_row.find("\"facts\":{").expect("facts object") + "\"facts\":".len();
    let readme_keys = top_level_keys(&state_row[facts_start..]);
    assert_eq!(
        readme_keys, firmware,
        "README state row ≠ firmware buildState()"
    );

    let Some(sim) = Sim::spawn("", &[]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    let (raw, state) = client.ask_raw(json!({"type": "read"}));
    assert_eq!(state["type"], "state", "{state}");
    let facts_start = raw.find("\"facts\": {").expect("facts object") + "\"facts\": ".len();
    let sim_keys = top_level_keys(&raw[facts_start..]);
    assert_eq!(
        sim_keys, firmware,
        "simulator build_state() ≠ firmware buildState()"
    );
}

// ---------------------------------------------------------------------------
// protocol-conformance-016：hello 欄位（含 pairingLocked）三方一致
// ---------------------------------------------------------------------------

#[test]
fn hello_fields_are_identical_across_firmware_simulator_and_readme() {
    let ino = read_repo_file("firmware/esp32-companion/esp32-companion.ino");
    let firmware = bracket_keys(
        &firmware_function_body(&ino, "static void sendHello("),
        "doc",
    );
    assert!(
        firmware.contains(&"pairingLocked".to_string()),
        "{firmware:?}"
    );

    let readme = read_repo_file("firmware/esp32-companion/README.md");
    let example = readme
        .lines()
        .find(|l| l.trim_start().starts_with("{\"type\":\"hello\""))
        .expect("README has a hello example line");
    assert_eq!(
        top_level_keys(example.trim()),
        firmware,
        "README hello example ≠ firmware"
    );

    let Some(sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    let (raw, hello) = client.ask_raw(json!({"type": "who"}));
    assert_eq!(hello["type"], "hello", "{hello}");
    assert_eq!(
        top_level_keys(&raw),
        firmware,
        "simulator hello ≠ firmware sendHello()"
    );
    assert_eq!(hello["pairingLocked"], false);
}

// ---------------------------------------------------------------------------
// protocol-conformance-012：nonce 在裝置端擋重放；README 不得再寫「僅收不驗」
// ---------------------------------------------------------------------------

#[test]
fn a_replayed_nonce_is_deduplicated_on_the_device_and_the_readme_says_so() {
    let readme = read_repo_file("firmware/esp32-companion/README.md");
    assert!(
        !readme.contains("僅收不驗"),
        "README protocol table must not claim the nonce is unchecked"
    );
    let cmd_row = readme
        .lines()
        .find(|l| l.contains("\"type\":\"cmd\"") && l.contains("\"nonce\""))
        .expect("README protocol table has a cmd row");
    assert!(
        cmd_row.contains("重放"),
        "cmd row must describe the device-side replay check: {cmd_row}"
    );

    let Some(sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    client.pair("9927");
    let first = client.ask(json!({
        "type": "cmd", "id": "A", "nonce": "N1", "name": "led.set", "params": {"r": 10}
    }));
    assert_eq!(first["applied"]["r"], 10, "{first}");
    let replay = client.ask(json!({
        "type": "cmd", "id": "B", "nonce": "N1", "name": "led.set", "params": {"r": 200}
    }));
    assert_eq!(replay, json!({"type": "ack", "id": "B", "dup": true}));
    let state = client.ask(json!({"type": "read"}));
    assert_eq!(
        state["facts"]["led"]["r"], 10,
        "a replayed nonce must not re-apply"
    );
}

// ---------------------------------------------------------------------------
// protocol-conformance-013：單行上限 639 bytes（模擬器鏡射韌體；host 先擋）
// ---------------------------------------------------------------------------

fn led_cmd_padded_to(len: usize, id: &str, nonce: &str) -> String {
    let base = json!({"type": "cmd", "id": id, "nonce": nonce, "name": "led.set",
                      "params": {"r": 30, "pad": ""}})
    .to_string();
    assert!(base.len() <= len, "base cmd already longer than {len}");
    json!({"type": "cmd", "id": id, "nonce": nonce, "name": "led.set",
           "params": {"r": 30, "pad": "x".repeat(len - base.len())}})
    .to_string()
}

#[test]
fn the_simulator_drops_an_oversize_line_with_an_id_less_bad_json_like_the_firmware() {
    let Some(sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    client.pair("9927");

    let exact = led_cmd_padded_to(639, "E", "N-exact");
    assert_eq!(exact.len(), 639);
    client.send_raw(&exact);
    let reply = client.recv().expect("reply to a 639-byte line").1;
    assert_eq!(reply["applied"]["r"], 30, "639 bytes is accepted: {reply}");

    let over = led_cmd_padded_to(640, "F", "N-over");
    assert_eq!(over.len(), 640);
    client.send_raw(&over);
    let reply = client.recv().expect("reply to a 640-byte line").1;
    assert_eq!(
        reply,
        json!({"type": "err", "reason": "bad-json"}),
        "the firmware answers an id-less bad-json for an overlong line"
    );
    let state = client.ask(json!({"type": "read"}));
    assert_eq!(
        state["facts"]["led"]["r"], 30,
        "the overlong cmd must not have been applied"
    );
}

/// host 端（真的 SerialRawLink＋DeviceLink 對模擬器）：超長 cmd 在寫出之前
/// 就被拒——收據原因 message-too-large、sim.log 裡沒有那則 cmd；正常 cmd 照走。
#[tokio::test(flavor = "multi_thread")]
async fn the_host_serial_transport_refuses_an_oversize_cmd_before_the_wire() {
    let Some(sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let raw = SerialRawLink::spawn(sim.pty_path.clone(), 115_200);
    let link = DeviceLink::new(raw.clone(), "esp32-sim01".into(), Some("9927".into()));
    let err = link
        .command(
            "big-cmd",
            "led.set",
            json!({"r": 10, "pad": "x".repeat(700)}),
            Duration::from_secs(2),
        )
        .await
        .expect_err("a 700-byte cmd must be refused on the host");
    match &err {
        LinkError::Refused(detail) => assert!(detail.starts_with("message too large"), "{detail}"),
        other => panic!("expected Refused, got {other}"),
    }
    let ack = link
        .command(
            "small-cmd",
            "led.set",
            json!({"r": 10}),
            Duration::from_secs(2),
        )
        .await
        .expect("a normal cmd still works on the same link");
    assert!(matches!(ack, DeviceMsg::Ack { .. }), "{ack:?}");
    // 模擬器端只看到握手與小 cmd，從沒收到超長那則。
    let log = sim.log_text();
    assert!(log.contains("\"small-cmd\""), "{log}");
    assert!(
        !log.contains("\"big-cmd\""),
        "the oversize cmd must never reach the wire:\n{log}"
    );
    raw.shutdown();
}

// ---------------------------------------------------------------------------
// protocol-conformance-016：配對暴力猜測 → 鎖定（模擬器鏡射韌體規則）
// ---------------------------------------------------------------------------

#[test]
fn five_wrong_pairing_codes_lock_pairing_until_the_window_passes() {
    let Some(sim) = Sim::spawn("9927", &["--pair-lockout-ms".into(), "1500".into()]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    for attempt in 1..=4 {
        let r = client.ask(json!({"type": "pair", "code": "wrong"}));
        assert_eq!(r, json!({"type": "pair-fail"}), "attempt {attempt}");
    }
    let locked = client.ask(json!({"type": "pair", "code": "wrong"}));
    assert_eq!(locked["type"], "pair-fail", "{locked}");
    assert_eq!(locked["reason"], "pair-locked", "{locked}");
    assert_eq!(locked["retryAfterMs"], 1500, "{locked}");

    // 鎖定期間：正確碼也不比對（誠實回 pair-locked，不延長鎖定）。
    let refused = client.ask(json!({"type": "pair", "code": "9927"}));
    assert_eq!(refused["reason"], "pair-locked", "{refused}");
    let hello = client.ask(json!({"type": "who"}));
    assert_eq!(hello["pairing"], true, "{hello}");
    assert_eq!(hello["pairingLocked"], true, "{hello}");
    let read = client.ask(json!({"type": "read"}));
    assert_eq!(read, json!({"type": "err", "reason": "not-paired"}));
    // stop-all 在鎖定期間仍然可用（fail-safe 方向，與配對無關）。
    let stop = client.ask(json!({"type": "stop-all"}));
    assert_eq!(stop, json!({"type": "ack", "stopAll": true}));

    std::thread::sleep(Duration::from_millis(1_700));
    let hello = client.ask(json!({"type": "who"}));
    assert_eq!(hello["pairingLocked"], false, "{hello}");
    let ok = client.ask(json!({"type": "pair", "code": "9927"}));
    assert_eq!(ok, json!({"type": "pair-ok"}));
    let state = client.ask(json!({"type": "read"}));
    assert_eq!(state["type"], "state", "{state}");
}

/// host 端只認 `type`：鎖定期內的 `pair-fail`（多了 reason／retryAfterMs）
/// 與帶 `pairingLocked` 的 hello 都要能解析——README〈配對流程〉如此承諾，
/// 否則真板一鎖定，host 會把它的回覆當成壞訊息而不是「配對被拒」。
#[test]
fn the_host_parses_the_locked_pair_fail_and_the_pairing_locked_hello() {
    // link-transports-051：reason／retryAfterMs 不得被靜默吃掉。裝置已經算好
    // 「碼可能是對的、N 毫秒後再試」，丟掉它就只剩「配對碼被拒絕」。
    assert_eq!(
        parse_device_msg(r#"{"type":"pair-fail","reason":"pair-locked","retryAfterMs":30000}"#),
        Some(DeviceMsg::PairFail {
            reason: Some("pair-locked".into()),
            retry_after_ms: Some(30_000),
        })
    );
    // 沒帶原因的 pair-fail（一般錯碼）仍要解析得出來。
    assert_eq!(
        parse_device_msg(r#"{"type":"pair-fail"}"#),
        Some(DeviceMsg::PairFail {
            reason: None,
            retry_after_ms: None,
        })
    );
    let hello = parse_device_msg(
        r#"{"type":"hello","deviceId":"esp32-companion-01","fw":"1.0.0","proto":1,"caps":["led.set"],"pairing":true,"pairingLocked":true}"#,
    );
    match hello {
        Some(DeviceMsg::Hello {
            device_id,
            pairing,
            pairing_locked,
            ..
        }) => {
            assert_eq!(device_id, "esp32-companion-01");
            assert!(pairing);
            assert!(pairing_locked, "hello.pairingLocked must not be dropped");
        }
        other => panic!("hello with pairingLocked must still parse: {other:?}"),
    }
}

/// 韌體端同一條規則存在（桌面上不能執行 .ino，至少把規則綁進測試）。
#[test]
fn the_firmware_implements_the_same_pairing_lockout() {
    let ino = read_repo_file("firmware/esp32-companion/esp32-companion.ino");
    assert!(
        ino.contains("PAIR_MAX_FAILURES = 5"),
        "firmware lockout threshold"
    );
    assert!(
        ino.contains("PAIR_LOCKOUT_MS   = 30000"),
        "firmware lockout window"
    );
    let pair_branch = {
        let start = ino
            .find("if (strcmp(type, \"pair\") == 0)")
            .expect("pair branch");
        let rest = &ino[start..];
        let end = rest.find("stop-all").unwrap_or(rest.len());
        rest[..end].to_string()
    };
    assert!(
        pair_branch.contains("pairLocked(link, now)"),
        "lockout gate before the compare"
    );
    assert!(
        pair_branch.contains("\"pair-locked\""),
        "locked reply reason"
    );
    assert!(
        pair_branch.contains("retryAfterMs"),
        "locked reply retryAfterMs"
    );
    assert!(
        pair_branch.contains("++g_pairFailures[link] >= PAIR_MAX_FAILURES"),
        "failure counter"
    );
    let readme = read_repo_file("firmware/esp32-companion/README.md");
    assert!(
        readme.contains("pair-locked"),
        "README pairing section documents the lockout"
    );
    assert!(
        readme.contains("pairingLocked"),
        "README documents hello.pairingLocked"
    );
}

// ---------------------------------------------------------------------------
// protocol-conformance-011：可控感測面；-1／null 原樣穿透；按鈕邊緣主動推播
// ---------------------------------------------------------------------------

#[test]
fn the_sensor_face_is_controllable_and_absent_values_pass_through() {
    let facts = std::env::temp_dir().join(format!(
        "esp32-sim-facts-{}-{}.json",
        std::process::id(),
        DIR_SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::write(&facts, "{}").expect("facts file");
    let Some(sim) = Sim::spawn(
        "9927",
        &["--facts-file".into(), facts.to_string_lossy().into_owned()],
    ) else {
        let _ = std::fs::remove_file(&facts);
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    client.pair("9927");
    let state = client.ask(json!({"type": "read"}));
    assert_eq!(state["facts"]["button"], false);
    assert_eq!(state["facts"]["distanceMm"], 842);

    // 按鈕邊緣 → 裝置主動推播（沒有 read）。
    sim.sigusr1();
    let pushed = client
        .recv()
        .expect("unsolicited state after the button edge")
        .1;
    assert_eq!(pushed["type"], "state", "{pushed}");
    assert_eq!(pushed["facts"]["button"], true, "{pushed}");
    let reads_in_log = sim
        .log_text()
        .lines()
        .filter(|l| l.starts_with(">>") && l.contains("\"read\""))
        .count();
    assert_eq!(
        reads_in_log, 1,
        "the push must not have been answered to a read"
    );

    // 控制檔：距離變 150；型別錯的鍵被忽略（不冒充韌體不可能回報的值）。
    std::fs::write(&facts, r#"{"distanceMm": 150, "lux": 9, "button": "bad"}"#).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let state = client.ask(json!({"type": "read"}));
    assert_eq!(state["facts"]["distanceMm"], 150, "{state}");
    assert_eq!(state["facts"]["lux"], 9, "{state}");
    assert_eq!(
        state["facts"]["button"], true,
        "a non-bool button must be ignored: {state}"
    );

    // 感測器缺席：-1／null 原樣穿透（不是 0、不是被吞掉）。
    std::fs::write(
        &facts,
        r#"{"distanceMm": -1, "tempC": null, "button": false}"#,
    )
    .unwrap();
    let pushed = client
        .recv()
        .expect("button edge from the facts file pushes state")
        .1;
    assert_eq!(pushed["facts"]["button"], false, "{pushed}");
    let state = client.ask(json!({"type": "read"}));
    assert_eq!(state["facts"]["distanceMm"], -1, "{state}");
    assert!(state["facts"]["tempC"].is_null(), "{state}");
    assert!(
        state["facts"].get("tempC").is_some(),
        "tempC must be present as null, not dropped: {state}"
    );
    // 韌體的 -1／null 語意也在 README 裡。
    let readme = read_repo_file("firmware/esp32-companion/README.md");
    assert!(
        readme.contains("--facts-file"),
        "README documents the simulator control channel"
    );
    let _ = std::fs::remove_file(&facts);
}

/// 起動即「感測器未接」模式。
#[test]
fn sensors_absent_mode_reports_the_firmware_honest_values() {
    let Some(sim) = Sim::spawn("", &["--sensors-absent".into()]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    let state = client.ask(json!({"type": "read"}));
    assert_eq!(state["facts"]["distanceMm"], -1, "{state}");
    assert!(state["facts"]["tempC"].is_null(), "{state}");
}

// ---------------------------------------------------------------------------
// protocol-conformance-009：韌體 BLE 廣播必須帶名稱（scan response）
// ---------------------------------------------------------------------------

#[test]
fn the_firmware_advertises_its_name_in_the_scan_response() {
    let ino = read_repo_file("firmware/esp32-companion/esp32-companion.ino");
    let body = firmware_function_body(&ino, "static void setupBle(");
    let enable = body
        .find("adv->enableScanResponse(true)")
        .expect("setupBle() must enable the scan response (NimBLE 2.x default is off)");
    let name = body
        .find("adv->setName(DEVICE_ID)")
        .expect("setupBle() must advertise DEVICE_ID as the local name");
    let start = body
        .find("adv->start()")
        .expect("setupBle() starts advertising");
    assert!(
        enable < name,
        "enableScanResponse must precede setName: NimBLE only puts the name into the scan \
         response when it is already enabled (the 128-bit UUID leaves no room in the main packet)"
    );
    assert!(name < start, "the name must be set before adv->start()");
    let readme = read_repo_file("firmware/esp32-companion/README.md");
    assert!(
        readme.contains("scan response"),
        "README must explain where the name lives and that runtime matches on the service UUID"
    );
}

/// 內部工具自檢：鍵抽取器對巢狀物件只回第一層。
#[test]
fn top_level_key_scanner_ignores_nested_objects() {
    assert_eq!(
        top_level_keys(r#"{"a":1,"b":{"x":2,"y":{"z":3}},"c":null}"#),
        vec!["a", "b", "c"]
    );
    assert_eq!(top_level_keys(r#"{"a": 1, "b": {"x": 2}}"#), vec!["a", "b"]);
}

// ---------------------------------------------------------------------------
// protocol-conformance-027：超出 float32 範圍的數值參數 —— 韌體 clamp 後 ack，
// 模擬器必須做同一件事。舊版讓 struct.pack 的 OverflowError 冒出去，
// **整個模擬器程序退出**：host 只看到 ack/read 逾時，把參數問題誤診成傳輸問題。
// ---------------------------------------------------------------------------

#[test]
fn out_of_range_numbers_are_clamped_like_the_firmware_and_never_kill_the_simulator() {
    let Some(mut sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    client.pair("9927");

    // 韌體：readNumber → as<float>()（溢位成 ±inf）→ roundToLong → clampLong。
    // 一定會 ack，且 applied 落在硬限制表的邊界值上。
    let cases: Vec<(&str, Value, &str, Value)> = vec![
        (
            "buzzer.beep",
            json!({"durationMs": 1e39}),
            "durationMs",
            json!(2000),
        ),
        ("led.set", json!({"r": 1e39}), "r", json!(255)),
        ("servo.move", json!({"angle": 1e40}), "angle", json!(170)),
        (
            "vibe.pulse",
            json!({"strength": 0.5, "durationMs": 1e39}),
            "durationMs",
            json!(3000),
        ),
    ];
    for (i, (name, params, field, expected)) in cases.iter().enumerate() {
        let reply = client.ask(json!({
            "type": "cmd", "id": format!("oor-{i}"), "nonce": format!("n-oor-{i}"),
            "name": name, "params": params
        }));
        assert_eq!(
            reply["type"], "ack",
            "{name} 超界參數必須 clamp 後 ack（韌體行為），得到 {reply}"
        );
        assert_eq!(
            reply["applied"][field], *expected,
            "{name}.{field} 必須被 clamp 到硬限制邊界：{reply}"
        );
        // 節流：vibe/servo 兩次之間要等夠久（韌體同規則）。
        std::thread::sleep(Duration::from_millis(600));
    }

    // 而且模擬器還活著——「裝置憑空消失」不是合法的協定回覆。
    let hello = client.ask(json!({"type": "who"}));
    assert_eq!(hello["type"], "hello", "模擬器必須還在：{hello}");
    assert!(
        sim.child.try_wait().expect("try_wait").is_none(),
        "模擬器程序不得因為一個參數而退出"
    );
}

/// 負的超界值往另一個邊界收斂（同一條 clamp 規則的另一半）。
#[test]
fn a_negative_out_of_range_number_clamps_to_the_low_bound() {
    let Some(sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    client.pair("9927");
    let reply = client.ask(json!({
        "type": "cmd", "id": "neg", "nonce": "n-neg",
        "name": "servo.move", "params": {"angle": -1e39}
    }));
    assert_eq!(reply["type"], "ack", "{reply}");
    assert_eq!(reply["applied"]["angle"], json!(10), "{reply}");
}

/// ArduinoJson 不接受 `NaN`／`Infinity` 字面值（整則訊息解析失敗 → bad-json）。
/// Python 的 json 預設**會**接受——模擬器必須擋掉，兩端才一致。
#[test]
fn json_constants_the_firmware_parser_rejects_are_bad_json_in_the_simulator_too() {
    let Some(sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    client.pair("9927");
    client
        .send_raw(r#"{"type":"cmd","id":"nan","nonce":"n1","name":"led.set","params":{"r":NaN}}"#);
    let reply = client.recv().expect("reply to a NaN literal").1;
    assert_eq!(
        reply,
        json!({"type": "err", "reason": "bad-json"}),
        "NaN 字面值：韌體的解析器讀不了，整則 bad-json"
    );
    let hello = client.ask(json!({"type": "who"}));
    assert_eq!(hello["type"], "hello", "模擬器必須還在：{hello}");
}

// ---------------------------------------------------------------------------
// protocol-conformance-031：README 的指令參數段不得與硬限制表矛盾
// ---------------------------------------------------------------------------

#[test]
fn the_readme_command_parameter_ranges_match_the_hard_limits() {
    let readme = read_repo_file("firmware/esp32-companion/README.md");
    assert!(
        !readme.contains("0..180"),
        "README 曾把 servo.move 寫成 angle 0..180，與硬限制表（10..170）與兩端實作矛盾"
    );
    let params_line = readme
        .lines()
        .find(|l| l.contains("servo.move {angle"))
        .expect("README lists the command parameters");
    assert!(
        params_line.contains("10..170"),
        "指令參數段的 servo 範圍必須與硬限制表一致：{params_line}"
    );
    // 兩端實作也必須是同一組邊界（README 不是唯一真相）。
    let ino = read_repo_file("firmware/esp32-companion/esp32-companion.ino");
    // 韌體用對齊過的空白（`SERVO_MIN_ANGLE      = 10;`），所以比對「常數名
    // 那一行的值」而不是固定字串。
    let firmware_bound = |name: &str| -> String {
        let line = ino
            .lines()
            .find(|l| l.contains(name) && l.contains('='))
            .unwrap_or_else(|| panic!("firmware defines {name}"));
        line.split('=')
            .nth(1)
            .unwrap_or_default()
            .trim()
            .trim_end_matches(';')
            .to_string()
    };
    assert_eq!(firmware_bound("SERVO_MIN_ANGLE"), "10");
    assert_eq!(firmware_bound("SERVO_MAX_ANGLE"), "170");
    let sim = read_repo_file("scripts/esp32-serial-sim.py");
    assert!(
        sim.contains("SERVO_MIN_ANGLE = 10"),
        "simulator servo lower bound"
    );
    assert!(
        sim.contains("SERVO_MAX_ANGLE = 170"),
        "simulator servo upper bound"
    );
}

// ---------------------------------------------------------------------------
// protocol-conformance-028：device→host 也必須有長度紀律（韌體＋README）
// ---------------------------------------------------------------------------

#[test]
fn the_firmware_gives_ble_notifications_a_length_discipline() {
    let ino = read_repo_file("firmware/esp32-companion/esp32-companion.ino");
    assert!(
        ino.contains("NimBLEDevice::setMTU("),
        "韌體必須主動提高偏好 MTU，否則預設 23 只送得動 20 bytes 的 payload"
    );
    let send_line = firmware_function_body(&ino, "static void sendLine(");
    assert!(
        send_line.contains("bleNotifyPayloadMax()"),
        "sendLine(LINK_BLE) 必須依協商後的 MTU 分段，不能無條件 setValue+notify：\n{send_line}"
    );
    assert!(
        send_line.contains("kNewline"),
        "分段之後要以換行界定一則訊息（host 端據此重組）：\n{send_line}"
    );
    let readme = read_repo_file("firmware/esp32-companion/README.md");
    assert!(
        readme.contains("裝置→host 方向") && readme.contains("MTU"),
        "README 的訊息上限段必須也寫出 device→host 方向（BLE 的 MTU-3 限制）"
    );
}

// ---------------------------------------------------------------------------
// protocol-conformance-044：模擬器的浮點 ack 必須是韌體送得出來的位元組
// ---------------------------------------------------------------------------

/// `"strength": 0.7` 這一格，模擬器與韌體不得長得不一樣。
/// 韌體：`float strength` → ArduinoJson 對 4-byte float 用
/// `decimalPlaces = 6`（`sizeof(T) >= 8 ? 9 : 6`）＋去尾零 → `0.7`。
/// Python 的 `json.dumps` 直接吐 float64 的 repr：float32 化之後的
/// `0.699999988079071`——真板永遠不會這樣送，任何以模擬器產生的黃金輸出
/// 都會與真板不同。
#[test]
fn the_simulator_serialises_the_vibe_strength_like_the_firmware_float() {
    let ino = read_repo_file("firmware/esp32-companion/esp32-companion.ino");
    let body = firmware_function_body(&ino, "static bool cmdVibePulse(");
    assert!(
        body.contains("float strength") && body.contains("applied[\"strength\"] = strength;"),
        "前提：韌體 ack 的 strength 是 C `float`（ArduinoJson 最多 6 位小數）：\n{body}"
    );

    let Some(sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);
    client.pair("9927");

    // 都是 policy 裁剪後常見的 magnitude，且都不是 float32 精確可表示的值。
    let cases = [("0.7", "0.7"), ("0.3", "0.3"), ("0.55", "0.55")];
    for (i, (requested, expected)) in cases.iter().enumerate() {
        let (raw_line, reply) = client.ask_raw(json!({
            "type": "cmd", "id": format!("vib-{i}"), "nonce": format!("n-vib-{i}"),
            "name": "vibe.pulse",
            "params": {"strength": requested.parse::<f64>().expect("case"), "durationMs": 1},
        }));
        assert_eq!(reply["type"], "ack", "{reply}");
        let literal = json_number_literal(&raw_line, "strength");
        assert_eq!(
            &literal, expected,
            "模擬器送出的位元組必須與韌體相同（原始行：{raw_line}）"
        );
        // 節流：韌體與模擬器都要求「上一發結束後」≥ 500ms（模擬器的 tick
        // 是 100ms，所以這裡留足餘裕）。
        std::thread::sleep(Duration::from_millis(800));
    }
}

/// 從一行原始 JSON 取出某個鍵的**字面值**（不經 serde 的數字正規化——
/// 這裡要驗的就是位元組本身）。
fn json_number_literal(raw_line: &str, key: &str) -> String {
    let needle = format!("\"{key}\"");
    let start = raw_line
        .find(&needle)
        .unwrap_or_else(|| panic!("{key} not in {raw_line}"))
        + needle.len();
    let rest = raw_line[start..].trim_start();
    let rest = rest
        .strip_prefix(':')
        .unwrap_or_else(|| panic!("{raw_line}"));
    rest.trim_start()
        .chars()
        .take_while(|c| !matches!(c, ',' | '}' | ' '))
        .collect()
}

// ---------------------------------------------------------------------------
// protocol-conformance-019：線協定 v1.1 的 `aip` 訊息三方一致
// ---------------------------------------------------------------------------

/// 韌體（`handleMessage`）、README 協定表、模擬器對 `aip` 這一種訊息必須說
/// 同一件事：**配對之後**收到的 host `aip` 行一律忽略（不回 err、不當成
/// unknown-type），配對之前照舊落在 `not-paired`。
///
/// 為什麼要有這條：`aip` 是 v1.1 追加的訊息，舊韌體不認得它。如果參考韌體
/// 把它當 `unknown-type` 回錯誤，host 每送一則 session frame 就換回一則
/// 錯誤——那會讓「這台裝置不支援 AIP」看起來像「這台裝置壞了」。
#[test]
fn aip_messages_are_handled_identically_by_firmware_simulator_and_readme() {
    let ino = read_repo_file("firmware/esp32-companion/esp32-companion.ino");
    let handle = firmware_function_body(&ino, "static void handleMessage(");
    assert!(
        handle.contains("\"aip\""),
        "韌體 handleMessage() 必須明確處理 aip（忽略），否則它會落到 unknown-type：\n{handle}"
    );

    let readme = read_repo_file("firmware/esp32-companion/README.md");
    assert!(
        readme.contains("{\"type\":\"aip\",\"envelope\":{..}}"),
        "README 協定表必須有 aip 這一列（兩個方向）"
    );

    let Some(mut sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let mut client = RawClient::open(&sim.pty_path);

    // 配對前：與 cmd/read 同一條規則——not-paired，不是靜默。
    let refused = client.ask(json!({"type": "aip", "envelope": {"specVersion": "aip/1.0"}}));
    assert_eq!(refused["type"], "err", "{refused}");
    assert_eq!(refused["reason"], "not-paired", "{refused}");

    client.pair("9927");

    // 配對後：忽略（不回任何東西），而且不得讓後續請求錯位。
    client.send(json!({"type": "aip", "envelope": {"specVersion": "aip/1.0"}}));
    let state = client.ask(json!({"type": "read"}));
    assert_eq!(state["type"], "state", "aip 之後 read 仍必須正常：{state}");
    assert!(
        sim.log_text().contains("aip"),
        "模擬器必須把忽略掉的 aip 行留痕：\n{}",
        sim.log_text()
    );

    // 裝置→host 方向：控制通道要求模擬器送一則 capability envelope。
    sim.control(json!({"op": "aip-capability"}));
    let frame = client
        .recv_matching(|v| v["type"] == "aip", Duration::from_secs(3))
        .expect("simulator must emit an aip line for the aip-capability op");
    let parsed = parse_device_msg(&frame.0).expect("host parses the simulator's aip line");
    match parsed {
        DeviceMsg::Aip { envelope } => {
            assert_eq!(envelope["specVersion"], "aip/1.0", "{envelope}");
            assert_eq!(envelope["messageType"], "capability", "{envelope}");
            assert_eq!(envelope["source"]["kind"], "device", "{envelope}");
            assert_eq!(envelope["source"]["id"], "esp32-sim01", "{envelope}");
        }
        other => panic!("expected DeviceMsg::Aip, got {other:?}"),
    }
}

/// 未配對的通道不得送出 session 流量：模擬器對 `aip-*` 控制指令必須誠實拒絕
/// （而不是先送出去再說）。裝置端與 host 端的閘門方向一致。
#[test]
fn the_simulator_refuses_to_emit_aip_before_pairing() {
    let Some(mut sim) = Sim::spawn("9927", &[]) else {
        return;
    };
    let client = RawClient::open(&sim.pty_path);
    sim.control(json!({"op": "aip-touch", "kind": "tap"}));
    assert!(
        client
            .recv_matching(|v| v["type"] == "aip", Duration::from_millis(800))
            .is_none(),
        "未配對的通道不得送出 aip"
    );
    assert!(
        sim.log_text().contains("aip op refused"),
        "拒絕必須留痕：\n{}",
        sim.log_text()
    );
}
