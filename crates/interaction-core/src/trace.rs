//! 追蹤紀錄契約 v1（trace record contract v1）。
//!
//! 三種紀錄責任共用同一張底層表（`audit`），但承諾不同：
//!
//! | class        | 內容                                   | 完整性承諾                               |
//! |--------------|----------------------------------------|------------------------------------------|
//! | `audit`      | 授權與狀態變更（consent／resume／終態） | 不取樣、不漏；關鍵轉移與狀態同一 transaction |
//! | `trace`      | 互動的接收、路由、派送、進度、結果      | 高頻進度可彙整，但保留起點／轉折／終點     |
//! | `diagnostic` | 協定解析、子程序 stderr、重連、內部錯誤 | 有界、脫敏、可截斷（標 truncated）         |
//!
//! 誠實階梯在型別層的表現：
//! - 不存在的因果資料寫 `None`，**不補造**（`causation_id`／`trace_id`／`source_at`）。
//! - `outcome` 可以是 [`TraceOutcome::Unknown`]；「不知道」是合法結果，謊稱成功不是。
//! - 排序權威永遠是儲存層的 `id`（核心接收序）；[`TraceRecord::source_at`] 是
//!   裝置／provider 自報的時間，不可信，只供參考。
//! - `actor` 只寫**已驗證**的身分類別（`human`／`runtime`／`watchdog`／
//!   `agent-session:<id>`／`device:<id>`／`api`）；絕不把呼叫端自報的 id 當成可信身分。
//! - `detail` 是安全摘要：不得含 token／secret／原始 prompt 全文；路徑只放
//!   `{"digest": sha256 前 12 hex, "basename": ...}`。

use crate::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 追蹤紀錄的 schema 版本；本輪 = 1。舊列（migration 前寫入）為 NULL，
/// 讀出時是 `None`，不補造。
pub const TRACE_RECORD_SCHEMA: i64 = 1;

/// [`TraceQuery::limit`] 的預設值。
pub const TRACE_QUERY_DEFAULT_LIMIT: u32 = 50;

/// [`TraceQuery::limit`] 的上限；查詢層一律 clamp 到 `1..=500`，
/// 不讓任何呼叫端把整段歷史載進記憶體。
pub const TRACE_QUERY_MAX_LIMIT: u32 = 500;

/// 紀錄責任分類。儲存層讀到 NULL／無法辨識的值時一律視為 [`TraceClass::Audit`]
/// （最保守：保存最久、不取樣）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum TraceClass {
    #[default]
    Audit,
    Trace,
    Diagnostic,
}

impl TraceClass {
    /// 三個 class 的權威順序（保存政策、計數、prune 都照這個走）。
    pub const ALL: [TraceClass; 3] = [TraceClass::Audit, TraceClass::Trace, TraceClass::Diagnostic];

    pub fn as_str(&self) -> &'static str {
        match self {
            TraceClass::Audit => "audit",
            TraceClass::Trace => "trace",
            TraceClass::Diagnostic => "diagnostic",
        }
    }

    /// 反向解析；不認得的字串回 `None`（由呼叫端決定怎麼誠實降級）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "audit" => Some(TraceClass::Audit),
            "trace" => Some(TraceClass::Trace),
            "diagnostic" => Some(TraceClass::Diagnostic),
            _ => None,
        }
    }
}

/// 一筆紀錄的結果。`None` = 這筆紀錄不描述結果（例如純粹的接收紀錄）；
/// [`TraceOutcome::Unknown`] = 有結果但我們不知道是什麼（誠實階梯）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum TraceOutcome {
    Accepted,
    Rejected,
    Ignored,
    Deferred,
    Completed,
    Failed,
    Cancelled,
    #[default]
    Unknown,
    /// 只能由人工驗證產生；adapter／agent 不得自稱 verified。
    Verified,
    Expired,
    /// 保存政策刪除（見 `TraceRetention`）。
    Pruned,
}

