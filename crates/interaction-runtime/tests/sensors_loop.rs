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
