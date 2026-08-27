//! Claude Code connector（spec §8.2）：本機 CLI，
//! `claude -p --input-format stream-json --output-format stream-json --verbose`。
//!
//! - 預設 `--permission-mode plan`（唯讀優先；放寬需 runtime 端人類同意）。
//! - 不使用 `--dangerously-skip-permissions`。
//! - 登入狀態用 `claude auth status`（JSON）；不接觸 credential。
//! - 事件解析為純函式（parse_claude_line），可離線以錄好的樣本測試。

use crate::process::{interrupt_tree, kill_tree, spawn_grouped};
use crate::{
    AgentConnector, AgentDiscovery, AgentKind, AgentSessionHandle, ApprovalDecision, GatewayError,
    GatewayEvent, SessionSpec,
};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

const DISCOVER_TIMEOUT: Duration = Duration::from_secs(6);
const EVENT_CHANNEL_CAP: usize = 256;

pub struct ClaudeConnector {
    pub binary: String,
}

impl Default for ClaudeConnector {
    fn default() -> Self {
        Self {
            binary: "claude".into(),
        }
    }
}

impl ClaudeConnector {
    pub fn with_binary(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
        }
    }
}

async fn run_capture(binary: &str, args: &[&str]) -> Result<String, String> {
    let out = tokio::time::timeout(DISCOVER_TIMEOUT, Command::new(binary).args(args).output())
        .await
        .map_err(|_| "timeout".to_string())?
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(200)
                .collect::<String>()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[async_trait::async_trait]
impl AgentConnector for ClaudeConnector {
    fn kind(&self) -> AgentKind {
        AgentKind::ClaudeCode
    }

    async fn discover(&self) -> AgentDiscovery {
        let version = match run_capture(&self.binary, &["--version"]).await {
            Ok(v) => v.trim().to_string(),
            Err(e) => {
                return AgentDiscovery::missing(
                    AgentKind::ClaudeCode,
                    format!("claude 不可用：{e}"),
                )
            }
        };
        // 登入狀態：讀 JSON 的 loggedIn 欄位；讀不到 → 誠實 unknown。
        let logged_in = match run_capture(&self.binary, &["auth", "status"]).await {
            Ok(body) => serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.get("loggedIn").and_then(|b| b.as_bool())),
            Err(_) => None,
        };
        AgentDiscovery {
            kind: AgentKind::ClaudeCode,
            found: true,
            binary_path: Some(self.binary.clone()),
            version: Some(version.clone()),
            logged_in,
            protocol_supported: Some(true), // stream-json 自 v1 起穩定存在
            detail: match logged_in {
                Some(true) => format!("{version}（已登入）"),
                Some(false) => format!("{version}（未登入——請先在終端執行 claude 登入）"),
                None => format!("{version}（登入狀態未知）"),
            },
        }
    }

    async fn start_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSessionHandle>, GatewayError> {
        let mut cmd = Command::new(&self.binary);
        cmd.current_dir(&spec.workdir)
            .arg("-p")
            .args(["--input-format", "stream-json"])
            .args(["--output-format", "stream-json"])
            .arg("--verbose");
        if spec.read_only {
            // Plan 模式＝唯讀優先：Claude 可讀可規劃，寫入工具需要核可，
            // 而 -p 非互動模式下沒有核可管道 ⇒ 實質唯讀。
            cmd.args(["--permission-mode", "plan"]);
        }
        if let Some(model) = &spec.model {
            cmd.args(["--model", model]);
        }
        if let Some(resume) = &spec.resume_provider_session {
            cmd.args(["--resume", resume]);
        }
        if let Some(turns) = spec.max_turns {
            cmd.args(["--max-turns", &turns.to_string()]);
        }
        let mut child = spawn_grouped(cmd)?;
        let stdout = child.stdout.take().ok_or(GatewayError::Closed)?;
        let stderr = child.stderr.take().ok_or(GatewayError::Closed)?;
        let stdin = child.stdin.take().ok_or(GatewayError::Closed)?;

        let (tx, rx) = mpsc::channel::<GatewayEvent>(EVENT_CHANNEL_CAP);
        let session_id = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));

        // stderr：吞掉但保留最後幾行（診斷；不視為事件）。
        let stderr_tail = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        {
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut t = tail.lock().expect("stderr tail lock");
                    t.push_str(&line);
                    t.push('\n');
                    let len = t.len();
                    if len > 2000 {
                        let cut = len - 2000;
                        t.drain(..cut);
                    }
                }
            });
        }

        // stdout：逐行解析為正規化事件。
        {
            let tx = tx.clone();
            let session_id = session_id.clone();
            let tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                let mut saw_result = false;
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => {
                            for ev in parse_claude_line(&line) {
                                if let GatewayEvent::SessionStarted {
                                    provider_session_id,
                                } = &ev
                                {
                                    *session_id.lock().expect("sid lock") =
                                        Some(provider_session_id.clone());
                                }
                                if matches!(
                                    ev,
                                    GatewayEvent::TaskClaimedCompleted { .. }
                                        | GatewayEvent::TaskFailed { .. }
                                ) {
                                    saw_result = true;
                                }
                                if tx.send(ev).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
                // 程序輸出結束：誠實回報 session 收場。
                let detail = {
                    let t = tail.lock().expect("stderr tail lock");
                    if t.trim().is_empty() {
                        None
                    } else {
                        Some(
                            t.trim()
                                .chars()
                                .rev()
                                .take(300)
                                .collect::<String>()
                                .chars()
                                .rev()
                                .collect(),
                        )
                    }
                };
                if !saw_result {
                    let _ = tx
                        .send(GatewayEvent::TaskFailed {
                            error: "agent 程序在回報結果前結束".into(),
                        })
                        .await;
                }
                let resumable = session_id.lock().expect("sid lock").is_some();
                let _ = tx
                    .send(GatewayEvent::SessionClosed { resumable, detail })
                    .await;
            });
        }

        let mut handle = ClaudeHandle {
            child,
            stdin: Some(stdin),
            events: Some(rx),
            session_id,
        };
        if let Some(prompt) = &spec.prompt {
            handle.send_user_message(prompt).await?;
        }
        Ok(Box::new(handle))
    }
}

