//! HTTP surface for the real one-shot consent (`maxUses`).
//!
//! 「只這一次」以前只是最短的 TTL：5 分鐘內可以用任意次數。這個檔案鎖住
//! 新的語意——`POST /v1/session/consent` 帶 `maxUses: 1` 之後，第一次成功
//! 派工就把它用掉，第二個 plan 必須被 Governor 擋在 `consent.required`。

use interaction_core::ActuatorId;
use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};

struct TestServer {
    _guard: tempfile::TempDir,
    base: String,
    token: String,
    runtime: Runtime,
    client: reqwest::Client,
}

impl TestServer {
    async fn spawn() -> Self {
        let guard = tempfile::tempdir().unwrap();
        let runtime = Runtime::start(RuntimeOptions {
            home: Some(guard.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: false,
        })
        .await
        .unwrap();
        let token = "test-token-0123456789".to_string();
        let (addr, _handle) =
            interaction_api::serve(runtime.clone(), "127.0.0.1", 0, token.clone())
                .await
                .unwrap();
        Self {
            _guard: guard,
            base: format!("http://{addr}"),
            token,
            runtime,
            client: reqwest::Client::new(),
        }
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        let resp = self
            .client
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn patch(&self, path: &str, body: Value) -> (u16, Value) {
        let resp = self
            .client
            .patch(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    /// One haptic plan through the mock actuator, executed; returns the receipt.
    async fn run_haptic_plan(&self) -> Value {
        let (status, plan) = self
            .post(
                "/v1/plans",
                json!({
                    "intent": "presence",
                    "candidates": ["mock.actuator"],
                    "minChannels": 1,
                    "maxChannels": 1,
                    "allowNoAction": false,
                    "preferredChannels": ["haptic"]
                }),
            )
            .await;
        assert_eq!(status, 200, "{plan}");
        let plan_id = plan["planId"].as_str().unwrap().to_string();
        let (status, receipts) = self
            .post(&format!("/v1/plans/{plan_id}/execute"), json!({}))
            .await;
        assert_eq!(status, 200, "{receipts}");
        receipts.as_array().unwrap()[0].clone()
    }
}

fn blocked_on_consent(receipt: &Value) -> bool {
    receipt["currentStatus"] == "blocked"
        && receipt["policyDecisions"]
            .as_array()
            .map(|ds| ds.iter().any(|d| d["rule"] == "consent.required"))
            .unwrap_or(false)
}

async fn armed_server() -> TestServer {
    let server = TestServer::spawn().await;
    server
        .runtime
        .registry
        .set_actuator_enabled(&ActuatorId::new("mock.actuator"), true)
        .await
        .unwrap();
    let (status, _) = server
        .patch(
            "/v1/policy",
            json!({"allowedChannels": ["conversation", "haptic"]}),
        )
        .await;
    assert_eq!(status, 200);
    let (status, session) = server
        .post("/v1/session/start", json!({"label": "one-shot"}))
        .await;
    assert_eq!(status, 200);
    assert_eq!(session["state"], "active");
    server
}

#[tokio::test]
async fn consent_with_max_uses_one_is_spent_by_the_first_dispatch() {
    let server = armed_server().await;

    let (status, session) = server
        .post(
            "/v1/session/consent",
            json!({"scope": "channel:haptic", "maxUses": 1}),
        )
        .await;
    assert_eq!(status, 200, "{session}");
    let consent = session["consents"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["scope"]["id"] == "haptic"))
        .unwrap_or_else(|| panic!("consent missing: {session}"));
    assert_eq!(consent["maxUses"], 1, "{consent}");
    assert_eq!(consent["remainingUses"], 1, "{consent}");

    let first = server.run_haptic_plan().await;
    assert!(!blocked_on_consent(&first), "第一次必須通過：{first}");

    let second = server.run_haptic_plan().await;
    assert!(
        blocked_on_consent(&second),
        "「只這一次」用過就要失效：{second}"
    );
}

#[tokio::test]
async fn consent_without_max_uses_keeps_the_unlimited_behaviour() {
    let server = armed_server().await;
    let (status, _) = server
        .post("/v1/session/consent", json!({"scope": "channel:haptic"}))
        .await;
    assert_eq!(status, 200);

    for round in 0..3 {
        let receipt = server.run_haptic_plan().await;
        assert!(
            !blocked_on_consent(&receipt),
            "沒帶 maxUses 的同意維持不限次（第 {round} 次）：{receipt}"
        );
    }
}

#[tokio::test]
async fn consent_with_max_uses_zero_is_rejected() {
    let server = armed_server().await;
    let (status, body) = server
        .post(
            "/v1/session/consent",
            json!({"scope": "channel:haptic", "maxUses": 0}),
        )
        .await;
    assert!(
        (400..500).contains(&status),
        "maxUses=0 是無意義的同意，必須被拒絕：{status} {body}"
    );
}

/// safety-invariants-058：HTTP 介面不得照收一個後端不會強制的 `maxUses`。
/// 受器／tool-operation 範圍沒有任何扣減點，帶次數必須被拒絕，而且 session
/// JSON 裡不得出現假的 `maxUses`／`remainingUses`。
#[tokio::test]
async fn consent_max_uses_is_rejected_for_scopes_that_never_spend_it() {
    let server = armed_server().await;

    for scope in ["receptor:microphone.listen", "tool:interaction.observe"] {
        let (status, body) = server
            .post(
                "/v1/session/consent",
                json!({"scope": scope, "expiresMinutes": 5, "maxUses": 1}),
            )
            .await;
        assert!(
            (400..500).contains(&status),
            "{scope} 沒有任何地方會用掉 maxUses，介面必須拒絕而不是照收：{status} {body}"
        );
    }

    let (status, session) = server.get("/v1/session").await;
    assert_eq!(status, 200, "{session}");
    let consents = session["consents"].as_array().cloned().unwrap_or_default();
    assert!(
        consents
            .iter()
            .all(|c| c["maxUses"].is_null() && c["remainingUses"].is_null()),
        "被拒絕的授權不得在 session JSON 留下假的次數：{session}"
    );

    // 純 TTL 的受器授權仍然可用。
    let (status, session) = server
        .post(
            "/v1/session/consent",
            json!({"scope": "receptor:microphone.listen", "expiresMinutes": 5}),
        )
        .await;
    assert_eq!(status, 200, "{session}");
    let mic = session["consents"]
        .as_array()
        .and_then(|cs| {
            cs.iter()
                .find(|c| {
                    c["scope"]["kind"] == json!("receptor")
                        && c["scope"]["id"] == json!("microphone.listen")
                })
                .cloned()
        })
        .expect("純 TTL 的受器授權必須成立");
    assert!(mic["maxUses"].is_null(), "{mic}");
}
