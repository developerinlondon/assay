//! `assay install` — fetch + verify + extract the binaries and Lua libraries
//! declared in a `Manifest.lua`.
//!
//! See `.claude/plans/21-libs-folder-and-install.md`.
//!
//! ## Pipeline
//!
//! 1. Read `Manifest.lua`, parse in a sandboxed mlua VM (`manifest`).
//! 2. Resolve dirs (cache / bin / lib) from CLI flags + per-user XDG
//!    fallbacks (root falls back to `/var/cache/assay`, `/usr/local/bin`,
//!    `/opt/assay/libs`).
//! 3. Build a [`fetch::FetchPlan`] per declared dep.
//! 4. With `--dry-run`: print the resolved plan and exit.
//! 5. Otherwise: fetch all deps in parallel via `tokio::spawn` (cache
//!    hits skip HTTP; sha256 verified for every artifact).
//! 6. Extract sequentially: extension binaries via
//!    `extract::install_extension_binary`; lib trees via
//!    `extract::install_lib_tree`.
//! 7. Write `Manifest.lock` next to the input manifest.
//!
//! Failures abort early. Cached entries from successful fetches are
//! preserved; bin / lib trees from earlier extractions are NOT rolled
//! back (matching plan 21's design, finalised treatment in phase 5).

pub mod extract;
pub mod fetch;
pub mod lock;
pub mod manifest;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use thiserror::Error;

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Path to the `Manifest.lua` declaring extensions + libs to install.
    #[arg(short = 'f', long, default_value = "./Manifest.lua")]
    pub manifest: PathBuf,

    /// Cache directory for downloaded artifacts.
    /// Default: `/var/cache/assay/` (root) or `$XDG_CACHE_HOME/assay/` (per-user).
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,

    /// Where to install extension binaries.
    /// Default: `/usr/local/bin/` (root) or `$HOME/.local/bin/` (per-user).
    #[arg(long)]
    pub bin_dir: Option<PathBuf>,

    /// Where to extract Lua libraries.
    /// Default: `/opt/assay/libs/` (root) or `$XDG_DATA_HOME/assay/libs/` (per-user).
    #[arg(long)]
    pub lib_dir: Option<PathBuf>,

    /// Skip network fetch; require every dep already present in the cache.
    #[arg(long)]
    pub offline: bool,

    /// Resolve and report only; do not write any files.
    #[arg(long)]
    pub dry_run: bool,

    /// Suppress per-dep progress output to stderr.
    #[arg(long)]
    pub no_progress: bool,
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("read {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    Manifest(#[from] manifest::ManifestError),

    #[error(transparent)]
    Fetch(#[from] fetch::FetchError),

    #[error(transparent)]
    Extract(#[from] extract::ExtractError),

    #[error("{count} dep(s) failed to fetch; see errors above")]
    FetchFailed { count: usize },

    #[error("write {path}: {source}")]
    WriteLock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Top-level entry point invoked by the `Install` clap subcommand.
pub async fn run(args: InstallArgs) -> ExitCode {
    match execute(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("assay install: {e}");
            let mut src = std::error::Error::source(&e);
            while let Some(s) = src {
                eprintln!("  caused by: {s}");
                src = std::error::Error::source(s);
            }
            ExitCode::from(1)
        }
    }
}

/// Run the install pipeline, returning a typed result. Useful for tests
/// that need to inspect the error path; the CLI entry [`run`] converts
/// this into an `ExitCode`.
pub async fn execute(args: InstallArgs) -> Result<(), InstallError> {
    // 1. Read + parse manifest.
    let source =
        std::fs::read_to_string(&args.manifest).map_err(|e| InstallError::ReadManifest {
            path: args.manifest.clone(),
            source: e,
        })?;
    let m = manifest::parse(&source, &args.manifest)?;

    // 2. Resolve dirs.
    let cache_dir = args.cache_dir.clone().unwrap_or_else(default_cache_dir);
    let bin_dir = args.bin_dir.clone().unwrap_or_else(default_bin_dir);
    let lib_dir = args.lib_dir.clone().unwrap_or_else(default_lib_dir);
    let arch = std::env::consts::ARCH.to_string();

    // 3. Build plans.
    let mut ext_plans: Vec<(manifest::Extension, fetch::FetchPlan)> = Vec::new();
    for e in &m.extensions {
        let plan = fetch::FetchPlan::for_extension(e, &arch, &cache_dir)?;
        ext_plans.push((e.clone(), plan));
    }
    let mut lib_plans: Vec<(manifest::Lib, fetch::FetchPlan)> = Vec::new();
    for l in &m.libs {
        lib_plans.push((l.clone(), fetch::FetchPlan::for_lib(l, &cache_dir)));
    }

    // 4. Dry-run: print plan and bail.
    if args.dry_run {
        print_plan(&m, &ext_plans, &lib_plans, &cache_dir, &bin_dir, &lib_dir);
        return Ok(());
    }

    // 5. Fetch everything in parallel.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("build reqwest client");

    let mut handles = Vec::with_capacity(ext_plans.len() + lib_plans.len());
    let all_plans = ext_plans
        .iter()
        .map(|(_, p)| p)
        .chain(lib_plans.iter().map(|(_, p)| p));
    for plan in all_plans {
        let plan = plan.clone();
        let client = client.clone();
        let offline = args.offline;
        handles.push(tokio::spawn(async move {
            let result = fetch::fetch(&plan, &client, offline).await;
            (plan.display_name.clone(), result)
        }));
    }

    let mut failures = 0;
    for h in handles {
        let (name, res) = h.await.expect("fetch task panicked");
        match res {
            Ok(()) => {
                if !args.no_progress {
                    eprintln!("  ✓ fetched {name}");
                }
            }
            Err(e) => {
                eprintln!("  ✗ {name}: {e}");
                failures += 1;
            }
        }
    }
    if failures > 0 {
        return Err(InstallError::FetchFailed { count: failures });
    }

    // 6. Extract.
    for (e, plan) in &ext_plans {
        extract::install_extension_binary(&plan.cache_path, &bin_dir, &e.name)?;
        if !args.no_progress {
            eprintln!(
                "  → installed {} → {}",
                e.name,
                bin_dir.join(&e.name).display()
            );
        }
    }
    for (l, plan) in &lib_plans {
        extract::install_lib_tree(&plan.cache_path, &lib_dir, &l.name)?;
        if !args.no_progress {
            eprintln!(
                "  → installed {} → {}/",
                l.name,
                lib_dir.join(&l.name).display()
            );
        }
    }

    // 7. Write Manifest.lock alongside the input manifest.
    let lockfile = lock::Lockfile {
        assay: m.assay.clone(),
        extensions: ext_plans
            .iter()
            .map(|(e, p)| lock::LockExtension::new(e, p))
            .collect(),
        libs: lib_plans
            .iter()
            .map(|(l, p)| lock::LockLib::new(l, p))
            .collect(),
    };
    let lock_path = args.manifest.with_file_name("Manifest.lock");
    std::fs::write(&lock_path, lockfile.to_lua()).map_err(|e| InstallError::WriteLock {
        path: lock_path.clone(),
        source: e,
    })?;
    if !args.no_progress {
        eprintln!("  → wrote {}", lock_path.display());
    }

    Ok(())
}

fn print_plan(
    m: &manifest::Manifest,
    ext_plans: &[(manifest::Extension, fetch::FetchPlan)],
    lib_plans: &[(manifest::Lib, fetch::FetchPlan)],
    cache_dir: &Path,
    bin_dir: &Path,
    lib_dir: &Path,
) {
    println!("# assay install (dry run)");
    if let Some(v) = &m.assay {
        println!("assay = {v}");
    }
    println!("cache_dir = {}", cache_dir.display());
    println!("bin_dir   = {}", bin_dir.display());
    println!("lib_dir   = {}", lib_dir.display());
    println!();
    println!("extensions:");
    for (_, p) in ext_plans {
        println!("  {} (sha256 {})", p.display_name, p.expected_sha256);
        println!("    url   = {}", p.url);
        println!("    cache = {}", p.cache_path.display());
    }
    println!();
    println!("libs:");
    for (_, p) in lib_plans {
        println!("  {} (sha256 {})", p.display_name, p.expected_sha256);
        println!("    url   = {}", p.url);
        println!("    cache = {}", p.cache_path.display());
    }
}

// --- default dir resolution ------------------------------------------

fn default_cache_dir() -> PathBuf {
    if is_root() {
        PathBuf::from("/var/cache/assay")
    } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("assay")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".cache/assay")
    } else {
        PathBuf::from("/tmp/assay-cache")
    }
}

fn default_bin_dir() -> PathBuf {
    if is_root() {
        PathBuf::from("/usr/local/bin")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".local/bin")
    } else {
        PathBuf::from("/usr/local/bin")
    }
}

fn default_lib_dir() -> PathBuf {
    if let Some(explicit) = std::env::var_os("ASSAY_LIB_DIR") {
        return PathBuf::from(explicit);
    }
    if is_root() {
        PathBuf::from("/opt/assay/libs")
    } else if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        PathBuf::from(xdg).join("assay/libs")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".local/share/assay/libs")
    } else {
        PathBuf::from("/opt/assay/libs")
    }
}

#[cfg(unix)]
fn is_root() -> bool {
    // SAFETY: getuid is always safe — no preconditions, no shared state.
    unsafe { libc::getuid() == 0 }
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}
