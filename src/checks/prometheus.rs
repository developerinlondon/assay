use crate::config::CheckConfig;
use crate::output::CheckResult;
use anyhow::{Context, Result};

pub struct PrometheusCheck;

impl PrometheusCheck {
    pub async fn execute(
        &self,
        config: &CheckConfig,
        client: &reqwest::Client,
    ) -> Result<CheckResult> {
        let base_url = config
            .url
            .as_deref()
            .context("prometheus check requires a 'url' field")?;
        let promql = config
            .query
            .as_deref()
            .context("prometheus check requires a 'query' field")?;

        let query_url = format!("{}/api/v1/query", base_url.trim_end_matches('/'));
        let resp = client
            .get(&query_url)
            .query(&[("query", promql)])
            .send()
            .await
            .with_context(|| format!("Prometheus query to {query_url}"))?;

        let body = resp
            .text()
            .await
            .context("reading Prometheus response body")?;

        let parsed: serde_json::Value =
            serde_json::from_str(&body).context("Prometheus response is not valid JSON")?;

        // Extract the numeric value from Prometheus instant query response
        let value = extract_prometheus_value(&parsed)?;

        let expect = match &config.expect {
            Some(e) => e,
            None => {
                // No expectations: pass if we got a value
                return Ok(CheckResult {
                    name: config.name.clone(),
                    passed: true,
                    duration_ms: 0,
                    message: Some(format!("query returned: {value}")),
                });
            }
        };

        if let Some(min) = expect.min
            && value < min
        {
            return Ok(CheckResult {
                name: config.name.clone(),
                passed: false,
                duration_ms: 0,
                message: Some(format!("expected min {min}, got {value}")),
            });
        }

        if let Some(max) = expect.max
            && value > max
        {
            return Ok(CheckResult {
                name: config.name.clone(),
                passed: false,
                duration_ms: 0,
                message: Some(format!("expected max {max}, got {value}")),
            });
        }

        Ok(CheckResult {
            name: config.name.clone(),
            passed: true,
            duration_ms: 0,
            message: None,
        })
    }
}

fn extract_prometheus_value(response: &serde_json::Value) -> Result<f64> {
    let status = response
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown");

    if status != "success" {
        let error_msg = response
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("Prometheus query failed: {error_msg}");
    }

    let results = response
        .get("data")
        .and_then(|d| d.get("result"))
        .and_then(|r| r.as_array())
        .context("unexpected Prometheus response format: missing data.result")?;

    if results.is_empty() {
        anyhow::bail!("Prometheus query returned no results");
    }

    let value_str = results[0]
        .get("value")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.get(1))
        .and_then(|v| v.as_str())
        .context("unexpected Prometheus result format: missing value")?;

    value_str
        .parse::<f64>()
        .with_context(|| format!("failed to parse Prometheus value: {value_str:?}"))
}
