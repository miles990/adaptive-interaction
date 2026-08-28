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

use crate::protocol::{DeviceLink, DeviceMsg, LinkError, RawLink};
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
        ComponentHealth::healthy().at(Utc::now())
    }

    async fn stop(&self) -> Result<(), ReceptorError> {
        Ok(())
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
                Err(LinkError::Unavailable(detail)) => {
                    last_err = detail;
                }
            }
        }
        Ok(DriverReceipt::start(&action, Utc::now())
            .failed("device-unreachable", &last_err)
            .finish())
    }

    async fn status(&self) -> ComponentHealth {
        ComponentHealth::healthy().at(Utc::now())
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

    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        self.link
            .stop_all(Duration::from_millis(1_500))
            .await
            .map_err(|e| ActuatorError::Unavailable(format!("stop-all not deliverable: {e}")))
    }
}
