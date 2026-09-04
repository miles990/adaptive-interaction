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
            resume_provider_session_id: None,
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
            resume_provider_session_id: None,
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
    // FirstSuccess 的「看過」旗標：PATCH 合併、GET 回傳；預設 false。
    let (_, prefs) = server.get("/v1/ui/preferences").await;
    assert_eq!(prefs["firstSuccessSeen"], false);
    let (code, prefs) = server
        .patch("/v1/ui/preferences", json!({"firstSuccessSeen": true}))
        .await;
    assert_eq!(code, 200);
    assert_eq!(prefs["firstSuccessSeen"], true);
    assert_eq!(prefs["mode"], "advanced", "merge keeps the other fields");
    let (_, prefs) = server.get("/v1/ui/preferences").await;
    assert_eq!(prefs["firstSuccessSeen"], true);

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
    // Preview is a dry run: it reports the diff and changes nothing.
    let (code, preview) = server
        .post(
            "/v1/onboarding/preview",
            json!({"disableReceptors": ["task.lifecycle"], "starterRecipes": ["starter-quiet-log"]}),
        )
        .await;
    assert_eq!(code, 200);
    assert_eq!(preview["receptors"][0]["id"], json!("task.lifecycle"));
    assert_eq!(preview["receptors"][0]["from"], json!("on"));
    assert_eq!(preview["receptors"][0]["to"], json!("off"));
    assert_eq!(preview["receptors"][0]["changed"], json!(true));
    assert_eq!(preview["changed"], json!(true));
    let (_, after_preview) = server.get("/v1/onboarding").await;
    assert_eq!(
        after_preview["completed"],
        json!(false),
        "preview must not complete onboarding"
    );
    // Unknown ids are refused the same way commit refuses them.
    let (code, _) = server
        .post(
            "/v1/onboarding/preview",
            json!({"enableReceptors": ["no.such.receptor"]}),
        )
        .await;
    assert_eq!(code, 404);
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

/// iPhone 配對面是「人類層」：配對指紋、裝置清單、撤銷與 BLE 閘道都不在
/// AI/工具平面。agent token 與 agent-session token 一律 403（fail closed）。
#[tokio::test]
async fn mobile_routes_are_human_only_for_agent_and_session_tokens() {
    let server = TestServer::spawn().await;
    let record = server
        .runtime
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: Some("provider.ai-agent.mobile-test".into()),
            agent_id: "mobile-test".into(),
            label: Some("mobile scope probe".into()),
            ttl_minutes: Some(5),
            data_scope: vec![],
            tool_scope: vec!["status".into()],
            consent_scope: vec![],
            allow_write: false,
            max_cost: None,
            max_messages: Some(5),
            delegation: None,
            workdir: None,
            resume_provider_session_id: None,
        })
        .await
        .unwrap();
    let session_token = server
        .runtime
        .issue_agent_session_capability(record.session_id.as_str())
        .await
        .unwrap();

    let routes = [
        (reqwest::Method::GET, "/v1/mobile/status", Value::Null),
        (
            reqwest::Method::POST,
            "/v1/mobile/pairing-session",
            json!({}),
        ),
        (reqwest::Method::DELETE, "/v1/mobile/devices/x", Value::Null),
        (
            reqwest::Method::POST,
            "/v1/mobile/ble/scan",
            json!({"durationMs": 1000}),
        ),
        // 每機動作（停止感測／測試連線）同樣是人類層。
        (
            reqwest::Method::POST,
            "/v1/mobile/devices/x/sensors/stop",
            json!({}),
        ),
        (
            reqwest::Method::POST,
            "/v1/mobile/devices/x/test",
            json!({}),
        ),
    ];
    for (label, token) in [
        ("agent token", server.agent_token.clone()),
        ("agent-session token", session_token.clone()),
    ] {
        for (method, path, body) in &routes {
            let response = server
                .client
                .request(method.clone(), format!("{}{path}", server.base))
                .bearer_auth(&token)
                .json(body)
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                403,
                "{label} must not reach {method} {path}"
            );
            let error: Value = response.json().await.unwrap();
            assert_eq!(error["error"]["code"], "token_scope_forbidden");
        }
    }

    // 人類 token 仍可讀狀態（權限縮減，不是整條路線壞掉）。
    let (code, status) = server.get("/v1/mobile/status").await;
    assert_eq!(code, 200);
    assert_eq!(status["started"], json!(false));

    // 人類 token 打不存在的裝置 → 404（不是假裝停成功）。
    let (code, error) = server
        .post("/v1/mobile/devices/iphone-nope/sensors/stop", json!({}))
        .await;
    assert_eq!(code, 404, "{error}");
    let (code, error) = server
        .post("/v1/mobile/devices/iphone-nope/test", json!({}))
        .await;
    assert_eq!(code, 404, "{error}");
}

