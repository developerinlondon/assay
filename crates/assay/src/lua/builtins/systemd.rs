/// systemd builtin — D-Bus client for systemd1 / machine1, plus journal reader.
///
/// Linux only. On non-Linux targets the table is registered empty; every
/// function returns a runtime error explaining the platform restriction.
///
/// # Architecture
///
/// One `zbus::Connection` (system bus) is opened lazily on the first call
/// and cached in a `tokio::sync::OnceCell`. All exposed functions use
/// `lua.create_async_function` so mlua drives them as Lua coroutines.
///
/// Journal reading uses `journalctl --output=json` (one-shot subprocess).
/// The pure-Rust `libsystemd` 0.7 crate covers daemon notifications only
/// and has no journal-reading surface.
///
/// # journal_follow
///
/// Not implemented — streaming sd_journal_wait across the FFI/tokio boundary
/// requires a cancellation handle that is tricky to wire through mlua safely.
/// Returns an explicit runtime error.
use mlua::Lua;

pub fn register_systemd(lua: &Lua) -> mlua::Result<()> {
    let t = lua.create_table()?;

    #[cfg(target_os = "linux")]
    impl_linux::register(lua, &t)?;

    #[cfg(not(target_os = "linux"))]
    {
        for name in &[
            "list_units",
            "unit_status",
            "is_active",
            "list_timers",
            "start",
            "stop",
            "restart",
            "reload",
            "list_machines",
            "machine_status",
            "machine_start",
            "machine_poweroff",
            "machine_reboot",
            "machine_terminate",
            "journal",
            "journal_follow",
        ] {
            let n = name.to_string();
            let f = lua.create_async_function(move |_, _args: mlua::MultiValue| {
                let n = n.clone();
                async move {
                    Err::<mlua::Value, _>(mlua::Error::runtime(format!(
                        "systemd.{n}: Linux only"
                    )))
                }
            })?;
            t.set(*name, f)?;
        }
    }

    lua.globals().set("systemd", t)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Linux implementation
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod impl_linux {
    use mlua::{Lua, Table};
    use std::sync::Arc;
    use tokio::sync::OnceCell;
    use zbus::{Connection, zvariant::OwnedObjectPath};

    static SYSTEM_BUS: OnceCell<Arc<Connection>> = OnceCell::const_new();

    async fn system_bus() -> Result<Arc<Connection>, zbus::Error> {
        SYSTEM_BUS
            .get_or_try_init(|| async {
                let conn = Connection::system().await?;
                Ok(Arc::new(conn))
            })
            .await
            .cloned()
    }

    // ── proxy builders ────────────────────────────────────────────────────────

    async fn manager_proxy(conn: &Connection) -> zbus::Result<zbus::Proxy<'_>> {
        zbus::proxy::Builder::new(conn)
            .destination("org.freedesktop.systemd1")?
            .path("/org/freedesktop/systemd1")?
            .interface("org.freedesktop.systemd1.Manager")?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
    }

    async fn machine_manager_proxy(conn: &Connection) -> zbus::Result<zbus::Proxy<'_>> {
        zbus::proxy::Builder::new(conn)
            .destination("org.freedesktop.machine1")?
            .path("/org/freedesktop/machine1")?
            .interface("org.freedesktop.machine1.Manager")?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
    }

    async fn unit_proxy<'c>(conn: &'c Connection, path: &str) -> zbus::Result<zbus::Proxy<'c>> {
        zbus::proxy::Builder::new(conn)
            .destination("org.freedesktop.systemd1")?
            .path(path.to_string())?
            .interface("org.freedesktop.systemd1.Unit")?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
    }

    async fn timer_proxy<'c>(conn: &'c Connection, path: &str) -> zbus::Result<zbus::Proxy<'c>> {
        zbus::proxy::Builder::new(conn)
            .destination("org.freedesktop.systemd1")?
            .path(path.to_string())?
            .interface("org.freedesktop.systemd1.Timer")?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
    }

    async fn machine_proxy<'c>(
        conn: &'c Connection,
        path: &str,
    ) -> zbus::Result<zbus::Proxy<'c>> {
        zbus::proxy::Builder::new(conn)
            .destination("org.freedesktop.machine1")?
            .path(path.to_string())?
            .interface("org.freedesktop.machine1.Machine")?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    fn glob_matches(pattern: &str, name: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        if let Some(suffix) = pattern.strip_prefix('*') {
            name.ends_with(suffix)
        } else if let Some(prefix) = pattern.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            name == pattern
        }
    }

    // ListUnits returns array of 10-tuples:
    // (name, description, load_state, active_state, sub_state,
    //  following, unit_object_path, job_id, job_type, job_object_path)
    type UnitRow = (
        String,          // 0 name
        String,          // 1 description
        String,          // 2 load_state
        String,          // 3 active_state
        String,          // 4 sub_state
        String,          // 5 following
        OwnedObjectPath, // 6 unit_object_path
        u32,             // 7 job_id
        String,          // 8 job_type
        OwnedObjectPath, // 9 job_object_path
    );

    // ListMachines returns array of 4-tuples: (name, class, service, object_path)
    type MachineRow = (String, String, String, OwnedObjectPath);

    // ── public registration ───────────────────────────────────────────────────

    pub fn register(lua: &Lua, t: &Table) -> mlua::Result<()> {
        register_units(lua, t)?;
        register_machines(lua, t)?;
        register_journal(lua, t)?;
        Ok(())
    }

    // ── units ─────────────────────────────────────────────────────────────────

    fn register_units(lua: &Lua, t: &Table) -> mlua::Result<()> {
        let list_units =
            lua.create_async_function(|lua, filter: Option<String>| async move {
                let conn = system_bus()
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("systemd.list_units: {e}")))?;
                let mgr = manager_proxy(&conn)
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("systemd.list_units: {e}")))?;

                let reply = mgr
                    .call_method("ListUnits", &())
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("systemd.list_units: {e}")))?;
                let rows: Vec<UnitRow> = reply
                    .body()
                    .deserialize()
                    .map_err(|e| {
                        mlua::Error::runtime(format!("systemd.list_units: deserialize: {e}"))
                    })?;

                let out = lua.create_table()?;
                let mut idx = 1usize;
                for row in &rows {
                    if let Some(ref pat) = filter
                        && !glob_matches(pat, &row.0)
                    {
                        continue;
                    }
                    let entry = lua.create_table()?;
                    entry.set("name", row.0.clone())?;
                    entry.set("description", row.1.clone())?;
                    entry.set("load", row.2.clone())?;
                    entry.set("active", row.3.clone())?;
                    entry.set("sub", row.4.clone())?;
                    entry.set("following", row.5.clone())?;
                    entry.set("unit_object_path", row.6.as_str().to_string())?;
                    out.set(idx, entry)?;
                    idx += 1;
                }
                Ok(out)
            })?;
        t.set("list_units", list_units)?;

        let unit_status = lua.create_async_function(|lua, name: String| async move {
            let conn = system_bus()
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.unit_status: {e}")))?;
            let mgr = manager_proxy(&conn)
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.unit_status: {e}")))?;

            let reply = mgr
                .call_method("LoadUnit", &name.as_str())
                .await
                .map_err(|e| {
                    mlua::Error::runtime(format!("systemd.unit_status: LoadUnit: {e}"))
                })?;
            let obj_path: OwnedObjectPath = reply.body().deserialize().map_err(|e| {
                mlua::Error::runtime(format!("systemd.unit_status: path: {e}"))
            })?;

            let uproxy = unit_proxy(&conn, obj_path.as_str())
                .await
                .map_err(|e| {
                    mlua::Error::runtime(format!("systemd.unit_status: proxy: {e}"))
                })?;

            let entry = lua.create_table()?;
            entry.set("name", name.clone())?;
            entry.set(
                "load",
                uproxy
                    .get_property::<String>("LoadState")
                    .await
                    .map_err(|e| {
                        mlua::Error::runtime(format!("systemd.unit_status: LoadState: {e}"))
                    })?,
            )?;
            entry.set(
                "active",
                uproxy
                    .get_property::<String>("ActiveState")
                    .await
                    .map_err(|e| {
                        mlua::Error::runtime(format!("systemd.unit_status: ActiveState: {e}"))
                    })?,
            )?;
            entry.set(
                "sub",
                uproxy
                    .get_property::<String>("SubState")
                    .await
                    .map_err(|e| {
                        mlua::Error::runtime(format!("systemd.unit_status: SubState: {e}"))
                    })?,
            )?;
            entry.set(
                "description",
                uproxy
                    .get_property::<String>("Description")
                    .await
                    .map_err(|e| {
                        mlua::Error::runtime(format!("systemd.unit_status: Description: {e}"))
                    })?,
            )?;
            entry.set(
                "fragment_path",
                uproxy
                    .get_property::<String>("FragmentPath")
                    .await
                    .unwrap_or_default(),
            )?;
            entry.set("unit_object_path", obj_path.as_str().to_string())?;

            if let Ok(pid) = uproxy.get_property::<u32>("MainPID").await {
                entry.set("main_pid", pid)?;
            }
            if let Ok(status) = uproxy.get_property::<i32>("ExecMainStatus").await {
                entry.set("exec_main_status", status)?;
            }
            if let Ok(ts) = uproxy.get_property::<u64>("ActiveEnterTimestamp").await {
                entry.set("since", ts)?;
            }

            Ok(entry)
        })?;
        t.set("unit_status", unit_status)?;

        let is_active = lua.create_async_function(|lua, name: String| async move {
            let conn = system_bus()
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.is_active: {e}")))?;
            let mgr = manager_proxy(&conn)
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.is_active: {e}")))?;

            let reply = mgr
                .call_method("LoadUnit", &name.as_str())
                .await
                .map_err(|e| {
                    mlua::Error::runtime(format!("systemd.is_active: LoadUnit: {e}"))
                })?;
            let obj_path: OwnedObjectPath = reply.body().deserialize().map_err(|e| {
                mlua::Error::runtime(format!("systemd.is_active: path: {e}"))
            })?;

            let uproxy = unit_proxy(&conn, obj_path.as_str())
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.is_active: proxy: {e}")))?;

            let active: String = uproxy
                .get_property("ActiveState")
                .await
                .map_err(|e| {
                    mlua::Error::runtime(format!("systemd.is_active: ActiveState: {e}"))
                })?;

            lua.pack(active == "active")
        })?;
        t.set("is_active", is_active)?;

        let list_timers = lua.create_async_function(|lua, ()| async move {
            let conn = system_bus()
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.list_timers: {e}")))?;
            let mgr = manager_proxy(&conn)
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.list_timers: {e}")))?;

            let reply = mgr
                .call_method("ListUnits", &())
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.list_timers: {e}")))?;
            let rows: Vec<UnitRow> = reply
                .body()
                .deserialize()
                .map_err(|e| {
                    mlua::Error::runtime(format!("systemd.list_timers: deserialize: {e}"))
                })?;

            let out = lua.create_table()?;
            let mut idx = 1usize;
            for row in rows {
                if !row.0.ends_with(".timer") {
                    continue;
                }
                let tproxy = timer_proxy(&conn, row.6.as_str()).await.map_err(|e| {
                    mlua::Error::runtime(format!("systemd.list_timers: timer proxy: {e}"))
                })?;

                let next_elapse: u64 = tproxy
                    .get_property("NextElapseUSecRealtime")
                    .await
                    .unwrap_or(0);
                let last_trigger: u64 =
                    tproxy.get_property("LastTriggerUSec").await.unwrap_or(0);
                let activates: String =
                    tproxy.get_property("Unit").await.unwrap_or_default();

                let now_usec = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_micros() as u64)
                    .unwrap_or(0);
                let passed = next_elapse > 0 && now_usec > next_elapse;

                let entry = lua.create_table()?;
                entry.set("unit", row.0.clone())?;
                entry.set("next_elapse_realtime", next_elapse)?;
                entry.set("last_trigger_realtime", last_trigger)?;
                entry.set("passed", passed)?;
                entry.set("activates", activates)?;
                out.set(idx, entry)?;
                idx += 1;
            }
            Ok(out)
        })?;
        t.set("list_timers", list_timers)?;

        // Unit lifecycle: start / stop / restart / reload
        for (lua_name, dbus_method) in &[
            ("start", "StartUnit"),
            ("stop", "StopUnit"),
            ("restart", "RestartUnit"),
            ("reload", "ReloadUnit"),
        ] {
            let dbus_method = dbus_method.to_string();
            let lua_name_str = lua_name.to_string();
            let f = lua.create_async_function(move |_lua, name: String| {
                let dbus_method = dbus_method.clone();
                let lua_name_str = lua_name_str.clone();
                async move {
                    let conn = system_bus().await.map_err(|e| {
                        mlua::Error::runtime(format!("systemd.{lua_name_str}: {e}"))
                    })?;
                    let mgr = manager_proxy(&conn).await.map_err(|e| {
                        mlua::Error::runtime(format!("systemd.{lua_name_str}: {e}"))
                    })?;
                    let reply = mgr
                        .call_method(
                            dbus_method.as_str(),
                            &(name.as_str(), "replace"),
                        )
                        .await
                        .map_err(|e| {
                            mlua::Error::runtime(format!(
                                "systemd.{lua_name_str}: {dbus_method}: {e}"
                            ))
                        })?;
                    let job_path: OwnedObjectPath =
                        reply.body().deserialize().map_err(|e| {
                            mlua::Error::runtime(format!(
                                "systemd.{lua_name_str}: job path: {e}"
                            ))
                        })?;
                    Ok(job_path.as_str().to_string())
                }
            })?;
            t.set(*lua_name, f)?;
        }

        Ok(())
    }

    // ── machines ──────────────────────────────────────────────────────────────

    fn register_machines(lua: &Lua, t: &Table) -> mlua::Result<()> {
        let list_machines = lua.create_async_function(|lua, ()| async move {
            let conn = system_bus()
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.list_machines: {e}")))?;
            let mgr = machine_manager_proxy(&conn)
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.list_machines: {e}")))?;

            let reply = mgr
                .call_method("ListMachines", &())
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.list_machines: {e}")))?;
            let rows: Vec<MachineRow> = reply
                .body()
                .deserialize()
                .map_err(|e| {
                    mlua::Error::runtime(format!("systemd.list_machines: deserialize: {e}"))
                })?;

            let out = lua.create_table()?;
            for (idx, row) in rows.iter().enumerate() {
                let mproxy = machine_proxy(&conn, row.3.as_str())
                    .await
                    .map_err(|e| {
                        mlua::Error::runtime(format!(
                            "systemd.list_machines: machine proxy: {e}"
                        ))
                    })?;

                let leader: u32 = mproxy.get_property("Leader").await.unwrap_or(0);
                let root_dir: String =
                    mproxy.get_property("RootDirectory").await.unwrap_or_default();

                let entry = lua.create_table()?;
                entry.set("name", row.0.clone())?;
                entry.set("class", row.1.clone())?;
                entry.set("service", row.2.clone())?;
                entry.set("leader_pid", leader)?;
                entry.set("root_directory", root_dir)?;
                entry.set("addresses", lua.create_table()?)?;
                out.set(idx + 1, entry)?;
            }
            Ok(out)
        })?;
        t.set("list_machines", list_machines)?;

        let machine_status = lua.create_async_function(|lua, name: String| async move {
            let conn = system_bus()
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.machine_status: {e}")))?;
            let mgr = machine_manager_proxy(&conn)
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.machine_status: {e}")))?;

            let reply = mgr
                .call_method("GetMachine", &name.as_str())
                .await
                .map_err(|e| {
                    mlua::Error::runtime(format!("systemd.machine_status: GetMachine: {e}"))
                })?;
            let obj_path: OwnedObjectPath = reply.body().deserialize().map_err(|e| {
                mlua::Error::runtime(format!("systemd.machine_status: path: {e}"))
            })?;

            let mproxy = machine_proxy(&conn, obj_path.as_str())
                .await
                .map_err(|e| {
                    mlua::Error::runtime(format!("systemd.machine_status: proxy: {e}"))
                })?;

            let entry = lua.create_table()?;
            entry.set(
                "name",
                mproxy
                    .get_property::<String>("Name")
                    .await
                    .map_err(|e| {
                        mlua::Error::runtime(format!("systemd.machine_status: Name: {e}"))
                    })?,
            )?;
            entry.set(
                "class",
                mproxy
                    .get_property::<String>("Class")
                    .await
                    .map_err(|e| {
                        mlua::Error::runtime(format!("systemd.machine_status: Class: {e}"))
                    })?,
            )?;
            entry.set(
                "service",
                mproxy
                    .get_property::<String>("Service")
                    .await
                    .map_err(|e| {
                        mlua::Error::runtime(format!("systemd.machine_status: Service: {e}"))
                    })?,
            )?;
            entry.set(
                "root_directory",
                mproxy
                    .get_property::<String>("RootDirectory")
                    .await
                    .unwrap_or_default(),
            )?;
            if let Ok(leader) = mproxy.get_property::<u32>("Leader").await {
                entry.set("leader_pid", leader)?;
            }
            if let Ok(ts) = mproxy.get_property::<u64>("Timestamp").await {
                entry.set("timestamp", ts)?;
            }
            entry.set("addresses", lua.create_table()?)?;
            Ok(entry)
        })?;
        t.set("machine_status", machine_status)?;

        let machine_start = lua.create_async_function(|_lua, name: String| async move {
            let conn = system_bus()
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.machine_start: {e}")))?;
            let mgr = manager_proxy(&conn)
                .await
                .map_err(|e| mlua::Error::runtime(format!("systemd.machine_start: {e}")))?;
            let svc = format!("systemd-nspawn@{name}.service");
            let reply = mgr
                .call_method("StartUnit", &(svc.as_str(), "replace"))
                .await
                .map_err(|e| {
                    mlua::Error::runtime(format!("systemd.machine_start: StartUnit: {e}"))
                })?;
            let job_path: OwnedObjectPath =
                reply.body().deserialize().map_err(|e| {
                    mlua::Error::runtime(format!("systemd.machine_start: job path: {e}"))
                })?;
            Ok(job_path.as_str().to_string())
        })?;
        t.set("machine_start", machine_start)?;

        for (lua_name, dbus_method) in &[
            ("machine_poweroff", "PowerOff"),
            ("machine_reboot", "Reboot"),
            ("machine_terminate", "Terminate"),
        ] {
            let dbus_method = dbus_method.to_string();
            let lua_name_str = lua_name.to_string();
            let f = lua.create_async_function(move |_lua, name: String| {
                let dbus_method = dbus_method.clone();
                let lua_name_str = lua_name_str.clone();
                async move {
                    let conn = system_bus().await.map_err(|e| {
                        mlua::Error::runtime(format!("systemd.{lua_name_str}: {e}"))
                    })?;
                    let mgr = machine_manager_proxy(&conn).await.map_err(|e| {
                        mlua::Error::runtime(format!("systemd.{lua_name_str}: {e}"))
                    })?;
                    mgr.call_method(dbus_method.as_str(), &name.as_str())
                        .await
                        .map_err(|e| {
                            mlua::Error::runtime(format!(
                                "systemd.{lua_name_str}: {dbus_method}: {e}"
                            ))
                        })?;
                    Ok(())
                }
            })?;
            t.set(*lua_name, f)?;
        }

        Ok(())
    }

    // ── journal ───────────────────────────────────────────────────────────────

    fn register_journal(lua: &Lua, t: &Table) -> mlua::Result<()> {
        // systemd.journal(opts) — one-shot read of last N journal entries.
        //
        // opts = { unit?, machine?, since?, until?, lines?=200, priority?=7 }
        // Returns [{ts, hostname, unit, message, priority, transport}, ...]
        //
        // Uses `journalctl --output=json` subprocess — the pure-Rust
        // `libsystemd` 0.7 crate has no journal-reading surface (it covers
        // daemon notifications only).
        let journal =
            lua.create_async_function(|lua, opts: Option<mlua::Table>| async move {
                let (unit, machine, since, until, lines, priority) =
                    if let Some(ref o) = opts {
                        (
                            o.get::<Option<String>>("unit")?,
                            o.get::<Option<String>>("machine")?,
                            o.get::<Option<String>>("since")?,
                            o.get::<Option<String>>("until")?,
                            o.get::<Option<u32>>("lines")?.unwrap_or(200),
                            o.get::<Option<u8>>("priority")?.unwrap_or(7),
                        )
                    } else {
                        (None, None, None, None, 200u32, 7u8)
                    };

                let mut cmd = tokio::process::Command::new("journalctl");
                cmd.arg("--output=json")
                    .arg("--no-pager")
                    .arg(format!("-n{lines}"))
                    .arg(format!("--priority={priority}"));

                if let Some(ref u) = unit {
                    cmd.arg(format!("--unit={u}"));
                }
                if let Some(ref m) = machine {
                    cmd.arg(format!("--machine={m}"));
                }
                if let Some(ref s) = since {
                    cmd.arg(format!("--since={s}"));
                }
                if let Some(ref u) = until {
                    cmd.arg(format!("--until={u}"));
                }

                let output = cmd.output().await.map_err(|e| {
                    mlua::Error::runtime(format!("systemd.journal: journalctl: {e}"))
                })?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(mlua::Error::runtime(format!(
                        "systemd.journal: journalctl exited non-zero: {stderr}"
                    )));
                }

                let stdout = String::from_utf8_lossy(&output.stdout);
                let out = lua.create_table()?;
                let mut idx = 1usize;

                for line in stdout.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let v: serde_json::Value =
                        serde_json::from_str(line).map_err(|e| {
                            mlua::Error::runtime(format!(
                                "systemd.journal: JSON parse: {e}"
                            ))
                        })?;
                    let obj = match v.as_object() {
                        Some(o) => o,
                        None => continue,
                    };

                    let get_str = |key: &str| -> String {
                        obj.get(key)
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    };

                    let ts: u64 = obj
                        .get("__REALTIME_TIMESTAMP")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);

                    let pri: u8 = obj
                        .get("PRIORITY")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(7);

                    let entry = lua.create_table()?;
                    entry.set("ts", ts)?;
                    entry.set("hostname", get_str("_HOSTNAME"))?;
                    entry.set("unit", get_str("_SYSTEMD_UNIT"))?;
                    entry.set("message", get_str("MESSAGE"))?;
                    entry.set("priority", pri)?;
                    entry.set("transport", get_str("_TRANSPORT"))?;
                    out.set(idx, entry)?;
                    idx += 1;
                }

                Ok(out)
            })?;
        t.set("journal", journal)?;

        let journal_follow =
            lua.create_async_function(|_, _args: mlua::MultiValue| async move {
                Err::<mlua::Value, _>(mlua::Error::runtime(
                    "systemd.journal_follow: not yet implemented; \
                     tracked in plan 18 Phase 3 followup",
                ))
            })?;
        t.set("journal_follow", journal_follow)?;

        Ok(())
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[cfg(test)]
    mod tests {
        use mlua::{Lua, LuaOptions, StdLib};

        fn make_lua() -> Lua {
            Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())
                .expect("Lua init failed")
        }

        fn setup(lua: &Lua) {
            let t = lua.create_table().unwrap();
            super::register(lua, &t).unwrap();
            lua.globals().set("systemd", t).unwrap();
        }

        // D-Bus tests require a running system bus and a live systemd.
        // Marked #[ignore] so `cargo test` passes in minimal CI environments
        // (Docker containers without a system bus, etc.).
        // Run with: cargo test --lib systemd:: -- --include-ignored

        #[tokio::test]
        #[ignore = "requires running system bus (systemd)"]
        async fn list_units_returns_init_scope() {
            let lua = make_lua();
            setup(&lua);
            let result: mlua::Table = lua
                .load("return systemd.list_units()")
                .eval_async()
                .await
                .expect("list_units failed");
            let found = (1..=result.raw_len()).any(|i| {
                result
                    .get::<mlua::Table>(i)
                    .ok()
                    .and_then(|t| t.get::<String>("name").ok())
                    .map(|n| n == "init.scope")
                    .unwrap_or(false)
            });
            assert!(found, "init.scope not found in list_units()");
        }

        #[tokio::test]
        #[ignore = "requires running system bus (systemd)"]
        async fn is_active_returns_bool() {
            let lua = make_lua();
            setup(&lua);
            let active: bool = lua
                .load(r#"return systemd.is_active("init.scope")"#)
                .eval_async()
                .await
                .expect("is_active failed");
            assert!(active, "init.scope should be active");
        }

        #[tokio::test]
        #[ignore = "requires running system bus (systemd)"]
        async fn list_machines_callable() {
            let lua = make_lua();
            setup(&lua);
            let _result: mlua::Table = lua
                .load("return systemd.list_machines()")
                .eval_async()
                .await
                .expect("list_machines failed");
        }

        #[tokio::test]
        #[ignore = "requires journalctl (systemd)"]
        async fn journal_smoke() {
            let lua = make_lua();
            setup(&lua);
            let result: mlua::Table = lua
                .load("return systemd.journal({lines=5})")
                .eval_async()
                .await
                .expect("journal failed");
            let len = result.raw_len();
            assert!(len <= 5, "expected <= 5 entries, got {len}");
            for i in 1..=len {
                let entry: mlua::Table = result.get(i).unwrap();
                let _msg: String = entry.get("message").expect("entry.message missing");
            }
        }

        #[tokio::test]
        #[ignore = "requires journalctl (systemd)"]
        async fn journal_filter_by_priority() {
            let lua = make_lua();
            setup(&lua);
            let result: mlua::Table = lua
                .load("return systemd.journal({lines=10, priority=3})")
                .eval_async()
                .await
                .expect("journal priority filter failed");
            for i in 1..=result.raw_len() {
                let entry: mlua::Table = result.get(i).unwrap();
                let pri: u8 = entry.get("priority").unwrap_or(255);
                assert!(
                    pri <= 3,
                    "expected priority <= 3, got {pri} in entry {i}"
                );
            }
        }
    }
}
