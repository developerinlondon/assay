pub mod async_bridge;
pub mod builtins;

use anyhow::Result;
use mlua::{Lua, LuaOptions, StdLib};

const DANGEROUS_GLOBALS: &[&str] = &[
    "load",
    "loadfile",
    "dofile",
    "collectgarbage",
    "print",
    "require",
];

fn lua_err(e: mlua::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

pub fn create_vm(client: reqwest::Client) -> Result<Lua> {
    let safe_libs = StdLib::ALL_SAFE ^ StdLib::IO ^ StdLib::OS ^ StdLib::PACKAGE;
    let lua = Lua::new_with(safe_libs, LuaOptions::default()).map_err(lua_err)?;
    lua.set_memory_limit(64 * 1024 * 1024).map_err(lua_err)?;
    sandbox(&lua).map_err(lua_err)?;
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
