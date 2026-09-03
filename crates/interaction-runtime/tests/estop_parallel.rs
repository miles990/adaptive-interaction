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

/// 一個立刻確認停止的動器（裝置有回 ack）。
struct ConfirmingActuator {
    id: String,
}

#[async_trait]
impl Actuator for ConfirmingActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new(&self.id, "Confirming", "haptic", "test")
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
        Ok(())
    }
}

/// 一個明白說「我停不了」的動器（driver 直接回錯）。
struct RefusingActuator {
    id: String,
}

#[async_trait]
impl Actuator for RefusingActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new(&self.id, "Refusing", "haptic", "test")
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
        Err(ActuatorError::Unavailable("serial link is gone".into()))
    }
}

/// link-transports-027：有動器沒回確認時，緊急停止不得對人或 AI 說
/// 「所有輸出已中止」，而且事件／audit payload 必須逐一列出未確認的動器。
#[tokio::test(flavor = "multi_thread")]
async fn an_emergency_stop_with_unconfirmed_actuators_never_claims_every_output_halted() {
    let home = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(home.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();

    let calls = Arc::new(AtomicUsize::new(0));
    // 一台拔線裝置（不回 ack）＋一個明說停不了的動器＋一個確認停了的動器。
    rt.registry
        .register_actuator(Arc::new(StubbornActuator {
            id: "deadlink.silent".into(),
            calls: calls.clone(),
        }))
        .await
        .unwrap();
    rt.registry
        .register_actuator(Arc::new(RefusingActuator {
            id: "deadlink.refuses".into(),
        }))
        .await
        .unwrap();
    rt.registry
        .register_actuator(Arc::new(ConfirmingActuator {
            id: "good.confirms".into(),
        }))
        .await
        .unwrap();

    let total_registered = rt.registry.all_actuator_instances().await.len();
    let payload = rt
        .emergency_stop("test", Some("drill".into()))
        .await
        .unwrap();

    // (a) 呼叫端不必事先知道總數就能看出誤差。
    let total = payload["totalActuators"]
        .as_u64()
        .expect("payload 必須列出動器總數（否則看不出 stoppedActuators 的誤差）")
        as usize;
    assert_eq!(total, total_registered, "totalActuators 必須是實際動器數量");
    let stopped = payload["stoppedActuators"]
        .as_u64()
        .expect("stoppedActuators") as usize;

    // (b) 未確認的動器要逐一列出（id＋原因），不能只剩一個裸數字。
    let unconfirmed = payload["unconfirmedActuators"]
        .as_array()
        .expect("payload 必須逐一列出未確認的動器")
        .clone();
    assert_eq!(
        unconfirmed.len(),
        2,
        "逾時未回 ack 與明說停不了的動器都必須列進未確認：{payload}"
    );
    assert_eq!(
        stopped + unconfirmed.len(),
        total,
        "已確認＋未確認必須等於總數：{payload}"
    );
    let ids: Vec<&str> = unconfirmed
        .iter()
        .filter_map(|u| u["actuatorId"].as_str())
        .collect();
    assert!(
        ids.contains(&"deadlink.silent") && ids.contains(&"deadlink.refuses"),
        "未確認清單必須指名是哪些動器：{ids:?}"
    );
    for entry in &unconfirmed {
        assert!(
            entry["detail"].as_str().is_some_and(|d| !d.is_empty()),
            "每個未確認動器都要帶原因（逾時／driver 回錯）：{entry}"
        );
        let outcome = entry["outcome"].as_str().unwrap_or_default();
        assert!(
            outcome == "unconfirmed" || outcome == "failed",
            "未確認動器的 outcome 必須誠實分類：{entry}"
        );
    }

    // (c) 給人與 AI 的那句話不得是無條件的「所有輸出已中止」。
    let message = rt
        .outbox
        .recent(20)
        .into_iter()
        .find(|m| m.intent == "emergency-stop")
        .expect("緊急停止要推一則訊息進 outbox");
    let text = message.text.unwrap_or_default();
    assert!(
        !text.contains("所有輸出已中止"),
        "有 2 個動器沒確認停止時不得宣稱所有輸出已中止：{text}"
    );
    assert!(text.contains('2'), "摘要必須說出未確認的動器台數：{text}");
}

/// 反面：全部都確認停了的時候，那句話要維持肯定，不得改成過度保留的措辭。
#[tokio::test(flavor = "multi_thread")]
async fn an_emergency_stop_that_every_actuator_confirmed_still_says_so_plainly() {
    let home = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(home.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    rt.registry
        .register_actuator(Arc::new(ConfirmingActuator {
            id: "good.confirms".into(),
        }))
        .await
        .unwrap();

    let payload = rt.emergency_stop("test", None).await.unwrap();
    assert_eq!(
        payload["unconfirmedActuators"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(usize::MAX),
        0,
        "全部確認時未確認清單必須是空的：{payload}"
    );
    assert_eq!(
        payload["stoppedActuators"], payload["totalActuators"],
        "全部確認時兩個數字必須相等：{payload}"
    );
    let text = rt
        .outbox
        .recent(20)
        .into_iter()
        .find(|m| m.intent == "emergency-stop")
        .and_then(|m| m.text)
        .expect("緊急停止要推一則訊息進 outbox");
    assert!(
        text.contains("所有輸出已中止"),
        "全部確認停止時要照實說：{text}"
    );
}
