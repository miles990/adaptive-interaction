//! Command dispatch. Every command talks to the daemon over HTTP — the same
//! application services the API and Tauri use — except `serve`, which hosts
//! the runtime in-process.

use crate::client::{exit_code_for_status, Client, EXIT_CONNECTION};
use crate::{Cli, Command};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

/// 續開要沿用的上限（分鐘／美元／訊息則數）。
struct ResumeLimits {
    ttl_minutes: u32,
    max_cost: f64,
    max_messages: u32,
}

/// 「接續上次」要送出的上限：使用者顯式帶的旗標優先，否則沿用**上一個
/// session 的實際上限**。
///
/// 誠實／最小權限：省略欄位不是「沿用」——後端省略就落到 runtime 預設
/// （120 分鐘、沒有金額上限、訊息吃 policy），每一項都比上次寬，會被
/// `check_resume_not_wider` 拒絕。所以這裡一律把上次的實際值算出來帶過去。
/// 讀不到上次的租期時退回租約長度（issuedAt→expiresAt），再讀不到才用 1
/// 分鐘這個**最窄**的值——不確定就選窄的，不選寬的。
fn resume_limits(
    previous: &Value,
    ttl: Option<u32>,
    max_cost: Option<f64>,
    max_messages: Option<u32>,
) -> ResumeLimits {
    let budget = previous.get("budget");
    let from_budget = budget
        .and_then(|b| b.get("maxDurationMs"))
        .and_then(|v| v.as_u64())
        .filter(|ms| *ms > 0)
        .map(|ms| ((ms as f64) / 60_000.0).round() as u32);
    let from_lease = || {
        let lease = previous.get("lease")?;
        let issued = chrono::DateTime::parse_from_rfc3339(lease.get("issuedAt")?.as_str()?).ok()?;
        let expires =
            chrono::DateTime::parse_from_rfc3339(lease.get("expiresAt")?.as_str()?).ok()?;
        let minutes = (expires - issued).num_seconds() as f64 / 60.0;
        (minutes > 0.0).then(|| minutes.round() as u32)
    };
    ResumeLimits {
        ttl_minutes: ttl.or(from_budget).or_else(from_lease).unwrap_or(1).max(1),
        max_cost: max_cost.unwrap_or_else(|| {
            budget
                .and_then(|b| b.get("maxCost"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        }),
        max_messages: max_messages.unwrap_or_else(|| {
            budget
                .and_then(|b| b.get("maxMessages"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32
        }),
    }
}

/// 續開要掛的資料夾：使用者顯式帶的優先，否則用後端記錄的
/// `resolvedWorkdir`（上一次**真的**掛上子程序的那一個目錄）。
///
/// `dataScope` 裡的 `workspace:` 只是呼叫端自己附加的人話標籤，兩者不一致
/// 時以後端的事實為準；連標籤都沒有就回 None，讓後端誠實拒絕（gateway
/// session 沒有記錄＝無法證明沒有換資料夾），而不是猜一個。
/// 續開的 `toolScope`：intent-only（只有 `conversation.generate`）的工作階段必須原樣帶回，
/// 否則空集合會被後端當成「這次要用工具」而誠實拒絕（宣告更窄、實權更寬）；
/// 其餘一律空集合，不把上次的工具沿用到新的唯讀工作。
fn resume_tool_scope(previous: &Value) -> Value {
    let intent_only = previous
        .get("toolScope")
        .and_then(|v| v.as_array())
        .is_some_and(|scope| scope.len() == 1 && scope[0] == "conversation.generate");
    if intent_only {
        json!(["conversation.generate"])
    } else {
        json!([])
    }
}

fn resume_workdir(previous: &Value, explicit: Option<&str>) -> Option<String> {
    if let Some(dir) = explicit.map(str::trim).filter(|d| !d.is_empty()) {
        return Some(dir.to_string());
    }
    if let Some(dir) = previous
        .get("resolvedWorkdir")
        .and_then(|v| v.as_str())
        .filter(|d| !d.is_empty())
    {
        return Some(dir.to_string());
    }
    previous
        .get("dataScope")
        .and_then(|v| v.as_array())
        .and_then(|scopes| {
            scopes
                .iter()
                .filter_map(|s| s.as_str())
                .find_map(|s| s.strip_prefix("workspace:"))
        })
        .filter(|d| !d.is_empty())
        .map(str::to_string)
}

#[derive(Subcommand)]
pub enum ReceptorCmd {
    List,
    Inspect {
        id: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Live-read the receptor through the full runtime path.
    Test {
        id: String,
    },
    /// Push an observation into a push receptor: --fact key=value ...
    Push {
        id: String,
        #[arg(long = "fact", value_parser = parse_kv)]
        facts: Vec<(String, String)>,
        #[arg(long, default_value_t = 1.0)]
        confidence: f64,
    },
    /// Add a dynamic push receptor.
    Add {
        driver: String,
        #[arg(long)]
        id: String,
        #[arg(long, default_value = "custom")]
        category: String,
        #[arg(long)]
        sensitive: bool,
    },
    Remove {
        id: String,
    },
}

#[derive(Subcommand)]
pub enum ActuatorCmd {
    List,
    Inspect {
        id: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Send a small bounded test action through the FULL policy path.
    Test {
        id: String,
    },
    /// Add a dynamic mock actuator (simulated device).
    Add {
        driver: String,
        #[arg(long)]
        id: String,
        #[arg(long, default_value = "haptic")]
        channel: String,
    },
    Remove {
        id: String,
    },
}

#[derive(Subcommand)]
pub enum RecipeCmd {
    List,
    Show {
        id: String,
    },
    /// Validate a recipe file (path) or an installed recipe (id).
    Validate {
        path_or_id: String,
    },
    /// Install/update a recipe from a YAML/JSON file.
    Apply {
        path: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Explain whether the recipe would fire now + dry-run its plan.
    Simulate {
        id: String,
        /// What-if scenario JSON, e.g. '{"quietHours":true,"aiUnavailable":true}'.
        #[arg(long)]
        scenario: Option<String>,
    },
    /// Deterministic natural-language summary of what the recipe does.
    Summary {
        id: String,
        #[arg(long)]
        locale: Option<String>,
    },
    /// Run the recipe now (trigger bypassed; policy still applies).
    Run {
        id: String,
    },
    Remove {
        id: String,
    },
}

#[derive(Subcommand)]
pub enum PrefsCmd {
    Show,
    /// Merge-patch preferences, e.g. '{"mode":"advanced"}'.
    Set {
        patch: String,
    },
}

#[derive(Subcommand)]
pub enum AssistCmd {
    List,
    /// Answer a pending assist request: decision = proceed | no-action.
    Resolve {
        request_id: String,
        decision: String,
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ActionCmd {
    List {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    Show {
        action_id: String,
    },
}

#[derive(Subcommand)]
pub enum PolicyCmd {
    Show,
    /// Validate the policy file on disk.
    Validate,
    /// Merge-patch the policy with a JSON document.
    Set {
        patch: String,
    },
}

#[derive(Subcommand)]
pub enum SessionCmd {
    Start {
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        ttl_minutes: Option<u32>,
        /// Consent scopes to grant at start, e.g. channel:haptic
        #[arg(long = "consent")]
        consents: Vec<String>,
    },
    Show,
    /// Grant a consent scope, e.g. channel:haptic or actuator:mock.actuator
    Consent {
        scope: String,
        #[arg(long)]
        expires_minutes: Option<u32>,
        /// Real "only this once": `--max-uses 1` is spent by the first
        /// authorized dispatch. Omit for the historical unlimited-within-TTL
        /// grant. Never resets on a failed dispatch. Only `actuator:` and
        /// `channel:` scopes spend uses; `receptor:` / `tool:` scopes refuse
        /// `--max-uses` (use `--expires-minutes` instead).
        #[arg(long)]
        max_uses: Option<u32>,
    },
    /// Revoke a consent scope (cancels covered in-flight actions).
    Revoke {
        scope: String,
    },
    Stop,
}

#[derive(Subcommand)]
pub enum SelfSub {
    /// Show version; --check compares with the latest GitHub release.
    Version {
        #[arg(long)]
        check: bool,
    },
    /// Update this binary from GitHub Releases (default: latest).
    Update {
        #[arg(long)]
        version: Option<String>,
    },
    /// Remove this binary; --purge also deletes ~/.adaptive-interaction.
    Uninstall {
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Install the cross-AI skill (embedded, version-matched) into an agent's skill dir.
    InstallSkill {
        /// Target directory (default ~/.claude/skills/orchestrate-adaptive-interaction)
        #[arg(long)]
        dest: Option<std::path::PathBuf>,
    },
    /// Download the desktop control center bundle for this platform.
    InstallDesktop {
        #[arg(long)]
        version: Option<String>,
        /// Save directory (default ~/Downloads)
        #[arg(long)]
        out_dir: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum ToolCmd {
    List,
    Describe {
        name: String,
    },
    Call {
        name: String,
        #[arg(long, default_value = "{}")]
        input: String,
    },
    /// Export tool definitions: openai | anthropic | gemini | openapi | json-schema
    Export {
        #[arg(long)]
        format: String,
        /// Write to a file instead of stdout.
        #[arg(long)]
        out: Option<String>,
    },
}

#[derive(Args)]
pub struct PlanArgs {
    #[arg(long)]
    pub intent: String,
    #[arg(long)]
    pub message: Option<String>,
    #[arg(long)]
    pub magnitude: Option<f64>,
    #[arg(long)]
    pub duration_ms: Option<u64>,
    /// Preferred channels in priority order (repeatable).
    #[arg(long = "channel")]
    pub channels: Vec<String>,
    /// Candidate actuator ids (repeatable; empty = all).
    #[arg(long = "candidate")]
    pub candidates: Vec<String>,
    #[arg(long, default_value_t = 0)]
    pub min_channels: u32,
    #[arg(long, default_value_t = 3)]
    pub max_channels: u32,
    #[arg(long)]
    pub deny_no_action: bool,
    /// Actuation mode: single|parallel|sequence|fallback|adaptive|redundant
    #[arg(long)]
    pub mode: Option<String>,
    /// Verification: best-effort|observed|none
    #[arg(long)]
    pub verification: Option<String>,
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected key=value, got {s:?}"))
}

/// Print a result: JSON mode prints raw JSON; human mode pretty-prints.
fn emit(cli_json: bool, status: u16, value: &Value) -> i32 {
    if cli_json {
        println!(
            "{}",
            serde_json::to_string(value).unwrap_or_else(|_| "{}".into())
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
        );
    }
    exit_code_for_status(status)
}

fn fail(err: anyhow::Error) -> i32 {
    eprintln!("error: {err}");
    if err.to_string().contains("daemon offline") {
        EXIT_CONNECTION
    } else {
        1
    }
}

pub async fn run(cli: Cli) -> i32 {
    match dispatch(&cli).await {
        Ok(code) => code,
        Err(e) => fail(e),
    }
}

async fn dispatch(cli: &Cli) -> Result<i32> {
    // `serve` hosts the runtime; everything else is a client call.
    if let Command::Serve { host, port } = &cli.command {
        return serve(cli, host.clone(), *port).await;
    }
    if let Command::SelfCmd { command } = &cli.command {
        let code = match command {
            SelfSub::Version { check } => crate::selfmgmt::cmd_version(*check, cli.json).await?,
            SelfSub::Update { version } => {
                crate::selfmgmt::cmd_update(version.clone(), cli.dry_run).await?
            }
            SelfSub::Uninstall { purge, yes } => crate::selfmgmt::cmd_uninstall(*purge, *yes)?,
            SelfSub::InstallSkill { dest } => crate::selfmgmt::cmd_install_skill(dest.clone())?,
            SelfSub::InstallDesktop { version, out_dir } => {
                crate::selfmgmt::cmd_install_desktop(version.clone(), out_dir.clone()).await?
            }
        };
        return Ok(code);
    }
    if let Command::Ui = &cli.command {
        eprintln!("The desktop control center lives in apps/interaction-desktop.");
        eprintln!("Dev:   cd apps/interaction-desktop && pnpm install && pnpm tauri dev");
        eprintln!("Build: cd apps/interaction-desktop && pnpm tauri build");
        eprintln!("It connects to the same daemon (interact-ai serve) and token.");
        return Ok(0);
    }

    let client = Client::new(
        cli.config.as_deref(),
        cli.api.clone(),
        cli.token.clone(),
        cli.agent_scope,
    )?;
    let (status, value): (u16, Value) = match &cli.command {
        Command::Status => client.get("/v1/status").await?,
        Command::Health => client.get("/health").await?,
        Command::Capabilities {
            include_unavailable,
            human,
            locale,
        } => {
            if *human {
                let locale = locale.clone().unwrap_or_default();
                client
                    .get(&format!(
                        "/v1/capabilities/human?includeUnavailable={include_unavailable}&locale={locale}"
                    ))
                    .await?
            } else {
                client
                    .get(&format!(
                        "/v1/capabilities?includeUnavailable={include_unavailable}"
                    ))
                    .await?
            }
        }
        Command::Catalog => client.get("/v1/catalog").await?,
        Command::Sensors { action } => match action {
            crate::SensorsAction::Listen { ms } => {
                client
                    .post(
                        "/v1/sensors/microphone/listen",
                        Some(json!({"durationMs": ms})),
                    )
                    .await?
            }
            crate::SensorsAction::Stop => client.post("/v1/sensors/stop", Some(json!({}))).await?,
        },
        Command::Knowledge { action } => match action {
            crate::KnowledgeAction::DomainPacks => {
                client.get("/v1/knowledge/domain-packs").await?
            }
            crate::KnowledgeAction::InstallPack { id } => {
                client
                    .post(
                        &format!("/v1/knowledge/domain-packs/{id}/install"),
                        Some(json!({})),
                    )
                    .await?
            }
            crate::KnowledgeAction::UninstallPack { id } => {
                client
                    .delete(&format!("/v1/knowledge/domain-packs/{id}"))
                    .await?
            }
            crate::KnowledgeAction::Search { query, k } => {
                client
                    .get(&format!(
                        "/v1/knowledge/search?q={}&k={k}",
                        urlencode(query)
                    ))
                    .await?
            }
            crate::KnowledgeAction::List { status } => {
                let q = status
                    .as_deref()
                    .map(|s| format!("?status={s}"))
                    .unwrap_or_default();
                client.get(&format!("/v1/knowledge/nodes{q}")).await?
            }
            crate::KnowledgeAction::Show { id } => {
                client.get(&format!("/v1/knowledge/nodes/{id}")).await?
            }
            crate::KnowledgeAction::ProposeClaim {
                title,
                content,
                evidence,
                confidence,
                domains,
                as_agent,
                activate,
            } => {
                let evidence: Value = serde_json::from_str(evidence)
                    .map_err(|e| anyhow::anyhow!("invalid evidence JSON: {e}"))?;
                client
                    .post(
                        "/v1/knowledge/nodes",
                        Some(json!({
                            "nodeType": "claim",
                            "title": title,
                            "content": content,
                            "evidence": evidence,
                            "confidence": confidence,
                            "domains": domains,
                            "asAgent": as_agent,
                            "activate": activate,
                        })),
                    )
                    .await?
            }
            crate::KnowledgeAction::Link {
                from,
                to,
                relation,
                origin,
                rationale,
                as_agent,
            } => {
                client
                    .post(
                        "/v1/knowledge/edges",
                        Some(json!({
                            "from": from,
                            "to": to,
                            "relation": relation,
                            "origin": origin,
                            "rationale": rationale,
                            "asAgent": as_agent,
                        })),
                    )
                    .await?
            }
            crate::KnowledgeAction::Review {
                id,
                verdict,
                note,
                as_agent,
            } => {
                client
                    .post(
                        &format!("/v1/knowledge/nodes/{id}/review"),
                        Some(json!({"verdict": verdict, "note": note, "asAgent": as_agent})),
                    )
                    .await?
            }
            crate::KnowledgeAction::Graph { id } => {
                client
                    .get(&format!("/v1/knowledge/nodes/{id}/graph"))
                    .await?
            }
            crate::KnowledgeAction::Receipts => client.get("/v1/knowledge/receipts").await?,
            crate::KnowledgeAction::UpdateCheck { trigger } => {
                client
                    .post(
                        "/v1/knowledge/update-check",
                        Some(json!({"trigger": trigger})),
                    )
                    .await?
            }
            crate::KnowledgeAction::Correct {
                original,
                correction,
                scope,
            } => {
                client
                    .post(
                        "/v1/knowledge/user-corrections",
                        Some(json!({
                            "originalAssumption": original,
                            "correction": correction,
                            "scope": scope,
                        })),
                    )
                    .await?
            }
        },
        Command::Assets { action } => match action {
            crate::AssetsAction::Import {
                path,
                text,
                description,
            } => {
                client
                    .post(
                        "/v1/assets/import",
                        Some(json!({"path": path, "content": text, "description": description})),
                    )
                    .await?
            }
            crate::AssetsAction::List => client.get("/v1/assets").await?,
            crate::AssetsAction::Show { hash } => client.get(&format!("/v1/assets/{hash}")).await?,
            crate::AssetsAction::Derive { hash } => {
                client.post(&format!("/v1/assets/{hash}/derive"), None).await?
            }
            crate::AssetsAction::Derivatives { hash } => {
                client.get(&format!("/v1/assets/{hash}/derivatives")).await?
            }
            crate::AssetsAction::Impact { hash } => {
                client.get(&format!("/v1/assets/{hash}/impact")).await?
            }
            crate::AssetsAction::Delete { hash } => {
                client.delete(&format!("/v1/assets/{hash}")).await?
            }
        },
        Command::Memory { action } => match action {
            crate::MemoryAction::List { layer, limit } => {
                let mut q = format!("?limit={limit}");
                if let Some(l) = layer {
                    q.push_str(&format!("&layer={l}"));
                }
                client.get(&format!("/v1/memory{q}")).await?
            }
            crate::MemoryAction::Show { id } => client.get(&format!("/v1/memory/{id}")).await?,
            crate::MemoryAction::Add {
                layer,
                kind,
                title,
                content,
                tags,
                as_agent,
            } => {
                client
                    .post(
                        "/v1/memory",
                        Some(json!({
                            "layer": layer,
                            "kind": kind,
                            "title": title,
                            "content": content,
                            "tags": tags,
                            "asAgent": as_agent,
                        })),
                    )
                    .await?
            }
            crate::MemoryAction::Set { id, patch } => {
                let patch: Value = serde_json::from_str(patch)
                    .map_err(|e| anyhow::anyhow!("invalid JSON patch: {e}"))?;
                client.patch(&format!("/v1/memory/{id}"), patch).await?
            }
            crate::MemoryAction::Delete { id } => {
                client.delete(&format!("/v1/memory/{id}")).await?
            }
            crate::MemoryAction::Export => client.get("/v1/memory/export").await?,
            crate::MemoryAction::ClearSession => {
                client
                    .post("/v1/memory/clear-session-context", Some(json!({})))
                    .await?
            }
            crate::MemoryAction::Bundle {
                task,
                agent,
                domains,
            } => {
                client
                    .post(
                        "/v1/memory/context-bundle",
                        Some(json!({"task": task, "agentId": agent, "domains": domains})),
                    )
                    .await?
            }
        },
        Command::Proactive { action } => match action {
            crate::ProactiveAction::Status => client.get("/v1/proactive-dialogue").await?,
            crate::ProactiveAction::Mode { mode } => {
                client
                    .patch("/v1/proactive-dialogue", json!({"mode": mode}))
                    .await?
            }
            crate::ProactiveAction::Set {
                max_per_hour,
                min_interval_minutes,
                merge_window_seconds,
                no_follow_up,
                dnd_defer,
                daily_sessions,
                daily_cost_usd,
                generative_agent,
            } => {
                let mut patch = serde_json::Map::new();
                for (key, value) in [
                    ("maxPerHour", max_per_hour.map(Value::from)),
                    ("minIntervalMinutes", min_interval_minutes.map(Value::from)),
                    ("mergeWindowSeconds", merge_window_seconds.map(Value::from)),
                    ("noFollowUp", no_follow_up.map(Value::from)),
                    ("dndDefer", dnd_defer.map(Value::from)),
                    ("dailyGenerativeSessions", daily_sessions.map(Value::from)),
                    ("dailyGenerativeCostUsd", daily_cost_usd.map(Value::from)),
                ] {
                    if let Some(value) = value {
                        patch.insert(key.into(), value);
                    }
                }
                if let Some(agent) = generative_agent {
                    patch.insert(
                        "generativeAgent".into(),
                        if agent == "none" { Value::Null } else { json!(agent) },
                    );
                }
                client
                    .patch("/v1/proactive-dialogue", Value::Object(patch))
                    .await?
            }
            crate::ProactiveAction::Quiet { minutes } => {
                client
                    .post(
                        "/v1/proactive-dialogue/quiet",
                        Some(json!({"minutes": minutes})),
                    )
                    .await?
            }
        },
        Command::Character { action } => match action {
            crate::CharacterAction::Status => {
                let (status, value) = client.get("/v1/status").await?;
                let block = value
                    .get("characterProtocol")
                    .cloned()
                    .unwrap_or(Value::Null);
                if (200..300).contains(&status) && block.is_null() {
                    return Err(anyhow::anyhow!(
                        "daemon does not report characterProtocol (older runtime?)"
                    ));
                }
                (status, block)
            }
            crate::CharacterAction::Instances => client.get("/v1/character/instances").await?,
            crate::CharacterAction::Manifest => client.get("/v1/character/manifest").await?,
            crate::CharacterAction::Adapters { action } => match action {
                crate::CharacterAdapterAction::List => {
                    client.get("/v1/character/adapters").await?
                }
                crate::CharacterAdapterAction::Add { name, manifest } => {
                    let text = std::fs::read_to_string(manifest)
                        .map_err(|e| anyhow::anyhow!("read manifest {manifest}: {e}"))?;
                    let manifest: Value = serde_json::from_str(&text)
                        .map_err(|e| anyhow::anyhow!("manifest is not valid JSON: {e}"))?;
                    let out = client
                        .post(
                            "/v1/character/adapters",
                            Some(json!({"displayName": name, "manifest": manifest})),
                        )
                        .await?;
                    if !cli.json && (200..300).contains(&out.0) {
                        eprintln!(
                            "note: the token is shown ONCE and stored only as sha256; keep it safe"
                        );
                    }
                    out
                }
                crate::CharacterAdapterAction::Revoke { id } => {
                    client
                        .delete(&format!("/v1/character/adapters/{}", urlencode(id)))
                        .await?
                }
            },
            crate::CharacterAction::Intent { intent, message } => {
                client
                    .post(
                        "/v1/character/intent",
                        Some(json!({"intent": intent, "message": message})),
                    )
                    .await?
            }
        },
        Command::Presentation { action } => match action {
            crate::PresentationAction::Status => client.get("/v1/presentation").await?,
            crate::PresentationAction::Hello { visible, pack } => {
                client
                    .post(
                        "/v1/presentation/hello",
                        Some(json!({"visible": visible, "packId": pack})),
                    )
                    .await?
            }
            crate::PresentationAction::Ack {
                action_id,
                outcome,
                detail,
            } => {
                client
                    .post(
                        "/v1/presentation/ack",
                        Some(json!({
                            "actionId": action_id,
                            "outcome": outcome,
                            "detail": detail,
                        })),
                    )
                    .await?
            }
        },
        Command::Agents { action } => match action {
            crate::AgentsAction::Providers { refresh } => {
                if *refresh {
                    client.post("/v1/agents/refresh", Some(json!({}))).await?
                } else {
                    client.get("/v1/agents").await?
                }
            }
            crate::AgentsAction::Route { kind } => {
                let q = kind
                    .as_deref()
                    .map(|k| format!("?kind={k}"))
                    .unwrap_or_default();
                client.get(&format!("/v1/agents/routing{q}")).await?
            }
            crate::AgentsAction::Approve {
                id,
                request_id,
                yes,
            } => {
                client
                    .post(
                        &format!("/v1/agent-sessions/{id}/approve"),
                        Some(json!({"requestId": request_id, "approve": yes})),
                    )
                    .await?
            }
            crate::AgentsAction::Interrupt { id } => {
                client
                    .post(
                        &format!("/v1/agent-sessions/{id}/interrupt"),
                        Some(json!({})),
                    )
                    .await?
            }
            crate::AgentsAction::Sessions => client.get("/v1/agent-sessions").await?,
            crate::AgentsAction::Show { id } => {
                client.get(&format!("/v1/agent-sessions/{id}")).await?
            }
            crate::AgentsAction::Create {
                agent,
                label,
                ttl,
                max_messages,
                workdir,
                allow_write,
                max_cost,
                resume,
            } => {
                client
                    .post(
                        "/v1/agent-sessions",
                        Some(json!({
                            "agentId": agent,
                            "label": label,
                            "ttlMinutes": ttl,
                            "maxMessages": max_messages,
                            "workdir": workdir,
                            "allowWrite": allow_write,
                            "toolScope": if *allow_write { json!(["workspace.write"]) } else { json!([]) },
                            "consentScope": if *allow_write { json!(["agent-session:workspace-write"]) } else { json!([]) },
                            "maxCost": max_cost,
                            "resumeProviderSessionId": resume,
                        })),
                    )
                    .await?
            }
            crate::AgentsAction::Resume {
                id,
                label,
                ttl,
                max_cost,
                max_messages,
                workdir,
            } => {
                let (previous_status, previous) =
                    client.get(&format!("/v1/agent-sessions/{id}")).await?;
                if !(200..300).contains(&previous_status) {
                    // 讀不到就照實回傳原始錯誤（404 等），不猜、不代填。
                    return Ok(emit(cli.json, previous_status, &previous));
                }
                let provider_session_id = previous
                    .get("providerSessionId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "agent session {id} 沒有 providerSessionId，無法續開（只有真的接上 \
                             codex／claude-code 子程序的 session 才有 provider 端 thread）"
                        )
                    })?
                    .to_string();
                let agent = previous
                    .get("agentId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("agent session {id} 沒有 agentId"))?
                    .to_string();
                // 續開＝新的租約與新的權限審核。舊 session 的 allowWrite／
                // toolScope／consentScope 一律不繼承：權限旗標重新上鎖，
                // 要寫入就得再走一次 `agents create --allow-write`。
                //
                // 上限則相反：省略旗標**不是**「不限制」，而是沿用上一次的
                // 實際上限。落到 runtime 預設（120 分鐘、沒有金額上限、訊息
                // 吃 policy）每一項都比上次寬，後端會誠實拒絕。
                let limits = resume_limits(&previous, *ttl, *max_cost, *max_messages);
                client
                    .post(
                        "/v1/agent-sessions",
                        Some(json!({
                            "agentId": agent,
                            "label": label.clone().or_else(|| {
                                previous
                                    .get("label")
                                    .and_then(|v| v.as_str())
                                    .map(|l| format!("{l}（續開）"))
                            }),
                            "ttlMinutes": limits.ttl_minutes,
                            "maxCost": limits.max_cost,
                            "maxMessages": limits.max_messages,
                            "workdir": resume_workdir(&previous, workdir.as_deref()),
                            "allowWrite": false,
                            "toolScope": resume_tool_scope(&previous),
                            "consentScope": json!([]),
                            "resumeProviderSessionId": provider_session_id,
                        })),
                    )
                    .await?
            }
            crate::AgentsAction::Send { id, kind, body } => {
                let body: serde_json::Value = serde_json::from_str(body.as_str())
                    .map_err(|e| anyhow::anyhow!("--body must be JSON: {e}"))?;
                client
                    .post(
                        &format!("/v1/agent-sessions/{id}/messages"),
                        Some(json!({"kind": kind, "body": body})),
                    )
                    .await?
            }
            crate::AgentsAction::Messages { id, direction } => {
                client
                    .get(&format!(
                        "/v1/agent-sessions/{id}/messages?direction={direction}"
                    ))
                    .await?
            }
            crate::AgentsAction::Report { id, event, payload } => {
                let payload: serde_json::Value = serde_json::from_str(payload.as_str())
                    .map_err(|e| anyhow::anyhow!("--payload must be JSON: {e}"))?;
                client
                    .post(
                        &format!("/v1/agent-sessions/{id}/report"),
                        Some(json!({"event": event, "payload": payload})),
                    )
                    .await?
            }
            crate::AgentsAction::Renew { id, extra_minutes } => {
                client
                    .post(
                        &format!("/v1/agent-sessions/{id}/renew"),
                        Some(json!({"extraMinutes": extra_minutes})),
                    )
                    .await?
            }
            crate::AgentsAction::Close {
                id,
                handoff,
                reason,
            } => {
                let handoff: Option<serde_json::Value> = match handoff {
                    Some(h) => Some(
                        serde_json::from_str(h.as_str())
                            .map_err(|e| anyhow::anyhow!("--handoff must be JSON: {e}"))?,
                    ),
                    None => None,
                };
                client
                    .post(
                        &format!("/v1/agent-sessions/{id}/close"),
                        Some(json!({"handoff": handoff, "reason": reason})),
                    )
                    .await?
            }
            crate::AgentsAction::Verify { id, note } => {
                client
                    .post(
                        &format!("/v1/agent-sessions/{id}/verify"),
                        Some(json!({"note": note})),
                    )
                    .await?
            }
        },
        Command::Mobile { action } => match action {
            crate::MobileAction::Status => client.get("/v1/mobile/status").await?,
            crate::MobileAction::Pair => {
                client.post("/v1/mobile/pairing-session", Some(json!({}))).await?
            }
            crate::MobileAction::Revoke { device_id } => {
                client
                    .delete(&format!("/v1/mobile/devices/{device_id}"))
                    .await?
            }
            crate::MobileAction::StopSensors { device_id } => {
                client
                    .post(
                        &format!("/v1/mobile/devices/{device_id}/sensors/stop"),
                        Some(json!({})),
                    )
                    .await?
            }
            crate::MobileAction::Test { device_id } => {
                client
                    .post(&format!("/v1/mobile/devices/{device_id}/test"), Some(json!({})))
                    .await?
            }
            crate::MobileAction::BleScan { duration_ms } => {
                client
                    .post(
                        "/v1/mobile/ble/scan",
                        Some(json!({ "durationMs": duration_ms })),
                    )
                    .await?
            }
        },
        Command::Providers { action } => match action {
            crate::ProvidersAction::Scan => {
                client.post("/v1/hardware/scan", Some(json!({}))).await?
            }
            crate::ProvidersAction::List => client.get("/v1/providers").await?,
            crate::ProvidersAction::Show { id } => {
                client.get(&format!("/v1/providers/{id}")).await?
            }
            crate::ProvidersAction::Pair { id, code } => {
                client
                    .post(
                        &format!("/v1/providers/{id}/pair"),
                        Some(json!({"pairingCode": code})),
                    )
                    .await?
            }
            crate::ProvidersAction::Transition { id, state } => {
                client
                    .post(
                        &format!("/v1/providers/{id}/transition"),
                        Some(json!({"state": state})),
                    )
                    .await?
            }
            crate::ProvidersAction::Test { id } => {
                client
                    .post(&format!("/v1/providers/{id}/test"), Some(json!({})))
                    .await?
            }
            crate::ProvidersAction::Revoke { id } => {
                client
                    .post(&format!("/v1/providers/{id}/revoke"), Some(json!({})))
                    .await?
            }
        },
        Command::Pause { duration, reason } => {
            let mut body = json!({});
            if let Some(d) = duration {
                let ms =
                    interaction_recipe::parse_duration_ms(d).map_err(|e| anyhow::anyhow!(e))?;
                body["durationMinutes"] = json!((ms / 60_000).max(1));
            }
            if let Some(r) = reason {
                body["reason"] = json!(r);
            }
            client.post("/v1/pause", Some(body)).await?
        }
        Command::Resume => client.post("/v1/pause/clear", None).await?,
        Command::Prefs { command } => match command {
            PrefsCmd::Show => client.get("/v1/ui/preferences").await?,
            PrefsCmd::Set { patch } => {
                let patch: Value = serde_json::from_str(patch)
                    .map_err(|e| anyhow::anyhow!("invalid JSON patch: {e}"))?;
                client.patch("/v1/ui/preferences", patch).await?
            }
        },
        Command::Onboarding => client.get("/v1/onboarding").await?,
        Command::Describe {
            kind,
            id,
            text,
            locale,
            manifest_hash,
        } => {
            client
                .put(
                    &format!("/v1/capabilities/{kind}/{id}/ai-description"),
                    json!({"locale": locale, "text": text, "manifestHash": manifest_hash}),
                )
                .await?
        }
        Command::Assists { command } => match command {
            AssistCmd::List => client.get("/v1/ai-assists").await?,
            AssistCmd::Resolve {
                request_id,
                decision,
                note,
            } => {
                client
                    .post(
                        &format!("/v1/ai-assists/{request_id}/resolve"),
                        Some(json!({"decision": decision, "note": note})),
                    )
                    .await?
            }
        },
        Command::Receptors { command } => receptors(&client, command).await?,
        Command::Actuators { command } => actuators(&client, command).await?,
        Command::Recipes { command } => recipes(&client, command).await?,
        Command::Observe {
            receptor,
            fresh,
            limit,
            max_age_ms,
        } => {
            if *fresh {
                let id = receptor
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("--fresh requires --receptor <id>"))?;
                client
                    .post(&format!("/v1/receptors/{id}/read"), None)
                    .await?
            } else {
                let mut body = json!({"limit": limit});
                if let Some(r) = receptor {
                    body["receptorId"] = json!(r);
                }
                if let Some(age) = max_age_ms {
                    body["maxAgeMs"] = json!(age);
                }
                client.post("/v1/observations/query", Some(body)).await?
            }
        }
        Command::Plan(args) => {
            let mut metadata = serde_json::Map::new();
            if let Some(mode) = &args.mode {
                metadata.insert("actuationMode".into(), json!(mode));
            }
            if let Some(v) = &args.verification {
                metadata.insert("verification".into(), json!(v));
            }
            let body = json!({
                "intent": args.intent,
                "message": args.message,
                "magnitude": args.magnitude,
                "durationMs": args.duration_ms,
                "preferredChannels": args.channels,
                "candidates": args.candidates,
                "minChannels": args.min_channels,
                "maxChannels": args.max_channels,
                "allowNoAction": !args.deny_no_action,
                "metadata": metadata,
            });
            client.post("/v1/plans", Some(body)).await?
        }
        Command::Simulate { plan_id } => {
            client
                .post(&format!("/v1/plans/{plan_id}/simulate"), None)
                .await?
        }
        Command::Execute { plan_id } => {
            if cli.dry_run {
                client
                    .post(
                        &format!("/v1/plans/{plan_id}/execute"),
                        Some(json!({"dryRun": true})),
                    )
                    .await?
            } else {
                client
                    .post(&format!("/v1/plans/{plan_id}/execute"), None)
                    .await?
            }
        }
        Command::Verify { action_id } => {
            client
                .post(&format!("/v1/actions/{action_id}/verify"), None)
                .await?
        }
        Command::Actions { command } => match command {
            ActionCmd::List { limit } => client.get(&format!("/v1/actions?limit={limit}")).await?,
            ActionCmd::Show { action_id } => {
                client.get(&format!("/v1/actions/{action_id}")).await?
            }
        },
        Command::Cancel { action_id } => {
            client
                .post(&format!("/v1/actions/{action_id}/cancel"), None)
                .await?
        }
        Command::Stop { all } => {
            if !all {
                return Err(anyhow::anyhow!(
                    "use `stop --all` (or `cancel <action-id>` for one)"
                ));
            }
            client.post("/v1/stop-all", None).await?
        }
        Command::EmergencyStop { clear, reason } => {
            if *clear {
                client.post("/v1/emergency-stop/clear", None).await?
            } else {
                client
                    .post("/v1/emergency-stop", Some(json!({"reason": reason})))
                    .await?
            }
        }
        Command::Policy { command } => match command {
            PolicyCmd::Show => client.get("/v1/policy").await?,
            PolicyCmd::Validate => {
                // Local validation of the policy file (works without daemon).
                let paths = interaction_runtime::Paths::resolve(cli.config.as_deref());
                let service = interaction_runtime::ConfigService::new(paths);
                match service.load_policy() {
                    Ok(_) => (200, json!({"valid": true})),
                    Err(e) => (400, json!({"valid": false, "error": e.to_string()})),
                }
            }
            PolicyCmd::Set { patch } => {
                let patch: Value = serde_json::from_str(patch)
                    .map_err(|e| anyhow::anyhow!("--patch must be JSON: {e}"))?;
                client.patch("/v1/policy", patch).await?
            }
        },
        Command::Session { command } => match command {
            SessionCmd::Start {
                label,
                ttl_minutes,
                consents,
            } => {
                client
                    .post(
                        "/v1/session/start",
                        Some(json!({
                            "label": label,
                            "ttlMinutes": ttl_minutes,
                            "consents": consents,
                        })),
                    )
                    .await?
            }
            SessionCmd::Show => client.get("/v1/session").await?,
            SessionCmd::Consent {
                scope,
                expires_minutes,
                max_uses,
            } => {
                client
                    .post(
                        "/v1/session/consent",
                        Some(json!({
                            "scope": scope,
                            "expiresMinutes": expires_minutes,
                            "maxUses": max_uses,
                        })),
                    )
                    .await?
            }
            SessionCmd::Revoke { scope } => {
                client
                    .post("/v1/session/revoke", Some(json!({"scope": scope})))
                    .await?
            }
            SessionCmd::Stop => client.post("/v1/session/stop", None).await?,
        },
        Command::Tools { command } => match command {
            ToolCmd::List => client.get("/v1/tools").await?,
            ToolCmd::Describe { name } => client.get(&format!("/v1/tools/{name}")).await?,
            ToolCmd::Call { name, input } => {
                let input: Value = serde_json::from_str(input)
                    .map_err(|e| anyhow::anyhow!("--input must be JSON: {e}"))?;
                client
                    .post(&format!("/v1/tools/{name}/call"), Some(input))
                    .await?
            }
            ToolCmd::Export { format, out } => {
                let (status, value) = client.get(&format!("/v1/tools/export/{format}")).await?;
                if let Some(path) = out {
                    if status < 300 {
                        let export = value.get("export").cloned().unwrap_or(value.clone());
                        std::fs::write(path, serde_json::to_string_pretty(&export)?)?;
                        eprintln!("wrote {path}");
                    }
                }
                (status, value)
            }
        },
        Command::Inbox {
            status,
            agent,
            device,
            task,
            domain,
            since,
            limit,
        } => {
            let mut parts = vec![format!("limit={}", limit.clamp(&1, &500))];
            for (key, value) in [
                ("status", status),
                ("agent", agent),
                ("device", device),
                ("task", task),
                ("domain", domain),
                ("since", since),
            ] {
                if let Some(value) = value.as_deref() {
                    parts.push(format!("{key}={}", urlencode(value)));
                }
            }
            client
                .get(&format!("/v1/activity/inbox?{}", parts.join("&")))
                .await?
        }
        Command::Events { seconds } => {
            client.tail_events(*seconds, cli.json).await?;
            return Ok(0);
        }
        Command::Outbox { limit } => client.get(&format!("/v1/outbox?limit={limit}")).await?,
        Command::Audit { limit } => client.get(&format!("/v1/audit?limit={limit}")).await?,
        Command::Serve { .. }
        | Command::Ui
        | Command::Completion { .. }
        | Command::SelfCmd { .. } => {
            unreachable!()
        }
    };
    Ok(emit(cli.json, status, &value))
}

async fn receptors(client: &Client, cmd: &ReceptorCmd) -> Result<(u16, Value)> {
    match cmd {
        ReceptorCmd::List => client.get("/v1/receptors").await,
        ReceptorCmd::Inspect { id } => client.get(&format!("/v1/receptors/{id}")).await,
        ReceptorCmd::Enable { id } => {
            client
                .patch(&format!("/v1/receptors/{id}"), json!({"enabled": true}))
                .await
        }
        ReceptorCmd::Disable { id } => {
            client
                .patch(&format!("/v1/receptors/{id}"), json!({"enabled": false}))
                .await
        }
        ReceptorCmd::Test { id } => client.post(&format!("/v1/receptors/{id}/test"), None).await,
        ReceptorCmd::Push {
            id,
            facts,
            confidence,
        } => {
            let facts: serde_json::Map<String, Value> = facts
                .iter()
                .map(|(k, v)| (k.clone(), parse_scalar(v)))
                .collect();
            client
                .post(
                    &format!("/v1/receptors/{id}/push"),
                    Some(json!({"facts": facts, "confidence": confidence})),
                )
                .await
        }
        ReceptorCmd::Add {
            driver,
            id,
            category,
            sensitive,
        } => {
            client
                .post(
                    "/v1/receptors",
                    Some(json!({
                        "driver": driver, "id": id,
                        "category": category, "sensitive": sensitive,
                    })),
                )
                .await
        }
        ReceptorCmd::Remove { id } => client.delete(&format!("/v1/receptors/{id}")).await,
    }
}

async fn actuators(client: &Client, cmd: &ActuatorCmd) -> Result<(u16, Value)> {
    match cmd {
        ActuatorCmd::List => client.get("/v1/actuators").await,
        ActuatorCmd::Inspect { id } => client.get(&format!("/v1/actuators/{id}")).await,
        ActuatorCmd::Enable { id } => {
            client
                .patch(&format!("/v1/actuators/{id}"), json!({"enabled": true}))
                .await
        }
        ActuatorCmd::Disable { id } => {
            client
                .patch(&format!("/v1/actuators/{id}"), json!({"enabled": false}))
                .await
        }
        ActuatorCmd::Test { id } => client.post(&format!("/v1/actuators/{id}/test"), None).await,
        ActuatorCmd::Add {
            driver,
            id,
            channel,
        } => {
            client
                .post(
                    "/v1/actuators",
                    Some(json!({"driver": driver, "id": id, "channel": channel})),
                )
                .await
        }
        ActuatorCmd::Remove { id } => client.delete(&format!("/v1/actuators/{id}")).await,
    }
}

async fn recipes(client: &Client, cmd: &RecipeCmd) -> Result<(u16, Value)> {
    match cmd {
        RecipeCmd::List => client.get("/v1/recipes").await,
        RecipeCmd::Show { id } => client.get(&format!("/v1/recipes/{id}")).await,
        RecipeCmd::Validate { path_or_id } => {
            if std::path::Path::new(path_or_id).exists() {
                let text = std::fs::read_to_string(path_or_id)?;
                client
                    .post("/v1/recipes/validate", Some(json!({"text": text})))
                    .await
            } else {
                // Installed recipe: fetch and revalidate.
                let (status, value) = client.get(&format!("/v1/recipes/{path_or_id}")).await?;
                if status >= 300 {
                    return Ok((status, value));
                }
                client
                    .post("/v1/recipes/validate", Some(json!({"recipe": value})))
                    .await
            }
        }
        RecipeCmd::Apply { path } => {
            let text = std::fs::read_to_string(path)?;
            client
                .post("/v1/recipes", Some(json!({"text": text})))
                .await
        }
        RecipeCmd::Enable { id } => {
            client
                .patch(&format!("/v1/recipes/{id}"), json!({"enabled": true}))
                .await
        }
        RecipeCmd::Disable { id } => {
            client
                .patch(&format!("/v1/recipes/{id}"), json!({"enabled": false}))
                .await
        }
        RecipeCmd::Simulate { id, scenario } => match scenario {
            Some(raw) => {
                let scenario: Value = serde_json::from_str(raw)
                    .map_err(|e| anyhow::anyhow!("invalid scenario JSON: {e}"))?;
                client
                    .post(
                        &format!("/v1/recipes/{id}/simulate-scenario"),
                        Some(scenario),
                    )
                    .await
            }
            None => {
                client
                    .post(&format!("/v1/recipes/{id}/simulate"), None)
                    .await
            }
        },
        RecipeCmd::Summary { id, locale } => {
            let locale = locale.clone().unwrap_or_else(|| "zh-TW".into());
            client
                .get(&format!("/v1/recipes/{id}/summary?locale={locale}"))
                .await
        }
        RecipeCmd::Run { id } => client.post(&format!("/v1/recipes/{id}/run"), None).await,
        RecipeCmd::Remove { id } => client.delete(&format!("/v1/recipes/{id}")).await,
    }
}

fn parse_scalar(s: &str) -> Value {
    if let Ok(b) = s.parse::<bool>() {
        return json!(b);
    }
    if let Ok(n) = s.parse::<i64>() {
        return json!(n);
    }
    if let Ok(f) = s.parse::<f64>() {
        return json!(f);
    }
    json!(s)
}

async fn serve(cli: &Cli, host: Option<String>, port: Option<u16>) -> Result<i32> {
    use interaction_runtime::{Runtime, RuntimeOptions};
    let runtime = Runtime::start(RuntimeOptions {
        home: cli.config.clone(),
        acquire_lock: true,
        in_memory_db: false,
        spawn_watchdog: true,
    })
    .await
    .map_err(|e| anyhow::anyhow!("runtime start failed: {e}"))?;

    let config = runtime.config.read().await.clone();
    let bind_host = host.unwrap_or(config.api_host.clone());
    if bind_host != "127.0.0.1" && bind_host != "localhost" && bind_host != "::1" {
        eprintln!(
            "WARNING: binding to {bind_host} exposes the API beyond loopback; \
             the capability token is the only barrier. Prefer 127.0.0.1."
        );
    }
    let bind_port = port.unwrap_or(config.api_port);
    let token = runtime
        .config_service
        .load_or_create_token()
        .map_err(|e| anyhow::anyhow!("token: {e}"))?;

    let (addr, handle) = interaction_api::serve(runtime.clone(), &bind_host, bind_port, token)
        .await
        .map_err(|e| anyhow::anyhow!("bind {bind_host}:{bind_port}: {e}"))?;
    eprintln!("interact-ai daemon listening on http://{addr}");
    eprintln!("human token file: {}", runtime.paths.token_file().display());
    eprintln!(
        "restricted agent token file: {}",
        runtime.paths.agent_token_file().display()
    );
    eprintln!("press Ctrl-C to stop");

    tokio::signal::ctrl_c().await.ok();
    eprintln!("shutting down…");
    runtime.shutdown().await;
    handle.abort();
    Ok(0)
}

#[cfg(test)]
mod resume_defaults_tests {
    use super::*;

    fn previous() -> Value {
        json!({
            "budget": {"maxDurationMs": 30 * 60_000u64, "maxCost": 0.5, "maxMessages": 7},
            "lease": {"issuedAt": "2026-01-01T00:00:00Z", "expiresAt": "2026-01-01T00:45:00Z"},
            "dataScope": ["workspace:/labelled/guess", "domain:project-source"],
            "resolvedWorkdir": "/real/mounted/dir"
        })
    }

    /// 省略旗標＝沿用上次的實際上限，不是落到 runtime 預設（那更寬）。
    #[test]
    fn omitted_flags_reuse_the_previous_limits() {
        let limits = resume_limits(&previous(), None, None, None);
        assert_eq!(limits.ttl_minutes, 30);
        assert_eq!(limits.max_cost, 0.5);
        assert_eq!(limits.max_messages, 7);
    }

    /// 顯式帶的旗標仍然照送（放寬與否由後端確定性裁決，CLI 不代為放行）。
    #[test]
    fn explicit_flags_win_over_the_previous_limits() {
        let limits = resume_limits(&previous(), Some(999), Some(9.0), Some(99));
        assert_eq!(limits.ttl_minutes, 999);
        assert_eq!(limits.max_cost, 9.0);
        assert_eq!(limits.max_messages, 99);
    }

    /// 舊後端沒有回報 maxDurationMs 時退回租約長度；連租約都讀不到就選
    /// 最窄的 1 分鐘——不確定選窄的，不選寬的。
    #[test]
    fn ttl_falls_back_to_the_lease_span_then_to_the_narrowest_value() {
        let mut without_duration = previous();
        without_duration["budget"]
            .as_object_mut()
            .unwrap()
            .remove("maxDurationMs");
        assert_eq!(
            resume_limits(&without_duration, None, None, None).ttl_minutes,
            45
        );
        assert_eq!(resume_limits(&json!({}), None, None, None).ttl_minutes, 1);
    }

    /// 資料夾以後端記錄的 resolvedWorkdir 為準；dataScope 的 `workspace:`
    /// 只是人話標籤，缺席時才當備援。
    #[test]
    fn workdir_prefers_the_recorded_directory_over_the_scope_label() {
        assert_eq!(
            resume_workdir(&previous(), None).as_deref(),
            Some("/real/mounted/dir")
        );
        assert_eq!(
            resume_workdir(&previous(), Some("/explicit")).as_deref(),
            Some("/explicit")
        );
        let mut legacy = previous();
        legacy.as_object_mut().unwrap().remove("resolvedWorkdir");
        assert_eq!(
            resume_workdir(&legacy, None).as_deref(),
            Some("/labelled/guess")
        );
        assert_eq!(resume_workdir(&json!({}), None), None);
    }
}
