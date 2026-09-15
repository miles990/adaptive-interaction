//! 「這件工作的經過」——把一個 agent session 的追蹤紀錄投影成人話。
//!
//! 為什麼這一層在 Rust 而不在前端：核心邏輯不進前端 JS（CLAUDE.md 不變量）。
//! CLI、HTTP API 與桌面共用同一份投影，所以「一般模式看得到什麼、看不到
//! 什麼」只有一個地方可以改，也只有一個地方要測。
//!
//! 誠實階梯在這一層的表現：
//! - `kind` 認不得就是「其他紀錄」，**絕不**把 kind 字串原樣當標籤——
//!   使用者看到的每一個字都是這裡寫死的人話。
//! - `claimed` 投影成「對方說已完成」，永遠不是「完成」；`verified` 只由
//!   人工驗證產生。
//! - `failure_reason` 只放安全摘要（`detail.reason`／`record.detail`），
//!   不放 stderr 全文——那是 diagnostic 類，只在技術詳情裡出現。
//! - 一個欄位沒有事實可填就是 `None`，不補造。

use crate::runtime::Runtime;
use interaction_core::{
    AgentSessionRecord, AgentSessionState, DomainError, DomainResult, Timestamp, TraceClass,
    TraceOutcome, TraceQuery, TraceRow,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 時間線上的一步。`label` 是人話，`kind`／`code` 只給進階模式與程式判斷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActivityStep {
    pub at: Timestamp,
    pub id: i64,
    pub kind: String,
    /// 人話標籤（固定文案；認不得的 kind 一律「其他紀錄」）。
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TraceOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// 這一筆有沒有可展開的技術細節（有才值得在進階模式畫一個展開鈕）。
    pub detail_available: bool,
}

/// 一個 agent session 的「經過」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionActivity {
    pub session_id: String,
    /// 發生了什麼（人話，一句話）。
    pub headline: String,
    /// 目前狀態（人話；與桌面 `workState.ts` 的文案表同一組字）。
    pub state_label: String,
    /// 執行階段（taxonomy 字串）；舊快照沒有這個欄位就是 `None`，不補造。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// `AgentSessionRecord.state` 的原始 taxonomy 值（技術層）。
    pub record_state: String,
    /// `open`｜`closed`：這個工作還算不算進行中。
    pub lifecycle: String,
    /// 失敗／不確定的安全摘要；沒有就是 `None`（不猜）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// 下一步該做什麼（人話）；進行中且不需要人類裁決時是 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
    pub timeline: Vec<ActivityStep>,
    /// 技術層的原始紀錄（進階模式才展開）。diagnostic 類只留脫敏後的 tail。
    pub records: Vec<TraceRow>,
    /// 還有更早的紀錄沒回（配合 `next_cursor` 往回翻）。
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
}

/// 查詢字串／IPC 參數的原始形狀（全部是字串與數字，還沒驗過）。
///
/// 為什麼放在 runtime 而不是各自的介面層：HTTP、Tauri IPC 與 CLI 必須是
/// **同一條**解析規則。三個地方各寫一次 `TraceClass::parse`，遲早會有一個
/// 悄悄把不認得的值當成「沒有篩選」——那會讓查詢者以為「沒有這種紀錄」。
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceQueryInput {
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub kind: Option<String>,
    pub class: Option<String>,
    pub actor: Option<String>,
    pub outcome: Option<String>,
    /// 含下界（RFC3339）。
    pub since: Option<String>,
    /// 不含上界（RFC3339）。
    pub until: Option<String>,
    /// 往回翻頁的 cursor：只回 `id < before`。
    pub before: Option<i64>,
    pub limit: Option<u32>,
}

