//! Codex exec fallback（spec §8.1）：`app-server` 不可用的舊版 Codex 走
//! `codex exec --json`，後續 turn 走 `codex exec resume <thread-id>`。
//!
//! 這條路徑沒有互動式 approval channel，因此固定 `approval_policy=never`；
//! 唯讀 session 用 `read-only`，人類明確建立的限權寫入 session 才用
//! `workspace-write`。絕不使用 danger-full-access、approve-for-me 或 bypass。

use crate::process::{
    apply_session_capability_env, remove_runtime_auth_env, spawn_grouped, ProcessGroup,
};
use crate::{AgentSessionHandle, ApprovalDecision, GatewayError, GatewayEvent, SessionSpec};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

const EVENT_CHANNEL_CAP: usize = 256;

/// 建立 fallback handle。此時不啟動 agent；第一則訊息才生成一個有界子程序。
pub async fn start(
    binary: String,
    spec: SessionSpec,
) -> Result<Box<dyn AgentSessionHandle>, GatewayError> {
    if !spec.workdir.is_dir() {
        return Err(GatewayError::Unavailable(format!(
            "workdir 不存在：{}",
            spec.workdir.display()
        )));
    }
    let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAP);
    let session_id = Arc::new(Mutex::new(spec.resume_provider_session.clone()));
    if let Some(provider_session_id) = &spec.resume_provider_session {
        let _ = event_tx
            .send(GatewayEvent::SessionStarted {
                provider_session_id: provider_session_id.clone(),
            })
            .await;
    }
    let mut handle = CodexExecHandle {
        binary,
        spec,
        events_tx: event_tx,
        events: Some(event_rx),
        session_id,
        group: ProcessGroup::empty(),
        busy: Arc::new(AtomicBool::new(false)),
        closed: Arc::new(AtomicBool::new(false)),
        cancel_requested: Arc::new(AtomicBool::new(false)),
    };
    if let Some(prompt) = handle.spec.prompt.clone() {
        handle.send_user_message(&prompt).await?;
    }
    Ok(Box::new(handle))
}

/// 純函式：鎖定 `codex exec` 的安全參數。`resume` 也重新覆寫 sandbox，
/// 防止外部提供的 thread id 帶入較寬權限。`--ignore-user-config` 避免使用者
/// 設定中的 MCP／hook／額外 writable roots 偷渡進受限 session；認證仍由
/// Codex 自己的 CODEX_HOME 管理。
pub fn exec_args(spec: &SessionSpec, prompt: &str, resume: Option<&str>) -> Vec<String> {
    let sandbox = if spec.write_enabled {
        "workspace-write"
    } else {
        "read-only"
    };
    let mut args = vec!["exec".to_string()];
    if let Some(thread_id) = resume {
        args.push("resume".into());
        args.extend([
            "--json".into(),
            "--skip-git-repo-check".into(),
            "--ignore-user-config".into(),
            "-c".into(),
            format!("sandbox_mode=\"{sandbox}\""),
            "-c".into(),
            "approval_policy=\"never\"".into(),
        ]);
        if let Some(model) = &spec.model {
            args.extend(["--model".into(), model.clone()]);
        }
        args.extend([thread_id.to_string(), prompt.to_string()]);
    } else {
        args.extend([
            "--json".into(),
            "--color".into(),
            "never".into(),
            "--sandbox".into(),
            sandbox.into(),
            "--cd".into(),
            spec.workdir.to_string_lossy().into_owned(),
            "--skip-git-repo-check".into(),
            "--ignore-user-config".into(),
            "-c".into(),
            "approval_policy=\"never\"".into(),
        ]);
        if let Some(model) = &spec.model {
            args.extend(["--model".into(), model.clone()]);
        }
        args.push(prompt.to_string());
    }
    args
}

