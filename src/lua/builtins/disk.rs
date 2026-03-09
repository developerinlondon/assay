use mlua::Lua;

pub fn register_disk(lua: &Lua) -> mlua::Result<()> {
    let disk_table = lua.create_table()?;

    // disk.usage(path) — returns {total, free, used, percent}
    let usage_fn = lua.create_function(|lua, path: String| {
        let stat = statvfs_info(&path).map_err(|e| {
            mlua::Error::runtime(format!("disk.usage: failed to stat {path:?}: {e}"))
        })?;

        let info = lua.create_table()?;
        info.set("total", stat.total)?;
        info.set("free", stat.free)?;
        info.set("used", stat.total - stat.free)?;
        info.set(
            "percent",
            if stat.total == 0 {
                0.0
            } else {
                (stat.total - stat.free) as f64 / stat.total as f64 * 100.0
            },
        )?;
        Ok(info)
    })?;
    disk_table.set("usage", usage_fn)?;

    lua.globals().set("disk", disk_table)?;

    // Tier 3: Lua composable helpers injected after Rust builtins
    lua.load(
        r#"
        -- disk.sweep(dir, age_secs) — remove entries older than age_secs seconds
        function disk.sweep(dir, age_secs)
            local now = time()
            local entries = fs.list(dir)
            local removed = 0
            for _, entry in ipairs(entries) do
                local full_path = dir .. "/" .. entry.name
                local stat = fs.stat(full_path)
                if stat and stat.modified then
                    if (now - stat.modified) > age_secs then
                        fs.remove(full_path)
                        removed = removed + 1
                    end
                end
            end
            return removed
        end

        -- disk.dir_size(path) — recursive directory size in bytes
        function disk.dir_size(path)
            local total = 0
            local entries = fs.readdir(path)
            for _, entry in ipairs(entries) do
                if entry.type == "file" then
                    local full_path = path .. "/" .. entry.path
                    local stat = fs.stat(full_path)
                    if stat then
                        total = total + stat.size
                    end
                end
            end
            return total
        end
        "#,
    )
    .exec()?;

    Ok(())
}

struct StatvfsInfo {
    total: u64,
    free: u64,
}

fn statvfs_info(path: &str) -> Result<StatvfsInfo, String> {
    use std::ffi::CString;

    let c_path = CString::new(path).map_err(|e| format!("invalid path: {e}"))?;

    // SAFETY: statvfs is a standard POSIX function. We pass a valid null-terminated
    // path and a zeroed-out buffer. The kernel fills the buffer on success (ret 0).
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        let ret = libc::statvfs(c_path.as_ptr(), &mut stat);
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            return Err(format!("{err}"));
        }
        let block_size = stat.f_frsize as u64;
        let total = stat.f_blocks as u64 * block_size;
        let free = stat.f_bfree as u64 * block_size;
        Ok(StatvfsInfo { total, free })
    }
}
