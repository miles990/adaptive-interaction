//! 規劃器 × 桌面角色迴歸測試（v0.4 對抗審查）：
//! 隱藏／未連線的 companion actuator 不得被規劃器選中（確定性拒絕
//! dispatch 的目標不進 plan），且註冊當下健康即誠實（不等第一次
//! watchdog refresh）；open 選擇不得挑需要呼叫者參數的 actuator
//! （animation／behaviorIntent），只落在真的可交付的 bubble；
//! 隱藏狀態在 status 與能力清單上仍看得見（誠實呈現不退化）。

use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use std::collections::BTreeMap;

async fn runtime() -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(RuntimeOptions {
        home: Some(dir.path().to_path_buf()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    })
    .await
    .unwrap();
    (dir, rt)
}

fn open_intent(message: Option<&str>) -> SemanticIntent {
    let mut intent = SemanticIntent::new("companion-open");
    intent.preferred_channels = vec!["desktop-pet".into()];
    intent.message = message.map(|s| s.to_string());
    intent
}

#[tokio::test]
async fn headless_startup_lists_companion_actuators_offline() {
    let (_g, rt) = runtime().await;
    // 未曾 hello：註冊當下即以實際健康入快取——headless daemon 不會有
    // 「第一次 refresh 前短暫可規劃」的窗口。
    let snap = rt
        .capabilities(&DiscoveryContext {
            include_unavailable: true,
            ..Default::default()
        })
        .await;
    for id in [
        "companion.bubble.show",
        "companion.animation.play",
        "companion.state.present",
    ] {
        let m = snap
            .actuators
            .iter()
            .find(|a| a.id.as_str() == id)
            .unwrap_or_else(|| panic!("{id} listed"));
        assert_eq!(
            m.availability,
            Availability::Offline,
            "{id} must be offline before any companion hello"
        );
    }

    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();
    let plan = rt
        .create_plan(
            open_intent(Some("哈囉")),
            vec![],
            0,
            1,
            true,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert!(
        plan.steps.iter().all(|s| s.channel != "desktop-pet"),
        "headless daemon must never plan onto desktop-pet: {:?}",
        plan.steps
    );
}

#[tokio::test]
async fn hidden_companion_not_planned_but_still_visible_in_status() {
    let (_g, rt) = runtime().await;
    // 已連線但隱藏。
    rt.presentation_hello(false, None).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();

    // 能力清單誠實：隱藏 → Offline，且原因看得出是「隱藏」不是「斷線」。
    let snap = rt
        .capabilities(&DiscoveryContext {
            include_unavailable: true,
            ..Default::default()
        })
        .await;
    let bubble = snap
        .actuators
        .iter()
        .find(|a| a.id.as_str() == "companion.bubble.show")
        .expect("bubble listed");
    assert_eq!(bubble.availability, Availability::Offline);
    assert_eq!(
        bubble.health.message.as_deref(),
        Some("companion hidden"),
        "hidden state must stay distinguishable from disconnected"
    );

    // presence 呈現不退化：status 仍顯示已連線、不可見。
    let status = rt.presentation_status();
    assert_eq!(status.get("connected").unwrap().as_bool(), Some(true));
    assert_eq!(status.get("visible").unwrap().as_bool(), Some(false));

    // open 選擇不再把頻道名額分給必然拒絕 dispatch 的 actuator，
    // 其他頻道照常交付。
    let plan = rt
        .create_plan(
            open_intent(Some("被隱藏了")),
            vec![],
            0,
            1,
            true,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert!(
        plan.steps.iter().all(|s| s.channel != "desktop-pet"),
        "hidden companion must not occupy a channel slot: {:?}",
        plan.steps
    );
    assert!(
        !plan.steps.is_empty(),
        "other channels must still deliver while the companion is hidden"
    );
}

#[tokio::test]
async fn open_selection_picks_message_capable_companion_step() {
    let (_g, rt) = runtime().await;
    rt.presentation_hello(true, Some("shu-agile".into())).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();

    let plan = rt
        .create_plan(
            open_intent(Some("開放選擇的氣泡")),
            vec![],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(plan.steps.len(), 1, "rejected={:?}", plan.rejected);
    assert_eq!(plan.steps[0].channel, "desktop-pet");
    // BTreeMap 順序下 alphabetically-first 的 animation.play 缺 `animation`
    // 參數必然失敗——open 選擇必須落在可帶訊息的 bubble。
    assert_eq!(
        plan.steps[0].actuator_id.as_str(),
        "companion.bubble.show",
        "open selection must land on the message-capable companion actuator"
    );
    for id in ["companion.animation.play", "companion.state.present"] {
        assert!(
            plan.rejected
                .iter()
                .any(|r| r.actuator_id.as_str() == id
                    && r.reason.contains("missing required parameter")),
            "{id} must be rejected with an explainable reason: {:?}",
            plan.rejected
        );
    }

    // 選出的 step 執行不會因缺參數而失敗：誠實走到 Dispatched（等 ack）。
    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0].current_status,
        ActionStatus::Dispatched,
        "errors={:?}",
        receipts[0].errors
    );
}

#[tokio::test]
async fn open_selection_without_message_skips_bubble() {
    let (_g, rt) = runtime().await;
    rt.presentation_hello(true, None).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();

    // 靜默策略：沒有可供 bubble 的訊息 → bubble 也不可合成，
    // desktop-pet 不得出現在 steps（否則 dispatch 必然被拒）。
    let strategy = MessageStrategy {
        mode: MessageMode::None,
        allow_silence: true,
        ..Default::default()
    };
    let plan = rt
        .create_plan(
            open_intent(None),
            vec![],
            0,
            2,
            true,
            Some(strategy),
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert!(
        plan.steps
            .iter()
            .all(|s| !s.actuator_id.as_str().starts_with("companion.")),
        "no message → no companion step: {:?}",
        plan.steps
    );
    assert!(
        plan.rejected.iter().any(|r| {
            r.actuator_id.as_str() == "companion.bubble.show"
                && r.reason.contains("missing required parameter message")
        }),
        "bubble must be rejected for the missing message: {:?}",
        plan.rejected
    );
}

#[tokio::test]
async fn explicitly_named_candidate_keeps_execute_time_rejection() {
    let (_g, rt) = runtime().await;
    rt.presentation_hello(true, None).await;
    rt.start_session(Some("t".into()), None, vec![])
        .await
        .unwrap();

    // 指名 candidates 不經 open-selection 參數閘：缺參數的 animation.play
    // 仍會被規劃進去，並在執行期以 receipt 誠實說明被拒原因。
    let mut intent = SemanticIntent::new("companion-test");
    intent.preferred_channels = vec!["desktop-pet".into()];
    let plan = rt
        .create_plan(
            intent,
            vec!["companion.animation.play".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(plan.steps.len(), 1);
    let receipts = rt
        .execute_plan(&plan.plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap();
    assert_eq!(receipts[0].current_status, ActionStatus::Failed);
    assert!(receipts[0]
        .errors
        .iter()
        .any(|e| e.message.contains("animation name is required")));
}
