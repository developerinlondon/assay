mod checks;
mod cli;
mod config;
mod lua;
mod mcp;
mod output;
mod runner;

use assay::install;

use clap::{Parser, Subcommand};
use mlua::LuaSerdeExt;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;
use std::time::Duration;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 20;
const DEFAULT_RESUME_TTL_SECS: u64 = 3600;
const TOOL_STDOUT_CAP_BYTES: usize = 512 * 1024;
const APPROVAL_REQUEST_PREFIX: &str = "__assay_approval_request__:";

pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("building HTTP client")
}

/// Assay — lightweight Lua scripting runtime for deployment verification.
///
/// Run with a subcommand, or pass a file directly for auto-detection:
///   assay run script.lua     Explicit run
///   assay script.lua         Auto-detect by extension (backward compat)
///   assay checks.yaml        YAML check orchestration
#[derive(Parser, Debug)]
#[command(name = "assay", version, about, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to a .yaml config or .lua script.
    file: Option<PathBuf>,

    /// Enable verbose logging (sets RUST_LOG=debug).
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Read-only mode: mutating builtins (shell, process, fs writes,
    /// http writes, ...) raise errors instead of executing.
    /// ASSAY_READONLY=1 activates the same mode.
    #[arg(long, global = true)]
    readonly: bool,

    /// Approval mode: mutating builtins suspend for per-operation human
    /// approval via the resume flow instead of executing.
    /// ASSAY_APPROVAL=1 activates the same mode. Takes precedence over
    /// --readonly.
    #[arg(long, global = true)]
    approval_mode: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
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
struct CliEngineOpts {
    /// Workflow engine base URL (default http://127.0.0.1:8080 / $ASSAY_ENGINE_URL).
    #[arg(long, global = true, env = "ASSAY_ENGINE_URL")]
    engine_url: Option<String>,
    /// Bearer token. CLI forwards it as `Authorization: Bearer <value>`;
    /// the engine decides whether it's an API key or a JWT.
    #[arg(long, global = true, env = "ASSAY_API_KEY")]
    api_key: Option<String>,
    /// Target namespace (default "main" / $ASSAY_NAMESPACE).
    #[arg(long, global = true, env = "ASSAY_NAMESPACE")]
    namespace: Option<String>,
    /// Output format: table | json | jsonl | yaml.
    /// Default is `table` on a TTY and `json` when stdout is piped.
    #[arg(long, global = true, env = "ASSAY_OUTPUT")]
    output: Option<String>,
    /// Path to a YAML config file. Discovery order: --config flag,
    /// ASSAY_CONFIG_FILE, $XDG_CONFIG_HOME/assay/config.yaml,
    /// $HOME/.config/assay/config.yaml, /etc/assay/config.yaml.
    #[arg(long, global = true, env = "ASSAY_CONFIG_FILE")]
    config: Option<String>,
}

impl CliEngineOpts {
    fn as_flags(&self) -> cli::GlobalFlags<'_> {
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
enum WorkflowCommands {
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
enum NamespaceCommands {
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
enum WorkerCommands {
    /// List registered workers
    List,
}

#[derive(Subcommand, Debug)]
enum QueueCommands {
    /// Pending / running activity counts per task queue
    Stats,
}

#[derive(Subcommand, Debug)]
enum ScheduleCommands {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScriptMode {
    Script,
    Tool,
}

#[derive(Clone, Debug)]
struct RunOptions {
    mode: ScriptMode,
    timeout_secs: u64,
    exec_mode: lua::ExecMode,
    approval: lua::ApprovalConfig,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            mode: resolve_script_mode(None),
            timeout_secs: DEFAULT_TOOL_TIMEOUT_SECS,
            exec_mode: resolve_exec_mode(false, false),
            approval: lua::approval_config_from_env(),
        }
    }
}

/// Resolve the execution mode from CLI flags and environment. Approval
/// mode wins over read-only when both are requested.
fn resolve_exec_mode(cli_readonly: bool, cli_approval: bool) -> lua::ExecMode {
    if cli_approval || lua::approval_from_env() {
        lua::ExecMode::Approval
    } else if cli_readonly || lua::readonly_from_env() {
        lua::ExecMode::ReadOnly
    } else {
        lua::ExecMode::Unrestricted
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Serialize)]
struct ToolSuccessEnvelope {
    ok: bool,
    status: &'static str,
    output: JsonValue,
    #[serde(rename = "requiresApproval")]
    requires_approval: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
    #[serde(skip_serializing_if = "is_false")]
    readonly: bool,
}

#[derive(Serialize)]
struct ToolErrorEnvelope {
    ok: bool,
    status: &'static str,
    error: String,
    #[serde(skip_serializing_if = "is_false")]
    readonly: bool,
}

#[derive(Deserialize)]
struct ApprovalRequestPayload {
    prompt: String,
    #[serde(default)]
    context: JsonValue,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    index: Option<u64>,
}

#[derive(Deserialize, Serialize)]
struct ResumeState {
    script_path: PathBuf,
    approval_prompt: String,
    approval_context: JsonValue,
    created_at: u64,
    ttl_secs: u64,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    approved_indices: Vec<u64>,
    #[serde(default)]
    pending_index: Option<u64>,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    script_args: Vec<String>,
    /// Every grant issued so far in this run's approval chain: the index,
    /// the op it was approved for, and (when supplied) who approved it.
    #[serde(default)]
    approved_ops: Vec<lua::ApprovedOp>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let exec_mode = resolve_exec_mode(cli.readonly, cli.approval_mode);
    let readonly = exec_mode.is_readonly();
    let approval = if exec_mode.is_approval() {
        lua::approval_config_from_env()
    } else {
        lua::ApprovalConfig::default()
    };

    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Some(Commands::Context { query, limit }) => run_context(&query, limit),
        Some(Commands::Exec { eval, file }) => {
            if let Some(code) = eval {
                run_lua_inline(&code, exec_mode, &approval).await
            } else if let Some(path) = file {
                let options = RunOptions {
                    exec_mode,
                    approval: approval.clone(),
                    ..RunOptions::default()
                };
                run_lua_script(&path, options, Vec::new()).await
            } else {
                eprintln!("error: exec requires either -e <code> or a file path");
                ExitCode::from(1)
            }
        }
        Some(Commands::Modules) => run_modules(exec_mode),
        Some(Commands::Run {
            file,
            mode,
            timeout,
            script_args,
        }) => {
            let options = RunOptions {
                mode: resolve_script_mode(mode.as_deref()),
                timeout_secs: timeout.unwrap_or(DEFAULT_TOOL_TIMEOUT_SECS),
                exec_mode,
                approval: approval.clone(),
            };
            dispatch_file(&file, options, script_args).await
        }
        Some(Commands::Resume {
            token,
            approve,
            resume_ttl,
            approver,
        }) => {
            resume_tool_execution(&token, &approve, resume_ttl, readonly, approver.as_deref()).await
        }
        Some(Commands::Workflow { global, command }) => {
            let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
                Ok(o) => o,
                Err(code) => return code,
            };
            match command {
                WorkflowCommands::Start {
                    workflow_type,
                    id,
                    input,
                    queue,
                    search_attrs,
                } => {
                    cli::commands::workflow_start(
                        &opts,
                        &workflow_type,
                        id,
                        input,
                        queue,
                        search_attrs,
                    )
                    .await
                }
                WorkflowCommands::List {
                    status,
                    workflow_type,
                    search_attrs,
                    limit,
                } => {
                    cli::commands::workflow_list(&opts, status, workflow_type, search_attrs, limit)
                        .await
                }
                WorkflowCommands::Describe { id } => {
                    cli::commands::workflow_describe(&opts, &id).await
                }
                WorkflowCommands::State { id, name } => {
                    cli::commands::workflow_state(&opts, &id, name.as_deref()).await
                }
                WorkflowCommands::Events { id, follow } => {
                    cli::commands::workflow_events(&opts, &id, follow).await
                }
                WorkflowCommands::Children { id } => {
                    cli::commands::workflow_children(&opts, &id).await
                }
                WorkflowCommands::Signal { id, name, payload } => {
                    cli::commands::workflow_signal(&opts, &id, &name, payload).await
                }
                WorkflowCommands::Cancel { id } => cli::commands::workflow_cancel(&opts, &id).await,
                WorkflowCommands::Terminate { id, reason } => {
                    cli::commands::workflow_terminate(&opts, &id, reason).await
                }
                WorkflowCommands::ContinueAsNew { id, input } => {
                    cli::commands::workflow_continue_as_new(&opts, &id, input).await
                }
                WorkflowCommands::Wait {
                    id,
                    timeout,
                    target,
                } => cli::commands::workflow_wait(&opts, &id, timeout, target).await,
            }
        }
        Some(Commands::Schedule { global, command }) => {
            let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
                Ok(o) => o,
                Err(code) => return code,
            };
            match command {
                ScheduleCommands::List => cli::commands::schedule_list(&opts).await,
                ScheduleCommands::Describe { name } => {
                    cli::commands::schedule_describe(&opts, &name).await
                }
                ScheduleCommands::Create {
                    name,
                    workflow_type,
                    cron,
                    timezone,
                    input,
                    queue,
                } => {
                    cli::commands::schedule_create(
                        &opts,
                        &name,
                        &workflow_type,
                        &cron,
                        timezone,
                        input,
                        Some(queue),
                    )
                    .await
                }
                ScheduleCommands::Patch {
                    name,
                    cron,
                    timezone,
                    input,
                    queue,
                    overlap,
                } => {
                    cli::commands::schedule_patch(
                        &opts, &name, cron, timezone, input, queue, overlap,
                    )
                    .await
                }
                ScheduleCommands::Pause { name } => {
                    cli::commands::schedule_pause(&opts, &name).await
                }
                ScheduleCommands::Resume { name } => {
                    cli::commands::schedule_resume(&opts, &name).await
                }
                ScheduleCommands::Delete { name } => {
                    cli::commands::schedule_delete(&opts, &name).await
                }
            }
        }
        Some(Commands::Namespace { global, command }) => {
            let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
                Ok(o) => o,
                Err(code) => return code,
            };
            match command {
                NamespaceCommands::Create { name } => {
                    cli::commands::namespace_create(&opts, &name).await
                }
                NamespaceCommands::List => cli::commands::namespace_list(&opts).await,
                NamespaceCommands::Describe { name } => {
                    cli::commands::namespace_describe(&opts, &name).await
                }
                NamespaceCommands::Delete { name } => {
                    cli::commands::namespace_delete(&opts, &name).await
                }
            }
        }
        Some(Commands::Worker { global, command }) => {
            let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
                Ok(o) => o,
                Err(code) => return code,
            };
            match command {
                WorkerCommands::List => cli::commands::worker_list(&opts).await,
            }
        }
        Some(Commands::Queue { global, command }) => {
            let opts = match cli::GlobalOpts::resolve(global.as_flags()) {
                Ok(o) => o,
                Err(code) => return code,
            };
            match command {
                QueueCommands::Stats => cli::commands::queue_stats(&opts).await,
            }
        }
        Some(Commands::Install(args)) => install::run(args).await,
        Some(Commands::McpServe) => mcp::serve().await,
        Some(Commands::Completion { shell }) => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            // Buffer first so we can exit cleanly on a broken pipe
            // (e.g. `assay completion bash | head`): clap_complete's
            // default writer panics on BrokenPipe.
            let mut buf: Vec<u8> = Vec::new();
            clap_complete::generate(shell, &mut cmd, "assay", &mut buf);
            use std::io::Write;
            match std::io::stdout().write_all(&buf) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: writing completion: {e}");
                    ExitCode::from(1)
                }
            }
        }
        None => {
            if let Some(ref file) = cli.file {
                let options = RunOptions {
                    exec_mode,
                    approval: approval.clone(),
                    ..RunOptions::default()
                };
                dispatch_file(file, options, Vec::new()).await
            } else {
                use clap::CommandFactory;
                Cli::command().print_help().ok();
                println!();
                ExitCode::from(1)
            }
        }
    }
}

