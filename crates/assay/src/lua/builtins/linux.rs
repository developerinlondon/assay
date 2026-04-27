//! `linux` Rust builtin — `/proc` and `/sys/fs/...` readers.
//!
//! Linux-only. On non-Linux targets the `linux` Lua table is registered
//! empty; no methods are exposed. This matches the `process` builtin's
//! posture (it has Linux-fast and macOS-fallback paths) but goes further:
//! /proc has no analogue on macOS / Windows, so there's nothing to fall
//! back to. Callers can probe `linux.kernel ~= nil` to detect support.

use mlua::Lua;

pub fn register_linux(lua: &Lua) -> mlua::Result<()> {
    let linux_table = lua.create_table()?;
    #[cfg(target_os = "linux")]
    linux_impl::register(lua, &linux_table)?;
    lua.globals().set("linux", linux_table)?;
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux_impl {
    // Implementation lives here. Plan 18 Phase 1.
    use mlua::Lua;

    pub fn register(_lua: &Lua, _t: &mlua::Table) -> mlua::Result<()> {
        Ok(())
    }
}
