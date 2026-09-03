//! Closed-loop test against a REAL local HTTP mock device:
//! receptor read → facts; actuator execute → bounded body + idempotency key →
//! acknowledged-only receipt; failure → failed receipt (no fake success);
//! retry recovers; the device NEVER sees the unbounded requested values.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use interaction_adapter_declarative::{build, parse_spec};
use interaction_core::{
    ActionId, ActionParameters, ActionStatus, ActuatorId, BoundedAction, CorrelationId, PlanId,
    RiskClass, SessionContext, SessionId,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

type SetCall = (Value, Option<String>);

#[derive(Clone, Default)]
struct DeviceState {
    set_calls: Arc<Mutex<Vec<SetCall>>>,
    fail_next: Arc<Mutex<u32>>,
}

async fn spawn_mock_device() -> (SocketAddr, DeviceState) {
    let state = DeviceState::default();
    let app = Router::new()
        .route(
            "/status",
            get(|| async { Json(json!({"power": true, "brightness": 40})) }),
        )
        .route(
            "/set",
            post(
                |State(s): State<DeviceState>,
                 headers: axum::http::HeaderMap,
                 Json(body): Json<Value>| async move {
                    {
                        let mut fails = s.fail_next.lock().unwrap();
                        if *fails > 0 {
                            *fails -= 1;
                            return (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({})));
                        }
                    }
                    let idem = headers
                        .get("Idempotency-Key")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from);
                    s.set_calls.lock().unwrap().push((body, idem));
                    (axum::http::StatusCode::OK, Json(json!({"queued": true})))
                },
            ),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, state)
}

fn spec_for(addr: SocketAddr) -> String {
    format!(
        r#"
schemaVersion: "1.0"
id: desk-light
capabilities:
  - kind: receptor
    id: status
    transport: http
    request: {{ method: GET, url: "http://{addr}/status" }}
    facts:
      "on": "/power"
      brightness: "/brightness"
  - kind: actuator
    id: set
    channel: light
    transport: http
    confirmation: acknowledged
    timeoutMs: 2000
    retry: {{ attempts: 3, backoffMs: 50 }}
    request:
      method: POST
      url: "http://{addr}/set"
      body: {{ brightness: "{{{{magnitude}}}}", note: "{{{{intent}}}}" }}
"#
    )
}

fn bounded_action() -> BoundedAction {
    let now = chrono::Utc::now();
    BoundedAction {
        action_id: ActionId::new("action-test-1"),
        plan_id: PlanId::new("plan-1"),
        session_id: SessionId::new("sess-1"),
        actuator_id: ActuatorId::new("desk-light.set"),
        intent: "calm".into(),
        risk_class: RiskClass::BoundedSideEffect,
        requested: ActionParameters {
            magnitude: Some(1.0), // AI asked full power
            ..Default::default()
        },
        effective: ActionParameters {
            magnitude: Some(0.25), // policy clamped
            duration_ms: Some(500),
            message: None,
            extra: None,
        },
        policy_decisions: vec![],
        expires_at: now + chrono::Duration::minutes(1),
        issued_at: now,
        correlation_id: CorrelationId::new("c1"),
        metadata: Default::default(),
        schema_version: "1.0".into(),
    }
}

#[tokio::test]
async fn receptor_reads_real_device_facts() {
    let (addr, _state) = spawn_mock_device().await;
    let built = build(&parse_spec(&spec_for(addr)).unwrap(), None).unwrap();
    let receptor = &built.receptors[0];
    receptor
        .start(SessionContext {
            session_id: SessionId::new("sess-1"),
        })
        .await
        .unwrap();
    let obs = receptor.read().await.unwrap();
    assert_eq!(obs.facts.get("on"), Some(&json!(true)));
    assert_eq!(obs.facts.get("brightness"), Some(&json!(40)));
    assert_eq!(obs.receptor_id.as_str(), "desk-light.status");
}

