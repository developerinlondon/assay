mod checks;
mod config;
mod lua;
mod output;
mod runner;

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

/// Assay — lightweight Lua runtime for Kubernetes.
///
/// Auto-detects behavior by file extension:
///   assay checks.yaml    YAML check orchestration (retry, backoff, structured output)
///   assay script.lua     Direct Lua script execution (all builtins available)
#[derive(Parser, Debug)]
#[command(name = "assay", version, about)]
struct Cli {
    /// Path to a .yaml config or .lua script.
    file: PathBuf,

    /// Enable verbose logging (sets RUST_LOG=debug).
    #[arg(short, long)]
    verbose: bool,
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

    let ext = cli
        .file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext {
        "yaml" | "yml" => run_yaml_checks(&cli.file).await,
        "lua" => run_lua_script(&cli.file).await,
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

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("building HTTP client");

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
