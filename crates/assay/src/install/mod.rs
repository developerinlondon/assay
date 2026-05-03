//! `assay install` — fetch + verify + extract the binaries and Lua libraries
//! declared in a `Manifest.lua`.
//!
//! See `.claude/plans/21-libs-folder-and-install.md`.

pub mod extract;
pub mod fetch;
pub mod manifest;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;

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
    /// Default: `/usr/local/bin/` (root) or `$XDG_BIN_HOME` (per-user).
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

pub async fn run(_args: InstallArgs) -> ExitCode {
    eprintln!("assay install: not yet implemented (phase 1 scaffolding)");
    ExitCode::from(1)
}
