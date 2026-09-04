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
