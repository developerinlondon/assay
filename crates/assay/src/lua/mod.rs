pub mod async_bridge;
pub mod builtins;

use anyhow::Result;
use include_dir::{Dir, include_dir};
use mlua::{Lua, LuaOptions, StdLib};

static STDLIB_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/stdlib");

/// Environment variable to override the global module search path.
pub const MODULES_PATH_ENV: &str = "ASSAY_MODULES_PATH";

const DANGEROUS_GLOBALS: &[&str] = &["load", "loadfile", "dofile"];

fn lua_err(e: mlua::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

pub fn create_vm(client: reqwest::Client) -> Result<Lua> {
    create_vm_with_paths(client, None)
}

#[allow(dead_code)]
pub fn create_vm_with_lib_path(client: reqwest::Client, lib_path: String) -> Result<Lua> {
    create_vm_with_paths(client, Some(lib_path))
}

pub fn create_vm_with_paths(
    client: reqwest::Client,
    global_modules_path: Option<String>,
) -> Result<Lua> {
    let libs = StdLib::ALL_SAFE;
    let lua = Lua::new_with(libs, LuaOptions::default()).map_err(lua_err)?;
    lua.set_memory_limit(64 * 1024 * 1024).map_err(lua_err)?;
    sandbox(&lua).map_err(lua_err)?;
    register_fs_loader(&lua, global_modules_path).map_err(lua_err)?;
    register_stdlib_loader(&lua).map_err(lua_err)?;
    builtins::register_all(&lua, client).map_err(lua_err)?;
    Ok(lua)
}

fn sandbox(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    for name in DANGEROUS_GLOBALS {
        globals.set(*name, mlua::Value::Nil)?;
    }

    let string_lib: mlua::Table = globals.get("string")?;
    string_lib.set("dump", mlua::Value::Nil)?;

    Ok(())
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
                let source = file.contents_utf8().ok_or_else(|| {
                    mlua::Error::runtime(format!("stdlib {path}: invalid UTF-8"))
                })?;
                let loader = lua
                    .load(source)
                    .set_name(format!("@assay/{path}"))
                    .into_function()?;
                return Ok(mlua::Value::Function(loader));
            }
        }

        Ok(mlua::Value::String(
            lua.create_string(format!("no embedded stdlib file: {}", candidates[0]))?,
        ))
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
