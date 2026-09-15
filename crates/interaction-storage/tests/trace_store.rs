//! 追蹤紀錄契約 v1 的儲存層驗收：schema 8→9 遷移、查詢與分頁、有界保存、
//! 「狀態＋稽核」同一 transaction，以及一個手動執行的效能量測。

use chrono::{Duration, Utc};
use interaction_core::{
    TraceClass, TraceOutcome, TraceQuery, TraceRecord, TraceRetention, TRACE_QUERY_MAX_LIMIT,
};
use interaction_storage::Store;
use serde_json::json;

/// 舊 DB（schema 8）升級到 9：舊列不補造，只是多了 NULL 欄位；新寫入才有
/// 完整的追蹤欄位。
#[test]
fn upgrading_a_schema_8_database_keeps_old_rows_and_adds_trace_columns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("state.db");
    {
        // 用 rusqlite 手工造一個 v8 的 DB：舊 audit 表＋幾列。
        let conn = rusqlite::Connection::open(&path).expect("open raw");
        conn.execute_batch(
            r#"
            CREATE TABLE audit (
                id     INTEGER PRIMARY KEY AUTOINCREMENT,
                at     TEXT NOT NULL,
                kind   TEXT NOT NULL,
                actor  TEXT NOT NULL,
                detail TEXT NOT NULL
            );
            INSERT INTO audit(at, kind, actor, detail) VALUES
                ('2026-09-01T00:00:00.000Z','consent.granted','human','{"scope":"mic"}'),
                ('2026-09-01T00:01:00.000Z','emergency.stop','human','{}');
            PRAGMA user_version = 8;
            "#,
        )
        .expect("seed v8");
    }

    let store = Store::open(&path).expect("open store");
    let old = store.query_trace(&TraceQuery::default()).expect("query");
    assert_eq!(old.len(), 2);
    assert_eq!(old[0].kind, "emergency.stop", "排序依 id DESC");
    assert_eq!(old[1].kind, "consent.granted");
    for row in &old {
        assert_eq!(
            row.class,
            TraceClass::Audit,
            "舊列 class NULL → 讀出為 audit"
        );
        assert_eq!(row.schema, None, "舊列不補造 schema 版本");
        assert_eq!(row.trace_id, None);
        assert_eq!(row.causation_id, None);
        assert_eq!(row.session_id, None);
        assert_eq!(row.outcome, None);
        assert_eq!(row.code, None);
        assert_eq!(row.source_at, None);
    }
    assert_eq!(old[1].detail, json!({"scope": "mic"}));

    // 新寫入帶完整追蹤欄位。
    let source_at = Utc::now() - Duration::seconds(3);
    let id = store
        .record(
            &TraceRecord::trace("agent-session.dispatched")
                .actor("agent-session:abc")
                .outcome(TraceOutcome::Accepted)
                .code("dispatch.accepted")
                .trace_id("t-1")
                .caused_by("msg-1")
                .session("sess-1")
                .source_at(source_at)
                .detail(json!({"channel": "codex"})),
        )
        .expect("record");
    assert!(id > 2, "row id 由 AUTOINCREMENT 接續舊列");

    let fresh = store
        .query_trace(&TraceQuery {
            trace_id: Some("t-1".into()),
            ..Default::default()
        })
        .expect("query by trace id");
    assert_eq!(fresh.len(), 1);
    let row = &fresh[0];
    assert_eq!(row.id, id);
    assert_eq!(row.class, TraceClass::Trace);
    assert_eq!(row.schema, Some(1));
    assert_eq!(row.actor, "agent-session:abc");
    assert_eq!(row.outcome, Some(TraceOutcome::Accepted));
    assert_eq!(row.code.as_deref(), Some("dispatch.accepted"));
    assert_eq!(row.causation_id.as_deref(), Some("msg-1"));
    assert_eq!(row.session_id.as_deref(), Some("sess-1"));
    assert_eq!(
        row.source_at.map(|t| t.timestamp_millis()),
        Some(source_at.timestamp_millis()),
        "source_at 只是附註，但存得下就要讀得回"
    );
    assert_eq!(row.detail, json!({"channel": "codex"}));

    // 相容入口仍然寫 class='audit', schema=1。
    store
        .audit("plan.created", "runtime", &json!({"steps": 2}))
        .expect("audit");
    let audits = store
        .query_trace(&TraceQuery {
            kind: Some("plan.created".into()),
            ..Default::default()
        })
        .expect("query audit");
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].class, TraceClass::Audit);
    assert_eq!(audits[0].schema, Some(1));
    assert_eq!(audits[0].trace_id, None, "相容入口不補造 trace_id");

    // audit_tail 既有欄位不變，新欄位附加。
    let tail = store.audit_tail(1).expect("tail");
    assert_eq!(tail[0]["kind"], json!("plan.created"));
    assert_eq!(tail[0]["actor"], json!("runtime"));
    assert_eq!(tail[0]["detail"], json!({"steps": 2}));
    assert_eq!(tail[0]["class"], json!("audit"));
    assert_eq!(tail[0]["traceId"], json!(null));
    assert!(tail[0]["at"].as_str().is_some());
    assert!(tail[0]["id"].as_i64().is_some());

    // 再開一次：migration 必須冪等。
    drop(store);
    let store = Store::open(&path).expect("reopen");
    assert_eq!(
        store
            .query_trace(&TraceQuery::default())
            .expect("query")
            .len(),
        4
    );
    let counts = store.trace_counts().expect("counts");
    assert_eq!(counts.get("audit"), Some(&3));
    assert_eq!(counts.get("trace"), Some(&1));
    assert_eq!(counts.get("diagnostic"), Some(&0));
}