/// 「停止所有感測」是安全遞減操作：三種 token 都可呼叫，回傳誠實報告
/// （頂層 stopped＝全部確認、uncertain＝有來源沒回覆），未認證仍 401。
#[tokio::test]
async fn sensors_stop_is_available_to_every_token_and_reports_honestly() {
    let server = TestServer::spawn().await;

    // 人類：沒有手機、本機沒在擷取 → stopped=true、uncertain=false、devices 空。
    let (code, report) = server.post("/v1/sensors/stop", json!({})).await;
    assert_eq!(code, 200);
    assert_eq!(report["stopped"], json!(true), "{report}");
    assert_eq!(report["uncertain"], json!(false), "{report}");
    assert_eq!(report["local"]["microphone"], json!("idle"), "{report}");
    assert_eq!(report["devices"], json!([]), "{report}");

    let record = server
        .runtime
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: Some("provider.ai-agent.sensors-stop".into()),
            agent_id: "sensors-stop".into(),
            label: Some("sensors stop scope".into()),
            ttl_minutes: Some(5),
            data_scope: vec![],
            tool_scope: vec!["status".into()],
            consent_scope: vec![],
            allow_write: false,
            max_cost: None,
            max_messages: Some(5),
            delegation: None,
            workdir: None,
            resume_provider_session_id: None,
        })
        .await
        .unwrap();
    let session_token = server
        .runtime
        .issue_agent_session_capability(record.session_id.as_str())
        .await
        .unwrap();

    for (label, token) in [
        ("agent token", server.agent_token.clone()),
        ("agent-session token", session_token.clone()),
    ] {
        let response = server
            .client
            .post(format!("{}/v1/sensors/stop", server.base))
            .bearer_auth(&token)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            200,
            "{label} must be able to stop sensing (safety-decreasing)"
        );
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["stopped"], json!(true), "{label}: {body}");
    }

    // audit 記得出是誰停的（agent 不會被記成 "api"）。
    let audits = server.runtime.store.audit_tail(50).unwrap();
    let actors: Vec<String> = audits
        .iter()
        .filter(|a| a["kind"] == json!("sensor.stopped-all"))
        .map(|a| a["actor"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(actors.iter().any(|a| a == "api"), "{actors:?}");
    assert!(actors.iter().any(|a| a == "agent"), "{actors:?}");
    assert!(
        actors.iter().any(|a| a.starts_with("agent:")),
        "session token 要記成 agent:<id>@<session>：{actors:?}"
    );

    // 未認證仍然 401。
    let response = server
        .client
        .post(format!("{}/v1/sensors/stop", server.base))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

/// 「claim ≠ verified」必須由 Runtime 確定性強制，而不是靠 UI 藏按鈕：
/// 只有人類 token 能驗證 claim，且 AI 不得用自我回報的 `report` 路徑
/// 把自己升級成 verified。
#[tokio::test]
async fn only_a_human_token_can_verify_a_claim_and_no_agent_can_self_upgrade() {
    let server = TestServer::spawn().await;
    let record = server
        .runtime
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: Some("provider.ai-agent.external-test".into()),
            agent_id: "external-test".into(),
            label: Some("verify scope".into()),
            ttl_minutes: Some(5),
            data_scope: vec![],
            tool_scope: vec![],
            consent_scope: vec![],
            allow_write: false,
            max_cost: None,
            max_messages: Some(5),
            delegation: None,
            workdir: None,
            resume_provider_session_id: None,
        })
        .await
        .unwrap();
    let id = record.session_id.as_str().to_string();
    let session_token = server
        .runtime
        .issue_agent_session_capability(&id)
        .await
        .unwrap();

    // 先有 claim（人類路徑），才有東西可以驗證。
    let (status, _) = server
        .post(
            &format!("/v1/agent-sessions/{id}/report"),
            json!({"event": "claimed-completed", "payload": {"summary": "我覺得做完了"}}),
        )
        .await;
    assert_eq!(status, 200);

    // agent token 與 session token 都不得碰 verify。
    for (label, token) in [
        ("agent token", server.agent_token.clone()),
        ("agent-session token", session_token.clone()),
    ] {
        let response = server
            .client
            .post(format!("{}/v1/agent-sessions/{id}/verify", server.base))
            .bearer_auth(&token)
            .json(&json!({"note": "我自己驗自己"}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            403,
            "{label} must never verify an agent claim"
        );
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["code"], "token_scope_forbidden");
    }
    // 沒有任何自我驗證留下痕跡。
    let (_, before) = server.get(&format!("/v1/agent-sessions/{id}")).await;
    assert_eq!(before["humanVerified"], Value::Null);
    assert_eq!(before["state"], "claimed-completed");

    // `verified` 不是可以自我回報的事件：report 路徑必須拒絕它。
    let (status, error) = server
        .post(
            &format!("/v1/agent-sessions/{id}/report"),
            json!({"event": "verified", "payload": {}}),
        )
        .await;
    assert_eq!(status, 400, "self-reported 'verified' must be refused");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("verified"),
        "{error}"
    );
    let (_, still) = server.get(&format!("/v1/agent-sessions/{id}")).await;
    assert_eq!(still["humanVerified"], Value::Null);
    assert_eq!(still["state"], "claimed-completed");

    // 人類 token 才是唯一的升級路徑。
    let (status, verified) = server
        .post(
            &format!("/v1/agent-sessions/{id}/verify"),
            json!({"note": "我親自看過輸出"}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(verified["humanVerified"]["note"], "我親自看過輸出");
    // 驗證是人類的註記，不是 agent 的新聲稱：狀態仍停在 claimed-completed。
    assert_eq!(verified["state"], "claimed-completed");
}

/// 誠實階梯的 `unknown`：程序結束卻沒有結果時，API 回報的是「未知」，
/// 而不是失敗或成功，而且未知的 session 不可被驗證成完成。
#[tokio::test]
async fn an_unknown_outcome_is_reportable_and_can_never_be_verified() {
    let server = TestServer::spawn().await;
    let record = server
        .runtime
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: Some("provider.ai-agent.external-test".into()),
            agent_id: "external-test".into(),
            label: Some("unknown outcome".into()),
            ttl_minutes: Some(5),
            data_scope: vec![],
            tool_scope: vec![],
            consent_scope: vec![],
            allow_write: false,
            max_cost: None,
            max_messages: Some(5),
            delegation: None,
            workdir: None,
            resume_provider_session_id: None,
        })
        .await
        .unwrap();
    let id = record.session_id.as_str().to_string();

    let (status, body) = server
        .post(
            &format!("/v1/agent-sessions/{id}/report"),
            json!({"event": "unknown", "payload": {"reason": "程序已結束而未回報結果"}}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["state"], "unknown");

    // 未知不是 claim：不得被升級成 verified。
    let (status, error) = server
        .post(&format!("/v1/agent-sessions/{id}/verify"), json!({}))
        .await;
    assert_eq!(status, 409);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("claimed-completed"),
        "{error}"
    );

    // 未知是 terminal：不再接受任何回報（也不會被偷偷翻成成功）。
    let (status, _) = server
        .post(
            &format!("/v1/agent-sessions/{id}/report"),
            json!({"event": "claimed-completed", "payload": {}}),
        )
        .await;
    assert_eq!(status, 409, "a terminal unknown cannot be re-opened");
}

/// spec §9.3：「測試裝置」是人類動作。agent／session token 一律 403，且
/// 沒測過的 provider 不得在 API 回應裡冒充「已測試」。
#[tokio::test]
async fn provider_test_is_human_only_and_never_fakes_evidence() {
    let server = TestServer::spawn().await;

    // 沒測過就是沒有證據：detail 裡不得出現 tested。
    let (status, providers) = server.get("/v1/providers").await;
    assert_eq!(status, 200);
    let builtin = providers
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["identity"]["id"] == "provider.local.builtin")
        .expect("builtin provider listed")
        .clone();
    assert!(
        !builtin["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("tested"),
        "untested provider must not claim evidence: {builtin}"
    );

    // agent token 不得測試裝置（人類控制面）。
    let denied = server
        .client
        .post(format!(
            "{}/v1/providers/provider.local.builtin/test",
            server.base
        ))
        .bearer_auth(&server.agent_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);
    let error: Value = denied.json().await.unwrap();
    assert_eq!(error["error"]["code"], "token_scope_forbidden");

    // 人類 token：唯讀測一次，結果誠實寫進證據。
    let (status, report) = server
        .post("/v1/providers/provider.local.builtin/test", json!({}))
        .await;
    assert_eq!(status, 200);
    assert_eq!(report["tested"]["how"], "human");
    assert_eq!(report["tested"]["ok"], report["ok"]);
    let (_, providers) = server.get("/v1/providers").await;
    let builtin = providers
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["identity"]["id"] == "provider.local.builtin")
        .unwrap()
        .clone();
    let detail: Value =
        serde_json::from_str(builtin["detail"].as_str().expect("detail json")).unwrap();
    assert_eq!(detail["tested"]["how"], "human");
    assert_eq!(detail["tested"]["ok"], report["ok"]);

    // 不存在的 provider：404，不是假裝測過。
    let (status, _) = server
        .post("/v1/providers/provider.nope/test", json!({}))
        .await;
    assert_eq!(status, 404);
}

// ---------------------------------------------------------------------------
// Character Presentation Protocol：外部 adapter WebSocket fixture（模擬 adapter，
// 程序內 tokio-tungstenite client；不是真外部程式）＋ adapter token 分權。
// ---------------------------------------------------------------------------

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const FIXTURE_MANIFEST: &str =
    include_str!("../../../examples/character-adapters/text-adapter.manifest.json");

async fn ws_connect(
    base: &str,
    token: &str,
) -> Result<WsStream, tokio_tungstenite::tungstenite::Error> {
    let url = format!(
        "{}/v1/character/ws?token={token}",
        base.replacen("http", "ws", 1)
    );
    tokio_tungstenite::connect_async(url)
        .await
        .map(|(ws, _)| ws)
}

fn ws_http_status(err: tokio_tungstenite::tungstenite::Error) -> u16 {
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status().as_u16(),
        other => panic!("expected an HTTP refusal, got {other}"),
    }
}

