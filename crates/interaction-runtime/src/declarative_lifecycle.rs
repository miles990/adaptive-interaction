//! 宣告式裝置綁定的**顯式**生命週期（AIP 1.0 澄清／v0.7.0）。
//!
//! 為什麼要有這個模組：在此之前，「這台宣告式裝置的綁定還在不在」只是一個
//! 布林集合（`declarative_rebind_pending`）。布林說得出「不在」，說不出
//! 「為什麼不在」，也說不出「正在回來的路上」——於是把 provider 轉回
//! `Available` 只能誠實地印一句「要重新啟動才會重新綁定」，使用者得重開
//! daemon 才拿得回一台自己剛剛停用又啟用的裝置。
//!
//! 現在狀態是顯式的：
//!
//! - [`DeclarativeLifecycle::Bound`]：連線、能力宣告、SensorSource 都在。
//! - [`DeclarativeLifecycle::Rebinding`]：正在重新綁定（帶世代；晚到的完成
//!   回呼依世代拒絕）。這段期間 `ProviderState` 是 `Disconnected`——誠實：
//!   還沒連上，不是「可用」。
//! - [`DeclarativeLifecycle::Unbound`]：綁定已經拆掉，並且說得出原因
//!   （[`UnboundReason`]）。`Revoked`／`Removed` **永遠不重新綁定**。
//!
//! 不變量：
//! - 誠實階梯：ProviderState 不得先於實際連線。握手 Ready 之前一律
//!   `Disconnected`；rebind 失敗就留在 `Disconnected` 並說得出原因。
//! - 撤銷不復活：`Unbound { Revoked | Removed }` 與 store 裡的
//!   `provider_off_reason == "revoked"` 都會讓 rebind 拒絕開始／拒絕收斂。
//! - 有界：登記表有上限、每一步等待都有 deadline、整個 rebind 有總預算。
//! - 世代：一次 rebind 只屬於一個世代。世代被取代（又停用了、又按了一次
//!   啟用、被撤銷）之後，舊 rebind 不得再改任何狀態。

use crate::runtime::{Runtime, RuntimeInner};
use crate::sensor_source::upgrade;
use interaction_adapter_declarative::protocol::{DeviceAipChannel, LinkReadiness};
use interaction_adapter_declarative::DeclarativeSpec;
use interaction_core::{ActuatorId, ProviderId, ProviderState, ReceptorId};
use interaction_registry::providers::provider_stopped;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

/// 同時記得多少台宣告式裝置的綁定。設定檔裝置由人類自己放進 `config/adapters`，
/// 但這張表仍然不得無界成長：超過就誠實拒絕記錄（那台裝置仍然會被註冊，
/// 只是不支援免重啟重新綁定，並留稽核）。
pub const MAX_DECLARATIVE_BINDINGS: usize = 64;

/// 舊連線「進行中的請求」收斂的預算。過了就誠實記 `drained: false`——
/// 不假裝那些請求有結果。
const REBIND_DRAIN_BUDGET: Duration = Duration::from_secs(3);
/// 收斂輪詢窗（不忙碌等待）。
const REBIND_POLL: Duration = Duration::from_millis(50);
/// 新連線握手的等待上限。等不到就是 rebind 失敗（誠實：沒連上）。
const REBIND_HANDSHAKE_BUDGET: Duration = Duration::from_secs(20);
/// 整個 rebind 的總預算（watchdog）。任何一步卡住都不得變成無限等待。
const REBIND_TOTAL_BUDGET: Duration = Duration::from_secs(40);

/// 綁定被拆掉的原因。誠實：`Disabled`／`Disconnected` 可以重新綁定，
/// `Revoked`／`Removed` 不行（重新綁定會讓撤銷失效）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnboundReason {
    /// 人類停用了這個 provider（disabled／closed／expired）。
    Disabled,
    /// 連線沒了（拔線、broker 斷、adapter 自己收工）。
    Disconnected,
    /// 人類撤銷了它。**不得**重新綁定。
    Revoked,
    /// spec／provider 記錄被移除。**不得**重新綁定。
    Removed,
}

impl UnboundReason {
    pub fn as_str(self) -> &'static str {
        match self {
            UnboundReason::Disabled => "disabled",
            UnboundReason::Disconnected => "disconnected",
            UnboundReason::Revoked => "revoked",
            UnboundReason::Removed => "removed",
        }
    }

    /// 這個原因允許免重啟重新綁定嗎？（撤銷／移除永遠不行。）
    pub fn rebindable(self) -> bool {
        matches!(self, UnboundReason::Disabled | UnboundReason::Disconnected)
    }
}

