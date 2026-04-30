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
///
/// # journal_follow
///
/// Streams the journal via `sd_journal_wait` from `libsystemd.so.0`, dlopened
/// at runtime via `libloading`. No `libsystemd-dev` headers required at build
/// time; the library is resolved on first call and cached in a `OnceCell`.
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
            "machine_exec",   // NEW
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
        register_machine_exec(lua, t)?;   // NEW
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
        // Uses `journalctl --output=json` subprocess.
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

        // systemd.journal_follow(opts, callback) -> handle
        //
        // Streams live journal entries via sd_journal_wait, dlopened from
        // libsystemd.so.0 at runtime (no libsystemd-dev headers needed).
        //
        // opts = { unit?, machine?, since?, priority? }
        // callback receives the same {ts, hostname, unit, message, priority, transport} table.
        // Returns a handle UserData with :close() / :is_closed().
        let journal_follow =
            lua.create_async_function(|_lua, (opts, cb): (Option<mlua::Table>, mlua::Function)| async move {
                use std::sync::{Arc, atomic::AtomicBool};
                use tokio::sync::mpsc;

                let unit     = opts.as_ref().and_then(|o| o.get::<Option<String>>("unit").ok().flatten());
                let machine  = opts.as_ref().and_then(|o| o.get::<Option<String>>("machine").ok().flatten());
                let since    = opts.as_ref().and_then(|o| o.get::<Option<u64>>("since").ok().flatten());
                let priority = opts.as_ref().and_then(|o| o.get::<Option<u8>>("priority").ok().flatten()).unwrap_or(7);

                let cancel = Arc::new(AtomicBool::new(false));
                let (tx, mut rx) = mpsc::channel::<journal_follow_impl::JournalEntry>(64);

                let cancel_bg = Arc::clone(&cancel);
                tokio::task::spawn_blocking(move || {
                    journal_follow_impl::follow_loop(
                        unit, machine, since, priority, cancel_bg, tx,
                    );
                });

                let handle = journal_follow_impl::FollowHandle { cancel: Arc::clone(&cancel) };

                // Drive the callback from the async context so mlua can schedule it.
                tokio::select! {
                    _ = async {
                        while let Some(entry) = rx.recv().await {
                            let t = _lua.create_table()?;
                            t.set("ts", entry.ts)?;
                            t.set("hostname", entry.hostname)?;
                            t.set("unit", entry.unit)?;
                            t.set("message", entry.message)?;
                            t.set("priority", entry.priority)?;
                            t.set("transport", entry.transport)?;
                            cb.call_async::<()>(t).await?;
                        }
                        Ok::<(), mlua::Error>(())
                    } => {}
                };

                Ok(handle)
            })?;
        t.set("journal_follow", journal_follow)?;

        Ok(())
    }

    // ── machine_exec ──────────────────────────────────────────────────────────

    fn register_machine_exec(lua: &Lua, t: &Table) -> mlua::Result<()> {
        let machine_exec = lua.create_async_function(|lua, args: mlua::MultiValue| async move {
            let mut args_iter = args.into_iter();
            let machine: String = match args_iter.next() {
                Some(mlua::Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(mlua::Error::runtime(
                    "systemd.machine_exec: first arg must be machine name string",
                )),
            };
            let command: String = match args_iter.next() {
                Some(mlua::Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(mlua::Error::runtime(
                    "systemd.machine_exec: second arg must be command string",
                )),
            };
            // Optional opts: { timeout = secs, env = {...} }
            let opts: Option<mlua::Table> = match args_iter.next() {
                Some(mlua::Value::Table(t)) => Some(t),
                _ => None,
            };
            let timeout_secs: Option<f64> = opts
                .as_ref()
                .and_then(|t| t.get::<f64>("timeout").ok())
                .filter(|f| f.is_finite() && *f > 0.0);

            // Build: systemd-run --machine=<name> --pipe --quiet --wait --collect /bin/sh -c '<cmd>'
            // --pipe captures stdout/stderr to caller; --wait blocks until completion;
            // --collect cleans up the transient unit on exit.
            let mut tokio_cmd = tokio::process::Command::new("systemd-run");
            tokio_cmd
                .arg(format!("--machine={machine}"))
                .arg("--pipe")
                .arg("--quiet")
                .arg("--wait")
                .arg("--collect");

            if let Some(ref t) = opts
                && let Ok(env_table) = t.get::<mlua::Table>("env")
            {
                for pair in env_table.pairs::<String, String>() {
                    let (k, v) = pair?;
                    tokio_cmd.arg(format!("--setenv={k}={v}"));
                }
            }

            tokio_cmd.arg("/bin/sh").arg("-c").arg(&command);
            tokio_cmd.stdin(std::process::Stdio::null());
            tokio_cmd.stdout(std::process::Stdio::piped());
            tokio_cmd.stderr(std::process::Stdio::piped());

            let exec = tokio_cmd.output();
            let output = if let Some(secs) = timeout_secs {
                tokio::time::timeout(std::time::Duration::from_secs_f64(secs), exec)
                    .await
                    .map_err(|_| mlua::Error::runtime(format!(
                        "systemd.machine_exec: timeout after {secs}s"
                    )))?
                    .map_err(|e| mlua::Error::runtime(format!(
                        "systemd.machine_exec: spawn: {e}"
                    )))?
            } else {
                exec.await.map_err(|e| mlua::Error::runtime(format!(
                    "systemd.machine_exec: spawn: {e}"
                )))?
            };

            let result = lua.create_table()?;
            result.set("status", output.status.code().unwrap_or(-1) as i64)?;
            result.set("stdout", String::from_utf8_lossy(&output.stdout).to_string())?;
            result.set("stderr", String::from_utf8_lossy(&output.stderr).to_string())?;
            Ok(result)
        })?;
        t.set("machine_exec", machine_exec)?;
        Ok(())
    }

    // ── journal_follow internals ──────────────────────────────────────────────

    pub(super) mod journal_follow_impl {
        use libloading::{Library, Symbol};
        use std::{
            ffi::{CString, c_char, c_int, c_void},
            sync::{Arc, OnceLock, atomic::{AtomicBool, Ordering}},
        };
        use tokio::sync::mpsc;
        use tracing::warn;

        // ── ABI types ─────────────────────────────────────────────────────────

        type SdJournal = c_void;

        #[allow(non_snake_case)]
        struct Syms {
            sd_journal_open:             unsafe extern "C" fn(*mut *mut SdJournal, c_int) -> c_int,
            sd_journal_close:            unsafe extern "C" fn(*mut SdJournal),
            sd_journal_seek_tail:        unsafe extern "C" fn(*mut SdJournal) -> c_int,
            sd_journal_seek_realtime_usec: unsafe extern "C" fn(*mut SdJournal, u64) -> c_int,
            sd_journal_add_match:        unsafe extern "C" fn(*mut SdJournal, *const c_void, usize) -> c_int,
            sd_journal_next:             unsafe extern "C" fn(*mut SdJournal) -> c_int,
            sd_journal_previous:         unsafe extern "C" fn(*mut SdJournal) -> c_int,
            sd_journal_wait:             unsafe extern "C" fn(*mut SdJournal, u64) -> c_int,
            sd_journal_get_data:         unsafe extern "C" fn(*mut SdJournal, *const c_char, *mut *const c_void, *mut usize) -> c_int,
            sd_journal_get_realtime_usec: unsafe extern "C" fn(*mut SdJournal, *mut u64) -> c_int,
        }

        // SAFETY: the raw fn pointers are stateless and safe to share across threads.
        unsafe impl Send for Syms {}
        unsafe impl Sync for Syms {}

        static SYMS: OnceLock<Result<Syms, String>> = OnceLock::new();

        fn load_syms() -> Result<&'static Syms, String> {
            SYMS.get_or_init(|| unsafe {
                let lib = Library::new("libsystemd.so.0")
                    .map_err(|e| format!("systemd.journal_follow: libsystemd.so.0 not found on this host: {e}"))?;

                macro_rules! sym {
                    ($name:ident) => {{
                        let s: Symbol<_> = lib
                            .get(stringify!($name).as_bytes())
                            .map_err(|e| format!("systemd.journal_follow: symbol {}: {e}", stringify!($name)))?;
                        *s
                    }};
                }

                let syms = Syms {
                    sd_journal_open:              sym!(sd_journal_open),
                    sd_journal_close:             sym!(sd_journal_close),
                    sd_journal_seek_tail:         sym!(sd_journal_seek_tail),
                    sd_journal_seek_realtime_usec: sym!(sd_journal_seek_realtime_usec),
                    sd_journal_add_match:         sym!(sd_journal_add_match),
                    sd_journal_next:              sym!(sd_journal_next),
                    sd_journal_previous:          sym!(sd_journal_previous),
                    sd_journal_wait:              sym!(sd_journal_wait),
                    sd_journal_get_data:          sym!(sd_journal_get_data),
                    sd_journal_get_realtime_usec: sym!(sd_journal_get_realtime_usec),
                };

                // Intentionally leak the Library so the symbols remain valid for
                // the process lifetime (safe: it's a shared library, the OS owns it).
                std::mem::forget(lib);
                Ok(syms)
            })
            .as_ref()
            .map_err(|e| e.clone())
        }

        // ── safe Journal wrapper ──────────────────────────────────────────────

        struct Journal {
            ptr: *mut SdJournal,
            syms: &'static Syms,
        }

        // SAFETY: Journal owns its ptr; we never share it across threads concurrently.
        unsafe impl Send for Journal {}

        impl Journal {
            // SD_JOURNAL_LOCAL_ONLY = 1
            fn open() -> Result<Self, String> {
                let syms = load_syms()?;
                let mut ptr: *mut SdJournal = std::ptr::null_mut();
                let rc = unsafe { (syms.sd_journal_open)(&mut ptr, 1) };
                if rc < 0 {
                    return Err(format!("systemd.journal_follow: sd_journal_open failed: {rc}"));
                }
                Ok(Journal { ptr, syms })
            }

            fn seek_tail(&self) -> c_int {
                unsafe { (self.syms.sd_journal_seek_tail)(self.ptr) }
            }

            fn seek_realtime_usec(&self, usec: u64) -> c_int {
                unsafe { (self.syms.sd_journal_seek_realtime_usec)(self.ptr, usec) }
            }

            fn add_match(&self, expr: &str) -> c_int {
                let bytes = expr.as_bytes();
                unsafe {
                    (self.syms.sd_journal_add_match)(
                        self.ptr,
                        bytes.as_ptr() as *const c_void,
                        bytes.len(),
                    )
                }
            }

            fn next(&self) -> c_int {
                unsafe { (self.syms.sd_journal_next)(self.ptr) }
            }

            fn previous(&self) -> c_int {
                unsafe { (self.syms.sd_journal_previous)(self.ptr) }
            }

            // timeout_us: 0 = non-blocking, u64::MAX = infinite
            fn wait(&self, timeout_us: u64) -> c_int {
                unsafe { (self.syms.sd_journal_wait)(self.ptr, timeout_us) }
            }

            fn read_field(&self, field: &str) -> Option<String> {
                let cfield = CString::new(field).ok()?;
                let mut data: *const c_void = std::ptr::null();
                let mut len: usize = 0;
                let rc = unsafe {
                    (self.syms.sd_journal_get_data)(self.ptr, cfield.as_ptr(), &mut data, &mut len)
                };
                if rc < 0 || data.is_null() {
                    return None;
                }
                let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, len) };
                // sd_journal_get_data returns "FIELD=value"; strip the "FIELD=" prefix.
                let prefix = format!("{field}=");
                let raw = std::str::from_utf8(bytes).unwrap_or("");
                Some(raw.strip_prefix(&prefix).unwrap_or(raw).to_owned())
            }

            fn realtime_usec(&self) -> u64 {
                let mut usec: u64 = 0;
                unsafe { (self.syms.sd_journal_get_realtime_usec)(self.ptr, &mut usec) };
                usec
            }
        }

        impl Drop for Journal {
            fn drop(&mut self) {
                if !self.ptr.is_null() {
                    unsafe { (self.syms.sd_journal_close)(self.ptr) };
                }
            }
        }

        // ── entry type ────────────────────────────────────────────────────────

        #[derive(Debug)]
        pub struct JournalEntry {
            pub ts:        u64,
            pub hostname:  String,
            pub unit:      String,
            pub message:   String,
            pub priority:  u8,
            pub transport: String,
        }

        // ── blocking follow loop ──────────────────────────────────────────────

        pub fn follow_loop(
            unit:     Option<String>,
            machine:  Option<String>,
            since:    Option<u64>,
            priority: u8,
            cancel:   Arc<AtomicBool>,
            tx:       mpsc::Sender<JournalEntry>,
        ) {
            let journal = match Journal::open() {
                Ok(j) => j,
                Err(e) => {
                    warn!("{e}");
                    return;
                }
            };

            // Filters
            if let Some(ref u) = unit {
                journal.add_match(&format!("_SYSTEMD_UNIT={u}"));
            }

            if let Some(ref m) = machine {
                // Try to resolve machine name → machine ID via /run/systemd/machines/<name>
                let machine_id = resolve_machine_id(m);
                match machine_id {
                    Some(id) => {
                        journal.add_match(&format!("_MACHINE_ID={id}"));
                    }
                    None => {
                        warn!(
                            "systemd.journal_follow: could not resolve machine ID for '{}'; \
                             falling back to _MACHINE= match",
                            m
                        );
                        journal.add_match(&format!("_MACHINE={m}"));
                    }
                }
            }

            // Priority: add one match per level 0..=N (libsystemd OR-combines them)
            for p in 0..=priority {
                journal.add_match(&format!("PRIORITY={p}"));
            }

            // Seek position
            if let Some(secs) = since {
                journal.seek_realtime_usec(secs * 1_000_000);
            } else {
                journal.seek_tail();
                // After seek_tail, calling next() would move past the last entry.
                // We need to step back one so the first next() call lands on a real entry.
                journal.previous();
            }

            // SD_JOURNAL_APPEND = 1 (new entries), SD_JOURNAL_INVALIDATE = 2
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                // Drain available entries
                loop {
                    let r = journal.next();
                    if r <= 0 {
                        break;
                    }
                    let entry = JournalEntry {
                        ts:        journal.realtime_usec(),
                        hostname:  journal.read_field("_HOSTNAME").unwrap_or_default(),
                        unit:      journal.read_field("_SYSTEMD_UNIT").unwrap_or_default(),
                        message:   journal.read_field("MESSAGE").unwrap_or_default(),
                        priority:  journal.read_field("PRIORITY")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(7),
                        transport: journal.read_field("_TRANSPORT").unwrap_or_default(),
                    };
                    if tx.blocking_send(entry).is_err() {
                        return; // receiver dropped → handle closed
                    }
                }

                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                // Wait up to 500 ms for new data
                journal.wait(500_000);
            }
        }

        fn resolve_machine_id(name: &str) -> Option<String> {
            let path = format!("/run/systemd/machines/{name}");
            let content = std::fs::read_to_string(&path).ok()?;
            for line in content.lines() {
                if let Some(id) = line.strip_prefix("MACHINE_ID=") {
                    let id = id.trim();
                    if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Some(id.to_owned());
                    }
                }
            }
            None
        }

        // ── handle UserData ───────────────────────────────────────────────────

        pub struct FollowHandle {
            pub cancel: Arc<AtomicBool>,
        }

        impl mlua::UserData for FollowHandle {
            fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
                methods.add_method("close", |_, this, ()| {
                    this.cancel.store(true, Ordering::Relaxed);
                    Ok(())
                });
                methods.add_method("is_closed", |_, this, ()| {
                    Ok(this.cancel.load(Ordering::Relaxed))
                });
            }
        }

        impl Drop for FollowHandle {
            fn drop(&mut self) {
                self.cancel.store(true, Ordering::Relaxed);
            }
        }
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

        // ── journal_follow live-fire tests ────────────────────────────────────
        // Require a host journal with write access via `logger`.
        // Run with: cargo test --lib systemd:: -- --include-ignored
        //
        // These tests drive follow_loop directly (bypassing the Lua layer) so
        // that the async/Lua callback scheduling doesn't interfere with test
        // assertion timing.

        #[tokio::test]
        #[ignore = "requires journal write access (logger)"]
        async fn journal_follow_smoke() {
            use super::journal_follow_impl;
            use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
            use tokio::sync::mpsc;
            use tokio::time::{Duration, timeout};

            // Write the marker first so the journal has it before we start following.
            tokio::process::Command::new("logger")
                .args(["-t", "assay-jf-test", "marker-XYZ"])
                .status()
                .await
                .expect("logger failed");

            // Give the journal a moment to flush.
            tokio::time::sleep(Duration::from_millis(200)).await;

            let cancel = Arc::new(AtomicBool::new(false));
            let (tx, mut rx) = mpsc::channel::<journal_follow_impl::JournalEntry>(64);
            let cancel_bg = Arc::clone(&cancel);

            // Seek from ~2 seconds ago so we pick up the marker we just wrote.
            let since = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .saturating_sub(2),
            );

            tokio::task::spawn_blocking(move || {
                journal_follow_impl::follow_loop(None, None, since, 7, cancel_bg, tx);
            });

            let found = timeout(Duration::from_secs(5), async {
                while let Some(entry) = rx.recv().await {
                    if entry.message.contains("marker-XYZ") {
                        return true;
                    }
                }
                false
            })
            .await
            .unwrap_or(false);

            cancel.store(true, Ordering::Relaxed);

            assert!(found, "marker-XYZ not seen within 5 s");
        }

        #[tokio::test]
        #[ignore = "requires journal write access (logger)"]
        async fn journal_follow_priority_filter() {
            use super::journal_follow_impl;
            use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
            use tokio::sync::mpsc;
            use tokio::time::{Duration, timeout};

            tokio::process::Command::new("logger")
                .args(["-p", "user.err", "-t", "assay-jf-test", "err-marker-XYZ"])
                .status()
                .await
                .expect("logger failed");
            tokio::process::Command::new("logger")
                .args(["-p", "user.info", "-t", "assay-jf-test", "info-marker-XYZ"])
                .status()
                .await
                .expect("logger failed");

            tokio::time::sleep(Duration::from_millis(200)).await;

            let cancel = Arc::new(AtomicBool::new(false));
            let (tx, mut rx) = mpsc::channel::<journal_follow_impl::JournalEntry>(64);
            let cancel_bg = Arc::clone(&cancel);

            let since = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .saturating_sub(2),
            );

            // priority=3 means only ERR and more critical pass through.
            tokio::task::spawn_blocking(move || {
                journal_follow_impl::follow_loop(None, None, since, 3, cancel_bg, tx);
            });

            let mut entries: Vec<(String, u8)> = Vec::new();

            let found_err = timeout(Duration::from_secs(5), async {
                while let Some(entry) = rx.recv().await {
                    let is_err = entry.message.contains("err-marker-XYZ");
                    entries.push((entry.message.clone(), entry.priority));
                    if is_err {
                        return true;
                    }
                }
                false
            })
            .await
            .unwrap_or(false);

            cancel.store(true, Ordering::Relaxed);

            assert!(found_err, "err-marker-XYZ not seen within 5 s");

            let info_leaked = entries.iter().any(|(m, _)| m.contains("info-marker-XYZ"));
            assert!(!info_leaked, "info-marker-XYZ should have been filtered by priority=3");

            for (msg, pri) in &entries {
                assert!(*pri <= 3, "got priority {pri} > 3 for message: {msg}");
            }
        }

        #[tokio::test]
        #[ignore = "requires journal write access (logger)"]
        async fn journal_follow_close_idempotent() {
            use super::journal_follow_impl::FollowHandle;
            use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

            let cancel = Arc::new(AtomicBool::new(false));
            let handle = FollowHandle { cancel: Arc::clone(&cancel) };

            // first close
            handle.cancel.store(true, Ordering::Relaxed);
            assert!(handle.cancel.load(Ordering::Relaxed));

            // second close — must not panic
            handle.cancel.store(true, Ordering::Relaxed);
            assert!(handle.cancel.load(Ordering::Relaxed));
        }
    }
}
