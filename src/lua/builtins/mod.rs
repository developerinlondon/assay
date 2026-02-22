mod assert;
mod core;
mod crypto;
#[cfg(feature = "db")]
mod db;
mod http;
mod json;
mod serialization;
mod template;
mod ws;

pub fn register_all(lua: &mlua::Lua, client: reqwest::Client) -> mlua::Result<()> {
    http::register_http(lua, client)?;
    json::register_json(lua)?;
    serialization::register_yaml(lua)?;
    serialization::register_toml(lua)?;
    assert::register_assert(lua)?;
    core::register_log(lua)?;
    core::register_env(lua)?;
    core::register_sleep(lua)?;
    core::register_time(lua)?;
    core::register_fs(lua)?;
    core::register_base64(lua)?;
    crypto::register_crypto(lua)?;
    core::register_regex(lua)?;
    core::register_async(lua)?;
    #[cfg(feature = "db")]
    db::register_db(lua)?;
    ws::register_ws(lua)?;
    template::register_template(lua)?;
    Ok(())
}
