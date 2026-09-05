//! Link 傳輸（serial/mqtt/ble）共用的 Receptor / Actuator 實作。
//!
//! 誠實階梯落地：
//! - execute：send 失敗＝failed；送達且裝置 ack＝dispatched→acknowledged
//!   （附裝置回報的 applied 值，展示韌體端 clamp）；送出後 ack 逾時／等待中
//!   重連＝dispatched＋`outcomeUnknown`（絕不重送、絕不冒充 acknowledged、
//!   也絕不冒充 failed）——runtime 讀到 `outcomeUnknown` 就立刻標成 uncertain
//!   （executor），watchdog 再對過了 ack 期限仍停在 dispatched 的收據兜底。
//!   Dispatched 的時間戳＝命令真正離開這裡的時刻（不是等待結束的時刻）。
//! - read：向裝置請求 state，逾時＝Unavailable（不用舊值冒充新觀察）。
//! - cancel：真的送 cancel 到裝置；只有裝置 ack 才回收據。
//! - estop：stop-all 直送裝置。
//! - health／status：**不得硬編 healthy**。健康度＝傳輸狀態＋握手狀態
//!   （＋actuator 的能力宣告）。裝置拔線／broker 斷線／被 disable 關閉
//!   一律 offline；連上但還沒握手是 degraded（首次讀取／命令時才握手，
//!   若在此回 offline，availability gate 會反過來讓握手永遠不會發生）。

use crate::protocol::{DeviceLink, DeviceMsg, LinkError, LinkReadiness, RawLink};
use crate::{
    qualified_id, substitute, unresolved_facts_detail, CapabilitySpec, CommandSpec, RetrySpec,
};
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
/// estop 的 stop-all ack 等待窗。runtime 對每個 actuator 的 emergency_stop
/// 以 2 秒為上限（runtime.rs），這裡取同一個值——不能更短：參考韌體在
/// broker 不通時 `maintainMqtt()` 的 `connect()` 會同步阻塞最多 ≈1.5s
/// （TCP connect 500ms＋等 CONNACK 1s），期間 Serial／BLE 上的 stop-all 要等
/// 阻塞結束才被處理；舊版只等 1s，真板上 estop 會被誤記為 UNCONFIRMED
/// （假陰性，發生在最不該有雜訊的路徑）。
const STOP_ALL_ACK_WINDOW: Duration = Duration::from_millis(2_000);

/// `LinkError::Refused` 的收據原因。身分／配對被拒是一類，訊息超過裝置
/// 單則上限（serial／MQTT 639 bytes、BLE 480 bytes）是另一類——兩者都
/// 「確定沒送出」，但混成同一個原因會誤導使用者去查配對碼。傳輸層對後者
/// 一律以 "message too large" 標示（serial.rs／mqtt.rs／ble.rs 同一慣例）。
fn refusal_reason(detail: &str) -> &'static str {
    if detail.contains("message too large") {
        "message-too-large"
    } else if detail.starts_with("pairing-locked") {
        // 配對鎖定期：裝置**沒有比對**這次的碼（正確的碼也一樣被擋）。
        // 混進 device-identity-or-pairing 會叫使用者去改一個其實正確的碼。
        "pairing-locked"
    } else {
        "device-identity-or-pairing"
    }
}

/// 裝置對 cancel 回的 err 裡，唯一代表「確定沒有這個效果在跑」的原因。
/// 逐字比對（不是 `contains`）：這一句會變成收據與 audit 上的確定結論，
/// 寧可把沒把握的原因落到 UNKNOWN，也不要多認一個。
fn is_no_such_effect(reason: &str) -> bool {
    reason.trim().eq_ignore_ascii_case("not-found")
}