impl TraceQueryInput {
    /// 驗證並轉成儲存層的 [`TraceQuery`]。
    ///
    /// 認不得的 class／outcome／時間字串一律回 `Validation`，**不**悄悄忽略：
    /// 忽略一個篩選條件比報錯危險得多——查詢者會以為「沒有這種紀錄」。
    pub fn into_query(self) -> DomainResult<TraceQuery> {
        fn reject<T>(what: &str, value: &str) -> DomainResult<T> {
            Err(DomainError::Validation(format!("unknown {what}: {value}")))
        }
        let class = match &self.class {
            None => None,
            Some(raw) => match TraceClass::parse(raw) {
                Some(class) => Some(class),
                None => return reject("class", raw),
            },
        };
        let outcome = match &self.outcome {
            None => None,
            Some(raw) => match TraceOutcome::parse(raw) {
                Some(outcome) => Some(outcome),
                None => return reject("outcome", raw),
            },
        };
        let time = |raw: &Option<String>, what: &str| -> DomainResult<Option<Timestamp>> {
            match raw {
                None => Ok(None),
                Some(value) => match chrono::DateTime::parse_from_rfc3339(value) {
                    Ok(parsed) => Ok(Some(parsed.with_timezone(&chrono::Utc))),
                    Err(_) => reject(what, value),
                },
            }
        };
        Ok(TraceQuery {
            trace_id: self.trace_id,
            session_id: self.session_id,
            kind: self.kind,
            class,
            actor: self.actor,
            outcome,
            since: time(&self.since, "since")?,
            until: time(&self.until, "until")?,
            before_id: self.before,
            // 上限由儲存層 clamp（`effective_limit`）：只有一個地方說了算。
            limit: self
                .limit
                .unwrap_or(interaction_core::TRACE_QUERY_DEFAULT_LIMIT),
        })
    }
}

/// 一頁追蹤紀錄。`next_cursor` 有值＝還可能有更早的紀錄。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TracePage {
    pub items: Vec<TraceRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
    /// 這一頁實際套用的上限（clamp 之後）。
    pub limit: u32,
}

/// 工作狀態 → 人話。**與桌面 `statusProjection/workState.ts` 的
/// `WORK_STATE_PROJECTION` 是同一組文案**（那邊是唯一的文案來源；這裡沿用，
/// 不自創新詞）。認不得的原始值照實說「結果不確定」，絕不回傳原始字串。
pub fn work_state_label(raw: &str) -> &'static str {
    match raw {
        "created" | "queued" => "正在準備",
        "fetched" => "已交給工作助手",
        "active" | "working" => "處理中",
        "waiting-for-input" | "waiting-input" => "等你回答",
        "waiting-for-consent" | "waiting-consent" => "等你允許",
        "blocked" => "無法繼續",
        "claimed-completed" => "對方說已完成",
        "verified" => "已由你確認",
        "failed" => "失敗",
        "timed-out" => "逾時失敗",
        "expired" => "已到期",
        "cancelled" | "closed" => "已取消",
        // `unknown` 與所有介面不認得的值共用同一條退路：不猜。
        _ => "結果不確定",
    }
}

/// 「派送給誰」的人話標籤。Codex／Claude Code 是使用者自己裝的產品名字，
/// 說得出來才有意義；認不得的 kind 一律說「工作助手」，不把 id 當標籤。
fn dispatched_label(kind: Option<&str>) -> &'static str {
    match kind {
        Some("codex") => "已交給 Codex",
        Some("claude-code") => "已交給 Claude Code",
        _ => "已交給工作助手",
    }
}