fn resolve_script_mode(cli_mode: Option<&str>) -> ScriptMode {
    match cli_mode
        .map(std::borrow::ToOwned::to_owned)
        .or_else(|| std::env::var("ASSAY_MODE").ok())
        .as_deref()
    {
        Some("tool") => ScriptMode::Tool,
        _ => ScriptMode::Script,
    }
}

async fn dispatch_file(
    file: &std::path::Path,
    options: RunOptions,
    script_args: Vec<String>,
) -> ExitCode {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        "yaml" | "yml" => run_yaml_checks(file, options.exec_mode).await,
        "lua" => run_lua_script(file, options, script_args).await,
        other => {
            eprintln!(
                "error: unsupported file extension {other:?} (expected .yaml, .yml, or .lua)"
            );
            ExitCode::from(1)
        }
    }
}

async fn run_yaml_checks(path: &std::path::Path, exec_mode: lua::ExecMode) -> ExitCode {
    let readonly = exec_mode.is_readonly();
    info!(config = %path.display(), readonly, "starting assay (check mode)");

    let cfg = match config::load(path) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("error: loading config from {}: {e:#}", path.display());
            return ExitCode::from(1);
        }
    };

    info!(
        checks = cfg.checks.len(),
        timeout_secs = cfg.timeout.as_secs(),
        retries = cfg.retries,
        "configuration loaded"
    );

    let result = runner::run(&cfg, exec_mode).await;
    result.print()
}

