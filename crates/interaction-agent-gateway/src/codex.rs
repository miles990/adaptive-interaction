//! Codex connector（spec §8.1）：正式路徑 `codex app-server`（stdio JSON-RPC）。
//!
//! - initialize / initialized 握手 → thread/start（sandbox=read-only 預設）
//!   → turn/start → 持續通知 → turn/completed。
//! - `turn/interrupt` 取消目前 turn。
//! - Approval ServerRequest（exec／patch／權限）→ 正規化為 waiting-for-consent，
//!   由 runtime 的人類介面裁決；預設拒絕，絕不替人類同意。
//! - 協定形狀取自 `codex app-server generate-json-schema`（0.149.1 鎖定），
//!   不手寫猜測。舊版不支援 app-server 時自動走受限 exec fallback。

use crate::process::{
    apply_session_capability_env, interrupt_tree, kill_tree, remove_runtime_auth_env,
    spawn_grouped, ProcessGroup,
};
use crate::{
    AgentConnector, AgentDiscovery, AgentKind, AgentSessionHandle, ApprovalDecision, GatewayError,
    GatewayEvent, SessionSpec,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

const DISCOVER_TIMEOUT: Duration = Duration::from_secs(6);
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_CHANNEL_CAP: usize = 256;
/// 等 writer task 真的把一行寫進 stdin＋flush 的上限。子程序卡死不讀
/// stdin 時 pipe 會填滿，write 永遠不返回——有界等待，逾時誠實回失敗。
const WRITE_ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// 一行待送出的 JSON-RPC。`ack` 在**寫入 stdin 並 flush 之後**回報結果：
/// 「排進有界佇列」不是送達，呼叫端（送達戳記／fetched 事件）必須等真正
/// 寫進去才可以聲稱送達。
struct OutLine {
    line: String,
    ack: Option<oneshot::Sender<bool>>,
}

pub struct CodexConnector {
    pub binary: String,
}

impl Default for CodexConnector {
    fn default() -> Self {
        Self {
            binary: "codex".into(),
        }
    }
}

impl CodexConnector {
    pub fn with_binary(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
        }
    }
}

async fn capture(binary: &str, args: &[&str]) -> Result<(bool, String), String> {
    let mut cmd = Command::new(binary);
    remove_runtime_auth_env(&mut cmd);
    let out = tokio::time::timeout(DISCOVER_TIMEOUT, cmd.args(args).output())
        .await
        .map_err(|_| "timeout".to_string())?
        .map_err(|e| e.to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), text))
}

#[async_trait::async_trait]
impl AgentConnector for CodexConnector {
    fn kind(&self) -> AgentKind {
        AgentKind::Codex
    }

    async fn discover(&self) -> AgentDiscovery {
        let version = match capture(&self.binary, &["--version"]).await {
            Ok((true, v)) => v.trim().to_string(),
            Ok((false, e)) | Err(e) => {
                return AgentDiscovery::missing(AgentKind::Codex, format!("codex 不可用：{e}"))
            }
        };
        let logged_in = match capture(&self.binary, &["login", "status"]).await {
            Ok((ok, text)) => {
                let t = text.to_lowercase();
                if t.contains("logged in") && !t.contains("not logged in") {
                    Some(true)
                } else if !ok || t.contains("not logged in") {
                    Some(false)
                } else {
                    None
                }
            }
            Err(_) => None,
        };
        let app_server = matches!(
            capture(&self.binary, &["app-server", "--help"]).await,
            Ok((true, _))
        );
        let exec_fallback = matches!(
            capture(&self.binary, &["exec", "--help"]).await,
            Ok((true, _))
        );
        AgentDiscovery {
            kind: AgentKind::Codex,
            found: true,
            binary_path: Some(self.binary.clone()),
            version: Some(version.clone()),
            logged_in,
            protocol_supported: Some(app_server || exec_fallback),
            detail: match (logged_in, app_server, exec_fallback) {
                (Some(true), true, _) => format!("{version}（已登入，app-server 可用）"),
                (Some(true), false, true) => {
                    format!("{version}（已登入，使用受限 exec fallback）")
                }
                (Some(true), false, false) => {
                    format!("{version}（已登入，但 app-server／exec 均不支援）")
                }
                (Some(false), _, _) => {
                    format!("{version}（未登入——請先在終端執行 codex login）")
                }
                (None, _, _) => format!("{version}（登入狀態未知）"),
            },
        }
    }

