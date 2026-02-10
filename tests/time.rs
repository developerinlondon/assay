mod common;

use common::{eval_lua, run_lua};
use std::time::Duration;

#[tokio::test]
async fn test_time_returns_epoch() {
    let result: f64 = eval_lua("return time()").await;
    assert!(result > 1_700_000_000.0);
}

#[tokio::test]
async fn test_sleep_brief() {
    let start = std::time::Instant::now();
    run_lua("sleep(0.05)").await.unwrap();
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(40));
    assert!(elapsed < Duration::from_millis(200));
}