async fn run_lua_script(
    path: &std::path::Path,
    options: RunOptions,
    script_args: Vec<String>,
) -> ExitCode {
    let script = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: reading {}: {e}", path.display());
            return ExitCode::from(1);
        }
    };

    let script = lua::async_bridge::strip_shebang(&script);

    match options.mode {
        ScriptMode::Script => {
            run_lua_script_mode(
                path,
                script,
                script_args,
                options.exec_mode,
                &options.approval,
            )
            .await
        }
        ScriptMode::Tool => {
            run_lua_tool_mode(
                path,
                script,
                options.timeout_secs,
                script_args,
                options.exec_mode,
                &options.approval,
            )
            .await
        }
    }
}

/// Populate Lua's standard `arg` global from the trailing positional
/// arguments we collected in `Commands::Run`. Mirrors stock `lua`:
/// `arg[0]` is the script path, `arg[1..]` are the user-passed args.
fn install_script_args(
    vm: &mlua::Lua,
    path: &std::path::Path,
    script_args: &[String],
) -> mlua::Result<()> {
    let table = vm.create_table()?;
    table.set(0, path.display().to_string())?;
    for (i, a) in script_args.iter().enumerate() {
        table.set(i as i64 + 1, a.as_str())?;
    }
    vm.globals().set("arg", table)
}

