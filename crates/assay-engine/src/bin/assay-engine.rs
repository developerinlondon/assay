//! Standalone assay-engine binary.
//!
//! Loads a TOML config, connects to the backend, runs migrations via
//! `{Postgres,Sqlite}Store::new` (which migrate on first connect), and
//! serves the composed router on the configured port. Also exposes a
//! `seed-sample` subcommand that drives a running engine over HTTP to
//! populate fixture users, OIDC clients, Zanzibar tuples, and demo
//! workflows so operators can play with the consoles immediately.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "assay-engine", version, about = "Assay workflow + auth engine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Run the HTTP server from a TOML config file.
    Serve {
        /// Path to the TOML config file.
        #[arg(long, short, env = "ASSAY_ENGINE_CONFIG")]
        config: PathBuf,
    },
    /// Populate a running engine with sample data (idempotent).
    ///
    /// Drives the running engine's HTTP API as an admin client. Re-run
    /// any time — every insert is guarded by a list-then-skip check or
    /// uses an upsert endpoint, so duplicate runs are no-ops.
    SeedSample {
        /// Path to the TOML config file. Used to discover the engine's
        /// public URL so operators don't have to repeat it. The
        /// `--base-url` override wins when both are set.
        #[arg(long, short, env = "ASSAY_ENGINE_CONFIG")]
        config: Option<PathBuf>,
        /// Base URL of the running engine (e.g. http://localhost:8420).
        /// Falls back to `server.public_url` from the config when unset.
        #[arg(long, env = "ASSAY_ENGINE_BASE_URL")]
        base_url: Option<String>,
        /// Admin api-key — must be present in `auth.admin_api_keys` of
        /// the running engine. Required because the seeder mints users,
        /// OIDC clients, and Zanzibar tuples via `/admin/*` endpoints.
        #[arg(long, env = "ASSAY_ADMIN_KEY")]
        admin_key: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { config } => {
            let cfg = match assay_engine::EngineConfig::from_file(&config) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("config error: {e:#}");
                    return ExitCode::from(2);
                }
            };
            init_tracing(&cfg.logging.level, &cfg.logging.format);
            if let Err(e) = assay_engine::run(cfg).await {
                eprintln!("engine error: {e:#}");
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Command::SeedSample {
            config,
            base_url,
            admin_key,
        } => {
            // Resolve the base URL: explicit flag wins; otherwise pull
            // it from the config's `server.public_url`. We don't init
            // tracing for this path — the seed reports straight to
            // stdout in a table-ish format.
            let base = match (base_url, config.as_ref()) {
                (Some(u), _) => u,
                (None, Some(p)) => match assay_engine::EngineConfig::from_file(p) {
                    Ok(cfg) => cfg.server.public_url,
                    Err(e) => {
                        eprintln!("read config {}: {e:#}", p.display());
                        return ExitCode::from(2);
                    }
                },
                (None, None) => {
                    eprintln!("seed-sample: pass --base-url or --config");
                    return ExitCode::from(2);
                }
            };
            match assay_engine::seed::run(&base, &admin_key).await {
                Ok(report) => {
                    print_report(&base, &report);
                    let any_failed = report
                        .iter()
                        .any(|r| matches!(r.status, assay_engine::seed::SeedStatus::Failed(_)));
                    if any_failed {
                        ExitCode::from(1)
                    } else {
                        ExitCode::SUCCESS
                    }
                }
                Err(e) => {
                    eprintln!("seed error: {e:#}");
                    ExitCode::from(1)
                }
            }
        }
    }
}

fn init_tracing(level: &str, format: &str) {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let builder = fmt().with_env_filter(filter);
    match format {
        "json" => builder.json().init(),
        _ => builder.init(),
    }
}

fn print_report(base: &str, report: &[assay_engine::seed::SeedReport]) {
    println!("seed-sample → {base}");
    println!("{:<16} {:<48} status", "kind", "name");
    let dashes = "-".repeat(80);
    println!("{dashes}");
    for r in report {
        let detail = match &r.status {
            assay_engine::seed::SeedStatus::Failed(e) => format!("failed: {e}"),
            assay_engine::seed::SeedStatus::Skipped(why) => format!("skipped: {why}"),
            other => other.label().to_string(),
        };
        println!("{:<16} {:<48} {detail}", r.kind, r.name);
    }
}