#[tokio::test]
async fn actuator_sends_bounded_values_and_reports_acknowledged_only() {
    let (addr, state) = spawn_mock_device().await;
    let built = build(&parse_spec(&spec_for(addr)).unwrap(), None).unwrap();
    let actuator = &built.actuators[0];

    let receipt = actuator.execute(bounded_action()).await.unwrap();
    // Honesty: device 200 = acknowledged, NEVER completed/observed.
    assert_eq!(receipt.current_status, ActionStatus::Acknowledged);
    assert!(!receipt
        .timestamps
        .iter()
        .any(|(s, _)| matches!(s, ActionStatus::Completed | ActionStatus::Observed)));

    let calls = state.set_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let (body, idem) = &calls[0];
    // The device saw the BOUNDED 0.25, not the requested 1.0.
    assert_eq!(body["brightness"], json!(0.25));
    assert_eq!(body["note"], json!("calm"));
    assert_eq!(idem.as_deref(), Some("action-test-1"));
}

#[tokio::test]
async fn transient_device_failure_is_retried_then_succeeds() {
    let (addr, state) = spawn_mock_device().await;
    *state.fail_next.lock().unwrap() = 2; // first two attempts 503
    let built = build(&parse_spec(&spec_for(addr)).unwrap(), None).unwrap();
    let receipt = built.actuators[0].execute(bounded_action()).await.unwrap();
    assert_eq!(receipt.current_status, ActionStatus::Acknowledged);
    assert_eq!(state.set_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn unreachable_device_yields_failed_receipt_not_fake_success() {
    // Port 9 (discard) — nothing listens.
    let yaml = r#"
schemaVersion: "1.0"
id: ghost
capabilities:
  - kind: actuator
    id: set
    transport: http
    timeoutMs: 300
    retry: { attempts: 2, backoffMs: 10 }
    request: { method: POST, url: "http://127.0.0.1:1/set", body: {} }
"#;
    let built = build(&parse_spec(yaml).unwrap(), None).unwrap();
    let receipt = built.actuators[0].execute(bounded_action()).await.unwrap();
    assert_eq!(receipt.current_status, ActionStatus::Failed);
    assert!(receipt
        .errors
        .iter()
        .any(|e| e.code == "device-unreachable"));
}

#[tokio::test]
async fn expired_action_is_rejected_before_any_network_io() {
    let (addr, state) = spawn_mock_device().await;
    let built = build(&parse_spec(&spec_for(addr)).unwrap(), None).unwrap();
    let mut action = bounded_action();
    action.expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
    let err = built.actuators[0].execute(action).await.unwrap_err();
    assert!(err.to_string().contains("expired"));
    assert!(state.set_calls.lock().unwrap().is_empty());
}

/// HTTP/SSE 沒有常駐連線，健康度只能誠實反映「最近一次請求」：
/// 從未通訊過不得宣稱 healthy；請求失敗後也不得繼續宣稱 healthy。
#[tokio::test]
async fn http_health_is_not_hard_coded_healthy() {
    use interaction_core::HealthStatus;

    let (addr, _state) = spawn_mock_device().await;
    let built = build(&parse_spec(&spec_for(addr)).unwrap(), None).unwrap();
    let receptor = &built.receptors[0];
    let actuator = &built.actuators[0];

    // 還沒跟裝置說過話：未驗證，不是 healthy。
    assert_eq!(receptor.health().await.status, HealthStatus::Degraded);
    assert_eq!(actuator.status().await.status, HealthStatus::Degraded);

    // 成功通訊後才是 healthy。
    receptor.read().await.unwrap();
    assert_eq!(receptor.health().await.status, HealthStatus::Healthy);
    actuator.execute(bounded_action()).await.unwrap();
    assert_eq!(actuator.status().await.status, HealthStatus::Healthy);

    // 連不上的裝置：receptor 誠實 offline（下一次讀取可自行恢復）。
    let ghost = r#"
schemaVersion: "1.0"
id: ghost
capabilities:
  - kind: receptor
    id: status
    transport: http
    timeoutMs: 300
    request: { method: GET, url: "http://127.0.0.1:1/status" }
    facts: { on: "/power" }
  - kind: actuator
    id: set
    transport: http
    timeoutMs: 300
    retry: { attempts: 1, backoffMs: 0 }
    request: { method: POST, url: "http://127.0.0.1:1/set", body: {} }
"#;
    let dead = build(&parse_spec(ghost).unwrap(), None).unwrap();
    assert!(dead.receptors[0].read().await.is_err());
    let health = dead.receptors[0].health().await;
    assert_eq!(health.status, HealthStatus::Offline, "{health:?}");

    // actuator 端：失敗後不得再宣稱 healthy（訊息要說出失敗原因）。
    // 這裡刻意不用 offline——status() 會擋下派工，而只有派工才可能證明
    // 它恢復了；標成 offline 等於自鎖，那不是誠實。
    let receipt = dead.actuators[0].execute(bounded_action()).await.unwrap();
    assert_eq!(receipt.current_status, ActionStatus::Failed);
    let status = dead.actuators[0].status().await;
    assert_ne!(status.status, HealthStatus::Healthy, "{status:?}");
    assert!(status
        .message
        .unwrap_or_default()
        .contains("最近一次請求失敗"));
}

/// 送出後才逾時（連線已建立、請求已寫出）＝裝置**可能已經執行了**。
/// 誠實階梯：不得自動重送（重複的實體效果比誠實的未知更糟），也不得
/// 記成 failed。收據停在 dispatched＋outcomeUnknown，由 runtime 標 uncertain。
#[tokio::test]
async fn a_timeout_after_the_request_was_sent_is_uncertain_and_never_resent() {
    use std::sync::atomic::{AtomicU32, Ordering};

    let hits = Arc::new(AtomicU32::new(0));
    let counter = hits.clone();
    let app = Router::new().route(
        "/set",
        post(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                // 裝置收到了、正在動作，但回覆遲遲不來。
                tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
                (axum::http::StatusCode::OK, Json(json!({})))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let yaml = format!(
        r#"
schemaVersion: "1.0"
id: slow-light
capabilities:
  - kind: actuator
    id: set
    transport: http
    timeoutMs: 300
    retry: {{ attempts: 3, backoffMs: 10 }}
    request: {{ method: POST, url: "http://{addr}/set", body: {{}} }}
"#
    );
    let built = build(&parse_spec(&yaml).unwrap(), None).unwrap();
    let receipt = built.actuators[0].execute(bounded_action()).await.unwrap();

    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "送出後逾時絕不重送（retry 只適用『確定沒送出』）"
    );
    assert_eq!(
        receipt.current_status,
        ActionStatus::Dispatched,
        "已送出、結果未知：{receipt:?}"
    );
    assert_ne!(
        receipt.current_status,
        ActionStatus::Failed,
        "未知不得冒充失敗"
    );
    assert_eq!(receipt.driver_response["sendOutcomeUnknown"], json!(true));
    assert_eq!(receipt.driver_response["outcomeUnknown"], json!(true));
    assert!(
        !receipt
            .errors
            .iter()
            .any(|e| e.code == "device-unreachable"),
        "「可能已送達」不是 unreachable：{:?}",
        receipt.errors
    );
}

/// 相對照：連線根本建立不起來＝確定沒送出，沒有實體效果 → 才可以重試，
/// 全部失敗後誠實記成 failed（不是未知）。
#[tokio::test]
async fn a_connect_failure_is_definitely_not_sent_so_it_may_be_retried() {
    let yaml = r#"
schemaVersion: "1.0"
id: ghost-retry
capabilities:
  - kind: actuator
    id: set
    transport: http
    timeoutMs: 300
    retry: { attempts: 3, backoffMs: 10 }
    request: { method: POST, url: "http://127.0.0.1:1/set", body: {} }
"#;
    let built = build(&parse_spec(yaml).unwrap(), None).unwrap();
    let receipt = built.actuators[0].execute(bounded_action()).await.unwrap();
    assert_eq!(receipt.current_status, ActionStatus::Failed);
    assert!(receipt
        .errors
        .iter()
        .any(|e| e.code == "device-unreachable"));
    assert!(
        !receipt.driver_response.contains_key("sendOutcomeUnknown"),
        "連不上是確定沒送出，不是未知：{receipt:?}"
    );
}

// ---------------------------------------------------------------------------
// link-transports-047：HTTP 動器的 emergency stop 只有「裝置真的收下」才算
// 已停止。舊版無條件回 Ok(())：沒宣告 stopRequest 時什麼都沒送、送了失敗也
// 被丟掉——runtime 會把一台還在動作的裝置列進 stoppedActuators。
// ---------------------------------------------------------------------------

fn actuator_spec_with_stop(addr: SocketAddr, stop_path: &str) -> String {
    format!(
        r#"
schemaVersion: "1.0"
id: siren
capabilities:
  - kind: actuator
    id: sound
    transport: http
    externalSideEffect: true
    timeoutMs: 2000
    request: {{ method: POST, url: "http://{addr}/set", body: {{}} }}
    stopRequest: {{ method: POST, url: "http://{addr}{stop_path}", body: {{}} }}
"#
    )
}

#[tokio::test]
async fn an_actuator_without_a_stop_endpoint_never_claims_a_confirmed_stop() {
    let (addr, _state) = spawn_mock_device().await;
    let yaml = format!(
        r#"
schemaVersion: "1.0"
id: siren-no-stop
capabilities:
  - kind: actuator
    id: sound
    transport: http
    externalSideEffect: true
    request: {{ method: POST, url: "http://{addr}/set", body: {{}} }}
"#
    );
    let built = build(&parse_spec(&yaml).unwrap(), None).unwrap();
    let err = built.actuators[0]
        .emergency_stop()
        .await
        .expect_err("no stop endpoint = nothing was sent, so nothing is confirmed");
    let text = err.to_string();
    assert!(text.contains("no stop endpoint"), "{text}");
    assert!(text.contains("UNKNOWN"), "{text}");
}

#[tokio::test]
async fn a_stop_request_the_device_refuses_is_not_a_confirmed_stop() {
    let (addr, _state) = spawn_mock_device().await;
    // /nope 不存在 → axum 回 404：裝置收到了但沒有停。
    let built = build(
        &parse_spec(&actuator_spec_with_stop(addr, "/nope")).unwrap(),
        None,
    )
    .unwrap();
    let err = built.actuators[0]
        .emergency_stop()
        .await
        .expect_err("HTTP 404 is not a confirmed stop");
    let text = err.to_string();
    assert!(text.contains("404"), "{text}");
    assert!(text.contains("UNCONFIRMED"), "{text}");
}

#[tokio::test]
async fn a_stop_request_that_never_reaches_the_device_is_not_a_confirmed_stop() {
    let yaml = r#"
schemaVersion: "1.0"
id: siren-offline
capabilities:
  - kind: actuator
    id: sound
    transport: http
    externalSideEffect: true
    request: { method: POST, url: "http://127.0.0.1:1/set", body: {} }
    stopRequest: { method: POST, url: "http://127.0.0.1:1/stop", body: {} }
"#;
    let built = build(&parse_spec(yaml).unwrap(), None).unwrap();
    let err = built.actuators[0]
        .emergency_stop()
        .await
        .expect_err("a stop request that was never sent confirms nothing");
    assert!(err.to_string().contains("UNCONFIRMED"), "{err}");
}

#[tokio::test]
async fn a_stop_request_the_device_accepts_is_a_confirmed_stop() {
    let (addr, state) = spawn_mock_device().await;
    let built = build(
        &parse_spec(&actuator_spec_with_stop(addr, "/set")).unwrap(),
        None,
    )
    .unwrap();
    built.actuators[0]
        .emergency_stop()
        .await
        .expect("a 2xx from the device is the deepest honest confirmation this transport has");
    assert_eq!(
        state.set_calls.lock().unwrap().len(),
        1,
        "the stop request really went out"
    );
}

// ---------------------------------------------------------------------------
// link-transports-050：一個 fact 都沒解出來不是一次成功的觀察
// （runtime 會據此把 provider 記成「已測試」）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_reply_that_resolves_no_declared_fact_is_not_an_observation() {
    let (addr, _state) = spawn_mock_device().await;
    // pointer 全部指向裝置沒有的欄位（欄位改名／spec 打錯的等價情境）。
    let yaml = format!(
        r#"
schemaVersion: "1.0"
id: mismatched
capabilities:
  - kind: receptor
    id: status
    transport: http
    request: {{ method: GET, url: "http://{addr}/status" }}
    facts:
      lumens: "/renamed/lumens"
      humidity: "/renamed/humidity"
"#
    );
    let built = build(&parse_spec(&yaml).unwrap(), None).unwrap();
    let err = built.receptors[0]
        .read()
        .await
        .expect_err("zero resolved facts must not be reported as a successful read");
    let text = err.to_string();
    assert!(
        text.contains("/renamed/lumens"),
        "點名解不到的 pointer：{text}"
    );
    assert!(text.contains("power"), "也要點名裝置實際回了哪些鍵：{text}");
    // 健康度也不得繼續宣稱 healthy。
    assert_ne!(
        built.receptors[0].health().await.status,
        interaction_core::HealthStatus::Healthy
    );
}
