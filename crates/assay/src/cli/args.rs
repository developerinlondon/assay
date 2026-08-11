use std::path::PathBuf;

use clap::{Parser, Subcommand};

use assay::install;

use crate::cli;

/// Assay — lightweight Lua scripting runtime for deployment verification.
///
/// Run with a subcommand, or pass a file directly for auto-detection:
///   assay run script.lua     Explicit run
///   assay script.lua         Auto-detect by extension (backward compat)
///   assay checks.yaml        YAML check orchestration
#[derive(Parser, Debug)]
#[command(name = "assay", version, about, args_conflicts_with_subcommands = true)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,

    /// Path to a .yaml config or .lua script.
    pub(crate) file: Option<PathBuf>,

    /// Enable verbose logging (sets RUST_LOG=debug).
    #[arg(short, long, global = true)]
    pub(crate) verbose: bool,

    /// Read-only mode: mutating builtins (shell, process, fs writes,
    /// http writes, ...) raise errors instead of executing.
    /// ASSAY_READONLY=1 activates the same mode.
    #[arg(long, global = true)]
    pub(crate) readonly: bool,

    /// Approval mode: mutating builtins suspend for per-operation human
    /// approval via the resume flow instead of executing.
    /// ASSAY_APPROVAL=1 activates the same mode. Takes precedence over
    /// --readonly.
    #[arg(long, global = true)]
    pub(crate) approval_mode: bool,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Commands {
    /// Search for modules matching a query
    Context {
        /// Search query string
        query: String,
        /// Maximum results to show
        #[arg(short, long, default_value = "5")]
        limit: usize,
    },
    /// Execute a Lua script inline or from file
    Exec {
        /// Evaluate Lua code directly
        #[arg(short = 'e', long = "eval")]
        eval: Option<String>,
        /// Lua script file to execute
        file: Option<PathBuf>,
    },
    /// List all available modules
    Modules,
    /// Run a file (yaml or lua)
    Run {
        /// Path to .yaml or .lua file
        file: PathBuf,
        #[arg(long, value_parser = ["tool", "script"])]
        mode: Option<String>,
        #[arg(long, default_value = "20")]
        timeout: Option<u64>,
        /// Positional arguments passed through to the Lua script as the
        /// `arg` global (a 1-indexed array, mirroring `lua` and `luajit`).
        /// Use `--` to separate them from `assay run`'s own flags:
        /// `assay run script.lua -- --email a@b.c --password hunter2`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        script_args: Vec<String>,
    },
    Resume {
        #[arg(long)]
        token: String,
        #[arg(long, value_parser = ["yes", "no"])]
        approve: String,
        #[arg(long, default_value = "3600")]
        resume_ttl: Option<u64>,
        /// Identity of whoever authorized this decision, recorded for
        /// audit in the resume state and echoed in the result envelope.
        #[arg(long)]
        approver: Option<String>,
    },
    /// Manage workflows
    Workflow {
        #[command(flatten)]
        global: CliEngineOpts,
        #[command(subcommand)]
        command: WorkflowCommands,
    },
    /// Manage schedules
    Schedule {
        #[command(flatten)]
        global: CliEngineOpts,
        #[command(subcommand)]
        command: ScheduleCommands,
    },
    /// Manage namespaces
    Namespace {
        #[command(flatten)]
        global: CliEngineOpts,
        #[command(subcommand)]
        command: NamespaceCommands,
    },
    /// Inspect workers registered with the engine
    Worker {
        #[command(flatten)]
        global: CliEngineOpts,
        #[command(subcommand)]
        command: WorkerCommands,
    },
    /// Inspect task-queue stats
    Queue {
        #[command(flatten)]
        global: CliEngineOpts,
        #[command(subcommand)]
        command: QueueCommands,
    },
    /// Install binaries + libs declared in a `Manifest.lua`.
    ///
    /// Reads `./Manifest.lua` (or `-f <path>`), fetches each declared
    /// extension binary + lib tarball over HTTPS, verifies sha256, and
    /// installs into the configured bin/lib paths. See
    /// `.claude/plans/21-libs-folder-and-install.md`.
    Install(install::InstallArgs),
    /// Generate shell completion scripts.
    ///
    /// Pipe the output into the appropriate shell-completion location:
    ///   bash:  assay completion bash > /etc/bash_completion.d/assay
    ///   zsh:   assay completion zsh  > "${fpath[1]}/_assay"
    ///   fish:  assay completion fish > ~/.config/fish/completions/assay.fish
    Completion {
        /// Target shell.
        shell: clap_complete::Shell,
    },
    /// Run a Model Context Protocol server over stdio.
    ///
    /// Speaks JSON-RPC 2.0 over stdin/stdout and exposes two tools:
    ///   assay_run     — execute a gated Lua script (readonly | approval)
    ///   assay_context — prompt-ready module docs for discovery
    ///
    /// The single `assay_run` tool composes all embedded modules through
    /// Lua, so the exposed schema stays tiny regardless of module count.
    McpServe,
}