/// 一筆紀錄 → 人話標籤。
///
/// 這張表是「一般模式看得到的字」的唯一來源。新的 kind 沒有進表就是
/// 「其他紀錄」——寧可少說一句，也不要把 `agent-session.provider-model`
/// 這種字串丟到使用者臉上。
fn step_label(row: &TraceRow) -> String {
    let code = row.code.as_deref();
    let reason = detail_str(&row.detail, "reason");
    match row.kind.as_str() {
        "agent-session.resume-checked" => match row.outcome {
            Some(TraceOutcome::Accepted) => "接續上次的工作：已接受".to_string(),
            Some(TraceOutcome::Rejected) => match reason {
                Some(why) => format!("接續上次的工作：已拒絕（{why}）"),
                None => "接續上次的工作：已拒絕".to_string(),
            },
            Some(TraceOutcome::Ignored) => "接續上次的工作：沒有東西可以接續".to_string(),
            _ => "接續上次的工作".to_string(),
        },
        "agent-session.dispatched" => {
            dispatched_label(detail_str(&row.detail, "providerKind").as_deref()).to_string()
        }
        "agent-session.provider-model" => "工作助手回報了它實際使用的模型".to_string(),
        "agent-session.task-delivered" => "任務已送達".to_string(),
        "agent-session.outcome" => match (row.outcome, code) {
            (Some(TraceOutcome::Claimed), _) => "對方說已完成（尚未檢查）".to_string(),
            (Some(TraceOutcome::Failed), _) => match reason {
                Some(why) => format!("失敗：{why}"),
                None => "失敗".to_string(),
            },
            (Some(TraceOutcome::Cancelled), _) => "已取消".to_string(),
            (Some(TraceOutcome::Expired), Some("lease.expired")) => "授權時間到期".to_string(),
            (Some(TraceOutcome::Expired), _) => "逾時，沒有等到結果".to_string(),
            (Some(TraceOutcome::Unknown), Some("runtime.restarted")) => {
                "程式重新啟動過，這一輪的結果不確定".to_string()
            }
            _ => "結果不確定".to_string(),
        },
        "agent-session.interrupt-requested" => match row.outcome {
            Some(TraceOutcome::Accepted) => "你要求中斷（等待確認）".to_string(),
            _ => "你要求中斷，但指令沒有送到".to_string(),
        },
        "agent-session.subprocess-stderr" => "工作助手有診斷輸出（可展開）".to_string(),
        "agent.approval" => match (row.outcome, code) {
            (_, Some("approval.watchdog-denied")) => "逾時沒有回應，已自動拒絕一項請求".to_string(),
            (Some(TraceOutcome::Accepted), _) => "你核准了一項請求".to_string(),
            _ => "你拒絕了一項請求".to_string(),
        },
        "agent-session.verified" => "已由你確認".to_string(),
        "agent-session.closed" => "已關閉".to_string(),
        "agent-session.emergency-stop" => "緊急停止".to_string(),
        "agent-session.capability-issued" => "核發了這次工作的通行證".to_string(),
        "consent.consumed" => "用掉了一次「只這一次」的允許".to_string(),
        "consent.rejected" => "有一項允許被拒絕".to_string(),
        "memory.updated" => "記憶被修改".to_string(),
        "trace.pruned" => "舊紀錄已依保存期限清理".to_string(),
        _ => "其他紀錄".to_string(),
    }
}