pub struct CodexExecHandle {
    binary: String,
    spec: SessionSpec,
    events_tx: mpsc::Sender<GatewayEvent>,
    events: Option<mpsc::Receiver<GatewayEvent>>,
    session_id: Arc<Mutex<Option<String>>>,
    group: ProcessGroup,
    busy: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    cancel_requested: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl AgentSessionHandle for CodexExecHandle {
    fn provider_session_id(&self) -> Option<String> {
        self.session_id.lock().expect("sid lock").clone()
    }

    async fn send_user_message(&mut self, text: &str) -> Result<(), GatewayError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(GatewayError::Closed);
        }
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(GatewayError::Busy(
                "上一輪 codex exec 尚未結束；請先中斷或等待".into(),
            ));
        }

        let resume = self.provider_session_id();
        let mut cmd = Command::new(&self.binary);
        remove_runtime_auth_env(&mut cmd);
        apply_session_capability_env(&mut cmd, &self.spec);
        cmd.current_dir(&self.spec.workdir)
            .args(exec_args(&self.spec, text, resume.as_deref()));
        let mut child = match spawn_grouped(cmd) {
            Ok(child) => child,
            Err(e) => {
                self.busy.store(false, Ordering::SeqCst);
                return Err(e.into());
            }
        };
        self.group.set_from_child(&child);
        let spawned_pgid = self.group.pgid();
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                self.busy.store(false, Ordering::SeqCst);
                return Err(GatewayError::Closed);
            }
        };
        let stderr = child.stderr.take();
        // prompt 已作為 argv 傳入；關閉 pipe，避免子程序誤等額外 stdin。
        child.stdin.take();

        let tx = self.events_tx.clone();
        let sid = self.session_id.clone();
        let busy = self.busy.clone();
        let group = self.group.clone();
        let cancel_requested = self.cancel_requested.clone();
        let closed = self.closed.clone();
        tokio::spawn(async move {
            let stderr_tail = Arc::new(Mutex::new(String::new()));
            let stderr_task = stderr.map(|stderr| {
                let tail = stderr_tail.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let mut t = tail.lock().expect("stderr tail lock");
                        t.push_str(&line);
                        t.push('\n');
                        let len = t.len();
                        if len > 2000 {
                            t.drain(..len - 2000);
                        }
                    }
                })
            });

            let mut lines = BufReader::new(stdout).lines();
            let mut saw_terminal = false;
            while let Ok(Some(line)) = lines.next_line().await {
                for ev in parse_exec_line(&line) {
                    if let GatewayEvent::SessionStarted {
                        provider_session_id,
                    } = &ev
                    {
                        *sid.lock().expect("sid lock") = Some(provider_session_id.clone());
                    }
                    if matches!(
                        ev,
                        GatewayEvent::TaskClaimedCompleted { .. }
                            | GatewayEvent::TaskFailed { .. }
                            | GatewayEvent::TaskCancelled
                    ) {
                        saw_terminal = true;
                    }
                    if tx.send(ev).await.is_err() {
                        break;
                    }
                }
            }
            let status = child.wait().await.ok();
            if let Some(task) = stderr_task {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), task).await;
            }
            let cancelled =
                cancel_requested.swap(false, Ordering::SeqCst) || closed.load(Ordering::SeqCst);
            let stderr_detail = stderr_tail
                .lock()
                .expect("stderr tail lock")
                .trim()
                .to_string();
            if let Some(event) = drain_outcome_event(
                saw_terminal,
                cancelled,
                ExitObservation::from_status(status),
                &stderr_detail,
            ) {
                let _ = tx.send(event).await;
            }
            group.clear_if(spawned_pgid);
            busy.store(false, Ordering::SeqCst);
        });
        Ok(())
    }

    async fn resolve_approval(
        &mut self,
        _request_id: &str,
        _decision: ApprovalDecision,
    ) -> Result<(), GatewayError> {
        Err(GatewayError::Protocol(
            "codex exec fallback 沒有互動核可管道；所有核可預設拒絕".into(),
        ))
    }

    async fn interrupt(&mut self) -> Result<(), GatewayError> {
        if self.busy.load(Ordering::SeqCst) {
            self.cancel_requested.store(true, Ordering::SeqCst);
            self.group.interrupt();
        }
        Ok(())
    }

    async fn kill(&mut self) -> Result<(), GatewayError> {
        self.closed.store(true, Ordering::SeqCst);
        self.cancel_requested.store(true, Ordering::SeqCst);
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

/// 子程序收攤時**觀察得到**的結束方式。`Signal`／`Unknown` 都不是錯誤證據。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitObservation {
    /// 讀得到 exit code。
    Code(i32),
    /// 有結束狀態但沒有 exit code（被訊號終止）。
    Signal,
    /// 連結束狀態都讀不到（`wait()` 失敗）。
    Unknown,
}

impl ExitObservation {
    fn from_status(status: Option<std::process::ExitStatus>) -> Self {
        match status {
            Some(status) => match status.code() {
                Some(code) => Self::Code(code),
                None => Self::Signal,
            },
            None => Self::Unknown,
        }
    }

    fn note(self) -> String {
        match self {
            Self::Code(code) => format!("exit {code}"),
            Self::Signal => "terminated by signal".into(),
            Self::Unknown => "exit status unknown".into(),
        }
    }
}

