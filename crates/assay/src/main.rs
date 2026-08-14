mod api;
mod checks;
mod cli;
mod config;
mod lua;
mod mcp;
mod output;
mod runner;
mod tool_mode;

use std::process::ExitCode;
use std::time::Duration;

use assay::install;
use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use cli::args::{Cli, Commands};
use tool_mode::{format_lua_error, resume_tool_execution, run_lua_tool_mode};

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
        Some(Commands::Context {
            query,
            limit,
            no_builtins,
        }) => run_context(&query, limit, !no_builtins),
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
        Some(Commands::Modules { json }) => run_modules(exec_mode, json),
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
            cli::dispatch::workflow(global, command).await
        }
        Some(Commands::Schedule { global, command }) => {
            cli::dispatch::schedule(global, command).await
        }
        Some(Commands::Namespace { global, command }) => {
            cli::dispatch::namespace(global, command).await
        }
        Some(Commands::Worker { global, command }) => cli::dispatch::worker(global, command).await,
        Some(Commands::Queue { global, command }) => cli::dispatch::queue(global, command).await,
        Some(Commands::Install(args)) => install::run(args).await,
        Some(Commands::McpServe) => mcp::serve().await,
        Some(Commands::ApiServe { bind }) => api::serve(&bind).await,
        Some(Commands::Completion { shell }) => run_completion(shell),
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

fn run_completion(shell: clap_complete::Shell) -> ExitCode {
    use clap::CommandFactory;
    use std::io::Write;

    let mut cmd = Cli::command();
    // Buffer first so we can exit cleanly on a broken pipe
    // (e.g. `assay completion bash | head`): clap_complete's
    // default writer panics on BrokenPipe.
    let mut buf: Vec<u8> = Vec::new();
    clap_complete::generate(shell, &mut cmd, "assay", &mut buf);
    match std::io::stdout().write_all(&buf) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: writing completion: {e}");
            ExitCode::from(1)
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
            run_lua_tool_mode(tool_mode::ToolModeRequest {
                path,
                script,
                timeout_secs: options.timeout_secs,
                script_args,
                exec_mode: options.exec_mode,
                approval: &options.approval,
            })
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
fn run_modules(exec_mode: lua::ExecMode, json: bool) -> ExitCode {
    use assay::discovery::discover_modules;

    let modules = discover_modules();

    // Deduplicate by name (Project > Global > BuiltIn priority already in order)
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<_> = modules
        .into_iter()
        .filter(|m| seen.insert(m.module_name.clone()))
        .collect();

    // Sort alphabetically for consistent output
    unique.sort_by(|a, b| a.module_name.cmp(&b.module_name));

    if json {
        // The payload is big enough that `| head` is the natural way to eyeball
        // it, and the default println! panics on the resulting broken pipe.
        use std::io::Write;
        return match writeln!(std::io::stdout(), "{}", modules_json(&unique)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: writing modules json: {e}");
                ExitCode::from(1)
            }
        };
    }

    // Print header
    println!("{:<30} {:<10} DESCRIPTION", "MODULE", "SOURCE");
    println!("{}", "-".repeat(80));

    for m in &unique {
        println!(
            "{:<30} {:<10} {}",
            m.module_name,
            m.source.label(),
            m.metadata.description
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

/// The `assay modules --json` payload. The `version` envelope lets a consumer
/// cache the metadata and invalidate it when the binary changes.
#[derive(serde::Serialize)]
struct ModulesPayload<'a> {
    version: &'a str,
    modules: Vec<ModuleEntry<'a>>,
}

#[derive(serde::Serialize)]
struct ModuleEntry<'a> {
    name: &'a str,
    source: &'a str,
    description: &'a str,
    keywords: &'a [String],
    env_vars: &'a [String],
    quickrefs: &'a [assay::metadata::QuickRef],
    auto_functions: &'a [String],
    icon: Option<&'a str>,
    category: Option<&'a str>,
}

fn modules_json(modules: &[assay::discovery::DiscoveredModule]) -> String {
    let payload = ModulesPayload {
        version: env!("CARGO_PKG_VERSION"),
        modules: modules
            .iter()
            .map(|m| ModuleEntry {
                name: &m.module_name,
                source: m.source.label(),
                description: &m.metadata.description,
                keywords: &m.metadata.keywords,
                env_vars: &m.metadata.env_vars,
                quickrefs: &m.metadata.quickrefs,
                auto_functions: &m.metadata.auto_functions,
                icon: m.metadata.icon.as_deref(),
                category: m.metadata.category.as_deref(),
            })
            .collect(),
    };

    serde_json::to_string_pretty(&payload).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

fn run_context(query: &str, limit: usize, include_builtins: bool) -> ExitCode {
    match render_context(query, limit, include_builtins) {
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
fn render_context(query: &str, limit: usize, include_builtins: bool) -> Result<String, String> {
    use assay::context::{
        ModuleContextEntry, QuickRefEntry, format_context, format_context_without_builtins,
    };
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

        if include_builtins {
            format_context(&entries)
        } else {
            format_context_without_builtins(&entries)
        }
    });

    handle
        .join()
        .map_err(|_| "context search failed".to_string())
}
