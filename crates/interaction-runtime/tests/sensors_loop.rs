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

    // Manual stop releases the device and clears the indicator.
    rt.stop_all_sensors("test").await.unwrap();
    assert!(fake.stopped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(rt.active_sensors().is_empty());
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