async fn run_lua_script_mode(
    path: &std::path::Path,
    script: &str,
    script_args: Vec<String>,
    exec_mode: lua::ExecMode,
    approval: &lua::ApprovalConfig,
) -> ExitCode {
    let readonly = exec_mode.is_readonly();
    info!(script = %path.display(), readonly, "starting assay (script mode)");

    let client = build_http_client();

    let vm = match lua::create_vm_with_options(
        client,
        lua::VmOptions {
            global_modules_path: None,
            mode: exec_mode,
            approval: approval.clone(),
        },
    ) {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("error: creating Lua VM: {e:#}");
            return ExitCode::from(1);
        }
    };

    if let Err(e) = install_script_args(&vm, path, &script_args) {
        eprintln!("error: installing arg global: {e}");
        return ExitCode::from(1);
    }

    let local = tokio::task::LocalSet::new();
    let result = local
        .run_until(async {
            vm.load(script)
                .set_name(format!("@{}", path.display()))
                .exec_async()
                .await
        })
        .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{}", format_lua_error(&e));
            ExitCode::from(1)
        }
    }
}

async fn run_lua_tool_mode(
    path: &std::path::Path,
    script: &str,
    timeout_secs: u64,
    script_args: Vec<String>,
    exec_mode: lua::ExecMode,
    approval: &lua::ApprovalConfig,
) -> ExitCode {
    let outcome =
        execute_tool_mode(path, script, timeout_secs, script_args, exec_mode, approval).await;
    print!("{}", outcome.envelope);
    ExitCode::SUCCESS
}

/// Result of a tool-mode run: the serialized JSON envelope plus its
/// `status` field. Shared by the CLI (prints `envelope`) and the MCP
/// server (surfaces `envelope` as tool content, maps `status` to isError).
pub struct ToolModeOutcome {
    pub envelope: String,
    pub status: &'static str,
}