#[test]
fn query_trace_filters_sorts_and_paginates() {
    let store = Store::open_in_memory().expect("store");

    // 第一批：trace class、trace-A / sess-A。
    for i in 0..5 {
        store
            .record(
                &TraceRecord::trace("agent-session.progress")
                    .actor("runtime")
                    .trace_id("trace-A")
                    .session("sess-A")
                    .outcome(TraceOutcome::Deferred)
                    .detail(json!({ "i": i })),
            )
            .expect("record");
    }
    std::thread::sleep(std::time::Duration::from_millis(5));
    let mark = Utc::now();
    std::thread::sleep(std::time::Duration::from_millis(5));

    // 第二批：另一段互動＋一筆 diagnostic。
    for i in 0..4 {
        store
            .record(
                &TraceRecord::trace("agent-session.dispatched")
                    .actor("agent-session:b")
                    .trace_id("trace-B")
                    .session("sess-B")
                    .outcome(TraceOutcome::Accepted)
                    .detail(json!({ "i": i })),
            )
            .expect("record");
    }
    store
        .record(
            &TraceRecord::diagnostic("codex.stderr")
                .actor("runtime")
                .trace_id("trace-B")
                .detail(json!({"truncated": true})),
        )
        .expect("record");
    store
        .audit("consent.granted", "human", &json!({}))
        .expect("audit");

    let all = store.query_trace(&TraceQuery::default()).expect("all");
    assert_eq!(all.len(), 11);
    let ids: Vec<i64> = all.iter().map(|r| r.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.reverse();
    assert_eq!(ids, sorted, "排序必須是穩定的 id DESC");

    let by_trace = store
        .query_trace(&TraceQuery {
            trace_id: Some("trace-B".into()),
            ..Default::default()
        })
        .expect("by trace");
    assert_eq!(by_trace.len(), 5);

    let by_session = store
        .query_trace(&TraceQuery {
            session_id: Some("sess-A".into()),
            ..Default::default()
        })
        .expect("by session");
    assert_eq!(by_session.len(), 5);

    let by_kind = store
        .query_trace(&TraceQuery {
            kind: Some("agent-session.dispatched".into()),
            ..Default::default()
        })
        .expect("by kind");
    assert_eq!(by_kind.len(), 4);

    let by_class = store
        .query_trace(&TraceQuery {
            class: Some(TraceClass::Diagnostic),
            ..Default::default()
        })
        .expect("by class");
    assert_eq!(by_class.len(), 1);
    assert_eq!(by_class[0].kind, "codex.stderr");

    let by_outcome = store
        .query_trace(&TraceQuery {
            outcome: Some(TraceOutcome::Deferred),
            ..Default::default()
        })
        .expect("by outcome");
    assert_eq!(by_outcome.len(), 5);

    let by_actor = store
        .query_trace(&TraceQuery {
            actor: Some("agent-session:b".into()),
            ..Default::default()
        })
        .expect("by actor");
    assert_eq!(by_actor.len(), 4);

    // 多個 filter 是 AND。
    let combined = store
        .query_trace(&TraceQuery {
            trace_id: Some("trace-B".into()),
            class: Some(TraceClass::Trace),
            ..Default::default()
        })
        .expect("combined");
    assert_eq!(combined.len(), 4);

    // since 是含下界，只回第二批（＋audit）。
    let since = store
        .query_trace(&TraceQuery {
            since: Some(mark),
            ..Default::default()
        })
        .expect("since");
    assert_eq!(since.len(), 6);
    assert!(since.iter().all(|r| r.at >= mark));

    // until 不含上界，只回第一批。
    let until = store
        .query_trace(&TraceQuery {
            until: Some(mark),
            ..Default::default()
        })
        .expect("until");
    assert_eq!(until.len(), 5);
    assert!(until.iter().all(|r| r.at < mark));

    // cursor 分頁：不重不漏。
    let mut seen: Vec<i64> = Vec::new();
    let mut cursor: Option<i64> = None;
    loop {
        let page = store
            .query_trace(&TraceQuery {
                limit: 3,
                before_id: cursor,
                ..Default::default()
            })
            .expect("page");
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 3);
        cursor = Some(page.last().expect("last").id);
        seen.extend(page.iter().map(|r| r.id));
    }
    assert_eq!(seen, ids, "分頁走完必須等於一次查完的結果");

    // limit clamp：上限 500。
    let bulk = Store::open_in_memory().expect("bulk store");
    for i in 0..(TRACE_QUERY_MAX_LIMIT + 10) {
        bulk.record(&TraceRecord::trace(format!("bulk.{i}")))
            .expect("record");
    }
    let clamped = bulk
        .query_trace(&TraceQuery {
            limit: 9_999,
            ..Default::default()
        })
        .expect("clamped");
    assert_eq!(clamped.len(), TRACE_QUERY_MAX_LIMIT as usize);
}

