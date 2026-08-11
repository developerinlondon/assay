//! Wraps the HTTP builtins with policy guards after registration, the same
//! shape `readonly` and `approval` use. Enforcement lives here rather than
//! inside the builtins so one place decides, and the transport code stays
//! unaware of who is allowed to call it.

use std::sync::Arc;

use mlua::{Lua, MultiValue, Table, Value};

use super::{
    active, credential, guard_http, is_redacted_header, redact_json_text, redact_keys,
    response_limit,
};

const VERBS: &[&str] = &["get", "post", "put", "patch", "delete"];

/// Pulls the method, the URL, and which argument the URL came from — the one
/// argument credential substitution must leave alone.
type Target = Arc<dyn Fn(&MultiValue) -> Option<(String, String, usize)>>;

pub fn apply(lua: &Lua) -> mlua::Result<()> {
    let Some(http) = lua.globals().get::<Option<Table>>("http")? else {
        return Ok(());
    };
    for verb in VERBS {
        wrap(lua, &http, verb, verb_target(verb))?;
    }
    // Client wrappers route every verb through `_client_request`, so guarding
    // the top-level verbs alone would leave that path open.
    wrap(lua, &http, "_client_request", client_request_target())?;
    wrap(lua, &http, "download", verb_target("get"))?;
    Ok(())
}

fn verb_target(verb: &'static str) -> Target {
    Arc::new(move |args| arg_string(args, 0).map(|url| (verb.to_string(), url, 0)))
}

fn client_request_target() -> Target {
    Arc::new(|args| Some((arg_string(args, 1)?, arg_string(args, 2)?, 2)))
}

fn arg_string(args: &MultiValue, at: usize) -> Option<String> {
    match args.iter().nth(at) {
        Some(Value::String(s)) => s.to_str().ok().map(|s| s.to_string()),
        _ => None,
    }
}

fn wrap(lua: &Lua, http: &Table, name: &str, target: Target) -> mlua::Result<()> {
    let Value::Function(inner) = http.get::<Value>(name)? else {
        return Ok(());
    };
    let wrapper = lua.create_async_function(move |lua, args: MultiValue| {
        let inner = inner.clone();
        let target = Arc::clone(&target);
        async move {
            let mut args = args;
            if let Some((method, url, url_index)) = target(&args) {
                credential::reject_in_url(&url)?;
                guard_http(&lua, &method, &url)?;
                args = fill_credentials(&lua, args, url_index)?;
            }
            let result = inner.call_async::<Value>(args).await?;
            sanitize(&lua, result)
        }
    })?;
    http.set(name, wrapper)
}

/// Swap credential placeholders for real values, everywhere except the URL.
/// This runs after the target check, so a secret is only ever materialised
/// for a request the policy has already allowed.
fn fill_credentials(lua: &Lua, args: MultiValue, url_index: usize) -> mlua::Result<MultiValue> {
    let Some(policy) = active(lua) else {
        return Ok(args);
    };
    if policy.credentials.is_empty() {
        return Ok(args);
    }
    let mut out = Vec::with_capacity(args.len());
    for (i, value) in args.into_iter().enumerate() {
        out.push(if i == url_index {
            value
        } else {
            credential::substitute(lua, &policy, value)?
        });
    }
    Ok(MultiValue::from_iter(out))
}

/// Enforce the size cap and strip declared keys before the response reaches
/// the script. The transport has already buffered the body, so the cap is a
/// disclosure control rather than a memory bound.
fn sanitize(lua: &Lua, result: Value) -> mlua::Result<Value> {
    let Value::Table(table) = &result else {
        return Ok(result);
    };
    if let Some(limit) = response_limit(lua)
        && let Ok(body) = table.get::<mlua::String>("body")
        && body.as_bytes().len() > limit
    {
        return Err(mlua::Error::runtime(format!(
            "policy: response body exceeds max_response_bytes ({limit})"
        )));
    }

    let keys = redact_keys(lua);
    if keys.is_empty() {
        return Ok(result);
    }
    redact_body(lua, table, &keys)?;
    redact_headers(table, &keys)?;
    Ok(result)
}

fn redact_body(lua: &Lua, table: &Table, keys: &[String]) -> mlua::Result<()> {
    let Ok(body) = table.get::<mlua::String>("body") else {
        return Ok(());
    };
    let bytes = body.as_bytes();
    let Some(redacted) = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| redact_json_text(text, keys))
    else {
        return Ok(());
    };
    table.set("body", lua.create_string(redacted.as_bytes())?)
}

fn redact_headers(table: &Table, keys: &[String]) -> mlua::Result<()> {
    let Ok(headers) = table.get::<Table>("headers") else {
        return Ok(());
    };
    let names: Vec<String> = headers
        .clone()
        .pairs::<String, Value>()
        .filter_map(|pair| pair.ok().map(|(name, _)| name))
        .filter(|name| is_redacted_header(name, keys))
        .collect();
    for name in names {
        headers.set(name, "[redacted]")?;
    }
    Ok(())
}
