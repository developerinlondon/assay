use std::str::FromStr;
use std::sync::Arc;

use anyhow::Result;
use cron::Schedule;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

use crate::store::WorkflowStore;
use crate::types::{WorkflowEvent, WorkflowRecord};

const SCHEDULER_POLL_SECS: u64 = 15;

/// Evaluates cron schedules and starts workflow runs when they're due.
/// Runs as a background tokio task.
pub async fn run_scheduler<S: WorkflowStore>(store: Arc<S>) {
    let mut tick = interval(Duration::from_secs(SCHEDULER_POLL_SECS));
    info!("Cron scheduler started (poll every {SCHEDULER_POLL_SECS}s)");

    loop {
        tick.tick().await;
        if let Err(e) = evaluate_schedules(&*store).await {
            error!("Scheduler error: {e}");
        }
    }
}

async fn evaluate_schedules<S: WorkflowStore>(store: &S) -> Result<()> {
    let schedules = store.list_schedules().await?;
    let now = timestamp_now();

    for sched in schedules {
        if sched.paused {
            continue;
        }

        // Check if the schedule is due
        let is_due = match sched.next_run_at {
            Some(next) => now >= next,
            None => true, // Never run before — run now
        };

        if !is_due {
            continue;
        }

        // Parse cron to compute next run
        let next_run = match compute_next_run(&sched.cron_expr) {
            Some(t) => t,
            None => {
                warn!("Invalid cron expression for schedule '{}': {}", sched.name, sched.cron_expr);
                continue;
            }
        };

        // Check overlap policy — skip if previous run is still active
        if sched.overlap_policy == "skip"
            && let Some(ref last_wf_id) = sched.last_workflow_id
            && let Some(wf) = store.get_workflow(last_wf_id).await?
            && !crate::types::WorkflowStatus::from_str(&wf.status)
                .map(|s| s.is_terminal())
                .unwrap_or(true)
        {
            debug!(
                "Schedule '{}': skipping — previous run {} still {}",
                sched.name, last_wf_id, wf.status
            );
            // Still update next_run_at so we don't re-evaluate every tick
            store
                .update_schedule_last_run(&sched.name, now, next_run, last_wf_id)
                .await?;
            continue;
        }

        // Start a new workflow run
        let workflow_id = format!("{}-{}", sched.name, now as u64);
        let run_id = format!("run-{workflow_id}");

        let wf = WorkflowRecord {
            id: workflow_id.clone(),
            run_id,
            workflow_type: sched.workflow_type.clone(),
            task_queue: sched.task_queue.clone(),
            status: "PENDING".to_string(),
            input: sched.input.clone(),
            result: None,
            error: None,
            parent_id: None,
            claimed_by: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        store.create_workflow(&wf).await?;
        store
            .append_event(&WorkflowEvent {
                id: None,
                workflow_id: workflow_id.clone(),
                seq: 1,
                event_type: "WorkflowStarted".to_string(),
                payload: sched.input.clone(),
                timestamp: now,
            })
            .await?;

        store
            .update_schedule_last_run(&sched.name, now, next_run, &workflow_id)
            .await?;

        info!(
            "Schedule '{}': started workflow {workflow_id} (type: {})",
            sched.name, sched.workflow_type
        );
    }

    Ok(())
}

fn compute_next_run(cron_expr: &str) -> Option<f64> {
    let schedule = Schedule::from_str(cron_expr).ok()?;
    let next = schedule.upcoming(chrono::Utc).next()?;
    Some(next.timestamp() as f64)
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