/// 下一則 JSON 訊息（跳過 ping/pong；5 s 內沒有就 panic）。
async fn ws_next(ws: &mut WsStream) -> Value {
    use futures::StreamExt;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let item = tokio::time::timeout_at(deadline, ws.next())
            .await
            .expect("websocket message within 5s")
            .expect("stream open")
            .expect("frame ok");
        match item {
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                return serde_json::from_str(&text).expect("wire json");
            }
            tokio_tungstenite::tungstenite::Message::Close(_) => panic!("socket closed early"),
            _ => continue,
        }
    }
}

/// 等到某種 type 的訊息（heartbeat 等其他訊息略過）。
async fn ws_wait_for(ws: &mut WsStream, kind: &str) -> Value {
    for _ in 0..20 {
        let msg = ws_next(ws).await;
        if msg["type"] == kind {
            return msg;
        }
    }
    panic!("no {kind} message arrived");
}

async fn ws_send(ws: &mut WsStream, message: Value) {
    use futures::SinkExt;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        message.to_string(),
    ))
    .await
    .expect("send");
}

fn fixture_negotiate(generation: u64) -> Value {
    let manifest: Value = serde_json::from_str(FIXTURE_MANIFEST).unwrap();
    json!({
        "type": "negotiate",
        "protocolVersion": "1.0",
        "characterId": manifest["characterId"],
        "manifestVersion": manifest["version"],
        "capabilities": manifest["capabilities"],
        "inputCapabilities": manifest["inputCapabilities"],
        "channels": manifest["channels"],
        "intents": manifest["intents"],
        "variants": [],
        "generation": generation,
    })
}

#[tokio::test]
async fn character_ws_fixture_negotiates_receives_intents_and_answers_receipts() {
    let server = TestServer::spawn().await;
    let manifest: Value = serde_json::from_str(FIXTURE_MANIFEST).unwrap();
    let (status, added) = server
        .post(
            "/v1/character/adapters",
            json!({"displayName": "文字 adapter（fixture）", "manifest": manifest}),
        )
        .await;
    assert_eq!(status, 200, "{added}");
    let adapter_id = added["adapterId"].as_str().unwrap().to_string();
    let token = added["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);
    let instance_id = format!("adapter:{adapter_id}");

    // human／agent／未知 token 一律拒絕上 WebSocket（401）。
    assert_eq!(
        ws_http_status(ws_connect(&server.base, &server.token).await.unwrap_err()),
        401
    );
    assert_eq!(
        ws_http_status(
            ws_connect(&server.base, &server.agent_token)
                .await
                .unwrap_err()
        ),
        401
    );
    assert_eq!(
        ws_http_status(ws_connect(&server.base, "not-a-token").await.unwrap_err()),
        401
    );
    assert_eq!(
        ws_http_status(ws_connect(&server.base, "").await.unwrap_err()),
        401
    );

    // adapter token 打人類路由：全部 403（status／estop／agent sessions／角色清單）。
    let adapter = |method: reqwest::Method, path: &str, body: Value| {
        server
            .client
            .request(method, format!("{}{path}", server.base))
            .bearer_auth(&token)
            .json(&body)
            .send()
    };
    for (method, path) in [
        (reqwest::Method::GET, "/v1/status"),
        (reqwest::Method::POST, "/v1/emergency-stop"),
        (reqwest::Method::POST, "/v1/emergency-stop/clear"),
        (reqwest::Method::POST, "/v1/agent-sessions"),
        (reqwest::Method::GET, "/v1/agent-sessions"),
        (reqwest::Method::POST, "/v1/agent-sessions/x/verify"),
        (reqwest::Method::POST, "/v1/agent-sessions/x/interrupt"),
        (reqwest::Method::GET, "/v1/character/instances"),
        (reqwest::Method::POST, "/v1/character/hello"),
        (reqwest::Method::POST, "/v1/character/adapters"),
        (reqwest::Method::POST, "/v1/character/intent"),
        (reqwest::Method::POST, "/v1/plans"),
        (reqwest::Method::PATCH, "/v1/policy"),
        (reqwest::Method::POST, "/v1/session/consent"),
        (reqwest::Method::GET, "/v1/events"),
    ] {
        let response = adapter(method.clone(), path, json!({})).await.unwrap();
        assert_eq!(
            response.status(),
            403,
            "adapter token must not reach {method} {path}"
        );
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["code"], "token_scope_forbidden");
    }
    // 別人的 instance 也不行（403）；自己的 instance 可以（回執未知 messageId → accepted:false）。
    let response = adapter(
        reqwest::Method::POST,
        "/v1/character/receipts",
        json!({"instanceId": "desktop-companion", "receipt": {
            "messageId": "m", "characterInstanceId": "desktop-companion", "generation": 1,
            "status": "accepted", "at": chrono::Utc::now()}}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 403);

    // 連線：第一則一定是 hello。
    let mut ws = ws_connect(&server.base, &token).await.expect("adapter ws");
    let hello = ws_next(&mut ws).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["protocolVersion"], "1.0");
    assert_eq!(hello["characterInstanceId"], instance_id);
    assert_eq!(hello["limits"]["maxMessageBytes"], 65536);
    assert_eq!(hello["limits"]["maxMessagesPerSecond"], 50);

    // negotiate → negotiated。
    ws_send(&mut ws, fixture_negotiate(1)).await;
    let negotiated = ws_wait_for(&mut ws, "negotiated").await;
    assert_eq!(negotiated["characterInstanceId"], instance_id);
    assert_eq!(negotiated["generation"], 1);
    assert_eq!(
        negotiated["resolutions"]["emergency"]["resolution"],
        "exact"
    );
    assert_eq!(
        negotiated["resolutions"]["play"]["resolution"],
        "substituted"
    );
    let (status, instances) = server.get("/v1/character/instances").await;
    assert_eq!(status, 200);
    let entry = instances["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["instanceId"] == instance_id)
        .cloned()
        .unwrap();
    assert_eq!(entry["connected"], true);
    assert_eq!(entry["negotiated"], true);
    assert_eq!(entry["origin"], "external");
    assert_eq!(entry["executable"], true);
    assert_eq!(entry["tested"], false);
    // 連接頁「可以接收／作者／版本」由這條路由直接提供（README §9）。
    assert!(entry["version"].is_string());
    assert!(
        entry.get("author").is_some(),
        "author key present (null when absent)"
    );
    assert!(entry["inputCapabilities"].is_array());
    let (_, status_doc) = server.get("/v1/status").await;
    assert_eq!(status_doc["characterProtocol"]["version"], "1.0");
    assert_eq!(status_doc["characterProtocol"]["instances"], 1);
    assert!(status_doc["characterProtocol"]["activeCharacter"].is_null());

    // 自己 instance 的回執經 HTTP（adapter token）也收：未知 messageId → accepted:false。
    let response = adapter(
        reqwest::Method::POST,
        "/v1/character/receipts",
        json!({"instanceId": instance_id, "receipt": {
            "messageId": "nope", "characterInstanceId": instance_id, "generation": 1,
            "status": "accepted", "at": chrono::Utc::now()}}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["accepted"], false);

    // human estop → intent emergency 經 WebSocket 送到 adapter。
    let (status, _) = server
        .post("/v1/emergency-stop", json!({"reason": "drill"}))
        .await;
    assert_eq!(status, 200);
    let intent = ws_wait_for(&mut ws, "intent").await;
    let envelope = &intent["envelope"];
    assert_eq!(envelope["intent"], "emergency");
    assert_eq!(envelope["truthState"], "emergency");
    assert_eq!(envelope["priority"], 100);
    assert_eq!(envelope["characterInstanceId"], instance_id);
    let message_id = envelope["messageId"].as_str().unwrap().to_string();

    // 回執 accepted → started → completed（只代表文字印出）。
    for status in ["accepted", "started", "completed"] {
        ws_send(
            &mut ws,
            json!({"type": "receipt", "receipt": {
                "messageId": message_id, "characterInstanceId": instance_id, "generation": 1,
                "status": status, "resolution": "exact", "at": chrono::Utc::now()}}),
        )
        .await;
    }
    let mut seen_completed = false;
    for _ in 0..40 {
        seen_completed = server.runtime.events.recent(300).iter().any(|e| {
            e.event_type == interaction_core::EventType::CharacterReceipt
                && e.payload["receipt"]["messageId"] == message_id.as_str()
                && e.payload["receipt"]["status"] == "completed"
        });
        if seen_completed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        seen_completed,
        "character.receipt completed event published"
    );
    let (_, instances) = server.get("/v1/character/instances").await;
    let entry = instances["instances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["instanceId"] == instance_id)
        .cloned()
        .unwrap();
    assert_eq!(entry["tested"], true);
    // 一個呈現回執永遠改不了任何工作 verification：估算 estop 期間沒有 verified 的 action。
    assert!(server
        .runtime
        .list_actions(None, 50)
        .unwrap()
        .iter()
        .all(|r| r
            .verification
            .as_ref()
            .map(|v| v.verdict != interaction_core::VerificationVerdict::Observed)
            .unwrap_or(true)));

    // 超過 64 KB → error{too-large}，連線不斷。
    let big = format!(
        "{{\"type\":\"heartbeat\",\"pad\":\"{}\"}}",
        "x".repeat(70_000)
    );
    ws_send(&mut ws, Value::String(String::new())).await; // 空字串是無效 wire → error{malformed}
    let malformed = ws_wait_for(&mut ws, "error").await;
    assert_eq!(malformed["code"], "malformed");
    {
        use futures::SinkExt;
        ws.send(tokio_tungstenite::tungstenite::Message::Text(big))
            .await
            .unwrap();
    }
    let too_large = ws_wait_for(&mut ws, "error").await;
    assert_eq!(too_large["code"], "too-large");
    assert!(!too_large["message"].as_str().unwrap().contains("xxxx"));

    // rate limit：一秒內 > 50 則 → error{rate-limited}（多出的被丟棄）。
    for _ in 0..70 {
        ws_send(&mut ws, json!({"type": "heartbeat"})).await;
    }
    let mut rate_limited = false;
    for _ in 0..80 {
        let msg = ws_next(&mut ws).await;
        if msg["type"] == "error" && msg["code"] == "rate-limited" {
            rate_limited = true;
            break;
        }
    }
    assert!(rate_limited, "gateway must answer error{{rate-limited}}");

    // 撤銷 → goodbye 後 socket 關閉；adapters 清單 revoked、instances 不再列出。
    let (status, revoked) = server
        .delete(&format!("/v1/character/adapters/{adapter_id}"))
        .await;
    assert_eq!(status, 200);
    assert_eq!(revoked["revoked"], true);
    assert_eq!(revoked["disconnected"], true);
    let mut saw_goodbye = false;
    {
        use futures::StreamExt;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, ws.next()).await {
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                    let v: Value = serde_json::from_str(&text).unwrap_or_default();
                    if v["type"] == "goodbye" {
                        saw_goodbye = true;
                    }
                }
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))))
                | Ok(None)
                | Ok(Some(Err(_)))
                | Err(_) => break,
                Ok(Some(Ok(_))) => {}
            }
        }
    }
    assert!(
        saw_goodbye,
        "revoked adapter is told goodbye before the close"
    );
    let (_, adapters) = server.get("/v1/character/adapters").await;
    assert_eq!(adapters["adapters"][0]["revoked"], true);
    assert_eq!(adapters["adapters"][0]["connected"], false);
    assert!(adapters["adapters"][0].get("token").is_none());
    // 撤銷後仍能顯示 manifest 事實（作者／版本／可接收／可執行／需要網路）。
    assert!(adapters["adapters"][0]["version"].is_string());
    assert!(adapters["adapters"][0]["inputCapabilities"].is_array());
    assert!(adapters["adapters"][0]["executable"].is_boolean());
    assert!(adapters["adapters"][0]["network"].is_boolean());
    assert!(adapters["adapters"][0]["characterDisplayName"].is_object());
    let (_, instances) = server.get("/v1/character/instances").await;
    assert!(instances["instances"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["instanceId"] != instance_id));
    assert_eq!(
        ws_http_status(ws_connect(&server.base, &token).await.unwrap_err()),
        401,
        "revoked token can no longer connect"
    );
    // agent token 看不到角色層。
    let response = server
        .client
        .get(format!("{}/v1/character/adapters", server.base))
        .bearer_auth(&server.agent_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
}

