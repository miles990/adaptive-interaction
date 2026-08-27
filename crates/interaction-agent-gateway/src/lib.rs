//! Agent Gateway：連接本機已安裝、已登入的 AI Agent（Codex／Claude Code），
//! 把各家原始事件正規化為統一的 GatewayEvent（spec §8.3）。
//!
//! 邊界（spec §8）：
//! - 不讀取、不複製、不保存任何 Agent 的 credential——登入由各 agent 自管。
//! - 第一版預設唯讀／Plan（Codex sandbox=read-only、Claude permission-mode=plan）。
//! - claimed-completed 是 agent 的**聲稱**，不是驗證；正規化層不升級任何狀態。
//! - 取消／estop 必須終止整個子程序樹；子程序絕不跨 runtime 重啟存活。

pub mod claude;
pub mod codex;
pub mod process;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    Codex,
    ClaudeCode,
}

impl AgentKind {
    pub fn agent_id(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
        }
    }
    pub fn provider_id(&self) -> &'static str {
        match self {
            Self::Codex => "provider.ai-agent.codex",
            Self::ClaudeCode => "provider.ai-agent.claude-code",
        }
    }
}

/// 發現結果：只讀取版本與登入狀態，不碰 credential 本體。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDiscovery {
    pub kind: AgentKind,
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// true / false / unknown（誠實三態：查不到就說不知道）。
    pub logged_in: Option<bool>,
    /// app-server（codex）或 stream-json（claude）是否可用。
    pub protocol_supported: Option<bool>,
    pub detail: String,
}

impl AgentDiscovery {
    pub fn missing(kind: AgentKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            found: false,
            binary_path: None,
            version: None,
            logged_in: None,
            protocol_supported: None,
            detail: detail.into(),
        }
    }
    /// 可以建立 session 的最低條件（登入未知時誠實視為不可用）。
    pub fn usable(&self) -> bool {
        self.found && self.logged_in == Some(true) && self.protocol_supported != Some(false)
    }
}

/// 正規化事件（spec §8.3 命名）。UI 不直接暴露各家原始事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "event")]
pub enum GatewayEvent {
    SessionStarted {
        /// Provider 端 session／thread id（進階詳情用；不是 runtime session id）。
        provider_session_id: String,
    },
    TaskAccepted,
    TaskProgress {
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    TaskWaitingForInput,
    TaskWaitingForConsent {
        request_id: String,
        summary: String,
    },
    ToolStarted {
        name: String,
    },
    ToolCompleted {
        name: String,
    },
    ArtifactProduced {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// Agent 聲稱完成——**不是**驗證。
    TaskClaimedCompleted {
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        num_turns: Option<u64>,
    },
    TaskFailed {
        error: String,
    },
    TaskCancelled,
    SessionClosed {
        /// 可否以 provider session id 續開（--resume / thread/resume）。
        resumable: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// 解析不了的原始行（誠實保留，不猜測語意；進階詳情可見）。
    Unparsed {
        raw: String,
    },
}

/// 建立 session 的規格（第一版：唯讀／Plan 預設，不可由 AI 自行放寬）。
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub workdir: PathBuf,
    pub prompt: Option<String>,
    /// 唯讀模式（預設 true；放寬需要人類在 runtime 端明確同意）。
    pub read_only: bool,
    pub model: Option<String>,
    /// 續開既有 provider session（claude --resume / codex thread/resume）。
    pub resume_provider_session: Option<String>,
    pub max_turns: Option<u32>,
}

impl SessionSpec {
    pub fn read_only_in(workdir: PathBuf) -> Self {
        Self {
            workdir,
            prompt: None,
            read_only: true,
            model: None,
            resume_provider_session: None,
            max_turns: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("agent not available: {0}")]
    Unavailable(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("session closed")]
    Closed,
}

/// 人類對 agent approval 請求的裁決（經 runtime 的 assist 流程）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

/// 一條進行中的 agent 子程序 session。
#[async_trait::async_trait]
pub trait AgentSessionHandle: Send {
    /// Provider 端 session id（一旦得知）。
    fn provider_session_id(&self) -> Option<String>;
    /// 送一則使用者／runtime 訊息進 agent。
    async fn send_user_message(&mut self, text: &str) -> Result<(), GatewayError>;
    /// 回覆一個等待中的 approval 請求。
    async fn resolve_approval(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), GatewayError>;
    /// 中斷目前 turn（不殺 session）。
    async fn interrupt(&mut self) -> Result<(), GatewayError>;
    /// 終止整個子程序樹（estop／close／lease 到期）。
    async fn kill(&mut self) -> Result<(), GatewayError>;
    /// 事件接收端（每個 handle 只能取一次）。
    fn take_events(&mut self) -> Option<tokio::sync::mpsc::Receiver<GatewayEvent>>;
}

#[async_trait::async_trait]
pub trait AgentConnector: Send + Sync {
    fn kind(&self) -> AgentKind;
    /// 發現：版本／登入／協定支援。絕不啟動互動式登入。
    async fn discover(&self) -> AgentDiscovery;
    async fn start_session(
        &self,
        spec: SessionSpec,
    ) -> Result<Box<dyn AgentSessionHandle>, GatewayError>;
}

/// 已知 connectors 的註冊表。
pub fn default_connectors() -> Vec<Box<dyn AgentConnector>> {
    vec![
        Box::new(claude::ClaudeConnector::default()),
        Box::new(codex::CodexConnector::default()),
    ]
}
