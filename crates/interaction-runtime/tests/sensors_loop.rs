//! Sensor privacy integration: default-off, consent-gated listen windows,
//! visible indicators, estop stops capture, no silent path.

use interaction_core::*;
use interaction_runtime::{Runtime, RuntimeOptions};
use std::sync::Arc;

async fn runtime() -> (tempfile::TempDir, Runtime, Arc<adapters_media::FakeSource>) {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    // Inject the deterministic fake backend (tests never open a real mic).
    let fake = Arc::new(adapters_media::FakeSource::new(0.4));
    rt.mic_receptor.as_ref().unwrap().swap_source(fake.clone());
    (dir, rt, fake)
}

#[tokio::test]
async fn microphone_is_off_by_default_and_consent_gated() {
    let (_g, rt, fake) = runtime().await;

    // Registered DISABLED (requires consent → not enabled at registration).
    let snap = rt
        .capabilities(&DiscoveryContext {
            include_unavailable: true,
            ..Default::default()
        })
        .await;
    let mic = snap
        .receptors
        .iter()
        .find(|r| r.id.as_str() == "microphone.listen")
        .expect("mic listed");
    assert!(mic.requires_consent);
    assert_ne!(mic.availability, Availability::Available);

    // Even after enabling, listening needs an explicit session consent.
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("microphone.listen"), true)
        .await
        .unwrap();
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let err = rt.begin_mic_listen(5_000, "test").await.unwrap_err();
    assert!(matches!(err, DomainError::ConsentRequired(_)));
    assert!(
        !fake.started.load(std::sync::atomic::Ordering::SeqCst),
        "no capture without consent"
    );

    // Consent granted → the window opens and the indicator turns on.
    rt.grant_consent("receptor:microphone.listen", None)
        .await
        .unwrap();
    rt.begin_mic_listen(5_000, "test").await.unwrap();
    assert!(fake.started.load(std::sync::atomic::Ordering::SeqCst));
    let sensors = rt.active_sensors();
    assert_eq!(sensors.len(), 1);
    assert_eq!(sensors[0].kind, "microphone");
    assert!(
        sensors[0].auto_stop_at.is_some(),
        "hard deadline always set"
    );

    // Observations carry derived facts only.
    let obs = rt
        .observe_fresh(&ReceptorId::new("microphone.listen"))
        .await
        .unwrap();
    assert_eq!(obs.facts["listening"], serde_json::json!(true));
    assert!(obs.facts.contains_key("level"));
    assert!(!obs.facts.contains_key("audio"));

    // Manual stop releases the device and clears the indicator, and reports
    // honestly: local mic stopped, no phones involved, nothing uncertain.
    let report = rt.stop_all_sensors("test").await.unwrap();
    assert!(fake.stopped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(rt.active_sensors().is_empty());
    assert_eq!(report.local.microphone, "stopped");
    assert!(report.devices.is_empty());
    assert!(report.stopped, "沒有任何來源沒確認 → 確實停了");
    assert!(!report.uncertain);
    assert_eq!(rt.active_sensors_all().await.len(), 0);
}

/// 「停止所有感測」的 audit 必須看得出停了什麼（不再是空的 detail），
/// 而且本機本來就沒在擷取時要誠實說 `idle`，不得發假的 sensor.stopped。
#[tokio::test]
async fn stop_all_sensors_reports_and_audits_what_it_actually_stopped() {
    let (_g, rt, _fake) = runtime().await;

    let report = rt.stop_all_sensors("test").await.unwrap();
    assert_eq!(
        report.local.microphone, "idle",
        "本來就沒在擷取＝idle，不是 stopped"
    );
    assert!(report.stopped && !report.uncertain);
    let stopped_events = rt
        .events
        .recent(50)
        .into_iter()
        .filter(|e| e.event_type == EventType::SensorStopped)
        .count();
    assert_eq!(stopped_events, 0, "沒有東西在跑就不該發 sensor.stopped");

    let audit = rt
        .store
        .audit_tail(50)
        .unwrap()
        .into_iter()
        .rfind(|a| a["kind"] == serde_json::json!("sensor.stopped-all"))
        .expect("sensor.stopped-all audit");
    assert_eq!(audit["detail"]["local"]["microphone"], "idle");
    assert!(
        audit["detail"]["devices"].is_array(),
        "audit 要逐台列出結果：{audit}"
    );
    assert_eq!(audit["detail"]["uncertain"], serde_json::json!(false));

    // 有在擷取時 → stopped，並且確實發了一則 sensor.stopped。
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("microphone.listen"), true)
        .await
        .unwrap();
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.grant_consent("receptor:microphone.listen", None)
        .await
        .unwrap();
    rt.begin_mic_listen(5_000, "test").await.unwrap();
    let report = rt.stop_all_sensors("test").await.unwrap();
    assert_eq!(report.local.microphone, "stopped");
    let stopped_events = rt
        .events
        .recent(50)
        .into_iter()
        .filter(|e| e.event_type == EventType::SensorStopped)
        .count();
    assert_eq!(stopped_events, 1);
}

#[tokio::test]
async fn emergency_stop_halts_capture_immediately() {
    let (_g, rt, fake) = runtime().await;
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("microphone.listen"), true)
        .await
        .unwrap();
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.grant_consent("receptor:microphone.listen", None)
        .await
        .unwrap();
    rt.begin_mic_listen(20_000, "test").await.unwrap();
    assert_eq!(rt.active_sensors().len(), 1);

    rt.emergency_stop("test", Some("estop".into()))
        .await
        .unwrap();
    assert!(fake.stopped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        rt.active_sensors().is_empty(),
        "indicator cleared with capture"
    );

    // While stopped, new listen windows are refused.
    let err = rt.begin_mic_listen(5_000, "test").await.unwrap_err();
    assert!(err.to_string().contains("emergency stop"));
}

