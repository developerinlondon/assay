//! Read-only mode guards, applied after `register_all` when the VM is
//! created with `readonly = true`. Mutating builtins stay registered
//! but are replaced with stubs that raise
//! `readonly: <name> blocked (write operations are disabled in read-only mode)`,
//! so semi-trusted scripts fail with a clear error instead of a
//! nil-index. Read paths (`http.get`, `fs.read`, `env.get`, `db.query`,
//! `systemd.list_units`, …) are untouched.

use mlua::{Lua, MultiValue, Table, Value};

/// Tables whose entire function surface is blocked.
const BLOCKED_TABLES: &[&str] = &["shell", "process", "machinectl"];

/// Individual functions blocked inside otherwise-usable tables.
const BLOCKED_FUNCTIONS: &[&str] = &[
    "http.post",
    "http.put",
    "http.patch",
    "http.delete",
    "http.serve",
    "http.serve_with_extra",
    "http.download",
    "ws.connect",
    "fs.write",
    "fs.write_bytes",
    "fs.remove",
    "fs.rename",
    "fs.copy",
    "fs.chmod",
    "fs.mkdir",
    "fs.tempdir",
    "fs.sub_in_file",
    "env.set",
    "db.execute",
    "oci.copy",
    "oci.tag",
    "oci.mutate",
    "systemd.start",
    "systemd.stop",
    "systemd.restart",
    "systemd.reload",
    "systemd.unit_action",
    "systemd.machine_start",
    "systemd.machine_poweroff",
    "systemd.machine_reboot",
    "systemd.machine_terminate",
    "systemd.machine_exec",
    "apt.update",
    "apt.install",
    "apt.remove",
    "apt.add_source",
    "compress.untar",
    "tar.create",
    "tar.extract",
    "io.popen",
];

pub fn apply(lua: &Lua) -> mlua::Result<()> {
    for path in BLOCKED_FUNCTIONS {
        block_function(lua, path)?;
    }
    for name in BLOCKED_TABLES {
        block_table(lua, name)?;
    }
    guard_http_client_request(lua)?;
    guard_io_open(lua)?;
    guard_io_output(lua)?;
    Ok(())
}

fn blocked(name: &str) -> mlua::Error {
    mlua::Error::runtime(format!(
        "readonly: {name} blocked (write operations are disabled in read-only mode)"
    ))
}

fn blocked_stub(lua: &Lua, name: &str) -> mlua::Result<mlua::Function> {
    let name = name.to_string();
    lua.create_function(move |_, _args: MultiValue| -> mlua::Result<Value> { Err(blocked(&name)) })
}

/// Replace a single `table.fn` global with a blocking stub. Missing
/// tables or functions (feature-gated builds) are skipped so the guard
/// never changes the surface shape of the VM.
fn block_function(lua: &Lua, path: &str) -> mlua::Result<()> {
    let Some((table_name, fn_name)) = path.split_once('.') else {
        return Ok(());
    };
    let Some(table) = lua.globals().get::<Option<Table>>(table_name)? else {
        return Ok(());
    };
    if table.get::<Value>(fn_name)?.is_function() {
        table.set(fn_name, blocked_stub(lua, path)?)?;
    }
    Ok(())
}

/// Replace every function in a global table with a blocking stub.
fn block_table(lua: &Lua, name: &str) -> mlua::Result<()> {
    let Some(table) = lua.globals().get::<Option<Table>>(name)? else {
        return Ok(());
    };
    for pair in table.clone().pairs::<Value, Value>() {
        let (key, value) = pair?;
        if let (Value::String(key_str), true) = (&key, value.is_function()) {
            let label = format!("{name}.{}", key_str.to_str()?);
            table.set(key, blocked_stub(lua, &label)?)?;
        }
    }
    Ok(())
}

/// `http.client(...)` wrappers route every verb through
/// `http._client_request(ud, method, ...)`; blocking only the
/// top-level `http.post` would leave that path open. The guard allows
/// `get` through and raises for every other verb.
fn guard_http_client_request(lua: &Lua) -> mlua::Result<()> {
    let Some(http) = lua.globals().get::<Option<Table>>("http")? else {
        return Ok(());
    };
    let Some(inner) = http.get::<Option<mlua::Function>>("_client_request")? else {
        return Ok(());
    };
    let wrapper = lua.create_async_function(move |_, args: MultiValue| {
        let inner = inner.clone();
        async move {
            let method = match args.iter().nth(1) {
                Some(Value::String(s)) => Some(s.to_str()?.to_string()),
                _ => None,
            };
            if let Some(method) = method
                && method != "get"
            {
                return Err(blocked(&format!("http.{method}")));
            }
            inner.call_async::<MultiValue>(args).await
        }
    })?;
    http.set("_client_request", wrapper)?;
    Ok(())
}

/// `io.open` stays available for reading; write and append modes raise.
fn guard_io_open(lua: &Lua) -> mlua::Result<()> {
    let Some(io_table) = lua.globals().get::<Option<Table>>("io")? else {
        return Ok(());
    };
    let Some(inner) = io_table.get::<Option<mlua::Function>>("open")? else {
        return Ok(());
    };
    let wrapper = lua.create_function(move |_, args: MultiValue| {
        let mode = match args.iter().nth(1) {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => "r".to_string(),
        };
        if mode.contains('w') || mode.contains('a') || mode.contains('+') {
            return Err(blocked("io.open"));
        }
        inner.call::<MultiValue>(args)
    })?;
    io_table.set("open", wrapper)?;
    Ok(())
}

/// `io.output(target)` opens its target for writing; only the
/// zero-argument read of the current output stays available.
fn guard_io_output(lua: &Lua) -> mlua::Result<()> {
    let Some(io_table) = lua.globals().get::<Option<Table>>("io")? else {
        return Ok(());
    };
    let Some(inner) = io_table.get::<Option<mlua::Function>>("output")? else {
        return Ok(());
    };
    let wrapper = lua.create_function(move |_, args: MultiValue| {
        if !args.is_empty() {
            return Err(blocked("io.output"));
        }
        inner.call::<MultiValue>(args)
    })?;
    io_table.set("output", wrapper)?;
    Ok(())
}
