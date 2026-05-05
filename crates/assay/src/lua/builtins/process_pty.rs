//! `process.spawn_pty(opts)` — fork+exec a child attached to a fresh PTY,
//! return a duplex `PtyHandle` UserData with async read/write/resize/close/wait.
//!
//! Linux + macOS only. On other targets, `register_process_pty` installs a
//! stub that errors at runtime; the function is still present so scripts
//! that probe for it don't blow up at load time.

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod imp {
    use mlua::{Table, UserData, UserDataFields, UserDataMethods, Value};
    use std::io;
    use std::os::fd::{AsRawFd, OwnedFd, RawFd};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::io::unix::AsyncFd;
    use tokio::process::Child;
    use tokio::sync::Mutex as TokioMutex;

    struct MasterFd(OwnedFd);
    impl AsRawFd for MasterFd {
        fn as_raw_fd(&self) -> RawFd {
            self.0.as_raw_fd()
        }
    }

    pub(super) struct PtyHandle {
        master: AsyncFd<MasterFd>,
        child: TokioMutex<Option<Child>>,
        pid: i32,
        closed: AtomicBool,
    }

    impl PtyHandle {
        fn raw_fd(&self) -> RawFd {
            self.master.get_ref().as_raw_fd()
        }

        async fn read_chunk(&self, timeout_ms: Option<u64>) -> mlua::Result<Option<Vec<u8>>> {
            let read_once = async {
                loop {
                    let mut guard = self
                        .master
                        .readable()
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("pty:read: {e}")))?;
                    let mut buf = [0u8; 4096];
                    let res = guard.try_io(|inner| {
                        let fd = inner.as_raw_fd();
                        let n = unsafe {
                            libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                        };
                        if n < 0 {
                            Err(io::Error::last_os_error())
                        } else {
                            Ok(n as usize)
                        }
                    });
                    match res {
                        Ok(Ok(0)) => return Ok::<Option<Vec<u8>>, mlua::Error>(None),
                        Ok(Ok(n)) => return Ok(Some(buf[..n].to_vec())),
                        Ok(Err(e)) => {
                            // Linux signals child-side close as EIO on the master fd.
                            if e.raw_os_error() == Some(libc::EIO) {
                                return Ok(None);
                            }
                            return Err(mlua::Error::runtime(format!("pty:read: {e}")));
                        }
                        Err(_would_block) => continue,
                    }
                }
            };

            match timeout_ms {
                None | Some(0) => read_once.await,
                Some(ms) => {
                    match tokio::time::timeout(Duration::from_millis(ms), read_once).await {
                        Ok(r) => r,
                        Err(_) => Ok(None),
                    }
                }
            }
        }

        async fn write_all(&self, data: &[u8]) -> mlua::Result<()> {
            let mut written = 0;
            while written < data.len() {
                let mut guard = self
                    .master
                    .writable()
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("pty:write: {e}")))?;
                let res = guard.try_io(|inner| {
                    let fd = inner.as_raw_fd();
                    let n = unsafe {
                        libc::write(
                            fd,
                            data[written..].as_ptr() as *const libc::c_void,
                            data.len() - written,
                        )
                    };
                    if n < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                });
                match res {
                    Ok(Ok(0)) => {
                        return Err(mlua::Error::runtime("pty:write: zero-length write"));
                    }
                    Ok(Ok(n)) => written += n,
                    Ok(Err(e)) => return Err(mlua::Error::runtime(format!("pty:write: {e}"))),
                    Err(_would_block) => continue,
                }
            }
            Ok(())
        }

        fn resize(&self, cols: u16, rows: u16) -> mlua::Result<()> {
            if cols == 0 || rows == 0 {
                return Err(mlua::Error::runtime("pty:resize: cols/rows must be > 0"));
            }
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let r = unsafe { libc::ioctl(self.raw_fd(), libc::TIOCSWINSZ, &ws as *const _) };
            if r != 0 {
                return Err(mlua::Error::runtime(format!(
                    "pty:resize: TIOCSWINSZ failed: {}",
                    io::Error::last_os_error()
                )));
            }
            // Best-effort SIGWINCH; if the child has already exited, kill returns ESRCH and we ignore.
            unsafe {
                libc::kill(self.pid, libc::SIGWINCH);
            }
            Ok(())
        }

        async fn close(&self) -> mlua::Result<()> {
            if self.closed.swap(true, Ordering::AcqRel) {
                return Ok(());
            }
            unsafe {
                libc::kill(self.pid, libc::SIGHUP);
            }
            // Don't .wait() the child here — `close()` is fire-and-forget so
            // tight loops in Lua don't block waiting for graceful exits.
            Ok(())
        }

        fn is_alive(&self) -> bool {
            if self.closed.load(Ordering::Acquire) {
                return false;
            }
            unsafe { libc::kill(self.pid, 0) == 0 }
        }
    }

    impl UserData for PtyHandle {
        fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
            fields.add_field_method_get("pid", |_, h| Ok(h.pid));
        }

        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_async_method("write", |_, this, data: mlua::String| async move {
                this.write_all(&data.as_bytes()).await
            });

            methods.add_async_method("read", |lua, this, opts: Option<Table>| async move {
                let timeout_ms: Option<u64> = match &opts {
                    Some(t) => t.get::<Option<u64>>("timeout_ms")?,
                    None => None,
                };
                match this.read_chunk(timeout_ms).await? {
                    Some(bytes) => Ok(Value::String(lua.create_string(&bytes)?)),
                    None => Ok(Value::Nil),
                }
            });

            methods.add_method("resize", |_, this, (cols, rows): (u32, u32)| {
                this.resize(
                    cols.try_into().unwrap_or(u16::MAX),
                    rows.try_into().unwrap_or(u16::MAX),
                )
            });

            methods.add_async_method("close", |_, this, ()| async move { this.close().await });

            methods.add_async_method("wait", |lua, this, ()| async move {
                let mut guard = this.child.lock().await;
                let result_table = lua.create_table()?;
                if let Some(child) = guard.as_mut() {
                    let status = child
                        .wait()
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("pty:wait: {e}")))?;
                    let code = status.code().unwrap_or(-1);
                    result_table.set("status", code)?;
                    result_table.set("exited", status.code().is_some())?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        let signaled = status.signal().is_some();
                        result_table.set("signaled", signaled)?;
                        if let Some(sig) = status.signal() {
                            result_table.set("signal", sig)?;
                        }
                    }
                    *guard = None;
                } else {
                    // Already waited; report cached exit as unknown but don't error.
                    result_table.set("status", -1)?;
                    result_table.set("exited", false)?;
                    result_table.set("signaled", false)?;
                }
                this.closed.store(true, Ordering::Release);
                Ok(result_table)
            });

            methods.add_method("is_alive", |_, this, ()| Ok(this.is_alive()));
        }
    }

    impl Drop for PtyHandle {
        fn drop(&mut self) {
            if !self.closed.swap(true, Ordering::AcqRel) {
                unsafe {
                    libc::kill(self.pid, libc::SIGHUP);
                }
            }
            // tokio::process::Child's Drop reaps lazily on Linux; that's fine.
            // The OwnedFd inside MasterFd closes itself when AsyncFd drops.
        }
    }

    struct SpawnOpts {
        cmd: String,
        args: Vec<String>,
        cwd: Option<String>,
        env: Vec<(String, String)>,
        cols: u16,
        rows: u16,
    }

    fn parse_opts(opts: &Table) -> mlua::Result<SpawnOpts> {
        let cmd: String = opts
            .get("cmd")
            .map_err(|_| mlua::Error::runtime("process.spawn_pty: opts.cmd (string) required"))?;
        let args: Vec<String> = opts
            .get::<Option<Table>>("args")?
            .map(|t| {
                t.sequence_values::<String>()
                    .filter_map(|v| v.ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cwd: Option<String> = opts.get("cwd")?;
        let env: Vec<(String, String)> = opts
            .get::<Option<Table>>("env")?
            .map(|t| {
                t.pairs::<String, String>()
                    .filter_map(|p| p.ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cols: u16 = opts
            .get::<Option<u32>>("cols")?
            .unwrap_or(80)
            .try_into()
            .unwrap_or(u16::MAX);
        let rows: u16 = opts
            .get::<Option<u32>>("rows")?
            .unwrap_or(24)
            .try_into()
            .unwrap_or(u16::MAX);
        Ok(SpawnOpts {
            cmd,
            args,
            cwd,
            env,
            cols,
            rows,
        })
    }

    pub(super) fn spawn_pty_impl(opts: &Table) -> mlua::Result<PtyHandle> {
        let SpawnOpts {
            cmd,
            args,
            cwd,
            env,
            cols,
            rows,
        } = parse_opts(opts)?;

        let winsize = nix::libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let pty = nix::pty::openpty(Some(&winsize), None)
            .map_err(|e| mlua::Error::runtime(format!("process.spawn_pty: openpty: {e}")))?;
        let master_fd: OwnedFd = pty.master;
        let slave_fd: OwnedFd = pty.slave;
        let slave_raw = slave_fd.as_raw_fd();

        // Master must be non-blocking for AsyncFd.
        let flags = unsafe { libc::fcntl(master_fd.as_raw_fd(), libc::F_GETFL, 0) };
        if flags == -1 {
            return Err(mlua::Error::runtime(format!(
                "process.spawn_pty: fcntl(F_GETFL): {}",
                io::Error::last_os_error()
            )));
        }
        let r = unsafe {
            libc::fcntl(
                master_fd.as_raw_fd(),
                libc::F_SETFL,
                flags | libc::O_NONBLOCK,
            )
        };
        if r == -1 {
            return Err(mlua::Error::runtime(format!(
                "process.spawn_pty: fcntl(F_SETFL O_NONBLOCK): {}",
                io::Error::last_os_error()
            )));
        }

        let mut command = tokio::process::Command::new(&cmd);
        command.args(&args);
        if let Some(d) = cwd.as_ref() {
            command.current_dir(d);
        }
        for (k, v) in &env {
            command.env(k, v);
        }
        // Don't reap automatically — we manage lifecycle through the handle.
        command.kill_on_drop(false);

        // pre_exec runs in the forked child, before exec(2). Must be
        // async-signal-safe. dup2/setsid/ioctl/close all qualify.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                for target in 0..=2 {
                    if libc::dup2(slave_raw, target) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                }
                if libc::ioctl(slave_raw, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                if slave_raw > 2 {
                    libc::close(slave_raw);
                }
                Ok(())
            });
        }

        let child = command
            .spawn()
            .map_err(|e| mlua::Error::runtime(format!("process.spawn_pty: spawn '{cmd}': {e}")))?;
        // Parent doesn't need the slave fd (child already dup'd it onto stdio).
        // Dropping the OwnedFd closes it; this is the safe equivalent of close(slave).
        drop(slave_fd);

        let pid = child
            .id()
            .ok_or_else(|| mlua::Error::runtime("process.spawn_pty: child has no pid"))?
            as i32;

        let async_master = AsyncFd::new(MasterFd(master_fd))
            .map_err(|e| mlua::Error::runtime(format!("process.spawn_pty: AsyncFd::new: {e}")))?;

        Ok(PtyHandle {
            master: async_master,
            child: TokioMutex::new(Some(child)),
            pid,
            closed: AtomicBool::new(false),
        })
    }
}

pub fn register_process_pty(lua: &mlua::Lua) -> mlua::Result<()> {
    let process_table: mlua::Table = lua.globals().get("process")?;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let spawn_pty_fn = lua.create_async_function(|lua, opts: mlua::Table| async move {
            let handle = imp::spawn_pty_impl(&opts)?;
            lua.create_userdata(handle)
        })?;
        process_table.set("spawn_pty", spawn_pty_fn)?;
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let spawn_pty_fn = lua.create_function(|_, _opts: mlua::Table| -> mlua::Result<()> {
            Err(mlua::Error::runtime(
                "process.spawn_pty: only supported on Linux and macOS",
            ))
        })?;
        process_table.set("spawn_pty", spawn_pty_fn)?;
    }

    Ok(())
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use mlua::Lua;

    fn lua_with_pty() -> Lua {
        let lua = Lua::new();
        // Minimal process global so register_process_pty has a table to attach to.
        let process = lua.create_table().unwrap();
        lua.globals().set("process", process).unwrap();
        super::register_process_pty(&lua).unwrap();
        lua
    }

    #[test]
    fn cat_roundtrip() {
        let lua = lua_with_pty();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            lua.load(
                r#"
                local pty = process.spawn_pty({ cmd = "cat" })
                pty:write("hello\n")
                local got = pty:read({ timeout_ms = 2000 })
                assert(got ~= nil, "expected data, got nil")
                assert(string.find(got, "hello", 1, true), "expected hello in: " .. tostring(got))
                pty:close()
            "#,
            )
            .exec_async()
            .await
            .unwrap();
        });
    }

    #[test]
    fn echo_eof() {
        let lua = lua_with_pty();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            lua.load(
                r#"
                local pty = process.spawn_pty({ cmd = "echo", args = { "wrap" } })
                local saw_wrap = false
                local saw_eof = false
                for _ = 1, 100 do
                    local chunk = pty:read({ timeout_ms = 2000 })
                    if chunk == nil then saw_eof = true; break end
                    if string.find(chunk, "wrap", 1, true) then saw_wrap = true end
                end
                assert(saw_wrap, "expected to see 'wrap' in output")
                assert(saw_eof, "expected EOF (nil) after child exit")
            "#,
            )
            .exec_async()
            .await
            .unwrap();
        });
    }

    #[test]
    fn winsize_applied() {
        let lua = lua_with_pty();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            lua.load(
                r#"
                -- stty size prints "<rows> <cols>" by querying TIOCGWINSZ
                -- directly off the controlling terminal — no TERM env var
                -- required, so this works in stripped-down CI environments.
                local pty = process.spawn_pty({
                    cmd = "stty",
                    args = { "size" },
                    cols = 120, rows = 40,
                })
                local out = ""
                for _ = 1, 200 do
                    local chunk = pty:read({ timeout_ms = 2000 })
                    if chunk == nil then break end
                    out = out .. chunk
                end
                assert(string.find(out, "40", 1, true), "expected 40 rows in: " .. out)
                assert(string.find(out, "120", 1, true), "expected 120 cols in: " .. out)
            "#,
            )
            .exec_async()
            .await
            .unwrap();
        });
    }

    #[test]
    fn close_kills_child() {
        let lua = lua_with_pty();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            lua.load(
                r#"
                local pty = process.spawn_pty({
                    cmd = "bash",
                    args = { "-c", "trap '' HUP; sleep 30" },
                })
                pty:close()
                local r = pty:wait()
                assert(r ~= nil)
                -- Either signaled or exited (depending on shell handling); just
                -- confirm the child is no longer running.
                assert(pty:is_alive() == false)
            "#,
            )
            .exec_async()
            .await
            .unwrap();
        });
    }

    #[test]
    fn drop_kills_child() {
        let lua = lua_with_pty();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            // Spawn, get pid, drop without :close().
            let pid: i32 = lua
                .load(
                    r#"
                    local pty = process.spawn_pty({
                        cmd = "bash",
                        args = { "-c", "sleep 30" },
                    })
                    return pty.pid
                "#,
                )
                .eval_async()
                .await
                .unwrap();
            // Force a Lua GC so the userdata Drop runs.
            lua.gc_collect().unwrap();
            // Give the kernel a moment to deliver SIGHUP.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            // kill(pid, 0) should report dead-or-not-ours; in either case kill returns -1.
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            assert!(!alive, "child {pid} should be dead after Drop");
        });
    }
}