#[tokio::test]
async fn estop_while_capturing_leaves_no_silent_path() {
    let (_g, rt, fake) = runtime().await;
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("microphone.listen"), true)
        .await
        .unwrap();
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.grant_consent("receptor:microphone.listen", None)
        .await
        .unwrap();
    rt.begin_mic_listen(20_000, "test").await.unwrap();

    // The whole time capture is on, status reports it (no silent capture).
    let status = rt.status().await;
    let sensors = status["activeSensors"].as_array().unwrap();
    assert_eq!(sensors.len(), 1);
    assert_eq!(sensors[0]["kind"], "microphone");

    rt.emergency_stop("test", None).await.unwrap();
    let status = rt.status().await;
    assert!(status["activeSensors"].as_array().unwrap().is_empty());
    assert!(fake.stopped.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn revoking_receptor_consent_stops_capture_immediately() {
    let (_g, rt, fake) = runtime().await;
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("microphone.listen"), true)
        .await
        .unwrap();
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.grant_consent("receptor:microphone.listen", None)
        .await
        .unwrap();
    rt.begin_mic_listen(20_000, "test").await.unwrap();
    assert_eq!(rt.active_sensors().len(), 1);

    // Revoking the mic's consent must stop capture NOW (regression: previously
    // ConsentScope::Receptor fell through and the mic kept capturing).
    rt.revoke_consent("receptor:microphone.listen")
        .await
        .unwrap();
    assert!(fake.stopped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(rt.active_sensors().is_empty());
}

#[tokio::test]
async fn mic_facts_are_not_persisted_retention_none() {
    let (_g, rt, _fake) = runtime().await;
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("microphone.listen"), true)
        .await
        .unwrap();
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.grant_consent("receptor:microphone.listen", None)
        .await
        .unwrap();
    rt.begin_mic_listen(5_000, "test").await.unwrap();
    // observe_fresh returns derived facts live…
    let obs = rt
        .observe_fresh(&ReceptorId::new("microphone.listen"))
        .await
        .unwrap();
    assert_eq!(obs.facts["listening"], serde_json::json!(true));
    // …but nothing is written to the store (retention: none is honored).
    let stored = rt
        .observe_stored(&ObservationQuery {
            receptor_id: Some(ReceptorId::new("microphone.listen")),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        stored.is_empty(),
        "no-retention receptor must not persist observations"
    );
}

#[tokio::test]
async fn estop_during_start_aborts_the_window() {
    // Engage estop, then attempt to open a window: begin_mic_listen must refuse
    // (the estop gate plus the post-open re-check both hold).
    let (_g, rt, fake) = runtime().await;
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("microphone.listen"), true)
        .await
        .unwrap();
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.grant_consent("receptor:microphone.listen", None)
        .await
        .unwrap();
    rt.emergency_stop("test", None).await.unwrap();
    assert!(rt.begin_mic_listen(5_000, "test").await.is_err());
    assert!(!fake.started.load(std::sync::atomic::Ordering::SeqCst));
}

/// v0.5 Phase 7 對抗審查第三輪（清單 2 的本機端回歸）：
/// `status.activeSensors` 走的是 `active_sensors_all()`——它必須把本機擷取
/// 完整帶出來，而在沒有配對手機時不得憑空長出遠端感測項目。
#[tokio::test]
async fn status_active_sensors_covers_local_capture_and_invents_nothing_remote() {
    let (_g, rt, _fake) = runtime().await;
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("microphone.listen"), true)
        .await
        .unwrap();
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    rt.grant_consent("receptor:microphone.listen", None)
        .await
        .unwrap();

    // 沒有任何感測時：兩條路徑都是空的（沒有幻覺出來的手機麥克風）。
    assert!(rt.active_sensors().is_empty());
    assert!(rt.active_sensors_all().await.is_empty());
    let status = rt.status().await;
    assert_eq!(
        status["activeSensors"].as_array().map(Vec::len),
        Some(0),
        "{status}"
    );

    // 本機麥克風開著時，status 一定看得見（感測不靜默）。
    rt.begin_mic_listen(2_000, "test").await.unwrap();
    let all = rt.active_sensors_all().await;
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].kind, "microphone");
    let status = rt.status().await;
    assert_eq!(status["activeSensors"][0]["kind"], "microphone", "{status}");

    // estop → 立刻停止並從 status 消失。
    rt.emergency_stop("test", None).await.unwrap();
    assert!(rt.active_sensors_all().await.is_empty());
    assert_eq!(
        rt.status().await["activeSensors"].as_array().map(Vec::len),
        Some(0)
    );
}

/// v0.6.0：「停止結果未知」補發的事件說得出**哪一個受器**還可能在擷取，而那個
/// 受器 id 是由來源 provider 自己宣告的（`SensorStopOutcome::sensor_ids`），
/// 不是 sensors.rs 裡寫死的裝置字面值。確認停止的來源不補事件（誠實：只有
/// 未確認的才需要人處理）。
#[derive(Debug)]
struct FakeStopOutcome {
    id: String,
    sensors: Vec<String>,
    outcome: String,
    waited_ms: u64,
    stopped: bool,
}

impl interaction_runtime::sensors::SensorStopOutcome for FakeStopOutcome {
    fn source_id(&self) -> &str {
        &self.id
    }
    fn sensor_ids(&self) -> Vec<String> {
        self.sensors.clone()
    }
    fn outcome_label(&self) -> &str {
        &self.outcome
    }
    fn waited_ms(&self) -> u64 {
        self.waited_ms
    }
    fn confirmed_stopped(&self) -> bool {
        self.stopped
    }
}

#[test]
fn stop_uncertain_payloads_name_the_receptors_the_provider_declared() {
    let outcomes = vec![
        FakeStopOutcome {
            id: "robot-1".into(),
            sensors: vec!["robot.mic-level".into(), "robot.camera".into()],
            outcome: "unknown".into(),
            waited_ms: 3000,
            stopped: false,
        },
        FakeStopOutcome {
            id: "robot-2".into(),
            sensors: vec!["robot.mic-level".into()],
            outcome: "stopped".into(),
            waited_ms: 12,
            stopped: true,
        },
    ];
    let payloads = interaction_runtime::sensors::sensor_stop_uncertain_payloads(&outcomes);
    assert_eq!(payloads.len(), 2, "只有未確認的來源補事件：{payloads:?}");
    assert!(payloads
        .iter()
        .all(|p| p["deviceId"] == serde_json::json!("robot-1")));
    let sensors: Vec<String> = payloads
        .iter()
        .filter_map(|p| p["sensor"].as_str().map(String::from))
        .collect();
    assert_eq!(sensors, vec!["robot.mic-level", "robot.camera"]);
    assert_eq!(payloads[0]["outcome"], serde_json::json!("unknown"));
    assert_eq!(payloads[0]["waitedMs"], serde_json::json!(3000));
}