    async fn start_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSessionHandle>, GatewayError> {
        // `disable_tools`（intent-only：一個 provider 工具都不給）在 codex
        // 沒有任何等價旗標——app-server 的 thread/start・thread/resume 與
        // exec fallback 能設的只有 cwd／approvalPolicy／sandbox，沙箱擋得住
        // 寫入，擋不住讀檔與 shell。收下這個 spec 等於把限制降級成 prompt
        // 文字。宣告了就必須執行得了，執行不了就誠實失敗（runtime 端在
        // gateway_attach 已先擋一次；這裡是連接器自己的最後一道）。
        if spec.disable_tools {
            return Err(GatewayError::Unavailable(
                "codex 無法確定性停用全部工具（app-server 與 exec 都沒有對應旗標）；intent-only session 不支援 codex".into(),
            ));
        }
        let app_server = matches!(
            capture(&self.binary, &["app-server", "--help"]).await,
            Ok((true, _))
        );
        if !app_server {
            let exec_supported = matches!(
                capture(&self.binary, &["exec", "--help"]).await,
                Ok((true, _))
            );
            if exec_supported {
                return crate::codex_exec::start(self.binary.clone(), spec).await;
            }
            return Err(GatewayError::Unavailable(
                "codex 不支援 app-server 或 exec --json".into(),
            ));
        }
        let mut cmd = Command::new(&self.binary);
        remove_runtime_auth_env(&mut cmd);
        apply_session_capability_env(&mut cmd, &spec);
        cmd.current_dir(&spec.workdir).arg("app-server");
        let mut child = spawn_grouped(cmd)?;
        // pgid 必須在 spawn 後立即捕捉：kill 路徑不能依賴 child 屆時是否已被收割。
        let group = ProcessGroup::of(&child);
        let stdout = child.stdout.take().ok_or(GatewayError::Closed)?;
        let stdin = child.stdin.take().ok_or(GatewayError::Closed)?;
        // stderr：診斷輸出，吞掉避免管線塞住。
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(_)) = lines.next_line().await {}
            });
        }

        let (event_tx, event_rx) = mpsc::channel::<GatewayEvent>(EVENT_CHANNEL_CAP);
        let (out_tx, mut out_rx) = mpsc::channel::<OutLine>(64);
        let shared = Arc::new(CodexShared {
            out_tx: out_tx.clone(),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            thread_id: Mutex::new(None),
            current_turn: Mutex::new(None),
            last_agent_message: Mutex::new(None),
            approvals: Mutex::new(HashMap::new()),
        });

        // writer task：寫入＋flush 之後才回報 ack（送達證明）。寫入失敗
        // 就結束，佇列裡等 ack 的呼叫端會因 sender 被丟棄而立刻收到失敗，
        // 不會有人無限等待。
        {
            let mut stdin = stdin;
            tokio::spawn(async move {
                while let Some(OutLine { line, ack }) = out_rx.recv().await {
                    let wrote = async {
                        stdin.write_all(line.as_bytes()).await?;
                        stdin.write_all(b"\n").await?;
                        stdin.flush().await
                    }
                    .await
                    .is_ok();
                    if let Some(ack) = ack {
                        let _ = ack.send(wrote);
                    }
                    if !wrote {
                        break;
                    }
                }
            });
        }

        // reader task：responses → pending；notifications／requests → 正規化。
        {
            let shared = shared.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                // 進行中 agent 訊息的彙整緩衝（delta 太吵，完成時一次進度）。
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else {
                        let _ = event_tx
                            .send(GatewayEvent::Unparsed {
                                raw: line.chars().take(300).collect(),
                            })
                            .await;
                        continue;
                    };
                    let has_id = v.get("id").is_some();
                    let method = v.get("method").and_then(|m| m.as_str());
                    match (has_id, method) {
                        (true, None) => {
                            // response
                            if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                                if let Some(txr) =
                                    shared.pending.lock().expect("pending lock").remove(&id)
                                {
                                    let _ = txr.send(v.get("result").cloned().unwrap_or_else(
                                        || json!({"error": v.get("error").cloned()}),
                                    ));
                                }
                            }
                        }
                        (true, Some(m)) => {
                            // ServerRequest（approval 等）：登記，等待人類裁決。
                            let rid = v.get("id").cloned().unwrap_or(Value::Null);
                            let key = rid.to_string();
                            shared
                                .approvals
                                .lock()
                                .expect("approvals lock")
                                .insert(key.clone(), (rid, m.to_string()));
                            let summary = approval_summary(m, v.get("params"));
                            let _ = event_tx
                                .send(GatewayEvent::TaskWaitingForConsent {
                                    request_id: key,
                                    summary,
                                })
                                .await;
                        }
                        (false, Some(m)) => {
                            for ev in normalize_codex_notification(m, v.get("params"), &shared) {
                                let _ = event_tx.send(ev).await;
                            }
                        }
                        _ => {
                            let _ = event_tx
                                .send(GatewayEvent::Unparsed {
                                    raw: line.chars().take(300).collect(),
                                })
                                .await;
                        }
                    }
                }
                let resumable = shared.thread_id.lock().expect("tid lock").is_some();
                let _ = event_tx
                    .send(GatewayEvent::SessionClosed {
                        resumable,
                        detail: None,
                    })
                    .await;
            });
        }

        // 握手＋開 thread。
        let init = rpc_request(
            &shared,
            "initialize",
            json!({"clientInfo": {"name": "adaptive-interaction", "version": env!("CARGO_PKG_VERSION")}}),
        )
        .await?;
        if init.get("error").map(|e| !e.is_null()).unwrap_or(false) {
            kill_tree(&mut child, &group, 500).await;
            return Err(GatewayError::Protocol(format!("initialize failed: {init}")));
        }
        let _ = out_tx
            .send(OutLine {
                line: json!({"jsonrpc": "2.0", "method": "initialized"}).to_string(),
                ack: None,
            })
            .await;

        // 續開**不是**繼承：舊 thread 當初開在哪個 cwd、拿到什麼 sandbox，
        // 與這次的 SessionSpec 無關。`thread/resume` 的參數 schema
        // （`app-server generate-json-schema` → ThreadResumeParams：threadId
        // 必填，cwd／approvalPolicy／sandbox 皆為可選）跟 `thread/start`
        // 一樣收得下這三個旗標，所以兩條路徑都重送同一份限制——resume 少送
        // 一個 sandbox，就等於讓舊 session 的寫入權跟著 thread id 復活。
        let sandbox = if spec.write_enabled {
            "workspace-write"
        } else {
            "read-only"
        };
        let cwd = spec.workdir.to_string_lossy().into_owned();
        let (method, thread) = if let Some(resume) = &spec.resume_provider_session {
            let params = json!({
                "threadId": resume,
                "cwd": cwd,
                "approvalPolicy": "untrusted",
                "sandbox": sandbox,
            });
            (
                "thread/resume",
                rpc_request(&shared, "thread/resume", params).await?,
            )
        } else {
            let params = json!({
                "cwd": cwd,
                "approvalPolicy": "untrusted",
                "ephemeral": false,
                "sandbox": sandbox,
            });
            (
                "thread/start",
                rpc_request(&shared, "thread/start", params).await?,
            )
        };
        let thread_id = thread
            .pointer("/thread/id")
            .or_else(|| thread.get("threadId"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string());
        let Some(tid) = thread_id else {
            kill_tree(&mut child, &group, 500).await;
            // resume 帶著重新上鎖的旗標被拒絕時，寧可整條連線誠實失敗，
            // 也不退回只送 threadId 的舊行為（那會是沒有 sandbox 的續開）。
            return Err(GatewayError::Protocol(format!(
                "{method} gave no thread id: {thread}"
            )));
        };
        *shared.thread_id.lock().expect("tid lock") = Some(tid.clone());
        let _ = event_tx
            .send(GatewayEvent::SessionStarted {
                provider_session_id: tid,
            })
            .await;

        let mut handle = CodexHandle {
            child,
            group,
            shared,
            events: Some(event_rx),
        };
        if let Some(prompt) = &spec.prompt {
            handle.send_user_message(prompt).await?;
        }
        Ok(Box::new(handle))
    }
}

