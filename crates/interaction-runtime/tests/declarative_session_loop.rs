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

const PAIRING_CODE: &str = "9927";

/// 為什麼每一支測試都要有**自己的** spec id：`register_provider_links`／
/// `shutdown_provider_links` 是 adapter 引擎裡的**行程層**表，鍵是 provider id
/// （production 一個 daemon 一份，所以那裡不會撞）。這個測試檔在**同一個行程**
/// 裡同時跑 7 個 Runtime；共用一個 spec id 時，後註冊的測試會覆蓋前一個的
/// 連線登記，而 `revoke_provider` 會把**別支測試**還在用的序列線 shutdown()
/// 掉——被害者的 `DeviceBinding` 看到 `LinkReadiness::Closed` 就收工，`who`
/// 一次都沒送出去，於是「成為成員」在 10 秒內永遠不成立。
/// （實測：預設並行下每 8 次約 3 次失敗，失敗的測試每次不同；模擬器 log 是
/// 全空的 `<<`——host 從來沒開口。）
static SPEC_SEQ: AtomicU64 = AtomicU64::new(0);

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

/// ESP32 序列【模擬器】子程序（pty）。
struct Sim {
    child: Child,
    dir: PathBuf,
    pty_path: String,
}

impl Sim {
    fn spawn(device_id: &str) -> Option<Sim> {
        if !python3_available() {
            eprintln!("python3 unavailable; skipping declarative session loop test");
            return None;
        }
        let dir =
            std::env::temp_dir().join(format!("decl-session-{}-{device_id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let pty_file = dir.join("pty");
        let child = Command::new("python3")
            .arg(repo_root().join("scripts/esp32-serial-sim.py"))
            .arg("--device-id")
            .arg(device_id)
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

/// 一支測試的完整器具：一台【模擬器】＋一組**只屬於這支測試**的識別字。
///
/// 識別字不共用，是因為 adapter 引擎的 provider→連線表是行程層的（見
/// [`SPEC_SEQ`]）。順帶也讓 audit／diagnostics 的斷言只可能看到自己的裝置。
struct Fixture {
    sim: Sim,
    spec_id: String,
    provider_id: String,
    receptor_id: String,
    device_id: String,
}

impl Fixture {
    fn spawn() -> Option<Fixture> {
        let seq = SPEC_SEQ.fetch_add(1, Ordering::SeqCst);
        let spec_id = format!("esp32sim{seq}");
        let device_id = format!("esp32-sim{seq:02}");
        let sim = Sim::spawn(&device_id)?;
        Some(Fixture {
            provider_id: format!("provider.adapter.{spec_id}"),
            receptor_id: format!("{spec_id}.presence"),
            spec_id,
            device_id,
            sim,
        })
    }

    fn spec(&self) -> interaction_adapter_declarative::DeclarativeSpec {
        serde_json::from_value(spec_json(
            &self.sim.pty_path,
            &self.spec_id,
            &self.device_id,
        ))
        .expect("spec")
    }

    fn log_text(&self) -> String {
        self.sim.log_text()
    }

    fn control(&mut self, op: Value) {
        self.sim.control(op)
    }

    fn unplug(&mut self) {
        self.sim.unplug()
    }

    fn members(&self, rt: &Runtime) -> Vec<Value> {
        members(rt)
    }

    fn has_device_member(&self, rt: &Runtime) -> bool {
        self.members(rt)
            .iter()
            .any(|m| m["party"]["kind"] == "device" && m["party"]["id"] == self.device_id)
    }
}

fn spec_json(pty: &str, spec_id: &str, device_id: &str) -> Value {
    let serial = json!({
        "port": pty,
        "baud": 115200,
        "expectedDeviceId": device_id,
        "pairingCode": PAIRING_CODE,
    });
    json!({
        "schemaVersion": "1",
        "id": spec_id,
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

/// 讓【模擬器】加入 session（重送 capability 直到 host 真的把它記成成員，
/// 或逾時）。回 false ＝逾時，呼叫端要自己 assert 並印出模擬器 log。
async fn join_session(fx: &mut Fixture, rt: &Runtime) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        fx.control(json!({"op": "aip-capability"}));
        if fx.has_device_member(rt) {
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

/// 有沒有「送不出去的 session 回覆」稽核（帶這台裝置的 id）。
fn has_undeliverable_audit(rt: &Runtime, device_id: &str) -> bool {
    rt.store
        .audit_tail(200)
        .unwrap_or_default()
        .iter()
        .any(|row| {
            row.get("kind").and_then(Value::as_str) == Some("aip.outbound-undeliverable")
                && row["detail"]["deviceId"] == device_id
        })
}

/// 某個裝置成員目前的 presence（不在名單上＝None）。
fn presence_of(rt: &Runtime, device_id: &str) -> Option<String> {
    members(rt)
        .iter()
        .find(|m| m["party"]["id"] == device_id)
        .map(|m| m["presence"].as_str().unwrap_or_default().to_string())
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
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");

    // 協商：模擬器送 capability → runtime 必須回 capability＋snapshot。
    assert!(
        join_session(&mut fx, &rt).await,
        "Serial 模擬器必須經宣告式 adapter 成為 session 成員；members={:?}\nsim log:\n{}",
        fx.members(&rt),
        fx.log_text()
    );
    // 協商回覆必須真的**走回序列線**（`>>` ＝模擬器收到的行）。
    assert!(
        wait_until(Duration::from_secs(5), || {
            let log = fx.log_text();
            async move { log.contains(">> {\"type\":\"aip\"") }
        })
        .await,
        "協商回覆必須真的走回序列線：\n{}",
        fx.log_text()
    );

    // 觸碰：shared state 必須改變（revision 前進）。
    let before = revision(&rt);
    fx.control(json!({"op": "aip-touch", "kind": "tap"}));
    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            async move { revision(&rt) > before }
        })
        .await,
        "touch 必須改變 shared state（revision {before} 沒有前進）\nsim log:\n{}",
        fx.log_text()
    );
}

// ---------------------------------------------------------------------------
// 能力宣告與感測停止
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_spec_declares_its_capabilities_and_registers_a_stop_path() {
    let Some(fx) = Fixture::spawn() else { return };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");

    let declaration = rt
        .capability_declarations()
        .declaration(&fx.provider_id)
        .expect("宣告式 adapter 必須向核心宣告自己的能力");
    assert!(
        declaration.receptors.contains(&fx.receptor_id),
        "{declaration:?}"
    );
    assert_eq!(
        declaration.high_risk_receptors,
        vec![fx.receptor_id.clone()],
        "requiresConsent 的受器＝這一族的高風險受器"
    );
    assert!(
        rt.sensor_source_ids().await.contains(&fx.provider_id),
        "宣告式裝置必須登記一個 SensorSource：{:?}",
        rt.sensor_source_ids().await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_all_sensors_gets_a_real_ack_from_the_serial_simulator() {
    let Some(fx) = Fixture::spawn() else { return };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");

    // consent-gated 受器預設關閉：使用者啟用它之後才有東西可停。
    rt.registry
        .set_receptor_enabled(&ReceptorId::new(&fx.receptor_id), true)
        .await
        .expect("enable receptor");

    let report = rt.stop_all_sensors("user").await.expect("stop all");
    let value = serde_json::to_value(&report).expect("report json");
    let sources = value["sources"].as_array().cloned().unwrap_or_default();
    let ours = sources
        .iter()
        .find(|d| d["sourceId"] == fx.provider_id || d["sourceId"] == fx.device_id)
        .unwrap_or_else(|| panic!("停止報告必須涵蓋這個 adapter：{value}"));
    assert_eq!(
        ours["outcome"],
        "stopped",
        "模擬器有回 stop-all ack，必須是真實的 confirmed：{value}\nsim log:\n{}",
        fx.log_text()
    );
    assert!(
        ours["sensors"]
            .as_array()
            .is_some_and(|s| s.iter().any(|v| *v == fx.receptor_id)),
        "報告必須說得出涵蓋哪些受器：{value}"
    );

    // no-stop-path 不得再出現在這個 adapter 的受器上。
    let audit = rt.store.audit_tail(50).expect("audit");
    let no_path = audit.iter().any(|row| {
        row.get("kind").and_then(Value::as_str) == Some("sensor.stop-not-requested")
            && row["detail"]["receptors"]
                .as_array()
                .is_some_and(|r| r.iter().any(|v| *v == fx.receptor_id))
    });
    assert!(!no_path, "這個 adapter 的受器不得再落到 no-stop-path");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_device_makes_the_stop_uncertain_not_stopped() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");
    rt.registry
        .set_receptor_enabled(&ReceptorId::new(&fx.receptor_id), true)
        .await
        .expect("enable receptor");

    fx.unplug();

    let report = rt.stop_all_sensors("user").await.expect("stop all");
    let value = serde_json::to_value(&report).expect("report json");
    let sources = value["sources"].as_array().cloned().unwrap_or_default();
    let ours = sources
        .iter()
        .find(|d| d["sourceId"] == fx.provider_id || d["sourceId"] == fx.device_id)
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

/// 感測不靜默：拔線後停不掉的受器**不得**從 `activeSensors` 消失。
/// 旗標翻掉只代表「本機這一側不再收資料」，不代表裝置停了擷取——沒有
/// 明確確認之前，那一筆必須以 `stop-unknown` 留在清單上。
#[tokio::test(flavor = "multi_thread")]
async fn a_silent_device_stays_visible_as_stop_unknown() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");
    rt.registry
        .set_receptor_enabled(&ReceptorId::new(&fx.receptor_id), true)
        .await
        .expect("enable receptor");

    fx.unplug();
    let report = rt.stop_all_sensors("user").await.expect("stop all");
    let value = serde_json::to_value(&report).expect("report json");
    assert!(
        value["stopped"] == json!(false),
        "沒有確認就不得宣稱全部停止：{value}"
    );

    let active = rt.active_sensors_all().await;
    let ours = active
        .iter()
        .find(|s| s.kind == fx.receptor_id)
        .unwrap_or_else(|| {
            panic!(
                "已要求停止但沒拿到確認的受器不得從 activeSensors 消失：{:?}",
                active
            )
        });
    assert_eq!(
        ours.state,
        interaction_runtime::sensors::SENSOR_STATE_STOP_UNKNOWN,
        "沒有確認就只能誠實標 stop-unknown：{ours:?}"
    );
}

/// 孤兒安全網：來源被移除（撤銷）時它還「可能在擷取」，那一筆必須留成
/// 有界可見的 stop-unknown ＋稽核，而不是靜靜消失。
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_silent_device_leaves_an_orphaned_capture_record() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");
    rt.registry
        .set_receptor_enabled(&ReceptorId::new(&fx.receptor_id), true)
        .await
        .expect("enable receptor");

    fx.unplug();
    rt.revoke_provider(&ProviderId::new(&fx.provider_id))
        .await
        .expect("revoke");

    let active = rt.active_sensors_all().await;
    let ours = active
        .iter()
        .find(|s| s.kind == fx.receptor_id)
        .unwrap_or_else(|| panic!("來源被移除時還可能在擷取：那一筆不得靜默消失：{:?}", active));
    assert_eq!(
        ours.state,
        interaction_runtime::sensors::SENSOR_STATE_STOP_UNKNOWN,
        "{ours:?}"
    );
    assert!(
        rt.store
            .audit_tail(200)
            .unwrap_or_default()
            .iter()
            .any(
                |row| row["kind"] == json!("sensor.source-removed-while-capturing")
                    && row["detail"]["sourceId"] == json!(fx.provider_id)
            ),
        "孤兒紀錄必須留稽核"
    );
}

// ---------------------------------------------------------------------------
// 撤銷：leave＋retract＋unregister，且不影響其他成員
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn revoking_the_spec_leaves_the_session_without_touching_other_members() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");

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
        join_session(&mut fx, &rt).await,
        "Serial 模擬器必須先加入：{:?}\nsim log:\n{}",
        fx.members(&rt),
        fx.log_text()
    );

    rt.revoke_provider(&ProviderId::new(&fx.provider_id))
        .await
        .expect("revoke");

    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            let device_id = fx.device_id.clone();
            async move { presence_of(&rt, &device_id).is_none() }
        })
        .await,
        "撤銷之後這台裝置必須離開 session：{:?}",
        fx.members(&rt)
    );
    assert!(
        rt.capability_declarations()
            .declaration(&fx.provider_id)
            .is_none(),
        "撤銷之後能力宣告必須撤回"
    );
    assert!(
        !rt.sensor_source_ids().await.contains(&fx.provider_id),
        "撤銷之後 SensorSource 必須解除登記：{:?}",
        rt.sensor_source_ids().await
    );
    // 其他成員完全不受影響。
    assert!(
        fx.members(&rt)
            .iter()
            .any(|m| m["party"]["id"] == "iphone-fixture"),
        "撤銷第二 adapter 不得影響其他裝置成員：{:?}",
        fx.members(&rt)
    );
}

// ---------------------------------------------------------------------------
// 斷線：presence reconnecting → tick 轉 offline（成員保留）
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn unplugging_the_device_marks_it_reconnecting_then_offline() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");

