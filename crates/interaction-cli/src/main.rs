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
    /// Use the restricted AI/tool capability token instead of the human
    /// control token. Human-only mutations will be refused by the API.
    #[arg(long, global = true)]
    pub agent_scope: bool,
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
    /// 知識圖譜（素材、主張、關係、候選複審）。
    Knowledge {
        #[command(subcommand)]
        action: KnowledgeAction,
    },
    /// 內容定址原始素材庫（write-once）。
    Assets {
        #[command(subcommand)]
        action: AssetsAction,
    },
    /// 小樞的分層記憶（保存期限、可見性、Context Bundle）。
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
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
    /// iPhone Mobile Provider: pairing, status, revocation.
    Mobile {
        #[command(subcommand)]
        action: MobileAction,
    },
    /// Character Presentation Protocol: instances, manifest, external adapter
    /// tokens and a human manual intent test (non-safety intents only).
    Character {
        #[command(subcommand)]
        action: CharacterAction,
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
    /// Unified Activity Inbox: Consent/Agent/Knowledge/Result/Safety items.
    Inbox {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        domain: Option<String>,
        /// RFC 3339 lower time bound.
        #[arg(long)]
        since: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
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
    /// Scan metadata for currently visible interaction hardware. Never opens
    /// camera, microphone, HID capture, BLE, mDNS, or a device connection.
    Scan,
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
    /// Test a provider once, read-only: reads its first readable receptor and
    /// records the result as evidence. Never triggers an actuator and never
    /// enables a disabled sensor.
    Test { id: String },
    /// Revoke a provider: capabilities disabled immediately; sticky.
    Revoke { id: String },
}

#[derive(Subcommand)]
pub enum AgentsAction {
    /// Discovered local AI agents (codex / claude-code): version, login, protocol.
    Providers {
        /// Re-run discovery now instead of returning the cached snapshot.
        #[arg(long)]
        refresh: bool,
    },
    /// Deterministic routing suggestion for a task kind (code/docs/…).
    Route {
        #[arg(long)]
        kind: Option<String>,
    },
    /// Resolve a pending approval request from a gateway agent.
    Approve {
        id: String,
        request_id: String,
        /// Approve instead of the default deny.
        #[arg(long)]
        yes: bool,
    },
    /// Interrupt the session's current turn (keeps the session open).
    Interrupt { id: String },
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
        /// Working directory for gateway agents (codex/claude-code).
        #[arg(long)]
        workdir: Option<String>,
        /// Allow file edits inside the explicit workdir. This is a human consent
        /// action; it never enables unrestricted filesystem or network access.
        #[arg(long)]
        allow_write: bool,
        /// Max session cost in USD (0 = policy default).
        #[arg(long)]
        max_cost: Option<f64>,
        /// Continue an existing PROVIDER session/thread (claude --resume /
        /// codex thread resume). Resuming never widens scope: the new session
        /// gets a fresh lease and the connector re-locks sandbox/permission
        /// flags.
        #[arg(long, value_name = "PROVIDER_SESSION_ID")]
        resume: Option<String>,
    },
    /// Continue a previous agent session's provider thread in a NEW leased
    /// session. Permission flags are re-locked (never inherited): pass the
    /// write flags again if you still want them.
    Resume {
        /// A previous agent session id (its providerSessionId is reused).
        id: String,
        #[arg(long)]
        label: Option<String>,
        /// Lease TTL in minutes. Omitted = reuse the previous session's actual
        /// limit (NOT the runtime default, which would be wider and refused).
        #[arg(long)]
        ttl: Option<u32>,
        /// Max session cost in USD. Omitted = reuse the previous session's.
        #[arg(long)]
        max_cost: Option<f64>,
        /// Max mailbox messages. Omitted = reuse the previous session's.
        #[arg(long)]
        max_messages: Option<u32>,
        /// Working directory for gateway agents (codex/claude-code). Omitted =
        /// the directory the previous session was actually mounted in
        /// (`resolvedWorkdir`); resuming may never move to another folder.
        #[arg(long)]
        workdir: Option<String>,
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
    /// Fetch mailbox messages (read-only for a human token; only an agent identity marks delivery).
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
    /// Human-verify a claimed-completed session (claim ≠ verified until you do).
    Verify {
        id: String,
        /// Optional note recording what you checked.
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(clap::Subcommand)]
pub enum MobileAction {
    /// Server status + paired devices (connected state, honest sensor status).
    Status,
    /// Begin a 5-minute pairing session (prints the code + QR payload).
    Pair,
    /// Revoke one paired iPhone (disconnects immediately).
    Revoke { device_id: String },
    /// Ask ONE paired iPhone to stop sensing and wait (bounded) for its
    /// confirmation. No answer in time = outcome unknown, never "stopped".
    StopSensors { device_id: String },
    /// Ping one paired iPhone over its live connection. `ok` only means the
    /// socket answered — it does not mean the App's features work.
    Test { device_id: String },
    /// Ask the connected iPhone to scan for BLE peripherals (gateway must be
    /// switched on in the App). No answer in time = outcome unknown, not empty.
    BleScan {
        #[arg(long, default_value_t = 4_000)]
        duration_ms: u64,
    },
}

#[derive(Subcommand)]
pub enum KnowledgeAction {
    /// List the ten built-in, versioned Domain Packs and installation state.
    DomainPacks,
    /// Install (or restore) one built-in Domain Pack.
    InstallPack {
        id: String,
    },
    /// Uninstall a Domain Pack; it stays absent across restart.
    UninstallPack {
        id: String,
    },
    /// Search the graph (FTS + lexical-vector candidates).
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        k: u32,
    },
    /// List nodes (optionally by status: candidate/active/…).
    List {
        #[arg(long)]
        status: Option<String>,
    },
    Show {
        id: String,
    },
    /// Propose a claim (human by default; --as-agent forces candidate).
    ProposeClaim {
        #[arg(long)]
        title: String,
        #[arg(long)]
        content: String,
        /// Evidence JSON array, e.g. '[{"url":"https://…","segment":"page=3"}]'
        #[arg(long, default_value = "[]")]
        evidence: String,
        #[arg(long)]
        confidence: Option<f64>,
        #[arg(long = "domain")]
        domains: Vec<String>,
        #[arg(long)]
        as_agent: Option<String>,
        /// Human only: publish directly as active.
        #[arg(long)]
        activate: bool,
    },
    /// Propose a typed relation between two nodes.
    Link {
        from: String,
        to: String,
        #[arg(long)]
        relation: String,
        #[arg(long, default_value = "ai-conjecture")]
        origin: String,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long)]
        as_agent: Option<String>,
    },
    /// Review a candidate: approve / reject / comment (agent verdicts demote to comments).
    Review {
        id: String,
        verdict: String,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        as_agent: Option<String>,
    },
    /// Expand a node's neighborhood.
    Graph {
        id: String,
    },
    /// Knowledge receipts (machine-readable update trail).
    Receipts,
    /// Ask the deterministic curator whether a trigger needs an update / AI.
    UpdateCheck {
        /// user-added-asset | source-changed | repo-commit | task-artifact |
        /// user-correction | conflict-detected | review-overdue |
        /// low-confidence-answer | periodic-health-check
        trigger: String,
    },
    /// Record a human correction as User Memory and a reviewable Knowledge Candidate.
    Correct {
        /// What the system previously assumed (optional).
        #[arg(long)]
        original: Option<String>,
        /// The user's correction.
        correction: String,
        /// Where this correction applies; never generalized beyond this scope automatically.
        #[arg(long)]
        scope: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AssetsAction {
    /// Import a local file (or inline --text) into the content-addressed store.
    Import {
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    List,
    Show {
        hash: String,
    },
    /// Run bounded local processors and persist provenance-linked derivatives.
    Derive {
        hash: String,
    },
    /// List persisted derivative status and exact source regions/time spans.
    Derivatives {
        hash: String,
    },
    /// Preview what deleting this asset would affect.
    Impact {
        hash: String,
    },
    /// Delete (cascades deleteWithParent derivatives; active knowledge → disputed).
    Delete {
        hash: String,
    },
}

#[derive(Subcommand)]
pub enum MemoryAction {
    /// List memories (optionally one layer), with derived status.
    List {
        #[arg(long)]
        layer: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    Show {
        id: String,
    },
    /// Add a memory (human actor). --as-agent demotes per the actor rules.
    Add {
        #[arg(long)]
        layer: String,
        #[arg(long, default_value = "fact")]
        kind: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        content: String,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        as_agent: Option<String>,
    },
    /// Merge-patch fields (title/content/tags/retention/…): JSON document.
    Set {
        id: String,
        patch: String,
    },
    Delete {
        id: String,
    },
    /// Export every memory as JSON (data sovereignty).
    Export,
    /// Clear session-context memories.
    ClearSession,
    /// Build the deterministic context bundle an agent would receive.
    Bundle {
        #[arg(long)]
        task: String,
        #[arg(long)]
        agent: String,
        #[arg(long = "domain")]
        domains: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum ProactiveAction {
    /// Show proactive-dialogue mode, hourly usage and quiet state.
    Status,
    /// Set the mode: off | necessary | natural | lively | custom
    Mode { mode: String },
    /// Configure deterministic limits and the explicit local generative Agent.
    Set {
        #[arg(long)]
        max_per_hour: Option<u32>,
        #[arg(long)]
        min_interval_minutes: Option<u32>,
        #[arg(long)]
        merge_window_seconds: Option<u32>,
        #[arg(long)]
        no_follow_up: Option<bool>,
        #[arg(long)]
        dnd_defer: Option<bool>,
        #[arg(long)]
        daily_sessions: Option<u32>,
        #[arg(long)]
        daily_cost_usd: Option<f64>,
        /// codex | claude-code | none
        #[arg(long)]
        generative_agent: Option<String>,
    },
    /// Ask the companion to stay quiet for a while.
    Quiet {
        #[arg(long, default_value_t = 60)]
        minutes: i64,
    },
}

#[derive(Subcommand)]
pub enum CharacterAction {
    /// Protocol version, instance count and the active desktop character.
    Status,
    /// Every character instance (desktop + external adapters) with its honest
    /// connected / negotiated / generation / tested state.
    Instances,
    /// The manifest of the desktop character negotiated via /v1/character/hello.
    Manifest,
    /// External adapter tokens (sha256-stored; revoke disconnects immediately).
    Adapters {
        #[command(subcommand)]
        action: CharacterAdapterAction,
    },
    /// AIP Character Session: authoritative shared character state (snapshot,
    /// diagnostics, resume). Read-only from the CLI — semantic events and
    /// safety intents can only come from a bound surface or the runtime itself.
    Session {
        #[command(subcommand)]
        action: CharacterSessionAction,
    },
    /// Human manual test: present ONE non-safety intent with truthState none.
    /// Safety intents (emergency/blocked/failed/verified-success/...) are refused:
    /// they can only be produced by runtime truth projection.
    Intent {
        /// idle | notice | acknowledge | think | work | greet | play | rest | sleep
        intent: String,
        /// Optional presentation hint (<= 200 chars).
        #[arg(long)]
        message: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum CharacterSessionAction {
    /// The authoritative snapshot (state message envelope).
    Status,
    /// Counters, members and event-log usage (no tokens, no paths, no payloads).
    Diagnostics,
    /// Ask for what was missed: patches when the log covers it, otherwise a
    /// full snapshot (that is not an error).
    Resume {
        #[arg(long, default_value_t = 0)]
        last_revision: u64,
        #[arg(long, default_value_t = 0)]
        last_sequence: u64,
        /// The sessionEpoch the caller remembers; a different one means the
        /// session was rebuilt and local state must be dropped.
        #[arg(long, default_value_t = 0)]
        epoch: u64,
    },
}

#[derive(Subcommand)]
pub enum CharacterAdapterAction {
    /// List registered external adapters (never prints tokens).
    List,
    /// Register an external adapter; prints adapterId and the token ONCE.
    Add {
        /// Human-readable name (<= 48 chars).
        #[arg(long)]
        name: String,
        /// Path to the adapter's CharacterManifest JSON (external-process /
        /// remote-device / web).
        #[arg(long)]
        manifest: String,
    },
    /// Revoke an adapter: token invalid immediately, connection closed.
    Revoke { id: String },
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
