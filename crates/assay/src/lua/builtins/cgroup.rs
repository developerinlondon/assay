//! `cgroup` Rust builtin — cgroup v2 unified-hierarchy readers.
//!
//! Linux-only. On non-Linux targets the `cgroup` Lua table is registered
//! empty. Plan 18 Phase 2.

use mlua::Lua;

pub fn register_cgroup(lua: &Lua) -> mlua::Result<()> {
    let cgroup_table = lua.create_table()?;
    #[cfg(target_os = "linux")]
    cgroup_impl::register(lua, &cgroup_table)?;
    lua.globals().set("cgroup", cgroup_table)?;
    Ok(())
}

#[cfg(target_os = "linux")]
mod cgroup_impl {
    use mlua::Lua;

    pub fn register(_lua: &Lua, _t: &mlua::Table) -> mlua::Result<()> {
        Ok(())
    }
}