// ---------------------------------------------------------------------------
// 「停止所有感測」對非 mobile 來源的誠實度（`docs/aip/architecture-boundaries.md` §4.1）
// ---------------------------------------------------------------------------

/// 一個跟 iPhone 完全無關的假來源：只用來證明 stop-all 的涵蓋範圍是由
/// **provider 宣告**驅動的，不是寫死 mobile。
struct FakeRobotMic;

#[async_trait::async_trait]
impl Receptor for FakeRobotMic {
    fn manifest(&self) -> ReceptorManifest {
        serde_json::from_value(serde_json::json!({
            "id": "robot.mic-level",
            "name": "測試機器人音量",
            "description": "fixture receptor for the stop-all coverage test",
            "category": "device",
            "provides": ["level"],
            "mode": "poll",
            "sensitivity": "intimate",
            "requiresConsent": true,
            "driver": "fixture.robot",
            "version": "0.0.0",
            "schemaVersion": "1.0"
        }))
        .expect("fixture manifest parses")
    }

    async fn start(&self, _context: SessionContext) -> Result<(), ReceptorError> {
        Ok(())
    }

    async fn read(&self) -> Result<Observation, ReceptorError> {
        Err(ReceptorError::Unavailable("fixture".into()))
    }

    async fn health(&self) -> ComponentHealth {
        ComponentHealth::healthy()
    }

    async fn stop(&self) -> Result<(), ReceptorError> {
        Ok(())
    }
}

/// 高風險受器的停止涵蓋範圍由 provider 自己宣告：一個非 mobile 的 provider
/// 宣告了高風險受器、受器也還啟用著，`stop_all_sensors` 卻從來沒問過它
/// ——那就**不得**回報「全部已停」，而且要補一則「結果未知」讓它在收件匣、
/// tray、status 上看得見（誠實階梯：沒問過連 requested 都談不上）。
#[tokio::test]
async fn stop_all_sensors_is_honest_about_high_risk_receptors_it_never_asked() {
    let (_g, rt, _fake) = runtime().await;
    let robot_mic = ReceptorId::new("robot.mic-level");

    rt.registry
        .register_receptor(Arc::new(FakeRobotMic))
        .await
        .expect("fixture receptor registers");
    rt.declare_provider_capabilities(
        interaction_runtime::providers::ProviderCapabilityDeclaration::new("provider.fake.robot")
            .with_class_label("測試機器人")
            .with_receptor("robot.mic-level")
            .with_high_risk_receptor("robot.mic-level"),
    );
    // 人類啟用了它（未啟用的受器沒有東西能流進來，不必也不得嚇人）。
    rt.registry
        .set_receptor_enabled(&robot_mic, true)
        .await
        .unwrap();

    let report = rt.stop_all_sensors("test").await.unwrap();
    assert!(
        !report.stopped,
        "沒有任何來源回報涵蓋 robot.mic-level，不得宣稱全部已停：{report:?}"
    );
    assert!(report.uncertain, "沒問過＝結果未知：{report:?}");

    let uncertain: Vec<_> = rt
        .events
        .recent(50)
        .into_iter()
        .filter(|e| e.event_type == EventType::SensorStopUncertain)
        .map(|e| e.payload)
        .collect();
    assert!(
        uncertain
            .iter()
            .any(|p| p["sensor"] == serde_json::json!("robot.mic-level")),
        "宣告過的高風險受器必須出現在補發的事件裡：{uncertain:?}"
    );

    // 停用之後就不再是「可能還在擷取」：不得每次 stop-all 都無條件嚇人。
    rt.registry
        .set_receptor_enabled(&robot_mic, false)
        .await
        .unwrap();
    let report = rt.stop_all_sensors("test").await.unwrap();
    assert!(report.stopped && !report.uncertain, "{report:?}");
}

// ---------------------------------------------------------------------------
// SensorSource port（M2 §3.1）：停止所有感測是**一個**協調器，任何來源（本機
// 麥克風、手機、宣告式裝置）都只是登記進來的一個 `SensorSource`。
// 這一段的假來源跟 iPhone 完全無關：協調器不得認得任何具體裝置。
// ---------------------------------------------------------------------------

use interaction_runtime::sensor_source::{
    SensorSource, SensorStopReport, SensorStopStatus, MAX_SENSOR_SOURCES,
};
use interaction_runtime::sensors::{SENSOR_STATE_ACTIVE, SENSOR_STATE_STOP_UNKNOWN};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FakeMode {
    /// 明確確認停了。
    Confirm,
    /// 收到了但在期限內不回覆（App 當掉／連線半開）。
    Timeout,
    /// 根本送不出去。
    Unreachable,
    /// 來源明確拒絕停止（它知道自己停不了）。
    Refuse,
}

/// 程序內假來源：可設定每一種停止結果，並記錄被問了幾次、target 是什麼。
struct FakeSensorSource {
    id: String,
    declaration: String,
    receptor: String,
    mode: std::sync::Mutex<FakeMode>,
    capturing: AtomicBool,
    stop_requested: AtomicBool,
    calls: AtomicUsize,
    targets: std::sync::Mutex<Vec<Option<String>>>,
}

