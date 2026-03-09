use mlua::Lua;

pub fn register_process(lua: &Lua) -> mlua::Result<()> {
    let process_table = lua.create_table()?;

    // process.list() — returns table of {pid, name, ...} from /proc
    let list_fn = lua.create_function(|lua, ()| {
        let procs = lua.create_table()?;
        let mut i = 1;

        let entries = std::fs::read_dir("/proc").map_err(|e| {
            mlua::Error::runtime(format!("process.list: failed to read /proc: {e}"))
        })?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Only numeric directories (PIDs)
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

            let info = lua.create_table()?;
            info.set("pid", pid)?;
            info.set("name", comm)?;

            // Try to read cmdline for full command
            let cmdline_path = entry.path().join("cmdline");
            if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
                let cmdline = cmdline.replace('\0', " ").trim().to_string();
                if !cmdline.is_empty() {
                    info.set("cmdline", cmdline)?;
                }
            }

            procs.set(i, info)?;
            i += 1;
        }

        Ok(procs)
    })?;
    process_table.set("list", list_fn)?;

    // process.is_running(name) — check if a process with given name exists
    let is_running_fn = lua.create_function(|_, name: String| {
        let entries = match std::fs::read_dir("/proc") {
            Ok(e) => e,
            Err(_) => return Ok(false),
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let dir_name = entry.file_name();
            // Only numeric directories
            if dir_name.to_string_lossy().parse::<u32>().is_err() {
                continue;
            }

            let comm_path = entry.path().join("comm");
            if let Ok(comm) = std::fs::read_to_string(&comm_path)
                && comm.trim() == name
            {
                return Ok(true);
            }
        }

        Ok(false)
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

        // Use libc::kill directly — no extra dependency needed
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