/// 一台宣告式裝置的綁定狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclarativeLifecycle {
    /// 連線＋能力宣告＋SensorSource 都在。
    Bound,
    /// 正在重新綁定；`generation` 是這一次嘗試的世代。
    Rebinding { generation: u64 },
    /// 綁定已拆掉，並說得出原因。
    Unbound { reason: UnboundReason },
}

impl DeclarativeLifecycle {
    pub fn label(&self) -> String {
        match self {
            DeclarativeLifecycle::Bound => "bound".into(),
            DeclarativeLifecycle::Rebinding { generation } => format!("rebinding#{generation}"),
            DeclarativeLifecycle::Unbound { reason } => format!("unbound:{}", reason.as_str()),
        }
    }
}

/// 一台宣告式裝置在 runtime 這一側的完整記錄。
pub(crate) struct DeclarativeBinding {
    /// 這台裝置的 spec（File=Truth 的那一份的記憶體副本）。重新綁定要重新
    /// 驗證它——不重新驗證就等於用一份沒人再看過的設定重開實體連線。
    pub(crate) spec: DeclarativeSpec,
    pub(crate) lifecycle: DeclarativeLifecycle,
    /// 上一次綁定開出來的 AIP 通道（**弱**參照：這張表不替連線續命）。
    /// 重新綁定用它確認舊連線真的收斂、真的關掉了。
    pub(crate) channels: Vec<Weak<dyn DeviceAipChannel>>,
    /// 已經開始過幾次 rebind。晚到的完成回呼依世代拒絕。
    pub(crate) generation: u64,
    /// 這台裝置被停用**之前**就已經關著的受器（＝人類自己關的）。
    ///
    /// 為什麼要記：重新綁定的第 7 步是「重新註冊整份 spec」，而 registry 對
    /// 新註冊的受器一律套用 manifest 的預設值——不需 consent 的受器預設**啟用**。
    /// 沒有這份紀錄的話，「停用裝置 → 再啟用」會順手打開一個人類先前手動關掉
    /// 的受器，那是一次沒有人下過的決定，也違反既有保證「回到 Available 不會
    /// 自動恢復任何能力」。有界：只可能是這份 spec 自己宣告的受器 id。
    pub(crate) human_disabled_receptors: Vec<String>,
}

impl Runtime {
    /// 綁定成立（或重新成立）：記住 spec 與這一次開出來的通道。
    ///
    /// 表滿了不會讓裝置註冊失敗（那台裝置照常有能力），只是它失去「免重啟
    /// 重新綁定」的能力——誠實留稽核，不靜默。
    pub(crate) fn note_declarative_bound(
        &self,
        provider_id: &str,
        spec: &DeclarativeSpec,
        channels: &[Arc<dyn DeviceAipChannel>],
    ) {
        let weak: Vec<Weak<dyn DeviceAipChannel>> = channels.iter().map(Arc::downgrade).collect();
        let mut full = false;
        if let Ok(mut map) = self.declarative_bindings.lock() {
            if !map.contains_key(provider_id) && map.len() >= MAX_DECLARATIVE_BINDINGS {
                full = true;
            } else {
                let entry =
                    map.entry(provider_id.to_string())
                        .or_insert_with(|| DeclarativeBinding {
                            spec: spec.clone(),
                            lifecycle: DeclarativeLifecycle::Bound,
                            channels: Vec::new(),
                            generation: 0,
                            human_disabled_receptors: Vec::new(),
                        });
                entry.spec = spec.clone();
                entry.channels = weak;
                // 重新綁定進行中的話，狀態**不得**在這裡就翻成 `Bound`：連線
                // 建起來只是第 7 步，握手還沒成立。提早宣告成功會讓後面那一步
                // 的失敗變成「被別人接手」，於是真正的失敗永遠不留痕。
                if !matches!(entry.lifecycle, DeclarativeLifecycle::Rebinding { .. }) {
                    entry.lifecycle = DeclarativeLifecycle::Bound;
                }
            }
        }
        if full {
            let _ = self.store.audit(
                "provider.rebind-not-tracked",
                "runtime",
                &json!({
                    "providerId": provider_id,
                    "limit": MAX_DECLARATIVE_BINDINGS,
                    "reason": "the declarative binding table is full; this device will need a restart to rebind",
                }),
            );
        }
    }

