mod checks;
mod config;
mod lua;
mod output;
mod runner;

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use tracing::info;
use tracing_subscriber::EnvFilter;

/// Assay — lightweight deployment verification runner.
///
/// Runs verification checks defined in a YAML config file.
/// Returns structured JSON results and exits 0 (all pass) or 1 (any fail).
#[derive(Parser, Debug)]
#[command(name = "assay", version, about)]
struct Cli {
    /// Path to the checks YAML config file.
    #[arg(short, long, default_value = "checks.yaml")]
    config: PathBuf,

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

    info!(config = %cli.config.display(), "starting assay");

    let cfg = match config::load(&cli.config) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("error: loading config from {}: {e:#}", cli.config.display());
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