/// 子程序在沒有回報結局的情況下收攤時，該送哪一個事件（純函式，可測）。
///
/// 誠實階梯（與 `claude.rs` 同一條規則、與 `interaction_core::AgentSessionState::Unknown`
/// 的定義一致）：
/// - agent 自己已經回報過結局 ⇒ `None`（不覆寫它的回報）。
/// - 人類中斷／關閉 ⇒ `TaskCancelled`。
/// - **只有觀察得到的錯誤**（非零 exit code）才是 `TaskFailed`。
/// - exit 0 卻沒有結果、被訊號終止、或連 exit status 都讀不到 ⇒ 結果**未知**
///   （`TaskOutcomeUnknown`，runtime 記為 `unknown`）。輸出格式漂移
///   （例如 `turn.completed` 改名）屬於這一類，不得記成失敗。
pub(crate) fn drain_outcome_event(
    saw_terminal: bool,
    cancelled: bool,
    exit: ExitObservation,
    stderr_tail: &str,
) -> Option<GatewayEvent> {
    if saw_terminal {
        return None;
    }
    if cancelled {
        return Some(GatewayEvent::TaskCancelled);
    }
    let note = exit.note();
    let tail = stderr_tail.trim();
    let suffix = if tail.is_empty() {
        String::new()
    } else {
        format!("：{tail}")
    };
    match exit {
        ExitObservation::Code(code) if code != 0 => Some(GatewayEvent::TaskFailed {
            error: format!("codex exec 以 {note} 結束而未回報結果{suffix}")
                .chars()
                .take(700)
                .collect(),
        }),
        _ => Some(GatewayEvent::TaskOutcomeUnknown {
            detail: Some(
                format!(
                    "codex exec 在回報結果前結束（{note}）；結果未知，未經人工確認前不得視為成功或失敗{suffix}"
                )
                .chars()
                .take(700)
                .collect(),
            ),
        }),
    }
}

/// 一行 `codex exec --json` JSONL → 正規化事件。未知／畸形輸入保留為
/// bounded Unparsed；不猜測成功。
pub fn parse_exec_line(line: &str) -> Vec<GatewayEvent> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![GatewayEvent::Unparsed {
            raw: line.chars().take(300).collect(),
        }];
    };
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "thread.started" => match v.get("thread_id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => vec![GatewayEvent::SessionStarted {
                provider_session_id: id.to_string(),
            }],
            _ => vec![GatewayEvent::Unparsed {
                raw: "thread.started without thread_id".into(),
            }],
        },
        "turn.started" => vec![GatewayEvent::TaskAccepted],
        "item.started" => normalize_exec_item(v.get("item"), false),
        "item.completed" => normalize_exec_item(v.get("item"), true),
        "turn.completed" => {
            let usage = v.get("usage");
            let total_tokens = usage.map(|u| {
                ["input_tokens", "cached_input_tokens", "output_tokens"]
                    .iter()
                    .filter_map(|k| u.get(k).and_then(Value::as_u64))
                    .sum()
            });
            let mut out = vec![];
            if usage.is_some() {
                out.push(GatewayEvent::TokenUsage {
                    total_tokens,
                    last_turn_tokens: total_tokens,
                });
            }
            out.push(GatewayEvent::TaskClaimedCompleted {
                summary: None,
                cost_usd: None,
                num_turns: Some(1),
            });
            out
        }
        "turn.failed" | "error" => vec![GatewayEvent::TaskFailed {
            error: v
                .pointer("/error/message")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("error"))
                .map(Value::to_string)
                .unwrap_or_else(|| "codex exec 回報失敗".into())
                .chars()
                .take(500)
                .collect(),
        }],
        _ => vec![GatewayEvent::Unparsed {
            raw: line.chars().take(300).collect(),
        }],
    }
}

