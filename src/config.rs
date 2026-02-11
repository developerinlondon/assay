use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    pub timeout: Duration,
    pub retries: u32,
    pub backoff: Duration,
    pub parallel: bool,
    pub checks: Vec<CheckConfig>,
}

#[derive(Debug, Clone)]
pub struct CheckConfig {
    pub name: String,
    pub check_type: CheckType,
    pub url: Option<String>,
    pub expect: Option<ExpectConfig>,
    pub query: Option<String>,
    pub file: Option<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckType {
    Http,
    Prometheus,
    Script,
}

#[derive(Debug, Clone, Default)]
pub struct ExpectConfig {
    pub status: Option<u16>,
    pub json: Option<String>,
    pub body: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Deserialize)]
struct RawConfig {
    #[serde(default = "default_timeout")]
    timeout: String,
    #[serde(default = "default_retries")]
    retries: u32,
    #[serde(default = "default_backoff")]
    backoff: String,
    #[serde(default)]
    parallel: bool,
    checks: Vec<RawCheck>,
}

fn default_timeout() -> String {
    "120s".to_string()
}

fn default_retries() -> u32 {
    3
}

fn default_backoff() -> String {
    "5s".to_string()
}

#[derive(Deserialize)]
struct RawCheck {
    name: String,
    #[serde(rename = "type")]
    check_type: String,
    url: Option<String>,
    expect: Option<RawExpect>,
    query: Option<String>,
    file: Option<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

#[derive(Deserialize)]
struct RawExpect {
    status: Option<u16>,
    json: Option<String>,
    body: Option<String>,
    min: Option<f64>,
    max: Option<f64>,
}

pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    if let Some(ms) = s.strip_suffix("ms") {
        let val: u64 = ms.parse().context("invalid milliseconds value")?;
        return Ok(Duration::from_millis(val));
    }
    if let Some(secs) = s.strip_suffix('s') {
        let val: u64 = secs.parse().context("invalid seconds value")?;
        return Ok(Duration::from_secs(val));
    }
    if let Some(mins) = s.strip_suffix('m') {
        let val: u64 = mins.parse().context("invalid minutes value")?;
        return Ok(Duration::from_secs(val * 60));
    }
    bail!("unsupported duration format: {s:?} (use e.g. '120s', '5m', '500ms')")
}

pub fn load(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    parse(&content)
}

pub fn parse(yaml: &str) -> Result<Config> {
    let raw: RawConfig = serde_yml::from_str(yaml).context("parsing YAML config")?;

    let timeout = parse_duration(&raw.timeout).context("parsing timeout")?;
    let backoff = parse_duration(&raw.backoff).context("parsing backoff")?;

    let checks = raw
        .checks
        .into_iter()
        .map(|c| {
            let check_type = match c.check_type.as_str() {
                "http" => CheckType::Http,
                "prometheus" => CheckType::Prometheus,
                "script" => CheckType::Script,
                other => bail!("unknown check type: {other:?}"),
            };
            Ok(CheckConfig {
                name: c.name,
                check_type,
                url: c.url,
                expect: c.expect.map(|e| ExpectConfig {
                    status: e.status,
                    json: e.json,
                    body: e.body,
                    min: e.min,
                    max: e.max,
                }),
                query: c.query,
                file: c.file,
                env: c.env,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Config {
        timeout,
        retries: raw.retries,
        backoff,
        parallel: raw.parallel,
        checks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("120s").unwrap(), Duration::from_secs(120));
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn test_parse_duration_millis() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
    }

    #[test]
    fn test_parse_config() {
        let yaml = r#"
timeout: 30s
retries: 2
backoff: 3s
parallel: false
checks:
  - name: health
    type: http
    url: http://localhost/health
    expect:
      status: 200
  - name: custom
    type: script
    file: /checks/verify.lua
    env:
      FOO: bar
"#;
        let config = parse(yaml).unwrap();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.retries, 2);
        assert_eq!(config.checks.len(), 2);
        assert_eq!(config.checks[0].check_type, CheckType::Http);
        assert_eq!(config.checks[1].check_type, CheckType::Script);
    }
}
