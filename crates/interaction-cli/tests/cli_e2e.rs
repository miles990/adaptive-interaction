//! CLI end-to-end (acceptance scenario H): simulates a shell-capable agent
//! following SKILL.md — capabilities → observe → plan → simulate → execute →
//! action show → verify — entirely through the `interact-ai` binary.

use serde_json::Value;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Daemon {
    child: Child,
    home: tempfile::TempDir,
    port: u16,
}

/// Sequential port allocation: avoids the classic bind-:0-then-release TOCTOU
/// race between concurrently spawning test daemons (a just-released ephemeral
/// port is exactly what the kernel likes to hand out next).
fn next_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static COUNTER: AtomicU16 = AtomicU16::new(0);
    let base = 21000 + (std::process::id() % 20000) as u16;
    base + COUNTER.fetch_add(7, Ordering::SeqCst)
}

impl Daemon {
    fn spawn() -> Self {
        let mut last_err = String::new();
        for _attempt in 0..3 {
            let home = tempfile::tempdir().unwrap();
            let port = next_port();
            let stderr_file = home.path().join("daemon.stderr");
            let mut child = Command::new(env!("CARGO_BIN_EXE_interact-ai"))
                .args([
                    "--config",
                    home.path().to_str().unwrap(),
                    "serve",
                    "--port",
                    &port.to_string(),
                ])
                .stdout(Stdio::null())
                .stderr(std::fs::File::create(&stderr_file).unwrap())
                .spawn()
                .expect("spawn daemon");
            // Wait for readiness; bail early if the daemon died (port taken…).
            let mut ready = false;
            for _ in 0..300 {
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    ready = true;
                    break;
                }
                if let Ok(Some(status)) = child.try_wait() {
                    last_err = format!(
                        "daemon exited early ({status}): {}",
                        std::fs::read_to_string(&stderr_file).unwrap_or_default()
                    );
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if ready {
                return Self { child, home, port };
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        panic!("daemon did not become ready after 3 attempts; last error: {last_err}");
    }

    fn cli(&self, args: &[&str]) -> (i32, Value, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_interact-ai"))
            .args([
                "--json",
                "--config",
                self.home.path().to_str().unwrap(),
                "--api",
                &format!("http://127.0.0.1:{}", self.port),
            ])
            .args(args)
            .output()
            .expect("run cli");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let json = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
        (output.status.code().unwrap_or(-1), json, stderr)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn scenario_h_skill_plus_cli_agent_loop() {
    let daemon = Daemon::spawn();

    // 1. Runtime check.
    let (code, status, _) = daemon.cli(&["status"]);
    assert_eq!(code, 0);
    assert_eq!(status["name"], "adaptive-interaction");

    // 2. Discover capabilities (JSON only on stdout — parseable by any agent).
    let (code, caps, _) = daemon.cli(&["capabilities"]);
    assert_eq!(code, 0);
    assert!(caps["actuators"].as_array().unwrap().len() >= 2);

    // 3. Start a session.
    let (code, session, _) = daemon.cli(&["session", "start", "--label", "agent"]);
    assert_eq!(code, 0);
    assert_eq!(session["state"], "active");

    // 4. Push + query observations.
    let (code, _, _) = daemon.cli(&[
        "receptors",
        "push",
        "task.lifecycle",
        "--fact",
        "event=task.completed",
        "--fact",
        "title=demo",
    ]);
    assert_eq!(code, 0);
    let (code, observations, _) = daemon.cli(&["observe", "--receptor", "task.lifecycle"]);
    assert_eq!(code, 0);
    assert_eq!(observations[0]["facts"]["event"], "task.completed");

    // 5. Plan.
    let (code, plan, _) = daemon.cli(&[
        "plan",
        "--intent",
        "celebration",
        "--candidate",
        "conversation",
        "--min-channels",
        "1",
        "--max-channels",
        "1",
        "--deny-no-action",
    ]);
    assert_eq!(code, 0);
    let plan_id = plan["planId"].as_str().unwrap().to_string();

    // 6. Simulate.
    let (code, sim, _) = daemon.cli(&["simulate", &plan_id]);
    assert_eq!(code, 0);
    assert_eq!(sim["wouldExecute"], true);

    // 7. Execute.
    let (code, receipts, _) = daemon.cli(&["execute", &plan_id]);
    assert_eq!(code, 0);
    let action_id = receipts[0]["actionId"].as_str().unwrap().to_string();
    assert_eq!(receipts[0]["currentStatus"], "completed");

    // 8. Action show.
    let (code, receipt, _) = daemon.cli(&["actions", "show", &action_id]);
    assert_eq!(code, 0);
    assert_eq!(receipt["currentStatus"], "completed");

    // 9. Verify.
    let (code, verified, _) = daemon.cli(&["verify", &action_id]);
    assert_eq!(code, 0);
    assert_eq!(verified["currentStatus"], "completed");

    // 10. The conversation output is retrievable (the agent can relay it).
    let (code, outbox, _) = daemon.cli(&["outbox"]);
    assert_eq!(code, 0);
    assert!(!outbox.as_array().unwrap().is_empty());
}

#[test]
fn tools_export_writes_all_formats() {
    let daemon = Daemon::spawn();
    let out_dir = daemon.home.path().join("exports");
    std::fs::create_dir_all(&out_dir).unwrap();
    for format in ["openai", "anthropic", "gemini", "openapi", "json-schema"] {
        let out = out_dir.join(format!("{format}.json"));
        let (code, _, stderr) = daemon.cli(&[
            "tools",
            "export",
            "--format",
            format,
            "--out",
            out.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{format}: {stderr}");
        let content: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert!(content.is_object(), "{format}");
    }
}

#[test]
fn emergency_stop_from_cli_and_stable_exit_codes() {
    let daemon = Daemon::spawn();
    daemon.cli(&["session", "start"]);

    let (code, result, _) = daemon.cli(&["emergency-stop", "--reason", "cli-drill"]);
    assert_eq!(code, 0);
    assert_eq!(result["reason"], "cli-drill");

    // Not-found exit code is 5.
    let (code, _, _) = daemon.cli(&["actions", "show", "nope"]);
    assert_eq!(code, 5);

    // Clear.
    let (code, _, _) = daemon.cli(&["emergency-stop", "--clear"]);
    assert_eq!(code, 0);
}

#[test]
fn daemon_offline_exit_code_is_3() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_interact-ai"))
        .args([
            "--json",
            "--config",
            home.path().to_str().unwrap(),
            "--api",
            "http://127.0.0.1:1", // nothing listens here
            "status",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    // stdout stays clean in error cases (diagnostics on stderr only).
    assert!(output.stdout.is_empty());
}

#[test]
fn duplicate_daemon_is_refused_by_instance_lock() {
    let daemon = Daemon::spawn();
    let port = next_port();
    let output = Command::new(env!("CARGO_BIN_EXE_interact-ai"))
        .args([
            "--config",
            daemon.home.path().to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .output()
        .expect("second daemon attempt");
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already holds")
            || stderr.contains("Conflict")
            || stderr.contains("conflict"),
        "stderr: {stderr}"
    );
}

/// 續開（resume）走 CLI 的兩個入口：`agents create --resume <provider id>`
/// 與 `agents resume <session id>`。續開不是「把舊權限接回來」——它是一份
/// 新租約：權限旗標重新上鎖，而且沒有 provider 端 thread 時要誠實拒絕，
/// 不得憑空編一個 id 出來。
#[test]
fn agents_resume_requires_a_real_provider_thread_and_never_inherits_write_access() {
    let daemon = Daemon::spawn();

    // 一個沒有接上任何子程序的 session：它沒有 provider 端 thread。
    let (code, created, stderr) = daemon.cli(&[
        "agents",
        "create",
        "--agent",
        "agent.cli-resume",
        "--label",
        "原本的工作",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let id = created["sessionId"].as_str().unwrap().to_string();
    assert_eq!(created["providerSessionId"], Value::Null);

    // 沒有 providerSessionId 就誠實拒絕續開（stdout 保持乾淨）。
    let output = Command::new(env!("CARGO_BIN_EXE_interact-ai"))
        .args([
            "--json",
            "--config",
            daemon.home.path().to_str().unwrap(),
            "--api",
            &format!("http://127.0.0.1:{}", daemon.port),
            "agents",
            "resume",
            &id,
        ])
        .output()
        .expect("run cli");
    assert_ne!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty(), "stdout stays clean on error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("providerSessionId"),
        "the refusal must say why: {stderr}"
    );

    // 不存在的 session 照實回傳 not-found 的退出碼（5），不代填。
    let (code, _, _) = daemon.cli(&["agents", "resume", "nope"]);
    assert_eq!(code, 5);

    // create --resume 把 provider thread id 帶進建立請求；新 session 仍是
    // 唯讀（權限旗標重新上鎖，不繼承任何東西）。
    let (code, resumed, stderr) = daemon.cli(&[
        "agents",
        "create",
        "--agent",
        "agent.cli-resume",
        "--label",
        "續開的工作",
        "--resume",
        "provider-thread-abc",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(resumed["allowWrite"], false);
    assert_eq!(resumed["consentScope"], serde_json::json!([]));
    assert_eq!(resumed["toolScope"], serde_json::json!([]));
    assert_ne!(resumed["sessionId"], created["sessionId"]);
}

/// Character Presentation Protocol 子命令：status／instances／manifest／adapters
/// add→list→revoke（token 只印一次、清單永不含 token）／intent（安全 intent 拒絕）。
#[test]
fn character_subcommands_manage_adapters_and_refuse_safety_intents() {
    let daemon = Daemon::spawn();

    let (code, status, _) = daemon.cli(&["character", "status"]);
    assert_eq!(code, 0);
    assert_eq!(status["version"], "1.0");
    assert_eq!(status["instances"], 0);
    assert!(status["activeCharacter"].is_null());

    let (code, instances, _) = daemon.cli(&["character", "instances"]);
    assert_eq!(code, 0);
    assert!(instances["instances"].as_array().unwrap().is_empty());

    // 尚未 hello → manifest 404 → exit 5。
    let (code, _, _) = daemon.cli(&["character", "manifest"]);
    assert_eq!(code, 5);

    let manifest_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/character-adapters/text-adapter.manifest.json"
    );
    let (code, added, _) = daemon.cli(&[
        "character",
        "adapters",
        "add",
        "--name",
        "文字 adapter（fixture）",
        "--manifest",
        manifest_path,
    ]);
    assert_eq!(code, 0, "{added}");
    let adapter_id = added["adapterId"].as_str().unwrap().to_string();
    let token = added["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);

    let (code, list, _) = daemon.cli(&["character", "adapters", "list"]);
    assert_eq!(code, 0);
    let entry = &list["adapters"][0];
    assert_eq!(entry["adapterId"], adapter_id);
    assert_eq!(entry["revoked"], false);
    assert_eq!(entry["connected"], false);
    assert!(entry.get("token").is_none());
    assert!(!list.to_string().contains(&token));

    // 安全 intent 只能由 runtime 事件產生：CLI 手動點播一律拒絕（403 → exit 4）。
    let (code, _, _) = daemon.cli(&["character", "intent", "emergency"]);
    assert_eq!(code, 4);
    let (code, _, _) = daemon.cli(&["character", "intent", "verified-success"]);
    assert_eq!(code, 4);
    // 非安全 intent：沒有連線的角色 → targets 空、誠實註記。
    let (code, out, _) = daemon.cli(&["character", "intent", "notice", "--message", "hi"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["truthState"], "none");
    assert_eq!(out["targets"], serde_json::json!([]));
    assert!(out["note"].as_str().unwrap().contains("no connected"));

    let (code, revoked, _) = daemon.cli(&["character", "adapters", "revoke", &adapter_id]);
    assert_eq!(code, 0);
    assert_eq!(revoked["revoked"], true);
    let (_, list, _) = daemon.cli(&["character", "adapters", "list"]);
    assert_eq!(list["adapters"][0]["revoked"], true);
    // 不存在的 adapter → 404 → exit 5。
    let (code, _, _) = daemon.cli(&["character", "adapters", "revoke", "adp-nope"]);
    assert_eq!(code, 5);
    // 缺檔案的 manifest 路徑 → 一般錯誤（exit 1），不會誤註冊。
    let (code, _, _) = daemon.cli(&[
        "character",
        "adapters",
        "add",
        "--name",
        "x",
        "--manifest",
        "/nonexistent/manifest.json",
    ]);
    assert_eq!(code, 1);
    let (_, list, _) = daemon.cli(&["character", "adapters", "list"]);
    assert_eq!(list["adapters"].as_array().unwrap().len(), 1);
}
