use mlua::Lua;
use std::time::{Duration, Instant};

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
        let name = comm.rsplit('/').next().unwrap_or(&comm).to_string();
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
        let proc_list = list_processes().map_err(mlua::Error::runtime)?;

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
    let is_running_fn = lua.create_function(|_, name: String| Ok(is_process_running(&name)))?;
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

    // process.spawn(opts) — launch a detached child process, return its pid.
    //
    // opts table:
    //   cmd      string  — required; binary path or name resolvable via PATH
    //   args     table?  — positional args (no shell parsing; pass them as
    //                      individual array elements)
    //   cwd      string? — working directory; defaults to caller's
    //   env      table?  — extra env vars merged onto the caller's environment
    //   stdout   string? — file path to redirect stdout to; default = inherit
    //   stderr   string? — file path to redirect stderr to; default = inherit
    //
    // Returns a table { pid = N }. The child runs detached — Rust's Child
    // handle is forgotten so the OS process survives the spawn() return.
    // Use process.wait(pid) to reap it later, or process.kill(pid) to
    // terminate. NOT auto-reaped — every spawn() must be paired with a
    // wait() to avoid zombies.
    let spawn_fn = lua.create_function(|lua, opts: mlua::Table| {
        let cmd: String = opts.get("cmd").map_err(|_| {
            mlua::Error::runtime("process.spawn: opts.cmd (string) is required")
        })?;

        let args: Option<Vec<String>> = opts.get::<Option<mlua::Table>>("args")?.map(|t| {
            t.sequence_values::<String>()
                .filter_map(|v| v.ok())
                .collect()
        });

        let cwd: Option<String> = opts.get("cwd")?;

        let env_pairs: Option<Vec<(String, String)>> =
            opts.get::<Option<mlua::Table>>("env")?.map(|t| {
                let mut pairs = Vec::new();
                for entry in t.pairs::<String, String>().flatten() {
                    pairs.push(entry);
                }
                pairs
            });

        let stdout_path: Option<String> = opts.get("stdout")?;
        let stderr_path: Option<String> = opts.get("stderr")?;

        let mut command = std::process::Command::new(&cmd);
        if let Some(a) = args {
            command.args(a);
        }
        if let Some(ref d) = cwd {
            command.current_dir(d);
        }
        if let Some(ref vars) = env_pairs {
            for (k, v) in vars {
                command.env(k, v);
            }
        }

        // Stdin always /dev/null — a backgrounded process should never
        // be expected to read from the caller's stdin, and inheriting it
        // can lock the parent script.
        command.stdin(std::process::Stdio::null());

        if let Some(ref path) = stdout_path {
            let file = std::fs::File::create(path).map_err(|e| {
                mlua::Error::runtime(format!(
                    "process.spawn: failed to open stdout file '{path}': {e}"
                ))
            })?;
            command.stdout(std::process::Stdio::from(file));
        }
        if let Some(ref path) = stderr_path {
            let file = std::fs::File::create(path).map_err(|e| {
                mlua::Error::runtime(format!(
                    "process.spawn: failed to open stderr file '{path}': {e}"
                ))
            })?;
            command.stderr(std::process::Stdio::from(file));
        }

        let child = command.spawn().map_err(|e| {
            mlua::Error::runtime(format!("process.spawn: failed to launch '{cmd}': {e}"))
        })?;
        let pid = child.id() as i32;

        // Detach: drop the Child without trying to reap. On Unix, Drop
        // doesn't kill the process; the OS keeps it running. The child
        // will become a zombie when it exits unless someone wait()s for
        // it — that's the caller's responsibility (process.wait below).
        std::mem::forget(child);

        let result = lua.create_table()?;
        result.set("pid", pid)?;
        Ok(result)
    })?;
    process_table.set("spawn", spawn_fn)?;

    // process.wait(pid, opts?) — wait for a spawned process to exit.
    //
    // opts table (all optional):
    //   timeout  number — seconds to wait before giving up (default: blocking)
    //
    // Returns: { status = N, exited = bool, signaled = bool, timed_out = bool }
    //   status     — exit code (0..255), or 128+sig if killed by signal
    //   exited     — true if the process called exit() normally
    //   signaled   — true if the process was killed by a signal
    //   timed_out  — true if `timeout` elapsed before the process exited
    //                (the process is still running; status is meaningless)
    //
    // Since process.spawn forgets the Child, we reap via libc::waitpid
    // directly. Calling wait() on a pid the caller didn't spawn is
    // valid as long as the pid is still in the caller's process group.
    let wait_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut iter = args.into_iter();
        let pid_v = iter
            .next()
            .ok_or_else(|| mlua::Error::runtime("process.wait: pid required"))?;
        let pid: i32 = match pid_v {
            mlua::Value::Integer(n) => n as i32,
            mlua::Value::Number(n) => n as i32,
            _ => return Err(mlua::Error::runtime("process.wait: pid must be a number")),
        };
        if pid <= 0 {
            return Err(mlua::Error::runtime("process.wait: pid must be > 0"));
        }

        let mut timeout_secs: Option<f64> = None;
        if let Some(mlua::Value::Table(opts)) = iter.next() {
            timeout_secs = opts.get::<Option<f64>>("timeout")?;
            if let Some(t) = timeout_secs
                && (!t.is_finite() || t < 0.0)
            {
                return Err(mlua::Error::runtime(
                    "process.wait: timeout must be a non-negative finite number",
                ));
            }
        }

        let deadline = timeout_secs.map(|s| Instant::now() + Duration::from_secs_f64(s));
        let mut raw_status: i32 = 0;
        loop {
            let flags = if deadline.is_some() {
                libc::WNOHANG
            } else {
                0
            };
            let ret = unsafe { libc::waitpid(pid, &mut raw_status, flags) };
            if ret == pid {
                break;
            } else if ret == 0 && deadline.is_some() {
                // WNOHANG: child not yet exited
                if Instant::now() >= deadline.unwrap() {
                    let result = lua.create_table()?;
                    result.set("status", -1)?;
                    result.set("exited", false)?;
                    result.set("signaled", false)?;
                    result.set("timed_out", true)?;
                    return Ok(result);
                }
                std::thread::sleep(Duration::from_millis(50));
                continue;
            } else if ret < 0 {
                let err = std::io::Error::last_os_error();
                return Err(mlua::Error::runtime(format!(
                    "process.wait: waitpid({pid}) failed: {err}"
                )));
            } else {
                // ret > 0 but != pid shouldn't happen with a specific pid.
                return Err(mlua::Error::runtime(format!(
                    "process.wait: waitpid({pid}) returned unexpected pid {ret}"
                )));
            }
        }

        let exited = libc::WIFEXITED(raw_status);
        let signaled = libc::WIFSIGNALED(raw_status);
        let status = if exited {
            libc::WEXITSTATUS(raw_status)
        } else if signaled {
            128 + libc::WTERMSIG(raw_status)
        } else {
            -1
        };

        let result = lua.create_table()?;
        result.set("status", status)?;
        result.set("exited", exited)?;
        result.set("signaled", signaled)?;
        result.set("timed_out", false)?;
        Ok(result)
    })?;
    process_table.set("wait", wait_fn)?;

    lua.globals().set("process", process_table)?;

    // Tier 3: Lua composable helper
    lua.load(
        r#"
        -- process.wait_idle(names, timeout, interval)
        -- Wait until none of the named processes are running.
        -- names: string or table of strings
        -- timeout: max seconds to wait (default 30)
        -- interval: poll interval in seconds (default 1)
        -- Returns true if all idle, false if timed out.
        function process.wait_idle(names, timeout, interval)
            timeout = timeout or 30
            interval = interval or 1
            if type(names) == "string" then names = {names} end
            local deadline = time() + timeout
            while time() < deadline do
                local any_running = false
                for _, name in ipairs(names) do
                    if process.is_running(name) then
                        any_running = true
                        break
                    end
                end
                if not any_running then return true end
                sleep(interval)
            end
            return false
        end
        "#,
    )
    .exec()?;

    Ok(())
}