fn normalize_exec_item(item: Option<&Value>, completed: bool) -> Vec<GatewayEvent> {
    let Some(item) = item else {
        return vec![GatewayEvent::Unparsed {
            raw: "item event without item".into(),
        }];
    };
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("item");
    match kind {
        "agent_message" => {
            if !completed {
                vec![]
            } else {
                vec![GatewayEvent::TaskProgress {
                    text: item
                        .get("text")
                        .and_then(Value::as_str)
                        .map(|s| s.chars().take(2000).collect()),
                }]
            }
        }
        "command_execution" | "file_change" | "tool_call" => {
            if completed {
                let mut out = vec![GatewayEvent::ToolCompleted {
                    name: kind.to_string(),
                }];
                if kind == "file_change" {
                    let path = item
                        .get("changes")
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(|c| c.get("path"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    out.push(GatewayEvent::ArtifactProduced { path });
                }
                out
            } else {
                vec![GatewayEvent::ToolStarted {
                    name: kind.to_string(),
                }]
            }
        }
        // reasoning 等內部事件不當作工作進度；未知型別保留給進階診斷。
        "reasoning" => vec![],
        _ => vec![GatewayEvent::Unparsed {
            raw: item.to_string().chars().take(300).collect(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn args_are_bounded_and_never_bypass_safety() {
        let ro = SessionSpec::read_only_in(PathBuf::from("/tmp/repo"));
        let args = exec_args(&ro, "inspect", None);
        assert!(args.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        assert!(args.iter().any(|a| a == "--ignore-user-config"));
        assert!(args.iter().any(|a| a == "approval_policy=\"never\""));
        assert!(!args.join(" ").contains("danger"));
        assert!(!args.join(" ").contains("approve-for-me"));

        let wr = SessionSpec::write_enabled_in(PathBuf::from("/tmp/repo"));
        let args = exec_args(&wr, "patch", Some("thread-1"));
        assert_eq!(&args[..2], ["exec", "resume"]);
        assert!(args.iter().any(|a| a == "sandbox_mode=\"workspace-write\""));
        assert!(!args.join(" ").contains("danger"));
    }

    #[test]
    fn recorded_jsonl_shapes_normalize_honestly() {
        assert_eq!(
            parse_exec_line(r#"{"type":"thread.started","thread_id":"abc"}"#),
            vec![GatewayEvent::SessionStarted {
                provider_session_id: "abc".into()
            }]
        );
        assert_eq!(
            parse_exec_line(r#"{"type":"turn.started"}"#),
            vec![GatewayEvent::TaskAccepted]
        );
        assert_eq!(
            parse_exec_line(
                r#"{"type":"item.completed","item":{"type":"agent_message","text":"OK"}}"#
            ),
            vec![GatewayEvent::TaskProgress {
                text: Some("OK".into())
            }]
        );
        let done = parse_exec_line(
            r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}"#,
        );
        assert_eq!(
            done[0],
            GatewayEvent::TokenUsage {
                total_tokens: Some(15),
                last_turn_tokens: Some(15)
            }
        );
        assert!(matches!(done[1], GatewayEvent::TaskClaimedCompleted { .. }));
    }

    /// 回歸（agent-honesty-023）：exit 0 但沒有結局事件（例如 codex 輸出格式
    /// 漂移、`turn.completed` 改名）＝結果**未知**，不是失敗。只有觀察得到的
    /// 非零 exit code 才誠實到可以說 failed。
    #[test]
    fn exit_without_terminal_event_is_unknown_unless_exit_code_says_otherwise() {
        // exit 0、沒有結局事件 ⇒ 未知（絕不是 failed）。
        let unknown = drain_outcome_event(false, false, ExitObservation::Code(0), "")
            .expect("收攤時必須回報一個結局");
        assert!(
            matches!(unknown, GatewayEvent::TaskOutcomeUnknown { .. }),
            "exit 0 而沒有結果＝結果未知，不得記為失敗：{unknown:?}"
        );

        // 被訊號終止／連 exit status 都讀不到 ⇒ 一樣是未知。
        for exit in [ExitObservation::Signal, ExitObservation::Unknown] {
            let event = drain_outcome_event(false, false, exit, "boom").expect("有結局");
            assert!(
                matches!(event, GatewayEvent::TaskOutcomeUnknown { .. }),
                "{exit:?} 沒有錯誤證據，必須是未知：{event:?}"
            );
        }

        // 觀察得到的錯誤（非零 exit code）才是 failed，且訊息有界。
        let failed = drain_outcome_event(false, false, ExitObservation::Code(2), &"x".repeat(4000))
            .expect("有結局");
        match failed {
            GatewayEvent::TaskFailed { error } => {
                assert!(error.contains("exit 2"), "{error}");
                assert!(error.chars().count() <= 700, "{}", error.chars().count());
            }
            other => panic!("非零 exit 必須是 failed：{other:?}"),
        }

        // agent 自己回報過結局 ⇒ 不覆寫；人類中斷 ⇒ cancelled。
        assert_eq!(
            drain_outcome_event(true, false, ExitObservation::Code(1), ""),
            None
        );
        assert_eq!(
            drain_outcome_event(false, true, ExitObservation::Code(0), ""),
            Some(GatewayEvent::TaskCancelled)
        );
    }

    #[test]
    fn malformed_lines_are_bounded_and_never_claim_success() {
        for line in ["not json", "{}", r#"{"type":"thread.started"}"#] {
            let events = parse_exec_line(line);
            assert!(!events
                .iter()
                .any(|e| matches!(e, GatewayEvent::TaskClaimedCompleted { .. })));
            assert!(events.iter().all(|e| match e {
                GatewayEvent::Unparsed { raw } => raw.chars().count() <= 300,
                _ => true,
            }));
        }
    }
}
