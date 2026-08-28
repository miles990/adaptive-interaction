//! Link 傳輸（serial/mqtt/ble）共用的 Receptor / Actuator 實作。
//!
//! 誠實階梯落地：
//! - execute：send 失敗＝failed；送達且裝置 ack＝dispatched→acknowledged
//!   （附裝置回報的 applied 值，展示韌體端 clamp）；送出後 ack 逾時＝
//!   dispatched＋`ackTimeout`（絕不重送、絕不冒充 acknowledged）——runtime
//!   watchdog 會把它誠實標為 uncertain。
//! - read：向裝置請求 state，逾時＝Unavailable（不用舊值冒充新觀察）。
//! - cancel：真的送 cancel 到裝置；只有裝置 ack 才回收據。
//! - estop：stop-all 直送裝置。
//! - health／status：**不得硬編 healthy**。健康度＝傳輸狀態＋握手狀態
//!   （＋actuator 的能力宣告）。裝置拔線／broker 斷線／被 disable 關閉
//!   一律 offline；連上但還沒握手是 degraded（首次讀取／命令時才握手，
//!   若在此回 offline，availability gate 會反過來讓握手永遠不會發生）。

use crate::protocol::{DeviceLink, DeviceMsg, LinkError, LinkReadiness, RawLink};
use crate::{qualified_id, substitute, CapabilitySpec, CommandSpec, RetrySpec};
use async_trait::async_trait;
use chrono::Utc;
use interaction_adapter_sdk::{ActuatorManifestBuilder, DriverReceipt, ReceptorManifestBuilder};
use interaction_core::{
    ActionId, ActionReceipt, Actuator, ActuatorError, BoundedAction, ComponentHealth, HumanMeta,
    Observation, Receptor, ReceptorError, ReceptorId, ReceptorMode, RiskClass, Sensitivity,
    SessionContext,
};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const RECENT_ACTIONS_CAP: usize = 32;
const APPLIED_NOTE_MAX_BYTES: usize = 2048;

pub struct LinkReceptor<L: RawLink> {
    pub spec: CapabilitySpec,
    pub adapter_id: String,
    pub link: Arc<DeviceLink<L>>,
    pub transport_label: &'static str,
}

#[async_trait]
impl<L: RawLink + 'static> Receptor for LinkReceptor<L> {
    fn manifest(&self) -> interaction_core::ReceptorManifest {
        let mut b = ReceptorManifestBuilder::new(
            &qualified_id(&self.adapter_id, &self.spec.id),
            self.spec.name.as_deref().unwrap_or(&self.spec.id),
            &format!("declarative.{}", self.adapter_id),
        )
        .description(self.spec.description.as_deref().unwrap_or(""))
        .category(self.spec.category.as_deref().unwrap_or("device"))
        .mode(ReceptorMode::Poll)
        .sensitivity(Sensitivity::Internal, self.spec.requires_consent)
        .refresh_interval_ms(self.spec.poll_interval_ms.unwrap_or(30_000));
        if let Some(h) = &self.spec.human {
            b = b.human(h.clone());
        }
        let keys: Vec<&str> = self.spec.facts.keys().map(String::as_str).collect();
        b.provides(&keys).build()
    }

    async fn start(&self, _context: SessionContext) -> Result<(), ReceptorError> {
        Ok(())
    }

    async fn read(&self) -> Result<Observation, ReceptorError> {
        let timeout =
            Duration::from_millis(self.spec.timeout_ms.unwrap_or(5_000).clamp(100, 60_000));
        let state = self
            .link
            .read_state(timeout)
            .await
            .map_err(|e| ReceptorError::Unavailable(format!("{} {e}", self.transport_label)))?;
        let mut obs = Observation::now(
            ReceptorId::new(qualified_id(&self.adapter_id, &self.spec.id)),
            format!("declarative.{}", self.adapter_id),
            Utc::now(),
        );
        for (fact, pointer) in &self.spec.facts {
            if let Some(v) = state.pointer(pointer) {
                obs.facts.insert(fact.clone(), v.clone());
            }
        }
        Ok(obs)
    }

    async fn health(&self) -> ComponentHealth {
        link_health(&self.link, self.transport_label).at(Utc::now())
    }

    async fn stop(&self) -> Result<(), ReceptorError> {
        Ok(())
    }
}

/// 傳輸＋握手狀態 → 誠實健康度（receptor/actuator 共用）。
fn link_health<L: RawLink>(link: &DeviceLink<L>, transport: &str) -> ComponentHealth {
    match link.readiness() {
        LinkReadiness::Ready => ComponentHealth::healthy(),
        LinkReadiness::NotHandshaken => ComponentHealth::degraded(format!(
            "{transport} 已連線，但尚未完成 hello/pair 握手（首次讀取／命令時進行）"
        )),
        LinkReadiness::Connecting => {
            ComponentHealth::degraded(format!("{transport} 連線中（尚未連上裝置）"))
        }
        LinkReadiness::Disconnected => ComponentHealth::offline(format!(
            "裝置未連線／未握手（{transport} 連不上：拔線、被占用或位址不對）"
        )),
        LinkReadiness::Closed => ComponentHealth::offline(format!(
            "{transport} 連線已由主機關閉（provider 被 disable／revoke）"
        )),
    }
}

