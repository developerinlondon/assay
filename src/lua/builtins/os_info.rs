use mlua::Lua;

pub fn register_os(lua: &Lua) -> mlua::Result<()> {
    let os_table = lua.create_table()?;

    // os.hostname() — returns hostname string
    let hostname_fn = lua.create_function(|_, ()| {
        get_hostname().map_err(|e| mlua::Error::runtime(format!("os.hostname: {e}")))
    })?;
    os_table.set("hostname", hostname_fn)?;

    // os.arch() — returns architecture string (e.g. "x86_64", "aarch64")
    let arch_fn = lua.create_function(|_, ()| Ok(std::env::consts::ARCH.to_string()))?;
    os_table.set("arch", arch_fn)?;

    // os.platform() — returns OS string (e.g. "linux", "macos", "windows")
    let platform_fn = lua.create_function(|_, ()| Ok(std::env::consts::OS.to_string()))?;
    os_table.set("platform", platform_fn)?;

    lua.globals().set("os", os_table)?;
    Ok(())
}

fn get_hostname() -> Result<String, String> {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname is a standard POSIX function. We pass a valid buffer
    // and its length. The kernel writes a null-terminated hostname into the buffer.
    let ret = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<libc::c_char>(), buf.len()) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!("{err}"));
    }
    // Find the null terminator
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..len].to_vec()).map_err(|e| format!("hostname is not valid UTF-8: {e}"))
}
