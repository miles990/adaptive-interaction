//! interact-ai: cross-AI CLI for the adaptive interaction runtime.
//!
//! Contract:
//! - stdout carries results only (with `--json`: machine-readable JSON, nothing else)
//! - stderr carries diagnostics
//! - stable exit codes (see [`exit_code_for`])

mod client;
mod commands;

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