/// Execute a Lua script in tool mode and return the JSON envelope without
/// emitting it. `status` mirrors the envelope's own field: "ok",
/// "needs_approval", "error", or "timeout".
async fn execute_tool_mode(
    path: &std::path::Path,
    script: &str,
    timeout_secs: u64,
    script_args: Vec<String>,
    exec_mode: lua::ExecMode,
    approval: &lua::ApprovalConfig,
) -> ToolModeOutcome {
    let readonly = exec_mode.is_readonly();
    // Under a gate (read-only or approval) `env.set` is itself gated, so
    // the mode marker is injected through the check-env table rather than a
    // script prefix that would trip the gate.
    let gated = exec_mode.is_readonly() || exec_mode.is_approval();
    info!(script = %path.display(), timeout_secs, readonly, "starting assay (tool mode)");
    let tool_script = if gated {
        script.to_string()
    } else {
        format!("env.set(\"ASSAY_MODE\", \"tool\")\n{script}")
    };

    let client = build_http_client();

    let vm = match lua::create_vm_with_options(
        client,
        lua::VmOptions {
            global_modules_path: None,
            mode: exec_mode,
            approval: approval.clone(),
        },
    ) {
        Ok(vm) => vm,
        Err(e) => {
            return ToolModeOutcome {
                envelope: build_tool_error("error", format!("creating Lua VM: {e:#}"), readonly),
                status: "error",
            };
        }
    };

    if gated {
        let mode_env =
            std::collections::HashMap::from([("ASSAY_MODE".to_string(), "tool".to_string())]);
        if let Err(e) = lua::inject_env(&vm, &mode_env) {
            return ToolModeOutcome {
                envelope: build_tool_error(
                    "error",
                    format!("injecting ASSAY_MODE: {e:#}"),
                    readonly,
                ),
                status: "error",
            };
        }
    }

    if let Err(e) = install_script_args(&vm, path, &script_args) {
        return ToolModeOutcome {
            envelope: build_tool_error("error", format!("installing arg global: {e}"), readonly),
            status: "error",
        };
    }

    let local = tokio::task::LocalSet::new();
    let execution = local.run_until(async {
        vm.load(&tool_script)
            .set_name(format!("@{}", path.display()))
            .eval_async::<mlua::Value>()
            .await
    });

    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), execution).await;

    match result {
        Ok(Ok(value)) => match lua_value_to_json(&vm, value) {
            Ok(output) => ToolModeOutcome {
                envelope: build_tool_success(output, readonly),
                status: "ok",
            },
            Err(e) => ToolModeOutcome {
                envelope: build_tool_error(
                    "error",
                    format!("serializing Lua result: {e}"),
                    readonly,
                ),
                status: "error",
            },
        },
        Ok(Err(e)) => {
            if let Some(request) = extract_approval_request(&e) {
                match persist_resume_state(
                    path,
                    request,
                    exec_mode,
                    &approval.approved_indices,
                    &lua::approved_ops_from_env(),
                    &script_args,
                ) {
                    Ok(requires_approval) => ToolModeOutcome {
                        envelope: build_tool_needs_approval(requires_approval, readonly),
                        status: "needs_approval",
                    },
                    Err(err) => ToolModeOutcome {
                        envelope: build_tool_error("error", err, readonly),
                        status: "error",
                    },
                }
            } else {
                ToolModeOutcome {
                    envelope: build_tool_error("error", format_lua_error(&e), readonly),
                    status: "error",
                }
            }
        }
        Err(_) => ToolModeOutcome {
            envelope: build_tool_error(
                "timeout",
                format!("execution timed out after {timeout_secs}s"),
                readonly,
            ),
            status: "timeout",
        },
    }
}

