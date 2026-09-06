//! Independent N3 review regressions; source fixtures never open physical sensors.
use interaction_runtime::sensor_source::{SensorSource, SensorStopReport, SensorStopStatus};
use interaction_runtime::sensors::SensorUse;
use interaction_runtime::{Runtime, RuntimeOptions};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Family {
    captures: Mutex<Vec<String>>,
    reports: Mutex<Vec<(String, String, SensorStopStatus)>>,
}
impl Family {
    fn set(&self, kind: &str, device: &str, status: SensorStopStatus) {
        *self.captures.lock().unwrap() = vec![kind.into()];
        *self.reports.lock().unwrap() = vec![(device.into(), kind.into(), status)];
    }
}
#[async_trait::async_trait]
impl SensorSource for Family {
    fn source_id(&self) -> String {
        "review.family".into()
    }
    fn declaration_id(&self) -> String {
        "review.family".into()
    }
    async fn active_captures(&self) -> Vec<SensorUse> {
        self.captures
            .lock()
            .unwrap()
            .iter()
            .map(|kind| SensorUse {
                kind: kind.clone(),
                started_at: chrono::Utc::now(),
                started_by: "review".into(),
                purpose: "review fixture".into(),
                auto_stop_at: None,
                state: "stop-unknown".into(),
            })
            .collect()
    }
    async fn request_stop(&self, _: Option<&str>, _: Duration, _: &str) -> Vec<SensorStopReport> {
        self.reports
            .lock()
            .unwrap()
            .iter()
            .map(|(device, kind, status)| {
                SensorStopReport::new(device, "review.family", vec![kind.clone()], *status, 0)
                    .with_via(Some("peer-ack"))
            })
            .collect()
    }
}
async fn runtime(home: &std::path::Path) -> Runtime {
    Runtime::start(RuntimeOptions {
        home: Some(home.into()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap()
}
async fn stop(rt: &Runtime) {
    rt.stop_all_sensor_sources("independent-review", "review", Duration::from_millis(20))
        .await;
}
fn family() -> Arc<Family> {
    Arc::new(Family {
        captures: Mutex::new(vec![]),
        reports: Mutex::new(vec![]),
    })
}

#[tokio::test]
async fn changing_a_live_family_capture_list_cannot_erase_prior_unknown_sensors() {
    let home = tempfile::tempdir().unwrap();
    let rt = runtime(home.path()).await;
    let source = family();
    source.set("camera-a", "device-a", SensorStopStatus::Unknown);
    rt.register_sensor_source(source.clone()).await.unwrap();
    stop(&rt).await;
    // Device A disconnects without a stop acknowledgment. The family source is
    // still registered; its live list now describes another device only.
    source.set("microphone-b", "device-b", SensorStopStatus::Unknown);
    stop(&rt).await;
    rt.shutdown().await;
    drop(rt);
    let rt = runtime(home.path()).await;
    let pending = rt.unresolved_stops().await;
    let sensors = &pending
        .iter()
        .find(|r| r.source_id == "review.family")
        .unwrap()
        .sensors;
    assert!(
        sensors.contains(&"camera-a".into()),
        "a newer live capture list is not evidence that prior camera-a stopped: {sensors:?}"
    );
    assert!(sensors.contains(&"microphone-b".into()));
    rt.shutdown().await;
}

#[tokio::test]
async fn a_family_member_stop_does_not_confirm_an_absent_members_same_sensor_kind() {
    let home = tempfile::tempdir().unwrap();
    let rt = runtime(home.path()).await;
    let source = family();
    source.set("camera", "device-a", SensorStopStatus::Unknown);
    rt.register_sensor_source(source.clone()).await.unwrap();
    stop(&rt).await;
    // Device A is absent from this new report; only B confirms its own camera.
    source.set("camera", "device-b", SensorStopStatus::Stopped);
    stop(&rt).await;
    rt.shutdown().await;
    drop(rt);
    let rt = runtime(home.path()).await;
    let pending = rt.unresolved_stops().await;
    assert!(
        pending
            .iter()
            .any(|r| r.source_id == "review.family" && r.sensors.contains(&"camera".into())),
        "device-b's acknowledgment cannot resolve absent device-a's camera"
    );
    rt.shutdown().await;
}

#[tokio::test]
async fn a_registered_family_does_not_hide_unknowns_after_the_live_capture_disappears() {
    let home = tempfile::tempdir().unwrap();
    let rt = runtime(home.path()).await;
    let source = family();
    source.set("camera-a", "device-a", SensorStopStatus::Unknown);
    rt.register_sensor_source(source.clone()).await.unwrap();
    stop(&rt).await;
    source.captures.lock().unwrap().clear();
    assert!(rt.active_sensors_all().await.is_empty());
    assert!(
        rt.unresolved_stops()
            .await
            .iter()
            .any(|entry| entry.source_id == "review.family"),
        "registration alone is not a visible capture or stop confirmation"
    );
    rt.shutdown().await;
}

struct ScopedFamily {
    capture_scopes: Mutex<Vec<String>>,
    stopped_scope: Mutex<Option<String>>,
}
impl ScopedFamily {
    fn captures(&self) -> Vec<(SensorUse, String)> {
        self.capture_scopes
            .lock()
            .unwrap()
            .iter()
            .map(|scope| {
                (
                    SensorUse {
                        kind: "camera".into(),
                        started_at: chrono::Utc::now(),
                        started_by: "fixture".into(),
                        purpose: "scoped fixture".into(),
                        auto_stop_at: None,
                        state: "stop-unknown".into(),
                    },
                    scope.clone(),
                )
            })
            .collect()
    }
}
#[async_trait::async_trait]
impl SensorSource for ScopedFamily {
    fn source_id(&self) -> String {
        "review.scoped".into()
    }
    fn declaration_id(&self) -> String {
        "review.scoped".into()
    }
    async fn active_captures(&self) -> Vec<SensorUse> {
        self.captures()
            .into_iter()
            .map(|(capture, _)| capture)
            .collect()
    }
    async fn scoped_captures(&self) -> Vec<(SensorUse, String)> {
        self.captures()
    }
    async fn request_stop(&self, _: Option<&str>, _: Duration, _: &str) -> Vec<SensorStopReport> {
        let Some(scope) = self.stopped_scope.lock().unwrap().clone() else {
            return vec![];
        };
        let mut report = SensorStopReport::new(
            "same-device-id",
            "review.scoped",
            vec!["camera".into()],
            SensorStopStatus::Stopped,
            0,
        )
        .with_via(Some("ack"));
        report.capture_scope = Some(scope.clone());
        self.capture_scopes
            .lock()
            .unwrap()
            .retain(|old| old != &scope);
        vec![report]
    }
}

#[tokio::test]
async fn scoped_confirmation_resolves_only_its_connection_and_dismissal_keeps_a_live_sibling() {
    let home = tempfile::tempdir().unwrap();
    let rt = runtime(home.path()).await;
    let source = Arc::new(ScopedFamily {
        capture_scopes: Mutex::new(vec!["connection-a".into()]),
        stopped_scope: Mutex::new(None),
    });
    rt.register_sensor_source(source.clone()).await.unwrap();
    stop(&rt).await;
    // The same device ID reconnects; a new connection can only confirm itself.
    *source.capture_scopes.lock().unwrap() = vec!["connection-b".into()];
    *source.stopped_scope.lock().unwrap() = Some("connection-b".into());
    stop(&rt).await;
    assert_eq!(
        rt.unresolved_stops().await.len(),
        1,
        "old A remains visible after B confirms itself"
    );
    *source.capture_scopes.lock().unwrap() = vec!["connection-c".into()];
    *source.stopped_scope.lock().unwrap() = None;
    stop(&rt).await;
    let visible = rt.unresolved_stops().await.remove(0);
    rt.dismiss_unresolved_stop(&visible.source_id, visible.generation, "human")
        .await
        .unwrap();
    assert!(
        rt.unresolved_stops().await.is_empty(),
        "only active C remains; A was human-dismissed"
    );
    source.capture_scopes.lock().unwrap().clear();
    assert_eq!(
        rt.unresolved_stops().await.len(),
        1,
        "dismissing A must not delete C's write-ahead reminder"
    );
    rt.shutdown().await;
}

#[tokio::test]
async fn a_scoped_successful_stop_leaves_no_restart_reminder() {
    let home = tempfile::tempdir().unwrap();
    let rt = runtime(home.path()).await;
    let source = Arc::new(ScopedFamily {
        capture_scopes: Mutex::new(vec!["connection-a".into()]),
        stopped_scope: Mutex::new(Some("connection-a".into())),
    });
    rt.register_sensor_source(source).await.unwrap();
    stop(&rt).await;
    assert!(rt.unresolved_stops().await.is_empty());
    rt.shutdown().await;
    drop(rt);
    let rt = runtime(home.path()).await;
    assert!(
        rt.unresolved_stops().await.is_empty(),
        "positive evidence must still resolve the exact connection"
    );
    rt.shutdown().await;
}
