//! Standalone assay-engine binary.
//!
//! Loads a TOML config, connects to the backend, runs migrations via
//! `{Postgres,Sqlite}Store::new` (which migrate on first connect), and
//! serves the composed router on the configured port.
//!
//! First-time setup is done from the assay-lua client — see
//! `examples/init/init.lua` for the canonical bootstrap script that
//! seeds Zanzibar namespaces, creates the admin user, and writes the
//! operator-grant tuples in one shot using `auth.admin_api_keys` as
//! the break-glass.

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
    /// Copy a stopped SQLite store into an empty Postgres one.
    #[cfg(all(feature = "backend-postgres", feature = "backend-sqlite"))]
    Migrate {
        /// SQLite data directory, as `sqlite:<dir>` or a bare path.
        #[arg(long)]
        from: String,
        /// Target Postgres URL, holding no engine data.
        #[arg(long)]
        to: String,
        /// Print the plan and row counts; write nothing.
        #[arg(long)]
        dry_run: bool,
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
        #[cfg(all(feature = "backend-postgres", feature = "backend-sqlite"))]
        Command::Migrate { from, to, dry_run } => migrate(&from, &to, dry_run).await,
    }
}

#[cfg(all(feature = "backend-postgres", feature = "backend-sqlite"))]
async fn migrate(from: &str, to: &str, dry_run: bool) -> ExitCode {
    use assay_engine::migrate::{Plan, parse_source, parse_target};

    // The report is the output; sqlx logs every `IF NOT EXISTS` notice
    // the schema bootstrap raises, which is noise around it.
    init_tracing("info,sqlx=warn", "pretty");
    let plan = match (parse_source(from), parse_target(to)) {
        (Ok(source_dir), Ok(target_url)) => Plan {
            source_dir,
            target_url,
            dry_run,
        },
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("migrate: {e:#}");
            return ExitCode::from(2);
        }
    };
    match assay_engine::migrate::run(plan).await {
        Ok(report) => {
            print!("{}", report.render());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("migrate failed: {e:#}");
            ExitCode::from(1)
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
