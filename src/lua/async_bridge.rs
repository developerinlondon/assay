use mlua::Lua;

pub async fn exec_lua_async(lua: &Lua, script: &str) -> mlua::Result<()> {
    lua.load(script).exec_async().await
}

pub async fn exec_lua_file_async(lua: &Lua, path: &str) -> mlua::Result<()> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| mlua::Error::runtime(format!("failed to read Lua script {path:?}: {e}")))?;
    exec_lua_async(lua, &content).await
}