    assert!(
        join_session(&mut fx, &rt).await,
        "先要成為成員：{:?}\nsim log:\n{}",
        fx.members(&rt),
        fx.log_text()
    );

    fx.unplug();

    let device_id = fx.device_id.clone();
    assert!(
        wait_until(Duration::from_secs(15), || {
            let rt = rt.clone();
            let device_id = device_id.clone();
            async move { presence_of(&rt, &device_id).as_deref() == Some("reconnecting") }
        })
        .await,
        "斷線必須是 reconnecting（成員保留），不是憑空消失：{:?}",
        fx.members(&rt)
    );

    // 既有 tick 負責把它轉成 offline，再過一段才 leave——這裡不另造第二條
    // 逾時路徑，只是把時鐘往前推。
    let presence_timeout = chrono::Duration::milliseconds(46_000);
    rt.character_session_tick_at(chrono::Utc::now() + presence_timeout)
        .await;
    assert_eq!(
        presence_of(&rt, &device_id).as_deref(),
        Some("offline"),
        "逾時之後必須是 offline（成員仍在名單上）：{:?}",
        fx.members(&rt)
    );
    // 再久一點（2 倍 presence 逾時）才真的離開名單——幽靈成員不得永遠留著。
    rt.character_session_tick_at(chrono::Utc::now() + chrono::Duration::minutes(30))
        .await;
    assert!(
        presence_of(&rt, &device_id).is_none(),
        "離線太久之後必須離開名單：{:?}",
        fx.members(&rt)
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
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");

    assert!(
        join_session(&mut fx, &rt).await,
        "先要成為成員：{:?}\nsim log:\n{}",
        fx.members(&rt),
        fx.log_text()
    );

    let device_id = fx.device_id.clone();
    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            let device_id = device_id.clone();
            async move {
                rt.store
                    .audit_tail(200)
                    .unwrap_or_default()
                    .iter()
                    .any(|row| {
                        row.get("kind").and_then(Value::as_str)
                            == Some("aip.outbound-undeliverable")
                            && row["detail"]["deviceId"] == device_id
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

// ---------------------------------------------------------------------------
// 廣播：其他成員造成的 shared state 變更必須真的到達這台裝置
// ---------------------------------------------------------------------------

/// 一則桌面（可信 host surface）的能力宣告。
fn host_surface_announcement() -> interaction_aip::CapabilityAnnouncement {
    serde_json::from_value(json!({
        "specVersions": ["aip/1.0"],
        "role": "host-renderer",
        "profiles": ["character-session"],
        "syncClasses": ["semantic"],
        "intents": ["idle"],
        "inputs": ["character.interaction.touch"],
    }))
    .expect("announcement")
}

/// 桌面送出的一則觸碰（身分＝可信 host surface，不是裝置）。
fn desktop_touch_envelope(message_id: &str) -> interaction_aip::Envelope {
    serde_json::from_value(json!({
        "specVersion": "aip/1.0",
        "messageId": message_id,
        "messageType": "event",
        "name": "character.interaction.touch",
        "source": {"kind": "human-surface", "id": "desktop"},
        "sessionId": "session.home",
        "occurredAt": chrono::Utc::now().to_rfc3339(),
        "expiresAt": (chrono::Utc::now() + chrono::Duration::seconds(5)).to_rfc3339(),
        "payload": {"kind": "tap", "intensity": 0.6},
    }))
    .expect("envelope")
}

/// 模擬器實際**收到**（`>>` ＝ host→裝置）且含有 `needle` 的行數。
fn received_lines(log: &str, needle: &str) -> usize {
    log.lines()
        .filter(|line| line.starts_with(">> ") && line.contains(needle))
        .count()
}

/// 最近一則「state patch 送不出去」的稽核。
fn undeliverable_patch(rt: &Runtime) -> Option<Value> {
    rt.store
        .audit_tail(200)
        .unwrap_or_default()
        .into_iter()
        .find(|row| {
            row.get("kind").and_then(Value::as_str) == Some("aip.outbound-undeliverable")
                && row["detail"]["name"] == "character.session.patch"
        })
}

/// 別的成員（桌面）造成的 shared state 變更走 `Output::Broadcast`／`Output::Send`，
/// 兩條都經 `character_session_send`。在此之前那條路徑對 `PartyKind::Device`
/// 硬編 iPhone 出站，所以宣告式裝置只收得到「對自己那則 frame 的直接回覆」——
/// 桌面顯示「已加入、已同步」，裝置其實從第一秒起就沒再收到任何狀態。
///
/// 誠實：這條線的單行上限是參考韌體的 639 bytes（`serial::MAX_LINE_BYTES`）。
/// 成員名單改變造成的 `state{kind:"patch"}`（實測 660–784 bytes）**放不進去**，
/// 那是真的送不到，必須以 `aip.outbound-undeliverable` 留痕——本測試同時釘住
/// 「送得到的真的送到了」與「送不到的沒有被靜默吞掉」，不為了綠燈放寬上限。
#[tokio::test(flavor = "multi_thread")]
async fn shared_state_changes_from_other_members_reach_the_serial_device() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");
    assert!(
        join_session(&mut fx, &rt).await,
        "先要成為成員：{:?}\nsim log:\n{}",
        fx.members(&rt),
        fx.log_text()
    );

    // 誠實：協商的 snapshot 超過單行上限，必須留稽核。
    let device_id = fx.device_id.clone();
    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            let device_id = device_id.clone();
            async move { has_undeliverable_audit(&rt, &device_id) }
        })
        .await,
        "送不出去的 snapshot 仍然必須留稽核"
    );

    // 1) 桌面（可信 host surface）加入並觸碰：它造成的行為意圖必須真的走回
    //    序列線給這台裝置。
    rt.character_session_join(
        interaction_runtime::character_session::desktop_party(),
        &host_surface_announcement(),
    )
    .await
    .expect("desktop joins");
    rt.character_session_submit(
        desktop_touch_envelope("fx-desktop-touch-1"),
        &interaction_runtime::character_session::desktop_party(),
    )
    .await
    .expect("submit");
    assert!(
        wait_until(Duration::from_secs(5), || {
            let log = fx.log_text();
            async move { received_lines(&log, "\"character.behavior.request\"") > 0 }
        })
        .await,
        "桌面觸碰造成的行為意圖必須真的經序列線到達這台裝置\nsim log:\n{}",
        fx.log_text()
    );
    // 同一次觸碰造成的 state patch 放不進 639 bytes：誠實留痕，不放寬上限。
    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            async move { undeliverable_patch(&rt).is_some() }
        })
        .await,
        "放不進單行上限的 state patch 必須留稽核（不得靜默丟棄）：{:?}",
        rt.store.audit_tail(50).unwrap_or_default()
    );
    let refused = undeliverable_patch(&rt).expect("patch audit");
    assert_eq!(
        refused["detail"]["transport"], "serial",
        "稽核必須說得出是哪一條線送不出去：{refused}"
    );

    // 2) 放得進單行上限的 shared state 廣播必須**真的**到達：緊急停止造成的
    //    真相變更（實測 450 bytes）走同一條 `Output::Broadcast` 路徑。
    let before = received_lines(&fx.log_text(), "\"kind\":\"patch\"");
    rt.emergency_stop("test", None).await.expect("estop");
    assert!(
        wait_until(Duration::from_secs(5), || {
            let log = fx.log_text();
            async move { received_lines(&log, "\"kind\":\"patch\"") > before }
        })
        .await,
        "shared state 的廣播必須真的經序列線到達這台裝置（patches before={before}）\
         \nsim log:\n{}",
        fx.log_text()
    );
}

