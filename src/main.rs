mod checks;
mod config;
mod lua;
mod output;
mod runner;

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
    },
    Resume {
        #[arg(long)]
        token: String,
        #[arg(long, value_parser = ["yes", "no"])]
        approve: String,
        #[arg(long, default_value = "3600")]
        resume_ttl: Option<u64>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScriptMode {
    Script,
    Tool,
}

#[derive(Clone, Copy, Debug)]
struct RunOptions {
    mode: ScriptMode,
    timeout_secs: u64,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            mode: resolve_script_mode(None),
            timeout_secs: DEFAULT_TOOL_TIMEOUT_SECS,
        }
    }
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
}

#[derive(Serialize)]
struct ToolErrorEnvelope {
    ok: bool,
    status: &'static str,
    error: String,
}

#[derive(Deserialize)]
struct ApprovalRequestPayload {
    prompt: String,
    #[serde(default)]
    context: JsonValue,
}

#[derive(Deserialize, Serialize)]
struct ResumeState {
    script_path: PathBuf,
    approval_prompt: String,
    approval_context: JsonValue,
    created_at: u64,
    ttl_secs: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

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
                run_lua_inline(&code).await
            } else if let Some(path) = file {
                run_lua_script(&path, RunOptions::default()).await
            } else {
                eprintln!("error: exec requires either -e <code> or a file path");
                ExitCode::from(1)
            }
        }
        Some(Commands::Modules) => run_modules(),
        Some(Commands::Run {
            file,
            mode,
            timeout,
        }) => {
            let options = RunOptions {
                mode: resolve_script_mode(mode.as_deref()),
                timeout_secs: timeout.unwrap_or(DEFAULT_TOOL_TIMEOUT_SECS),
            };
            dispatch_file(&file, options).await
        }
        Some(Commands::Resume {
            token,
            approve,
            resume_ttl,
        }) => resume_tool_execution(&token, &approve, resume_ttl).await,
        None => {
            if let Some(ref file) = cli.file {
                dispatch_file(file, RunOptions::default()).await
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

async fn dispatch_file(file: &std::path::Path, options: RunOptions) -> ExitCode {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        "yaml" | "yml" => run_yaml_checks(file).await,
        "lua" => run_lua_script(file, options).await,
        other => {
            eprintln!(
                "error: unsupported file extension {other:?} (expected .yaml, .yml, or .lua)"
            );
            ExitCode::from(1)
        }
    }
}

async fn run_yaml_checks(path: &std::path::Path) -> ExitCode {
    info!(config = %path.display(), "starting assay (check mode)");

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

    let result = runner::run(&cfg).await;
    result.print()
}

async fn run_lua_script(path: &std::path::Path, options: RunOptions) -> ExitCode {
    let script = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: reading {}: {e}", path.display());
            return ExitCode::from(1);
        }
    };

    let script = lua::async_bridge::strip_shebang(&script);

    match options.mode {
        ScriptMode::Script => run_lua_script_mode(path, script).await,
        ScriptMode::Tool => run_lua_tool_mode(path, script, options.timeout_secs).await,
    }
}

async fn run_lua_script_mode(path: &std::path::Path, script: &str) -> ExitCode {
    info!(script = %path.display(), "starting assay (script mode)");

    let client = build_http_client();

    let vm = match lua::create_vm(client) {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("error: creating Lua VM: {e:#}");
            return ExitCode::from(1);
        }
    };

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

async fn run_lua_tool_mode(path: &std::path::Path, script: &str, timeout_secs: u64) -> ExitCode {
    info!(script = %path.display(), timeout_secs, "starting assay (tool mode)");
    let tool_script = format!("env.set(\"ASSAY_MODE\", \"tool\")\n{script}");

    let client = build_http_client();

    let vm = match lua::create_vm(client) {
        Ok(vm) => vm,
        Err(e) => {
            emit_tool_error("error", format!("creating Lua VM: {e:#}"));
            return ExitCode::SUCCESS;
        }
    };

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
            Ok(output) => {
                emit_tool_success(output);
                ExitCode::SUCCESS
            }
            Err(e) => {
                emit_tool_error("error", format!("serializing Lua result: {e}"));
                ExitCode::SUCCESS
            }
        },
        Ok(Err(e)) => {
            if let Some(request) = extract_approval_request(&e) {
                match persist_resume_state(path, request) {
                    Ok(requires_approval) => emit_tool_needs_approval(requires_approval),
                    Err(err) => emit_tool_error("error", err),
                }
            } else {
                emit_tool_error("error", format_lua_error(&e));
            }
            ExitCode::SUCCESS
        }
        Err(_) => {
            emit_tool_error(
                "timeout",
                format!("execution timed out after {timeout_secs}s"),
            );
            ExitCode::SUCCESS
        }
    }
}

