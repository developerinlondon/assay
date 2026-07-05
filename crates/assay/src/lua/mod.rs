pub mod async_bridge;
pub mod builtins;
pub mod file_source;

#[cfg(feature = "server")]
#[allow(unused_imports)]
pub use builtins::LuaAxumRouter;

use anyhow::Result;
use include_dir::{Dir, include_dir};
use mlua::{Lua, LuaOptions, StdLib};

static STDLIB_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/stdlib");

/// Environment variable to override the global module search path.
pub const MODULES_PATH_ENV: &str = "ASSAY_MODULES_PATH";

/// Comma-separated list of additional globals to nil out at VM
/// creation time (defense-in-depth knob for hardened deployments).
/// Names support dotted paths into stdlib tables — e.g.
/// `ASSAY_BLOCK_GLOBALS=dofile,os.execute,debug.getinfo`.
pub const BLOCK_GLOBALS_ENV: &str = "ASSAY_BLOCK_GLOBALS";

/// Set to `1` or `true` to activate read-only mode for every VM the
/// process creates. Mutating builtins stay registered but raise
/// `readonly: <name> blocked` errors instead of executing. The
/// `--readonly` CLI flag activates the same mode per invocation.
pub const READONLY_ENV: &str = "ASSAY_READONLY";

pub fn readonly_from_env() -> bool {
    matches!(
        std::env::var(READONLY_ENV).ok().as_deref().map(str::trim),
        Some("1") | Some("true")
    )
}

/// Set to `1` or `true` to activate approval mode for every VM the process
/// creates. Mutating builtins stay registered but suspend for
/// per-operation approval via the resume flow instead of executing. The
/// `--approval-mode` CLI flag activates the same mode per invocation and
/// takes precedence over read-only mode.
pub const APPROVAL_ENV: &str = "ASSAY_APPROVAL";

/// Comma-separated set of already-approved operation indices for an
/// approval-mode re-run (set by the resume machinery).
pub(crate) const APPROVED_INDICES_ENV: &str = "ASSAY_APPROVED_INDICES";

/// The single operation index to fail terminally on an approval-mode
/// re-run (set by the resume machinery when a decision is `no`).
pub(crate) const DENIED_INDEX_ENV: &str = "ASSAY_DENIED_INDEX";

/// Prefix that marks a runtime error as an approval request. The tool-mode
/// runner extracts the JSON payload that follows it to suspend the run.
pub(crate) const APPROVAL_REQUEST_PREFIX: &str = "__assay_approval_request__:";

pub fn approval_from_env() -> bool {
    matches!(
        std::env::var(APPROVAL_ENV).ok().as_deref().map(str::trim),
        Some("1") | Some("true")
    )
}

/// Execution mode selecting which post-registration gate (if any) is
/// applied to a VM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecMode {
    #[default]
    Unrestricted,
    ReadOnly,
    Approval,
}

impl ExecMode {
    pub fn is_readonly(self) -> bool {
        matches!(self, ExecMode::ReadOnly)
    }

    pub fn is_approval(self) -> bool {
        matches!(self, ExecMode::Approval)
    }
}

/// Per-run approval state consumed by the approval gate: which operation
/// indices are pre-approved for this run and which single index (if any)
/// must fail terminally.
#[derive(Clone, Debug, Default)]
pub struct ApprovalConfig {
    pub approved_indices: Vec<u64>,
    pub denied_index: Option<u64>,
}

/// Resolve the approval set + denied index from the environment. Empty
/// when the vars are absent, which is the first-run (nothing approved
/// yet) case.
pub fn approval_config_from_env() -> ApprovalConfig {
    let approved_indices = std::env::var(APPROVED_INDICES_ENV)
        .ok()
        .map(|raw| parse_indices(&raw))
        .unwrap_or_default();
    let denied_index = std::env::var(DENIED_INDEX_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok());
    ApprovalConfig {
        approved_indices,
        denied_index,
    }
}

fn parse_indices(raw: &str) -> Vec<u64> {
    raw.split(',')
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<u64>().ok()
            }
        })
        .collect()
}

/// Full VM configuration. New knobs are added here rather than by widening
/// `create_vm_configured`, keeping the older factory signatures stable.
#[derive(Clone, Debug, Default)]
pub struct VmOptions {
    pub global_modules_path: Option<String>,
    pub mode: ExecMode,
    pub approval: ApprovalConfig,
}

