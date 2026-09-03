//! v0.5.1 review wave 4 regressions around consent and the dispatch gates.
//!
//! 1. 真正的「只這一次」consent（`maxUses`／`remainingUses`）：第一次成功授權
//!    就消耗掉，第二個 plan 必須被 Governor 擋下；派工前的最後一道閘門不得
//!    把「剛剛才合法消耗掉最後一次配額」的那個動作誤判成被撤銷。
//! 2. `simulate_plan` 也要查 provider 閘門：被停用的 provider 底下的能力，
//!    dry run 不得回 `wouldExecute = true`。

use async_trait::async_trait;
use interaction_adapter_sdk::{ActuatorManifestBuilder, DriverReceipt};
use interaction_core::*;
use interaction_policy::ActionSource;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

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

// ---------------------------------------------------------------------------
// simulate_plan 也要查 provider 閘門
// ---------------------------------------------------------------------------

#[tokio::test]
async fn simulate_plan_blocks_when_the_owning_provider_is_not_operational() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("simulate".into()), None, vec![])
        .await
        .unwrap();

    let mut intent = SemanticIntent::new("success");
    intent.preferred_channels = vec!["conversation".into()];
    let plan = rt
        .create_plan(
            intent,
            vec!["conversation".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap();
    let before = rt.simulate_plan(&plan.plan_id).await.unwrap();
    assert!(before.would_execute, "provider 還開著時本來就會執行");

    // 擁有 conversation 動器的 provider 被停用：run_step 已經會擋，dry run
    // 也必須說實話，否則「模擬結果」會誤導 AI／使用者。
    rt.transition_provider(
        &ProviderId::new("provider.local.builtin"),
        ProviderState::Disabled,
    )
    .await
    .unwrap();

    let after = rt.simulate_plan(&plan.plan_id).await.unwrap();
    assert!(
        !after.would_execute,
        "停用中的 provider 不得回 wouldExecute=true：{after:?}"
    );
    assert_eq!(after.steps[0].outcome, AuthorizationOutcome::Blocked);
    let reason = after.steps[0]
        .decisions
        .iter()
        .find_map(|d| match d {
            PolicyDecision::Blocked { rule, reason } if rule == "provider.not-operational" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a provider block: {:?}", after.steps[0].decisions));
    assert!(reason.contains("provider.local.builtin"), "{reason}");
}

// ---------------------------------------------------------------------------
// 真正的「只這一次」consent
// ---------------------------------------------------------------------------

/// 動器需要同意、走 haptic 頻道，執行只到 dispatched（留下 open receipt，
/// 方便檢查失敗路徑也不歸還配額）。`fail` 讓 driver 直接回錯。
struct ConsentProbeActuator {
    fail: bool,
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl Actuator for ConsentProbeActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new("one-shot.probe", "One-shot probe", "haptic", "test")
            .risk(RiskClass::Low)
            .requires_consent(true)
            .supports_cancel(true)
            .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(ActuatorError::Rejected("device said no".into()));
        }
        Ok(DriverReceipt::start(&action, chrono::Utc::now())
            .dispatched()
            .acknowledged()
            .finish())
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

async fn one_shot_runtime(fail: bool) -> (tempfile::TempDir, Runtime, Arc<AtomicUsize>) {
    let (dir, rt) = runtime().await;
    rt.start_session(Some("one-shot".into()), None, vec![])
        .await
        .unwrap();
    rt.update_policy(json!({
        "allowedChannels": ["conversation", "haptic"],
        "actuatorAllowlist": ["conversation", "local-log", "one-shot.probe"],
        "channelLimits": {"haptic": {"enabled": true}}
    }))
    .await
    .unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    rt.registry
        .register_actuator(Arc::new(ConsentProbeActuator {
            fail,
            executions: executions.clone(),
        }))
        .await
        .unwrap();
    // requires_consent 的動器預設關閉（安全預設）；人類先打開它。
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("one-shot.probe"), true)
        .await
        .unwrap();
    (dir, rt, executions)
}

async fn probe_plan(rt: &Runtime) -> PlanId {
    let mut intent = SemanticIntent::new("presence");
    intent.preferred_channels = vec!["haptic".into()];
    rt.create_plan(
        intent,
        vec!["one-shot.probe".into()],
        1,
        1,
        false,
        None,
        BTreeMap::new(),
    )
    .await
    .unwrap()
    .plan_id
}

async fn run_probe(rt: &Runtime) -> ActionReceipt {
    let plan_id = probe_plan(rt).await;
    rt.execute_plan(&plan_id, ActionSource::ExplicitRequest, false)
        .await
        .unwrap()
        .remove(0)
}

fn blocked_on_consent(receipt: &ActionReceipt) -> bool {
    receipt.current_status == ActionStatus::Blocked
        && receipt.policy_decisions.iter().any(
            |d| matches!(d, PolicyDecision::Blocked { rule, .. } if rule == "consent.required"),
        )
}

fn remaining_uses(rt_session: &Session, scope: &ConsentScope) -> Option<u32> {
    rt_session
        .consents
        .iter()
        .find(|c| &c.scope == scope)
        .and_then(|c| c.remaining_uses)
}

#[tokio::test]
async fn one_shot_consent_is_spent_by_the_first_dispatch_and_blocks_the_next_plan() {
    let (_g, rt, executions) = one_shot_runtime(false).await;
    let scope = ConsentScope::Actuator("one-shot.probe".into());
    rt.grant_consent_with_uses("actuator:one-shot.probe", None, Some(1))
        .await
        .unwrap();

    // 第一次：真的派工出去（沒有在 pre_dispatch_gate 自我阻塞）。
    let first = run_probe(&rt).await;
    assert!(!blocked_on_consent(&first), "第一次必須通過：{first:?}");
    assert!(
        !first.policy_decisions.iter().any(
            |d| matches!(d, PolicyDecision::Blocked { rule, .. } if rule == "consent.pre-dispatch")
        ),
        "剛消耗掉最後一次配額的動作不得被自己的閘門擋下：{:?}",
        first.policy_decisions
    );
    assert_eq!(executions.load(Ordering::SeqCst), 1, "driver 應該真的收到");

    // 第二個 plan：同意已經用完 ⇒ Governor 擋。
    let second = run_probe(&rt).await;
    assert!(blocked_on_consent(&second), "用過就要失效：{second:?}");
    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "第二次不得抵達 driver"
    );

    let session = rt.current_session().await.unwrap();
    assert_eq!(remaining_uses(&session, &scope), Some(0));
    assert!(!session.has_consent(&scope, chrono::Utc::now()));
}

#[tokio::test]
async fn a_spent_one_shot_consent_stays_spent_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_path_buf();
    let opts = || RuntimeOptions {
        home: Some(home.clone()),
        acquire_lock: false,
        in_memory_db: false,
        spawn_watchdog: false,
    };
    let executions = Arc::new(AtomicUsize::new(0));
    {
        let rt = Runtime::start(opts()).await.unwrap();
        rt.start_session(Some("one-shot".into()), None, vec![])
            .await
            .unwrap();
        rt.update_policy(json!({
            "allowedChannels": ["conversation", "haptic"],
            "actuatorAllowlist": ["conversation", "local-log", "one-shot.probe"],
            "channelLimits": {"haptic": {"enabled": true}}
        }))
        .await
        .unwrap();
        rt.registry
            .register_actuator(Arc::new(ConsentProbeActuator {
                fail: false,
                executions: executions.clone(),
            }))
            .await
            .unwrap();
        rt.registry
            .set_actuator_enabled(&ActuatorId::new("one-shot.probe"), true)
            .await
            .unwrap();
        rt.grant_consent_with_uses("actuator:one-shot.probe", None, Some(1))
            .await
            .unwrap();
        assert!(!blocked_on_consent(&run_probe(&rt).await));
        rt.shutdown().await;
    }

    let rt = Runtime::start(opts()).await.unwrap();
    rt.registry
        .register_actuator(Arc::new(ConsentProbeActuator {
            fail: false,
            executions: executions.clone(),
        }))
        .await
        .unwrap();
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("one-shot.probe"), true)
        .await
        .unwrap();
    let session = rt
        .current_session()
        .await
        .expect("session must be restored");
    let scope = ConsentScope::Actuator("one-shot.probe".into());
    assert_eq!(
        remaining_uses(&session, &scope),
        Some(0),
        "重啟不得把用掉的次數還回來"
    );
    let after = run_probe(&rt).await;
    assert!(blocked_on_consent(&after), "{after:?}");
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    rt.shutdown().await;
}