fn detail_str(detail: &Value, key: &str) -> Option<String> {
    detail
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 這一筆紀錄有沒有值得展開的東西（null／空物件都不算）。
fn detail_available(detail: &Value) -> bool {
    match detail {
        Value::Null => false,
        Value::Object(map) => !map.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// diagnostic 類（子程序 stderr）在 `records` 裡只留脫敏後的 tail 與兩個
/// 界限標記。原始 detail 的其餘欄位（位元組數、行數）對使用者沒有意義，
/// 而 tail 以外的任何東西都只會擴大外洩面。
fn narrow_diagnostic(mut row: TraceRow) -> TraceRow {
    if row.class == TraceClass::Diagnostic {
        row.detail = json!({
            "tail": row.detail.get("tail").cloned().unwrap_or(Value::Null),
            // 誠實：被截斷／被丟掉多少行也要說得出來。
            "truncated": row.detail.get("truncated").cloned().unwrap_or(Value::Null),
            "linesDropped": row.detail.get("linesDropped").cloned().unwrap_or(Value::Null),
        });
    }
    row
}

/// 「下一步該做什麼」。進行中而且不需要人類做決定時就是 `None`——
/// 沒有事情要做的時候不要硬擠一句話出來。
fn next_step_for(record: &AgentSessionRecord, verified: bool) -> Option<&'static str> {
    if verified {
        return Some("無需處理");
    }
    match record.state {
        AgentSessionState::ClaimedCompleted => Some("請檢查結果並確認"),
        AgentSessionState::WaitingForConsent => Some("請核准或拒絕"),
        AgentSessionState::WaitingForInput => Some("請回答"),
        AgentSessionState::Failed | AgentSessionState::Unknown | AgentSessionState::TimedOut => {
            Some("可重新交代一件工作；需要細節可展開技術詳情")
        }
        AgentSessionState::Cancelled | AgentSessionState::Closed | AgentSessionState::Expired => {
            Some("無需處理")
        }
        AgentSessionState::Created | AgentSessionState::Active => None,
    }
}

/// 人類是否**針對目前這一輪 claim** 確認過。與桌面 `verifiedForCurrentClaim`
/// 同一條規則：`claim_id` 對不上就不算（上一輪的綠勾不沿用到這一輪）。
fn verified_for_current_claim(record: &AgentSessionRecord) -> bool {
    match &record.human_verified {
        Some(verification) => verification.claim_id == record.claim_id,
        None => false,
    }
}

impl Runtime {
    /// 一頁追蹤紀錄。三個介面（HTTP／Tauri IPC／CLI 經 HTTP）共用同一份
    /// clamp 與分頁規則：回滿一頁才給 cursor，沒回滿就是到底了。
    pub fn query_trace_page(&self, input: TraceQueryInput) -> DomainResult<TracePage> {
        let query = input.into_query()?;
        let limit = query.effective_limit();
        let items = self.store.query_trace(&query)?;
        let next_cursor = (items.len() as u32 == limit)
            .then(|| items.last().map(|row| row.id))
            .flatten();
        Ok(TracePage {
            items,
            next_cursor,
            limit,
        })
    }

    /// 一個 agent session 的「經過」：人話摘要＋時間線＋原始紀錄。
    ///
    /// `before` 是往回翻頁的 cursor（只回 `id < before`）；`limit` 由儲存層
    /// clamp 到 `1..=500`。
    ///
    /// 摘要（headline／state_label／failure_reason／next_step）**不**受分頁
    /// 影響：它們各走一次有界的專用查詢，所以往回翻頁不會讓「這件工作現在
    /// 怎麼了」跟著變。
    pub async fn agent_session_activity(
        &self,
        id: &str,
        before: Option<i64>,
        limit: u32,
    ) -> DomainResult<AgentSessionActivity> {
        // 先確認這個 session 真的存在：不存在就是 NotFound，不回一份空殼
        // 假裝有這件工作。（被拒絕的續開嘗試沒有 session 紀錄，它的稽核
        // 要用 `/v1/trace?sessionId=…` 查——那條路徑不需要 session 存在。）
        let record = self.get_agent_session(id).await?;

        let query = TraceQuery {
            session_id: Some(id.to_string()),
            before_id: before,
            limit,
            ..Default::default()
        };
        let effective_limit = query.effective_limit();
        let rows = self.store.query_trace(&query)?;
        let truncated = rows.len() as u32 == effective_limit;
        let next_cursor = truncated.then(|| rows.last().map(|row| row.id)).flatten();

        let timeline: Vec<ActivityStep> = rows
            .iter()
            .map(|row| ActivityStep {
                at: row.at,
                id: row.id,
                kind: row.kind.clone(),
                label: step_label(row),
                outcome: row.outcome,
                code: row.code.clone(),
                detail_available: detail_available(&row.detail),
            })
            .collect();
        let records: Vec<TraceRow> = rows.into_iter().map(narrow_diagnostic).collect();

        // 最新一筆（不分頁）：headline 的來源。
        let latest = self
            .store
            .query_trace(&TraceQuery {
                session_id: Some(id.to_string()),
                limit: 1,
                ..Default::default()
            })?
            .into_iter()
            .next();
        // 最新一筆終態紀錄（不分頁）：失敗原因的來源。
        let outcome_row = self
            .store
            .query_trace(&TraceQuery {
                session_id: Some(id.to_string()),
                kind: Some("agent-session.outcome".to_string()),
                limit: 1,
                ..Default::default()
            })?
            .into_iter()
            .next();

        let verified = verified_for_current_claim(&record);
        // phase 比 state 細（fetched／working 這些值 state 裡根本沒有），
        // 有就用它；人工驗證過的那一刻 phase 已經是 `verified`。
        let phase = record.phase.clone();
        let state_source = phase.as_deref().unwrap_or_else(|| {
            // `AgentSessionState` 的 serde 是 kebab-case，與投影表同一組 key。
            state_taxonomy(record.state)
        });
        let state_label = if verified {
            work_state_label("verified")
        } else {
            work_state_label(state_source)
        };

        let failure_reason = outcome_row
            .as_ref()
            .filter(|row| {
                matches!(
                    row.outcome,
                    Some(TraceOutcome::Failed)
                        | Some(TraceOutcome::Unknown)
                        | Some(TraceOutcome::Expired)
                        | Some(TraceOutcome::Cancelled)
                )
            })
            .and_then(|row| detail_str(&row.detail, "reason"))
            // 終態紀錄沒有帶原因時，退回 session 自己的摘要（同樣是安全摘要）。
            .or_else(|| {
                matches!(
                    record.state,
                    AgentSessionState::Failed
                        | AgentSessionState::Unknown
                        | AgentSessionState::TimedOut
                )
                .then(|| record.detail.clone())
                .flatten()
            });

        let headline = match latest.as_ref() {
            Some(row) => step_label(row),
            // 一筆紀錄都沒有（例如 migration 之前建立的舊 session）：照實說。
            None => "這件工作還沒有留下任何紀錄".to_string(),
        };

        Ok(AgentSessionActivity {
            session_id: id.to_string(),
            headline,
            state_label: state_label.to_string(),
            phase,
            record_state: state_taxonomy(record.state).to_string(),
            lifecycle: if record.state.is_open() {
                "open".to_string()
            } else {
                "closed".to_string()
            },
            failure_reason,
            next_step: next_step_for(&record, verified).map(str::to_string),
            timeline,
            records,
            truncated,
            next_cursor,
        })
    }
}

/// `AgentSessionState` 的 taxonomy 字串（與 serde 的 kebab-case 同一組值）。
fn state_taxonomy(state: AgentSessionState) -> &'static str {
    match state {
        AgentSessionState::Created => "created",
        AgentSessionState::Active => "active",
        AgentSessionState::WaitingForInput => "waiting-for-input",
        AgentSessionState::WaitingForConsent => "waiting-for-consent",
        AgentSessionState::ClaimedCompleted => "claimed-completed",
        AgentSessionState::Failed => "failed",
        AgentSessionState::TimedOut => "timed-out",
        AgentSessionState::Cancelled => "cancelled",
        AgentSessionState::Expired => "expired",
        AgentSessionState::Closed => "closed",
        AgentSessionState::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        kind: &str,
        outcome: Option<TraceOutcome>,
        code: Option<&str>,
        detail: Value,
    ) -> TraceRow {
        TraceRow {
            id: 1,
            kind: kind.to_string(),
            outcome,
            code: code.map(str::to_string),
            detail,
            ..Default::default()
        }
    }

    #[test]
    fn every_known_kind_gets_a_human_label_and_unknown_kinds_never_leak() {
        // 認得的 kind：標籤是人話，而且不含 kind 字串本身。
        for kind in [
            "agent-session.resume-checked",
            "agent-session.dispatched",
            "agent-session.provider-model",
            "agent-session.task-delivered",
            "agent-session.outcome",
            "agent-session.interrupt-requested",
            "agent-session.subprocess-stderr",
            "agent.approval",
            "agent-session.verified",
            "agent-session.closed",
            "agent-session.emergency-stop",
            "agent-session.capability-issued",
            "consent.consumed",
            "consent.rejected",
            "memory.updated",
            "trace.pruned",
        ] {
            let label = step_label(&row(kind, None, None, Value::Null));
            assert!(!label.is_empty(), "{kind} 沒有人話標籤");
            assert!(!label.contains(kind), "{kind} 的標籤把 kind 字串漏出去了");
            assert_ne!(label, "其他紀錄", "{kind} 應該有專屬標籤");
        }
        // 認不得的 kind：一律「其他紀錄」，絕不把原始字串當標籤。
        let label = step_label(&row("brand.new-kind", None, None, Value::Null));
        assert_eq!(label, "其他紀錄");
        assert!(!label.contains("brand.new-kind"));
    }

    #[test]
    fn a_claim_is_never_labelled_as_completed() {
        let claimed = step_label(&row(
            "agent-session.outcome",
            Some(TraceOutcome::Claimed),
            Some("outcome.claimed-completed"),
            Value::Null,
        ));
        assert_eq!(claimed, "對方說已完成（尚未檢查）");
        // 主詞必須是「對方」：完成是誰說的，一眼看得出來。
        assert!(claimed.starts_with("對方說"), "claim 必須歸屬給 agent 自己");
        assert!(!claimed.contains("已由你確認"));
        // 失敗帶得出原因；沒有原因就不編一個。
        assert_eq!(
            step_label(&row(
                "agent-session.outcome",
                Some(TraceOutcome::Failed),
                Some("outcome.connector-error"),
                json!({"reason": "連接器沒有回應"}),
            )),
            "失敗：連接器沒有回應"
        );
        assert_eq!(
            step_label(&row(
                "agent-session.outcome",
                Some(TraceOutcome::Failed),
                Some("outcome.connector-error"),
                Value::Null,
            )),
            "失敗"
        );
        // 兩種 expired 說的是不同的事。
        assert_eq!(
            step_label(&row(
                "agent-session.outcome",
                Some(TraceOutcome::Expired),
                Some("lease.expired"),
                Value::Null,
            )),
            "授權時間到期"
        );
        assert_eq!(
            step_label(&row(
                "agent-session.outcome",
                Some(TraceOutcome::Expired),
                Some("outcome.timed-out"),
                Value::Null,
            )),
            "逾時，沒有等到結果"
        );
    }

    #[test]
    fn work_state_labels_match_the_desktop_projection_table() {
        // 桌面 `workState.ts` 的文案表：這裡一字不差地沿用。
        assert_eq!(work_state_label("created"), "正在準備");
        assert_eq!(work_state_label("fetched"), "已交給工作助手");
        assert_eq!(work_state_label("working"), "處理中");
        assert_eq!(work_state_label("active"), "處理中");
        assert_eq!(work_state_label("waiting-for-input"), "等你回答");
        assert_eq!(work_state_label("waiting-consent"), "等你允許");
        assert_eq!(work_state_label("claimed-completed"), "對方說已完成");
        assert_eq!(work_state_label("verified"), "已由你確認");
        assert_eq!(work_state_label("failed"), "失敗");
        assert_eq!(work_state_label("timed-out"), "逾時失敗");
        assert_eq!(work_state_label("expired"), "已到期");
        assert_eq!(work_state_label("closed"), "已取消");
        // 介面不認得的值：不猜，也不回傳原始字串。
        assert_eq!(work_state_label("brand-new-state"), "結果不確定");
        assert_eq!(work_state_label("unknown"), "結果不確定");
    }

    #[test]
    fn diagnostic_details_are_narrowed_to_the_redacted_tail() {
        let mut stderr_row = row(
            "agent-session.subprocess-stderr",
            None,
            None,
            json!({
                "tail": "warn: something",
                "truncated": true,
                "linesDropped": 4,
                "bytesSeen": 90_000,
                "linesSeen": 400,
            }),
        );
        stderr_row.class = TraceClass::Diagnostic;
        let narrowed = narrow_diagnostic(stderr_row);
        assert_eq!(narrowed.detail["tail"], "warn: something");
        assert_eq!(narrowed.detail["truncated"], true);
        assert_eq!(narrowed.detail["linesDropped"], 4);
        // tail 以外的欄位不外流。
        assert!(narrowed.detail.get("bytesSeen").is_none());
        assert!(narrowed.detail.get("linesSeen").is_none());

        // 非 diagnostic 的 detail 原封不動。
        let audit = row(
            "agent-session.closed",
            None,
            None,
            json!({"reason": "done"}),
        );
        assert_eq!(narrow_diagnostic(audit).detail["reason"], "done");
    }

    #[test]
    fn detail_available_is_false_for_nothing_to_show() {
        assert!(!detail_available(&Value::Null));
        assert!(!detail_available(&json!({})));
        assert!(!detail_available(&json!([])));
        assert!(detail_available(&json!({"reason": "x"})));
    }
}