pub struct ClaudeHandle {
    child: Child,
    stdin: Option<ChildStdin>,
    events: Option<mpsc::Receiver<GatewayEvent>>,
    session_id: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl AgentSessionHandle for ClaudeHandle {
    fn provider_session_id(&self) -> Option<String> {
        self.session_id.lock().expect("sid lock").clone()
    }

    async fn send_user_message(&mut self, text: &str) -> Result<(), GatewayError> {
        let stdin = self.stdin.as_mut().ok_or(GatewayError::Closed)?;
        let msg = json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "text", "text": text}]},
        });
        stdin
            .write_all(format!("{msg}\n").as_bytes())
            .await
            .map_err(|_| GatewayError::Closed)?;
        stdin.flush().await.map_err(|_| GatewayError::Closed)?;
        Ok(())
    }

    async fn resolve_approval(
        &mut self,
        _request_id: &str,
        _decision: ApprovalDecision,
    ) -> Result<(), GatewayError> {
        // -p 模式沒有互動核可管道（plan 模式下寫入工具直接被拒）。
        Err(GatewayError::Protocol(
            "claude -p 模式沒有互動核可管道".into(),
        ))
    }

    async fn interrupt(&mut self) -> Result<(), GatewayError> {
        interrupt_tree(&self.child);
        Ok(())
    }

    async fn kill(&mut self) -> Result<(), GatewayError> {
        self.stdin.take(); // 關 stdin（-p 模式的正常收尾訊號）
        kill_tree(&mut self.child, 1500).await;
        Ok(())
    }

    fn take_events(&mut self) -> Option<mpsc::Receiver<GatewayEvent>> {
        self.events.take()
    }
}

