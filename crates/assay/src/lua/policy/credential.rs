//! Credential handles the VM cannot read.
//!
//! `credential.get(name)` hands back opaque placeholders, not secrets. The
//! real values are substituted into the outgoing request by the HTTP wrapper,
//! after the policy has already decided the target is allowed — so a script
//! can compose a request that authenticates without ever holding the secret.

use mlua::{Lua, Value};

use super::Policy;

const MARK: char = '\u{1}';
const TAG: &str = "assay-cred";

pub fn placeholder(name: &str, field: &str) -> String {
    format!("{MARK}{TAG}{MARK}{name}{MARK}{field}{MARK}")
}

pub fn contains_placeholder(text: &str) -> bool {
    text.contains(MARK) && text.contains(TAG)
}

/// Register the global `credential` table. Only names the policy declares
/// resolve; anything else is an error rather than a silent empty handle.
pub fn register(lua: &Lua, policy: &Policy) -> mlua::Result<()> {
    let declared: Vec<(String, Vec<String>)> = policy
        .credentials
        .iter()
        .map(|(name, fields)| (name.clone(), fields.keys().cloned().collect()))
        .collect();

    let get = lua.create_function(move |lua, name: String| {
        let Some((_, fields)) = declared.iter().find(|(n, _)| *n == name) else {
            return Err(mlua::Error::runtime(format!(
                "credential: '{name}' is not declared in the policy"
            )));
        };
        let handle = lua.create_table()?;
        for field in fields {
            handle.set(field.as_str(), placeholder(&name, field))?;
        }
        Ok(handle)
    })?;

    let table = lua.create_table()?;
    table.set("get", get)?;
    lua.globals().set("credential", table)
}

/// Replace placeholders with the real values a moment before the request
/// leaves. Walks nested tables so a module that builds a JSON body out of
/// its options table is covered without changing that module.
pub fn substitute(lua: &Lua, policy: &Policy, value: Value) -> mlua::Result<Value> {
    match value {
        Value::String(s) => {
            let text = s.to_str()?.to_string();
            if !contains_placeholder(&text) {
                return Ok(Value::String(s));
            }
            Ok(Value::String(lua.create_string(expand(policy, &text))?))
        }
        Value::Table(t) => {
            let out = lua.create_table()?;
            for pair in t.pairs::<Value, Value>() {
                let (k, v) = pair?;
                out.set(k, substitute(lua, policy, v)?)?;
            }
            Ok(Value::Table(out))
        }
        other => Ok(other),
    }
}

fn expand(policy: &Policy, text: &str) -> String {
    let mut out = text.to_string();
    for (name, fields) in &policy.credentials {
        for (field, env_key) in fields {
            let token = placeholder(name, field);
            if !out.contains(&token) {
                continue;
            }
            let resolved = std::env::var(env_key).unwrap_or_default();
            out = out.replace(&token, &resolved);
        }
    }
    out
}

/// A placeholder in a URL would put the secret in a request line, and from
/// there into every access log on the path. Refuse instead of substituting.
pub fn reject_in_url(url: &str) -> mlua::Result<()> {
    if contains_placeholder(url) {
        return Err(mlua::Error::runtime(
            "credential: a credential handle cannot be used in a URL",
        ));
    }
    Ok(())
}