fn lua_err(e: mlua::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

#[allow(dead_code)]
pub fn create_vm(client: reqwest::Client) -> Result<Lua> {
    create_vm_configured(client, None, readonly_from_env())
}

#[allow(dead_code)]
pub fn create_vm_with_lib_path(client: reqwest::Client, lib_path: String) -> Result<Lua> {
    create_vm_configured(client, Some(lib_path), readonly_from_env())
}

#[allow(dead_code)]
pub fn create_vm_with_paths(
    client: reqwest::Client,
    global_modules_path: Option<String>,
) -> Result<Lua> {
    create_vm_configured(client, global_modules_path, readonly_from_env())
}

pub fn create_vm_configured(
    client: reqwest::Client,
    global_modules_path: Option<String>,
    readonly: bool,
) -> Result<Lua> {
    let mode = if readonly {
        ExecMode::ReadOnly
    } else {
        ExecMode::Unrestricted
    };
    create_vm_with_options(
        client,
        VmOptions {
            global_modules_path,
            mode,
            approval: ApprovalConfig::default(),
        },
    )
}

pub fn create_vm_with_options(client: reqwest::Client, options: VmOptions) -> Result<Lua> {
    let VmOptions {
        global_modules_path,
        mode,
        approval,
    } = options;
    let libs = StdLib::ALL_SAFE;
    let lua = Lua::new_with(libs, LuaOptions::default()).map_err(lua_err)?;
    lua.set_memory_limit(64 * 1024 * 1024).map_err(lua_err)?;
    sandbox(&lua).map_err(lua_err)?;
    register_fs_loader(&lua, global_modules_path).map_err(lua_err)?;
    register_stdlib_loader(&lua).map_err(lua_err)?;
    builtins::register_all(&lua, client).map_err(lua_err)?;
    match mode {
        ExecMode::ReadOnly => builtins::readonly::apply(&lua).map_err(lua_err)?,
        ExecMode::Approval => builtins::approval::apply(&lua, &approval).map_err(lua_err)?,
        ExecMode::Unrestricted => {}
    }
    Ok(lua)
}

fn sandbox(lua: &Lua) -> mlua::Result<()> {
    // Block bytecode-level escape hatches only. Source-level loaders
    // (`load` / `loadfile` / `dofile`) stay available — operator scripts
    // are trusted to compose themselves out of multiple files (seed +
    // init bootstraps, shared helpers, etc.). `string.dump` stays
    // blocked because it produces native bytecode that defeats the
    // memory/CPU caps the runtime relies on.
    let globals = lua.globals();
    let string_lib: mlua::Table = globals.get("string")?;
    string_lib.set("dump", mlua::Value::Nil)?;

    if let Ok(extra) = std::env::var(BLOCK_GLOBALS_ENV) {
        for raw in extra.split(',') {
            let name = raw.trim();
            if name.is_empty() {
                continue;
            }
            nil_dotted_path(lua, name)?;
        }
    }

    Ok(())
}

/// Resolve a dotted Lua path (e.g. `"os.execute"` or `"debug.getinfo"`)
/// against globals and set the leaf to nil. A bare name (e.g.
/// `"dofile"`) clears it from `_G`. Missing intermediate tables are
/// silently skipped so a typo in `ASSAY_BLOCK_GLOBALS` doesn't fail
/// VM creation.
fn nil_dotted_path(lua: &Lua, path: &str) -> mlua::Result<()> {
    let parts: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Ok(());
    }
    let mut current: mlua::Table = lua.globals();
    for segment in &parts[..parts.len() - 1] {
        let next: mlua::Value = current.get(*segment)?;
        match next {
            mlua::Value::Table(t) => current = t,
            _ => return Ok(()),
        }
    }
    current.set(parts[parts.len() - 1], mlua::Value::Nil)
}