/// 稽核的 `transport` 必須說得出**這一則**是從哪條線進來的。寫死 `iphone`
/// 會讓「有人從序列線偽造身分」在稽核上長得像「某台 iPhone 出問題」。
#[tokio::test(flavor = "multi_thread")]
async fn an_identity_mismatch_on_the_serial_line_is_audited_as_serial() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");
    assert!(
        join_session(&mut fx, &rt).await,
        "先要成為成員：{:?}\nsim log:\n{}",
        fx.members(&rt),
        fx.log_text()
    );

    // 偽造身分：這條線宣稱自己是另一台裝置。
    fx.control(json!({
        "op": "aip-touch",
        "kind": "tap",
        "source": {"kind": "device", "id": "someone-else"},
    }));

    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            async move {
                rt.store
                    .audit_tail(200)
                    .unwrap_or_default()
                    .iter()
                    .any(|row| {
                        row.get("kind").and_then(Value::as_str) == Some("aip.identity-mismatch")
                            && row["detail"]["transport"] == "serial"
                    })
            }
        })
        .await,
        "序列線的身分不符必須記成 transport=serial：{:?}",
        rt.store
            .audit_tail(50)
            .unwrap_or_default()
            .iter()
            .filter(|r| r["kind"] == "aip.identity-mismatch")
            .cloned()
            .collect::<Vec<_>>()
    );
}

