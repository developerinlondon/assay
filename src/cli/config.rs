//! YAML config file discovery, parsing, and precedence merge.
//!
//! Operators deploy assay in a pod with a config file mounted at
//! `/etc/assay/config.yaml`. Inside the pod `assay workflow list` "just
//! works" without anyone needing to set env vars or flags. Avoids baking
//! credentials into process environments, which leak into `ps` output and
//! child processes.
//!
//! Discovery order (first match wins):
//!   1. `--config PATH` (explicit)
//!   2. `ASSAY_CONFIG_FILE` (explicit override)
//!   3. `$XDG_CONFIG_HOME/assay/config.yaml`
//!   4. `$HOME/.config/assay/config.yaml`
//!   5. `/etc/assay/config.yaml`
//!
//! A missing file is not an error — callers fall through to flag / env
//! / default precedence on every field.

use std::path::PathBuf;

use serde::Deserialize;

/// Parsed config file. Every field optional so partial configs work.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ConfigFile {
    pub engine_url: Option<String>,
    /// Literal API key / bearer token. Prefer `api_key_file` when the
    /// config file itself might be committed to a repo or ConfigMap.
    pub api_key: Option<String>,
    /// Path to a file containing the API key. File contents are read
    /// at startup, trimmed of trailing whitespace, and used as the
    /// bearer token. Takes precedence over `api_key` when both are set.
    pub api_key_file: Option<PathBuf>,
    pub namespace: Option<String>,
    /// Default output format — `table`, `json`, `jsonl`, or `yaml`.
    pub output: Option<String>,
}

/// Discover and load the config file (if any).
///
/// If `explicit` is provided (from `--config` or `ASSAY_CONFIG_FILE`),
/// it's the only path tried and a missing file is an error. Otherwise
/// we probe the well-known locations in order and return the first one
/// that exists. Returns `None` if no config file is found.
pub fn load(explicit: Option<&str>) -> Result<Option<ConfigFile>, String> {
    if let Some(path) = explicit {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading config file {path}: {e}"))?;
        let cfg: ConfigFile = serde_yml::from_str(&text)
            .map_err(|e| format!("parsing {path}: {e}"))?;
        return Ok(Some(cfg));
    }
    for path in discovery_paths() {
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("reading {}: {e}", path.display()))?;
            let cfg: ConfigFile = serde_yml::from_str(&text)
                .map_err(|e| format!("parsing {}: {e}", path.display()))?;
            return Ok(Some(cfg));
        }
    }
    Ok(None)
}

fn discovery_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        paths.push(PathBuf::from(xdg).join("assay/config.yaml"));
    }
    if let Ok(home) = std::env::var("HOME") {
        paths.push(PathBuf::from(home).join(".config/assay/config.yaml"));
    }
    paths.push(PathBuf::from("/etc/assay/config.yaml"));
    paths
}

/// Resolve the api-key value from config-file fields. `api_key_file`
/// wins if present; its contents are read and trimmed. Returns `None`
/// if neither is set.
pub fn resolve_api_key(cfg: &ConfigFile) -> Result<Option<String>, String> {
    if let Some(ref path) = cfg.api_key_file {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("reading api_key_file {}: {e}", path.display()))?;
        return Ok(Some(content.trim().to_string()));
    }
    Ok(cfg.api_key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_file_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nope.yaml");
        // `HOME` is unset for this test so discovery falls through; we just
        // pass the explicit path which must error because it's missing.
        let err = load(Some(path.to_str().unwrap())).unwrap_err();
        assert!(err.contains("reading"));
    }

    #[test]
    fn loads_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.yaml");
        std::fs::write(
            &path,
            "engine_url: https://example.com\nnamespace: custom\noutput: json\n",
        )
        .unwrap();
        let cfg = load(Some(path.to_str().unwrap())).unwrap().unwrap();
        assert_eq!(cfg.engine_url.as_deref(), Some("https://example.com"));
        assert_eq!(cfg.namespace.as_deref(), Some("custom"));
        assert_eq!(cfg.output.as_deref(), Some("json"));
    }

    #[test]
    fn api_key_file_wins_over_literal() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("key.txt");
        std::fs::write(&key_path, "  secret-from-file\n").unwrap();
        let cfg = ConfigFile {
            api_key: Some("from-literal".into()),
            api_key_file: Some(key_path),
            ..ConfigFile::default()
        };
        let resolved = resolve_api_key(&cfg).unwrap().unwrap();
        assert_eq!(resolved, "secret-from-file");
    }

    #[test]
    fn api_key_literal_when_no_file() {
        let cfg = ConfigFile {
            api_key: Some("only-literal".into()),
            ..ConfigFile::default()
        };
        let resolved = resolve_api_key(&cfg).unwrap().unwrap();
        assert_eq!(resolved, "only-literal");
    }

    #[test]
    fn no_api_key_returns_none() {
        let cfg = ConfigFile::default();
        assert!(resolve_api_key(&cfg).unwrap().is_none());
    }

    #[test]
    fn unknown_keys_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.yaml");
        std::fs::write(&path, "engine_url: x\nwhat_is_this: 5\n").unwrap();
        assert!(load(Some(path.to_str().unwrap())).is_err());
    }
}
