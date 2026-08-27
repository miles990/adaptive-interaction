//! interact-ai: cross-AI CLI for the adaptive interaction runtime.
//!
//! Contract:
//! - stdout carries results only (with `--json`: machine-readable JSON, nothing else)
//! - stderr carries diagnostics
//! - stable exit codes (see [`exit_code_for`])

mod client;
mod commands;
mod selfmgmt;

use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "interact-ai",
    version,
    about = "Adaptive interaction runtime CLI: discover capabilities, observe, plan, execute, verify, adapt.",
    propagate_version = true
)]
pub struct Cli {
    /// Emit raw JSON on stdout (no human text).
    #[arg(long, global = true)]
    pub json: bool,
    /// Interaction home directory (default ~/.adaptive-interaction or $INTERACT_AI_HOME).
    #[arg(long, global = true, env = "INTERACT_AI_HOME")]
    pub config: Option<PathBuf>,
    /// API base URL (default from config, e.g. http://127.0.0.1:8787).
    #[arg(long, global = true, env = "INTERACT_AI_API")]
    pub api: Option<String>,
    /// API token (default: read from state/api-token).
    #[arg(long, global = true, env = "INTERACT_AI_TOKEN", hide_env_values = true)]
    pub token: Option<String>,
    /// Suppress non-essential stderr output.
    #[arg(long, short, global = true)]
    pub quiet: bool,
    /// Verbose diagnostics on stderr.
    #[arg(long, short, global = true)]
    pub verbose: bool,
    /// Disable colored output.
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Dry-run where applicable.
    #[arg(long, global = true)]
    pub dry_run: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Runtime status.
    Status,
    /// Liveness/readiness of the daemon.
    Health,
    /// Discover receptors, actuators, tools and constraints.
    Capabilities {
        #[arg(long)]
        include_unavailable: bool,
        /// Human-readable cards (names, badges, data/impact semantics).
        #[arg(long)]
        human: bool,
        /// Locale for --human output (default: stored preference / zh-TW).
        #[arg(long)]
        locale: Option<String>,
    },
    /// Show the common capability catalog (canonical names, icons, aliases).
    Catalog,
    /// Capability providers (devices/services/agents): list, pair, revoke.
    Providers {
        #[command(subcommand)]
        action: ProvidersAction,
    },
    /// Agent sessions: leased, budgeted delegated work with a mailbox.
    Agents {
        #[command(subcommand)]
        action: AgentsAction,
    },
    /// Proactive dialogue policy (deterministic frequency limits).
    Proactive {
        #[command(subcommand)]
        action: ProactiveAction,
    },
    /// Companion presentation surface: presence heartbeat + command acks.
    Presentation {
        #[command(subcommand)]
        action: PresentationAction,
    },
    /// High-sensitivity sensors (microphone): bounded listen windows, stop.
    Sensors {
        #[command(subcommand)]
        action: SensorsAction,
    },
    /// Pause proactive interactions (a normal control; NOT emergency stop).
    Pause {
        /// Duration like "2h", "45m" (omit = until resumed).
        #[arg(long = "for")]
        duration: Option<String>,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Resume proactive interactions after a pause.
    Resume,
    /// Show or update UI/display preferences (mode, locale, custom names).
    Prefs {
        #[command(subcommand)]
        command: commands::PrefsCmd,
    },
    /// Show onboarding state (draft, completion, starter recipes).
    Onboarding,
    /// Write an AI-assisted description for a capability (bound to its
    /// current manifest hash; facts are never changed by descriptions).
    Describe {
        /// receptor | actuator | tool
        kind: String,
        id: String,
        #[arg(long)]
        text: String,
        #[arg(long, default_value = "zh-TW")]
        locale: String,
        /// Manifest hash from `capabilities --human` (stale hash is refused).
        #[arg(long)]
        manifest_hash: String,
    },
    /// Pending AI assist requests and their resolution.
    Assists {
        #[command(subcommand)]
        command: commands::AssistCmd,
    },
    /// Manage receptors.
    Receptors {
        #[command(subcommand)]
        command: commands::ReceptorCmd,
    },
    /// Manage actuators.
    Actuators {
        #[command(subcommand)]
        command: commands::ActuatorCmd,
    },
    /// Manage interaction recipes.
    Recipes {
        #[command(subcommand)]
        command: commands::RecipeCmd,
    },
    /// Query stored observations or read a receptor live.
    Observe {
        #[arg(long)]
        receptor: Option<String>,
        /// Read live from the receptor instead of the store.
        #[arg(long)]
        fresh: bool,
        #[arg(long, default_value_t = 10)]
        limit: u32,
        #[arg(long)]
        max_age_ms: Option<u64>,
    },
    /// Create a plan from a semantic intent.
    Plan(commands::PlanArgs),
    /// Dry-run a plan through the policy governor.
    Simulate { plan_id: String },
    /// Execute an authorized plan (accepted != completed; verify afterwards).
    Execute { plan_id: String },
    /// Re-verify an action against fresh observations.
    Verify { action_id: String },
    /// Inspect actions.
    Actions {
        #[command(subcommand)]
        command: commands::ActionCmd,
    },
    /// Cancel one action.
    Cancel { action_id: String },
    /// Cancel all open actions (soft stop). For the hard stop see emergency-stop.
    Stop {
        #[arg(long)]
        all: bool,
    },
    /// EMERGENCY STOP: halt all actuators and cancel everything. --clear re-arms.
    EmergencyStop {
        #[arg(long)]
        clear: bool,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Show or modify policy.
    Policy {
        #[command(subcommand)]
        command: commands::PolicyCmd,
    },
    /// Manage the interaction session and consents.
    Session {
        #[command(subcommand)]
        command: commands::SessionCmd,
    },
    /// Inspect, call and export tools.
    Tools {
        #[command(subcommand)]
        command: commands::ToolCmd,
    },
    /// Tail the live event stream (SSE).
    Events {
        /// Keep following (default prints buffered recent events and follows).
        #[arg(long, default_value_t = 30)]
        seconds: u64,
    },
    /// Show recent conversation / web-ui messages.
    Outbox {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Show the audit trail.
    Audit {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Run the runtime daemon (HTTP API on 127.0.0.1).
    Serve {
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Hints for launching the desktop control center.
    Ui,
    /// Manage this installation: update / uninstall / version / install-skill / install-desktop.
    #[command(name = "self")]
    SelfCmd {
        #[command(subcommand)]
        command: commands::SelfSub,
    },
    /// Generate shell completions.
    Completion { shell: clap_complete::Shell },
}

fn main() {
    let cli = Cli::parse();
    let filter = if cli.verbose {
        "info,interaction=debug"
    } else if cli.quiet {
        "error"
    } else {
        "warn,interact_ai=info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .with_ansi(!cli.no_color)
        .init();

    if let Command::Completion { shell } = &cli.command {
        let mut cmd = Cli::command();
        clap_complete::generate(*shell, &mut cmd, "interact-ai", &mut std::io::stdout());
        std::process::exit(0);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = runtime.block_on(commands::run(cli));
    std::process::exit(code);
}

#[derive(Subcommand)]
pub enum ProvidersAction {
    /// List all providers with lifecycle state and capabilities.
    List,
    /// Show one provider.
    Show { id: String },
    /// Pair with a discovered provider using the code it displays.
    Pair {
        id: String,
        /// Pairing code shown by the device/service.
        #[arg(long)]
        code: String,
    },
    /// Explicit lifecycle transition (install/disabled/available/…).
    Transition {
        id: String,
        #[arg(long)]
        state: String,
    },
    /// Revoke a provider: capabilities disabled immediately; sticky.
    Revoke { id: String },
}

#[derive(Subcommand)]
pub enum AgentsAction {
    /// List agent sessions (state, lease, budget).
    Sessions,
    /// Show one agent session.
    Show { id: String },
    /// Create a leased agent session.
    Create {
        /// Agent profile id (e.g. agent.coder).
        #[arg(long)]
        agent: String,
        #[arg(long)]
        label: Option<String>,
        /// Lease TTL in minutes (default 120).
        #[arg(long)]
        ttl: Option<u32>,
        /// Max mailbox messages (default from policy).
        #[arg(long)]
        max_messages: Option<u32>,
    },
    /// Send a message into a session mailbox.
    Send {
        id: String,
        /// message kind (task/question/cancel/…)
        #[arg(long, default_value = "task")]
        kind: String,
        /// JSON body.
        #[arg(long, default_value = "{}")]
        body: String,
    },
    /// Fetch mailbox messages (to-session marks delivery).
    Messages {
        id: String,
        #[arg(long, default_value = "to-session")]
        direction: String,
    },
    /// Report session state (the agent host calls this).
    Report {
        id: String,
        #[arg(long)]
        event: String,
        #[arg(long, default_value = "null")]
        payload: String,
    },
    /// Renew a renewable lease before it expires.
    Renew {
        id: String,
        #[arg(long, default_value_t = 30)]
        extra_minutes: u32,
    },
    /// Close a session (optionally with a bounded handoff JSON).
    Close {
        id: String,
        #[arg(long)]
        handoff: Option<String>,
        #[arg(long, default_value = "closed")]
        reason: String,
    },
}

#[derive(Subcommand)]
pub enum ProactiveAction {
    /// Show proactive-dialogue mode, hourly usage and quiet state.
    Status,
    /// Set the mode: off | necessary | natural | lively | custom
    Mode { mode: String },
    /// Ask the companion to stay quiet for a while.
    Quiet {
        #[arg(long, default_value_t = 60)]
        minutes: i64,
    },
}

#[derive(Subcommand)]
pub enum PresentationAction {
    /// Show companion-surface presence and pending command count.
    Status,
    /// Report a companion-surface heartbeat (normally sent by the desktop app).
    Hello {
        #[arg(long, default_value_t = true)]
        visible: bool,
        #[arg(long)]
        pack: Option<String>,
    },
    /// Acknowledge one presentation command (normally sent by the desktop app).
    Ack {
        action_id: String,
        /// displayed | completed | interrupted | failed | unsupported
        #[arg(long, default_value = "displayed")]
        outcome: String,
        #[arg(long)]
        detail: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SensorsAction {
    /// Begin one bounded microphone listen window (needs enable + consent).
    Listen {
        /// Window length in ms (hard-capped at 30000).
        #[arg(long, default_value_t = 10_000)]
        ms: u64,
    },
    /// Stop all sensors immediately.
    Stop,
}
