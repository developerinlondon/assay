use crate::config::CheckConfig;
use crate::output::CheckResult;
use anyhow::{Context, Result, bail};

pub struct HttpCheck;

impl HttpCheck {
    pub async fn execute(&self, config: &CheckConfig, client: &reqwest::Client) -> Result<CheckResult> {
        let url = config
            .url
            .as_deref()
            .context("http check requires a 'url' field")?;

        let resp = client
            .get(url)
            .send()
            .await
            .with_context(|| format!("HTTP GET {url}"))?;

        let status = resp.status().as_u16();
        let body = resp
            .text()
            .await
            .with_context(|| format!("reading response body from {url}"))?;

        let expect = match &config.expect {
            Some(e) => e,
            None => {
                let passed = (200..300).contains(&status);
                return Ok(CheckResult {
                    name: config.name.clone(),
                    passed,
                    duration_ms: 0,
                    message: if passed {
                        None
                    } else {
                        Some(format!("expected 2xx, got {status}"))
                    },
                });
            }
        };

        if let Some(expected_status) = expect.status
            && status != expected_status
        {
            return Ok(CheckResult {
                name: config.name.clone(),
                passed: false,
                duration_ms: 0,
                message: Some(format!("expected status {expected_status}, got {status}")),
            });
        }

        if let Some(ref expected_body) = expect.body
            && !body.contains(expected_body)
        {
            return Ok(CheckResult {
                name: config.name.clone(),
                passed: false,
                duration_ms: 0,
                message: Some(format!("body does not contain {expected_body:?}")),
            });
        }

        if let Some(ref json_expr) = expect.json {
            let passed = evaluate_json_expression(&body, json_expr)?;
            if !passed {
                return Ok(CheckResult {
                    name: config.name.clone(),
                    passed: false,
                    duration_ms: 0,
                    message: Some(format!("JSON expression failed: {json_expr}")),
                });
            }
        }

        Ok(CheckResult {
            name: config.name.clone(),
            passed: true,
            duration_ms: 0,
            message: None,
        })
    }
}

fn evaluate_json_expression(body: &str, expr: &str) -> Result<bool> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).context("response body is not valid JSON")?;

    let expr = expr.trim();
    let parts: Vec<&str> = expr.splitn(2, "==").collect();
    if parts.len() != 2 {
        bail!("unsupported JSON expression syntax: {expr:?} (expected '.path == value')");
    }

    let path = parts[0].trim();
    let expected_str = parts[1].trim();

    let actual = navigate_json_path(&parsed, path)?;

    let expected: serde_json::Value = if expected_str.starts_with('"') && expected_str.ends_with('"') {
        serde_json::Value::String(expected_str[1..expected_str.len() - 1].to_string())
    } else if expected_str == "true" {
        serde_json::Value::Bool(true)
    } else if expected_str == "false" {
        serde_json::Value::Bool(false)
    } else if expected_str == "null" {
        serde_json::Value::Null
    } else if let Ok(n) = expected_str.parse::<i64>() {
        serde_json::Value::Number(n.into())
    } else if let Ok(n) = expected_str.parse::<f64>() {
        serde_json::json!(n)
    } else {
        serde_json::Value::String(expected_str.to_string())
    };

    Ok(actual == &expected)
}

fn navigate_json_path<'a>(value: &'a serde_json::Value, path: &str) -> Result<&'a serde_json::Value> {
    let path = path.trim_start_matches('.');
    let mut current = value;
    for segment in path.split('.') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        current = current
            .get(segment)
            .with_context(|| format!("JSON path '.{segment}' not found in response"))?;
    }
    Ok(current)
}
