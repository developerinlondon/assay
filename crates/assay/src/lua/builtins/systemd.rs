//! `systemd` Rust builtin — async D-Bus client (`org.freedesktop.systemd1`,
//! `org.freedesktop.machine1`) plus a journal reader/streamer.
//!
//! Linux-only. On non-Linux targets the `systemd` Lua table is registered
//! empty. Plan 18 Phase 3.

use mlua::Lua;

pub fn register_systemd(lua: &Lua) -> mlua::Result<()> {
    let systemd_table = lua.create_table()?;
    #[cfg(target_os = "linux")]
    systemd_impl::register(lua, &systemd_table)?;
    lua.globals().set("systemd", systemd_table)?;
    Ok(())
}

#[cfg(target_os = "linux")]
mod systemd_impl {
    use mlua::Lua;

    pub fn register(_lua: &Lua, _t: &mlua::Table) -> mlua::Result<()> {
        Ok(())
    }
}
