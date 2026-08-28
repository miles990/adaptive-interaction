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