#[test]
fn prune_trace_enforces_days_and_count_limits_per_class() {
    let store = Store::open_in_memory().expect("store");
    let now = Utc::now();
    for i in 0..6 {
        store
            .record(&TraceRecord::audit(format!("audit.{i}")))
            .expect("audit row");
        store
            .record(&TraceRecord::trace(format!("trace.{i}")))
            .expect("trace row");
        store
            .record(&TraceRecord::diagnostic(format!("diag.{i}")))
            .expect("diag row");
    }

    let pruned_rows = |store: &Store| {
        store
            .query_trace(&TraceQuery {
                kind: Some("trace.pruned".into()),
                limit: 500,
                ..Default::default()
            })
            .expect("pruned rows")
    };

    // 都在政策範圍內：空刪不寫。
    let noop = store
        .prune_trace(&TraceRetention::default(), now)
        .expect("noop prune");
    assert!(noop.is_empty());
    assert!(
        pruned_rows(&store).is_empty(),
        "沒刪到東西就不得留下 trace.pruned"
    );
    assert_eq!(store.trace_counts().expect("counts").get("audit"), Some(&6));

    // 10 天後：diagnostic（7 天）全過期，trace（30）／audit（90）不動。
    let first = store
        .prune_trace(&TraceRetention::default(), now + Duration::days(10))
        .expect("prune day 10");
    assert_eq!(first.diagnostic, 6);
    assert_eq!(first.trace, 0);
    assert_eq!(first.audit, 0);
    let rows = pruned_rows(&store);
    assert_eq!(rows.len(), 1, "一次 prune 只寫一筆");
    assert_eq!(rows[0].class, TraceClass::Audit);
    assert_eq!(rows[0].actor, "runtime");
    assert_eq!(rows[0].outcome, Some(TraceOutcome::Pruned));
    assert_eq!(
        rows[0].detail["removed"],
        json!({"audit":0,"trace":0,"diagnostic":6})
    );
    assert_eq!(rows[0].detail["policy"]["diagnosticDays"], json!(7));
    assert_eq!(rows[0].detail["policy"]["traceMax"], json!(50_000));
    assert_eq!(
        store.trace_counts().expect("counts").get("diagnostic"),
        Some(&0)
    );

    // 40 天後：trace（30 天）也過期；audit 仍在 90 天內。
    let second = store
        .prune_trace(&TraceRetention::default(), now + Duration::days(40))
        .expect("prune day 40");
    assert_eq!(second.trace, 6);
    assert_eq!(second.audit, 0);
    assert_eq!(pruned_rows(&store).len(), 2);

    // 筆數上限：audit 只留最新 3 筆（此刻 audit 類有 audit.0..5 ＋ 兩筆
    // trace.pruned ＝ 8 列）。
    let retention = TraceRetention {
        audit_days: 3_650,
        audit_max: 3,
        ..Default::default()
    };
    let third = store.prune_trace(&retention, now).expect("prune by max");
    assert_eq!(third.audit, 5);
    let remaining = store
        .query_trace(&TraceQuery {
            class: Some(TraceClass::Audit),
            limit: 500,
            ..Default::default()
        })
        .expect("remaining");
    // 留最新 3 筆（audit.5 ＋ 兩筆 trace.pruned），加上這次 prune 自己寫的
    // 第三筆 trace.pruned——它在刪除之後才寫，所以會短暫超出上限一筆，
    // 下一次 prune 才收斂（不遞迴）。
    let kinds: Vec<&str> = remaining.iter().map(|r| r.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec!["trace.pruned", "trace.pruned", "trace.pruned", "audit.5"],
        "只留最新的列"
    );
    assert_eq!(
        remaining[0].detail["removed"],
        json!({"audit":5,"trace":0,"diagnostic":0})
    );
    assert_eq!(remaining[0].detail["policy"]["auditMax"], json!(3));
}