struct CodexShared {
    out_tx: mpsc::Sender<OutLine>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    next_id: AtomicU64,
    thread_id: Mutex<Option<String>>,
    current_turn: Mutex<Option<String>>,
    /// 最後一則 agentMessage 全文（turn/completed 的聲稱摘要來源）。
    last_agent_message: Mutex<Option<String>>,
    /// request_id(字串) → (原始 id JSON, method)。
    approvals: Mutex<HashMap<String, (Value, String)>>,
}

/// 送出一行並等到它**真的寫進子程序 stdin＋flush**。排進佇列不算送達：
/// 任何以「已送達」為前提的誠實回報（delivered 戳記、fetched 事件、人類
/// approval 裁決）都必須走這條路徑。
async fn send_line_acked(shared: &Arc<CodexShared>, line: String) -> Result<(), GatewayError> {
    let (ack_tx, ack_rx) = oneshot::channel();
    shared
        .out_tx
        .send(OutLine {
            line,
            ack: Some(ack_tx),
        })
        .await
        .map_err(|_| GatewayError::Closed)?;
    match tokio::time::timeout(WRITE_ACK_TIMEOUT, ack_rx).await {
        Ok(Ok(true)) => Ok(()),
        // writer 回報寫入失敗，或 writer 已結束（sender 被丟棄）。
        Ok(Ok(false)) | Ok(Err(_)) => Err(GatewayError::Closed),
        Err(_) => Err(GatewayError::Protocol(
            "agent 子程序未讀取 stdin（寫入逾時）".into(),
        )),
    }
}

