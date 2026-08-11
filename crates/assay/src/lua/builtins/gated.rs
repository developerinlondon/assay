//! Shared catalog of assay's mutating builtin surface. Both the read-only
//! gate (`readonly.rs`) and the approval gate (`approval.rs`) consume this
//! one list so the two modes classify the same operations as mutating.

use mlua::{Lua, MultiValue, Table, Value};

/// Tables whose entire function surface is gated.
pub(crate) const BLOCKED_TABLES: &[&str] = &["shell", "process", "machinectl"];

/// Individual functions gated inside otherwise-usable tables.
pub(crate) const BLOCKED_FUNCTIONS: &[&str] = &[
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

/// `http.client(...)` wrappers route every verb through
/// `http._client_request(ud, method, ...)`. Only `get` is a read; every
/// other verb is a mutating (gated) request.
pub(crate) fn is_gated_http_verb(method: &str) -> bool {
    method != "get"
}

/// Verbs the gates re-wrap individually, because whether they mutate can
/// depend on the target rather than the verb alone.
pub(crate) const HTTP_GATED_VERBS: &[&str] = &["post", "put", "patch", "delete"];

pub(crate) fn is_http_verb_path(path: &str) -> bool {
    HTTP_GATED_VERBS
        .iter()
        .any(|verb| path == format!("http.{verb}"))
}

/// A call is a read when the verb is inherently one, or when policy declares
/// this exact target a read — the case that lets an authentication POST run
/// under read-only mode.
pub(crate) fn http_call_is_read(lua: &Lua, method: &str, url: Option<&str>) -> bool {
    if !is_gated_http_verb(method) {
        return true;
    }
    url.is_some_and(|u| crate::lua::policy::is_read(lua, method, u))
}

/// Wrap `http.post|put|patch|delete` so each mode decides per target. The
/// callback runs only for calls that are not reads; returning `Ok` admits
/// the operation, an error refuses or suspends it.
pub(crate) fn wrap_http_verbs<F>(lua: &Lua, on_write: F) -> mlua::Result<()>
where
    F: Fn(&str, &str) -> mlua::Result<()> + Clone + 'static,
{
    let Some(http) = lua.globals().get::<Option<Table>>("http")? else {
        return Ok(());
    };
    for verb in HTTP_GATED_VERBS {
        let Value::Function(inner) = http.get::<Value>(*verb)? else {
            continue;
        };
        let on_write = on_write.clone();
        let wrapper = lua.create_async_function(move |lua, args: MultiValue| {
            let inner = inner.clone();
            let on_write = on_write.clone();
            async move {
                let url = first_string_arg(&args);
                if !http_call_is_read(&lua, verb, url.as_deref()) {
                    on_write(&format!("http.{verb}"), url.as_deref().unwrap_or(""))?;
                }
                inner.call_async::<MultiValue>(args).await
            }
        })?;
        http.set(*verb, wrapper)?;
    }
    Ok(())
}

fn first_string_arg(args: &MultiValue) -> Option<String> {
    args.iter().find_map(|value| match value {
        Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
        _ => None,
    })
}
