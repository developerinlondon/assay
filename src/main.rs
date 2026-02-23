mod checks;
mod config;
mod lua;
mod output;
mod runner;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

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
    },
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
        Some(Commands::Context { .. }) => {
            println!("context: not yet implemented");
            ExitCode::SUCCESS
        }
        Some(Commands::Exec { .. }) => {
            println!("exec: not yet implemented");
            ExitCode::SUCCESS
        }
        Some(Commands::Modules) => {
            println!("modules: not yet implemented");
            ExitCode::SUCCESS
        }
        Some(Commands::Run { file }) => dispatch_file(&file).await,
        None => {
            if let Some(ref file) = cli.file {
                dispatch_file(file).await
            } else {
                use clap::CommandFactory;
                Cli::command().print_help().ok();
                println!();
                ExitCode::from(1)
            }
        }
    }
}

async fn dispatch_file(file: &std::path::Path) -> ExitCode {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        "yaml" | "yml" => run_yaml_checks(file).await,
        "lua" => run_lua_script(file).await,
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

async fn run_lua_script(path: &std::path::Path) -> ExitCode {
    info!(script = %path.display(), "starting assay (script mode)");

    let script = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: reading {}: {e}", path.display());
            return ExitCode::from(1);
        }
    };

    let client = build_http_client();

    let vm = match lua::create_vm(client) {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("error: creating Lua VM: {e:#}");
            return ExitCode::from(1);
        }
    };

    let script = lua::async_bridge::strip_shebang(&script);

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
