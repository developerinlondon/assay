//! Shared catalog of assay's mutating builtin surface. Both the read-only
//! gate (`readonly.rs`) and the approval gate (`approval.rs`) consume this
//! one list so the two modes classify the same operations as mutating.
//!
//! HTTP is the one surface where the name of the operation is not enough to
//! tell a read from a write, so it gets a classifier rather than a list: an
//! explicit `classify:` on the matching policy rule first, then the built-in
//! AWS classification in `aws`, then the verb.

mod aws;

use mlua::{Lua, MultiValue, Table, Value};
use sha2::{Digest, Sha256};

use crate::lua::policy::{self, Classification, Policy};

/// Tables whose entire function surface is gated.
pub(crate) const BLOCKED_TABLES: &[&str] = &["shell", "process", "machinectl"];

/// One entry of `package.loaded` / `package.preload`, when it is a table.
pub(crate) fn package_entry(lua: &Lua, registry: &str, name: &str) -> mlua::Result<Option<Table>> {
    let Some(package) = lua.globals().get::<Option<Table>>("package")? else {
        return Ok(None);
    };
    let Some(sub) = package.get::<Option<Table>>(registry)? else {
        return Ok(None);
    };
    sub.get::<Option<Table>>(name)
}

/// Every table a top-level name resolves to. `require` reads
/// `package.loaded` before any searcher, so a guard that walks `_G` alone
/// leaves a second, ungated handle on the same library. For most names the
/// two are one table and this returns one entry; for a Lua standard library
/// that assay replaced on `_G` — `os` is the live case — they are different
/// tables and only the `package.loaded` one carries the real functions.
pub(crate) fn tables_for(lua: &Lua, name: &str) -> mlua::Result<Vec<Table>> {
    let mut out: Vec<Table> = Vec::new();
    if let Some(global) = lua.globals().get::<Option<Table>>(name)? {
        out.push(global);
    }
    if let Some(loaded) = package_entry(lua, "loaded", name)?
        && !out.iter().any(|seen| seen == &loaded)
    {
        out.push(loaded);
    }
    Ok(out)
}

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
    // Lua's own mutators. They are absent from the `os` table assay puts on
    // `_G` (hostname, arch, time, …) and present on the real one that
    // `require("os")` returns, so these entries only ever bite there.
    "os.execute",
    "os.remove",
    "os.rename",
    "os.tmpname",
    "os.exit",
];

/// Verbs that are a read on their own. Everything else has to be classified
/// against the target before a mode will admit it.
pub(crate) fn verb_is_read(method: &str) -> bool {
    method.eq_ignore_ascii_case("get")
}

/// Verbs the gates re-wrap individually, because whether they mutate depends
/// on the target rather than the verb alone. `get` is in the list because it
/// can carry a mutation: the AWS Query API takes `?Action=TerminateInstances`
/// on a `GET`.
pub(crate) const HTTP_GATED_VERBS: &[&str] = &["get", "post", "put", "patch", "delete"];

pub(crate) fn is_http_verb_path(path: &str) -> bool {
    HTTP_GATED_VERBS
        .iter()
        .any(|verb| path == format!("http.{verb}"))
}

/// One outbound HTTP call, as much of it as classification needs. Header
/// values are here because the SigV4 signature and the AWS operation target
/// live in them; nothing derived from them is shown to anyone — see
/// `header_names` for what the approval payload gets.
pub(crate) struct HttpCall {
    pub method: String,
    pub url: Option<String>,
    pub headers: Vec<(String, String)>,
    /// Two caller header names equal but for case; only one value is sent.
    pub headers_collide: bool,
    pub body: Option<String>,
}

/// Whether a call is a read, and so runs unhindered in every mode.
pub(crate) fn http_call_is_read(lua: &Lua, call: &HttpCall) -> bool {
    call_is_read(policy::active(lua).as_deref(), call)
}

/// The same answer from the policy alone, for a redirect hop the transport
/// is about to follow and no Lua is at hand.
pub(crate) fn call_is_read(policy: Option<&Policy>, call: &HttpCall) -> bool {
    match decide(policy, call).0 {
        Some(Classification::Read) => true,
        Some(Classification::Write) => false,
        None => verb_is_read(&call.method),
    }
}

/// The classification and, for a write the verb does not explain, why.
///
/// An explicit `classify:` on the matching rule is the operator's word and
/// wins. A matching rule without one only admits the target, so the built-in
/// AWS verdict still stands: a `hosts: [ec2…] methods: [GET]` rule must not
/// reopen `GET ?Action=TerminateInstances`. No matching rule is a denial the
/// policy wrapper raises with its own message, so the gate falls back to the
/// verb rather than shadowing it.
fn decide(policy: Option<&Policy>, call: &HttpCall) -> (Option<Classification>, Option<String>) {
    if host_header_disagrees(call) {
        let reason = "Host header does not name the URL's host".to_string();
        return (Some(Classification::Write), Some(reason));
    }
    if let (Some(policy), Some(url)) = (policy.filter(|p| p.has_http_rules()), &call.url) {
        match policy.rule_classification(&call.method, url) {
            Err(_) => return (None, None),
            Ok(Some(explicit)) => return (Some(explicit), None),
            Ok(None) => {}
        }
    }
    (aws::classify(call), aws::refusal_reason(call))
}