impl TraceOutcome {
    pub const ALL: [TraceOutcome; 11] = [
        TraceOutcome::Accepted,
        TraceOutcome::Rejected,
        TraceOutcome::Ignored,
        TraceOutcome::Deferred,
        TraceOutcome::Completed,
        TraceOutcome::Failed,
        TraceOutcome::Cancelled,
        TraceOutcome::Unknown,
        TraceOutcome::Verified,
        TraceOutcome::Expired,
        TraceOutcome::Pruned,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            TraceOutcome::Accepted => "accepted",
            TraceOutcome::Rejected => "rejected",
            TraceOutcome::Ignored => "ignored",
            TraceOutcome::Deferred => "deferred",
            TraceOutcome::Completed => "completed",
            TraceOutcome::Failed => "failed",
            TraceOutcome::Cancelled => "cancelled",
            TraceOutcome::Unknown => "unknown",
            TraceOutcome::Verified => "verified",
            TraceOutcome::Expired => "expired",
            TraceOutcome::Pruned => "pruned",
        }
    }

    /// 反向解析；不認得的字串回 `None`。儲存層讀到不認得的值會降級成
    /// [`TraceOutcome::Unknown`]（我們確實不知道那是什麼結果）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "accepted" => Some(TraceOutcome::Accepted),
            "rejected" => Some(TraceOutcome::Rejected),
            "ignored" => Some(TraceOutcome::Ignored),
            "deferred" => Some(TraceOutcome::Deferred),
            "completed" => Some(TraceOutcome::Completed),
            "failed" => Some(TraceOutcome::Failed),
            "cancelled" => Some(TraceOutcome::Cancelled),
            "unknown" => Some(TraceOutcome::Unknown),
            "verified" => Some(TraceOutcome::Verified),
            "expired" => Some(TraceOutcome::Expired),
            "pruned" => Some(TraceOutcome::Pruned),
            _ => None,
        }
    }
}

/// 要寫進儲存層的一筆紀錄（尚未有 `id`／`at`——那兩個由儲存層在接收當下決定）。
///
/// 用 class 建構子起手，再用 builder 補資訊：
///
/// ```
/// use interaction_core::{TraceOutcome, TraceRecord};
///
/// let record = TraceRecord::audit("agent-session.resume-checked")
///     .actor("runtime")
///     .outcome(TraceOutcome::Rejected)
///     .code("resume.scope-widened")
///     .trace_id("sess-1")
///     .caused_by("req-7")
///     .session("sess-1")
///     .detail(serde_json::json!({ "requested": "wider" }));
/// assert_eq!(record.kind, "agent-session.resume-checked");
/// ```
///
/// 註：`trace_id` 的 builder 叫 [`TraceRecord::trace_id`]，不是 `trace()`——
/// 後者已經是 `TraceClass::Trace` 的建構子（同一個 inherent impl 不能有兩個
/// 同名函式）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceRecord {
    pub class: TraceClass,
    /// 既有 audit kind 命名：`<領域>.<事件>`，例：`agent-session.resume-checked`。
    pub kind: String,
    /// 只寫已驗證的身分類別，不寫呼叫端自報的 id。
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TraceOutcome>,
    /// 結構化原因碼，kebab-case 且以領域為前綴，例：`resume.scope-widened`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// 同一段互動的 trace：agent 工作 = agentSessionId；resume 檢查 = 被接續的
    /// 原 session id（找得到時）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// 直接原因（上一個事件／messageId／requestId／resumeProviderSessionId…）；
    /// 不知道就 `None`，不補造。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 來源自報的發生時間（不可信，只供參考）；核心接收時間由儲存層寫 `at`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_at: Option<Timestamp>,
    /// 安全摘要。不得含 token／secret／原始 prompt 全文。
    pub detail: Value,
}

impl Default for TraceRecord {
    fn default() -> Self {
        Self {
            class: TraceClass::Audit,
            kind: String::new(),
            actor: "runtime".to_string(),
            outcome: None,
            code: None,
            trace_id: None,
            causation_id: None,
            session_id: None,
            source_at: None,
            detail: Value::Object(serde_json::Map::new()),
        }
    }
}

impl TraceRecord {
    pub fn new(class: TraceClass, kind: impl Into<String>) -> Self {
        Self {
            class,
            kind: kind.into(),
            ..Default::default()
        }
    }

    /// 授權與狀態變更：不取樣、不漏。
    pub fn audit(kind: impl Into<String>) -> Self {
        Self::new(TraceClass::Audit, kind)
    }

    /// 互動的接收、路由、派送、進度、結果。
    pub fn trace(kind: impl Into<String>) -> Self {
        Self::new(TraceClass::Trace, kind)
    }

    /// 協定解析、子程序 stderr、重連、內部錯誤（有界、脫敏、可截斷）。
    pub fn diagnostic(kind: impl Into<String>) -> Self {
        Self::new(TraceClass::Diagnostic, kind)
    }

    pub fn actor(mut self, actor: impl Into<String>) -> Self {
        self.actor = actor.into();
        self
    }

    pub fn outcome(mut self, outcome: TraceOutcome) -> Self {
        self.outcome = Some(outcome);
        self
    }

    pub fn code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// 設定 `trace_id`（同一段互動的串接鍵）。
    pub fn trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    /// 設定 `causation_id`（直接原因）。
    pub fn caused_by(mut self, causation_id: impl Into<String>) -> Self {
        self.causation_id = Some(causation_id.into());
        self
    }

