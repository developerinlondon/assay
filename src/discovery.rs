//! Module discovery and search index builder.
//!
//! Discovers Assay modules from three sources (in priority order):
//! 1. Project — `./modules/` relative to CWD
//! 2. Global  — `$ASSAY_MODULES_PATH` or `~/.assay/modules/`
//! 3. BuiltIn — embedded stdlib + hardcoded Rust builtins

use include_dir::{include_dir, Dir};
use crate::search::{SearchEngine, SearchResult};

use crate::metadata::{self, ModuleMetadata};
#[cfg(not(feature = "db"))]
use crate::search::BM25Index;
#[cfg(feature = "db")]
use crate::search_fts5::FTS5Index;

static STDLIB_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/stdlib");

/// Where a discovered module originates from.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleSource {
    /// Embedded in the binary via `include_dir!`
    BuiltIn,
    /// Found in `./modules/` relative to CWD
    Project,
    /// Found in `$ASSAY_MODULES_PATH` or `~/.assay/modules/`
    Global,
}

/// A module discovered during the discovery phase.
#[derive(Debug, Clone)]
pub struct DiscoveredModule {
    pub module_name: String,
    pub source: ModuleSource,
    pub metadata: ModuleMetadata,
    pub lua_source: String,
}

/// Hardcoded Rust builtins with their descriptions and search keywords.
const BUILTINS: &[(&str, &str, &[&str])] = &[
    (
        "http",
        "HTTP client and server: get, post, put, patch, delete, serve",
        &["http", "client", "server", "request", "response", "headers", "endpoint", "api", "webhook", "rest"],
    ),
    (
        "json",
        "JSON serialization: parse and encode",
        &["json", "serialization", "deserialize", "stringify", "parse", "encode", "format"],
    ),
    (
        "yaml",
        "YAML serialization: parse and encode",
        &["yaml", "serialization", "deserialize", "parse", "encode", "format"],
    ),
    (
        "toml",
        "TOML serialization: parse and encode",
        &["toml", "serialization", "deserialize", "parse", "encode", "configuration"],
    ),
    (
        "fs",
        "Filesystem: read and write files",
        &["fs", "filesystem", "file", "read", "write", "io", "path"],
    ),
    (
        "crypto",
        "Cryptography: jwt_sign, hash, hmac, random",
        &["crypto", "jwt", "signature", "hash", "hmac", "encryption", "random", "security", "password", "signing", "rsa", "sha256"],
    ),
    (
        "base64",
        "Base64 encoding and decoding",
        &["base64", "encoding", "decode", "encode", "binary"],
    ),
    (
        "regex",
        "Regular expressions: match, find, find_all, replace",
        &["regex", "pattern", "match", "find", "replace", "regular-expression", "regexp"],
    ),
    (
        "db",
        "Database: connect, query, execute, close (Postgres, MySQL, SQLite)",
        &["db", "database", "sql", "postgres", "mysql", "sqlite", "connection", "query", "execute"],
    ),
    (
        "ws",
        "WebSocket: connect, send, recv, close",
        &["ws", "websocket", "connection", "message", "streaming", "realtime", "socket"],
    ),
    (
        "template",
        "Jinja2-compatible templates: render file or string",
        &["template", "jinja2", "rendering", "string-template", "mustache", "render"],
    ),
    (
        "async",
        "Async tasks: spawn, spawn_interval, await, cancel",
        &["async", "asynchronous", "task", "coroutine", "concurrent", "spawn", "interval"],
    ),
    (
        "assert",
        "Assertions: eq, gt, lt, contains, not_nil, matches",
        &["assert", "assertion", "test", "validation", "comparison", "check", "verify"],
    ),
    (
        "log",
        "Logging: info, warn, error",
        &["log", "logging", "output", "debug", "error", "warning", "info", "trace"],
    ),
    (
        "env",
        "Environment variables: get",
        &["env", "environment", "variable", "configuration", "config"],
    ),
    (
        "sleep",
        "Sleep for N seconds",
        &["sleep", "delay", "pause", "wait", "time"],
    ),
    (
        "time",
        "Unix timestamp in seconds",
        &["time", "timestamp", "unix", "epoch", "clock", "datetime"],
    ),
];

