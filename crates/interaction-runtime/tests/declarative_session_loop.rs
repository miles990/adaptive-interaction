//! 第二裝置的 AIP profile：Serial【模擬器】經 **production** 的宣告式 adapter
//! ／`DeviceLink` 進入 Character Session（M2 §3.3）。
//!
//! 明確標示：這裡跑的是 `scripts/esp32-serial-sim.py` pty **模擬器**，不是
//! ESP32 真板。真板驗收仍為零——本檔案不能拿來宣稱「真板可用」。裡面的
//! 「另一台裝置成員」也只是**程序內 fixture**（直接 join 一個 device party），
//! 不是 fake_iphone 程序、更不是 iPhone 真機。
//!
//! 覆蓋：capability 協商回覆、touch → shared state 改變並到達桌面成員、
//! 能力宣告與 SensorSource 登記、stop_all_sensors 取得真實 confirmed／
//! 模擬器不回 ack 時 uncertain、no-stop-path 不再出現、撤銷 → leave＋retract
//! ＋unregister（且不影響其他成員）、斷線 → presence reconnecting → tick 轉
//! offline。
#![cfg(unix)]

use interaction_aip::Party;
use interaction_core::{ProviderId, ReceptorId};
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const DEVICE_ID: &str = "esp32-sim01";
const PAIRING_CODE: &str = "9927";
const SPEC_ID: &str = "esp32sim";
const PROVIDER_ID: &str = "provider.adapter.esp32sim";
const RECEPTOR_ID: &str = "esp32sim.presence";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
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

/// ESP32 序列【模擬器】子程序（pty）。
struct Sim {
    child: Child,
    dir: PathBuf,
    pty_path: String,
}

