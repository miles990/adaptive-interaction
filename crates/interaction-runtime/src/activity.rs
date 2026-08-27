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
            items.push(ActivityInboxItem {
                item_id: receipt.action_id.as_str().to_string(),
                kind: "action-result".into(),
                status: status.clone(),
                title: receipt.intent.clone(),
                occurred_at,
                route: "activity".into(),
                needs_decision: matches!(status.as_str(), "uncertain" | "blocked"),
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
            items.push(ActivityInboxItem {
                item_id: event.event_id.as_str().to_string(),
                kind: "safety-event".into(),
                status: if event_type == "emergency.stop" {
                    "emergency".into()
                } else {
                    event_type.clone()
                },
                title: event_type,
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
        });
        items.sort_by_key(|item| std::cmp::Reverse(item.occurred_at));
        let total = items.len();
        items.truncate(filter.limit.unwrap_or(100).clamp(1, 500) as usize);
        let pending = items.iter().filter(|item| item.needs_decision).count();
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
