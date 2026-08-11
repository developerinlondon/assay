//! Shared catalog of assay's mutating builtin surface. Both the read-only
//! gate (`readonly.rs`) and the approval gate (`approval.rs`) consume this
//! one list so the two modes classify the same operations as mutating.

use mlua::{Lua, MultiValue, Table, Value};
use sha2::{Digest, Sha256};

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
    F: Fn(&str, &str, &str, &[String]) -> mlua::Result<()> + Clone + 'static,
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
                    let op = format!("http.{verb}");
                    let digest = operation_digest(&op, &args);
                    on_write(
                        &op,
                        url.as_deref().unwrap_or(""),
                        &digest,
                        &header_names(&args),
                    )?;
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

/// Canonical digest of one gated call and its arguments. A grant bound to
/// the operation name alone approves every call of that name, so a replay
/// whose arguments changed still spends it; this binds a grant to one
/// exact call.
pub(crate) fn operation_digest(op: &str, args: &MultiValue) -> String {
    let mut hasher = Sha256::new();
    hasher.update(op.as_bytes());
    for value in args.iter() {
        hasher.update([0u8]);
        absorb(value, &mut hasher, 0);
    }
    format!("{:x}", hasher.finalize())
}

/// Tables are absorbed with keys sorted, so two structurally equal tables
/// digest identically regardless of Lua's iteration order.
fn absorb(value: &Value, hasher: &mut Sha256, depth: usize) {
    if depth > 16 {
        hasher.update(b"deep");
        return;
    }
    match value {
        Value::Nil => hasher.update(b"nil"),
        Value::Boolean(b) => hasher.update(if *b { &b"true"[..] } else { &b"false"[..] }),
        Value::Integer(i) => hasher.update(i.to_string().as_bytes()),
        Value::Number(n) => hasher.update(n.to_string().as_bytes()),
        Value::String(s) => hasher.update(s.as_bytes()),
        Value::Table(t) => {
            let mut entries: Vec<(String, Value)> = t
                .clone()
                .pairs::<Value, Value>()
                .flatten()
                .map(|(k, v)| (key_repr(&k), v))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            hasher.update(b"{");
            for (key, entry) in entries {
                hasher.update(key.as_bytes());
                hasher.update(b"=");
                absorb(&entry, hasher, depth + 1);
                hasher.update(b";");
            }
            hasher.update(b"}");
        }
        other => hasher.update(other.type_name().as_bytes()),
    }
}

fn key_repr(value: &Value) -> String {
    match value {
        Value::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_default(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Boolean(b) => b.to_string(),
        other => other.type_name().to_string(),
    }
}

/// Header *names* only: the approval payload is shown to a human and
/// persisted in resume state, and a header value is where a bearer token
/// lives. The digest still covers the values.
pub(crate) fn header_names(args: &MultiValue) -> Vec<String> {
    let mut names = Vec::new();
    for value in args.iter() {
        let Value::Table(table) = value else { continue };
        let Ok(Value::Table(headers)) = table.get::<Value>("headers") else {
            continue;
        };
        for (key, _) in headers.pairs::<Value, Value>().flatten() {
            names.push(key_repr(&key));
        }
    }
    names.sort();
    names.dedup();
    names
}