async fn rpc_request(
    shared: &Arc<CodexShared>,
    method: &str,
    params: Value,
) -> Result<Value, GatewayError> {
    let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = oneshot::channel();
    shared.pending.lock().expect("pending lock").insert(id, tx);
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    // 寫不進去就別等 30 秒的 RPC timeout：立刻誠實失敗，並收回登記。
    if let Err(e) = send_line_acked(shared, line).await {
        shared.pending.lock().expect("pending lock").remove(&id);
        return Err(e);
    }
    tokio::time::timeout(RPC_TIMEOUT, rx)
        .await
        .map_err(|_| GatewayError::Protocol(format!("{method} timed out")))?
        .map_err(|_| GatewayError::Closed)
}

fn approval_summary(method: &str, params: Option<&Value>) -> String {
    let detail = params
        .and_then(|p| {
            p.pointer("/command")
                .or_else(|| p.pointer("/cmd"))
                .or_else(|| p.pointer("/path"))
                .or_else(|| p.pointer("/reason"))
        })
        .map(|v| v.to_string())
        .unwrap_or_default();
    format!("codex 請求核可：{method} {detail}")
        .chars()
        .take(300)
        .collect()
}

/// 純函式（除了 turn id 記錄）：codex 通知 → 正規化事件。
fn normalize_codex_notification(
    method: &str,
    params: Option<&Value>,
    shared: &Arc<CodexShared>,
) -> Vec<GatewayEvent> {
    match method {
        "turn/started" => {
            if let Some(turn_id) = params
                .and_then(|p| p.pointer("/turn/id").or_else(|| p.get("turnId")))
                .and_then(|t| t.as_str())
            {
                *shared.current_turn.lock().expect("turn lock") = Some(turn_id.to_string());
            }
            vec![GatewayEvent::TaskAccepted]
        }
        "item/started" => {
            let kind = params
                .and_then(|p| p.pointer("/item/type"))
                .and_then(|t| t.as_str())
                .unwrap_or("item");
            match kind {
                "commandExecution" | "mcpToolCall" | "fileChange" | "toolCall" => {
                    vec![GatewayEvent::ToolStarted {
                        name: kind.to_string(),
                    }]
                }
                _ => vec![],
            }
        }
        "item/completed" => {
            let item = params.and_then(|p| p.get("item"));
            let kind = item
                .and_then(|i| i.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            match kind {
                "agentMessage" => {
                    let text = item
                        .and_then(|i| i.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    if !text.is_empty() {
                        *shared.last_agent_message.lock().expect("lam lock") =
                            Some(text.chars().take(4000).collect());
                    }
                    vec![GatewayEvent::TaskProgress {
                        text: if text.is_empty() {
                            None
                        } else {
                            Some(text.chars().take(2000).collect())
                        },
                    }]
                }
                "commandExecution" | "mcpToolCall" | "fileChange" | "toolCall" => {
                    vec![GatewayEvent::ToolCompleted {
                        name: kind.to_string(),
                    }]
                }
                _ => vec![],
            }
        }
        "turn/completed" => {
            *shared.current_turn.lock().expect("turn lock") = None;
            // 聲稱摘要＝最後一則 agentMessage（turn/completed 本身不帶內容）。
            let summary = shared.last_agent_message.lock().expect("lam lock").take();
            // 協定（app-server generate-json-schema）：turn 的每一種結局
            // ——完成、被 turn/interrupt 中斷、失敗——都只透過 turn/completed
            // 通知，用 `turn.status`（completed／interrupted／failed／
            // inProgress）區分；ServerNotification 裡沒有 turn/failed。
            // 誠實階梯：中斷是 cancelled、失敗是 failed，只有 status=
            // completed 才是（agent 的）聲稱；讀不到 status 就是結果未知。
            let status = params
                .and_then(|p| p.pointer("/turn/status"))
                .and_then(Value::as_str);
            match status {
                Some("completed") => vec![GatewayEvent::TaskClaimedCompleted {
                    summary,
                    cost_usd: None,
                    num_turns: None,
                }],
                Some("interrupted") => vec![GatewayEvent::TaskCancelled],
                Some("failed") => {
                    let error = params
                        .and_then(|p| p.pointer("/turn/error/message"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            params
                                .and_then(|p| p.pointer("/turn/error"))
                                .filter(|e| !e.is_null())
                                .map(Value::to_string)
                        })
                        .unwrap_or_else(|| "codex turn failed（未附錯誤訊息）".into());
                    vec![GatewayEvent::TaskFailed {
                        error: error.chars().take(500).collect(),
                    }]
                }
                other => vec![GatewayEvent::TaskOutcomeUnknown {
                    detail: Some(match other {
                        Some(s) => format!("turn/completed 帶了無法解讀的 turn.status={s:?}"),
                        None => "turn/completed 沒有帶 turn.status（協定要求必填）".into(),
                    }),
                }],
            }
        }
        // `error` 是 app-server 的通用錯誤通知；協定裡沒有 turn/failed
        // （turn 失敗走 turn/completed + status=failed，見上）。
        "error" => vec![GatewayEvent::TaskFailed {
            error: params
                .map(|p| p.to_string())
                .unwrap_or_else(|| method.to_string())
                .chars()
                .take(500)
                .collect(),
        }],
        "thread/tokenUsage/updated" => {
            // 形狀鎖定 0.149.1 schema（ThreadTokenUsageUpdatedNotification）：
            // params.tokenUsage.{total,last}.totalTokens。codex 只回報 token
            // 數、沒有 USD 成本；讀不到的欄位誠實回 None，不猜、不換算。
            let usage = params.and_then(|p| p.get("tokenUsage"));
            vec![GatewayEvent::TokenUsage {
                total_tokens: usage
                    .and_then(|u| u.pointer("/total/totalTokens"))
                    .and_then(|t| t.as_u64()),
                last_turn_tokens: usage
                    .and_then(|u| u.pointer("/last/totalTokens"))
                    .and_then(|t| t.as_u64()),
            }]
        }
        "thread/status/changed"
        | "item/agentMessage/delta"
        | "item/reasoning/textDelta"
        | "item/reasoning/summaryTextDelta"
        | "item/commandExecution/outputDelta"
        | "thread/started"
        | "account/rateLimits/updated" => {
            vec![] // 高頻／重複資訊：進度已由 item/completed 傳遞
        }
        _ => vec![],
    }
}

pub struct CodexHandle {
    child: Child,
    group: ProcessGroup,
    shared: Arc<CodexShared>,
    events: Option<mpsc::Receiver<GatewayEvent>>,
}

#[async_trait::async_trait]
impl AgentSessionHandle for CodexHandle {
    fn provider_session_id(&self) -> Option<String> {
        self.shared.thread_id.lock().expect("tid lock").clone()
    }

    async fn send_user_message(&mut self, text: &str) -> Result<(), GatewayError> {
        let tid = self
            .provider_session_id()
            .ok_or_else(|| GatewayError::Protocol("no thread".into()))?;
        // turn/start 的 response 在 turn 完成時才回；不能等它（會塞住 30s
        // RPC timeout），進度靠通知流。但「送出」必須是真的送出：等 writer
        // 寫進 stdin＋flush 才回 Ok——排進佇列不是送達，呼叫端會據此蓋
        // delivered 戳記並發 fetched 事件。
        let id = self.shared.next_id.fetch_add(1, Ordering::SeqCst);
        let line = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "turn/start",
            "params": {"threadId": tid, "input": [{"type": "text", "text": text}]},
        })
        .to_string();
        send_line_acked(&self.shared, line).await
    }

    async fn resolve_approval(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), GatewayError> {
        // 先讀、不先刪：裁決寫進 stdin 失敗時請求必須還在，人類（或看門狗）
        // 才能重試；一送就刪會讓失敗的裁決永遠變成「unknown approval request」。
        let Some((rid, _method)) = self
            .shared
            .approvals
            .lock()
            .expect("approvals lock")
            .get(request_id)
            .cloned()
        else {
            return Err(GatewayError::Protocol(format!(
                "unknown approval request {request_id}"
            )));
        };
        let decision_str = match decision {
            ApprovalDecision::Approve => "accept",
            ApprovalDecision::Deny => "reject",
        };
        let line = json!({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {"decision": decision_str},
        })
        .to_string();
        // 人類裁決必須真的送到 agent 才算數（尤其是 deny）。
        send_line_acked(&self.shared, line).await?;
        self.shared
            .approvals
            .lock()
            .expect("approvals lock")
            .remove(request_id);
        Ok(())
    }

    async fn interrupt(&mut self) -> Result<(), GatewayError> {
        let tid = self.provider_session_id();
        let turn = self.shared.current_turn.lock().expect("turn lock").clone();
        if let (Some(tid), Some(turn_id)) = (tid, turn) {
            let id = self.shared.next_id.fetch_add(1, Ordering::SeqCst);
            let line = json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "turn/interrupt",
                "params": {"threadId": tid, "turnId": turn_id},
            })
            .to_string();
            // 協定中斷寫不進去（子程序卡死）就退回訊號中斷，不假裝成功。
            if send_line_acked(&self.shared, line).await.is_err() {
                interrupt_tree(&self.child);
            }
        } else {
            interrupt_tree(&self.child);
        }
        Ok(())
    }

    async fn kill(&mut self) -> Result<(), GatewayError> {
        kill_tree(&mut self.child, &self.group, 1500).await;
        Ok(())
    }

    fn process_group(&self) -> ProcessGroup {
        self.group.clone()
    }

    fn take_events(&mut self) -> Option<mpsc::Receiver<GatewayEvent>> {
        self.events.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Arc<CodexShared> {
        let (tx, _rx) = mpsc::channel(4);
        Arc::new(CodexShared {
            out_tx: tx,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            thread_id: Mutex::new(None),
            current_turn: Mutex::new(None),
            last_agent_message: Mutex::new(None),
            approvals: Mutex::new(HashMap::new()),
        })
    }

    #[test]
    fn notifications_normalize_honestly() {
        let s = shared();
        // turn/started 記下 turn id 並回 accepted。
        let ev = normalize_codex_notification(
            "turn/started",
            Some(&serde_json::json!({"turn": {"id": "turn-1"}})),
            &s,
        );
        assert_eq!(ev, vec![GatewayEvent::TaskAccepted]);
        assert_eq!(s.current_turn.lock().unwrap().as_deref(), Some("turn-1"));

        // agentMessage 完成 → 進度（不是 completed！）。
        let ev = normalize_codex_notification(
            "item/completed",
            Some(&serde_json::json!({"item": {"type": "agentMessage", "text": "看完了"}})),
            &s,
        );
        assert_eq!(
            ev,
            vec![GatewayEvent::TaskProgress {
                text: Some("看完了".into())
            }]
        );

        // turn/completed（status=completed）→ 聲稱完成（claim，非驗證）。
        let ev = normalize_codex_notification(
            "turn/completed",
            Some(&serde_json::json!({
                "threadId": "t",
                "turn": {"id": "turn-1", "items": [], "status": "completed"}
            })),
            &s,
        );
        assert!(matches!(ev[0], GatewayEvent::TaskClaimedCompleted { .. }));
        assert!(s.current_turn.lock().unwrap().is_none());

        // 高頻 delta 靜默。
        assert!(normalize_codex_notification("item/agentMessage/delta", None, &s).is_empty());
    }

    /// regression（agent-honesty）：turn/completed 曾無條件翻成 claimed-
    /// completed。協定裡 turn 的每種結局都只走 turn/completed，用
    /// turn.status 區分：interrupted（使用者按了中斷）必須是 cancelled、
    /// failed 必須是 failed（帶 turn.error.message），缺 status 是結果未知
    /// ——三者都絕不可變成「Agent 說做完了」。
    #[test]
    fn turn_completed_reads_turn_status_instead_of_assuming_a_claim() {
        let s = shared();
        let interrupted = normalize_codex_notification(
            "turn/completed",
            Some(&serde_json::json!({
                "threadId": "t",
                "turn": {"id": "turn-1", "items": [], "status": "interrupted"}
            })),
            &s,
        );
        assert_eq!(interrupted, vec![GatewayEvent::TaskCancelled]);

        let failed = normalize_codex_notification(
            "turn/completed",
            Some(&serde_json::json!({
                "threadId": "t",
                "turn": {
                    "id": "turn-2",
                    "items": [],
                    "status": "failed",
                    "error": {"message": "boom: sandbox denied"}
                }
            })),
            &s,
        );
        match &failed[..] {
            [GatewayEvent::TaskFailed { error }] => assert!(error.contains("boom"), "{error}"),
            other => panic!("status=failed must be TaskFailed, got {other:?}"),
        }

        // 缺 status（協定要求必填）→ 結果未知，不是聲稱。
        let missing = normalize_codex_notification(
            "turn/completed",
            Some(&serde_json::json!({"threadId": "t", "turn": {"id": "turn-3", "items": []}})),
            &s,
        );
        assert!(
            matches!(missing[..], [GatewayEvent::TaskOutcomeUnknown { .. }]),
            "missing turn.status must be unknown, got {missing:?}"
        );
        let none = normalize_codex_notification("turn/completed", None, &s);
        assert!(
            matches!(none[..], [GatewayEvent::TaskOutcomeUnknown { .. }]),
            "turn/completed without params must be unknown, got {none:?}"
        );
        // inProgress 對「turn 結束」而言自相矛盾 → 同樣未知。
        let odd = normalize_codex_notification(
            "turn/completed",
            Some(&serde_json::json!({
                "threadId": "t",
                "turn": {"id": "turn-4", "items": [], "status": "inProgress"}
            })),
            &s,
        );
        assert!(matches!(odd[..], [GatewayEvent::TaskOutcomeUnknown { .. }]));

        for events in [&interrupted, &failed, &missing, &none, &odd] {
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, GatewayEvent::TaskClaimedCompleted { .. })),
                "never a claim: {events:?}"
            );
        }
    }

    /// regression：resolve_approval 曾「先刪登記再送」——寫入 stdin 失敗後
    /// 請求就從連接器消失，人類重試只會得到 unknown approval request。
    #[tokio::test]
    async fn a_failed_approval_write_keeps_the_request_so_it_can_be_retried() {
        let s = shared();
        s.approvals.lock().unwrap().insert(
            "9001".into(),
            (
                serde_json::json!(9001),
                "item/commandExecution/requestApproval".into(),
            ),
        );
        // out_tx 的接收端在 shared() 裡已被丟棄 ⇒ 每次寫入都會 Closed。
        let mut handle = CodexHandle {
            child: Command::new("true").spawn().unwrap(),
            group: ProcessGroup::empty(),
            shared: s.clone(),
            events: None,
        };
        let err = handle
            .resolve_approval("9001", ApprovalDecision::Deny)
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::Closed), "{err:?}");
        assert!(
            s.approvals.lock().unwrap().contains_key("9001"),
            "an undelivered decision must leave the request registered"
        );
    }

    /// regression（token 用量通知曾被整個丟棄，codex 用量完全不可見）。
    #[test]
    fn token_usage_notifications_normalize_without_pretending_usd() {
        let s = shared();
        let ev = normalize_codex_notification(
            "thread/tokenUsage/updated",
            Some(&serde_json::json!({
                "threadId": "t-1",
                "turnId": "u-1",
                "tokenUsage": {
                    "total": {"totalTokens": 1234, "inputTokens": 1000, "outputTokens": 200,
                              "cachedInputTokens": 30, "reasoningOutputTokens": 4},
                    "last": {"totalTokens": 56, "inputTokens": 40, "outputTokens": 15,
                             "cachedInputTokens": 0, "reasoningOutputTokens": 1},
                },
            })),
            &s,
        );
        assert_eq!(
            ev,
            vec![GatewayEvent::TokenUsage {
                total_tokens: Some(1234),
                last_turn_tokens: Some(56),
            }]
        );
        // 形狀對不上 → 誠實 None，不猜測數字。
        let ev = normalize_codex_notification("thread/tokenUsage/updated", None, &s);
        assert_eq!(
            ev,
            vec![GatewayEvent::TokenUsage {
                total_tokens: None,
                last_turn_tokens: None,
            }]
        );
    }

    #[test]
    fn approval_summary_is_bounded_and_never_panics() {
        let long = "x".repeat(10_000);
        let s = approval_summary(
            "item/commandExecution/requestApproval",
            Some(&serde_json::json!({"command": long})),
        );
        assert!(s.chars().count() <= 300);
        let _ = approval_summary("execCommandApproval", None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn connector_uses_exec_fallback_and_resumes_the_same_thread() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-codex.sh");
        let argv_log = dir.path().join("argv.log");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
if [ "$1" = "--version" ]; then echo 'codex-cli 0.1'; exit 0; fi
if [ "$1" = "login" ]; then echo 'Logged in'; exit 0; fi
if [ "$1" = "app-server" ]; then exit 1; fi
if [ "$1" = "exec" ] && [ "$2" = "--help" ]; then exit 0; fi
echo '{{"type":"thread.started","thread_id":"fallback-thread"}}'
echo '{{"type":"turn.started"}}'
echo '{{"type":"item.completed","item":{{"type":"agent_message","text":"fallback-ok"}}}}'
echo '{{"type":"turn.completed","usage":{{"input_tokens":2,"output_tokens":1}}}}'
"#,
                argv_log.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let connector = CodexConnector::with_binary(script.to_string_lossy());
        let discovery = connector.discover().await;
        assert!(discovery.usable(), "{discovery:?}");
        assert!(discovery.detail.contains("exec fallback"));

        let mut handle = connector
            .start_session(SessionSpec::read_only_in(dir.path().to_path_buf()))
            .await
            .unwrap();
        let mut events = handle.take_events().unwrap();
        handle.send_user_message("first").await.unwrap();
        let mut saw_claim = false;
        for _ in 0..10 {
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(event, GatewayEvent::TaskClaimedCompleted { .. }) {
                saw_claim = true;
                break;
            }
        }
        assert!(saw_claim);
        assert_eq!(
            handle.provider_session_id().as_deref(),
            Some("fallback-thread")
        );

        // reader 在 claim 後還要 wait/reap；有界等待 busy 清除，再送第二輪。
        for _ in 0..40 {
            match handle.send_user_message("second").await {
                Ok(()) => break,
                Err(GatewayError::Busy(_)) => tokio::time::sleep(Duration::from_millis(25)).await,
                Err(e) => panic!("second turn failed: {e}"),
            }
        }
        let mut second_claim = false;
        for _ in 0..10 {
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(event, GatewayEvent::TaskClaimedCompleted { .. }) {
                second_claim = true;
                break;
            }
        }
        assert!(second_claim);
        let log = std::fs::read_to_string(argv_log).unwrap();
        assert!(log.contains("exec --json"), "{log}");
        assert!(log.contains("exec resume"), "{log}");
        assert!(log.contains("fallback-thread second"), "{log}");
        assert!(!log.contains("danger"), "{log}");
        handle.kill().await.unwrap();
    }
}
