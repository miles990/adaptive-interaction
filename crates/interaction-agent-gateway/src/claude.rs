//! Claude Code connector（spec §8.2）：本機 CLI，
//! `claude -p --input-format stream-json --output-format stream-json --verbose`。
//!
//! - 預設 `--permission-mode plan`（唯讀優先；放寬需 runtime 端人類同意）。
//! - 寫入型限權 session（write_enabled，由人類建立 payload 明確帶入）用
//!   `--permission-mode acceptEdits`：只放行檔案編輯，其餘工具照常受限。
//! - 任何模式都不使用 `--dangerously-skip-permissions`（有 regression test）。
//! - `--safe-mode`＋空的 strict MCP config 隔離使用者 hooks／plugins／MCP；
//!   本產品的 Agent Session 不繼承任意外部工具或啟動成本。
//! - 登入狀態用 `claude auth status`（JSON）；不接觸 credential。
//! - 事件解析為純函式（parse_claude_line），可離線以錄好的樣本測試。

use crate::process::{
    apply_session_capability_env, remove_runtime_auth_env, spawn_grouped, ProcessGroup,
};
use crate::{
    AgentConnector, AgentDiscovery, AgentKind, AgentSessionHandle, ApprovalDecision, GatewayError,
    GatewayEvent, SessionSpec,
};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
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
    let mut cmd = Command::new(binary);
    remove_runtime_auth_env(&mut cmd);
    let out = tokio::time::timeout(DISCOVER_TIMEOUT, cmd.args(args).output())
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