pub struct LinkActuator<L: RawLink> {
    pub spec: CapabilitySpec,
    pub command: CommandSpec,
    pub adapter_id: String,
    pub link: Arc<DeviceLink<L>>,
    pub transport_label: &'static str,
    /// 最近送出的 actions（cancel 需要原 action 才能建誠實收據）。
    pub recent: Mutex<VecDeque<(String, BoundedAction)>>,
}

impl<L: RawLink> LinkActuator<L> {
    pub fn new(
        spec: CapabilitySpec,
        command: CommandSpec,
        adapter_id: String,
        link: Arc<DeviceLink<L>>,
        transport_label: &'static str,
    ) -> Self {
        Self {
            spec,
            command,
            adapter_id,
            link,
            transport_label,
            recent: Mutex::new(VecDeque::new()),
        }
    }
}

fn bounded_applied_note(applied: &Value) -> Value {
    let text = applied.to_string();
    if text.len() > APPLIED_NOTE_MAX_BYTES {
        json!({"truncated": true})
    } else {
        applied.clone()
    }
}

#[async_trait]
impl<L: RawLink + 'static> Actuator for LinkActuator<L> {
    fn manifest(&self) -> interaction_core::ActuatorManifest {
        let mut b = ActuatorManifestBuilder::new(
            &qualified_id(&self.adapter_id, &self.spec.id),
            self.spec.name.as_deref().unwrap_or(&self.spec.id),
            self.spec.channel.as_deref().unwrap_or("device"),
            &format!("declarative.{}", self.adapter_id),
        )
        .description(self.spec.description.as_deref().unwrap_or(""))
        .risk(self.spec.risk.unwrap_or(RiskClass::BoundedSideEffect))
        .external(self.spec.external_side_effect)
        .requires_consent(true); // 外部裝置輸出一律 consent-gated
        if let Some(h) = &self.spec.human {
            b = b.human(h.clone());
        } else {
            use interaction_core::{ConfirmationLevel, EffectSemantics, TriState};
            b = b.human(HumanMeta {
                effect: Some(EffectSemantics {
                    // link 傳輸有真實 ack：最深誠實等級是 acknowledged。
                    confirmation_level: ConfirmationLevel::Acknowledged,
                    external_side_effect: TriState::Unknown,
                    physical_effect: TriState::Unknown,
                    reversible: TriState::Unknown,
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        b.build()
    }

    async fn execute(&self, action: BoundedAction) -> Result<ActionReceipt, ActuatorError> {
        if action.expires_at <= Utc::now() {
            return Err(ActuatorError::Rejected("action expired".into()));
        }
        let params = self
            .command
            .params
            .as_ref()
            .map(|template| substitute(template, &action))
            .unwrap_or_else(|| json!({}));
        let timeout =
            Duration::from_millis(self.spec.timeout_ms.unwrap_or(5_000).clamp(100, 60_000));
        // Retry 只適用「送出失敗」（Unavailable）——送出成功後 ack 逾時
        // 絕不重送（實體效果不得重複觸發）。
        let retry = self.spec.retry.clone().unwrap_or(RetrySpec {
            attempts: 1,
            backoff_ms: 0,
        });
        {
            let mut recent = self.recent.lock().await;
            if recent.len() >= RECENT_ACTIONS_CAP {
                recent.pop_front();
            }
            recent.push_back((action.action_id.as_str().to_string(), action.clone()));
        }
        let mut last_err = String::new();
        for attempt in 0..retry.attempts.max(1) {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(retry.backoff_ms)).await;
            }
            match self
                .link
                .command(
                    action.action_id.as_str(),
                    &self.command.name,
                    params.clone(),
                    timeout,
                )
                .await
            {
                Ok(DeviceMsg::Ack { applied, dup, .. }) => {
                    let mut receipt = DriverReceipt::start(&action, Utc::now())
                        .dispatched()
                        .note("transport", json!(self.transport_label))
                        .acknowledged();
                    if let Some(applied) = applied {
                        // 裝置回報的實際套用值（韌體硬限制 clamp 後）——
                        // 有界記錄，讓人看見「要求 vs 實際」。
                        receipt = receipt.note("deviceApplied", bounded_applied_note(&applied));
                    }
                    if dup == Some(true) {
                        receipt = receipt.note("deduplicated", json!(true));
                    }
                    return Ok(receipt.finish());
                }
                Ok(DeviceMsg::Err { reason, .. }) => {
                    // 裝置明確拒絕（rate-limited / not-paired / 超界）＝失敗。
                    return Ok(DriverReceipt::start(&action, Utc::now())
                        .dispatched()
                        .note("transport", json!(self.transport_label))
                        .failed("device-refused", &reason)
                        .finish());
                }
                Ok(_) => {
                    last_err = "unexpected device reply".into();
                }
                Err(LinkError::Timeout(detail)) => {
                    // 已送出、無 ack：結果未知。不重送、不冒充失敗或成功。
                    return Ok(DriverReceipt::start(&action, Utc::now())
                        .dispatched()
                        .note("transport", json!(self.transport_label))
                        .note("ackTimeout", json!(true))
                        .note("detail", json!(detail))
                        .finish());
                }
                Err(LinkError::Refused(detail)) => {
                    return Ok(DriverReceipt::start(&action, Utc::now())
                        .failed("device-identity-or-pairing", &detail)
                        .finish());
                }
                Err(LinkError::NotAdvertised(detail)) => {
                    // 裝置沒宣告這個能力：cmd 從未送出，沒有實體效果，
                    // 也不重試（重試不會讓裝置長出新能力）。
                    return Ok(DriverReceipt::start(&action, Utc::now())
                        .note("transport", json!(self.transport_label))
                        .failed("capability-not-advertised", &detail)
                        .finish());
                }
                Err(LinkError::Uncertain(detail)) => {
                    // 送出「途中」失敗（例如 BLE write 已寫出但沒有回應）：
                    // 是否送達未知 → 不重試（重試會重複實體效果）、
                    // 也不冒充失敗。runtime watchdog 會標成 uncertain。
                    return Ok(DriverReceipt::start(&action, Utc::now())
                        .dispatched()
                        .note("transport", json!(self.transport_label))
                        .note("sendOutcomeUnknown", json!(true))
                        .note("detail", json!(detail))
                        .finish());
                }
                Err(LinkError::Reset(detail)) => {
                    // 等待中連線重置：命令可能已到裝置，結果未知；
                    // 佇列裡的舊命令已被清掉，絕不重送。
                    return Ok(DriverReceipt::start(&action, Utc::now())
                        .dispatched()
                        .note("transport", json!(self.transport_label))
                        .note("outcomeUnknown", json!(true))
                        .failed("link-reset", &detail)
                        .finish());
                }
                Err(LinkError::Unavailable(detail)) => {
                    // 「確定未送出」（連線未開、佇列滿、握手沒完成）才可重試。
                    last_err = detail;
                }
            }
        }
        Ok(DriverReceipt::start(&action, Utc::now())
            .failed("device-unreachable", &last_err)
            .finish())
    }

    async fn status(&self) -> ComponentHealth {
        // 裝置在 hello.caps 明確沒宣告這個能力＝這台裝置做不到，
        // 不論連線多健康都不是可用的動器。
        if self.link.advertises(&self.command.name) == Some(false) {
            return ComponentHealth::offline(format!("裝置未宣告此能力：{}", self.command.name))
                .at(Utc::now());
        }
        link_health(&self.link, self.transport_label).at(Utc::now())
    }

    async fn cancel(&self, action_id: &ActionId) -> Result<ActionReceipt, ActuatorError> {
        let action = {
            let recent = self.recent.lock().await;
            recent
                .iter()
                .find(|(id, _)| id == action_id.as_str())
                .map(|(_, a)| a.clone())
        };
        let Some(action) = action else {
            return Err(ActuatorError::NotFound(format!(
                "{action_id}: not a recent {} action on this adapter",
                self.transport_label
            )));
        };
        match self
            .link
            .cancel(action_id.as_str(), Duration::from_millis(2_000))
            .await
        {
            Ok(DeviceMsg::Ack {
                cancelled: Some(true),
                ..
            }) => Ok(DriverReceipt::start(&action, Utc::now())
                .dispatched()
                .note("cancelRequested", json!(true))
                .note("deviceCancelled", json!(true))
                .acknowledged()
                .finish()),
            Ok(_) => Err(ActuatorError::NotFound(format!(
                "{action_id}: device reports no cancellable effect"
            ))),
            Err(e) => Err(ActuatorError::Unavailable(format!(
                "cancel could not be confirmed: {e} — effect state UNKNOWN"
            ))),
        }
    }

    /// estop：送 stop-all 並等裝置 ack。裝置沒 ack ＝「已送出／未確認」——
    /// 誠實回 Err，runtime 的 stoppedActuators 不得把它算成已停止。
    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        self.link
            .stop_all(Duration::from_millis(1_000))
            .await
            .map_err(|e| match e {
                LinkError::Timeout(detail) | LinkError::Reset(detail) => {
                    ActuatorError::Unavailable(format!("{detail} ({})", self.transport_label))
                }
                other => ActuatorError::Unavailable(format!("stop-all not deliverable: {other}")),
            })
    }
}