    /// 綁定被拆掉了（停用／撤銷／連線消失／移除）。只記錄已知的裝置。
    pub(crate) fn note_declarative_unbound(&self, provider_id: &str, reason: UnboundReason) {
        if let Ok(mut map) = self.declarative_bindings.lock() {
            if let Some(entry) = map.get_mut(provider_id) {
                entry.lifecycle = DeclarativeLifecycle::Unbound { reason };
                entry.channels.retain(|w| w.strong_count() > 0);
            }
        }
    }

    /// 綁定被拆掉，但只在它目前還是 `Bound` 時才改（呼叫端已經知道更精確的
    /// 原因時，不得被這個較弱的原因蓋掉）。
    pub(crate) fn note_declarative_unbound_if_bound(
        &self,
        provider_id: &str,
        reason: UnboundReason,
    ) {
        if let Ok(mut map) = self.declarative_bindings.lock() {
            if let Some(entry) = map.get_mut(provider_id) {
                if entry.lifecycle == DeclarativeLifecycle::Bound {
                    entry.lifecycle = DeclarativeLifecycle::Unbound { reason };
                }
            }
        }
    }

    /// 這台裝置目前的綁定狀態（沒有記錄＝不是宣告式裝置）。
    pub fn declarative_lifecycle(&self, provider_id: &str) -> Option<DeclarativeLifecycle> {
        self.declarative_bindings
            .lock()
            .ok()
            .and_then(|map| map.get(provider_id).map(|e| e.lifecycle.clone()))
    }

    /// 這台裝置的 spec 是不是還記得（免重啟重新綁定的前提）。
    pub(crate) fn declarative_spec_of(&self, provider_id: &str) -> Option<DeclarativeSpec> {
        self.declarative_bindings
            .lock()
            .ok()
            .and_then(|map| map.get(provider_id).map(|e| e.spec.clone()))
    }