/// 純函式：一行 stream-json → 0..n 個正規化事件。
/// 壓力與畸形輸入下絕不 panic；解析不了的行保留為 Unparsed。
pub fn parse_claude_line(line: &str) -> Vec<GatewayEvent> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![GatewayEvent::Unparsed {
            raw: line.chars().take(300).collect(),
        }];
    };
    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let subtype = v.get("subtype").and_then(|t| t.as_str()).unwrap_or("");
    match (ty, subtype) {
        ("system", "init") => {
            let sid = v
                .get("session_id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            if sid.is_empty() {
                vec![GatewayEvent::Unparsed {
                    raw: "system/init without session_id".into(),
                }]
            } else {
                vec![
                    GatewayEvent::SessionStarted {
                        provider_session_id: sid,
                    },
                    GatewayEvent::TaskAccepted,
                ]
            }
        }
        ("assistant", _) => {
            // 內容可含 text 與 tool_use；text → 進度、tool_use → 工具事件。
            let mut out = vec![];
            if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                for item in content {
                    match item.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            if !text.is_empty() {
                                out.push(GatewayEvent::TaskProgress {
                                    text: Some(text.chars().take(2000).collect()),
                                });
                            }
                        }
                        Some("tool_use") => {
                            out.push(GatewayEvent::ToolStarted {
                                name: item
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("unknown")
                                    .to_string(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            if out.is_empty() {
                out.push(GatewayEvent::TaskProgress { text: None });
            }
            out
        }
        ("result", _) => {
            let is_error = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
            let cost = v.get("total_cost_usd").and_then(|c| c.as_f64());
            let turns = v.get("num_turns").and_then(|n| n.as_u64());
            if is_error || subtype.starts_with("error") {
                vec![GatewayEvent::TaskFailed {
                    error: v
                        .get("result")
                        .and_then(|r| r.as_str())
                        .unwrap_or(subtype)
                        .chars()
                        .take(500)
                        .collect(),
                }]
            } else {
                vec![GatewayEvent::TaskClaimedCompleted {
                    summary: v
                        .get("result")
                        .and_then(|r| r.as_str())
                        .map(|s| s.chars().take(4000).collect()),
                    cost_usd: cost,
                    num_turns: turns,
                }]
            }
        }
        // 心跳／統計／hook 類系統事件：不是任務語意，靜默略過。
        ("system", _) | ("rate_limit_event", _) | ("user", _) => vec![],
        _ => vec![GatewayEvent::Unparsed {
            raw: line.chars().take(300).collect(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_recorded_live_sample_shape() {
        // 與實測樣本同形（connector-probe 錄得）。
        let init = r#"{"type":"system","subtype":"init","session_id":"26af1963","model":"claude-haiku-4-5"}"#;
        let events = parse_claude_line(init);
        assert_eq!(
            events[0],
            GatewayEvent::SessionStarted {
                provider_session_id: "26af1963".into()
            }
        );
        assert_eq!(events[1], GatewayEvent::TaskAccepted);

        let asst = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"OK"}]}}"#;
        assert_eq!(
            parse_claude_line(asst),
            vec![GatewayEvent::TaskProgress {
                text: Some("OK".into())
            }]
        );

        let result = r#"{"type":"result","subtype":"success","is_error":false,"result":"OK","total_cost_usd":0.02,"num_turns":1}"#;
        match &parse_claude_line(result)[0] {
            GatewayEvent::TaskClaimedCompleted {
                summary,
                cost_usd,
                num_turns,
            } => {
                assert_eq!(summary.as_deref(), Some("OK"));
                assert_eq!(*cost_usd, Some(0.02));
                assert_eq!(*num_turns, Some(1));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn error_results_map_to_failed_not_completed() {
        let line =
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"stopped"}"#;
        assert!(matches!(
            parse_claude_line(line)[0],
            GatewayEvent::TaskFailed { .. }
        ));
    }

    #[test]
    fn malformed_and_hostile_lines_never_panic() {
        for garbage in [
            "",
            "not json at all",
            "{\"type\":\"???\"}",
            "{\"type\":\"assistant\"}",
            "{\"type\":\"result\"}",
            "{unclosed",
            &"x".repeat(100_000),
        ] {
            let _ = parse_claude_line(garbage); // 不 panic 即通過
        }
        // result 無 is_error → 誠實視為聲稱完成（is_error 預設 false 由上游定義）
        let claim = parse_claude_line("{\"type\":\"result\"}");
        assert!(matches!(
            claim[0],
            GatewayEvent::TaskClaimedCompleted { .. }
        ));
    }

    #[test]
    fn tool_use_maps_to_tool_started() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{}}]}}"#;
        assert_eq!(
            parse_claude_line(line),
            vec![GatewayEvent::ToolStarted {
                name: "Read".into()
            }]
        );
    }
}