/// Global flags shared by `workflow` / `schedule` / `namespace` / `worker` /
/// `queue` subcommands. Each backed by an env var so pods can drop these
/// into their environment without passing flags on every invocation.
#[derive(clap::Args, Debug)]
pub(crate) struct CliEngineOpts {
    /// Workflow engine base URL (default http://127.0.0.1:8080 / $ASSAY_ENGINE_URL).
    #[arg(long, global = true, env = "ASSAY_ENGINE_URL")]
    pub(crate) engine_url: Option<String>,
    /// Bearer token. CLI forwards it as `Authorization: Bearer <value>`;
    /// the engine decides whether it's an API key or a JWT.
    #[arg(long, global = true, env = "ASSAY_API_KEY")]
    pub(crate) api_key: Option<String>,
    /// Target namespace (default "main" / $ASSAY_NAMESPACE).
    #[arg(long, global = true, env = "ASSAY_NAMESPACE")]
    pub(crate) namespace: Option<String>,
    /// Output format: table | json | jsonl | yaml.
    /// Default is `table` on a TTY and `json` when stdout is piped.
    #[arg(long, global = true, env = "ASSAY_OUTPUT")]
    pub(crate) output: Option<String>,
    /// Path to a YAML config file. Discovery order: --config flag,
    /// ASSAY_CONFIG_FILE, $XDG_CONFIG_HOME/assay/config.yaml,
    /// $HOME/.config/assay/config.yaml, /etc/assay/config.yaml.
    #[arg(long, global = true, env = "ASSAY_CONFIG_FILE")]
    pub(crate) config: Option<String>,
}

impl CliEngineOpts {
    pub(crate) fn as_flags(&self) -> cli::GlobalFlags<'_> {
        cli::GlobalFlags {
            engine_url: self.engine_url.as_deref(),
            api_key: self.api_key.as_deref(),
            namespace: self.namespace.as_deref(),
            output: self.output.as_deref(),
            config: self.config.as_deref(),
        }
    }
}

