mod apt;
mod assert;
mod cgroup;
mod compress;
pub mod core;
mod crypto;
#[cfg(feature = "db")]
mod db;
mod disk;
pub mod http;
mod json;
mod linux;
mod machinectl;
mod markdown;
mod oci;
mod os_info;
mod process;
mod process_pty;
mod serialization;
mod shell;
mod systemd;
mod tarball;
mod template;
mod ws;

#[cfg(feature = "server")]
pub use http::LuaAxumRouter;

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
    core::register_string_helpers(lua)?;
    core::register_base64(lua)?;
    crypto::register_crypto(lua)?;
    core::register_regex(lua)?;
    core::register_async(lua)?;
    #[cfg(feature = "db")]
    db::register_db(lua)?;
    ws::register_ws(lua)?;
    template::register_template(lua)?;
    markdown::register_markdown(lua)?;
    shell::register_shell(lua)?;
    process::register_process(lua)?;
    process_pty::register_process_pty(lua)?;
    disk::register_disk(lua)?;
    os_info::register_os(lua)?;
    apt::register_apt(lua)?;
    compress::register_compress(lua)?;
    linux::register_linux(lua)?;
    cgroup::register_cgroup(lua)?;
    systemd::register_systemd(lua)?;
    machinectl::register_machinectl(lua)?;
    oci::register_oci(lua)?;
    tarball::register_tar(lua)?;
    Ok(())
}
