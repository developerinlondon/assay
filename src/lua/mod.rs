pub mod async_bridge;
pub mod builtins;

use anyhow::Result;
use include_dir::{Dir, include_dir};
use mlua::{Lua, LuaOptions, StdLib};

static STDLIB_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/stdlib");

const DANGEROUS_GLOBALS: &[&str] = &["load", "loadfile", "dofile", "collectgarbage", "print"];

fn lua_err(e: mlua::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

pub fn create_vm(client: reqwest::Client) -> Result<Lua> {
    let safe_libs = StdLib::ALL_SAFE ^ StdLib::IO ^ StdLib::OS;
    let lua = Lua::new_with(safe_libs, LuaOptions::default()).map_err(lua_err)?;
    lua.set_memory_limit(64 * 1024 * 1024).map_err(lua_err)?;
    sandbox(&lua).map_err(lua_err)?;
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

    let stdlib_searcher = lua.create_function(|lua, module_name: String| {
        let path = if let Some(rest) = module_name.strip_prefix("assay.") {
            format!("{rest}.lua")
        } else {
            return Ok(mlua::Value::String(
                lua.create_string(format!("not an assay.* module: {module_name}"))?,
            ));
        };

        match STDLIB_DIR.get_file(&path) {
            Some(file) => {
                let source = file
                    .contents_utf8()
                    .ok_or_else(|| mlua::Error::runtime(format!("stdlib {path}: invalid UTF-8")))?;
                let loader = lua
                    .load(source)
                    .set_name(format!("@assay/{path}"))
                    .into_function()?;
                Ok(mlua::Value::Function(loader))
            }
            None => Ok(mlua::Value::String(
                lua.create_string(format!("no embedded stdlib file: {path}"))?,
            )),
        }
    })?;

    let len = searchers.len()?;
    searchers.set(len + 1, stdlib_searcher)?;

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
