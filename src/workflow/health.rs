use std::sync::Arc;

use anyhow::Result;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

use crate::workflow::store::WorkflowStore;
use crate::workflow::types::WorkflowStatus;

const HEALTH_CHECK_SECS: u64 = 30;
const WORKER_TIMEOUT_SECS: f64 = 90.0;

/// Detects dead workers and releases their claimed tasks.
/// Detects timed-out activities and marks them failed for retry.
/// Runs as a background tokio task.
pub async fn run_health_monitor<S: WorkflowStore>(store: Arc<S>) {
    let mut tick = interval(Duration::from_secs(HEALTH_CHECK_SECS));
    info!("Health monitor started (check every {HEALTH_CHECK_SECS}s)");

    loop {
        tick.tick().await;
        if let Err(e) = check_health(&*store).await {
            error!("Health monitor error: {e}");
        }
    }
}

async fn check_health<S: WorkflowStore>(store: &S) -> Result<()> {
    let now = timestamp_now();

    // 1. Remove dead workers (no heartbeat within timeout)
    let cutoff = now - WORKER_TIMEOUT_SECS;
    let dead_workers = store.remove_dead_workers(cutoff).await?;
    for worker_id in &dead_workers {
        warn!("Removed dead worker: {worker_id}");
    }

    // 2. Find activities that have timed out (heartbeat expired)
    let timed_out = store.get_timed_out_activities(now).await?;
    for act in &timed_out {
        let act_id = act.id.unwrap_or(-1);

        if act.attempt < act.max_attempts {
            // Retry: mark as failed so the engine can reschedule
            store
                .complete_activity(
                    act_id,
                    None,
                    Some("heartbeat timeout"),
                    true,
                )
                .await?;
            warn!(
                "Activity {} timed out (attempt {}/{}), marked for retry",
                act.name, act.attempt, act.max_attempts
            );
        } else {
            // Max retries exhausted — fail permanently
            store
                .complete_activity(
                    act_id,
                    None,
                    Some("heartbeat timeout — max retries exhausted"),
                    true,
                )
                .await?;

            // Fail the parent workflow
            store
                .update_workflow_status(
                    &act.workflow_id,
                    WorkflowStatus::Failed,
                    None,
                    Some(&format!(
                        "Activity '{}' timed out after {} attempts",
                        act.name, act.max_attempts
                    )),
                )
                .await?;
            warn!(
                "Activity {} permanently failed — workflow {} marked FAILED",
                act.name, act.workflow_id
            );
        }
    }

    Ok(())
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