/// 非 ack／err 的回覆在診斷訊息裡的稱呼（不揭露內容，只說是哪一種）。
fn describe_reply(msg: &DeviceMsg) -> &'static str {
    match msg {
        DeviceMsg::Ack { .. } => "an ack that does not confirm the cancel",
        DeviceMsg::Hello { .. } => "hello",
        DeviceMsg::PairOk => "pair-ok",
        DeviceMsg::PairFail { .. } => "pair-fail",
        DeviceMsg::State { .. } => "state",
        DeviceMsg::Err { .. } => "err",
        // 線協定 v1.1 的 AIP envelope：它不是任何 cmd/cancel 的回覆，只是
        // 剛好在同一條線上經過（角色 session 的訊息流）。
        DeviceMsg::Aip { .. } => "an aip frame (not a reply to this request)",
    }
}

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
        // 一個 fact 都沒解出來＝這次沒有讀到任何資料（韌體欄位改名、pointer
        // 打錯、裝置回了空的 facts）。回 Ok 會讓 runtime 把它當成一次成功的
        // 觀察、把 provider 記成「已測試」，並把一筆零 fact 的觀察寫進 store
        // 與事件流——那是拿 metadata 冒充資料。
        if obs.facts.is_empty() {
            return Err(ReceptorError::Unavailable(unresolved_facts_detail(
                self.transport_label,
                &self.spec.facts,
                &state,
            )));
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
        // 傳輸還連著，但裝置本身沉默太久：不得繼續說「此刻真的能用它」。
        LinkReadiness::Stale { silent_ms } => ComponentHealth::degraded(format!(
            "{transport} 連線還在，但已 {} 秒沒聽到裝置（可能斷電或離線；狀態未知）",
            silent_ms / 1_000
        )),
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
        .requires_consent(true) // 外部裝置輸出一律 consent-gated
        // 裝置安全上限：spec 的 `limits:` 直接進 manifest，Policy Governor 的
        // min(AI 請求, 使用者偏好, session 限制, **裝置安全上限**, 剩餘預算)
        // 才有「裝置」那一項——不能只靠韌體自己 clamp。
        .limits(self.spec.limits.clone());
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
            // 真正的送出時刻。收據要在 command() 回來之後才建（結果決定
            // 狀態），但 Dispatched 的時間戳必須是「命令離開這裡的那一刻」，
            // 不是 ack 等待結束的時刻——watchdog 用它的年齡判斷「已送出、
            // 一直沒 ack」，蓋成結束時刻年齡就永遠 ≈0，收據會卡在 dispatched。
            let sent_at = Utc::now();
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
                    let mut receipt = DriverReceipt::start(&action, sent_at)
                        .dispatched_at(sent_at)
                        .note("transport", json!(self.transport_label))
                        .acknowledged();
                    // spec 有配對碼，但裝置說它不需要配對、而且這條 link 從沒
                    // 見過它比對碼＝無法證明那組碼被比對過（參考韌體
                    // PAIRING_CODE 為空時對任何碼都回 pair-ok）。收據要說出來：
                    // 這張收據的身分證據只有裝置自報的 deviceId。
                    if self.link.pairing_unverified() {
                        receipt = receipt.note("pairingUnverified", json!(true));
                    } else if self.link.pairing_not_recompared() {
                        // 中間態：這條 link 先前真的比對過碼，但這次握手裝置
                        // 說「這條通道已經配對」而沒有重比。不是未驗證，
                        // 也不該假裝這次剛比對過。
                        receipt = receipt.note("pairingNotRecompared", json!(true));
                    }
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
                    return Ok(DriverReceipt::start(&action, sent_at)
                        .dispatched_at(sent_at)
                        .note("transport", json!(self.transport_label))
                        .failed("device-refused", &reason)
                        .finish());
                }
                Ok(_) => {
                    last_err = "unexpected device reply".into();
                }
                Err(LinkError::Timeout(detail)) => {
                    // 已送出、無 ack：結果未知。不重送、不冒充失敗或成功。
                    // outcomeUnknown 是 runtime 判讀「這張收據的結果不明」的
                    // 統一旗標（executor 立刻標 uncertain；watchdog 兜底）。
                    return Ok(DriverReceipt::start(&action, sent_at)
                        .dispatched_at(sent_at)
                        .note("transport", json!(self.transport_label))
                        .note("ackTimeout", json!(true))
                        .note("outcomeUnknown", json!(true))
                        .note("detail", json!(detail))
                        .finish());
                }
                Err(LinkError::Refused(detail)) => {
                    // 確定沒送出（身分／配對被拒，或訊息超過裝置單則上限）：
                    // 不標 dispatched、不重試。
                    return Ok(DriverReceipt::start(&action, Utc::now())
                        .note("transport", json!(self.transport_label))
                        .failed(refusal_reason(&detail), &detail)
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
                    // 也不冒充失敗。executor／watchdog 會標成 uncertain。
                    return Ok(DriverReceipt::start(&action, sent_at)
                        .dispatched_at(sent_at)
                        .note("transport", json!(self.transport_label))
                        .note("sendOutcomeUnknown", json!(true))
                        .note("outcomeUnknown", json!(true))
                        .note("detail", json!(detail))
                        .finish());
                }
                Err(LinkError::Reset(detail)) => {
                    // 等待中連線重置：命令可能已到裝置，結果**未知**；
                    // 佇列／inflight 裡的舊命令已被清掉，絕不重送。
                    // 未知不是失敗：標 failed 會讓人與 AI 合理地重下同一
                    // 命令＝重複實體效果。收據停在 dispatched＋outcomeUnknown，
                    // 由 runtime 判成 uncertain（誠實階梯：結果未知→uncertain）。
                    return Ok(DriverReceipt::start(&action, sent_at)
                        .dispatched_at(sent_at)
                        .note("transport", json!(self.transport_label))
                        .note("outcomeUnknown", json!(true))
                        .note("reason", json!("link-reset"))
                        .note("detail", json!(detail))
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
            // 「沒有可取消的效果」是一個**確定結論**，只有裝置真的表過態
            // （韌體：解析成功、確定沒有這個 id 在跑＝`not-found`）才能講。
            Ok(DeviceMsg::Err { reason, .. }) if is_no_such_effect(&reason) => {
                Err(ActuatorError::NotFound(format!(
                    "{action_id}: device reports no cancellable effect ({reason})"
                )))
            }
            // 其餘的裝置錯誤代表「這則 cancel 沒被處理」——`busy`（BLE 入站
            // 佇列滿，write 在解析前就被丟掉，震動／蜂鳴仍在跑）、`not-paired`
            // （裝置重開機）、`rate-limited`……效果狀態一律 UNKNOWN，
            // 不得翻成裝置從未做過的宣稱。
            Ok(DeviceMsg::Err { reason, .. }) => Err(ActuatorError::Unavailable(format!(
                "{action_id}: the device refused the cancel ({reason}) \
                 — the effect state is UNKNOWN"
            ))),
            Ok(other) => Err(ActuatorError::Unavailable(format!(
                "{action_id}: unexpected device reply to the cancel ({}) \
                 — the effect state is UNKNOWN",
                describe_reply(&other)
            ))),
            Err(e) => Err(ActuatorError::Unavailable(format!(
                "cancel could not be confirmed: {e} — effect state UNKNOWN"
            ))),
        }
    }

    /// estop：送 stop-all 並等裝置 ack（窗口見 STOP_ALL_ACK_WINDOW）。裝置沒 ack
    /// ＝「已送出／未確認」——誠實回 Err，runtime 的 stoppedActuators 不得把它
    /// 算成已停止。
    async fn emergency_stop(&self) -> Result<(), ActuatorError> {
        self.link
            .stop_all(STOP_ALL_ACK_WINDOW)
            .await
            .map_err(|e| match e {
                LinkError::Timeout(detail) | LinkError::Reset(detail) => {
                    ActuatorError::Unavailable(format!("{detail} ({})", self.transport_label))
                }
                other => ActuatorError::Unavailable(format!("stop-all not deliverable: {other}")),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 訊息超過裝置單則上限與身分／配對被拒都是 Refused，但收據原因必須分開
    /// （前者不該讓人去查配對碼）。三種傳輸的超長 detail 都以 "message too
    /// large" 標示。
    #[test]
    fn refusal_reasons_distinguish_oversize_from_identity() {
        assert_eq!(
            refusal_reason("message too large (700 bytes > 639); nothing was written"),
            "message-too-large"
        );
        assert_eq!(
            refusal_reason("ble message too large (500 bytes > 480)"),
            "message-too-large"
        );
        assert_eq!(
            refusal_reason("pairing code rejected by device"),
            "device-identity-or-pairing"
        );
        assert_eq!(
            refusal_reason("device identity mismatch: expected \"a\", got \"b\""),
            "device-identity-or-pairing"
        );
    }

    /// estop 的 ack 窗口 = runtime 每個 actuator 的 estop 上限（2s），
    /// 且必須蓋過韌體 MQTT 重連時的最壞阻塞（≈1.5s）。
    #[test]
    fn stop_all_window_covers_the_firmware_reconnect_block() {
        assert!(STOP_ALL_ACK_WINDOW >= Duration::from_millis(2_000));
        assert!(STOP_ALL_ACK_WINDOW > Duration::from_millis(1_500));
    }
}
