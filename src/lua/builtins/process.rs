use mlua::Lua;

/// Parse process list from /proc (Linux)
#[cfg(target_os = "linux")]
fn list_processes() -> Result<Vec<(i64, String, Option<String>)>, String> {
    let entries = std::fs::read_dir("/proc")
        .map_err(|e| format!("process.list: failed to read /proc: {e}"))?;

    let mut result = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        let pid: i64 = match name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        let comm_path = entry.path().join("comm");
        let comm = std::fs::read_to_string(&comm_path)
            .unwrap_or_default()
            .trim()
            .to_string();

        if comm.is_empty() {
            continue;
        }

        let cmdline_path = entry.path().join("cmdline");
        let cmdline = std::fs::read_to_string(&cmdline_path)
            .ok()
            .map(|s| s.replace('\0', " ").trim().to_string())
            .filter(|s| !s.is_empty());

        result.push((pid, comm, cmdline));
    }
    Ok(result)
}

/// Parse process list from `ps -eo pid,comm` (macOS / fallback)
#[cfg(not(target_os = "linux"))]
fn list_processes() -> Result<Vec<(i64, String, Option<String>)>, String> {
    let output = std::process::Command::new("ps")
        .args(["-eo", "pid,comm"])
        .output()
        .map_err(|e| format!("process.list: failed to run ps: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "process.list: ps failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut result = Vec::new();
    for line in stdout.lines().skip(1) {
        // skip header
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let pid: i64 = match parts.next().and_then(|s| s.trim().parse().ok()) {
            Some(p) => p,
            None => continue,
        };
        let comm = match parts.next() {
            Some(s) => s.trim().to_string(),
            None => continue,
        };
        if comm.is_empty() {
            continue;
        }
        // Extract just the binary name from the full path (e.g. /usr/sbin/syslogd -> syslogd)
        let name = comm
            .rsplit('/')
            .next()
            .unwrap_or(&comm)
            .to_string();
        result.push((pid, name, Some(comm)));
    }
    Ok(result)
}

/// Check if a process with the given name is running
fn is_process_running(name: &str) -> bool {
    match list_processes() {
        Ok(procs) => procs.iter().any(|(_, comm, _)| comm == name),
        Err(_) => false,
    }
}

pub fn register_process(lua: &Lua) -> mlua::Result<()> {
    let process_table = lua.create_table()?;

    // process.list() — returns table of {pid, name, cmdline?}
    let list_fn = lua.create_function(|lua, ()| {
        let proc_list =
            list_processes().map_err(mlua::Error::runtime)?;

        let procs = lua.create_table()?;
        for (i, (pid, name, cmdline)) in proc_list.into_iter().enumerate() {
            let info = lua.create_table()?;
            info.set("pid", pid)?;
            info.set("name", name)?;
            if let Some(cmd) = cmdline {
                info.set("cmdline", cmd)?;
            }
            procs.set(i + 1, info)?;
        }

        Ok(procs)
    })?;
    process_table.set("list", list_fn)?;

    // process.is_running(name) — check if a process with given name exists
    let is_running_fn = lua.create_function(|_, name: String| {
        Ok(is_process_running(&name))
    })?;
    process_table.set("is_running", is_running_fn)?;

    // process.kill(pid, signal?) — send signal to process (default SIGTERM = 15)
    let kill_fn = lua.create_function(|_, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let pid: i32 = args_iter
            .next()
            .ok_or_else(|| mlua::Error::runtime("process.kill: pid required"))
            .and_then(|v| match v {
                mlua::Value::Integer(n) => Ok(n as i32),
                mlua::Value::Number(n) => Ok(n as i32),
                _ => Err(mlua::Error::runtime("process.kill: pid must be a number")),
            })?;

        // Validate pid > 0 to prevent dangerous signals:
        // pid 0 = caller's process group, pid -1 = all permitted processes
        if pid <= 0 {
            return Err(mlua::Error::runtime(format!(
                "process.kill: pid must be > 0, got {pid}"
            )));
        }

        let signal: i32 = args_iter
            .next()
            .map(|v| match v {
                mlua::Value::Integer(n) => Ok(n as i32),
                mlua::Value::Number(n) => Ok(n as i32),
                mlua::Value::Nil => Ok(15), // SIGTERM
                _ => Err(mlua::Error::runtime(
                    "process.kill: signal must be a number",
                )),
            })
            .unwrap_or(Ok(15))?;

        if signal < 0 {
            return Err(mlua::Error::runtime(format!(
                "process.kill: signal must be >= 0, got {signal}"
            )));
        }

        // Use libc::kill via the libc crate — works on both Linux and macOS
        let result = unsafe { libc::kill(pid, signal) };
        if result == 0 {
            Ok(true)
        } else {
            let err = std::io::Error::last_os_error();
            Err(mlua::Error::runtime(format!(
                "process.kill: failed to send signal {signal} to pid {pid}: {err}"
            )))
        }
    })?;
    process_table.set("kill", kill_fn)?;

    lua.globals().set("process", process_table)?;
    Ok(())
}