    /// 設定 `session_id`（可查詢的 agent session 關聯）。
    pub fn session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// 設定來源自報時間；不可信，排序永遠不看它。
    pub fn source_at(mut self, source_at: Timestamp) -> Self {
        self.source_at = Some(source_at);
        self
    }

    pub fn detail(mut self, detail: Value) -> Self {
        self.detail = detail;
        self
    }
}

/// 追蹤紀錄查詢。每個欄位都是可選的 AND 條件；排序永遠 `id DESC`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<TraceClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TraceOutcome>,
    /// 含下界（`at >= since`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<Timestamp>,
    /// 不含上界（`at < until`），方便把時間切成不重疊的區段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<Timestamp>,
    /// 分頁 cursor：只回 `id < before_id`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_id: Option<i64>,
    /// 預設 [`TRACE_QUERY_DEFAULT_LIMIT`]，上限 [`TRACE_QUERY_MAX_LIMIT`]。
    pub limit: u32,
}

impl Default for TraceQuery {
    fn default() -> Self {
        Self {
            trace_id: None,
            session_id: None,
            kind: None,
            class: None,
            actor: None,
            outcome: None,
            since: None,
            until: None,
            before_id: None,
            limit: TRACE_QUERY_DEFAULT_LIMIT,
        }
    }
}

impl TraceQuery {
    /// clamp 到 `1..=TRACE_QUERY_MAX_LIMIT`；0 也會變成 1（回空集合不是查詢者的本意）。
    pub fn effective_limit(&self) -> u32 {
        self.limit.clamp(1, TRACE_QUERY_MAX_LIMIT)
    }
}

/// 從儲存層讀回的一筆紀錄。`id` 是核心接收序，也是唯一的排序權威。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceRow {
    pub id: i64,
    /// 核心接收時間（可信）。
    pub at: Timestamp,
    pub class: TraceClass,
    /// 紀錄 schema 版本；migration 之前寫入的舊列是 `None`（不補造）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<i64>,
    pub kind: String,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TraceOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 來源自報時間（不可信）；無法解析時降級成 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_at: Option<Timestamp>,
    pub detail: Value,
}

impl Default for TraceRow {
    fn default() -> Self {
        Self {
            id: 0,
            at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
            class: TraceClass::Audit,
            schema: None,
            kind: String::new(),
            actor: String::new(),
            outcome: None,
            code: None,
            trace_id: None,
            causation_id: None,
            session_id: None,
            source_at: None,
            detail: Value::Null,
        }
    }
}

/// 有界保存政策：每個 class 各有「天數」與「最多筆數（留最新）」兩道上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceRetention {
    pub audit_days: i64,
    pub audit_max: u64,
    pub trace_days: i64,
    pub trace_max: u64,
    pub diagnostic_days: i64,
    pub diagnostic_max: u64,
}

impl Default for TraceRetention {
    fn default() -> Self {
        Self {
            audit_days: 90,
            audit_max: 100_000,
            trace_days: 30,
            trace_max: 50_000,
            diagnostic_days: 7,
            diagnostic_max: 10_000,
        }
    }
}

impl TraceRetention {
    /// 取某個 class 的 `(天數, 最多筆數)`。天數為負視為「不限天數」。
    pub fn limits(&self, class: TraceClass) -> (i64, u64) {
        match class {
            TraceClass::Audit => (self.audit_days, self.audit_max),
            TraceClass::Trace => (self.trace_days, self.trace_max),
            TraceClass::Diagnostic => (self.diagnostic_days, self.diagnostic_max),
        }
    }
}

/// 一次 prune 實際刪掉的筆數（逐 class）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase", default)]
pub struct TracePruned {
    pub audit: u64,
    pub trace: u64,
    pub diagnostic: u64,
}

