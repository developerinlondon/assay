use std::sync::Arc;

use anyhow::Result;
use tokio::time::{Duration, interval};
use tracing::{error, info, warn};

use crate::activities::{failed_event_payload, settled_event_payload};
use crate::store::WorkflowStore;
use crate::types::{ActivitySettlement, SettleOutcome, WorkflowActivity, WorkflowStatus};

const HEALTH_CHECK_SECS: u64 = 30;
const WORKER_TIMEOUT_SECS: f64 = 90.0;
/// Upper bound on half-settled activities repaired per pass, so a backlog
/// drains over successive passes instead of one long run of transactions.
const RECONCILE_BATCH: i64 = 100;

/// Detects dead workers and releases their claimed tasks.
/// Detects timed-out activities and re-queues them for retry (terminal
/// failure + workflow FAILED once attempts are exhausted).
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
    check_health_at(store, timestamp_now()).await
}

/// One health-check pass at an injected `now` — public so integration tests
/// can drive the reaper deterministically without waiting on wall-clock.
pub async fn check_health_at<S: WorkflowStore>(store: &S, now: f64) -> Result<()> {
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
            // Retry: re-queue with the same exponential backoff fail_activity
            // uses. (Terminally completing here used to wedge the run: the
            // activity went FAILED with no event and no needs_dispatch, so the
            // workflow sat RUNNING forever.)
            let backoff = act.initial_interval_secs * act.backoff_coefficient.powi(act.attempt - 1);
            store
                .requeue_activity_for_retry(act_id, act.attempt + 1, now + backoff)
                .await?;
            warn!(
                "Activity {} heartbeat timed out (attempt {}/{}) — re-queued with backoff",
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

    // 3. Converge activities that settled without their history event
    reconcile_settled_activities(store).await?;

    Ok(())
}

/// Re-settle activities that reached a terminal status without their
/// terminal history event. Deterministic replay reads such a workflow as
/// still waiting on the activity, so the run sits RUNNING with nothing left
/// to wake it. `settle_activity` closes that window for new completions;
/// rows written by an older engine still need this pass. Returns the number
/// repaired.
pub async fn reconcile_settled_activities<S: WorkflowStore>(store: &S) -> Result<usize> {
    let candidates = store.list_unsettled_activities(RECONCILE_BATCH).await?;
    let mut repaired = 0;
    for act in candidates {
        let Some(id) = act.id else { continue };
        let failed = act.status == "FAILED";
        let payload = settlement_payload(&act, id, failed);
        let outcome = store
            .settle_activity(&ActivitySettlement {
                activity_id: id,
                workflow_id: &act.workflow_id,
                result: act.result.as_deref(),
                error: act.error.as_deref(),
                failed,
                event_type: if failed {
                    "ActivityFailed"
                } else {
                    "ActivityCompleted"
                },
                payload: &payload,
                now: timestamp_now(),
            })
            .await?;
        if outcome == SettleOutcome::Repaired {
            repaired += 1;
            warn!(
                "Activity {} (id {id}) was settled without its history event — \
                 appended it and re-armed workflow {}",
                act.name, act.workflow_id
            );
        }
    }
    Ok(repaired)
}

/// Rebuild the terminal event payload from the stored row, in the shape the
/// live completion path would have written.
fn settlement_payload(act: &WorkflowActivity, id: i64, failed: bool) -> String {
    if failed {
        failed_event_payload(
            id,
            act.seq,
            &act.name,
            act.error.as_deref().unwrap_or("activity failed"),
            act.attempt,
        )
    } else {
        settled_event_payload(
            id,
            act.seq,
            &act.name,
            act.result.as_deref(),
            act.error.as_deref(),
        )
    }
    .to_string()
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