impl Sim {
    fn spawn() -> Option<Sim> {
        if !python3_available() {
            eprintln!("python3 unavailable; skipping declarative session loop test");
            return None;
        }
        let dir = std::env::temp_dir().join(format!(
            "decl-session-{}-{}",
            std::process::id(),
            DIR_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let pty_file = dir.join("pty");
        let child = Command::new("python3")
            .arg(repo_root().join("scripts/esp32-serial-sim.py"))
            .arg("--device-id")
            .arg(DEVICE_ID)
            .arg("--pairing-code")
            .arg(PAIRING_CODE)
            .arg("--pty-path-file")
            .arg(&pty_file)
            .arg("--log")
            .arg(dir.join("sim.log"))
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn simulator");
        let deadline = Instant::now() + Duration::from_secs(5);
        let pty_path = loop {
            if let Ok(text) = std::fs::read_to_string(&pty_file) {
                if !text.trim().is_empty() {
                    break text.trim().to_string();
                }
            }
            assert!(
                Instant::now() < deadline,
                "simulator never published its pty"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        Some(Sim {
            child,
            dir,
            pty_path,
        })
    }

    fn control(&mut self, op: Value) {
        let stdin = self.child.stdin.as_mut().expect("piped stdin");
        writeln!(stdin, "{op}").expect("write op");
        stdin.flush().expect("flush op");
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(self.dir.join("sim.log")).unwrap_or_default()
    }

    /// 拔線：模擬器整個消失（host 端看到的就是「裝置不見了」）。
    fn unplug(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.dir.starts_with(std::env::temp_dir()) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

fn spec_json(pty: &str) -> Value {
    let serial = json!({
        "port": pty,
        "baud": 115200,
        "expectedDeviceId": DEVICE_ID,
        "pairingCode": PAIRING_CODE,
    });
    json!({
        "schemaVersion": "1",
        "id": SPEC_ID,
        "displayName": "ESP32 序列模擬器（fixture）",
        "capabilities": [
            {
                "kind": "receptor",
                "id": "presence",
                "name": "距離感測",
                "category": "environment",
                "transport": "serial",
                "serial": serial,
                "pollIntervalMs": 3_600_000,
                "facts": {"distanceMm": "/facts/distanceMm"},
                // 需要 consent ＝ 這一族的高風險受器（停止路徑必須涵蓋它）。
                "requiresConsent": true,
            },
            {
                "kind": "actuator",
                "id": "led",
                "channel": "light",
                "transport": "serial",
                "serial": serial,
                "command": {"name": "led.set", "params": {"r": 255, "g": 0, "b": 0}},
            },
        ],
    })
}

async fn start_runtime(home: &tempfile::TempDir) -> Runtime {
    Runtime::start(RuntimeOptions {
        home: Some(home.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .expect("runtime")
}

/// 有界輪詢：條件成立就回 true，逾時回 false（絕不無限等待）。
async fn wait_until<F, Fut>(timeout: Duration, mut probe: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if probe().await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn members(rt: &Runtime) -> Vec<Value> {
    rt.character_session_diagnostics_value()
        .expect("diagnostics")
        .get("members")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn has_device_member(rt: &Runtime) -> bool {
    members(rt)
        .iter()
        .any(|m| m["party"]["kind"] == "device" && m["party"]["id"] == DEVICE_ID)
}

fn revision(rt: &Runtime) -> u64 {
    rt.character_session_diagnostics_value()
        .expect("diagnostics")["revision"]
        .as_u64()
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 綁定：capability 協商 → 成員；touch → shared state
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_serial_device_joins_the_session_and_its_touch_changes_the_shared_state() {
    let Some(mut sim) = Sim::spawn() else { return };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    let spec = serde_json::from_value(spec_json(&sim.pty_path)).expect("spec");
    rt.register_declarative_spec(&spec).await.expect("register");

    // 協商：模擬器送 capability → runtime 必須回 capability＋snapshot。
    assert!(
        wait_until(Duration::from_secs(10), || {
            sim.control(json!({"op": "aip-capability"}));
            let rt = rt.clone();
            async move { has_device_member(&rt) }
        })
        .await,
        "Serial 模擬器必須經宣告式 adapter 成為 session 成員；members={:?}\nsim log:\n{}",
        members(&rt),
        sim.log_text()
    );
    // 協商回覆必須真的**走回序列線**（`>>` ＝模擬器收到的行）。
    assert!(
        wait_until(Duration::from_secs(5), || {
            let log = sim.log_text();
            async move { log.contains(">> {\"type\":\"aip\"") }
        })
        .await,
        "協商回覆必須真的走回序列線：\n{}",
        sim.log_text()
    );

    // 觸碰：shared state 必須改變（revision 前進）。
    let before = revision(&rt);
    sim.control(json!({"op": "aip-touch", "kind": "tap"}));
    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            async move { revision(&rt) > before }
        })
        .await,
        "touch 必須改變 shared state（revision {before} 沒有前進）\nsim log:\n{}",
        sim.log_text()
    );
}

// ---------------------------------------------------------------------------
// 能力宣告與感測停止
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_spec_declares_its_capabilities_and_registers_a_stop_path() {
    let Some(sim) = Sim::spawn() else { return };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    let spec = serde_json::from_value(spec_json(&sim.pty_path)).expect("spec");
    rt.register_declarative_spec(&spec).await.expect("register");

    let declaration = rt
        .capability_declarations()
        .declaration(PROVIDER_ID)
        .expect("宣告式 adapter 必須向核心宣告自己的能力");
    assert!(
        declaration.receptors.iter().any(|r| r == RECEPTOR_ID),
        "{declaration:?}"
    );
    assert_eq!(
        declaration.high_risk_receptors,
        vec![RECEPTOR_ID.to_string()],
        "requiresConsent 的受器＝這一族的高風險受器"
    );
    assert!(
        rt.sensor_source_ids()
            .await
            .iter()
            .any(|s| s == PROVIDER_ID),
        "宣告式裝置必須登記一個 SensorSource：{:?}",
        rt.sensor_source_ids().await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_all_sensors_gets_a_real_ack_from_the_serial_simulator() {
    let Some(sim) = Sim::spawn() else { return };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    let spec = serde_json::from_value(spec_json(&sim.pty_path)).expect("spec");
    rt.register_declarative_spec(&spec).await.expect("register");

    // consent-gated 受器預設關閉：使用者啟用它之後才有東西可停。
    rt.registry
        .set_receptor_enabled(&ReceptorId::new(RECEPTOR_ID), true)
        .await
        .expect("enable receptor");

    let report = rt.stop_all_sensors("user").await.expect("stop all");
    let value = serde_json::to_value(&report).expect("report json");
    let sources = value["sources"].as_array().cloned().unwrap_or_default();
    let ours = sources
        .iter()
        .find(|d| d["sourceId"] == PROVIDER_ID || d["sourceId"] == DEVICE_ID)
        .unwrap_or_else(|| panic!("停止報告必須涵蓋這個 adapter：{value}"));
    assert_eq!(
        ours["outcome"],
        "stopped",
        "模擬器有回 stop-all ack，必須是真實的 confirmed：{value}\nsim log:\n{}",
        sim.log_text()
    );
    assert!(
        ours["sensors"]
            .as_array()
            .is_some_and(|s| s.iter().any(|v| v == RECEPTOR_ID)),
        "報告必須說得出涵蓋哪些受器：{value}"
    );

    // no-stop-path 不得再出現在這個 adapter 的受器上。
    let audit = rt.store.audit_tail(50).expect("audit");
    let no_path = audit.iter().any(|row| {
        row.get("kind").and_then(Value::as_str) == Some("sensor.stop-not-requested")
            && row["detail"]["receptors"]
                .as_array()
                .is_some_and(|r| r.iter().any(|v| v == RECEPTOR_ID))
    });
    assert!(!no_path, "這個 adapter 的受器不得再落到 no-stop-path");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_device_makes_the_stop_uncertain_not_stopped() {
    let Some(mut sim) = Sim::spawn() else { return };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    let spec = serde_json::from_value(spec_json(&sim.pty_path)).expect("spec");
    rt.register_declarative_spec(&spec).await.expect("register");
    rt.registry
        .set_receptor_enabled(&ReceptorId::new(RECEPTOR_ID), true)
        .await
        .expect("enable receptor");

    sim.unplug();

    let report = rt.stop_all_sensors("user").await.expect("stop all");
    let value = serde_json::to_value(&report).expect("report json");
    let sources = value["sources"].as_array().cloned().unwrap_or_default();
    let ours = sources
        .iter()
        .find(|d| d["sourceId"] == PROVIDER_ID || d["sourceId"] == DEVICE_ID)
        .unwrap_or_else(|| panic!("停止報告必須涵蓋這個 adapter：{value}"));
    assert_ne!(
        ours["outcome"], "stopped",
        "沒有 ack 就不得宣稱已停：{value}"
    );
    assert!(
        value["stopped"] == json!(false),
        "整體報告不得宣稱全部停止：{value}"
    );
}

// ---------------------------------------------------------------------------
// 撤銷：leave＋retract＋unregister，且不影響其他成員
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn revoking_the_spec_leaves_the_session_without_touching_other_members() {
    let Some(mut sim) = Sim::spawn() else { return };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    let spec = serde_json::from_value(spec_json(&sim.pty_path)).expect("spec");
    rt.register_declarative_spec(&spec).await.expect("register");

    // 另一個成員：桌面（可信 host surface）＋一台**程序內 fixture 裝置**
    // （不是 fake_iphone 程序、不是真機）。
    let announcement = serde_json::from_value(json!({
        "specVersions": ["aip/1.0"],
        "role": "remote-renderer",
        "profiles": ["character-session"],
        "syncClasses": ["semantic"],
        "intents": ["idle"],
        "inputs": ["character.interaction.touch"],
    }))
    .expect("announcement");
    rt.character_session_join(Party::device("iphone-fixture"), &announcement)
        .await
        .expect("fixture device joins");

    assert!(
        wait_until(Duration::from_secs(10), || {
            sim.control(json!({"op": "aip-capability"}));
            let rt = rt.clone();
            async move { has_device_member(&rt) }
        })
        .await,
        "Serial 模擬器必須先加入：{:?}",
        members(&rt)
    );

    rt.revoke_provider(&ProviderId::new(PROVIDER_ID))
        .await
        .expect("revoke");

    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            async move { !has_device_member(&rt) }
        })
        .await,
        "撤銷之後這台裝置必須離開 session：{:?}",
        members(&rt)
    );
    assert!(
        rt.capability_declarations()
            .declaration(PROVIDER_ID)
            .is_none(),
        "撤銷之後能力宣告必須撤回"
    );
    assert!(
        !rt.sensor_source_ids()
            .await
            .iter()
            .any(|s| s == PROVIDER_ID),
        "撤銷之後 SensorSource 必須解除登記：{:?}",
        rt.sensor_source_ids().await
    );
    // 其他成員完全不受影響。
    assert!(
        members(&rt)
            .iter()
            .any(|m| m["party"]["id"] == "iphone-fixture"),
        "撤銷第二 adapter 不得影響其他裝置成員：{:?}",
        members(&rt)
    );
}

// ---------------------------------------------------------------------------
// 斷線：presence reconnecting → tick 轉 offline（成員保留）
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn unplugging_the_device_marks_it_reconnecting_then_offline() {
    let Some(mut sim) = Sim::spawn() else { return };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    let spec = serde_json::from_value(spec_json(&sim.pty_path)).expect("spec");
    rt.register_declarative_spec(&spec).await.expect("register");

    assert!(
        wait_until(Duration::from_secs(10), || {
            sim.control(json!({"op": "aip-capability"}));
            let rt = rt.clone();
            async move { has_device_member(&rt) }
        })
        .await,
        "先要成為成員：{:?}",
        members(&rt)
    );

    sim.unplug();

    let presence_of = |rt: Runtime| async move {
        members(&rt)
            .iter()
            .find(|m| m["party"]["id"] == DEVICE_ID)
            .map(|m| m["presence"].as_str().unwrap_or_default().to_string())
    };
    assert!(
        wait_until(Duration::from_secs(15), || {
            let rt = rt.clone();
            async move { presence_of(rt).await.as_deref() == Some("reconnecting") }
        })
        .await,
        "斷線必須是 reconnecting（成員保留），不是憑空消失：{:?}",
        members(&rt)
    );

    // 既有 tick 負責把它轉成 offline，再過一段才 leave——這裡不另造第二條
    // 逾時路徑，只是把時鐘往前推。
    let presence_timeout = chrono::Duration::milliseconds(46_000);
    rt.character_session_tick_at(chrono::Utc::now() + presence_timeout)
        .await;
    assert_eq!(
        presence_of(rt.clone()).await.as_deref(),
        Some("offline"),
        "逾時之後必須是 offline（成員仍在名單上）：{:?}",
        members(&rt)
    );
    // 再久一點（2 倍 presence 逾時）才真的離開名單——幽靈成員不得永遠留著。
    rt.character_session_tick_at(chrono::Utc::now() + chrono::Duration::minutes(30))
        .await;
    assert!(
        presence_of(rt.clone()).await.is_none(),
        "離線太久之後必須離開名單：{:?}",
        members(&rt)
    );
    // 身分強度：Serial 的身分弱於已配對 iPhone，稽核裡必須說得出來。
    let audit = rt.store.audit_tail(100).expect("audit");
    assert!(
        audit.iter().any(|row| {
            row["detail"]["identityStrength"] == "transport-hello+device-side-pairing"
        }),
        "Serial／MQTT 的身分強度必須誠實留痕，不得沿用「已驗證身分」"
    );
}

// ---------------------------------------------------------------------------
// 誠實：送不出去的 session 回覆不得靜默消失
// ---------------------------------------------------------------------------

/// 協商會產生兩則回覆（capability＋snapshot）。snapshot envelope 目前**超過**
/// 參考韌體的單行上限（`serial::MAX_LINE_BYTES` = 639 bytes），所以 serial 傳輸
/// 在寫上線之前就誠實拒絕它。
///
/// 這條測試釘住的是「拒絕不得靜默」：送不出去的 envelope 必須留稽核（帶原因），
/// 否則桌面會看到一個已經加入的成員，而那台裝置其實從來沒有收到任何狀態——
/// 「已同步」與「什麼都沒收到」在畫面上長得一模一樣。
#[tokio::test(flavor = "multi_thread")]
async fn a_session_reply_that_does_not_fit_the_wire_is_audited_not_silently_dropped() {
    let Some(mut sim) = Sim::spawn() else { return };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    let spec = serde_json::from_value(spec_json(&sim.pty_path)).expect("spec");
    rt.register_declarative_spec(&spec).await.expect("register");

    assert!(
        wait_until(Duration::from_secs(10), || {
            sim.control(json!({"op": "aip-capability"}));
            let rt = rt.clone();
            async move { has_device_member(&rt) }
        })
        .await,
        "先要成為成員：{:?}",
        members(&rt)
    );

    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            async move {
                rt.store
                    .audit_tail(200)
                    .unwrap_or_default()
                    .iter()
                    .any(|row| {
                        row.get("kind").and_then(Value::as_str)
                            == Some("aip.outbound-undeliverable")
                            && row["detail"]["deviceId"] == DEVICE_ID
                    })
            }
        })
        .await,
        "送不出去的 session 回覆必須留稽核（帶原因），不得只落一行 debug log；\naudit={:?}",
        rt.store
            .audit_tail(30)
            .unwrap_or_default()
            .iter()
            .map(|r| r["kind"].clone())
            .collect::<Vec<_>>()
    );
}
