//! `assay` CLI subcommand implementations for the workflow engine.
//!
//! These are thin HTTP clients over the workflow engine's REST API. The
//! `assay.workflow` Lua stdlib mirrors the same surface programmatically;
//! this CLI exists for operators at a terminal (kubectl exec into a pod,
//! ad-hoc debugging, shell scripts). Lua scripts are the preferred path
//! for automation.

pub mod client;
pub mod commands;
pub mod config;
pub mod input;
pub mod output;
pub mod table;

use std::process::ExitCode;

pub use output::Output;

/// Global config resolved from (in precedence order): CLI flags,
/// environment variables, optional YAML config file, hardcoded defaults.
#[derive(Clone, Debug)]
pub struct GlobalOpts {
    pub engine_url: String,
    pub api_key: Option<String>,
    pub namespace: String,
    pub output: Output,
}

/// Inputs from the clap layer — flag overrides only. Env vars are read
/// inside `resolve` so flag > env > config-file > default precedence is
/// implemented once in a single place.
#[derive(Default)]
pub struct GlobalFlags<'a> {
    pub engine_url: Option<&'a str>,
    pub api_key: Option<&'a str>,
    pub namespace: Option<&'a str>,
    pub output: Option<&'a str>,
    pub config: Option<&'a str>,
}

impl GlobalOpts {
    /// Build a resolved `GlobalOpts` from CLI flags + environment +
    /// config file. Returns an exit code if something fatal happened
    /// during config loading (bad YAML, unknown keys, unreadable
    /// `api_key_file`) so callers can bail cleanly.
    pub fn resolve(flags: GlobalFlags) -> Result<Self, ExitCode> {
        let explicit_cfg = flags
            .config
            .map(String::from)
            .or_else(|| std::env::var("ASSAY_CONFIG_FILE").ok());
        let cfg = match config::load(explicit_cfg.as_deref()) {
            Ok(Some(c)) => c,
            Ok(None) => config::ConfigFile::default(),
            Err(e) => {
                eprintln!("error: {e}");
                return Err(ExitCode::from(1));
            }
        };

        let file_api_key = match config::resolve_api_key(&cfg) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: {e}");
                return Err(ExitCode::from(1));
            }
        };

        let engine_url = flags
            .engine_url
            .map(String::from)
            .or_else(|| std::env::var("ASSAY_ENGINE_URL").ok())
            .or(cfg.engine_url)
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
        let api_key = flags
            .api_key
            .map(String::from)
            .or_else(|| std::env::var("ASSAY_API_KEY").ok())
            .or(file_api_key);
        let namespace = flags
            .namespace
            .map(String::from)
            .or_else(|| std::env::var("ASSAY_NAMESPACE").ok())
            .or(cfg.namespace)
            .unwrap_or_else(|| "main".to_string());

        let output_str = flags
            .output
            .map(String::from)
            .or_else(|| std::env::var("ASSAY_OUTPUT").ok())
            .or(cfg.output);
        let output = match output_str {
            Some(s) => match Output::from_user_string(&s) {
                Some(o) => o,
                None => {
                    eprintln!(
                        "error: unknown output format '{s}' (expected table, json, jsonl, yaml)"
                    );
                    return Err(ExitCode::from(1));
                }
            },
            None => Output::tty_adaptive_default(),
        };

        Ok(Self {
            engine_url,
            api_key,
            namespace,
            output,
        })
    }
}
