use mlua::{Lua, Value};

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

    // os.time() — returns current UTC epoch as integer (standard Lua)
    let time_fn = lua.create_function(|_, ()| {
        Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64)
    })?;
    os_table.set("time", time_fn)?;

    // os.clock() — returns CPU time in seconds (standard Lua)
    let start = std::time::Instant::now();
    let clock_fn = lua.create_function(move |_, ()| Ok(start.elapsed().as_secs_f64()))?;
    os_table.set("clock", clock_fn)?;

    // os.date(format?, time?, tz_offset?) — format a timestamp
    // Supports: "!%Y-%m-%dT%H:%M:%SZ" (ISO 8601), "*t" (table), and strftime patterns.
    // If format starts with "!", uses UTC (offset 0) regardless of tz_offset.
    // tz_offset is hours from UTC (e.g. -5 for EST, -4 for EDT, 5.5 for IST).
    // Defaults to 0 (UTC) if not provided.
    let date_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();
        let format: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            Some(Value::Nil) | None => "%c".to_string(),
            _ => return Err(mlua::Error::runtime("os.date: format must be a string")),
        };
        let epoch: i64 = match args_iter.next() {
            Some(Value::Integer(n)) => n,
            Some(Value::Number(n)) => n as i64,
            Some(Value::Nil) | None => {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
            }
            _ => return Err(mlua::Error::runtime("os.date: time must be a number")),
        };
        let tz_offset_hours: f64 = match args_iter.next() {
            Some(Value::Number(n)) => n,
            Some(Value::Integer(n)) => n as f64,
            Some(Value::Nil) | None => 0.0,
            _ => return Err(mlua::Error::runtime("os.date: tz_offset must be a number (hours from UTC)")),
        };

        // "!" prefix forces UTC (offset 0)
        let (fmt, offset_secs) = if let Some(stripped) = format.strip_prefix('!') {
            (stripped, 0i64)
        } else {
            (format.as_str(), (tz_offset_hours * 3600.0) as i64)
        };

        // Apply timezone offset to epoch
        let secs = epoch + offset_secs;
        let mut days = (secs / 86400) as i32;
        let tod = (secs % 86400) as i32;
        let hour = tod / 3600;
        let min = (tod % 3600) / 60;
        let sec = tod % 60;

        let mut year: i32 = 1970;
        loop {
            let yd = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 366 } else { 365 };
            if days < yd { break; }
            days -= yd;
            year += 1;
        }
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let mdays: [i32; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut month: i32 = 0;
        while month < 12 && days >= mdays[month as usize] {
            days -= mdays[month as usize];
            month += 1;
        }
        let day = days + 1;
        month += 1; // 1-indexed

        // "*t" returns a table (standard Lua)
        if fmt == "*t" {
            let t = lua.create_table()?;
            t.set("year", year)?;
            t.set("month", month)?;
            t.set("day", day)?;
            t.set("hour", hour)?;
            t.set("min", min)?;
            t.set("sec", sec)?;
            return Ok(Value::Table(t));
        }

        // strftime-style substitution
        let result = fmt
            .replace("%Y", &format!("{year:04}"))
            .replace("%m", &format!("{month:02}"))
            .replace("%d", &format!("{day:02}"))
            .replace("%H", &format!("{hour:02}"))
            .replace("%M", &format!("{min:02}"))
            .replace("%S", &format!("{sec:02}"))
            .replace("%c", &format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02}"));

        Ok(Value::String(lua.create_string(&result)?))
    })?;
    os_table.set("date", date_fn)?;

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
