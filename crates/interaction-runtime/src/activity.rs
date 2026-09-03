//! Unified human activity/inbox projection. Every surface consumes this
//! application-service result instead of inventing page-local truth.

use crate::runtime::Runtime;
use chrono::{DateTime, Utc};
use interaction_core::{DomainResult, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    /// 徽章的 `pendingCount` 一律在分頁截斷前算完，通知中心用這個篩選就能
    /// 拿到全部待決定項，而不是從「最近 N 筆」裡碰運氣。
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

        if let Some(nodes) = self.knowledge_list(Some("candidate"), 500).await?["nodes"].as_array()
        {
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
        // 角色演出，不是需要人類裁決的外部副作用：結果未知仍留在歷史裡
        // （誠實），但不佔「待我決定」。
        //
        // 只認 driver，不認通道也不認 id 前綴：`iphone.character` 也走
        // `desktop-pet` 通道，但它是「送到另一台實體裝置」的外部副作用——
        // 結果未知一定要人看見。「被安全規則阻止」不論通道都仍要人看見。
        let presentation_actuators: std::collections::HashSet<String> = self
            .registry
            .actuator_manifests()
            .await
            .into_iter()
            .filter(|manifest| manifest.driver == "builtin.presentation")
            .map(|manifest| manifest.id.as_str().to_string())
            .collect();

        for receipt in self.list_actions(None, 200)? {
            let status = serde_json::to_value(receipt.current_status)
                .ok()
                .and_then(|value| value.as_str().map(String::from))
                .unwrap_or_else(|| "unknown".into());
            let occurred_at = receipt
                .timestamps
                .last()
                .map(|(_, at)| *at)
                .unwrap_or_else(Utc::now);
            let needs_decision = match status.as_str() {
                "uncertain" => !presentation_actuators.contains(receipt.actuator_id.as_str()),
                "blocked" => true,
                _ => false,
            };
            items.push(ActivityInboxItem {
                item_id: receipt.action_id.as_str().to_string(),
                kind: "action-result".into(),
                status: status.clone(),
                title: receipt.intent.clone(),
                occurred_at,
                route: "activity".into(),
                needs_decision,
                agent_id: None,
                device_id: Some(receipt.actuator_id.as_str().to_string()),
                task_id: Some(receipt.plan_id.as_str().to_string()),
                domains: vec![],
                detail: serde_json::to_value(receipt).unwrap_or_default(),
            });
        }

        for event in self.events.recent(200) {
            if !matches!(
                event.event_type,
                interaction_core::EventType::SensorStarted
                    | interaction_core::EventType::SensorStopped
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
                _ => (event_type.clone(), format!("感測停止：{sensor_label}")),
            };
            items.push(ActivityInboxItem {
                item_id: event.event_id.as_str().to_string(),
                kind: "safety-event".into(),
                status,
                title,
                occurred_at: event.timestamp,
                route: "safety".into(),
                needs_decision: false,
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
        items.truncate(filter.limit.unwrap_or(100).clamp(1, 500) as usize);
        Ok(json!({
            "items": items,
            "count": items.len(),
            "totalBeforeLimit": total,
            "pendingCount": pending,
            "filters": filter,
            "generatedAt": Utc::now(),
        }))
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