/// session 啟動參數（純函式，可離線測試）。安全不變量：
/// - 任何模式都不得出現 `--dangerously-skip-permissions`。
/// - 唯讀（預設）＝ `--permission-mode plan`；寫入型限權 session
///   （write_enabled，人類建立 payload 明確帶入）＝ `--permission-mode
///   acceptEdits`——只放行檔案編輯，其餘工具照常受限。
pub fn claude_session_args(spec: &SessionSpec) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-p".into(),
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--safe-mode".into(),
        "--strict-mcp-config".into(),
        "--mcp-config".into(),
        r#"{"mcpServers":{}}"#.into(),
    ];
    if spec.write_enabled {
        args.extend(["--permission-mode".into(), "acceptEdits".into()]);
        // 明列內建工具，避免使用者設定中的 MCP／plugin tools 擴大 session。
        // 寫入型仍不包含 Bash／WebFetch／WebSearch；需要指令時由其他受治理
        // tool/session 處理，不能把「可改檔」偷換成「任意執行／網路」。
        args.extend(["--tools".into(), "Read,Glob,Grep,Edit,Write".into()]);
    } else if spec.disable_tools {
        args.extend(["--permission-mode".into(), "plan".into()]);
        // Claude documents an empty --tools value as disabling every tool.
        args.extend(["--tools".into(), "".into()]);
    } else {
        // Plan 模式＝唯讀優先：Claude 可讀可規劃，寫入工具需要核可，
        // 而 -p 非互動模式下沒有核可管道 ⇒ 實質唯讀。
        args.extend(["--permission-mode".into(), "plan".into()]);
        args.extend(["--tools".into(), "Read,Glob,Grep".into()]);
    }
    if let Some(model) = &spec.model {
        args.extend(["--model".into(), model.clone()]);
    }
    if let Some(resume) = &spec.resume_provider_session {
        args.extend(["--resume".into(), resume.clone()]);
    }
    if let Some(turns) = spec.max_turns {
        args.extend(["--max-turns".into(), turns.to_string()]);
    }
    if let Some(cost) = spec.max_cost_usd.filter(|cost| *cost > 0.0) {
        args.extend(["--max-budget-usd".into(), cost.to_string()]);
    }
    args
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
        remove_runtime_auth_env(&mut cmd);
        apply_session_capability_env(&mut cmd, &spec);
        cmd.current_dir(&spec.workdir)
            .args(claude_session_args(&spec));
        let mut child = spawn_grouped(cmd)?;
        // pgid 必須在 spawn 後立即捕捉：kill 路徑不能依賴 child 屆時是否已被收割。
        let group = ProcessGroup::of(&child);
        let stdout = child.stdout.take().ok_or(GatewayError::Closed)?;
        let stderr = child.stderr.take().ok_or(GatewayError::Closed)?;
        let stdin = child.stdin.take().ok_or(GatewayError::Closed)?;

        let (tx, rx) = mpsc::channel::<GatewayEvent>(EVENT_CHANNEL_CAP);
        let session_id = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        // 「這一輪有沒有收到結果」是**每一輪**的問題：session 可多輪，
        // 第一輪的 result 不能替第二輪擔保。送出新訊息時（handle）重置，
        // 讀到 result（stdout task）才設。
        let saw_result = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

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

        // stdout：逐行解析為正規化事件。Child 交給這個 task 持有並收割，
        // 好讓收場時能讀到真正的 exit status（kill 路徑走 process group，
        // 不需要 Child——見 ClaudeHandle::kill）。
        {
            let tx = tx.clone();
            let session_id = session_id.clone();
            let tail = stderr_tail.clone();
            let saw_result = saw_result.clone();
            tokio::spawn(async move {
                let mut child = child;
                let mut lines = BufReader::new(stdout).lines();
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
                                    saw_result.store(true, std::sync::atomic::Ordering::SeqCst);
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
                // 程序輸出結束：等它真的退出，才知道收場是什麼。
                let status = child.wait().await.ok();
                // 被訊號終止時 code() 是 None——那多半是我們自己的 kill，
                // 不是 agent 的錯誤，不得記成失敗。
                let exit_code = status.and_then(|s| s.code());
                let stderr_tail: Option<String> = {
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
                let exit_note = match (status, exit_code) {
                    (_, Some(code)) => format!("exit {code}"),
                    (Some(_), None) => "terminated by signal".to_string(),
                    (None, None) => "exit status unknown".to_string(),
                };
                let detail = Some(match &stderr_tail {
                    Some(t) => format!("{exit_note}; stderr: {t}"),
                    None => exit_note.clone(),
                });
                // 只有**觀察得到的錯誤**（非零 exit code）才是 failed。
                // exit 0 卻沒有結果，或被訊號終止 ⇒ 結果未知，交給
                // SessionClosed，由 runtime 誠實記為 unknown。
                // saw_result 是「本輪」的：第二輪送出後重置，所以第二輪
                // 以後的非零 exit 一樣會被記成 failed，不會被第一輪的
                // result 吞掉。
                if !saw_result.load(std::sync::atomic::Ordering::SeqCst) {
                    if let Some(code) = exit_code.filter(|code| *code != 0) {
                        let error: String = match &stderr_tail {
                            Some(t) => format!("agent 程序以 exit {code} 結束而未回報結果：{t}"),
                            None => format!("agent 程序以 exit {code} 結束而未回報結果"),
                        }
                        .chars()
                        .take(700)
                        .collect();
                        let _ = tx.send(GatewayEvent::TaskFailed { error }).await;
                    }
                }
                let resumable = session_id.lock().expect("sid lock").is_some();
                let _ = tx
                    .send(GatewayEvent::SessionClosed { resumable, detail })
                    .await;
            });
        }

        let mut handle = ClaudeHandle {
            group,
            stdin: Some(stdin),
            events: Some(rx),
            session_id,
            saw_result,
        };
        if let Some(prompt) = &spec.prompt {
            handle.send_user_message(prompt).await?;
        }
        Ok(Box::new(handle))
    }
}