#[tokio::test]
async fn a_genuine_revocation_between_authorize_and_dispatch_still_blocks() {
    // 修掉「自我阻塞」之後，pre_dispatch_gate 對『真的被撤銷』仍必須擋。
    let (_g, rt, executions) = one_shot_runtime(false).await;
    rt.grant_consent_with_uses("actuator:one-shot.probe", None, Some(1))
        .await
        .unwrap();
    // 直接撤銷（模擬授權與派工之間人類按了撤回）：下一次執行不得抵達 driver。
    rt.revoke_consent("actuator:one-shot.probe").await.unwrap();
    let receipt = run_probe(&rt).await;
    assert!(blocked_on_consent(&receipt), "{receipt:?}");
    assert_eq!(executions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failed_dispatch_does_not_refund_the_one_shot_use() {
    // 設計決定：同意在 Governor 判定 Authorized 的瞬間就已經被行使，
    // driver 之後失敗也不歸還——否則「只這一次」會變成「每次成功一次」。
    let (_g, rt, executions) = one_shot_runtime(true).await;
    rt.grant_consent_with_uses("actuator:one-shot.probe", None, Some(1))
        .await
        .unwrap();
    let first = run_probe(&rt).await;
    assert_eq!(first.current_status, ActionStatus::Failed, "{first:?}");
    assert_eq!(executions.load(Ordering::SeqCst), 1);

    let scope = ConsentScope::Actuator("one-shot.probe".into());
    let session = rt.current_session().await.unwrap();
    assert_eq!(remaining_uses(&session, &scope), Some(0), "失敗不歸還");
    let second = run_probe(&rt).await;
    assert!(blocked_on_consent(&second), "{second:?}");
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_plans_racing_one_shot_consent_only_let_one_through() {
    let (_g, rt, executions) = one_shot_runtime(false).await;
    rt.grant_consent_with_uses("actuator:one-shot.probe", None, Some(1))
        .await
        .unwrap();

    let mut handles = Vec::new();
    for _ in 0..8 {
        let plan_id = probe_plan(&rt).await;
        let cloned = rt.clone_handle();
        handles.push(tokio::spawn(async move {
            cloned
                .execute_plan(&plan_id, ActionSource::ExplicitRequest, false)
                .await
                .unwrap()
                .remove(0)
        }));
    }
    let mut allowed = 0;
    for handle in handles {
        let receipt = handle.await.unwrap();
        if !blocked_on_consent(&receipt) {
            allowed += 1;
        }
    }
    assert_eq!(
        allowed,
        1,
        "authorization_lock 內的原子消耗必須讓並行只過一個；receipts: {:?}",
        rt.list_actions(None, 100).unwrap()
    );
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_consent_without_max_uses_is_still_unlimited_within_its_ttl() {
    let (_g, rt, executions) = one_shot_runtime(false).await;
    rt.grant_consent("actuator:one-shot.probe", None)
        .await
        .unwrap();
    for round in 0..3 {
        let receipt = run_probe(&rt).await;
        assert!(!blocked_on_consent(&receipt), "第 {round} 次：{receipt:?}");
    }
    assert_eq!(executions.load(Ordering::SeqCst), 3);
    let scope = ConsentScope::Actuator("one-shot.probe".into());
    assert_eq!(
        remaining_uses(&rt.current_session().await.unwrap(), &scope),
        None
    );
}

#[tokio::test]
async fn max_uses_zero_is_refused_instead_of_creating_a_dead_consent() {
    let (_g, rt, _executions) = one_shot_runtime(false).await;
    let err = rt
        .grant_consent_with_uses("actuator:one-shot.probe", None, Some(0))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)), "{err:?}");
    let session = rt.current_session().await.unwrap();
    assert!(session.consents.is_empty(), "不得留下一筆永遠用不了的同意");
}

/// 高風險動器即使沒有標 `requiresConsent`，Governor 也是靠「動器範圍的人類
/// 同意」當作核准（policy 步驟 7）。那筆同意如果是一次性的，也必須被用掉，
/// 否則「只這一次」在高風險路徑上就是空話。
struct HighRiskActuator {
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl Actuator for HighRiskActuator {
    fn manifest(&self) -> ActuatorManifest {
        ActuatorManifestBuilder::new("high-risk.probe", "High risk probe", "haptic", "test")
            .risk(RiskClass::High)
            .requires_consent(false)
            .build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Ok(DriverReceipt::start(&action, chrono::Utc::now())
            .dispatched()
            .acknowledged()
            .finish())
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

#[tokio::test]
async fn one_shot_consent_used_as_high_risk_approval_is_also_spent() {
    let (_g, rt) = runtime().await;
    rt.start_session(Some("one-shot".into()), None, vec![])
        .await
        .unwrap();
    rt.update_policy(json!({
        "allowedChannels": ["conversation", "haptic"],
        "actuatorAllowlist": ["conversation", "local-log", "high-risk.probe"],
        "channelLimits": {"haptic": {"enabled": true}},
        "initiative": "active"
    }))
    .await
    .unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    rt.registry
        .register_actuator(Arc::new(HighRiskActuator {
            executions: executions.clone(),
        }))
        .await
        .unwrap();
    rt.registry
        .set_actuator_enabled(&ActuatorId::new("high-risk.probe"), true)
        .await
        .unwrap();
    rt.grant_consent_with_uses("actuator:high-risk.probe", None, Some(1))
        .await
        .unwrap();

    async fn plan_of(rt: &Runtime) -> PlanId {
        let mut intent = SemanticIntent::new("presence");
        intent.preferred_channels = vec!["haptic".into()];
        rt.create_plan(
            intent,
            vec!["high-risk.probe".into()],
            1,
            1,
            false,
            None,
            BTreeMap::new(),
        )
        .await
        .unwrap()
        .plan_id
    }

    let first_plan = plan_of(&rt).await;
    let first = rt
        .execute_plan(&first_plan, ActionSource::ExplicitRequest, false)
        .await
        .unwrap()
        .remove(0);
    assert!(
        !first.is_terminal() || first.current_status != ActionStatus::Blocked,
        "{first:?}"
    );
    assert_eq!(executions.load(Ordering::SeqCst), 1);

    let second_plan = plan_of(&rt).await;
    let second = rt
        .execute_plan(&second_plan, ActionSource::ExplicitRequest, false)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(second.current_status, ActionStatus::Blocked, "{second:?}");
    assert!(
        second.policy_decisions.iter().any(|d| matches!(
            d,
            PolicyDecision::ApprovalRequired { rule, .. } if rule == "risk.approval"
        )),
        "第二次必須回到「需要人類核准」：{:?}",
        second.policy_decisions
    );
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

/// safety-invariants-058：`maxUses` 只有動器派工路徑（動器／頻道範圍）真的會
/// 扣減。受器與 tool-operation 沒有等價的原子消耗點，因此不得照收一個後端
/// 永遠不會強制的次數——否則事件、audit 與 session JSON 都在回報一個假的
/// 「只這一次」。
#[tokio::test]
async fn max_uses_is_refused_for_scopes_where_nothing_ever_spends_it() {
    let (_g, rt, _executions) = one_shot_runtime(false).await;

    for scope in [
        "receptor:microphone.listen",
        "receptor:iphone.mic-level",
        "tool:interaction.observe",
    ] {
        let err = rt
            .grant_consent_with_uses(scope, Some(5), Some(1))
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Validation(_)),
            "{scope} 的 maxUses 沒有任何地方會扣減，必須在授權時就拒絕：{err:?}"
        );
        let session = rt.current_session().await.unwrap();
        assert!(
            !session
                .consents
                .iter()
                .any(|c| c.max_uses.is_some() && c.revoked_at.is_none()),
            "被拒絕的授權不得留下任何帶 maxUses 的同意：{:?}",
            session.consents
        );
    }

    // 同樣的範圍改成純 TTL（不帶 maxUses）仍然照常可以授權——修法不得順手
    // 拿掉受器的短效授權能力。
    let session = rt
        .grant_consent_with_uses("receptor:microphone.listen", Some(5), None)
        .await
        .unwrap();
    let mic = session
        .consents
        .iter()
        .find(|c| c.scope == ConsentScope::Receptor("microphone.listen".into()))
        .expect("純 TTL 的受器授權必須成立");
    assert_eq!(mic.max_uses, None);
    assert_eq!(mic.remaining_uses, None);

    // 真的會被扣減的範圍不受影響。
    rt.grant_consent_with_uses("actuator:one-shot.probe", None, Some(1))
        .await
        .expect("動器範圍的 maxUses 是後端真的會用掉的，必須維持可用");
    rt.grant_consent_with_uses("channel:haptic", None, Some(1))
        .await
        .expect("頻道範圍的 maxUses 也會在派工時用掉");
}