#[tokio::test]
async fn character_hello_route_negotiates_and_manual_intent_refuses_safety() {
    let server = TestServer::spawn().await;
    let (status, _) = server.get("/v1/character/manifest").await;
    assert_eq!(status, 404, "no desktop character before hello");
    let manifest = json!({
        "schemaVersion": "1.0",
        "characterId": "plain-text",
        "displayName": { "zh-TW": "純文字角色", "en": "Plain text" },
        "version": "1.0.0",
        "adapterKind": "in-process",
        "entrypoint": { "kind": "builtin", "id": "text" },
        "assets": [],
        "capabilities": {
            "visual.presence": { "supported": true },
            "visual.textBubble": { "supported": true }
        },
        "inputCapabilities": { "input.click": { "supported": true }, "input.text": { "supported": true } },
        "channels": ["bubble"],
        "states": ["idle", "line"],
        "intents": ["idle", "notice", "acknowledge", "think", "work", "wait", "ask", "request-consent",
                    "blocked", "unknown", "claim-completed", "verified-success", "failed", "cancelled",
                    "offline", "emergency", "greet", "play", "rest", "sleep"],
        "variants": [],
        "locales": ["zh-TW", "en"],
        "compatibility": { "protocol": "1.x" }
    });
    let negotiate = json!({
        "protocolVersion": "1.0",
        "characterId": "plain-text",
        "manifestVersion": "1.0.0",
        "capabilities": manifest["capabilities"],
        "inputCapabilities": manifest["inputCapabilities"],
        "channels": manifest["channels"],
        "intents": manifest["intents"],
        "variants": [],
        "generation": 1,
    });
    let (status, out) = server
        .post(
            "/v1/character/hello",
            json!({"manifest": manifest, "negotiate": negotiate, "visible": true}),
        )
        .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["instanceId"], "desktop-companion");
    assert_eq!(out["generation"], 1);
    assert_eq!(
        out["negotiated"]["resolutions"]["notice"]["via"],
        "visual.textBubble"
    );
    let (status, got) = server.get("/v1/character/manifest").await;
    assert_eq!(status, 200);
    assert_eq!(got["characterId"], "plain-text");
    let (_, status_doc) = server.get("/v1/status").await;
    assert_eq!(
        status_doc["characterProtocol"]["activeCharacter"]["displayName"]["zh-TW"],
        "純文字角色"
    );
    let (_, providers) = server.get("/v1/providers").await;
    let companion = providers
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["identity"]["id"] == "provider.companion.desktop")
        .cloned()
        .unwrap();
    assert_eq!(
        companion["identity"]["displayName"],
        "桌面角色：純文字角色（Presentation）"
    );

    // 手動 intent：非安全可以（targets = 桌面 instance），安全一律 403。
    let (status, out) = server
        .post(
            "/v1/character/intent",
            json!({"intent": "notice", "message": "測試"}),
        )
        .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["targets"], json!(["desktop-companion"]));
    assert_eq!(out["truthState"], "none");
    let (status, out) = server
        .post("/v1/character/intent", json!({"intent": "emergency"}))
        .await;
    assert_eq!(status, 403, "{out}");
    let (status, _) = server
        .post(
            "/v1/character/intent",
            json!({"intent": "verified-success"}),
        )
        .await;
    assert_eq!(status, 403);
    // 回執（human token）推進：accepted → started → completed 對應 character.intent 的 messageId。
    let message_id = server
        .runtime
        .events
        .recent(100)
        .iter()
        .rev()
        .find(|e| e.event_type == interaction_core::EventType::CharacterIntent)
        .map(|e| {
            e.payload["envelope"]["messageId"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .unwrap();
    let (status, out) = server
        .post(
            "/v1/character/receipts",
            json!({"instanceId": "desktop-companion", "receipt": {
                "messageId": message_id, "characterInstanceId": "desktop-companion",
                "generation": 1, "status": "accepted", "at": chrono::Utc::now()}}),
        )
        .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["accepted"], true);
    assert_eq!(out["status"], "accepted");
    // 事件（human token）→ receptor observation。
    let (status, out) = server
        .post(
            "/v1/character/events",
            json!({"instanceId": "desktop-companion", "event": {
                "protocolVersion": "1.0", "eventId": "evt-1", "characterInstanceId": "desktop-companion",
                "generation": 1, "timestamp": chrono::Utc::now(), "kind": "character.clicked",
                "payload": {"x": 3, "y": 4}, "privacyClass": "internal"}}),
        )
        .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["decision"], "queued");
    let (_, obs) = server
        .post(
            "/v1/observations/query",
            json!({"receptorId": "companion.click", "limit": 5}),
        )
        .await;
    assert_eq!(obs[0]["facts"]["kind"], "companion-clicked");
}

