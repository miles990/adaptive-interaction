//! HTTP API end-to-end tests (acceptance scenario I: an HTTP host driving the
//! runtime with no MCP anywhere).

use interaction_runtime::{Runtime, RuntimeOptions};
use serde_json::{json, Value};

struct TestServer {
    _guard: tempfile::TempDir,
    pub base: String,
    pub token: String,
    pub agent_token: String,
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
        let agent_token = std::fs::read_to_string(runtime.paths.agent_token_file())
            .unwrap()
            .trim()
            .to_string();
        Self {
            _guard: guard,
            base: format!("http://{addr}"),
            token,
            agent_token,
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

    async fn delete(&self, path: &str) -> (u16, Value) {
        let resp = self
            .client
            .delete(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn builtin_domain_packs_have_api_install_uninstall_and_context_scope() {
    let server = TestServer::spawn().await;
    let (status, packs) = server.get("/v1/knowledge/domain-packs").await;
    assert_eq!(status, 200);
    assert_eq!(packs["count"], 10);
    assert!(packs["packs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["installed"] == true));

    let (status, removed) = server
        .delete("/v1/knowledge/domain-packs/task-planning")
        .await;
    assert_eq!(status, 200);
    assert_eq!(removed["installed"], false);
    let (status, restored) = server
        .post(
            "/v1/knowledge/domain-packs/task-planning/install",
            json!({}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(restored["installed"], true);

    let (status, bundle) = server
        .post(
            "/v1/memory/context-bundle",
            json!({"task": "plan", "agentId": "codex", "domains": ["task-planning"]}),
        )
        .await;
    assert_eq!(status, 200);
    assert!(bundle["includes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["domainPackId"] == "task-planning"));
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
async fn agent_token_cannot_grant_consent_clear_estop_or_mutate_human_state() {
    let server = TestServer::spawn().await;
    let agent = |method: reqwest::Method, path: &str, body: Value| {
        server
            .client
            .request(method, format!("{}{path}", server.base))
            .bearer_auth(&server.agent_token)
            .json(&body)
            .send()
    };

    for (method, path, body) in [
        (
            reqwest::Method::POST,
            "/v1/session/start",
            json!({"consents": ["actuator:x"]}),
        ),
        (
            reqwest::Method::POST,
            "/v1/session/consent",
            json!({"scope": "actuator:x"}),
        ),
        (
            reqwest::Method::PATCH,
            "/v1/policy",
            json!({"requireApprovalAt": "critical"}),
        ),
        (reqwest::Method::POST, "/v1/emergency-stop/clear", json!({})),
        (
            reqwest::Method::POST,
            "/v1/agent-sessions",
            json!({"agentId": "codex"}),
        ),
        (
            reqwest::Method::POST,
            "/v1/assets/import",
            json!({"content": "x"}),
        ),
    ] {
        let response = agent(method, path, body).await.unwrap();
        assert_eq!(response.status(), 403, "agent token must not access {path}");
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["code"], "token_scope_forbidden");
    }

    // Direct human/control-plane reads are also denied. Knowledge reads must
    // go through canonical tools so actor demotion and future session/domain
    // scoping have one enforcement point.
    for path in [
        "/v1/memory",
        "/v1/assets",
        "/v1/knowledge/nodes",
        "/v1/agent-sessions",
        "/v1/audit",
    ] {
        let response = agent(reqwest::Method::GET, path, Value::Null)
            .await
            .unwrap();
        assert_eq!(response.status(), 403, "agent token must not read {path}");
    }

    // Canonical read tool remains usable; token separation is capability
    // reduction, not an accidental outage of the AI tool plane.
    let response = agent(
        reqwest::Method::POST,
        "/v1/tools/interaction.status/call",
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);

    // Safety direction remains available to an AI host.
    let response = agent(
        reqwest::Method::POST,
        "/v1/emergency-stop",
        json!({"reason": "agent requested safe stop"}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    assert!(server.runtime.is_estopped());
}

#[tokio::test]
async fn knowledge_tools_require_a_live_session_token_and_enforce_tool_and_domain_scopes() {
    let server = TestServer::spawn().await;
    let (_status, allowed) = server
        .post(
            "/v1/knowledge/nodes",
            json!({
                "title": "Allowed rust fact",
                "content": "borrow checking permission example",
                "domains": ["rust"],
                "evidence": [{"url": "https://example.test/rust", "segment": "p=1"}]
            }),
        )
        .await;
    let (_status, secret) = server
        .post(
            "/v1/knowledge/nodes",
            json!({
                "title": "Private finance fact",
                "content": "permission finance secret",
                "domains": ["finance"],
                "evidence": [{"url": "https://example.test/finance", "segment": "p=1"}]
            }),
        )
        .await;
    let record = server
        .runtime
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: Some("provider.ai-agent.external-test".into()),
            agent_id: "external-test".into(),
            label: Some("scoped knowledge".into()),
            ttl_minutes: Some(5),
            data_scope: vec!["domain:rust".into()],
            tool_scope: vec![
                "knowledge.search".into(),
                "knowledge.get".into(),
                "knowledge.propose-claim".into(),
            ],
            consent_scope: vec!["knowledge:rust".into()],
            allow_write: false,
            max_cost: None,
            max_messages: Some(5),
            delegation: None,
            workdir: None,
        })
        .await
        .unwrap();
    let token = server
        .runtime
        .issue_agent_session_capability(record.session_id.as_str())
        .await
        .unwrap();
    let call = |token: String, tool: &'static str, body: Value| {
        server
            .client
            .post(format!("{}/v1/tools/{tool}/call", server.base))
            .bearer_auth(token)
            .json(&body)
            .send()
    };

    // The legacy shared agent token is not a Knowledge DB capability.
    let response = call(
        server.agent_token.clone(),
        "interaction.knowledge_search",
        json!({"query": "permission"}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 403);

    let response = call(
        token.clone(),
        "interaction.knowledge_search",
        json!({"query": "permission"}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let results: Value = response.json().await.unwrap();
    let ids = results["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["nodeId"].as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&allowed["nodeId"].as_str().unwrap()));
    assert!(!ids.contains(&secret["nodeId"].as_str().unwrap()));

    let response = call(
        token.clone(),
        "interaction.knowledge_get",
        json!({"nodeId": secret["nodeId"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        response.status(),
        403,
        "cross-domain node read must fail closed"
    );
    let response = call(token.clone(), "interaction.status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        403,
        "session token only grants its tool scope"
    );
    let response = call(
        token.clone(),
        "interaction.knowledge_propose_claim",
        json!({
            "title": "Scoped proposal",
            "content": "Only a candidate",
            "domains": ["finance"],
            "evidence": [{"url": "https://example.test/evidence", "segment": "p=2"}]
        }),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 403, "proposal cannot widen domain scope");
    let response = call(
        token.clone(),
        "interaction.knowledge_propose_claim",
        json!({
            "title": "Scoped proposal",
            "content": "Only a candidate",
            "domains": ["rust"],
            "evidence": [{"url": "https://example.test/evidence", "segment": "p=2"}]
        }),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let proposed: Value = response.json().await.unwrap();
    assert_eq!(proposed["status"], "candidate");
    let (_status, receipts) = server.get("/v1/knowledge/receipts").await;
    assert!(receipts["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|receipt| {
            receipt["agentSessions"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == record.session_id.as_str()))
        }));

    server
        .runtime
        .close_agent_session(record.session_id.as_str(), None, "done")
        .await
        .unwrap();
    let response = call(
        token,
        "interaction.knowledge_search",
        json!({"query": "permission"}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 401, "closed lease revokes session token");
}

#[tokio::test]
async fn multimodal_derivation_and_bounded_source_preview_are_real_api_operations() {
    let server = TestServer::spawn().await;
    let ppm = server._guard.path().join("api-preview.ppm");
    std::fs::write(&ppm, b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00").unwrap();
    let (status, asset) = server
        .post(
            "/v1/assets/import",
            json!({"path": ppm, "mediaType": "image", "source": "api-test"}),
        )
        .await;
    assert_eq!(status, 200);
    let hash = asset["hash"].as_str().unwrap();

    let (status, report) = server
        .post(&format!("/v1/assets/{hash}/derive"), json!({}))
        .await;
    assert_eq!(status, 200);
    let thumbnail_hash = report["derivatives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "thumbnail")
        .and_then(|item| item["outputHash"].as_str())
        .unwrap();
    let (status, derivatives) = server.get(&format!("/v1/assets/{hash}/derivatives")).await;
    assert_eq!(status, 200);
    assert!(derivatives["derivatives"].as_array().unwrap().len() >= 2);
    let (status, preview) = server
        .get(&format!("/v1/assets/{thumbnail_hash}/preview"))
        .await;
    assert_eq!(status, 200);
    assert_eq!(preview["mime"], "image/png");
    assert!(preview["dataBase64"].as_str().unwrap().starts_with("iVBOR"));
}

#[tokio::test]
async fn unified_activity_inbox_applies_compound_runtime_filters() {
    let server = TestServer::spawn().await;
    let record = server
        .runtime
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: Some("provider.ai-agent.external-test".into()),
            agent_id: "codex-reviewer".into(),
            label: Some("Rust repository review".into()),
            ttl_minutes: Some(5),
            data_scope: vec!["domain:rust".into()],
            tool_scope: vec!["knowledge.read".into()],
            consent_scope: vec![],
            allow_write: false,
            max_cost: None,
            max_messages: Some(5),
            delegation: None,
            workdir: None,
        })
        .await
        .unwrap();

    let (status, inbox) = server
        .get("/v1/activity/inbox?agent=codex&task=repository&domain=rust&status=created")
        .await;
    assert_eq!(status, 200);
    assert_eq!(inbox["count"], 1);
    assert_eq!(inbox["items"][0]["itemId"], record.session_id.as_str());

    let (status, empty) = server
        .get("/v1/activity/inbox?agent=codex&device=camera&domain=rust")
        .await;
    assert_eq!(status, 200);
    assert_eq!(empty["count"], 0, "all compound predicates must apply");

    let (status, candidate) = server
        .post(
            "/v1/knowledge/nodes",
            json!({
                "nodeType": "entity",
                "title": "Candidate by Claude",
                "content": "candidate",
                "asAgent": "claude-code",
                "domains": ["review"]
            }),
        )
        .await;
    assert_eq!(status, 200);
    let (status, inbox) = server
        .get("/v1/activity/inbox?status=candidate&agent=claude-code&domain=review")
        .await;
    assert_eq!(status, 200);
    assert_eq!(inbox["count"], 1);
    assert_eq!(inbox["items"][0]["itemId"], candidate["nodeId"]);
}

#[tokio::test]
async fn hardware_scan_is_authed_metadata_only_and_preserves_sensor_truth() {
    let server = TestServer::spawn().await;
    assert!(server.runtime.active_sensors().is_empty());

    let (status, report) = server.post("/v1/hardware/scan", json!({})).await;
    assert_eq!(status, 200);
    assert_eq!(report["sensorActivationAttempted"], false);
    assert!(report["devices"]
        .as_array()
        .is_some_and(|rows| rows.len() >= 17));
    assert!(server.runtime.active_sensors().is_empty());

    let unauthenticated = reqwest::Client::new()
        .post(format!("{}/v1/hardware/scan", server.base))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), 401);
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
async fn presentation_behavior_telemetry_is_sanitized_and_visible() {
    let server = TestServer::spawn().await;
    let (status, hello) = server
        .post(
            "/v1/presentation/hello",
            json!({
                "visible": true,
                "packId": "shu-agile",
                "behaviorState": {
                    "activation": 0.4,
                    "attention": 0.8,
                    "taskLoad": 0.6,
                    "interactionReadiness": 0.3,
                    "familiarity": 0.2,
                    "recentInterruptions": 1.0,
                    "currentFocus": "task.progress",
                    "lastInteractionAt": 1_700_000_000_000_i64,
                    "base": "idle",
                    "transient": "acting",
                    "explanation": "client must not control this text"
                }
            }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(hello["behaviorState"]["attention"], 0.8);
    assert_eq!(
        hello["behaviorExplanation"],
        "目前有工作進行中，所以注意力集中在任務上。"
    );

    let (status, current) = server.get("/v1/presentation").await;
    assert_eq!(status, 200);
    assert_eq!(current["behaviorState"]["currentFocus"], "task.progress");
    assert_ne!(
        current["behaviorExplanation"],
        "client must not control this text"
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