#[derive(Subcommand, Debug)]
pub(crate) enum WorkflowCommands {
    /// Start a new workflow run
    Start {
        /// Workflow type name
        #[arg(long = "type")]
        workflow_type: String,
        /// Workflow ID (auto-generated if omitted)
        #[arg(long)]
        id: Option<String>,
        /// JSON input. Literal, `@file.json`, or `-` for stdin.
        #[arg(long)]
        input: Option<String>,
        /// Task queue (default "default")
        #[arg(long)]
        queue: Option<String>,
        /// JSON search attributes (indexed metadata for filtering).
        /// Literal, `@file.json`, or `-` for stdin.
        #[arg(long)]
        search_attrs: Option<String>,
    },
    /// List workflows
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long = "type")]
        workflow_type: Option<String>,
        /// Filter by search attributes. Literal JSON, `@file.json`, or `-` for stdin.
        #[arg(long)]
        search_attrs: Option<String>,
        #[arg(long, default_value = "20")]
        limit: i64,
    },
    /// Describe a workflow
    Describe {
        /// Workflow ID
        id: String,
    },
    /// Read the latest state snapshot written by `ctx:register_query` handlers.
    /// With a query name, returns just that query's value; without, the full map.
    State {
        /// Workflow ID
        id: String,
        /// Query handler name (omit to dump all registered queries)
        name: Option<String>,
    },
    /// Read a workflow's event log, optionally streaming new events.
    Events {
        /// Workflow ID
        id: String,
        /// Poll for new events every 500ms until the workflow terminates.
        #[arg(long)]
        follow: bool,
    },
    /// List a parent workflow's child workflows
    Children {
        /// Parent workflow ID
        id: String,
    },
    /// Send a signal to a workflow
    Signal {
        /// Workflow ID
        id: String,
        /// Signal name
        name: String,
        /// JSON payload. Literal, `@file.json`, or `-` for stdin.
        payload: Option<String>,
    },
    /// Cancel a workflow
    Cancel {
        /// Workflow ID
        id: String,
    },
    /// Terminate a workflow
    Terminate {
        /// Workflow ID
        id: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Retry the terminal failed activity in place, preserving history.
    Retry {
        /// Workflow ID
        id: String,
        /// Audit identity of the operator requesting the retry
        #[arg(long)]
        requested_by: String,
        /// Why the failed activity is safe to retry now
        #[arg(long)]
        reason: String,
    },
    /// Close out a workflow and start a fresh run with the same type,
    /// namespace, and task queue. Client-side continue-as-new — distinct
    /// from the worker-side `ctx:continue_as_new`.
    #[command(name = "continue-as-new")]
    ContinueAsNew {
        /// Workflow ID
        id: String,
        /// JSON input for the new run. Literal, `@file.json`, or `-` for stdin.
        #[arg(long)]
        input: Option<String>,
    },
    /// Block until a workflow reaches a terminal state (or a specific
    /// target status). Exits 0 on COMPLETED / match, 1 on FAILED /
    /// CANCELLED / TIMED_OUT (when no target), 2 on timeout.
    Wait {
        /// Workflow ID
        id: String,
        /// Max seconds to wait (default 300)
        #[arg(long, default_value = "300")]
        timeout: u64,
        /// Specific status to wait for (default: any terminal status)
        #[arg(long)]
        target: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum NamespaceCommands {
    /// Create a namespace
    Create {
        /// Namespace name
        name: String,
    },
    /// List namespaces
    List,
    /// Describe a namespace (includes live counts)
    Describe {
        /// Namespace name
        name: String,
    },
    /// Delete a namespace
    Delete {
        /// Namespace name
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum WorkerCommands {
    /// List registered workers
    List,
}

#[derive(Subcommand, Debug)]
pub(crate) enum QueueCommands {
    /// Pending / running activity counts per task queue
    Stats,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScheduleCommands {
    /// List schedules
    List,
    /// Describe a single schedule
    Describe {
        /// Schedule name
        name: String,
    },
    /// Create a schedule
    Create {
        /// Schedule name
        name: String,
        #[arg(long = "type")]
        workflow_type: String,
        #[arg(long)]
        cron: String,
        /// IANA timezone for cron evaluation (default UTC)
        #[arg(long)]
        timezone: Option<String>,
        #[arg(long)]
        input: Option<String>,
        #[arg(long, default_value = "default")]
        queue: String,
    },
    /// Apply a partial update to a schedule. Only fields present are
    /// changed; unchanged fields keep their values.
    Patch {
        /// Schedule name
        name: String,
        #[arg(long)]
        cron: Option<String>,
        #[arg(long)]
        timezone: Option<String>,
        #[arg(long)]
        input: Option<String>,
        #[arg(long)]
        queue: Option<String>,
        #[arg(long)]
        overlap: Option<String>,
    },
    /// Pause a schedule
    Pause {
        /// Schedule name
        name: String,
    },
    /// Resume a schedule
    Resume {
        /// Schedule name
        name: String,
    },
    /// Delete a schedule
    Delete {
        /// Schedule name
        name: String,
    },
}
