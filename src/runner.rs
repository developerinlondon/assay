use crate::build_http_client;
use crate::checks;
use crate::config::Config;
use crate::output::{CheckResult, RunResult};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{error, info, warn};

pub async fn run(config: &Config) -> RunResult {
    if config.parallel {
        warn!("parallel execution not yet implemented, running sequentially");
    }

    let start = Instant::now();
    let client = build_http_client();
    let results = Arc::new(Mutex::new(Vec::with_capacity(config.checks.len())));

    let run_future = run_all_checks(config, &client, Arc::clone(&results));

    match timeout(config.timeout, run_future).await {
        Ok(()) => {}
        Err(_) => {
            error!(
                timeout_secs = config.timeout.as_secs(),
                "global timeout exceeded"
            );
            let mut results = results.lock().await;
            let completed = results.len();
            for check_config in config.checks.iter().skip(completed) {
                results.push(CheckResult {
                    name: check_config.name.clone(),
                    passed: false,
                    duration_ms: 0,
                    message: Some(format!(
                        "global timeout of {}s exceeded",
                        config.timeout.as_secs()
                    )),
                });
            }
        }
    }

    let results = Arc::into_inner(results)
        .expect("all references dropped")
        .into_inner();
    let all_passed = results.iter().all(|r| r.passed);
    let duration_ms = start.elapsed().as_millis() as u64;

    RunResult {
        passed: all_passed,
        checks: results,
        duration_ms,
    }
}

async fn run_all_checks(
    config: &Config,
    client: &reqwest::Client,
    results: Arc<Mutex<Vec<CheckResult>>>,
) {
    for check_config in &config.checks {
        let result = run_check_with_retries(config, check_config, client).await;
        let passed_str = if result.passed { "PASS" } else { "FAIL" };
        info!(
            check = check_config.name,
            result = passed_str,
            duration_ms = result.duration_ms,
            "check completed"
        );
        results.lock().await.push(result);
    }
}

async fn run_check_with_retries(
    config: &Config,
    check_config: &crate::config::CheckConfig,
    client: &reqwest::Client,
) -> CheckResult {
    let max_attempts = config.retries + 1;

    for attempt in 1..=max_attempts {
        let result = checks::run_check(check_config, client).await;

        if result.passed {
            return result;
        }

        if attempt == max_attempts {
            return result;
        }

        let backoff_secs = config.backoff.as_secs() * attempt as u64;
        info!(
            check = check_config.name,
            attempt,
            max_attempts,
            backoff_secs,
            message = result.message.as_deref().unwrap_or(""),
            "check failed, retrying"
        );

        let backoff_duration = config.backoff * attempt;
        tokio::time::sleep(backoff_duration).await;
    }

    CheckResult {
        name: check_config.name.clone(),
        passed: false,
        duration_ms: 0,
        message: Some("max retries exhausted".to_string()),
    }
}