/// Discover all modules: embedded stdlib + `./modules/` + `~/.assay/modules/` (or `$ASSAY_MODULES_PATH`).
///
/// Returns modules ordered by priority: Project first, then Global, then BuiltIn.
/// Callers can deduplicate by name, keeping the highest-priority (first) occurrence.
pub fn discover_modules() -> Vec<DiscoveredModule> {
    let mut modules = Vec::new();

    // Priority 1: Project modules (./modules/)
    discover_filesystem_modules(
        std::path::Path::new("./modules"),
        ModuleSource::Project,
        &mut modules,
    );

    // Priority 2: Global modules ($ASSAY_MODULES_PATH or ~/.assay/modules/)
    let global_path = resolve_global_modules_path();
    if let Some(path) = global_path {
        discover_filesystem_modules(&path, ModuleSource::Global, &mut modules);
    }

    // Priority 3: Embedded stdlib .lua files
    discover_embedded_stdlib(&mut modules);

    // Priority 3 (continued): Hardcoded Rust builtins
    discover_rust_builtins(&mut modules);

    modules
}

/// Build a search index from discovered modules.
///
/// When feature `db` is enabled: uses `FTS5Index`.
/// When feature `db` is disabled: uses `BM25Index`.
pub fn build_index(modules: &[DiscoveredModule]) -> Box<dyn SearchEngine> {
    #[cfg(feature = "db")]
    {
        let mut idx = FTS5Index::new();
        for m in modules {
            idx.add_document(
                &m.module_name,
                &[
                    ("keywords", &m.metadata.keywords.join(" "), 3.0),
                    ("module_name", &m.module_name, 2.0),
                    ("description", &m.metadata.description, 1.0),
                    ("functions", &m.metadata.auto_functions.join(" "), 1.0),
                ],
            );
        }
        Box::new(idx)
    }
    #[cfg(not(feature = "db"))]
    {
        let mut idx = BM25Index::new();
        for m in modules {
            idx.add_document(
                &m.module_name,
                &[
                    ("keywords", &m.metadata.keywords.join(" "), 3.0),
                    ("module_name", &m.module_name, 2.0),
                    ("description", &m.metadata.description, 1.0),
                    ("functions", &m.metadata.auto_functions.join(" "), 1.0),
                ],
            );
        }
        Box::new(idx)
    }
}

/// Convenience: discover all modules, build index, search, return results.
pub fn search_modules(query: &str, limit: usize) -> Vec<SearchResult> {
    let modules = discover_modules();
    let index = build_index(&modules);
    index.search(query, limit)
}

/// Resolve the global modules directory path.
///
/// Checks `$ASSAY_MODULES_PATH` first, then falls back to `~/.assay/modules/`.
/// Returns `None` if neither is available.
fn resolve_global_modules_path() -> Option<std::path::PathBuf> {
    if let Ok(custom) = std::env::var(crate::lua::MODULES_PATH_ENV) {
        return Some(std::path::PathBuf::from(custom));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Some(std::path::Path::new(&home).join(".assay/modules"));
    }
    None
}

/// Discover `.lua` files from a filesystem directory.
///
/// Silently skips if the directory does not exist.
fn discover_filesystem_modules(
    dir: &std::path::Path,
    source: ModuleSource,
    modules: &mut Vec<DiscoveredModule>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return, // Directory doesn't exist or can't be read — skip silently
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }

        let lua_source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let module_name = format!("assay.{stem}");
        let meta = metadata::parse_metadata(&lua_source);

        modules.push(DiscoveredModule {
            module_name,
            source: source.clone(),
            metadata: meta,
            lua_source,
        });
    }
}

/// Discover embedded stdlib `.lua` files from `include_dir!`.
fn discover_embedded_stdlib(modules: &mut Vec<DiscoveredModule>) {
    for file in STDLIB_DIR.files() {
        let path = file.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            continue;
        }

        let lua_source = match file.contents_utf8() {
            Some(s) => s,
            None => continue,
        };

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let module_name = format!("assay.{stem}");
        let meta = metadata::parse_metadata(lua_source);

        modules.push(DiscoveredModule {
            module_name,
            source: ModuleSource::BuiltIn,
            metadata: meta,
            lua_source: lua_source.to_string(),
        });
    }
}

/// Add hardcoded Rust builtins (not Lua files) to the module list.
fn discover_rust_builtins(modules: &mut Vec<DiscoveredModule>) {
    for &(name, description, kw) in BUILTINS {
        modules.push(DiscoveredModule {
            module_name: name.to_string(),
            source: ModuleSource::BuiltIn,
            lua_source: String::new(),
            metadata: ModuleMetadata {
                module_name: name.to_string(),
                description: description.to_string(),
                keywords: kw.iter().map(|k| k.to_string()).collect(),
                ..Default::default()
            },
        });
    }
}
