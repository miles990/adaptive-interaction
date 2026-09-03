//! 緊急停止的動器階段必須是**平行**的，而且沒確認的不得算成已停止。
//!
//! 為什麼：一台拔線的裝置上常掛好幾個動器（參考 ESP32 spec 就是 1 受器＋
//! 4 動器共用同一條 serial link）。逐一等待時，每個動器各吃滿 2 秒逾時，
//! 光這一台就把其他裝置（手機、其他板子）的 stop-all 卡在後面 8 秒——
//! 緊急停止不能讓其他裝置排隊。

use async_trait::async_trait;
use interaction_adapter_sdk::ActuatorManifestBuilder;
use interaction_core::*;
use interaction_runtime::{Runtime, RuntimeOptions};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 一個「停不下來」的動器：estop 送出去了，但裝置遲遲不確認
/// （比 runtime 給每個動器的 2 秒上限還久）。
struct StubbornActuator {
    id: String,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Actuator for StubbornActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new(&self.id, "Stubborn", "haptic", "test")
            .risk(RiskClass::Low)
            .requires_consent(false)
            .build()
    }

    async fn execute(&self, _action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::Unavailable("not used in this test".into()))
    }

    async fn status(&self) -> ComponentHealth {
        ComponentHealth::healthy()
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        Err(ActuatorError::NotFound(action_id.to_string()))
    }

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // 裝置沒有回應 stop-all 的 ack：超過 runtime 的 2 秒上限。
        tokio::time::sleep(Duration::from_secs(3)).await;
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_actuator_phase_of_an_emergency_stop_is_bounded_and_parallel() {
    let home = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(home.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();

    // 同一台「拔線裝置」上的四個動器。
    let calls = Arc::new(AtomicUsize::new(0));
    for n in 0..4 {
        rt.registry
            .register_actuator(Arc::new(StubbornActuator {
                id: format!("deadlink.act{n}"),
                calls: calls.clone(),
            }))
            .await
            .unwrap();
    }
    let total = rt.registry.all_actuator_instances().await.len();

    let started = Instant::now();
    let payload = rt
        .emergency_stop("test", Some("drill".into()))
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "每個動器都要被要求停止（不能因為前面慢就跳過）"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "四個各 2 秒逾時的動器若是逐一等待就要 8 秒；平行才會在 ~2 秒內結束（實測 {elapsed:?}）"
    );
    // 沒有在窗內確認的動器**不得**算成已停止（誠實階梯：未確認≠已停止）。
    let stopped = payload["stoppedActuators"].as_u64().unwrap_or(u64::MAX) as usize;
    assert!(
        stopped <= total.saturating_sub(4),
        "逾時未確認的 4 個動器不得被算成已停止（stopped={stopped}, total={total}）"
    );
}