/// A caller-set `Host` is what the server routes on, so a URL naming an IP
/// literal or another name would be classified as somewhere it is not.
fn host_header_disagrees(call: &HttpCall) -> bool {
    let Some((_, given)) = call.headers.iter().find(|(name, _)| name == "host") else {
        return false;
    };
    let url = call.url.as_deref().and_then(|u| url::Url::parse(u).ok());
    let Some(host) = url.as_ref().and_then(|u| u.host_str()) else {
        return true;
    };
    let given = authority_host(given.trim());
    let bare = |h: &str| h.strip_suffix('.').unwrap_or(h).to_ascii_lowercase();
    bare(given) != bare(host)
}

/// `host:port`, `[::1]:port` or a bare host, without the port.
fn authority_host(authority: &str) -> &str {
    if authority.starts_with('[') {
        return authority
            .find(']')
            .map_or(authority, |end| &authority[..=end]);
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    }
}

/// What a gate is told about a call it is about to refuse or suspend. Owned
/// rather than borrowed: this is built once per refused call, which is a
/// terminal path, and it keeps the callback bound a plain `Fn(&GatedCall)`.
pub(crate) struct GatedCall {
    pub op: String,
    pub url: String,
    pub digest: String,
    pub header_names: Vec<String>,
    /// Why the classifier called this a write, when it knows something more
    /// specific than the verb — `aws ec2 Action=TerminateInstances is not a
    /// read`. Vetted for disclosure before it gets here.
    pub reason: Option<String>,
}

/// Why this call is not a read, in terms safe to put in an error message.
/// `None` when the verb is the whole story, or a policy rule decided.
pub(crate) fn write_reason(lua: &Lua, call: &HttpCall) -> Option<String> {
    call_write_reason(policy::active(lua).as_deref(), call)
}

pub(crate) fn call_write_reason(policy: Option<&Policy>, call: &HttpCall) -> Option<String> {
    decide(policy, call).1
}

/// The gate a VM runs under, which the transport reads to judge redirects.
#[derive(Clone, Copy)]
pub(crate) struct GateMode(pub &'static str);

pub(crate) fn mark_gated(lua: &Lua, mode: &'static str) {
    lua.set_app_data(GateMode(mode));
}

/// Wrap `http.get|post|put|patch|delete` so each mode decides per target. The
/// callback runs only for calls that are not reads; returning `Ok` admits
/// the operation, an error refuses or suspends it.
pub(crate) fn wrap_http_verbs<F>(lua: &Lua, on_write: F) -> mlua::Result<()>
where
    F: Fn(&GatedCall) -> mlua::Result<()> + Clone + 'static,
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
                let call = http_call_from_args(verb, &args, 0)?;
                if !http_call_is_read(&lua, &call) {
                    let op = format!("http.{verb}");
                    on_write(&GatedCall {
                        digest: operation_digest(&op, &args),
                        url: call.url.clone().unwrap_or_default(),
                        header_names: header_names(&args),
                        reason: write_reason(&lua, &call),
                        op,
                    })?;
                }
                inner.call_async::<MultiValue>(args).await
            }
        })?;
        http.set(*verb, wrapper)?;
    }
    Ok(())
}

/// Read one HTTP call out of the Lua arguments, mirroring how
/// `builtins::http::parse_request_args` reads the same shapes. `skip` is 0 for
/// the `http.<verb>` globals and 2 for `http._client_request(ud, method, …)`.
///
/// A table body is auto-JSON and cannot carry a form-encoded `Action`, so it
/// is read past rather than stringified.
pub(crate) fn http_call_from_args(
    method: &str,
    args: &MultiValue,
    skip: usize,
) -> mlua::Result<HttpCall> {
    let mut rest = args.iter().skip(skip);
    let url = match rest.next() {
        Some(Value::String(s)) => Some(s.to_str()?.to_string()),
        _ => None,
    };

    let has_body_slot = !matches!(method, "get" | "delete");
    let mut body = None;
    if has_body_slot && let Some(Value::String(s)) = rest.next() {
        body = Some(s.to_str()?.to_string());
    }
    let opts = match rest.next() {
        Some(Value::Table(t)) => Some(t.clone()),
        _ => None,
    };

    let mut headers = Vec::new();
    let mut headers_collide = false;
    if let Some(opts) = opts {
        // `get`/`delete` have no body slot in their shorthand, so a body for
        // those arrives via `opts.body`.
        if !has_body_slot && let Ok(Value::String(s)) = opts.get::<Value>("body") {
            body = Some(s.to_str()?.to_string());
        }
        if let Ok(Value::Table(table)) = opts.get::<Value>("headers") {
            headers = super::http::caller_header_pairs(method, &table)?;
            headers_collide = table.pairs::<Value, Value>().count() > headers.len();
        }
    }

    Ok(HttpCall {
        method: method.to_string(),
        url,
        headers,
        headers_collide,
        body,
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
