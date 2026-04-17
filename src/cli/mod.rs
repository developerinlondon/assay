//! `assay workflow …` / `assay schedule …` CLI subcommand implementations.
//!
//! These are thin HTTP clients over the workflow engine's REST API. The
//! `assay.workflow` Lua stdlib mirrors the same surface programmatically;
//! this CLI exists for operators at a terminal (kubectl exec into a pod,
//! ad-hoc debugging, shell scripts). Lua scripts are the preferred path
//! for automation.

pub mod client;
pub mod commands;
pub mod table;

/// Global flags + env fallbacks shared by every CLI subcommand.
///
/// Precedence is flag → env → hardcoded default. No config-file support
/// in this release (see `.claude/plans/06-*.md` for the scope rationale).
#[derive(Clone, Debug)]
pub struct GlobalOpts {
    pub engine_url: String,
    pub api_key: Option<String>,
    pub namespace: String,
}

impl GlobalOpts {
    pub fn resolve(
        flag_engine_url: Option<&str>,
        flag_api_key: Option<&str>,
        flag_namespace: Option<&str>,
    ) -> Self {
        let engine_url = flag_engine_url
            .map(String::from)
            .or_else(|| std::env::var("ASSAY_ENGINE_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
        let api_key = flag_api_key
            .map(String::from)
            .or_else(|| std::env::var("ASSAY_API_KEY").ok());
        let namespace = flag_namespace
            .map(String::from)
            .or_else(|| std::env::var("ASSAY_NAMESPACE").ok())
            .unwrap_or_else(|| "main".to_string());
        Self {
            engine_url,
            api_key,
            namespace,
        }
    }
}