#[test]
fn record_and_agent_session_commit_in_one_transaction() {
    let store = Store::open_in_memory().expect("store");

    store
        .transaction(|tx| {
            tx.save_agent_session("sess-1", r#"{"id":"sess-1","state":"working"}"#)?;
            tx.record(
                &TraceRecord::audit("agent-session.state")
                    .actor("runtime")
                    .trace_id("sess-1")
                    .session("sess-1")
                    .outcome(TraceOutcome::Accepted)
                    .code("state.working"),
            )?;
            Ok(())
        })
        .expect("transaction");
    assert_eq!(store.all_agent_sessions().expect("sessions").len(), 1);
    assert_eq!(
        store
            .query_trace(&TraceQuery {
                session_id: Some("sess-1".into()),
                ..Default::default()
            })
            .expect("query")
            .len(),
        1
    );

    // 提交失敗：狀態與稽核都不得落地。
    store.force_next_transaction_error("disk on fire");
    let err = store
        .transaction(|tx| {
            tx.save_agent_session("sess-2", r#"{"id":"sess-2","state":"closed"}"#)?;
            tx.record(
                &TraceRecord::audit("agent-session.closed")
                    .actor("runtime")
                    .session("sess-2")
                    .outcome(TraceOutcome::Completed),
            )?;
            Ok(())
        })
        .expect_err("armed commit failure");
    assert!(err.to_string().contains("disk on fire"), "{err}");
    assert_eq!(
        store.all_agent_sessions().expect("sessions").len(),
        1,
        "sess-2 不得落地"
    );
    assert!(
        store
            .query_trace(&TraceQuery {
                session_id: Some("sess-2".into()),
                ..Default::default()
            })
            .expect("query")
            .is_empty(),
        "稽核列必須和它描述的狀態一起回滾"
    );
}

/// 效能量測（不是驗收）：手動跑
/// `cargo test -p interaction-storage -- --ignored perf_trace --nocapture`，
/// 印出一行 JSON 供文件引用。數字只代表這台機器的這次量測。
#[test]
#[ignore = "效能量測：需要 --ignored --nocapture 手動執行"]
fn perf_trace_write_and_query_baseline() {
    use std::time::Instant;

    const AUTOCOMMIT_ROWS: usize = 20_000;
    const BATCH_ROWS: usize = 1_000;
    const QUERY_ITERATIONS: usize = 100;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("perf.db");
    let store = Store::open(&path).expect("store");

    let kinds = [
        "agent-session.received",
        "agent-session.dispatched",
        "agent-session.progress",
        "agent-session.closed",
    ];
    let make = |i: usize| {
        let class = match i % 3 {
            0 => TraceClass::Audit,
            1 => TraceClass::Trace,
            _ => TraceClass::Diagnostic,
        };
        TraceRecord::new(class, kinds[i % kinds.len()])
            .actor("runtime")
            .trace_id(format!("t-{}", i % 200))
            .session(format!("s-{}", i % 50))
            .caused_by(format!("m-{i}"))
            .outcome(TraceOutcome::Accepted)
            .code("perf.sample")
            .detail(json!({ "i": i, "note": "perf baseline row" }))
    };

    let started = Instant::now();
    for i in 0..AUTOCOMMIT_ROWS {
        store.record(&make(i)).expect("record");
    }
    let autocommit = started.elapsed();

    let started = Instant::now();
    store
        .transaction(|tx| {
            for i in 0..BATCH_ROWS {
                tx.record(&make(AUTOCOMMIT_ROWS + i))?;
            }
            Ok(())
        })
        .expect("batch");
    let batch = started.elapsed();

    let bench = |query: TraceQuery| {
        let started = Instant::now();
        let mut rows = 0usize;
        for _ in 0..QUERY_ITERATIONS {
            rows += store.query_trace(&query).expect("query").len();
        }
        (
            started.elapsed().as_secs_f64() * 1_000.0 / QUERY_ITERATIONS as f64,
            rows / QUERY_ITERATIONS,
        )
    };
    let (by_trace_ms, by_trace_rows) = bench(TraceQuery {
        trace_id: Some("t-7".into()),
        limit: 50,
        ..Default::default()
    });
    let (by_session_ms, by_session_rows) = bench(TraceQuery {
        session_id: Some("s-3".into()),
        limit: 50,
        ..Default::default()
    });
    let (by_kind_ms, by_kind_rows) = bench(TraceQuery {
        kind: Some("agent-session.progress".into()),
        limit: 50,
        ..Default::default()
    });
    let started = Instant::now();
    let mut tail_rows = 0usize;
    for _ in 0..QUERY_ITERATIONS {
        tail_rows += store.audit_tail(50).expect("tail").len();
    }
    let tail_ms = started.elapsed().as_secs_f64() * 1_000.0 / QUERY_ITERATIONS as f64;

    let before = store.trace_counts().expect("counts");
    let retention = TraceRetention {
        audit_max: 1_000,
        trace_max: 1_000,
        diagnostic_max: 1_000,
        ..Default::default()
    };
    let started = Instant::now();
    let pruned = store.prune_trace(&retention, Utc::now()).expect("prune");
    let prune_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let after = store.trace_counts().expect("counts");

    let size_of = |suffix: &str| {
        let mut p = path.clone();
        if !suffix.is_empty() {
            p.set_file_name(format!("perf.db{suffix}"));
        }
        std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
    };
    let db_bytes = size_of("");
    let wal_bytes = size_of("-wal");

    let report = json!({
        "measurement": "perf_trace_write_and_query_baseline",
        "writeAutocommit": {
            "rows": AUTOCOMMIT_ROWS,
            "totalMs": (autocommit.as_secs_f64() * 1_000.0 * 100.0).round() / 100.0,
            "avgUs": (autocommit.as_secs_f64() * 1_000_000.0 / AUTOCOMMIT_ROWS as f64 * 100.0).round() / 100.0,
        },
        "writeTransaction": {
            "rows": BATCH_ROWS,
            "totalMs": (batch.as_secs_f64() * 1_000.0 * 100.0).round() / 100.0,
            "avgUs": (batch.as_secs_f64() * 1_000_000.0 / BATCH_ROWS as f64 * 100.0).round() / 100.0,
        },
        "queryAvgMs": {
            "iterations": QUERY_ITERATIONS,
            "byTraceId": (by_trace_ms * 1_000.0).round() / 1_000.0,
            "bySessionId": (by_session_ms * 1_000.0).round() / 1_000.0,
            "byKind": (by_kind_ms * 1_000.0).round() / 1_000.0,
            "tail50": (tail_ms * 1_000.0).round() / 1_000.0,
        },
        "queryRows": {
            "byTraceId": by_trace_rows,
            "bySessionId": by_session_rows,
            "byKind": by_kind_rows,
            "tail50": tail_rows / QUERY_ITERATIONS,
        },
        "prune": {
            "ms": (prune_ms * 100.0).round() / 100.0,
            "removed": {"audit": pruned.audit, "trace": pruned.trace, "diagnostic": pruned.diagnostic},
            "countsBefore": before,
            "countsAfter": after,
        },
        "dbBytes": {"main": db_bytes, "wal": wal_bytes, "total": db_bytes + wal_bytes},
    });
    println!("PERF {}", serde_json::to_string(&report).expect("json"));
}