impl FakeSensorSource {
    fn new(id: &str, mode: FakeMode) -> Arc<Self> {
        Arc::new(FakeSensorSource {
            id: id.to_string(),
            declaration: format!("declaration.{id}"),
            receptor: format!("{id}.mic-level"),
            mode: std::sync::Mutex::new(mode),
            capturing: AtomicBool::new(true),
            stop_requested: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            targets: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(AtomicOrdering::SeqCst)
    }

    fn targets(&self) -> Vec<Option<String>> {
        self.targets.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl SensorSource for FakeSensorSource {
    fn source_id(&self) -> String {
        self.id.clone()
    }

    fn declaration_id(&self) -> String {
        self.declaration.clone()
    }

    async fn active_captures(&self) -> Vec<interaction_runtime::sensors::SensorUse> {
        if !self.capturing.load(AtomicOrdering::SeqCst) {
            return vec![];
        }
        let state = if self.stop_requested.load(AtomicOrdering::SeqCst) {
            SENSOR_STATE_STOP_UNKNOWN
        } else {
            SENSOR_STATE_ACTIVE
        };
        vec![interaction_runtime::sensors::SensorUse {
            kind: self.receptor.clone(),
            started_at: chrono::Utc::now(),
            started_by: self.id.clone(),
            purpose: "fixture capture".into(),
            auto_stop_at: None,
            state: state.to_string(),
        }]
    }

    async fn request_stop(
        &self,
        target: Option<&str>,
        deadline: Duration,
        _reason: &str,
    ) -> Vec<SensorStopReport> {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        self.targets
            .lock()
            .unwrap()
            .push(target.map(str::to_string));
        let sensors = vec![self.receptor.clone()];
        if !self.capturing.load(AtomicOrdering::SeqCst) {
            return vec![SensorStopReport::new(
                self.id.clone(),
                self.declaration.clone(),
                sensors,
                SensorStopStatus::AlreadyStopped,
                0,
            )];
        }
        let mode = *self.mode.lock().unwrap();
        let status = match mode {
            FakeMode::Confirm => {
                self.capturing.store(false, AtomicOrdering::SeqCst);
                SensorStopStatus::Stopped
            }
            FakeMode::Timeout => {
                self.stop_requested.store(true, AtomicOrdering::SeqCst);
                tokio::time::sleep(deadline).await;
                SensorStopStatus::Unknown
            }
            FakeMode::Unreachable => SensorStopStatus::Unreachable,
            FakeMode::Refuse => SensorStopStatus::Refused,
        };
        vec![SensorStopReport::new(
            self.id.clone(),
            self.declaration.clone(),
            sensors,
            status,
            0,
        )]
    }
}

fn uncertain_sensors(rt: &Runtime) -> Vec<String> {
    rt.events
        .recent(200)
        .into_iter()
        .filter(|e| e.event_type == EventType::SensorStopUncertain)
        .filter_map(|e| e.payload["sensor"].as_str().map(String::from))
        .collect()
}

fn stopped_sensors(rt: &Runtime) -> Vec<String> {
    rt.events
        .recent(200)
        .into_iter()
        .filter(|e| e.event_type == EventType::SensorStopped)
        .filter_map(|e| e.payload["sensor"].as_str().map(String::from))
        .collect()
}

/// 每一種停止結果都要誠實地變成一份 report＋對應的事件：確認停了才發
/// `sensor.stopped`；unknown／unreachable／refused 一律補 `sensor.stop-uncertain`
/// （requested≠stopped）。
#[tokio::test]
async fn every_stop_outcome_of_a_source_is_reported_and_evented_honestly() {
    for (mode, label, confirmed) in [
        (FakeMode::Confirm, "stopped", true),
        (FakeMode::Timeout, "unknown", false),
        (FakeMode::Unreachable, "unreachable", false),
        (FakeMode::Refuse, "refused", false),
    ] {
        let (_g, rt, _fake) = runtime().await;
        let source = FakeSensorSource::new("fixture.source", mode);
        rt.register_sensor_source(source.clone())
            .await
            .expect("source registers");

        let sweep = rt
            .stop_all_sensor_sources("test", "stop-all-sensors", Duration::from_millis(120))
            .await;
        let mine: Vec<_> = sweep
            .reports
            .iter()
            .filter(|r| r.source_id == "fixture.source")
            .collect();
        assert_eq!(mine.len(), 1, "{mode:?}: {:?}", sweep.reports);
        assert_eq!(mine[0].outcome.as_str(), label, "{mode:?}");
        assert_eq!(mine[0].confirmed(), confirmed, "{mode:?}");
        assert_eq!(sweep.stopped(), confirmed, "{mode:?}: {sweep:?}");
        assert_eq!(sweep.uncertain(), !confirmed, "{mode:?}: {sweep:?}");

        let sensor = "fixture.source.mic-level".to_string();
        if confirmed {
            assert!(
                stopped_sensors(&rt).contains(&sensor),
                "{mode:?}: 確認停了要發 sensor.stopped"
            );
            assert!(!uncertain_sensors(&rt).contains(&sensor), "{mode:?}");
        } else {
            assert!(
                uncertain_sensors(&rt).contains(&sensor),
                "{mode:?}: 未確認一律補 sensor.stop-uncertain"
            );
            assert!(
                !stopped_sensors(&rt).contains(&sensor),
                "{mode:?}: 沒確認不得謊稱停了"
            );
        }
    }
}

/// 重複停止是冪等的：第二次問，來源誠實回「本來就沒在擷取」，
/// 不重發 sensor.stopped，也不變成 uncertain。
#[tokio::test]
async fn stopping_a_source_twice_is_idempotent() {
    let (_g, rt, _fake) = runtime().await;
    let source = FakeSensorSource::new("fixture.source", FakeMode::Confirm);
    rt.register_sensor_source(source.clone()).await.unwrap();

    let first = rt
        .stop_all_sensor_sources("test", "stop-all-sensors", Duration::from_millis(120))
        .await;
    let first_mine = first
        .reports
        .iter()
        .find(|r| r.source_id == "fixture.source")
        .expect("reported");
    assert_eq!(first_mine.outcome, SensorStopStatus::Stopped);
    let second = rt
        .stop_all_sensor_sources("test", "stop-all-sensors", Duration::from_millis(120))
        .await;
    let mine = second
        .reports
        .iter()
        .find(|r| r.source_id == "fixture.source")
        .expect("still reported");
    assert_eq!(mine.outcome, SensorStopStatus::AlreadyStopped);
    assert!(second.stopped() && !second.uncertain(), "{second:?}");
    assert_eq!(source.calls(), 2);
    assert_eq!(
        stopped_sensors(&rt)
            .iter()
            .filter(|s| s.as_str() == "fixture.source.mic-level")
            .count(),
        1,
        "已經停了的來源不得再發一次 sensor.stopped"
    );
}

/// 停止進行中把來源移除：仍然要拿到那一份 uncertain 報告，而且它的感測
/// **不得從 activeSensors 靜默消失**（消失＝宣稱它停了）。
#[tokio::test(flavor = "multi_thread")]
async fn a_source_removed_while_stopping_still_reports_and_stays_visible() {
    let (_g, rt, _fake) = runtime().await;
    let source = FakeSensorSource::new("fixture.source", FakeMode::Timeout);
    rt.register_sensor_source(source.clone()).await.unwrap();
    assert_eq!(rt.active_sensors_all().await.len(), 1, "登記後看得見");

    let rt2 = rt.clone();
    let sweep = tokio::spawn(async move {
        rt2.stop_all_sensor_sources("test", "stop-all-sensors", Duration::from_millis(400))
            .await
    });
    // 停止還在進行中就把來源移除。
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(rt.unregister_sensor_source("fixture.source").await);

    let sweep = sweep.await.expect("sweep finishes");
    let mine = sweep
        .reports
        .iter()
        .find(|r| r.source_id == "fixture.source")
        .expect("移除不得吞掉進行中的停止結果");
    assert_eq!(mine.outcome, SensorStopStatus::Unknown);
    assert!(sweep.uncertain(), "{sweep:?}");

    let visible = rt.active_sensors_all().await;
    assert!(
        visible.iter().any(|s| s.kind == "fixture.source.mic-level"),
        "來源被移除但沒確認停止 → 仍要看得見（感測不靜默）：{visible:?}"
    );
    assert!(
        visible
            .iter()
            .all(|s| s.kind != "fixture.source.mic-level" || s.state != SENSOR_STATE_ACTIVE),
        "已要求停止的不得再標 active：{visible:?}"
    );
}

/// 掃描必須有界：一個永遠不回覆的來源不得把停止拖成無限等待。
#[tokio::test(flavor = "multi_thread")]
async fn the_stop_sweep_is_bounded_by_the_deadline() {
    let (_g, rt, _fake) = runtime().await;
    rt.register_sensor_source(FakeSensorSource::new("fixture.slow", FakeMode::Timeout))
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let sweep = rt
        .stop_all_sensor_sources("test", "stop-all-sensors", Duration::from_millis(150))
        .await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(1200),
        "等待必須有界：{elapsed:?}"
    );
    assert!(sweep.uncertain(), "{sweep:?}");
}

/// 來源登記表是有界的：超過上限的登記被拒絕並留稽核（區網／設定檔上的東西
/// 不得把記憶體吃光），而且既有來源不受影響。
#[tokio::test]
async fn the_sensor_source_registry_is_bounded() {
    let (_g, rt, _fake) = runtime().await;
    let mut accepted = 0usize;
    for i in 0..MAX_SENSOR_SOURCES + 4 {
        if rt
            .register_sensor_source(FakeSensorSource::new(
                &format!("fixture.{i}"),
                FakeMode::Confirm,
            ))
            .await
            .is_ok()
        {
            accepted += 1;
        }
    }
    assert!(
        accepted < MAX_SENSOR_SOURCES + 4,
        "超過上限必須被拒絕（accepted={accepted}）"
    );
    assert!(rt.sensor_source_ids().await.len() <= MAX_SENSOR_SOURCES);
    let audit = rt
        .store
        .audit_tail(200)
        .unwrap()
        .into_iter()
        .rfind(|a| a["kind"] == serde_json::json!("sensor.source-rejected"));
    assert!(audit.is_some(), "拒絕要留痕");
}

/// wire 契約：`devices` 不動（HTTP／CLI／桌面逐欄位讀），非 mobile 來源另外
/// 放在 `sources`；`stopped`／`uncertain` 兩個陣列一起看。
#[tokio::test]
async fn the_stop_all_report_keeps_devices_and_adds_sources() {
    let (_g, rt, _fake) = runtime().await;
    rt.register_sensor_source(FakeSensorSource::new("fixture.source", FakeMode::Refuse))
        .await
        .unwrap();
    let report = rt.stop_all_sensors("test").await.unwrap();
    let wire = serde_json::to_value(&report).unwrap();
    assert!(wire["devices"].is_array(), "{wire}");
    assert_eq!(wire["devices"].as_array().map(Vec::len), Some(0), "{wire}");
    assert_eq!(
        wire["sources"][0]["sourceId"],
        serde_json::json!("fixture.source"),
        "{wire}"
    );
    assert_eq!(wire["sources"][0]["outcome"], serde_json::json!("refused"));
    assert_eq!(wire["stopped"], serde_json::json!(false), "{wire}");
    assert_eq!(wire["uncertain"], serde_json::json!(true), "{wire}");
}

/// X1：緊急停止與「停止所有感測」是**同一個**協調器。宣告了高風險受器但沒有
/// 任何來源涵蓋它時，兩條路徑必須回同樣的 uncertain，並補同樣的
/// `sensor.stop-uncertain{no-stop-path}`——緊急停止不得比使用者按的那顆按鈕
/// 更樂觀。
#[tokio::test(flavor = "multi_thread")]
async fn emergency_stop_and_stop_all_sensors_agree_about_an_unstoppable_receptor() {
    async fn declare_uncoverable(rt: &Runtime) {
        rt.registry
            .register_receptor(Arc::new(FakeRobotMic))
            .await
            .expect("fixture receptor registers");
        rt.declare_provider_capabilities(
            interaction_runtime::providers::ProviderCapabilityDeclaration::new(
                "provider.fake.robot",
            )
            .with_receptor("robot.mic-level")
            .with_high_risk_receptor("robot.mic-level"),
        );
        rt.registry
            .set_receptor_enabled(&ReceptorId::new("robot.mic-level"), true)
            .await
            .unwrap();
    }

    let (_g1, user_rt, _f1) = runtime().await;
    declare_uncoverable(&user_rt).await;
    let user_report = user_rt.stop_all_sensors("test").await.unwrap();

    let (_g2, estop_rt, _f2) = runtime().await;
    declare_uncoverable(&estop_rt).await;
    let estop = estop_rt.emergency_stop("test", None).await.unwrap();
    let estop_report = &estop["sensors"];

    assert_eq!(
        estop_report["stopped"],
        serde_json::json!(user_report.stopped),
        "兩條路徑對同一種情況必須一致：{estop_report}"
    );
    assert_eq!(estop_report["stopped"], serde_json::json!(false));
    assert_eq!(estop_report["uncertain"], serde_json::json!(true));
    assert!(user_report.uncertain);

    for (name, rt) in [("stop-all", &user_rt), ("estop", &estop_rt)] {
        let payloads: Vec<_> = rt
            .events
            .recent(200)
            .into_iter()
            .filter(|e| e.event_type == EventType::SensorStopUncertain)
            .map(|e| e.payload)
            .collect();
        assert!(
            payloads
                .iter()
                .any(|p| p["sensor"] == serde_json::json!("robot.mic-level")
                    && p["outcome"] == serde_json::json!("no-stop-path")),
            "{name}: 沒有停止管道要補事件：{payloads:?}"
        );
        let audit = rt
            .store
            .audit_tail(200)
            .unwrap()
            .into_iter()
            .rfind(|a| a["kind"] == serde_json::json!("sensor.stop-not-requested"));
        assert!(audit.is_some(), "{name}: 沒問到的受器要留稽核");
    }
}

/// S4（DELETE 競態）：直接刪掉一個高風險受器時，桌面必須**先**請涵蓋它的來源
/// 停止感測。否則受器記錄消失、來源還在錄，畫面卻一片安靜。
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_high_risk_receptor_asks_its_source_to_stop_first() {
    let (_g, rt, _fake) = runtime().await;
    let source = FakeSensorSource::new("fixture.source", FakeMode::Timeout);
    rt.register_sensor_source(source.clone()).await.unwrap();
    rt.registry
        .register_receptor(Arc::new(FakeRobotMic))
        .await
        .unwrap();
    rt.declare_provider_capabilities(
        interaction_runtime::providers::ProviderCapabilityDeclaration::new(
            "declaration.fixture.source",
        )
        .with_receptor("robot.mic-level")
        .with_high_risk_receptor("robot.mic-level"),
    );
    rt.registry
        .set_receptor_enabled(&ReceptorId::new("robot.mic-level"), true)
        .await
        .unwrap();

    rt.unregister_receptor(&ReceptorId::new("robot.mic-level"))
        .await
        .expect("receptor is removed");

    assert_eq!(source.calls(), 1, "刪除前要先請來源停止感測");
    assert!(
        rt.registry
            .receptor(&ReceptorId::new("robot.mic-level"))
            .await
            .is_err(),
        "受器確實被移除"
    );
    let visible = rt.active_sensors_all().await;
    assert!(
        visible.iter().any(|s| s.kind == "fixture.source.mic-level"),
        "來源沒確認停止 → 不得整筆消失：{visible:?}"
    );
    assert!(
        uncertain_sensors(&rt).contains(&"fixture.source.mic-level".to_string()),
        "沒確認就要補 sensor.stop-uncertain"
    );
}

/// 撤銷一個 provider 時，登記在它底下的來源要走**同一個** request_stop
/// （target 指名那一台），結果進 `provider.revoked` 的稽核。
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_provider_stops_its_sensor_source_with_a_target() {
    let (_g, rt, _fake) = runtime().await;
    let source = FakeSensorSource::new("provider.fixture", FakeMode::Confirm);
    rt.register_sensor_source(source.clone()).await.unwrap();
    let pid = interaction_core::ProviderId::new("provider.fixture.unit-1");
    rt.providers
        .register(interaction_core::ProviderDescriptor {
            identity: interaction_core::ProviderIdentity {
                id: pid.clone(),
                kind: interaction_core::ProviderKind::Device,
                display_name: "fixture unit".into(),
                trust_level: interaction_core::TrustLevel::Paired,
                origin: "fixture".into(),
                version: "0".into(),
                fingerprint: None,
                human: None,
            },
            state: interaction_core::ProviderState::Available,
            receptors: vec![],
            actuators: vec![],
            tool_operations: vec![],
            paired_at: None,
            last_seen: None,
            detail: None,
        })
        .await
        .expect("fixture provider registers");

    rt.revoke_provider(&pid).await.expect("revoked");

    assert_eq!(source.calls(), 1, "撤銷要走同一條停止路徑");
    assert_eq!(
        source.targets(),
        vec![Some("unit-1".to_string())],
        "只停這一台，不是整族"
    );
    let audit = rt
        .store
        .audit_tail(200)
        .unwrap()
        .into_iter()
        .rfind(|a| a["kind"] == serde_json::json!("provider.revoked"))
        .expect("provider.revoked audit");
    assert_eq!(
        audit["detail"]["sensorStop"]["reports"][0]["outcome"],
        serde_json::json!("stopped"),
        "停止結果要進稽核：{audit}"
    );
}

/// 撤銷與停止掃描**同時**發生：兩條路徑都會問同一個來源。兩者都必須有界回來、
/// 各自誠實（沒確認就是未知），而且那台來源的感測不得在中途靜默消失。
#[tokio::test(flavor = "multi_thread")]
async fn a_revoke_racing_a_stop_sweep_stays_bounded_and_honest() {
    let (_g, rt, _fake) = runtime().await;
    let source = FakeSensorSource::new("provider.fixture", FakeMode::Timeout);
    rt.register_sensor_source(source.clone()).await.unwrap();
    let pid = interaction_core::ProviderId::new("provider.fixture.unit-1");
    rt.providers
        .register(interaction_core::ProviderDescriptor {
            identity: interaction_core::ProviderIdentity {
                id: pid.clone(),
                kind: interaction_core::ProviderKind::Device,
                display_name: "fixture unit".into(),
                trust_level: interaction_core::TrustLevel::Paired,
                origin: "fixture".into(),
                version: "0".into(),
                fingerprint: None,
                human: None,
            },
            state: interaction_core::ProviderState::Available,
            receptors: vec![],
            actuators: vec![],
            tool_operations: vec![],
            paired_at: None,
            last_seen: None,
            detail: None,
        })
        .await
        .unwrap();

    let started = std::time::Instant::now();
    let sweeping = {
        let rt = rt.clone();
        tokio::spawn(async move {
            rt.stop_all_sensor_sources("test", "stop-all-sensors", Duration::from_millis(300))
                .await
        })
    };
    let revoking = {
        let rt = rt.clone();
        let pid = pid.clone();
        tokio::spawn(async move { rt.revoke_provider(&pid).await })
    };
    let sweep = sweeping.await.expect("sweep finishes");
    revoking
        .await
        .expect("revoke finishes")
        .expect("revoke succeeds");
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "兩條路徑都必須有界：{:?}",
        started.elapsed()
    );
    assert!(sweep.uncertain(), "沒確認就是未知：{sweep:?}");
    assert!(
        source.calls() >= 2,
        "兩條路徑都問過來源：{}",
        source.calls()
    );
    let audit = rt
        .store
        .audit_tail(200)
        .unwrap()
        .into_iter()
        .rfind(|a| a["kind"] == serde_json::json!("provider.revoked"))
        .expect("provider.revoked audit");
    assert_eq!(
        audit["detail"]["sensorStop"]["reports"][0]["outcome"],
        serde_json::json!("unknown"),
        "撤銷路徑也不得謊稱停了：{audit}"
    );
    let visible = rt.active_sensors_all().await;
    assert!(
        visible
            .iter()
            .any(|s| s.kind == "provider.fixture.mic-level"),
        "競態不得讓感測靜默消失：{visible:?}"
    );
}

/// 「來源被移除時還在擷取」的記錄也必須有界：來源反覆登記／移除不得讓這張表
/// （以及 `activeSensors`）無界成長。滿了就丟最舊的一筆並留稽核——被丟掉的
/// 那一筆從來沒有被說成「已停止」。
#[tokio::test]
async fn the_removed_while_capturing_record_is_bounded() {
    let (_g, rt, _fake) = runtime().await;
    for i in 0..MAX_SENSOR_SOURCES + 8 {
        let id = format!("fixture.churn-{i}");
        rt.register_sensor_source(FakeSensorSource::new(&id, FakeMode::Timeout))
            .await
            .expect("registers");
        assert!(rt.unregister_sensor_source(&id).await);
    }
    let visible = rt.active_sensors_all().await;
    assert!(
        visible.len() <= MAX_SENSOR_SOURCES,
        "有界：{} 筆",
        visible.len()
    );
    let dropped = rt
        .store
        .audit_tail(200)
        .unwrap()
        .into_iter()
        .rfind(|a| a["kind"] == serde_json::json!("sensor.removed-capture-record-dropped"));
    assert!(dropped.is_some(), "丟掉最舊的一筆要留痕");
}

// ---------------------------------------------------------------------------
// 未解決停止（M3 §b）：來源被移除時「可能還在擷取」的那一筆，在即時清單的
// 60 秒可見窗過去之後**不得**變成「一切正常」。它離開即時清單，轉進一份
// 不受 TTL 影響的「未解決停止」摘要，只能由同 id 的**新**來源確認停止、
// 或人類明確解除來清掉。
// ---------------------------------------------------------------------------

use interaction_runtime::sensor_source::{MAX_UNRESOLVED_STOPS, ORPHAN_CAPTURE_VISIBLE};

/// 把時鐘推過即時可見窗（測試不必真的等 60 秒）。
fn expire_orphan_window(rt: &Runtime) {
    rt.sensor_clock
        .advance(ORPHAN_CAPTURE_VISIBLE + Duration::from_secs(1));
}

fn audit_kinds(rt: &Runtime) -> Vec<String> {
    rt.store
        .audit_tail(200)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|row| row["kind"].as_str().map(str::to_string))
        .collect()
}

/// 到期的孤兒記錄**不得**靜靜消失成「全部正常」：它轉進未解決停止摘要，
/// 而且那份摘要不受 TTL 影響。
#[tokio::test]
async fn orphan_ttl_moves_unknown_to_unresolved_not_to_normal() {
    let (_g, rt, _fake) = runtime().await;
    rt.register_sensor_source(FakeSensorSource::new("fixture.gone", FakeMode::Timeout))
        .await
        .unwrap();
    let generation = rt
        .sensor_source_generation("fixture.gone")
        .await
        .expect("registered sources carry a generation");
    assert!(rt.unregister_sensor_source("fixture.gone").await);

    // 可見窗之內：留在即時清單上（感測不靜默）。
    assert!(
        rt.active_sensors_all()
            .await
            .iter()
            .any(|s| s.kind == "fixture.gone.mic-level"),
        "可見窗之內必須看得見"
    );
    assert!(
        rt.unresolved_stops().await.is_empty(),
        "還在即時清單上的不算未解決摘要"
    );

    expire_orphan_window(&rt);

    let visible = rt.active_sensors_all().await;
    assert!(
        !visible.iter().any(|s| s.kind == "fixture.gone.mic-level"),
        "過期之後不再佔著即時清單：{visible:?}"
    );
    let unresolved = rt.unresolved_stops().await;
    let mine = unresolved
        .iter()
        .find(|u| u.source_id == "fixture.gone")
        .unwrap_or_else(|| panic!("過期不等於停了，必須留成未解決停止：{unresolved:?}"));
    assert_eq!(mine.generation, generation);
    assert_eq!(mine.sensors, vec!["fixture.gone.mic-level".to_string()]);
    assert!(
        mine.last_known
            .iter()
            .all(|c| c.state == SENSOR_STATE_STOP_UNKNOWN),
        "最後看到的狀態不得被改寫成別的東西：{mine:?}"
    );
    assert!(
        audit_kinds(&rt)
            .iter()
            .any(|k| k == "sensor.unresolved-stop-recorded"),
        "轉進未解決摘要要留稽核"
    );

    // 再過久也不會自己消失（它不是 TTL 快取，是一筆沒有結論的事）。
    expire_orphan_window(&rt);
    expire_orphan_window(&rt);
    assert_eq!(
        rt.unresolved_stops().await.len(),
        1,
        "未解決停止不隨時間過期"
    );
}

/// 同一個 id 重新登記一個**新**來源，不得抹掉舊那一次登記留下的未解決記錄：
/// 新來源不知道舊連線那一頭發生過什麼事。
#[tokio::test]
async fn same_id_new_source_does_not_clear_old_generation_unknown() {
    let (_g, rt, _fake) = runtime().await;
    rt.register_sensor_source(FakeSensorSource::new("fixture.churn", FakeMode::Timeout))
        .await
        .unwrap();
    let first = rt
        .sensor_source_generation("fixture.churn")
        .await
        .expect("generation");
    assert!(rt.unregister_sensor_source("fixture.churn").await);

    // 同一個 id 又回來了（裝置重連／adapter 重新綁定）。
    rt.register_sensor_source(FakeSensorSource::new("fixture.churn", FakeMode::Timeout))
        .await
        .unwrap();
    let second = rt
        .sensor_source_generation("fixture.churn")
        .await
        .expect("generation");
    assert!(second > first, "重新登記要是新的世代：{first} → {second}");

    let visible = rt.active_sensors_all().await;
    assert!(
        visible
            .iter()
            .filter(|s| s.kind == "fixture.churn.mic-level")
            .count()
            >= 2,
        "舊那一筆「可能還在擷取」與新來源的擷取要同時看得見：{visible:?}"
    );

    expire_orphan_window(&rt);
    let unresolved = rt.unresolved_stops().await;
    assert!(
        unresolved
            .iter()
            .any(|u| u.source_id == "fixture.churn" && u.generation == first),
        "同 id 的新來源不得替舊世代作證：{unresolved:?}"
    );
}

/// 只有「同 id 的**新**來源對那個受器確認停止」才清得掉未解決停止。
#[tokio::test]
async fn confirmed_stop_from_new_source_clears_unresolved() {
    let (_g, rt, _fake) = runtime().await;
    rt.register_sensor_source(FakeSensorSource::new("fixture.back", FakeMode::Timeout))
        .await
        .unwrap();
    assert!(rt.unregister_sensor_source("fixture.back").await);
    expire_orphan_window(&rt);
    assert_eq!(rt.unresolved_stops().await.len(), 1, "先有一筆未解決");

    // 同一台裝置回來了，而且這次它明確確認停止。
    rt.register_sensor_source(FakeSensorSource::new("fixture.back", FakeMode::Confirm))
        .await
        .unwrap();
    let sweep = rt
        .stop_all_sensor_sources("test", "stop-all-sensors", Duration::from_millis(200))
        .await;
    assert!(
        sweep
            .reports
            .iter()
            .any(|r| r.source_id == "fixture.back" && r.outcome == SensorStopStatus::Stopped),
        "新來源要真的確認停止：{sweep:?}"
    );

    assert!(
        rt.unresolved_stops().await.is_empty(),
        "同 id 新來源確認停止之後才清得掉：{:?}",
        rt.unresolved_stops().await
    );
    assert!(
        audit_kinds(&rt)
            .iter()
            .any(|k| k == "sensor.unresolved-stop-resolved"),
        "清除也要留稽核"
    );
}

/// 人為解除必須是**明確**的：指名世代、留稽核，而且絕不宣稱「它停了」。
#[tokio::test]
async fn dismiss_unresolved_is_explicit_and_audited() {
    let (_g, rt, _fake) = runtime().await;
    rt.register_sensor_source(FakeSensorSource::new("fixture.dismiss", FakeMode::Timeout))
        .await
        .unwrap();
    let generation = rt
        .sensor_source_generation("fixture.dismiss")
        .await
        .expect("generation");
    assert!(rt.unregister_sensor_source("fixture.dismiss").await);
    expire_orphan_window(&rt);
    assert_eq!(rt.unresolved_stops().await.len(), 1);

    // 世代不對＝不同的一筆：不得誤消。
    let wrong = rt
        .dismiss_unresolved_stop("fixture.dismiss", generation + 99, "user")
        .await;
    assert!(matches!(wrong, Err(DomainError::NotFound(_))), "{wrong:?}");
    assert_eq!(rt.unresolved_stops().await.len(), 1, "誤指的世代不得清掉");

    let out = rt
        .dismiss_unresolved_stop("fixture.dismiss", generation, "user")
        .await
        .expect("dismissed");
    assert_eq!(out["confirmedStopped"], serde_json::json!(false), "{out}");
    assert_eq!(out["generation"], serde_json::json!(generation), "{out}");
    assert!(rt.unresolved_stops().await.is_empty());
    let audit = rt
        .store
        .audit_tail(200)
        .unwrap()
        .into_iter()
        .rfind(|a| a["kind"] == serde_json::json!("sensor.unresolved-stop-dismissed"))
        .expect("人為解除必須留稽核");
    assert_eq!(audit["actor"], serde_json::json!("user"), "{audit}");
    assert_eq!(
        audit["detail"]["sourceId"],
        serde_json::json!("fixture.dismiss"),
        "{audit}"
    );
}

/// 三個面向是分開的：即時清單（activeSensors）、未解決摘要（現在還不知道
/// 結果的事）、歷史（稽核）。過期只在前兩者之間搬動，歷史永遠留著。
#[tokio::test]
async fn live_unresolved_and_history_are_separate() {
    let (_g, rt, _fake) = runtime().await;
    rt.register_sensor_source(FakeSensorSource::new("fixture.split", FakeMode::Timeout))
        .await
        .unwrap();
    assert!(rt.unregister_sensor_source("fixture.split").await);

    // 1) 即時清單有、未解決摘要沒有。
    assert!(rt
        .active_sensors_all()
        .await
        .iter()
        .any(|s| s.kind == "fixture.split.mic-level"));
    assert!(rt.unresolved_stops().await.is_empty());
    // status 這時不該多一個空欄位。
    assert!(
        rt.status().await.get("unresolvedStops").is_none(),
        "沒有未解決的事就不該序列化這個欄位"
    );

    expire_orphan_window(&rt);

    // 2) 即時清單沒有、未解決摘要有、status 看得到。
    assert!(!rt
        .active_sensors_all()
        .await
        .iter()
        .any(|s| s.kind == "fixture.split.mic-level"));
    assert_eq!(rt.unresolved_stops().await.len(), 1);
    let status = rt.status().await;
    let listed = status["unresolvedStops"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(listed.len(), 1, "{status}");
    assert_eq!(
        listed[0]["sourceId"],
        serde_json::json!("fixture.split"),
        "{status}"
    );

    // 3) 歷史（稽核）兩件事都在，而且移除當下那一則從頭到尾沒有被改寫。
    let kinds = audit_kinds(&rt);
    assert!(kinds
        .iter()
        .any(|k| k == "sensor.source-removed-while-capturing"));
    assert!(kinds.iter().any(|k| k == "sensor.unresolved-stop-recorded"));
}

/// 未解決摘要自己也必須有界：它不隨時間過期，所以滿了要丟最舊的一筆並留痕。
#[tokio::test]
async fn the_unresolved_stop_summary_is_bounded() {
    let (_g, rt, _fake) = runtime().await;
    for i in 0..MAX_UNRESOLVED_STOPS + 4 {
        let id = format!("fixture.flood-{i}");
        rt.register_sensor_source(FakeSensorSource::new(&id, FakeMode::Timeout))
            .await
            .expect("registers");
        assert!(rt.unregister_sensor_source(&id).await);
        expire_orphan_window(&rt);
        // 每一輪都讓過期的那些搬進未解決摘要。
        let _ = rt.active_sensors_all().await;
    }
    assert!(
        rt.unresolved_stops().await.len() <= MAX_UNRESOLVED_STOPS,
        "有界：{} 筆",
        rt.unresolved_stops().await.len()
    );
    assert!(
        audit_kinds(&rt)
            .iter()
            .any(|k| k == "sensor.unresolved-stop-dropped"),
        "丟掉最舊的一筆要留痕"
    );
}
