//! HTTP API end-to-end tests (acceptance scenario I: an HTTP host driving the
//! runtime with no MCP anywhere).

use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};

struct TestServer {
    _guard: tempfile::TempDir,
    pub base: String,
    pub token: String,
    pub runtime: Runtime,
    client: reqwest::Client,
}

impl TestServer {
    async fn spawn() -> Self {
        let guard = tempfile::tempdir().unwrap();
        let runtime = Runtime::start(RuntimeOptions {
            home: Some(guard.path().to_path_buf()),
            acquire_lock: false,
            in_memory_db: false,
            spawn_watchdog: true,
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
}

#[tokio::test]
async fn health_is_public_but_api_requires_token() {
    let server = TestServer::spawn().await;
    // Health without token.
    let resp = reqwest::get(format!("{}/health", server.base))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // Status without token → 401.
    let resp = reqwest::Client::new()
        .get(format!("{}/v1/status", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    // Wrong token → 401.
    let resp = reqwest::Client::new()
        .get(format!("{}/v1/status", server.base))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    // Right token → 200.
    let (status, body) = server.get("/v1/status").await;
    assert_eq!(status, 200);
    assert_eq!(body["name"], "adaptive-interaction");
}

#[tokio::test]
async fn scenario_i_http_host_full_loop() {
    let server = TestServer::spawn().await;

    // 1. Discover capabilities.
    let (status, caps) = server.get("/v1/capabilities").await;
    assert_eq!(status, 200);
    assert!(caps["actuators"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == "conversation"));
    assert!(!caps["toolOperations"].as_array().unwrap().is_empty());

    // 2. Start a session.
    let (status, session) = server
        .post("/v1/session/start", json!({"label": "http-host"}))
        .await;
    assert_eq!(status, 200);
    assert_eq!(session["state"], "active");

    // 3. Push an observation (task completed).
    let (status, obs) = server
        .post(
            "/v1/receptors/task.lifecycle/push",
            json!({"facts": {"event": "task.completed", "title": "build"}}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(obs["facts"]["event"], "task.completed");

    // 4. Plan.
    let (status, plan) = server
        .post(
            "/v1/plans",
            json!({
                "intent": "success",
                "candidates": ["conversation"],
                "minChannels": 1,
                "maxChannels": 1,
                "allowNoAction": false
            }),
        )
        .await;
    assert_eq!(status, 200);
    let plan_id = plan["planId"].as_str().unwrap().to_string();

    // 5. Simulate (no side effects).
    let (status, sim) = server
        .post(&format!("/v1/plans/{plan_id}/simulate"), json!({}))
        .await;
    assert_eq!(status, 200);
    assert_eq!(sim["wouldExecute"], true);

    // 6. Execute.
    let (status, receipts) = server
        .post(&format!("/v1/plans/{plan_id}/execute"), json!({}))
        .await;
    assert_eq!(status, 200);
    let receipt = &receipts.as_array().unwrap()[0];
    assert_eq!(receipt["currentStatus"], "completed");
    let action_id = receipt["actionId"].as_str().unwrap().to_string();

    // 7. Receipt via GET.
    let (status, fetched) = server.get(&format!("/v1/actions/{action_id}")).await;
    assert_eq!(status, 200);
    assert_eq!(fetched["currentStatus"], "completed");
    // Timestamps show the honest path: accepted is present and distinct from completed.
    let states: Vec<&str> = fetched["timestamps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| pair[0].as_str().unwrap())
        .collect();
    assert!(states.contains(&"accepted"));
    assert!(states.contains(&"completed"));

    // 8. Events endpoint replays what happened (Last-Event-ID = 0 → everything).
    let events = server.runtime.events.replay_after(0);
    let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert!(types.contains(&"session.started"));
    assert!(types.contains(&"receptor.observation"));
    assert!(types.contains(&"plan.created"));
    assert!(types.contains(&"action.accepted"));
    assert!(types.contains(&"action.completed"));

    // 9. Outbox carries the actual message.
    let (_s, outbox) = server.get("/v1/outbox").await;
    assert!(!outbox.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn tool_calls_work_without_mcp() {
    let server = TestServer::spawn().await;
    server.post("/v1/session/start", json!({})).await;

    // status / capabilities / observe / plan / execute — all as TOOLS.
    let (s, _) = server
        .post("/v1/tools/interaction.status/call", json!({}))
        .await;
    assert_eq!(s, 200);
    let (s, caps) = server
        .post("/v1/tools/interaction.capabilities/call", json!({}))
        .await;
    assert_eq!(s, 200);
    assert!(caps["actuators"].as_array().is_some());

    let (s, plan) = server
        .post(
            "/v1/tools/interaction.plan/call",
            json!({"intent": "progress", "candidates": ["conversation"], "minChannels": 1, "maxChannels": 1, "allowNoAction": false}),
        )
        .await;
    assert_eq!(s, 200);
    let plan_id = plan["planId"].as_str().unwrap();

    // Underscore alias (platform-normalized name) also resolves.
    let (s, receipts) = server
        .post(
            "/v1/tools/interaction_execute/call",
            json!({"planId": plan_id}),
        )
        .await;
    assert_eq!(s, 200);
    let action_id = receipts[0]["actionId"].as_str().unwrap();

    let (s, receipt) = server
        .post(
            "/v1/tools/interaction.action_status/call",
            json!({"actionId": action_id}),
        )
        .await;
    assert_eq!(s, 200);
    assert_eq!(receipt["currentStatus"], "completed");

    // Unknown tool → 404.
    let (s, _) = server
        .post("/v1/tools/interaction.nope/call", json!({}))
        .await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn tool_exports_all_formats() {
    let server = TestServer::spawn().await;
    for format in ["openai", "anthropic", "gemini", "openapi", "json-schema"] {
        let (status, body) = server.get(&format!("/v1/tools/export/{format}")).await;
        assert_eq!(status, 200, "{format}");
        assert!(body["export"].is_object(), "{format}");
        assert_eq!(body["warnings"].as_array().unwrap().len(), 0, "{format}");
    }
    let (status, _) = server.get("/v1/tools/export/banana").await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn emergency_stop_via_api() {
    let server = TestServer::spawn().await;
    server.post("/v1/session/start", json!({})).await;
    let (status, result) = server
        .post("/v1/emergency-stop", json!({"reason": "drill"}))
        .await;
    assert_eq!(status, 200);
    assert_eq!(result["reason"], "drill");
    assert!(server.runtime.is_estopped());

    // Execution is now refused with 423 Locked.
    let (_s, plan) = server
        .post("/v1/plans", json!({"intent": "presence", "candidates": ["conversation"], "minChannels": 1, "maxChannels": 1, "allowNoAction": false}))
        .await;
    if let Some(plan_id) = plan["planId"].as_str() {
        let (s, _) = server
            .post(&format!("/v1/plans/{plan_id}/execute"), json!({}))
            .await;
        assert_eq!(s, 423);
    }
    let (s, _) = server.post("/v1/emergency-stop/clear", json!({})).await;
    assert_eq!(s, 200);
    assert!(!server.runtime.is_estopped());
}

#[tokio::test]
async fn recipe_crud_and_validation() {
    let server = TestServer::spawn().await;
    // Invalid recipe: precise errors, no crash.
    let (status, result) = server
        .post("/v1/recipes/validate", json!({"text": "id: x\nname: y\n"}))
        .await;
    assert_eq!(status, 200);
    assert_eq!(result["valid"], false);

    let recipe_yaml = r#"
id: test-recipe
name: Test
trigger:
  mode: any
  steps:
    - receptor: manual.event
decision:
  objective: test
actuation:
  candidates: [conversation]
  minChannels: 0
  maxChannels: 1
"#;
    let (status, created) = server
        .post("/v1/recipes", json!({"text": recipe_yaml}))
        .await;
    assert_eq!(status, 200);
    assert_eq!(created["id"], "test-recipe");

    let (status, list) = server.get("/v1/recipes").await;
    assert_eq!(status, 200);
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["recipe"]["id"] == "test-recipe"));

    // Disable, simulate, remove.
    let (s, _) = server
        .patch("/v1/recipes/test-recipe", json!({"enabled": false}))
        .await;
    assert_eq!(s, 200);
    server.post("/v1/session/start", json!({})).await;
    let (s, sim) = server
        .post("/v1/recipes/test-recipe/simulate", json!({}))
        .await;
    assert_eq!(s, 200);
    assert_eq!(
        sim["trigger"]["fired"], false,
        "disabled recipe must not fire"
    );
    let resp = reqwest::Client::new()
        .delete(format!("{}/v1/recipes/test-recipe", server.base))
        .bearer_auth(&server.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn sse_stream_replays_with_last_event_id() {
    let server = TestServer::spawn().await;
    server
        .post("/v1/session/start", json!({"label": "sse"}))
        .await;
    server
        .post(
            "/v1/receptors/manual.event/push",
            json!({"facts": {"event": "ping"}}),
        )
        .await;

    // Connect with Last-Event-ID: 0 → replay should include session.started.
    let resp = reqwest::Client::new()
        .get(format!("{}/v1/events", server.base))
        .bearer_auth(&server.token)
        .header("Last-Event-ID", "0")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    // Read the first chunk(s) with a timeout; must contain replayed events.
    let mut collected = String::new();
    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline && !collected.contains("receptor.observation") {
        match tokio::time::timeout(std::time::Duration::from_millis(500), stream.next()).await {
            Ok(Some(Ok(chunk))) => collected.push_str(&String::from_utf8_lossy(&chunk)),
            _ => break,
        }
    }
    assert!(collected.contains("session.started"), "got: {collected}");
    assert!(
        collected.contains("receptor.observation"),
        "got: {collected}"
    );
}

#[tokio::test]
async fn oversized_payload_is_rejected() {
    let server = TestServer::spawn().await;
    let big = "x".repeat(interaction_api::MAX_BODY_BYTES + 1024);
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/observations/query", server.base))
        .bearer_auth(&server.token)
        .json(&json!({"padding": big}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 413);
}

// ---------------------------------------------------------------------------
// Human layer endpoints
// ---------------------------------------------------------------------------

impl TestServer {
    async fn put(&self, path: &str, body: Value) -> (u16, Value) {
        let resp = self
            .client
            .put(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn human_layer_endpoints_roundtrip() {
    let server = TestServer::spawn().await;

    // Catalog is served and versioned.
    let (code, catalog) = server.get("/v1/catalog").await;
    assert_eq!(code, 200);
    assert!(catalog["entries"].as_array().unwrap().len() >= 30);

    // Human capability projection (zh-TW default locale).
    let (code, caps) = server
        .get("/v1/capabilities/human?includeUnavailable=true")
        .await;
    assert_eq!(code, 200);
    let conv = caps["actuators"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "conversation")
        .unwrap()
        .clone();
    assert_eq!(conv["displayName"], "對話訊息");
    let hash = conv["manifestHash"].as_str().unwrap().to_string();

    // AI description: wrong hash → 409; right hash → stored.
    let (code, _) = server
        .put(
            "/v1/capabilities/actuator/conversation/ai-description",
            json!({"locale": "zh-TW", "text": "x", "manifestHash": "0000000000000000"}),
        )
        .await;
    assert_eq!(code, 409);
    let (code, _) = server
        .put(
            "/v1/capabilities/actuator/conversation/ai-description",
            json!({"locale": "zh-TW", "text": "以對話文字回覆", "manifestHash": hash}),
        )
        .await;
    assert_eq!(code, 200);

    // UI preferences persist through the API.
    let (code, prefs) = server.get("/v1/ui/preferences").await;
    assert_eq!(code, 200);
    assert_eq!(prefs["mode"], "simple");
    let (code, prefs) = server
        .patch("/v1/ui/preferences", json!({"mode": "advanced"}))
        .await;
    assert_eq!(code, 200);
    assert_eq!(prefs["mode"], "advanced");
    let (code, _) = server
        .patch("/v1/ui/preferences", json!({"mode": "bogus"}))
        .await;
    assert_eq!(code, 400);

    // Pause is a separate state from emergency stop.
    let (code, pause) = server
        .post(
            "/v1/pause",
            json!({"durationMinutes": 60, "reason": "focus"}),
        )
        .await;
    assert_eq!(code, 200);
    assert_eq!(pause["paused"], json!(true));
    let (_, ready) = server.get("/ready").await;
    assert_eq!(
        ready["emergencyStop"],
        json!(false),
        "pause must NOT engage estop"
    );
    let (_, status) = server.get("/v1/status").await;
    assert_eq!(status["proactivePause"]["paused"], json!(true));
    let (code, pause) = server.post("/v1/pause/clear", json!({})).await;
    assert_eq!(code, 200);
    assert_eq!(pause["paused"], json!(false));

    // Onboarding draft + commit.
    let (code, ob) = server.get("/v1/onboarding").await;
    assert_eq!(code, 200);
    assert_eq!(ob["completed"], json!(false));
    let (code, _) = server.put("/v1/onboarding/draft", json!({"step": 2})).await;
    assert_eq!(code, 200);
    let (code, result) = server
        .post(
            "/v1/onboarding/commit",
            json!({"starterRecipes": ["starter-quiet-log"], "policyPatch": {"initiative": "suggest"}}),
        )
        .await;
    assert_eq!(code, 200);
    assert_eq!(result["completed"], json!(true));

    // Recipe summary + scenario simulation for the starter recipe.
    let (code, summary) = server
        .get("/v1/recipes/starter-quiet-log/summary?locale=zh-TW")
        .await;
    assert_eq!(code, 200);
    assert!(summary["summary"].as_str().unwrap().contains("不需要 AI"));

    server
        .post("/v1/session/start", json!({"label": "t"}))
        .await;
    let (code, report) = server
        .post(
            "/v1/recipes/starter-quiet-log/simulate-scenario",
            json!({"event": {"receptor": "task.lifecycle", "facts": {"event": "task.started"}}}),
        )
        .await;
    assert_eq!(code, 200);
    assert!(report["sideEffects"].as_str().unwrap().contains("模擬"));

    // Recipe YAML↔JSON conversion preserves unknown fields.
    let (code, converted) = server
        .post(
            "/v1/recipes/convert",
            json!({
                "to": "yaml",
                "text": "{\"id\":\"c\",\"name\":\"c\",\"futureField\":{\"x\":1},\"trigger\":{\"mode\":\"single\",\"steps\":[{\"receptor\":\"task.lifecycle\"}]},\"decision\":{\"objective\":\"t\"},\"actuation\":{\"candidates\":[\"conversation\"]}}"
            }),
        )
        .await;
    assert_eq!(code, 200);
    assert_eq!(converted["valid"], json!(true));
    assert!(converted["text"].as_str().unwrap().contains("futureField"));
}