    /// 這台裝置**上一次**綁定開出來、現在還活著的通道。
    fn declarative_previous_channels(&self, provider_id: &str) -> Vec<Arc<dyn DeviceAipChannel>> {
        self.declarative_bindings
            .lock()
            .ok()
            .map(|map| {
                map.get(provider_id)
                    .map(|e| e.channels.iter().filter_map(Weak::upgrade).collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    /// spec／provider 記錄整個消失：留一個墓碑（`Unbound { Removed }`）並丟掉
    /// 通道參照——**不得**再重新綁定（`UnboundReason::rebindable()` 為 false）。
    ///
    /// 為什麼留墓碑而不是整筆刪掉：整筆刪掉之後 `declarative_lifecycle` 會回
    /// `None`，而 `None` 的意思是「這道閘門對它一無所知」（例如綁定表滿了），
    /// 不是「它被移除了」。兩者對入站 frame 的判斷相反，不能共用同一個表示。
    ///
    /// 誠實範圍：今天沒有任何 production 路徑呼叫它——spec 檔被刪掉只會在下一次
    /// 啟動時「沒有被載入」（那時整張表本來就是空的）。runtime 沒有偵測檔案刪除
    /// 的能力，沒有的東西不假裝有；這個入口是給主機（與測試）明確表態用的。
    pub fn note_declarative_removed(&self, provider_id: &str) {
        if let Ok(mut map) = self.declarative_bindings.lock() {
            if let Some(entry) = map.get_mut(provider_id) {
                entry.lifecycle = DeclarativeLifecycle::Unbound {
                    reason: UnboundReason::Removed,
                };
                entry.channels.clear();
            }
        }
    }

    /// 「允許再試一次」：把可重新綁定的 `Unbound` 推進 `Rebinding{generation}`。
    ///
    /// 回 `None` ＝不該（也不會）開始：不是宣告式裝置、已經在 rebind、已經
    /// 綁著、或原因是撤銷／移除（撤銷不得復活）。
    pub(crate) fn begin_declarative_rebind(&self, provider_id: &str) -> Option<u64> {
        let mut map = self.declarative_bindings.lock().ok()?;
        let entry = map.get_mut(provider_id)?;
        match entry.lifecycle {
            DeclarativeLifecycle::Unbound { reason } if reason.rebindable() => {
                entry.generation = entry.generation.saturating_add(1);
                let generation = entry.generation;
                entry.lifecycle = DeclarativeLifecycle::Rebinding { generation };
                Some(generation)
            }
            _ => None,
        }
    }

    /// 記下「這台裝置被停用之前，人類已經關掉的受器」。停用路徑在**翻旗標
    /// 之前**呼叫它——之後就分不出「人類關的」與「停用順手關的」了。
    pub(crate) fn note_declarative_human_disabled(&self, provider_id: &str, ids: Vec<String>) {
        if let Ok(mut map) = self.declarative_bindings.lock() {
            if let Some(entry) = map.get_mut(provider_id) {
                entry.human_disabled_receptors = ids;
            }
        }
    }

    /// 讀那份紀錄（**不**取走）。
    ///
    /// 為什麼不是 take：重新註冊 spec 有可能失敗，而失敗時受器已經被
    /// unregister 掉了——紀錄先被取走的話，下一次重新綁定就會用預設值把它們
    /// 全部打開。清除交給 [`Runtime::clear_declarative_human_disabled`]，
    /// 只在真的套用成功之後做。
    pub(crate) fn declarative_human_disabled(&self, provider_id: &str) -> Vec<String> {
        self.declarative_bindings
            .lock()
            .ok()
            .and_then(|map| {
                map.get(provider_id)
                    .map(|entry| entry.human_disabled_receptors.clone())
            })
            .unwrap_or_default()
    }

    /// 清掉那份紀錄（已經套用回 registry 了）。留著的話，之後一次由**斷線**
    /// 觸發的重新綁定會拿一份過期的名單去關掉人類早就重新打開的受器。
    pub(crate) fn clear_declarative_human_disabled(&self, provider_id: &str) {
        if let Ok(mut map) = self.declarative_bindings.lock() {
            if let Some(entry) = map.get_mut(provider_id) {
                entry.human_disabled_receptors.clear();
            }
        }
    }

    /// 這一次 rebind 還是目前這一輪嗎？（晚到的回呼依世代拒絕。）
    fn declarative_rebind_current(&self, provider_id: &str, generation: u64) -> bool {
        matches!(
            self.declarative_lifecycle(provider_id),
            Some(DeclarativeLifecycle::Rebinding { generation: g }) if g == generation
        )
    }

    /// rebind 結束：成功＝`Bound`（`note_declarative_bound` 已經寫過），
    /// 失敗＝退回 `Unbound { Disconnected }`——**只有**世代仍然成立時才改。
    fn settle_declarative_rebind(&self, provider_id: &str, generation: u64, ok: bool) -> bool {
        let Ok(mut map) = self.declarative_bindings.lock() else {
            return false;
        };
        let Some(entry) = map.get_mut(provider_id) else {
            return false;
        };
        match entry.lifecycle {
            DeclarativeLifecycle::Rebinding { generation: g } if g == generation => {
                entry.lifecycle = if ok {
                    DeclarativeLifecycle::Bound
                } else {
                    DeclarativeLifecycle::Unbound {
                        reason: UnboundReason::Disconnected,
                    }
                };
                true
            }
            _ => false,
        }
    }

    /// 啟動一次有界的背景重新綁定。
    ///
    /// 刻意**不**放進 `transition_provider` 內文：那條路徑是人類按下「啟用」
    /// 的同步回應，不該被一條要等握手的連線拖住；而且狀態必須在回應之前就
    /// 誠實地變成 `Disconnected`（尚未連上），不是「可用」。
    pub(crate) fn spawn_declarative_rebind(&self, provider_id: &str, generation: u64) {
        let weak = self.weak_inner();
        let provider_id = provider_id.to_string();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + REBIND_TOTAL_BUDGET;
            let outcome = tokio::time::timeout_at(
                deadline,
                run_declarative_rebind(&weak, &provider_id, generation),
            )
            .await;
            let failure = match outcome {
                Ok(Ok(())) => None,
                Ok(Err(reason)) => Some(reason),
                Err(_) => Some(RebindStop::Failed(format!(
                    "the rebind exceeded its bounded budget ({} s); nothing was assumed to have \
                     connected",
                    REBIND_TOTAL_BUDGET.as_secs()
                ))),
            };
            let Some(stop) = failure else { return };
            let Some(rt) = upgrade(&weak) else { return };
            match stop {
                // 被別的決定接手（又停用了／又按了一次啟用／被撤銷）：這一輪
                // 什麼都不改，只留痕——它沒有失敗，它只是不再是現在的答案。
                RebindStop::Superseded(reason) => {
                    let _ = rt.store.audit(
                        "provider.rebind-superseded",
                        "runtime",
                        &json!({
                            "providerId": provider_id,
                            "generation": generation,
                            "reason": reason,
                        }),
                    );
                }
                RebindStop::Failed(reason) => {
                    rt.fail_declarative_rebind(&provider_id, generation, &reason)
                        .await
                }
            }
        });
    }

    /// 還沒真的開始就放棄這一輪（例如狀態機不允許進 `Disconnected`）：把世代
    /// 收回去，不留一個沒有人推進的 `Rebinding`。
    pub(crate) async fn abandon_declarative_rebind(
        &self,
        provider_id: &str,
        generation: u64,
        reason: &str,
    ) {
        if !self.settle_declarative_rebind(provider_id, generation, false) {
            return;
        }
        let _ = self.store.audit(
            "provider.rebind-not-started",
            "runtime",
            &json!({
                "providerId": provider_id,
                "generation": generation,
                "reason": reason,
            }),
        );
    }

    /// rebind 沒有成功：狀態留在 `Disconnected`（誠實：沒連上），detail 說得出
    /// 原因，並留稽核。**不**把它翻回 Available——那會讓「可用」變成謊話。
    async fn fail_declarative_rebind(&self, provider_id: &str, generation: u64, reason: &str) {
        if !self.settle_declarative_rebind(provider_id, generation, false) {
            // 世代已經被接手（又停用了／又按了一次啟用／被撤銷）：這一輪的
            // 結果不再是現在的答案，所以不動任何狀態——但也不靜默，否則
            // 「那次重連怎麼了」在稽核上是一段空白。
            let _ = self.store.audit(
                "provider.rebind-superseded",
                "runtime",
                &json!({
                    "providerId": provider_id,
                    "generation": generation,
                    "reason": reason,
                    "note": "another decision took over before this rebind finished; its result \
                             was discarded and no state was changed",
                }),
            );
            return;
        }
        let id = ProviderId::new(provider_id);
        self.note_provider_rebind_failed(&id, reason).await;
        let _ = self.store.audit(
            "provider.rebind-failed",
            "runtime",
            &json!({
                "providerId": provider_id,
                "generation": generation,
                "reason": reason,
            }),
        );
    }
}

/// rebind 停下來的兩種原因：真的失敗，或這一輪已經不算數了。
enum RebindStop {
    Failed(String),
    Superseded(String),
}

/// rebind 的八個步驟。每一步都可能因為世代被接手而中止（`Superseded`）。
///
/// 1. 停新請求（不再登記出站；狀態已經是 `Rebinding`）
/// 2. 收斂進行中（有界等待 in-flight 歸零，等不到就誠實記 `drained:false`）
/// 3. 請來源停止並記錄結果（不假設成功）
/// 4. 清 reader／task／subscription／PROVIDER_LINKS 舊項
/// 5. 失效舊 connection generation（晚到的 callback 依世代被拒）
/// 6. 重新驗證設定與授權（撤銷／移除不得復活）
/// 7. 建新連線並握手協商
/// 8. 握手 Ready 之後才把 ProviderState 收斂為 Available
async fn run_declarative_rebind(
    weak: &Weak<RuntimeInner>,
    provider_id: &str,
    generation: u64,
) -> Result<(), RebindStop> {
    let id = ProviderId::new(provider_id);

    // ---- 1) 停新請求 -------------------------------------------------------
    let (previous, spec) = {
        let Some(rt) = upgrade(weak) else {
            return Err(RebindStop::Superseded("the runtime is gone".into()));
        };
        if !rt.declarative_rebind_current(provider_id, generation) {
            return Err(RebindStop::Superseded("another decision took over".into()));
        }
        let previous = rt.declarative_previous_channels(provider_id);
        for channel in &previous {
            // 出站表留著等於之後每一則廣播都往一條要被換掉的線上送。
            rt.unregister_device_outbound(&channel.expected_device_id());
        }
        let Some(spec) = rt.declarative_spec_of(provider_id) else {
            return Err(RebindStop::Failed(
                "this device's spec is no longer tracked; a restart is needed to rebind".into(),
            ));
        };
        let _ = rt.store.audit(
            "provider.rebinding",
            "runtime",
            &json!({
                "providerId": provider_id,
                "generation": generation,
                "previousLinks": previous.iter().map(|c| c.describe()).collect::<Vec<_>>(),
            }),
        );
        (previous, spec)
    };

    // ---- 2) 收斂進行中 -----------------------------------------------------
    let drain_deadline = tokio::time::Instant::now() + REBIND_DRAIN_BUDGET;
    let drained = loop {
        let pending: usize = previous.iter().map(|c| c.in_flight()).sum();
        if pending == 0 {
            break true;
        }
        if tokio::time::Instant::now() >= drain_deadline {
            break false;
        }
        tokio::time::sleep(REBIND_POLL).await;
    };

    // ---- 3) 請來源停止並記錄結果（不得假設成功） ---------------------------
    // ---- 4) 清 reader／task／subscription／PROVIDER_LINKS 舊項 --------------
    // ---- 5) 失效舊 connection generation ------------------------------------
    let (sensor_stop, closed_links, stale) = {
        let Some(rt) = upgrade(weak) else {
            return Err(RebindStop::Superseded("the runtime is gone".into()));
        };
        if !rt.declarative_rebind_current(provider_id, generation) {
            return Err(RebindStop::Superseded("another decision took over".into()));
        }
        let sensor_stop = rt
            .stop_provider_sensing(provider_id, crate::sensors::SENSOR_STOP_REASON_REBIND)
            .await;
        let closed_links = rt.close_declarative_links(&id, "rebinding");
        // 舊通道的握手世代在 `shutdown()` 之後一律失效：晚到的 ack／frame 會
        // 因為世代不符被拒絕（`DeviceLink::same_generation`）。這裡只記錄，
        // 不假設它們「已經停了」。
        let stale: Vec<serde_json::Value> = previous
            .iter()
            .map(|c| {
                json!({
                    "deviceId": c.expected_device_id(),
                    "transport": c.transport_label(),
                    "readiness": format!("{:?}", c.readiness()),
                    "handshakeInvalidated": !c.handshake_ready(),
                    "inFlight": c.in_flight(),
                })
            })
            .collect();
        (sensor_stop, closed_links, stale)
    };
    drop(previous);

    // ---- 6) 重新驗證設定與授權 ---------------------------------------------
    // 6 與 7 共用同一把 provider 序列化鎖：驗證通過與新連線登記之間不得插進
    // 一次停用／撤銷，否則那個決定會關掉「舊的」連線，而我們接著開出來的新
    // 連線沒有人關得掉。握手等待（步驟 7 後半）刻意**不**持鎖——人類的停用
    // 不該被一台不回應的裝置擋 20 秒。
    let built = {
        let Some(rt) = upgrade(weak) else {
            return Err(RebindStop::Superseded("the runtime is gone".into()));
        };
        let _serialized = rt.providers.lock_provider(&id).await;
        if !rt.declarative_rebind_current(provider_id, generation) {
            return Err(RebindStop::Superseded("another decision took over".into()));
        }
        // 撤銷不得復活：store 的決定跨重啟有效，rebind 也一樣要遵守。
        if let Some(reason) = rt.provider_off_reason(&id) {
            return Err(RebindStop::Failed(format!(
                "this device is still marked off ({reason}); a revoked or disabled device is \
                 never rebound by a background task"
            )));
        }
        let desc = rt
            .providers
            .get(&id)
            .await
            .map_err(|e| RebindStop::Failed(e.to_string()))?;
        if provider_stopped(desc.state) {
            return Err(RebindStop::Superseded(format!(
                "the provider went back to {:?} while rebinding",
                desc.state
            )));
        }
        interaction_adapter_declarative::validate_spec(&spec)
            .map_err(|e| RebindStop::Failed(format!("the adapter spec is no longer valid: {e}")))?;
        // 舊能力必須先讓位：registry 拒絕重複註冊同一個 id，而**別的**還在
        // 操作中的 provider 也宣告的能力不歸這裡處置（旗標只有一份）。
        let shared = rt.shared_capability_holders(&desc).await;
        for rid in &desc.receptors {
            if shared.receptors.iter().any(|r| r == rid) {
                continue;
            }
            let _ = rt.registry.unregister_receptor(&ReceptorId::new(rid)).await;
        }
        for aid in &desc.actuators {
            if shared.actuators.iter().any(|a| a == aid) {
                continue;
            }
            let _ = rt.registry.unregister_actuator(&ActuatorId::new(aid)).await;
        }

        // ---- 7) 建新連線（握手在鎖外面等） ---------------------------------
        // 重新註冊會把能力帶回**剛啟動時**的預設：需要 consent 的受器仍然是
        // 關的，但不需要 consent 的一律預設啟用。人類先前手動關掉的那些必須
        // 在同一把鎖裡立刻關回去——否則「停用 → 啟用」等於替使用者按下了一個
        // 他沒有按的開關（既有保證：回到 Available 不自動恢復任何能力）。
        let human_disabled = rt.declarative_human_disabled(provider_id);
        rt.register_declarative_spec(&spec)
            .await
            .map_err(|e| RebindStop::Failed(format!("the device could not be rebuilt: {e}")))?;
        let mut kept_disabled = Vec::new();
        for rid in &human_disabled {
            let id = ReceptorId::new(rid);
            if rt.registry.set_receptor_enabled(&id, false).await.is_ok() {
                kept_disabled.push(rid.clone());
            }
        }
        // 套用完才清：register 失敗時受器已經被 unregister 掉了，紀錄必須留給
        // 下一次嘗試，否則預設值會把它們全部打開。
        rt.clear_declarative_human_disabled(provider_id);
        (rt.declarative_previous_channels(provider_id), kept_disabled)
    };
    let (channels, kept_disabled) = built;
    let handshake = if channels.is_empty() {
        // 沒有裝置線（純 HTTP adapter）：沒有握手可等。誠實：這不是「握手成功」。
        "none"
    } else {
        let deadline = tokio::time::Instant::now() + REBIND_HANDSHAKE_BUDGET;
        loop {
            if channels.iter().all(|c| c.handshake_ready()) {
                break "ready";
            }
            if channels
                .iter()
                .any(|c| matches!(c.readiness(), LinkReadiness::Closed))
            {
                return Err(RebindStop::Failed(
                    "a device link was closed while the rebind was waiting for its handshake"
                        .into(),
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(RebindStop::Failed(format!(
                    "the device did not complete its hello/pairing handshake within {} s; it is \
                     NOT connected",
                    REBIND_HANDSHAKE_BUDGET.as_secs()
                )));
            }
            tokio::time::sleep(REBIND_POLL).await;
        }
    };

    // ---- 8) 收斂 ProviderState ---------------------------------------------
    let Some(rt) = upgrade(weak) else {
        return Err(RebindStop::Superseded("the runtime is gone".into()));
    };
    if !rt.settle_declarative_rebind(provider_id, generation, true) {
        return Err(RebindStop::Superseded(
            "another decision took over before the rebind could be committed".into(),
        ));
    }
    rt.converge_provider_after_rebind(&id).await;
    let _ = rt.store.audit(
        "provider.rebound",
        "runtime",
        &json!({
            "providerId": provider_id,
            "generation": generation,
            "drained": drained,
            "closedLinks": closed_links,
            "staleChannels": stale,
            "sensorStop": sensor_stop,
            "handshake": handshake,
            // 人類先前手動關掉、這一輪刻意保持關閉的受器。空陣列＝沒有這種
            // 受器，不是「不知道」。
            "keptDisabledReceptors": kept_disabled,
        }),
    );
    Ok(())
}

/// 這一族目前記得的所有綁定（診斷用；順序固定）。
impl Runtime {
    pub fn declarative_bindings_report(&self) -> Vec<serde_json::Value> {
        let snapshot: BTreeMap<String, String> = self
            .declarative_bindings
            .lock()
            .map(|map| {
                map.iter()
                    .map(|(id, entry)| (id.clone(), entry.lifecycle.label()))
                    .collect()
            })
            .unwrap_or_default();
        snapshot
            .into_iter()
            .map(|(id, lifecycle)| json!({"providerId": id, "lifecycle": lifecycle}))
            .collect()
    }
}

/// `ProviderState::Disabled`／`Closed`／`Expired`／`Revoked` → 綁定被拆掉的原因。
pub(crate) fn unbound_reason_for_state(state: ProviderState) -> UnboundReason {
    match state {
        ProviderState::Revoked => UnboundReason::Revoked,
        _ => UnboundReason::Disabled,
    }
}
