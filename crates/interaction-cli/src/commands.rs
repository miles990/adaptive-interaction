//! Command dispatch. Every command talks to the daemon over HTTP — the same
//! application services the API and Tauri use — except `serve`, which hosts
//! the runtime in-process.

use crate::client::{exit_code_for_status, Client, EXIT_CONNECTION};
use crate::{Cli, Command};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

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
    },
    /// Revoke a consent scope (cancels covered in-flight actions).
    Revoke {
        scope: String,
    },
    Stop,
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
    if let Command::Ui = &cli.command {
        eprintln!("The desktop control center lives in apps/interaction-desktop.");
        eprintln!("Dev:   cd apps/interaction-desktop && pnpm install && pnpm tauri dev");
        eprintln!("Build: cd apps/interaction-desktop && pnpm tauri build");
        eprintln!("It connects to the same daemon (interact-ai serve) and token.");
        return Ok(0);
    }

    let client = Client::new(cli.config.as_deref(), cli.api.clone(), cli.token.clone())?;
    let (status, value): (u16, Value) = match &cli.command {
        Command::Status => client.get("/v1/status").await?,
        Command::Health => client.get("/health").await?,
        Command::Capabilities {
            include_unavailable,
        } => {
            client
                .get(&format!(
                    "/v1/capabilities?includeUnavailable={include_unavailable}"
                ))
                .await?
        }
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
            } => {
                client
                    .post(
                        "/v1/session/consent",
                        Some(json!({"scope": scope, "expiresMinutes": expires_minutes})),
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
        Command::Events { seconds } => {
            client.tail_events(*seconds, cli.json).await?;
            return Ok(0);
        }
        Command::Outbox { limit } => client.get(&format!("/v1/outbox?limit={limit}")).await?,
        Command::Audit { limit } => client.get(&format!("/v1/audit?limit={limit}")).await?,
        Command::Serve { .. } | Command::Ui | Command::Completion { .. } => unreachable!(),
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
        RecipeCmd::Simulate { id } => {
            client
                .post(&format!("/v1/recipes/{id}/simulate"), None)
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
    eprintln!("token file: {}", runtime.paths.token_file().display());
    eprintln!("press Ctrl-C to stop");

    tokio::signal::ctrl_c().await.ok();
    eprintln!("shutting down…");
    runtime.shutdown().await;
    handle.abort();
    Ok(0)
}
