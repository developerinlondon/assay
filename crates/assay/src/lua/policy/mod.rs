//! Capability policy: what a script may require, read from the environment,
//! and send over HTTP, enforced inside the builtins.
//!
//! Orthogonal to `ExecMode` — a policy narrows what is reachable, the mode
//! decides whether a mutating operation runs, suspends, or is refused. With
//! no policy loaded every check passes and behaviour is unchanged.

pub mod apply;
pub mod credential;
mod glob;
mod redact;
mod schema;

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use mlua::Lua;

pub use redact::{is_redacted_header, redact_json_text};
pub use schema::Classify;
use schema::{HttpRule, PolicyFile};

/// Path to a policy file applied to every VM this process creates. Follows
/// the same env-driven pattern as the other sandbox knobs.
pub const POLICY_FILE_ENV: &str = "ASSAY_POLICY_FILE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    Read,
    Write,
}

#[derive(Debug, Default)]
pub struct Policy {
    module_allow: Option<HashSet<String>>,
    env_allow: Option<HashSet<String>>,
    http_rules: Option<Vec<HttpRule>>,
    max_response_bytes: Option<usize>,
    redact: Vec<String>,
    pub(crate) credentials: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Clone)]
pub struct PolicyHandle(pub Arc<Policy>);

/// The active policy for this VM, or `None` when the process runs unpoliced.
pub fn active(lua: &Lua) -> Option<Arc<Policy>> {
    lua.app_data_ref::<PolicyHandle>()
        .map(|handle| Arc::clone(&handle.0))
}

pub fn install(lua: &Lua, policy: Arc<Policy>) {
    lua.set_app_data(PolicyHandle(policy));
}

pub fn env_visible(lua: &Lua, key: &str) -> bool {
    active(lua).is_none_or(|p| p.env_visible(key))
}

pub fn guard_require(lua: &Lua, module: &str) -> mlua::Result<()> {
    match active(lua) {
        Some(p) if !p.module_allowed(module) => Err(mlua::Error::runtime(format!(
            "policy: module '{module}' is not in the allowed set"
        ))),
        _ => Ok(()),
    }
}

pub fn guard_http(lua: &Lua, method: &str, url: &str) -> mlua::Result<()> {
    match active(lua) {
        Some(p) => p
            .check_http(method, url)
            .map(|_| ())
            .map_err(mlua::Error::runtime),
        None => Ok(()),
    }
}

/// Whether the policy treats this request as a read. Drives the gates, so a
/// declared authentication POST can proceed under read-only mode.
pub fn is_read(lua: &Lua, method: &str, url: &str) -> bool {
    active(lua)
        .and_then(|p| p.check_http(method, url).ok())
        .is_some_and(|c| c == Classification::Read)
}

pub fn response_limit(lua: &Lua) -> Option<usize> {
    active(lua).and_then(|p| p.max_response_bytes())
}

pub fn redact_keys(lua: &Lua) -> Vec<String> {
    active(lua)
        .map(|p| p.redact_keys().to_vec())
        .unwrap_or_default()
}

pub fn from_env() -> Result<Option<Arc<Policy>>, String> {
    let Some(path) = std::env::var(POLICY_FILE_ENV)
        .ok()
        .filter(|p| !p.is_empty())
    else {
        return Ok(None);
    };
    Ok(Some(Arc::new(Policy::load(&path)?)))
}

impl Policy {
    pub fn load(path: &str) -> Result<Self, String> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("policy: cannot read {path}: {e}"))?;
        Self::parse(&source)
    }

    pub fn parse(source: &str) -> Result<Self, String> {
        let file = PolicyFile::parse(source)?;
        let http = file.http;
        Ok(Policy {
            module_allow: file
                .modules
                .and_then(|m| m.allow)
                .map(|list| list.into_iter().collect()),
            env_allow: file
                .env
                .map(|e| e.allow.into_iter().collect::<HashSet<String>>()),
            max_response_bytes: http.as_ref().and_then(|h| h.max_response_bytes),
            redact: http.as_ref().map(|h| h.redact.clone()).unwrap_or_default(),
            http_rules: http.and_then(|h| h.rules),
            credentials: file.credentials,
        })
    }

    pub fn module_allowed(&self, module: &str) -> bool {
        match &self.module_allow {
            Some(allow) => allow.contains(module),
            None => true,
        }
    }

    pub fn env_visible(&self, key: &str) -> bool {
        match &self.env_allow {
            Some(allow) => allow.contains(key),
            None => true,
        }
    }

    pub fn max_response_bytes(&self) -> Option<usize> {
        self.max_response_bytes
    }

    pub fn redact_keys(&self) -> &[String] {
        &self.redact
    }

    pub fn check_http(&self, method: &str, url: &str) -> Result<Classification, String> {
        let Some(rules) = &self.http_rules else {
            return Ok(default_classification(method));
        };
        let parsed = url::Url::parse(url)
            .map_err(|e| format!("policy: cannot parse request URL '{url}': {e}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| format!("policy: request URL '{url}' has no host"))?;
        let path = parsed.path();

        for rule in rules {
            if rule_matches(rule, host, method, path) {
                return Ok(match rule.classify {
                    Some(Classify::Read) => Classification::Read,
                    Some(Classify::Write) => Classification::Write,
                    None => default_classification(method),
                });
            }
        }
        Err(format!(
            "policy: {} {host}{path} is not allowed by any http rule",
            method.to_ascii_uppercase()
        ))
    }
}

fn rule_matches(rule: &HttpRule, host: &str, method: &str, path: &str) -> bool {
    let host_ok = rule.hosts.iter().any(|h| glob::host_matches(h, host));
    let method_ok = rule.methods.is_empty()
        || rule
            .methods
            .iter()
            .any(|m| m.trim().eq_ignore_ascii_case(method));
    let path_ok = rule.paths.is_empty() || rule.paths.iter().any(|p| glob::path_matches(p, path));
    host_ok && method_ok && path_ok
}

fn default_classification(method: &str) -> Classification {
    if method.eq_ignore_ascii_case("get") {
        Classification::Read
    } else {
        Classification::Write
    }
}
