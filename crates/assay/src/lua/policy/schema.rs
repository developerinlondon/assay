use std::collections::BTreeMap;

use serde::Deserialize;

pub const SUPPORTED_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyFile {
    pub version: u32,
    #[serde(default)]
    pub modules: Option<ModuleSection>,
    #[serde(default)]
    pub env: Option<EnvSection>,
    #[serde(default)]
    pub http: Option<HttpSection>,
    #[serde(default)]
    pub credentials: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleSection {
    #[serde(default)]
    pub allow: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvSection {
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpSection {
    #[serde(default)]
    pub max_response_bytes: Option<usize>,
    #[serde(default)]
    pub redact: Vec<String>,
    #[serde(default)]
    pub rules: Option<Vec<HttpRule>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpRule {
    pub hosts: Vec<String>,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub classify: Option<Classify>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Classify {
    Read,
    Write,
}

impl PolicyFile {
    pub fn parse(source: &str) -> Result<Self, String> {
        let file: PolicyFile =
            serde_yml::from_str(source).map_err(|e| format!("policy: invalid YAML: {e}"))?;
        if file.version != SUPPORTED_VERSION {
            return Err(format!(
                "policy: unsupported version {} (this build understands {SUPPORTED_VERSION})",
                file.version
            ));
        }
        file.validate()?;
        Ok(file)
    }

    fn validate(&self) -> Result<(), String> {
        let Some(http) = &self.http else {
            return Ok(());
        };
        for (i, rule) in http.rules.iter().flatten().enumerate() {
            if rule.hosts.is_empty() {
                return Err(format!("policy: http.rules[{i}] needs at least one host"));
            }
            for method in &rule.methods {
                if !is_known_method(method) {
                    return Err(format!(
                        "policy: http.rules[{i}] has unknown method '{method}'"
                    ));
                }
            }
        }
        Ok(())
    }
}

fn is_known_method(method: &str) -> bool {
    matches!(
        method.trim().to_ascii_uppercase().as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    )
}