/// diagnostics 必須說得出每個成員的身分是**怎麼來的**：桌面是 host surface、
/// 宣告式裝置只有傳輸層 hello、沒有出站通道的成員則**省略**這個欄位（不猜）。
#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_report_where_each_member_identity_came_from() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");
    assert!(
        join_session(&mut fx, &rt).await,
        "先要成為成員：{:?}\nsim log:\n{}",
        fx.members(&rt),
        fx.log_text()
    );
    rt.character_session_join(
        interaction_runtime::character_session::desktop_party(),
        &host_surface_announcement(),
    )
    .await
    .expect("desktop joins");
    // 沒有出站通道的**程序內 fixture 裝置**（不是真機、也不是模擬器）。
    rt.character_session_join(
        Party::device("fixture-without-a-channel"),
        &host_surface_announcement(),
    )
    .await
    .expect("fixture joins");

    let members = fx.members(&rt);
    let find = |id: &str| {
        members
            .iter()
            .find(|m| m["party"]["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("成員 {id} 不在名單上：{members:?}"))
    };
    assert_eq!(
        find(&fx.device_id)["identityStrength"],
        json!("transport-hello+device-side-pairing"),
        "宣告式裝置的身分只有傳輸層 hello＋裝置端配對，不得沿用已配對 iPhone 的強度"
    );
    assert_eq!(
        find("desktop")["identityStrength"],
        json!("host-surface"),
        "桌面是可信 host surface（human token 綁定出來的身分）"
    );
    assert!(
        find("fixture-without-a-channel")
            .get("identityStrength")
            .is_none(),
        "查不到出站通道就**省略**這個欄位——不猜、也不冒充已驗證：{:?}",
        find("fixture-without-a-channel")
    );
}

/// 撤銷之後這台裝置的出站通道必須從表上消失：留著等於之後每一則廣播都往
/// 一條已經關掉的線上送。
#[tokio::test(flavor = "multi_thread")]
async fn revoking_the_spec_removes_its_outbound_channel() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");
    assert!(
        join_session(&mut fx, &rt).await,
        "先要成為成員：{:?}\nsim log:\n{}",
        fx.members(&rt),
        fx.log_text()
    );
    assert!(
        rt.device_outbound_ids().contains(&fx.device_id),
        "握手成立的裝置必須登記成一條出站通道：{:?}",
        rt.device_outbound_ids()
    );

    rt.revoke_provider(&ProviderId::new(&fx.provider_id))
        .await
        .expect("revoke");

    assert!(
        !rt.device_outbound_ids().contains(&fx.device_id),
        "撤銷之後出站表不得還留著它：{:?}",
        rt.device_outbound_ids()
    );
}