/// §8 的 50 則/s 對每個入口一致：adapter token 走 HTTP 也要被同一個計數器擋下，
/// 不能靠改用 `POST /v1/character/{receipts,events}` 繞過 WebSocket 的限制。
///
/// 決定論（9.9）：限流是純 token bucket，時間由呼叫端注入，所以這裡注入假時鐘
/// 後可以精確斷言「50 則通過、第 51 則 rate-limited、推進 20 ms 恰好再放行 1
/// 則」，而不是量真實時鐘再放寬邊界。三個入口（HTTP receipts／HTTP events／
/// WebSocket）共用同一份預算也在同一條測試裡逐一驗證。
#[tokio::test]
async fn adapter_token_http_routes_share_the_websocket_rate_limit() {
    let server = TestServer::spawn().await;
    let manifest: Value = serde_json::from_str(FIXTURE_MANIFEST).unwrap();
    let (status, added) = server
        .post(
            "/v1/character/adapters",
            json!({"displayName": "文字 adapter（速率）", "manifest": manifest}),
        )
        .await;
    assert_eq!(status, 200, "{added}");
    let adapter_id = added["adapterId"].as_str().unwrap().to_string();
    let token = added["token"].as_str().unwrap().to_string();
    let instance_id = format!("adapter:{adapter_id}");
    // instance 要先存在（adapter 連上 WebSocket 才會註冊）。
    let mut ws = ws_connect(&server.base, &token).await.expect("adapter ws");
    assert_eq!(ws_next(&mut ws).await["type"], "hello");

    // 決定論：注入假時鐘。限流是純 token bucket（`interaction-character`
    // `wire.rs::RateLimiter`：capacity = 50、refill = 50/1000 = 0.05 格/ms），
    // 完全由呼叫端注入的 `now` 驅動——HTTP receipts／events 與 WebSocket 三個
    // 入口都經過同一個 `CharacterHub::now()`。改用假時鐘後，這個測試不再量真實
    // 時鐘、不受機器負載影響（限流演算法本身一行未改）。
    let base = chrono::Utc::now();
    let clock_at = |ms: i64| {
        let at = base + chrono::Duration::milliseconds(ms);
        std::sync::Arc::new(move || at) as interaction_runtime::character::NowFn
    };
    server.runtime.character.set_clock(clock_at(0));

    let post_as_adapter = |path: &'static str, body: Value| {
        server
            .client
            .post(format!("{}{path}", server.base))
            .bearer_auth(&token)
            .json(&body)
            .send()
    };

    // 回執本身的 messageId 不存在（沒有對應的 intent），所以「通過限流」的正常
    // 回應就是 accepted:false / status:"unknown-message"——與 "rate-limited"
    // 明確可分。
    let post_receipt = |i: usize| {
        let body = json!({"instanceId": instance_id, "receipt": {
            "messageId": format!("m{i}"), "characterInstanceId": instance_id,
            "generation": 0, "status": "accepted", "at": chrono::Utc::now()}});
        async move {
            let response = post_as_adapter("/v1/character/receipts", body)
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            response.json::<Value>().await.unwrap()
        }
    };

    // 第 1..=50 則：預算滿的那一瞬間全部通過。
    for i in 0..50 {
        let body = post_receipt(i).await;
        assert_eq!(body["status"], "unknown-message", "第 {i} 則應通過：{body}");
    }
    // 第 51 則：同一瞬間預算用盡 → 誠實回 rate-limited。
    let body = post_receipt(50).await;
    assert_eq!(body["accepted"], false, "{body}");
    assert_eq!(body["status"], "rate-limited", "{body}");

    // events 共用同一份預算：此時已超量 → 誠實回 dropped{rate-limited}。
    let response = post_as_adapter(
        "/v1/character/events",
        json!({"instanceId": instance_id, "event": {
            "protocolVersion": "1.0", "eventId": "evt-rate", "characterInstanceId": instance_id,
            "generation": 0, "timestamp": chrono::Utc::now(), "kind": "character.text-submitted",
            "payload": {"text": "spam"}, "privacyClass": "internal"}}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["decision"], "dropped", "{body}");
    assert_eq!(body["reason"], "rate-limited", "{body}");

    // WebSocket 也吃同一份預算（不是各自一份）：HTTP 把預算耗盡後，WS 上的
    // adapter→runtime 訊息立刻收到 error{code:"rate-limited"}。
    ws_send(&mut ws, fixture_negotiate(1)).await;
    let error = ws_wait_for(&mut ws, "error").await;
    assert_eq!(error["code"], "rate-limited", "{error}");

    // 50 則/s ⇒ 每 20 ms 回補剛好 1 格：推進 20 ms 後恰好再接受 1 則。
    server.runtime.character.set_clock(clock_at(20));
    let body = post_receipt(51).await;
    assert_eq!(body["status"], "unknown-message", "{body}");
    let body = post_receipt(52).await;
    assert_eq!(body["status"], "rate-limited", "{body}");

    // 再推進 20 ms，那唯一一格由 WS 取用 → negotiate 這次被接受。
    server.runtime.character.set_clock(clock_at(40));
    ws_send(&mut ws, fixture_negotiate(1)).await;
    let negotiated = ws_wait_for(&mut ws, "negotiated").await;
    assert_eq!(negotiated["generation"], 1, "{negotiated}");
    // WS 用掉了那一格，HTTP 這邊立刻又是空的。
    let body = post_receipt(53).await;
    assert_eq!(body["status"], "rate-limited", "{body}");
}

