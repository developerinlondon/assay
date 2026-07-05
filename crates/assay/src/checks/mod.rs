pub mod http;
pub mod prometheus;
pub mod script;

use crate::config::CheckConfig;
use crate::lua::ExecMode;
use crate::output::CheckResult;
use std::time::Instant;

pub async fn run_check(
    config: &CheckConfig,
    client: &reqwest::Client,
    exec_mode: ExecMode,
) -> CheckResult {
    let start = Instant::now();
    let result = match config.check_type {
        crate::config::CheckType::Http => http::HttpCheck.execute(config, client).await,
        crate::config::CheckType::Prometheus => {
            prometheus::PrometheusCheck.execute(config, client).await
        }
        crate::config::CheckType::Script => {
            script::ScriptCheck.execute(config, client, exec_mode).await
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(mut check_result) => {
            check_result.duration_ms = duration_ms;
            check_result
        }
        Err(e) => CheckResult {
            name: config.name.clone(),
            passed: false,
            duration_ms,
            message: Some(format!("{e:#}")),
        },
    }
}