/// Resume a suspended tool-mode run and RETURN its envelope without emitting
/// it. Shared by the CLI `resume` command (which prints the envelope) and the
/// MCP `assay_resume` tool (which surfaces it as tool content). `approve` is
/// "yes" to approve the pending operation, anything else to deny it. The
/// child's stderr is forwarded as diagnostics; its stdout is the resumed run's
/// JSON envelope, returned here with its parsed `status`.
async fn resume_tool_outcome(
    token: &str,
    approve: &str,
    resume_ttl: Option<u64>,
    readonly: bool,
    approver: Option<&str>,
) -> ToolModeOutcome {
    let err_outcome = |message: String| ToolModeOutcome {
        envelope: build_tool_error("error", message, readonly),
        status: "error",
    };

    let state_dir = match resolve_state_dir() {
        Ok(dir) => dir,
        Err(err) => return err_outcome(err),
    };

    let state_path = state_dir.join("resume").join(format!("{token}.json"));
    if !state_path.exists() {
        return err_outcome("invalid resume token".to_string());
    }

    let state = match fs::read_to_string(&state_path) {
        Ok(content) => match serde_json::from_str::<ResumeState>(&content) {
            Ok(state) => state,
            Err(err) => return err_outcome(format!("parsing resume state: {err}")),
        },
        Err(err) => return err_outcome(format!("reading resume state: {err}")),
    };

    let now = unix_timestamp_now();
    let ttl_secs = resume_ttl.unwrap_or(state.ttl_secs);
    if state.created_at.saturating_add(ttl_secs) < now {
        return err_outcome("resume token expired".to_string());
    }

    let current_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => return err_outcome(format!("locating assay binary: {err}")),
    };

    let mut command = Command::new(current_exe);
    command.args([
        "run",
        "--mode",
        "tool",
        state.script_path.to_string_lossy().as_ref(),
    ]);
    // Re-run with the original positional arguments so the `arg` global —
    // and any control flow that depends on it — is identical across runs.
    if !state.script_args.is_empty() {
        command.arg("--");
        command.args(&state.script_args);
    }
    command
        .env("ASSAY_MODE", "tool")
        .env("ASSAY_APPROVAL_RESULT", approve)
        .env("ASSAY_STATE_DIR", &state_dir);

    info!(
        decision = approve,
        approver = approver.unwrap_or("-"),
        op = state.op.as_deref().unwrap_or("-"),
        index = ?state.pending_index,
        "resume decision"
    );

    if state.mode.as_deref() == Some("approval") {
        command.env(lua::APPROVAL_ENV, "1");
        let mut approved = state.approved_indices.clone();
        let mut approved_ops = state.approved_ops.clone();
        match approve {
            "yes" => {
                if let Some(idx) = state.pending_index {
                    if !approved.contains(&idx) {
                        approved.push(idx);
                    }
                    // Bind the grant to the op it was issued for, so the
                    // re-run cannot spend it on a different operation.
                    if let Some(op) = state.op.clone()
                        && !approved_ops.iter().any(|entry| entry.index == idx)
                    {
                        approved_ops.push(lua::ApprovedOp {
                            index: idx,
                            op,
                            approver: approver.map(str::to_owned),
                        });
                    }
                }
            }
            _ => {
                if let Some(idx) = state.pending_index {
                    command.env(lua::DENIED_INDEX_ENV, idx.to_string());
                }
            }
        }
        let joined = approved
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        command.env(lua::APPROVED_INDICES_ENV, joined);
        if !approved_ops.is_empty()
            && let Ok(serialized) = serde_json::to_string(&approved_ops)
        {
            command.env(lua::APPROVED_OPS_ENV, serialized);
        }
    } else if readonly {
        command.env(lua::READONLY_ENV, "1");
    }

    let output = match command.output() {
        Ok(output) => output,
        Err(err) => return err_outcome(format!("spawning resume execution: {err}")),
    };

    // The child's stderr is diagnostic (logs) — forward it. Safe on the CLI
    // and on the MCP stdout channel alike, which only ever carries the envelope.
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }

    let envelope = String::from_utf8_lossy(&output.stdout).into_owned();
    let resumed_status = serde_json::from_slice::<JsonValue>(&output.stdout)
        .ok()
        .and_then(|json| json.get("status").cloned())
        .and_then(|status| status.as_str().map(str::to_owned));

    // Echo the audit identity in the envelope the caller sees. Best-effort:
    // a non-object envelope is passed through untouched.
    let envelope = match approver {
        Some(who) => match serde_json::from_str::<JsonValue>(&envelope) {
            Ok(JsonValue::Object(mut map)) => {
                map.insert("approver".to_string(), JsonValue::String(who.to_string()));
                serde_json::to_string(&JsonValue::Object(map)).unwrap_or(envelope)
            }
            _ => envelope,
        },
        None => envelope,
    };

    // Drop the token once the run reaches a terminal state; a still-pending
    // approval keeps it for the next resume. Best-effort: a cleanup hiccup
    // must not turn a successful resume into an error.
    let terminal = output.status.success() && resumed_status.as_deref() != Some("needs_approval");
    if terminal {
        let _ = fs::remove_file(&state_path);
    }

    let status: &'static str = match resumed_status.as_deref() {
        Some("ok") => "ok",
        Some("needs_approval") => "needs_approval",
        Some("timeout") => "timeout",
        _ => "error",
    };
    ToolModeOutcome { envelope, status }
}

/// CLI `resume` command: resume a suspended run and print its envelope.
async fn resume_tool_execution(
    token: &str,
    approve: &str,
    resume_ttl: Option<u64>,
    readonly: bool,
    approver: Option<&str>,
) -> ExitCode {
    let outcome = resume_tool_outcome(token, approve, resume_ttl, readonly, approver).await;
    if !outcome.envelope.is_empty() {
        print!("{}", outcome.envelope);
    }
    ExitCode::SUCCESS
}

async fn run_lua_inline(
    code: &str,
    exec_mode: lua::ExecMode,
    approval: &lua::ApprovalConfig,
) -> ExitCode {
    let readonly = exec_mode.is_readonly();
    info!(readonly, "starting assay (inline eval mode)");

    let client = build_http_client();

    let vm = match lua::create_vm_with_options(
        client,
        lua::VmOptions {
            global_modules_path: None,
            mode: exec_mode,
            approval: approval.clone(),
        },
    ) {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("error: creating Lua VM: {e:#}");
            return ExitCode::from(1);
        }
    };

    let script = lua::async_bridge::strip_shebang(code);

    let local = tokio::task::LocalSet::new();
    let result = local
        .run_until(async { vm.load(script).set_name("@<eval>").exec_async().await })
        .await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{}", format_lua_error(&e));
            ExitCode::from(1)
        }
    }
}

fn format_lua_error(err: &mlua::Error) -> String {
    match err {
        mlua::Error::RuntimeError(msg) => msg.clone(),
        mlua::Error::CallbackError { traceback, cause } => {
            let cause_msg = format_lua_error(cause);
            if traceback.is_empty() {
                cause_msg
            } else {
                format!("{cause_msg}\n{traceback}")
            }
        }
        other => format!("{other}"),
    }
}