/// 裝置的身分指紋屬於人類層的配對資訊：`/v1/mobile` 整條路由 agent token 都
/// 讀不到，就不能從 `/v1/providers` 繞過去讀（人類 token 仍然看得到）。
#[tokio::test(flavor = "multi_thread")]
async fn agent_token_never_sees_a_device_identity_fingerprint() {
    use interaction_core::{
        ProviderDescriptor, ProviderId, ProviderIdentity, ProviderKind, ProviderState, TrustLevel,
    };

    let server = TestServer::spawn().await;
    let provider_id = "provider.mobile.iphone-test01";
    let fingerprint = "a".repeat(64);
    server
        .runtime
        .providers
        .register(ProviderDescriptor {
            identity: ProviderIdentity {
                id: ProviderId::new(provider_id),
                kind: ProviderKind::Device,
                display_name: "iPhone：測試".into(),
                trust_level: TrustLevel::Paired,
                origin: "mobile-wss".into(),
                version: String::new(),
                fingerprint: Some(fingerprint.clone()),
                human: None,
            },
            state: ProviderState::Available,
            receptors: vec![],
            actuators: vec![],
            tool_operations: vec![],
            paired_at: None,
            last_seen: None,
            detail: None,
        })
        .await
        .unwrap();

    // 人類 token：看得到（這是人類層的配對資訊）。
    let (status, list) = server.get("/v1/providers").await;
    assert_eq!(status, 200);
    let mine = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["identity"]["id"] == json!(provider_id))
        .expect("registered provider");
    assert_eq!(mine["identity"]["fingerprint"], json!(fingerprint));
    let (status, one) = server.get(&format!("/v1/providers/{provider_id}")).await;
    assert_eq!(status, 200);
    assert_eq!(one["identity"]["fingerprint"], json!(fingerprint));

    // agent token：`/v1/mobile` 直接 403，`/v1/providers` 也不得洩漏同一個值。
    let denied = server
        .client
        .get(format!("{}/v1/mobile/status", server.base))
        .bearer_auth(&server.agent_token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403, "agent token 不得讀 /v1/mobile");

    for path in ["/v1/providers", &format!("/v1/providers/{provider_id}")] {
        let response = server
            .client
            .get(format!("{}{path}", server.base))
            .bearer_auth(&server.agent_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        let body: Value = response.json().await.unwrap();
        let text = serde_json::to_string(&body).unwrap();
        assert!(
            !text.contains(&fingerprint),
            "{path} 不得對 agent token 透出裝置身分指紋：{text}"
        );
        let descriptor = match &body {
            Value::Array(items) => items
                .iter()
                .find(|p| p["identity"]["id"] == json!(provider_id))
                .cloned()
                .expect("provider in list"),
            other => other.clone(),
        };
        assert_eq!(
            descriptor["identity"]["fingerprint"],
            Value::Null,
            "{path}: {descriptor}"
        );
        // 其餘欄位照常可讀（這不是把整條路由關掉）。
        assert_eq!(descriptor["identity"]["id"], json!(provider_id));
        assert_eq!(descriptor["state"], json!("available"));
    }
}