pub struct ClaudeHandle {
    /// spawn 當下捕捉的 process group。Child 由 stdout task 持有／收割，
    /// 所以中斷與終止都只靠 pgid 訊號——鎖外、不依賴 Child 的存活狀態。
    group: ProcessGroup,
    stdin: Option<ChildStdin>,
    events: Option<mpsc::Receiver<GatewayEvent>>,
    session_id: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// 本輪是否已讀到 result／error（與 stdout task 共用；每次送出新訊息重置）。
    saw_result: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
        // 新的一輪開始：上一輪的 result 不再替這一輪擔保。在寫入**之前**
        // 重置——寫入一落地 agent 就可能回話甚至結束，reader 不得先看到
        // 舊的 true。寫入失敗時子程序也已經在收場，reader 依 exit code
        // 誠實判定即可。
        self.saw_result
            .store(false, std::sync::atomic::Ordering::SeqCst);
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
        self.group.interrupt();
        Ok(())
    }

    async fn kill(&mut self) -> Result<(), GatewayError> {
        self.stdin.take(); // 關 stdin（-p 模式的正常收尾訊號）
                           // 整組 SIGTERM → 寬限 → SIGKILL。領頭由 stdout task 的 child.wait()
                           // 收割（它同時把真實 exit status 寫進 SessionClosed detail）。
        self.group.terminate(1500).await;
        Ok(())
    }

    fn process_group(&self) -> ProcessGroup {
        self.group.clone()
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
                // system/init 只代表「子程序起來了」——此時還沒有任何任務
                // 送進去，更沒有人在工作。誠實階梯：進度（working）必須等
                // 第一個 assistant／tool 事件，不能由啟動訊息偽造。
                vec![GatewayEvent::SessionStarted {
                    provider_session_id: sid,
                }]
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
    fn session_args_keep_read_and_write_scopes_bounded() {
        let ro = SessionSpec::read_only_in(std::path::PathBuf::from("/tmp/repo"));
        let ro_args = claude_session_args(&ro);
        assert!(ro_args
            .windows(2)
            .any(|w| w == ["--permission-mode", "plan"]));
        assert!(ro_args
            .windows(2)
            .any(|w| w == ["--tools", "Read,Glob,Grep"]));
        assert!(ro_args.iter().any(|arg| arg == "--safe-mode"));
        assert!(ro_args.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(ro_args
            .windows(2)
            .any(|w| w == ["--mcp-config", r#"{"mcpServers":{}}"#]));

        let wr = SessionSpec::write_enabled_in(std::path::PathBuf::from("/tmp/repo"));
        let wr_args = claude_session_args(&wr);
        assert!(wr_args
            .windows(2)
            .any(|w| w == ["--permission-mode", "acceptEdits"]));
        assert!(wr_args
            .windows(2)
            .any(|w| w == ["--tools", "Read,Glob,Grep,Edit,Write"]));
        let all = wr_args.join(" ");
        assert!(!all.contains("dangerously"));
        assert!(!all.contains("Bash"));
        assert!(!all.contains("WebFetch"));
        assert!(!all.contains("WebSearch"));
    }

    #[test]
    fn parses_the_recorded_live_sample_shape() {
        // 與實測樣本同形（connector-probe 錄得）。
        let init = r#"{"type":"system","subtype":"init","session_id":"26af1963","model":"claude-haiku-4-5"}"#;
        let events = parse_claude_line(init);
        assert_eq!(
            events,
            vec![GatewayEvent::SessionStarted {
                provider_session_id: "26af1963".into()
            }]
        );

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

    /// regression（誠實階梯）：`system/init` 只是「子程序起來了」，曾被
    /// 直接翻成 TaskAccepted，讓角色在任務送進去之前就演「工作中」。
    /// 進度只能來自真正的 assistant／tool 事件。
    #[test]
    fn init_never_reports_work_before_the_first_assistant_or_tool_event() {
        let init = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;
        let events = parse_claude_line(init);
        assert!(
            !events.iter().any(|e| matches!(
                e,
                GatewayEvent::TaskAccepted | GatewayEvent::TaskProgress { .. }
            )),
            "init must not imply work: {events:?}"
        );
        // 第一個 assistant 事件才是「在做事」。
        assert_eq!(
            parse_claude_line(
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"開始看"}]}}"#
            ),
            vec![GatewayEvent::TaskProgress {
                text: Some("開始看".into())
            }]
        );
        assert_eq!(
            parse_claude_line(
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Grep"}]}}"#
            ),
            vec![GatewayEvent::ToolStarted {
                name: "Grep".into()
            }]
        );
    }

    /// Approval 對稱性：`claude -p` 沒有互動核可管道，所以正規化層**絕不**
    /// 為 claude session 產生 waiting-for-consent（那會讓 UI 出現一個永遠
    /// 無法裁決的假請求），而 resolve_approval 一律誠實拒絕。
    #[test]
    fn claude_never_fabricates_an_approval_request() {
        for line in [
            r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write"}]}}"#,
            r#"{"type":"system","subtype":"permission_request","tool":"Write"}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"denied"}]}}"#,
            r#"{"type":"result","subtype":"error_permission","is_error":true,"result":"needs approval"}"#,
        ] {
            assert!(
                !parse_claude_line(line)
                    .iter()
                    .any(|e| matches!(e, GatewayEvent::TaskWaitingForConsent { .. })),
                "claude must never produce waiting-for-consent: {line}"
            );
        }
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