fn lua_value_to_json(lua: &mlua::Lua, value: mlua::Value) -> Result<JsonValue, mlua::Error> {
    lua.from_value(value)
}

fn extract_approval_request(err: &mlua::Error) -> Option<ApprovalRequestPayload> {
    let message = format_lua_error(err);
    let start = message.find(APPROVAL_REQUEST_PREFIX)?;
    let payload = &message[start + APPROVAL_REQUEST_PREFIX.len()..];
    let json_payload = payload
        .split_once('\n')
        .map(|(json, _)| json)
        .unwrap_or(payload);
    serde_json::from_str(json_payload).ok()
}

fn persist_resume_state(
    script_path: &std::path::Path,
    request: ApprovalRequestPayload,
    mode: lua::ExecMode,
    approved_indices: &[u64],
    approved_ops: &[lua::ApprovedOp],
    script_args: &[String],
) -> Result<JsonValue, String> {
    let state_dir = resolve_state_dir()?;
    let resume_dir = state_dir.join("resume");
    fs::create_dir_all(&resume_dir)
        .map_err(|err| format!("creating resume state directory: {err}"))?;

    let token = format!("{:032x}", rand::random::<u128>());
    let resolved_script_path = if script_path.is_absolute() {
        script_path.to_path_buf()
    } else {
        match script_path.canonicalize() {
            Ok(path) => path,
            Err(_) => script_path.to_path_buf(),
        }
    };
    let mode_label = mode.is_approval().then(|| "approval".to_string());
    let state = ResumeState {
        script_path: resolved_script_path,
        approval_prompt: request.prompt.clone(),
        approval_context: request.context.clone(),
        created_at: unix_timestamp_now(),
        ttl_secs: DEFAULT_RESUME_TTL_SECS,
        mode: mode_label,
        approved_indices: approved_indices.to_vec(),
        pending_index: request.index,
        op: request.op.clone(),
        summary: request.summary.clone(),
        script_args: script_args.to_vec(),
        approved_ops: approved_ops.to_vec(),
    };

    let serialized =
        serde_json::to_vec(&state).map_err(|err| format!("serializing resume state: {err}"))?;
    fs::write(resume_dir.join(format!("{token}.json")), serialized)
        .map_err(|err| format!("writing resume state: {err}"))?;

    let mut requires = serde_json::Map::new();
    requires.insert("prompt".to_string(), JsonValue::String(request.prompt));
    requires.insert("context".to_string(), request.context);
    requires.insert("resumeToken".to_string(), JsonValue::String(token));
    if let Some(op) = request.op {
        requires.insert("op".to_string(), JsonValue::String(op));
    }
    if let Some(summary) = request.summary {
        requires.insert("summary".to_string(), JsonValue::String(summary));
    }
    if let Some(index) = request.index {
        requires.insert("index".to_string(), JsonValue::from(index));
    }
    Ok(JsonValue::Object(requires))
}

fn resolve_state_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("ASSAY_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Ok(dir) = std::env::var("OPENCLAW_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }

    match std::env::var("HOME") {
        Ok(home) => Ok(PathBuf::from(home).join(".assay").join("state")),
        Err(_) => Err("resolving state directory: HOME is not set".to_string()),
    }
}

fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn build_tool_success(output: JsonValue, readonly: bool) -> String {
    let mut envelope = ToolSuccessEnvelope {
        ok: true,
        status: "ok",
        output,
        requires_approval: None,
        truncated: None,
        readonly,
    };

    if let Ok(serialized) = serde_json::to_vec(&envelope)
        && serialized.len() > TOOL_STDOUT_CAP_BYTES
    {
        envelope = truncate_tool_envelope(envelope);
    }

    match serde_json::to_string(&envelope) {
        Ok(serialized) => serialized,
        Err(e) => build_tool_error("error", format!("serializing tool envelope: {e}"), readonly),
    }
}

fn build_tool_needs_approval(requires_approval: JsonValue, readonly: bool) -> String {
    let envelope = ToolSuccessEnvelope {
        ok: true,
        status: "needs_approval",
        output: JsonValue::Null,
        requires_approval: Some(requires_approval),
        truncated: None,
        readonly,
    };

    match serde_json::to_string(&envelope) {
        Ok(serialized) => serialized,
        Err(err) => build_tool_error(
            "error",
            format!("serializing tool envelope: {err}"),
            readonly,
        ),
    }
}