/// safety-invariants-078：`POST /v1/agent-sessions/{id}/interrupt` 指名單一
/// session，所以必須有擁有權。三條路徑的邊界：
///   * session-scoped capability token → 只能中斷自己的 session；
///   * legacy 共享 agent token → 沒有 session 身分（建不了也列不到 session），
///     一律 403 `token_scope_forbidden`（v0.5.1 起的刻意收斂）；
///   * human token → 保留管理能力，可中斷任何 session。
#[tokio::test]
async fn interrupt_requires_session_ownership_or_a_human_token() {
    let server = TestServer::spawn().await;
    let make = |agent: &'static str| interaction_runtime::agents::CreateAgentSession {
        provider_id: Some(format!("provider.ai-agent.{agent}")),
        agent_id: agent.into(),
        label: Some("interrupt ownership probe".into()),
        ttl_minutes: Some(5),
        data_scope: vec![],
        tool_scope: vec!["status".into()],
        consent_scope: vec![],
        allow_write: false,
        max_cost: None,
        max_messages: Some(5),
        delegation: None,
        workdir: None,
        resume_provider_session_id: None,
    };
    let a = server
        .runtime
        .create_agent_session(make("owner-a"))
        .await
        .unwrap();
    let b = server
        .runtime
        .create_agent_session(make("owner-b"))
        .await
        .unwrap();
    let a_id = a.session_id.as_str().to_string();
    let b_id = b.session_id.as_str().to_string();
    let a_token = server
        .runtime
        .issue_agent_session_capability(&a_id)
        .await
        .unwrap();

    let interrupt = |token: String, id: String| {
        server
            .client
            .post(format!("{}/v1/agent-sessions/{id}/interrupt", server.base))
            .bearer_auth(token)
            .json(&json!({}))
            .send()
    };

    // 1) 跨 session：A 的 token 打 B 的 id → 403 token_scope_forbidden。
    let response = interrupt(a_token.clone(), b_id.clone()).await.unwrap();
    assert_eq!(
        response.status(),
        403,
        "session-scoped token must not interrupt another session"
    );
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "token_scope_forbidden");

    // 2) legacy 共享 agent token：任何 session 都不行（含存在的 session id）。
    for id in [a_id.clone(), b_id.clone(), "no-such-session".to_string()] {
        let response = interrupt(server.agent_token.clone(), id.clone())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            403,
            "legacy agent token must not interrupt {id}"
        );
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["code"], "token_scope_forbidden");
    }
    // 前提：legacy token 也建不了 session，所以「自己的 session」不存在。
    let response = server
        .client
        .post(format!("{}/v1/agent-sessions", server.base))
        .bearer_auth(&server.agent_token)
        .json(&json!({"agentId": "owner-c"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let response = server
        .client
        .get(format!("{}/v1/agent-sessions", server.base))
        .bearer_auth(&server.agent_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);

    // 3) 自己的 session：授權通過，請求真的抵達 runtime。這兩個 session 沒有
    //    受管子程序（沒有真的 spawn codex／claude-code），所以 runtime 誠實
    //    回 404 not_found；關鍵是它**不是** 403 scope 錯誤。
    let response = interrupt(a_token.clone(), a_id.clone()).await.unwrap();
    assert_eq!(
        response.status(),
        404,
        "own-session interrupt must pass authorization and reach the runtime"
    );
    let error: Value = response.json().await.unwrap();
    assert_ne!(error["error"]["code"], "token_scope_forbidden");
    assert!(error["error"]["message"].as_str().unwrap().contains(&a_id));

    // 4) human token 保留管理能力：任何 session 都不被 scope 擋。
    for id in [a_id.clone(), b_id.clone()] {
        let (status, body) = server
            .post(&format!("/v1/agent-sessions/{id}/interrupt"), json!({}))
            .await;
        assert_eq!(status, 404, "{body}");
        assert_ne!(body["error"]["code"], "token_scope_forbidden");
    }

    // 5) 緊急停止不走 `/interrupt`：runtime 內部路徑仍終止所有 open session。
    assert_eq!(server.runtime.open_agent_sessions().await, 2);
    let (status, stopped) = server
        .post("/v1/emergency-stop", json!({"reason": "ownership test"}))
        .await;
    assert_eq!(status, 200, "{stopped}");
    for id in [a_id.clone(), b_id.clone()] {
        let record = server.runtime.get_agent_session(&id).await.unwrap();
        assert_eq!(
            record.state,
            interaction_core::AgentSessionState::Cancelled,
            "estop must cancel {id}"
        );
    }
    assert_eq!(server.runtime.open_agent_sessions().await, 0);
}

/// agent-honesty-021：非 gateway（輪詢型）agent session 必須真的能用自己的
/// session token 取走信箱裡的任務——否則 `dispatched → fetched → acknowledged`
/// 這條誠實階梯對整類 session 都是死碼，介面卻一直說「它來取走之後才會開始」。
///
/// 同時鎖住兩條界線：human token 的 GET 仍然是純觀看（不蓋送達戳記），
/// 跨 session 的 capability token 一律 403。
#[tokio::test]
async fn an_agent_session_can_fetch_its_own_mailbox_and_that_is_what_marks_delivery() {
    let server = TestServer::spawn().await;
    let make_session = |label: &'static str| {
        let runtime = server.runtime.clone();
        async move {
            runtime
                .create_agent_session(interaction_runtime::agents::CreateAgentSession {
                    provider_id: Some("provider.ai-agent.external-test".into()),
                    agent_id: "agent.coder".into(),
                    label: Some(label.into()),
                    ttl_minutes: Some(5),
                    data_scope: vec![],
                    tool_scope: vec![],
                    consent_scope: vec![],
                    allow_write: false,
                    max_cost: None,
                    max_messages: Some(5),
                    delegation: None,
                    workdir: None,
                    resume_provider_session_id: None,
                })
                .await
                .unwrap()
        }
    };
    let id = make_session("polling agent")
        .await
        .session_id
        .as_str()
        .to_string();
    let other_id = make_session("someone else")
        .await
        .session_id
        .as_str()
        .to_string();
    let session_token = server
        .runtime
        .issue_agent_session_capability(&id)
        .await
        .unwrap();
    let other_token = server
        .runtime
        .issue_agent_session_capability(&other_id)
        .await
        .unwrap();

    // 人類派一則任務進信箱。
    let (status, sent) = server
        .post(
            &format!("/v1/agent-sessions/{id}/messages"),
            json!({"kind": "task", "body": {"summary": "去把燈打開"}}),
        )
        .await;
    assert_eq!(status, 200, "{sent}");
    assert!(
        sent["deliveredAt"].is_null(),
        "還沒有人來取，不得先蓋送達戳記：{sent}"
    );

    // 人類看一眼信箱：純觀看，不得把「看過」偽裝成「agent 收到了」。
    let (status, peeked) = server
        .get(&format!("/v1/agent-sessions/{id}/messages"))
        .await;
    assert_eq!(status, 200, "{peeked}");
    assert!(
        peeked[0]["deliveredAt"].is_null(),
        "human token 的 GET 是純觀看：{peeked}"
    );

    // 別人的 session token 不得讀這個信箱。
    let response = server
        .client
        .get(format!("{}/v1/agent-sessions/{id}/messages", server.base))
        .bearer_auth(&other_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        403,
        "跨 session 的 capability token 不得讀別人的信箱"
    );

    // legacy agent token 沒有 session 身分，證明不了擁有權，一律拒絕。
    let response = server
        .client
        .get(format!("{}/v1/agent-sessions/{id}/messages", server.base))
        .bearer_auth(&server.agent_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        403,
        "legacy agent token 不帶 session 身分，不得取走任何 session 的信箱"
    );

    // 這個 session 自己的 token：真的取得到，而且取走＝送達。
    let response = server
        .client
        .get(format!("{}/v1/agent-sessions/{id}/messages", server.base))
        .bearer_auth(&session_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        200,
        "agent 必須能用自己的 session token 取走自己的任務"
    );
    let fetched: Value = response.json().await.unwrap();
    assert_eq!(fetched.as_array().map(|a| a.len()), Some(1), "{fetched}");
    assert!(
        fetched[0]["deliveredAt"].is_string(),
        "agent 身分的取走必須蓋上送達戳記：{fetched}"
    );

    // 送達之後，狀態序列裡要看得到 `fetched`（taxonomy §7.4）。
    let fetched_event = server.runtime.events.recent(300).into_iter().any(|e| {
        e.event_type == interaction_core::EventType::AgentSessionState
            && e.payload["agentSessionId"] == json!(id)
            && e.payload["state"] == json!("fetched")
    });
    assert!(
        fetched_event,
        "任務真的被 agent 取走必須發出 `fetched`（沒有它，介面會永遠停在「準備中」）"
    );

    // 再讀一次不得重複蓋章／重複發 fetched（送達只發生一次）。
    let response = server
        .client
        .get(format!("{}/v1/agent-sessions/{id}/messages", server.base))
        .bearer_auth(&session_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let again: Value = response.json().await.unwrap();
    assert_eq!(
        again[0]["deliveredAt"], fetched[0]["deliveredAt"],
        "{again}"
    );
    let fetched_events = server
        .runtime
        .events
        .recent(300)
        .into_iter()
        .filter(|e| {
            e.event_type == interaction_core::EventType::AgentSessionState
                && e.payload["agentSessionId"] == json!(id)
                && e.payload["state"] == json!("fetched")
        })
        .count();
    assert_eq!(fetched_events, 1, "送達只發生一次，fetched 不得重複發");
}

/// safety-invariants-057：agent／session token 也打得到 estop、stop-all 與
/// cancel。audit 必須寫實際的 principal（比照 sensors/stop），否則事後分不出
/// 是人按的還是 AI 觸發的。
#[tokio::test]
async fn stop_operations_record_the_real_principal_in_the_audit_trail() {
    let server = TestServer::spawn().await;

    // 1) agent token 觸發的緊急停止。
    let response = server
        .client
        .post(format!("{}/v1/emergency-stop", server.base))
        .bearer_auth(&server.agent_token)
        .json(&json!({"reason": "agent drill"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let (status, audit) = server.get("/v1/audit?limit=200").await;
    assert_eq!(status, 200, "{audit}");
    let entry = audit
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == json!("emergency.stop"))
        .cloned()
        .expect("緊急停止要留下 audit");
    assert_eq!(
        entry["actor"], "agent",
        "agent token 觸發的緊急停止不得記成人類的 \"api\"：{entry}"
    );

    // 人類解除，才能繼續下一段。
    let (status, _) = server.post("/v1/emergency-stop/clear", json!({})).await;
    assert_eq!(status, 200);

    // 2) agent token 觸發的 stop-all（逐一取消 → action.cancelled 的 actor）。
    let (status, session) = server
        .post("/v1/session/start", json!({"label": "stop-all"}))
        .await;
    assert_eq!(status, 200, "{session}");
    let response = server
        .client
        .post(format!("{}/v1/stop-all", server.base))
        .bearer_auth(&server.agent_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // 3) session-scoped capability token 觸發的緊急停止：actor 要指名哪個
    //    agent、哪個 session。
    let record = server
        .runtime
        .create_agent_session(interaction_runtime::agents::CreateAgentSession {
            provider_id: Some("provider.ai-agent.external-test".into()),
            agent_id: "agent.coder".into(),
            label: Some("stop actor".into()),
            ttl_minutes: Some(5),
            data_scope: vec![],
            tool_scope: vec![],
            consent_scope: vec![],
            allow_write: false,
            max_cost: None,
            max_messages: Some(5),
            delegation: None,
            workdir: None,
            resume_provider_session_id: None,
        })
        .await
        .unwrap();
    let sid = record.session_id.as_str().to_string();
    let session_token = server
        .runtime
        .issue_agent_session_capability(&sid)
        .await
        .unwrap();
    let response = server
        .client
        .post(format!("{}/v1/emergency-stop", server.base))
        .bearer_auth(&session_token)
        .json(&json!({"reason": "session drill"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let (_, audit) = server.get("/v1/audit?limit=200").await;
    let entry = audit
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == json!("emergency.stop"))
        .cloned()
        .expect("緊急停止要留下 audit");
    assert_eq!(
        entry["actor"],
        json!(format!("agent:agent.coder@{sid}")),
        "session token 觸發的停止必須記到是哪個 agent／哪個 session：{entry}"
    );
}

/// safety-invariants-057（cancel 分支）：agent token 取消單一動作時，
/// `action.cancelled` 的 audit actor 同樣不得寫死成人類的 "api"。
#[tokio::test]
async fn cancelling_an_action_records_the_real_principal_in_the_audit_trail() {
    let server = TestServer::spawn().await;
    server
        .runtime
        .registry
        .set_actuator_enabled(&interaction_core::ActuatorId::new("mock.actuator"), true)
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
        .post("/v1/session/start", json!({"label": "cancel actor"}))
        .await;
    assert_eq!(status, 200, "{session}");
    let (status, _) = server
        .post(
            "/v1/session/consent",
            json!({"scope": "actuator:mock.actuator"}),
        )
        .await;
    assert_eq!(status, 200);

    let (status, plan) = server
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
    let (status, receipts) = server
        .post(&format!("/v1/plans/{plan_id}/execute"), json!({}))
        .await;
    assert_eq!(status, 200, "{receipts}");
    let action_id = receipts[0]["actionId"].as_str().unwrap().to_string();

    let response = server
        .client
        .post(format!("{}/v1/actions/{action_id}/cancel", server.base))
        .bearer_auth(&server.agent_token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success() || response.status().as_u16() == 409,
        "取消請求要嘛成功、要嘛因為已終結而衝突：{}",
        response.status()
    );
    if response.status().is_success() {
        let (_, audit) = server.get("/v1/audit?limit=200").await;
        let entry = audit
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["kind"] == json!("action.cancelled"))
            .cloned()
            .expect("取消要留下 audit");
        assert_eq!(
            entry["actor"], "agent",
            "agent token 觸發的取消不得記成人類的 \"api\"：{entry}"
        );
    }
}

// ---------------------------------------------------------------------------
// AIP Character Session：四條 HTTP 路由的分權與形狀
// ---------------------------------------------------------------------------

/// human token 讀得到 snapshot／diagnostics，也送得出可信 surface 的語意事件；
/// agent／session-scoped／adapter token 一律 403（SSE 的同名事件同界線）。
#[tokio::test]
async fn character_session_routes_are_human_only() {
    let server = TestServer::spawn().await;

    // 1) human：snapshot 是一則 AIP `state{kind:"snapshot"}` envelope。
    let (status, snapshot) = server.get("/v1/character-session").await;
    assert_eq!(status, 200, "{snapshot}");
    assert_eq!(snapshot["specVersion"], "aip/1.0");
    assert_eq!(snapshot["messageType"], "state");
    assert_eq!(snapshot["payload"]["kind"], "snapshot");
    assert_eq!(
        snapshot["target"],
        json!({"kind":"human-surface","id":"desktop"})
    );
    let revision = snapshot["payload"]["revision"].as_u64().expect("revision");
    let epoch = snapshot["payload"]["sessionEpoch"].as_u64().expect("epoch");

    // 2) human：diagnostics 不含 token、路徑、原始 payload。
    let (status, diagnostics) = server.get("/v1/character-session/diagnostics").await;
    assert_eq!(status, 200, "{diagnostics}");
    assert_eq!(diagnostics["sessionId"], "session.home");
    assert!(diagnostics["counters"].is_object());
    let printed = diagnostics.to_string();
    assert!(!printed.contains(&server.token), "diagnostics 不得帶 token");
    assert!(!printed.contains("/"), "diagnostics 不得帶路徑：{printed}");

    // 3) human：resume 回 patches 或 snapshot（都不是錯誤）。
    let (status, resumed) = server
        .post(
            "/v1/character-session/resume",
            json!({"lastRevision": revision, "lastSequence": 0, "epoch": epoch}),
        )
        .await;
    // HTTP 自帶請求-回應對應，所以回的是 `response` 的 **payload**（wss 才需要
    // 完整 envelope＋causationId）。
    assert_eq!(status, 200, "{resumed}");
    assert!(
        resumed["kind"] == json!("patches") || resumed["kind"] == json!("snapshot"),
        "{resumed}"
    );

    // 4) 未 join 的 surface 送 event → not-a-member（桌面要先 /v1/character/hello）。
    let event = json!({
        "specVersion": "aip/1.0",
        "messageId": "http-touch-1",
        "messageType": "event",
        "name": "character.interaction.touch",
        "source": {"kind": "human-surface", "id": "desktop"},
        "sessionId": "session.home",
        "occurredAt": chrono::Utc::now().to_rfc3339(),
        "expiresAt": (chrono::Utc::now() + chrono::Duration::seconds(5)).to_rfc3339(),
        "payload": {"kind": "tap"},
    });
    let (status, result) = server
        .post("/v1/character-session/events", json!({"envelope": event}))
        .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["messageType"], "result");
    assert_eq!(result["payload"]["status"], "rejected");
    assert_eq!(result["payload"]["code"], "not-a-member");

    // 5) 偽造身分：human token 綁定的是 human-surface:desktop，不是別人。
    let mut forged = event.clone();
    forged["messageId"] = json!("http-touch-forged");
    forged["source"] = json!({"kind": "device", "id": "iphone-someone"});
    let (status, result) = server
        .post("/v1/character-session/events", json!({"envelope": forged}))
        .await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["payload"]["code"], "identity-mismatch");

    // 6) agent token：四條路由全 403（GET 與 POST 都是）。
    for (method, path, body) in [
        (reqwest::Method::GET, "/v1/character-session", Value::Null),
        (
            reqwest::Method::GET,
            "/v1/character-session/diagnostics",
            Value::Null,
        ),
        (
            reqwest::Method::POST,
            "/v1/character-session/resume",
            json!({"lastRevision": 0}),
        ),
        (
            reqwest::Method::POST,
            "/v1/character-session/events",
            json!({"envelope": event}),
        ),
    ] {
        let mut request = server
            .client
            .request(method.clone(), format!("{}{path}", server.base))
            .bearer_auth(&server.agent_token);
        if !body.is_null() {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        assert_eq!(
            response.status(),
            403,
            "{method} {path} 必須拒絕 agent token"
        );
    }

    // 7) character adapter token：同樣 403（它只能 POST 自己的回執／事件）。
    let manifest: Value = serde_json::from_str(FIXTURE_MANIFEST).unwrap();
    let (status, added) = server
        .post(
            "/v1/character/adapters",
            json!({"displayName": "文字 adapter（session 分權）", "manifest": manifest}),
        )
        .await;
    assert_eq!(status, 200, "{added}");
    let adapter_token = added["token"].as_str().unwrap().to_string();
    let response = server
        .client
        .get(format!("{}/v1/character-session", server.base))
        .bearer_auth(&adapter_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
}