fn register_stdlib_loader(lua: &Lua) -> mlua::Result<()> {
    let package: mlua::Table = lua.globals().get("package")?;
    let searchers: mlua::Table = package.get("searchers")?;

    // Resolves `require("assay.ory.kratos")` -> "ory/kratos.lua" by replacing
    // dots with slashes, matching standard Lua package loading convention.
    // Tries "<path>.lua" first, then falls back to "<path>/init.lua" so
    // both `stdlib/ory.lua` (flat convenience wrapper) and
    // `stdlib/ory/kratos.lua` (nested submodule) resolve correctly.
    let stdlib_searcher = lua.create_function(|lua, module_name: String| {
        let rest = match module_name.strip_prefix("assay.") {
            Some(r) => r,
            None => {
                return Ok(mlua::Value::String(
                    lua.create_string(format!("not an assay.* module: {module_name}"))?,
                ));
            }
        };

        let base = rest.replace('.', "/");
        let candidates = [format!("{base}.lua"), format!("{base}/init.lua")];

        for path in &candidates {
            if let Some(file) = STDLIB_DIR.get_file(path) {
                let source = file
                    .contents_utf8()
                    .ok_or_else(|| mlua::Error::runtime(format!("stdlib {path}: invalid UTF-8")))?;
                let loader = lua
                    .load(source)
                    .set_name(format!("@assay/{path}"))
                    .into_function()?;
                return Ok(mlua::Value::Function(loader));
            }
        }

        Ok(mlua::Value::String(lua.create_string(format!(
            "no embedded stdlib file: {}",
            candidates[0]
        ))?))
    })?;

    let len = searchers.len()?;
    searchers.set(len + 1, stdlib_searcher)?;

    Ok(())
}

fn register_fs_loader(lua: &Lua, global_modules_path: Option<String>) -> mlua::Result<()> {
    let package: mlua::Table = lua.globals().get("package")?;
    let searchers: mlua::Table = package.get("searchers")?;

    // Same dotted-path resolution as the stdlib loader: `assay.ory.kratos`
    // -> "ory/kratos.lua", falling back to "ory/kratos/init.lua".
    let fs_searcher = lua.create_function(move |lua, module_name: String| {
        let rest = match module_name.strip_prefix("assay.") {
            Some(r) => r,
            None => {
                return Ok(mlua::Value::String(
                    lua.create_string(format!("not an assay.* module: {module_name}"))?,
                ));
            }
        };
        let base = rest.replace('.', "/");
        let candidates = [format!("{base}.lua"), format!("{base}/init.lua")];

        let try_load = |dir: &std::path::Path| -> Option<(std::path::PathBuf, String)> {
            for rel in &candidates {
                let full = dir.join(rel);
                if let Ok(source) = std::fs::read_to_string(&full) {
                    return Some((full, source));
                }
            }
            None
        };

        // Priority 1: ./modules/<path>.lua (per-project)
        if let Some((full, source)) = try_load(std::path::Path::new("./modules")) {
            let loader = lua
                .load(source)
                .set_name(format!("@{}", full.display()))
                .into_function()?;
            return Ok(mlua::Value::Function(loader));
        }

        // Priority 2: $ASSAY_MODULES_PATH or ~/.assay/modules/<path>.lua
        let global_path = if let Some(ref custom_path) = global_modules_path {
            std::path::PathBuf::from(custom_path)
        } else if let Ok(modules_env) = std::env::var(MODULES_PATH_ENV) {
            std::path::PathBuf::from(modules_env)
        } else if let Ok(home) = std::env::var("HOME") {
            std::path::Path::new(&home).join(".assay/modules")
        } else {
            std::path::PathBuf::new()
        };

        if !global_path.as_os_str().is_empty()
            && let Some((full, source)) = try_load(&global_path)
        {
            let loader = lua
                .load(source)
                .set_name(format!("@{}", full.display()))
                .into_function()?;
            return Ok(mlua::Value::Function(loader));
        }

        // Priority 3: Built-in modules are handled by register_stdlib_loader
        // Return nil to fall through to the next searcher
        Ok(mlua::Value::Nil)
    })?;

    let len = searchers.len()?;
    searchers.set(len + 1, fs_searcher)?;

    Ok(())
}

pub fn inject_env(lua: &Lua, env: &std::collections::HashMap<String, String>) -> Result<()> {
    if env.is_empty() {
        return Ok(());
    }
    let globals = lua.globals();
    let env_table: mlua::Table = globals.get("env").map_err(lua_err)?;
    let check_env: mlua::Table = env_table.get("_check_env").map_err(lua_err)?;
    for (k, v) in env {
        check_env.set(k.as_str(), v.as_str()).map_err(lua_err)?;
    }
    Ok(())
}