fn build_tool_error(status: &'static str, error_message: String, readonly: bool) -> String {
    let envelope = ToolErrorEnvelope {
        ok: false,
        status,
        error: error_message,
        readonly,
    };

    serde_json::to_string(&envelope).unwrap_or_else(|e| {
        format!(
            "{{\"ok\":false,\"status\":\"error\",\"error\":\"serializing tool envelope: {e}\"}}"
        )
    })
}

fn truncate_tool_envelope(mut envelope: ToolSuccessEnvelope) -> ToolSuccessEnvelope {
    let serialized_output =
        serde_json::to_string(&envelope.output).unwrap_or_else(|_| "null".to_string());
    let boundaries: Vec<usize> = serialized_output
        .char_indices()
        .map(|(idx, _)| idx)
        .chain(std::iter::once(serialized_output.len()))
        .collect();

    let suffix = if serialized_output.is_empty() {
        ""
    } else {
        "..."
    };
    let mut low = 0usize;
    let mut high = boundaries.len().saturating_sub(1);
    let mut best = JsonValue::String(suffix.to_string());

    while low <= high {
        let mid = low + (high - low) / 2;
        let candidate = format!("{}{}", &serialized_output[..boundaries[mid]], suffix);
        envelope.output = JsonValue::String(candidate.clone());
        envelope.truncated = Some(true);

        match serde_json::to_vec(&envelope) {
            Ok(serialized) if serialized.len() <= TOOL_STDOUT_CAP_BYTES => {
                best = JsonValue::String(candidate);
                low = mid.saturating_add(1);
            }
            _ => {
                if mid == 0 {
                    break;
                }
                high = mid - 1;
            }
        }
    }

    envelope.output = best;
    envelope.truncated = Some(true);
    envelope
}

fn run_modules(exec_mode: lua::ExecMode) -> ExitCode {
    use assay::discovery::{ModuleSource, discover_modules};

    let modules = discover_modules();

    // Deduplicate by name (Project > Global > BuiltIn priority already in order)
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<_> = modules
        .into_iter()
        .filter(|m| seen.insert(m.module_name.clone()))
        .collect();

    // Sort alphabetically for consistent output
    unique.sort_by(|a, b| a.module_name.cmp(&b.module_name));

    // Print header
    println!("{:<30} {:<10} DESCRIPTION", "MODULE", "SOURCE");
    println!("{}", "-".repeat(80));

    for m in &unique {
        let source_label = match m.source {
            ModuleSource::BuiltIn => "builtin",
            ModuleSource::Project => "project",
            ModuleSource::Global => "global",
        };
        println!(
            "{:<30} {:<10} {}",
            m.module_name, source_label, m.metadata.description
        );
    }

    if exec_mode.is_readonly() {
        println!();
        println!(
            "read-only mode active: mutating builtins raise 'readonly: <name> blocked' errors"
        );
    } else if exec_mode.is_approval() {
        println!();
        println!("approval mode active: mutating builtins pause for per-operation approval");
    }

    ExitCode::SUCCESS
}

fn run_context(query: &str, limit: usize) -> ExitCode {
    match render_context(query, limit) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

/// Render prompt-ready module context Markdown for `query`, returning the
/// same text the `assay context` CLI prints. Shared by the CLI path and
/// the MCP `assay_context` tool.
fn render_context(query: &str, limit: usize) -> Result<String, String> {
    use assay::context::{ModuleContextEntry, QuickRefEntry, format_context};
    use assay::discovery::{discover_modules, search_modules};

    // Run on a dedicated thread to avoid tokio runtime nesting.
    // FTS5Index creates its own tokio::Runtime for SQLite operations,
    // which panics if called from within the #[tokio::main] context.
    let query = query.to_string();
    let handle = std::thread::spawn(move || {
        let results = search_modules(&query, limit);
        let all_modules = discover_modules();

        let entries: Vec<ModuleContextEntry> = results
            .iter()
            .filter_map(|result| {
                all_modules
                    .iter()
                    .find(|m| m.module_name == result.id)
                    .map(|m| ModuleContextEntry {
                        module_name: m.module_name.clone(),
                        description: m.metadata.description.clone(),
                        env_vars: m.metadata.env_vars.clone(),
                        quickrefs: m
                            .metadata
                            .quickrefs
                            .iter()
                            .map(|qr| QuickRefEntry {
                                signature: qr.signature.clone(),
                                return_hint: qr.return_hint.clone(),
                                description: qr.description.clone(),
                            })
                            .collect(),
                    })
            })
            .collect();

        format_context(&entries)
    });

    handle
        .join()
        .map_err(|_| "context search failed".to_string())
}
