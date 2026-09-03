//! Unified human activity/inbox projection. Every surface consumes this
//! application-service result instead of inventing page-local truth.

use crate::runtime::Runtime;
use chrono::{DateTime, Utc};
use interaction_core::{ActionReceipt, DomainResult, PolicyDecision, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

/// 歷史區塊的時間視窗：最近 N 筆收據。純歷史用，不決定徽章數字。
const HISTORY_RECEIPT_LIMIT: u32 = 200;

/// 安全事件歷史區塊的視窗：最近 N 筆事件。純歷史用。
/// `sensor.stop-uncertain` 是例外（要人處理），不受這個視窗限制。
const HISTORY_EVENT_LIMIT: usize = 200;

/// 「待你決定」的收據狀態：結果未知（誠實階梯：不得當成成功或失敗）與
/// 被安全規則阻止，兩者都是黏著終態且沒有 ack／dismiss 介面。
const PENDING_RECEIPT_STATUSES: &[&str] = &["uncertain", "blocked"];

/// 待決定收據的掃描上限。只看「最近 200 筆」的話，較舊的未決項會被較新的
/// 收據擠出視窗而從徽章上靜靜消失，介面還會宣稱「目前沒有待決定事項」——
/// 所以另外用狀態查詢把視窗外的待決項撈回來。撈滿這個上限時
/// `pendingCountExact` 誠實回 `false`，介面必須改口「至少 N 項，還有未載入的」。
const PENDING_RECEIPT_SCAN_LIMIT: u32 = 1000;

/// 知識複審候選的掃描上限（同樣算待你決定，超過上限一樣標記為不精確）。
const CANDIDATE_SCAN_LIMIT: u32 = 1000;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ActivityInboxFilter {
    pub status: Option<String>,
    pub agent: Option<String>,
    pub device: Option<String>,
    pub task: Option<String>,
    pub domain: Option<String>,
    pub since: Option<DateTime<Utc>>,
    /// `true`：只要待你決定的項目；`false`：只要不需決定的；缺席：全部。
    /// 徽章的 `pendingCount` 一律在分頁截斷前算完，且待決定項是直接依狀態
    /// 查出來的（不受歷史區塊「最近 N 筆」的時間視窗影響）；只有在待決定項
    /// 多到撞上掃描上限時 `pendingCountExact` 才會是 `false`。
    pub needs_decision: Option<bool>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityInboxItem {
    pub item_id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub occurred_at: Timestamp,
    pub route: String,
    pub needs_decision: bool,
    pub agent_id: Option<String>,
    pub device_id: Option<String>,
    pub task_id: Option<String>,
    pub domains: Vec<String>,
    pub detail: Value,
}

impl Runtime {
    /// `sensor.stop-uncertain` 的裝置人話名稱。優先用已配對手機的名稱
    /// （payload 只有 deviceId——原始 id 不進一般模式的標題），
    /// 手機已被撤銷／查不到時退回感測來源的人話名稱，再不行才說「裝置」。
    async fn stop_uncertain_device_label(&self, payload: &Value) -> String {
        if let Some(device_id) = payload.get("deviceId").and_then(Value::as_str) {
            if let Some(name) = self.mobile.device_name(device_id).await {
                return name;
            }
        }
        match payload.get("sensor").and_then(Value::as_str) {
            Some(sensor) if sensor.starts_with("iphone.") => "iPhone".into(),
            Some(sensor) => sensor_display_name(sensor),
            None => "裝置".into(),
        }
    }

    pub async fn activity_inbox(&self, filter: ActivityInboxFilter) -> DomainResult<Value> {
        let mut items = Vec::new();

        for assist in self.pending_ai_assists().await {
            items.push(ActivityInboxItem {
                item_id: assist.request_id.clone(),
                kind: "ai-assist".into(),
                status: "waiting-for-input".into(),
                title: assist.reason.clone(),
                occurred_at: assist.created_at,
                route: "automations".into(),
                needs_decision: true,
                agent_id: None,
                device_id: None,
                task_id: Some(assist.recipe_id.clone()),
                domains: assist.data_scope.clone(),
                detail: serde_json::to_value(assist).unwrap_or_default(),
            });
        }

        for session in self.list_agent_sessions().await {
            let status = serde_json::to_value(session.state)
                .ok()
                .and_then(|value| value.as_str().map(String::from))
                .unwrap_or_else(|| "unknown".into());
            let needs_decision = matches!(
                status.as_str(),
                "waiting-for-consent" | "waiting-for-input" | "claimed-completed"
            );
            items.push(ActivityInboxItem {
                item_id: session.session_id.as_str().to_string(),
                kind: "agent-session".into(),
                status,
                title: session
                    .label
                    .clone()
                    .unwrap_or_else(|| session.agent_id.clone()),
                occurred_at: session.created_at,
                route: "ai".into(),
                needs_decision,
                agent_id: Some(session.agent_id.clone()),
                device_id: None,
                task_id: session
                    .delegation
                    .as_ref()
                    .map(|delegation| delegation.root_task_id.clone()),
                domains: session
                    .data_scope
                    .iter()
                    .filter_map(|scope| scope.strip_prefix("domain:").map(String::from))
                    .collect(),
                detail: serde_json::to_value(session).unwrap_or_default(),
            });
        }

        let candidates = self
            .knowledge_list(Some("candidate"), CANDIDATE_SCAN_LIMIT)
            .await?;
        // 撈滿上限＝可能還有沒撈到的候選，數字不得宣稱精確。
        let candidates_exact = candidates["nodes"]
            .as_array()
            .is_none_or(|nodes| nodes.len() < CANDIDATE_SCAN_LIMIT as usize);
        if let Some(nodes) = candidates["nodes"].as_array() {
            for node in nodes {
                let id = node["nodeId"].as_str().unwrap_or("unknown").to_string();
                let occurred_at = node["createdAt"]
                    .as_str()
                    .and_then(|value| value.parse::<DateTime<Utc>>().ok())
                    .unwrap_or_else(Utc::now);
                items.push(ActivityInboxItem {
                    item_id: id.clone(),
                    kind: "knowledge-review".into(),
                    status: "candidate".into(),
                    title: node["title"]
                        .as_str()
                        .unwrap_or("未命名知識候選")
                        .to_string(),
                    occurred_at,
                    route: "memory".into(),
                    needs_decision: true,
                    agent_id: (node["createdBy"]["kind"].as_str() == Some("agent"))
                        .then(|| node["createdBy"]["id"].as_str().map(String::from))
                        .flatten(),
                    device_id: None,
                    task_id: None,
                    domains: node["domains"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(String::from)
                        .collect(),
                    detail: node.clone(),
                });
            }
        }

        // L0 純呈現（桌面角色視窗，driver `builtin.presentation`）的動作是
        // 角色演出，不是需要人類裁決的外部副作用：結果未知或被安靜時段擋下
        // 都仍留在歷史裡（誠實），但不佔「待我決定」——使用者對它沒有任何
        // 可做的決定。
        //
        // 只認 driver，不認通道也不認 id 前綴：`iphone.character` 也走
        // `desktop-pet` 通道，但它是「送到另一台實體裝置」的外部副作用——
        // 結果未知或被擋下一定要人看見。
        let presentation_actuators: HashSet<String> = self
            .registry
            .actuator_manifests()
            .await
            .into_iter()
            .filter(|manifest| manifest.driver == "builtin.presentation")
            .map(|manifest| manifest.id.as_str().to_string())
            .collect();

        // 歷史區塊：最近 N 筆收據（含不需決定的）。
        let mut seen_receipts: HashSet<String> = HashSet::new();
        for receipt in self.list_actions(None, HISTORY_RECEIPT_LIMIT)? {
            seen_receipts.insert(receipt.action_id.as_str().to_string());
            items.push(receipt_item(receipt, &presentation_actuators));
        }
        // 待決定區塊：直接依狀態查，把落在歷史視窗外的未決項撈回來。
        // 少了這一段，一筆較舊的「結果未知」實體動作只要被 200 筆較新的
        // 收據擠出視窗，就會從徽章與「待我決定」裡靜靜消失。
        let open_pending = self
            .store
            .receipts_with_status(PENDING_RECEIPT_STATUSES, PENDING_RECEIPT_SCAN_LIMIT)?;
        let pending_receipts_exact = open_pending.len() < PENDING_RECEIPT_SCAN_LIMIT as usize;
        for receipt in open_pending {
            if !seen_receipts.insert(receipt.action_id.as_str().to_string()) {
                continue;
            }
            let item = receipt_item(receipt, &presentation_actuators);
            // 視窗外只補「真的要人決定」的項目；其餘（例如安靜時段擋下的
            // 角色演出）留在歷史區塊，不灌爆收件匣。
            if item.needs_decision {
                items.push(item);
            }
        }

        // 安全事件多半只是歷史通知（`needs_decision: false`），所以這個
        // 最近 N 筆的視窗不會影響徽章數字。
        // 例外：`sensor.stop-uncertain`（要求手機停止感測但沒等到確認）是
        // 「要人處理」的項目——它不得因為被較新的事件擠出歷史視窗就從徽章上
        // 靜靜消失，所以對它掃整個事件環（`recent` 只會回它手上還有的）。
        let events = self.events.recent(usize::MAX);
        let history_start = events.len().saturating_sub(HISTORY_EVENT_LIMIT);
        for (index, event) in events.into_iter().enumerate() {
            let stop_uncertain =
                event.event_type == interaction_core::EventType::SensorStopUncertain;
            if !stop_uncertain && index < history_start {
                continue;
            }
            if !matches!(
                event.event_type,
                interaction_core::EventType::SensorStarted
                    | interaction_core::EventType::SensorStopped
                    | interaction_core::EventType::SensorStopUncertain
                    | interaction_core::EventType::EmergencyStop
            ) {
                continue;
            }
            let event_type = serde_json::to_value(event.event_type)
                .ok()
                .and_then(|value| value.as_str().map(String::from))
                .unwrap_or_else(|| "event".into());
            let device_id = event
                .payload
                .get("deviceId")
                .or_else(|| event.payload.get("sensor"))
                .and_then(Value::as_str)
                .map(String::from);
            // 解除緊急停止也走 EmergencyStop 事件（payload.cleared=true）：
            // 不分辨的話，使用者剛解除就會看到一筆新的「緊急停止」。
            let cleared = event.event_type == interaction_core::EventType::EmergencyStop
                && event.payload.get("cleared").and_then(Value::as_bool) == Some(true);
            let sensor_label = device_id
                .as_deref()
                .map(sensor_display_name)
                .unwrap_or_else(|| "感測器".into());
            // 標題是人話，不是原始 event_type（原始碼仍在 detail.eventType）。
            let (status, title): (String, String) = match event.event_type {
                interaction_core::EventType::EmergencyStop if cleared => {
                    ("emergency-cleared".into(), "緊急停止已解除".into())
                }
                interaction_core::EventType::EmergencyStop => {
                    ("emergency".into(), "緊急停止已啟動".into())
                }
                interaction_core::EventType::SensorStarted => {
                    (event_type.clone(), format!("感測開始：{sensor_label}"))
                }
                interaction_core::EventType::SensorStopUncertain => {
                    // 誠實：要求停止 ≠ 已停止。手機沒回覆時它可能還在擷取，
                    // 所以標題說「結果不確定」，並點名是哪一台裝置。
                    let device = self.stop_uncertain_device_label(&event.payload).await;
                    (event_type.clone(), format!("感測停止結果不確定：{device}"))
                }
                _ => (event_type.clone(), format!("感測停止：{sensor_label}")),
            };
            items.push(ActivityInboxItem {
                item_id: event.event_id.as_str().to_string(),
                kind: "safety-event".into(),
                status,
                title,
                occurred_at: event.timestamp,
                route: "safety".into(),
                // 「停止結果不確定」是使用者要處理的事（去手機上確認／再停一次），
                // 其餘安全事件只是歷史通知。
                needs_decision: stop_uncertain,
                agent_id: None,
                device_id,
                task_id: event
                    .correlation_id
                    .as_ref()
                    .map(|id| id.as_str().to_string()),
                domains: vec![],
                detail: serde_json::to_value(event).unwrap_or_default(),
            });
        }

        let normalized = |value: &str| value.trim().to_lowercase();
        items.retain(|item| {
            filter
                .status
                .as_deref()
                .is_none_or(|value| normalized(&item.status).contains(&normalized(value)))
                && filter.agent.as_deref().is_none_or(|value| {
                    item.agent_id
                        .as_deref()
                        .is_some_and(|agent| normalized(agent).contains(&normalized(value)))
                })
                && filter.device.as_deref().is_none_or(|value| {
                    item.device_id
                        .as_deref()
                        .is_some_and(|device| normalized(device).contains(&normalized(value)))
                })
                && filter.task.as_deref().is_none_or(|value| {
                    normalized(&item.title).contains(&normalized(value))
                        || item
                            .task_id
                            .as_deref()
                            .is_some_and(|task| normalized(task).contains(&normalized(value)))
                })
                && filter.domain.as_deref().is_none_or(|value| {
                    item.domains
                        .iter()
                        .any(|domain| normalized(domain).contains(&normalized(value)))
                })
                && filter.since.is_none_or(|since| item.occurred_at >= since)
                && filter
                    .needs_decision
                    .is_none_or(|wanted| item.needs_decision == wanted)
        });
        items.sort_by_key(|item| std::cmp::Reverse(item.occurred_at));
        let total = items.len();
        // 待決定數量必須在分頁截斷「之前」算完：徽章代表全部待辦，
        // 不是本頁剛好裝得下的那幾筆（截斷後計算會漏報）。
        let pending = items.iter().filter(|item| item.needs_decision).count();
        // 待決定項是依狀態查出來的，所以只有撞到掃描上限時才可能不完整；
        // 這時 `pendingCount` 是「至少」而不是「總共」，介面必須據此改口，
        // 絕不能在旗標為 false 時宣稱「目前沒有待決定事項」。
        let pending_count_exact = pending_receipts_exact && candidates_exact;
        items.truncate(filter.limit.unwrap_or(100).clamp(1, 500) as usize);
        Ok(json!({
            "items": items,
            "count": items.len(),
            "totalBeforeLimit": total,
            "pendingCount": pending,
            "pendingCountExact": pending_count_exact,
            "filters": filter,
            "generatedAt": Utc::now(),
        }))
    }
}

/// 收據 → 收件匣項目。`needs_decision` 的判準：
/// * 結果未知（`uncertain`）與被安全規則阻止（`blocked`）是人要處理的項目；
/// * 但 L0 純呈現（driver `builtin.presentation`）只是本機角色演出，被安靜
///   時段擋下或沒被確認時使用者沒有任何可做的決定 → 只留歷史，不上徽章；
/// * 例外的例外：呈現動作若卡在「需要人類核可」，那確實有一個決定要做
///   （核可只能由人給），仍要人看見。
fn receipt_item(
    receipt: ActionReceipt,
    presentation_actuators: &HashSet<String>,
) -> ActivityInboxItem {
    let status = serde_json::to_value(receipt.current_status)
        .ok()
        .and_then(|value| value.as_str().map(String::from))
        .unwrap_or_else(|| "unknown".into());
    let occurred_at = receipt
        .timestamps
        .last()
        .map(|(_, at)| *at)
        .unwrap_or_else(Utc::now);
    let awaits_human_approval = receipt
        .policy_decisions
        .iter()
        .any(|decision| matches!(decision, PolicyDecision::ApprovalRequired { .. }));
    let presentation_only = presentation_actuators.contains(receipt.actuator_id.as_str());
    let needs_decision = matches!(status.as_str(), "uncertain" | "blocked")
        && (!presentation_only || awaits_human_approval);
    ActivityInboxItem {
        item_id: receipt.action_id.as_str().to_string(),
        kind: "action-result".into(),
        status,
        title: receipt.intent.clone(),
        occurred_at,
        route: "activity".into(),
        needs_decision,
        agent_id: None,
        device_id: Some(receipt.actuator_id.as_str().to_string()),
        task_id: Some(receipt.plan_id.as_str().to_string()),
        domains: vec![],
        detail: serde_json::to_value(receipt).unwrap_or_default(),
    }
}

/// 感測器／裝置 id → 人話（認不得的照原樣顯示，不猜）。
fn sensor_display_name(raw: &str) -> String {
    match raw {
        "microphone" | "mic" | "builtin.microphone" => "麥克風".into(),
        "camera" | "builtin.camera" => "攝影機".into(),
        other => other.to_string(),
    }
}