async fn resume_tool_execution(token: &str, approve: &str, resume_ttl: Option<u64>) -> ExitCode {
    let state_dir = match resolve_state_dir() {
        Ok(dir) => dir,
        Err(err) => {
            emit_tool_error("error", err);
            return ExitCode::SUCCESS;
        }
    };

    let state_path = state_dir.join("resume").join(format!("{token}.json"));
    if !state_path.exists() {
        emit_tool_error("error", "invalid resume token".to_string());
        return ExitCode::SUCCESS;
    }

    let state = match fs::read_to_string(&state_path) {
        Ok(content) => match serde_json::from_str::<ResumeState>(&content) {
            Ok(state) => state,
            Err(err) => {
                emit_tool_error("error", format!("parsing resume state: {err}"));
                return ExitCode::SUCCESS;
            }
        },
        Err(err) => {
            emit_tool_error("error", format!("reading resume state: {err}"));
            return ExitCode::SUCCESS;
        }
    };

    let now = unix_timestamp_now();
    let ttl_secs = resume_ttl.unwrap_or(state.ttl_secs);
    if state.created_at.saturating_add(ttl_secs) < now {
        emit_tool_error("error", "resume token expired".to_string());
        return ExitCode::SUCCESS;
    }

    let current_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            emit_tool_error("error", format!("locating assay binary: {err}"));
            return ExitCode::SUCCESS;
        }
    };

    let output = match Command::new(current_exe)
        .args([
            "run",
            "--mode",
            "tool",
            state.script_path.to_string_lossy().as_ref(),
        ])
        .env("ASSAY_MODE", "tool")
        .env("ASSAY_APPROVAL_RESULT", approve)
        .env("ASSAY_STATE_DIR", &state_dir)
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            emit_tool_error("error", format!("spawning resume execution: {err}"));
            return ExitCode::SUCCESS;
        }
    };

    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    if !output.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }

    let resumed_status = serde_json::from_slice::<JsonValue>(&output.stdout)
        .ok()
        .and_then(|json| json.get("status").cloned())
        .and_then(|status| status.as_str().map(str::to_owned));
    let should_cleanup =
        output.status.success() && resumed_status.as_deref() != Some("needs_approval");

    if should_cleanup && let Err(err) = fs::remove_file(&state_path) {
        emit_tool_error("error", format!("cleaning up resume state: {err}"));
        return ExitCode::SUCCESS;
    }

    ExitCode::SUCCESS
}

async fn run_lua_inline(code: &str) -> ExitCode {
    info!("starting assay (inline eval mode)");

    let client = build_http_client();

    let vm = match lua::create_vm(client) {
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
    let state = ResumeState {
        script_path: resolved_script_path,
        approval_prompt: request.prompt.clone(),
        approval_context: request.context.clone(),
        created_at: unix_timestamp_now(),
        ttl_secs: DEFAULT_RESUME_TTL_SECS,
    };

    let serialized =
        serde_json::to_vec(&state).map_err(|err| format!("serializing resume state: {err}"))?;
    fs::write(resume_dir.join(format!("{token}.json")), serialized)
        .map_err(|err| format!("writing resume state: {err}"))?;

    Ok(serde_json::json!({
        "prompt": request.prompt,
        "context": request.context,
        "resumeToken": token,
    }))
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

fn emit_tool_success(output: JsonValue) {
    let mut envelope = ToolSuccessEnvelope {
        ok: true,
        status: "ok",
        output,
        requires_approval: None,
        truncated: None,
    };

    if let Ok(serialized) = serde_json::to_vec(&envelope)
        && serialized.len() > TOOL_STDOUT_CAP_BYTES
    {
        envelope = truncate_tool_envelope(envelope);
    }

    match serde_json::to_string(&envelope) {
        Ok(serialized) => print!("{serialized}"),
        Err(e) => emit_tool_error("error", format!("serializing tool envelope: {e}")),
    }
}

fn emit_tool_needs_approval(requires_approval: JsonValue) {
    let envelope = ToolSuccessEnvelope {
        ok: true,
        status: "needs_approval",
        output: JsonValue::Null,
        requires_approval: Some(requires_approval),
        truncated: None,
    };

    match serde_json::to_string(&envelope) {
        Ok(serialized) => print!("{serialized}"),
        Err(err) => emit_tool_error("error", format!("serializing tool envelope: {err}")),
    }
}

fn emit_tool_error(status: &'static str, error_message: String) {
    let envelope = ToolErrorEnvelope {
        ok: false,
        status,
        error: error_message,
    };

    match serde_json::to_string(&envelope) {
        Ok(serialized) => print!("{serialized}"),
        Err(e) => print!(
            "{{\"ok\":false,\"status\":\"error\",\"error\":\"serializing tool envelope: {e}\"}}"
        ),
    }
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

fn run_modules() -> ExitCode {
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

    ExitCode::SUCCESS
}

fn run_context(query: &str, limit: usize) -> ExitCode {
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

    match handle.join() {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(_) => {
            eprintln!("error: context search failed");
            ExitCode::from(1)
        }
    }
}
