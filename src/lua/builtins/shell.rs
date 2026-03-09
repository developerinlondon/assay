use mlua::Lua;

pub fn register_shell(lua: &Lua) -> mlua::Result<()> {
    let shell_table = lua.create_table()?;

    let exec_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let cmd: String = args_iter
            .next()
            .ok_or_else(|| mlua::Error::runtime("shell.exec: command string required"))
            .and_then(|v| lua.unpack(v))?;

        // Parse optional options table
        let mut cwd: Option<String> = None;
        let mut env_vars: Option<Vec<(String, String)>> = None;
        let mut stdin_data: Option<String> = None;
        let mut timeout_secs: Option<f64> = None;

        if let Some(mlua::Value::Table(opts)) = args_iter.next() {
            cwd = opts.get::<Option<String>>("cwd")?;
            stdin_data = opts.get::<Option<String>>("stdin")?;
            timeout_secs = opts.get::<Option<f64>>("timeout")?;

            if let Some(env_table) = opts.get::<Option<mlua::Table>>("env")? {
                let mut pairs = Vec::new();
                for pair in env_table.pairs::<String, String>() {
                    let (k, v) = pair?;
                    pairs.push((k, v));
                }
                env_vars = Some(pairs);
            }
        }

        // Validate timeout before use — Duration::from_secs_f64 panics on NaN/negative
        if let Some(secs) = timeout_secs
            && (!secs.is_finite() || secs < 0.0)
        {
            return Err(mlua::Error::runtime(
                "shell.exec: timeout must be a non-negative finite number",
            ));
        }

        // Build command — use /bin/sh explicitly for predictable behavior
        let mut command = std::process::Command::new("/bin/sh");
        command.arg("-c").arg(&cmd);

        if let Some(ref dir) = cwd {
            command.current_dir(dir);
        }

        if let Some(ref vars) = env_vars {
            for (k, v) in vars {
                command.env(k, v);
            }
        }

        command.stdin(if stdin_data.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        let mut child = command.spawn().map_err(|e| {
            mlua::Error::runtime(format!("shell.exec: failed to spawn command: {e}"))
        })?;

        // Write stdin if provided
        if let Some(ref data) = stdin_data {
            use std::io::Write;
            if let Some(mut child_stdin) = child.stdin.take() {
                child_stdin.write_all(data.as_bytes()).map_err(|e| {
                    mlua::Error::runtime(format!("shell.exec: failed to write stdin: {e}"))
                })?;
            }
        }

        // Wait with optional timeout
        let output = if let Some(secs) = timeout_secs {
            let duration = std::time::Duration::from_secs_f64(secs);
            let start = std::time::Instant::now();

            // Drain stdout/stderr in background threads to prevent pipe buffer deadlock.
            // If the child fills the OS pipe buffer (~64KB), it blocks waiting for a reader.
            // Without concurrent draining, try_wait() polls forever since the child never exits.
            let mut stdout_handle = child.stdout.take().map(|mut s| {
                std::thread::spawn(move || {
                    let mut buf = String::new();
                    let _ = std::io::Read::read_to_string(&mut s, &mut buf);
                    buf
                })
            });
            let mut stderr_handle = child.stderr.take().map(|mut s| {
                std::thread::spawn(move || {
                    let mut buf = String::new();
                    let _ = std::io::Read::read_to_string(&mut s, &mut buf);
                    buf
                })
            });

            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let stdout = stdout_handle
                            .take()
                            .map_or_else(String::new, |h| h.join().unwrap_or_default());
                        let stderr = stderr_handle
                            .take()
                            .map_or_else(String::new, |h| h.join().unwrap_or_default());
                        break Ok((status.code().unwrap_or(-1), stdout, stderr, false));
                    }
                    Ok(None) => {
                        if start.elapsed() >= duration {
                            let _ = child.kill();
                            let _ = child.wait();
                            // Join reader threads to avoid leaking them
                            if let Some(handle) = stdout_handle.take() {
                                let _ = handle.join();
                            }
                            if let Some(handle) = stderr_handle.take() {
                                let _ = handle.join();
                            }
                            break Ok((-1, String::new(), String::new(), true));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => {
                        break Err(mlua::Error::runtime(format!(
                            "shell.exec: error waiting for process: {e}"
                        )));
                    }
                }
            }?
        } else {
            let output = child.wait_with_output().map_err(|e| {
                mlua::Error::runtime(format!("shell.exec: failed to wait for command: {e}"))
            })?;
            let status = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            (status, stdout, stderr, false)
        };

        let result = lua.create_table()?;
        result.set("status", output.0)?;
        result.set("stdout", output.1)?;
        result.set("stderr", output.2)?;
        result.set("timed_out", output.3)?;

        Ok(result)
    })?;
    shell_table.set("exec", exec_fn)?;

    lua.globals().set("shell", shell_table)?;
    Ok(())
}
