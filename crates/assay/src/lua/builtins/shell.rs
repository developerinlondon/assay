use mlua::Lua;

pub fn register_shell(lua: &Lua) -> mlua::Result<()> {
    let shell_table = lua.create_table()?;

    // shell.exec(cmd, opts?) → { status, stdout, stderr, timed_out }
    //
    // tokio::process-based — yields to the runtime while the child runs so
    // other Lua coroutines (and HTTP handlers) make progress concurrently.
    // The previous std::process + thread::sleep polling implementation
    // blocked the LocalSet for the entire duration of the child, freezing
    // any other in-flight async work in the same Lua VM.
    let exec_fn = lua.create_async_function(|lua, args: mlua::MultiValue| async move {
        let mut args_iter = args.into_iter();

        let cmd: String = args_iter
            .next()
            .ok_or_else(|| mlua::Error::runtime("shell.exec: command string required"))
            .and_then(|v| lua.unpack(v))?;

        let mut cwd: Option<String> = None;
        let mut env_vars: Option<Vec<(String, String)>> = None;
        // Stored as raw bytes so binary stdin (e.g. tarballs streamed into
        // an nspawn container) survives the mlua String boundary.
        let mut stdin_bytes: Option<Vec<u8>> = None;
        let mut timeout_secs: Option<f64> = None;

        if let Some(mlua::Value::Table(opts)) = args_iter.next() {
            cwd = opts.get::<Option<String>>("cwd")?;
            if let Some(s) = opts.get::<Option<mlua::String>>("stdin")? {
                stdin_bytes = Some(s.as_bytes().to_vec());
            }
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

        if let Some(secs) = timeout_secs
            && (!secs.is_finite() || secs < 0.0)
        {
            return Err(mlua::Error::runtime(
                "shell.exec: timeout must be a non-negative finite number",
            ));
        }

        // Build command — /bin/sh -c <cmd>, predictable across distros.
        let mut command = tokio::process::Command::new("/bin/sh");
        command.arg("-c").arg(&cmd);
        if let Some(ref dir) = cwd {
            command.current_dir(dir);
        }
        if let Some(ref vars) = env_vars {
            for (k, v) in vars {
                command.env(k, v);
            }
        }
        command.stdin(if stdin_bytes.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);

        let mut child = command.spawn().map_err(|e| {
            mlua::Error::runtime(format!("shell.exec: failed to spawn command: {e}"))
        })?;

        // Push stdin first so we don't deadlock waiting on a child that
        // expects input. Take stdin → write → drop (closes the pipe).
        if let Some(bytes) = stdin_bytes.as_deref()
            && let Some(mut child_stdin) = child.stdin.take()
        {
            use tokio::io::AsyncWriteExt;
            if let Err(e) = child_stdin.write_all(bytes).await {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(mlua::Error::runtime(format!(
                    "shell.exec: failed to write stdin: {e}"
                )));
            }
            let _ = child_stdin.shutdown().await;
        }

        // Drain stdout/stderr in background tasks so a full pipe buffer
        // (~64KB) doesn't block the child's exit.
        let stdout_h = child.stdout.take();
        let stderr_h = child.stderr.take();
        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut s) = stdout_h {
                use tokio::io::AsyncReadExt;
                let _ = s.read_to_end(&mut buf).await;
            }
            buf
        });
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut s) = stderr_h {
                use tokio::io::AsyncReadExt;
                let _ = s.read_to_end(&mut buf).await;
            }
            buf
        });

        // Optional timeout. On timeout we explicitly kill+wait the child to
        // reap it; without that, kill_on_drop only schedules an async reap
        // that the runtime may not service before tearing down.
        let result = lua.create_table()?;
        match timeout_secs {
            Some(secs) if secs > 0.0 => {
                let dur = std::time::Duration::from_secs_f64(secs);
                match tokio::time::timeout(dur, child.wait()).await {
                    Ok(Ok(status)) => {
                        let stdout = stdout_task.await.unwrap_or_default();
                        let stderr = stderr_task.await.unwrap_or_default();
                        result.set("status", status.code().unwrap_or(-1) as i64)?;
                        result.set("stdout", String::from_utf8_lossy(&stdout).into_owned())?;
                        result.set("stderr", String::from_utf8_lossy(&stderr).into_owned())?;
                        result.set("timed_out", false)?;
                    }
                    Ok(Err(e)) => {
                        return Err(mlua::Error::runtime(format!(
                            "shell.exec: error waiting for process: {e}"
                        )));
                    }
                    Err(_elapsed) => {
                        let _ = child.start_kill();
                        let _ = child.wait().await;
                        stdout_task.abort();
                        stderr_task.abort();
                        result.set("status", -1i64)?;
                        result.set("stdout", "")?;
                        result.set("stderr", "")?;
                        result.set("timed_out", true)?;
                    }
                }
            }
            _ => {
                let status = child.wait().await.map_err(|e| {
                    mlua::Error::runtime(format!("shell.exec: failed to wait for command: {e}"))
                })?;
                let stdout = stdout_task.await.unwrap_or_default();
                let stderr = stderr_task.await.unwrap_or_default();
                result.set("status", status.code().unwrap_or(-1) as i64)?;
                result.set("stdout", String::from_utf8_lossy(&stdout).into_owned())?;
                result.set("stderr", String::from_utf8_lossy(&stderr).into_owned())?;
                result.set("timed_out", false)?;
            }
        }

        Ok(result)
    })?;
    shell_table.set("exec", exec_fn)?;

    lua.globals().set("shell", shell_table)?;
    Ok(())
}