impl TracePruned {
    pub fn total(&self) -> u64 {
        self.audit + self.trace + self.diagnostic
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    /// 累加某個 class 的刪除數。
    pub fn add(&mut self, class: TraceClass, removed: u64) {
        match class {
            TraceClass::Audit => self.audit += removed,
            TraceClass::Trace => self.trace += removed,
            TraceClass::Diagnostic => self.diagnostic += removed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_fills_only_what_was_given() {
        let now = chrono::Utc::now();
        let record = TraceRecord::trace("agent-session.progress")
            .actor("agent-session:abc")
            .outcome(TraceOutcome::Deferred)
            .code("progress.throttled")
            .trace_id("t-1")
            .caused_by("msg-9")
            .session("sess-1")
            .source_at(now)
            .detail(serde_json::json!({ "dropped": 3 }));

        assert_eq!(record.class, TraceClass::Trace);
        assert_eq!(record.kind, "agent-session.progress");
        assert_eq!(record.actor, "agent-session:abc");
        assert_eq!(record.outcome, Some(TraceOutcome::Deferred));
        assert_eq!(record.code.as_deref(), Some("progress.throttled"));
        assert_eq!(record.trace_id.as_deref(), Some("t-1"));
        assert_eq!(record.causation_id.as_deref(), Some("msg-9"));
        assert_eq!(record.session_id.as_deref(), Some("sess-1"));
        assert_eq!(record.source_at, Some(now));
        assert_eq!(record.detail["dropped"], 3);

        // 沒給的因果資料就是 None，不補造。
        let bare = TraceRecord::diagnostic("codex.stderr");
        assert_eq!(bare.class, TraceClass::Diagnostic);
        assert_eq!(bare.actor, "runtime");
        assert!(bare.outcome.is_none());
        assert!(bare.trace_id.is_none());
        assert!(bare.causation_id.is_none());
        assert!(bare.session_id.is_none());
        assert!(bare.source_at.is_none());

        assert_eq!(
            TraceRecord::audit("consent.granted").class,
            TraceClass::Audit
        );
    }

    #[test]
    fn class_and_outcome_round_trip_through_as_str() {
        for class in TraceClass::ALL {
            assert_eq!(TraceClass::parse(class.as_str()), Some(class));
            let json = serde_json::to_value(class).unwrap();
            assert_eq!(json, serde_json::Value::String(class.as_str().to_string()));
            assert_eq!(
                serde_json::from_value::<TraceClass>(json).unwrap(),
                class,
                "serde 與 as_str 必須是同一組字串"
            );
        }
        assert_eq!(TraceClass::parse("nope"), None);

        for outcome in TraceOutcome::ALL {
            assert_eq!(TraceOutcome::parse(outcome.as_str()), Some(outcome));
            let json = serde_json::to_value(outcome).unwrap();
            assert_eq!(
                json,
                serde_json::Value::String(outcome.as_str().to_string())
            );
            assert_eq!(
                serde_json::from_value::<TraceOutcome>(json).unwrap(),
                outcome
            );
        }
        assert_eq!(TraceOutcome::parse("nope"), None);
    }

    #[test]
    fn deserializing_tolerates_unknown_and_missing_fields() {
        // 相容規則：未知欄位忽略，缺欄位用 Default。
        let record: TraceRecord = serde_json::from_str(
            r#"{"kind":"consent.granted","futureField":{"x":1},"anotherOne":"ignored"}"#,
        )
        .expect("未知欄位不得讓反序列化失敗");
        assert_eq!(record.kind, "consent.granted");
        assert_eq!(record.class, TraceClass::Audit);
        assert_eq!(record.actor, "runtime");
        assert!(record.code.is_none());

        let row: TraceRow = serde_json::from_str(
            r#"{"id":7,"at":"2026-09-15T00:00:00.000Z","class":"trace","kind":"k",
                "actor":"runtime","brandNewField":[1,2,3]}"#,
        )
        .expect("未知欄位不得讓反序列化失敗");
        assert_eq!(row.id, 7);
        assert_eq!(row.class, TraceClass::Trace);
        assert!(row.schema.is_none());
        assert!(row.source_at.is_none());

        let query: TraceQuery =
            serde_json::from_str(r#"{"traceId":"t-1","unknownFilter":true}"#).unwrap();
        assert_eq!(query.trace_id.as_deref(), Some("t-1"));
        assert_eq!(query.limit, TRACE_QUERY_DEFAULT_LIMIT);
    }

    #[test]
    fn query_limit_is_clamped_and_retention_defaults_match_the_contract() {
        assert_eq!(TraceQuery::default().effective_limit(), 50);
        assert_eq!(
            TraceQuery {
                limit: 9_999,
                ..Default::default()
            }
            .effective_limit(),
            TRACE_QUERY_MAX_LIMIT
        );
        assert_eq!(
            TraceQuery {
                limit: 0,
                ..Default::default()
            }
            .effective_limit(),
            1
        );

        let retention = TraceRetention::default();
        assert_eq!(retention.limits(TraceClass::Audit), (90, 100_000));
        assert_eq!(retention.limits(TraceClass::Trace), (30, 50_000));
        assert_eq!(retention.limits(TraceClass::Diagnostic), (7, 10_000));
        assert_eq!(TRACE_RECORD_SCHEMA, 1);

        let mut pruned = TracePruned::default();
        assert!(pruned.is_empty());
        pruned.add(TraceClass::Trace, 4);
        pruned.add(TraceClass::Diagnostic, 2);
        assert_eq!(pruned.total(), 6);
        assert_eq!(pruned.trace, 4);
        assert_eq!(pruned.audit, 0);
    }
}