/// 宣告式裝置**第一次** capability（還不是成員）就帶錯 `sessionId`：§8 的 session-binding
/// 這一關擋下它時，稽核的 `transport` 必須說「serial」——這條路徑以前寫死 `iphone`，
/// 一台序列裝置設定錯了會在稽核上長得像「某台 iPhone 出問題」。
/// （這支測試寫在修正之後：缺陷由 D2 的獨立驗證者以程式碼閱讀確認；紅燈以突變驗證——
/// 把該處稽核改回寫死 `iphone` 時本測試失敗。）
#[tokio::test(flavor = "multi_thread")]
async fn a_capability_from_another_session_on_the_serial_line_is_audited_as_serial() {
    let Some(mut fx) = Fixture::spawn() else {
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.register_declarative_spec(&fx.spec())
        .await
        .expect("register");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seq = 0u32;
    let rejected = loop {
        seq += 1;
        // 與模擬器的 aip-capability 相同的 envelope，只有 sessionId 指向別的 session。
        fx.control(json!({
            "op": "aip-raw",
            "envelope": {
                "specVersion": "aip/1.0",
                "messageId": format!("fx-wrong-session-cap-{seq}"),
                "messageType": "capability",
                "name": "character.session.capability",
                "source": {"kind": "device", "id": fx.device_id},
                "sessionId": "session.somewhere-else",
                "occurredAt": chrono::Utc::now().to_rfc3339(),
                "payload": {
                    "specVersions": ["aip/1.0"],
                    "role": "remote-renderer",
                    "profiles": ["character-session"],
                    "syncClasses": ["semantic"],
                    "intents": ["idle"],
                    "inputs": ["character.interaction.touch"],
                },
            },
        }));
        let hit = rt
            .store
            .audit_tail(200)
            .unwrap_or_default()
            .into_iter()
            .find(|row| {
                row.get("kind").and_then(Value::as_str) == Some("aip.rejected")
                    && row["detail"]["stage"] == "session-binding"
            });
        if hit.is_some() {
            break hit;
        }
        if Instant::now() >= deadline {
            break None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let row = rejected.unwrap_or_else(|| {
        panic!(
            "另一個 session 的 capability 必須被擋在 session-binding；members={:?}\nsim log:\n{}",
            fx.members(&rt),
            fx.log_text()
        )
    });
    assert_eq!(
        row["detail"]["transport"],
        json!("serial"),
        "序列線的 session-binding 拒絕必須記成 transport=serial：{row:?}"
    );
    assert_eq!(
        row["detail"]["identityStrength"],
        json!("transport-hello+device-side-pairing"),
        "稽核要帶這條線的身分強度：{row:?}"
    );
    assert!(
        !fx.has_device_member(&rt),
        "帶錯 sessionId 不得 join：{:?}",
        fx.members(&rt)
    );
}

/// 沒有出站通道的裝置成員（從快照還原、或程序內 fixture）收不到任何廣播——
/// 這不能只是一行 debug log：桌面上「已加入」與「一則狀態都沒收到」長得一模一樣。
/// 不需要模擬器：這裡的成員是程序內 fixture。
#[tokio::test(flavor = "multi_thread")]
async fn a_member_without_an_outbound_channel_is_audited_when_a_broadcast_cannot_reach_it() {
    let home = tempfile::tempdir().unwrap();
    let rt = start_runtime(&home).await;
    rt.character_session_join(
        Party::device("fixture-without-a-channel"),
        &host_surface_announcement(),
    )
    .await
    .expect("fixture joins");
    rt.character_session_join(
        interaction_runtime::character_session::desktop_party(),
        &host_surface_announcement(),
    )
    .await
    .expect("desktop joins");

    // 一則所有成員都該收到的 shared state 變更（緊急停止的真相變更）。
    rt.emergency_stop("test", None).await.expect("estop");

    let no_channel = |rt: &Runtime| {
        rt.store
            .audit_tail(200)
            .unwrap_or_default()
            .into_iter()
            .find(|row| {
                row.get("kind").and_then(Value::as_str) == Some("aip.outbound-undeliverable")
                    && row["detail"]["reason"] == "no-channel"
                    && row["detail"]["deviceId"] == "fixture-without-a-channel"
            })
    };
    assert!(
        wait_until(Duration::from_secs(5), || {
            let rt = rt.clone();
            async move { no_channel(&rt).is_some() }
        })
        .await,
        "送不到沒有通道的成員必須留 aip.outbound-undeliverable{{reason:\"no-channel\"}}：{:?}",
        rt.store
            .audit_tail(50)
            .unwrap_or_default()
            .iter()
            .filter(|r| r["kind"] == "aip.outbound-undeliverable")
            .cloned()
            .collect::<Vec<_>>()
    );
    let row = no_channel(&rt).expect("audit row");
    assert!(
        row["detail"].get("transport").is_none(),
        "查不到通道就沒有 transport 可說——不猜：{row:?}"
    );
}
