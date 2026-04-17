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

        // Leader election: only one instance should evaluate cron schedules.
        // SQLite always returns true (single-instance).
        // Postgres uses pg_try_advisory_lock — only one pod wins.
        match store.try_acquire_scheduler_lock().await {
            Ok(true) => {
                if let Err(e) = evaluate_schedules(&*store).await {
                    error!("Scheduler error: {e}");
                }
            }
            Ok(false) => {
                debug!("Scheduler: not the leader, skipping cron evaluation");
            }
            Err(e) => {
                error!("Scheduler: leader election failed: {e}");
            }
        }
    }
}

async fn evaluate_schedules<S: WorkflowStore>(store: &S) -> Result<()> {
    let namespaces = store.list_namespaces().await?;
    let now = timestamp_now();

    for ns in &namespaces {
        if let Err(e) = evaluate_namespace_schedules(store, &ns.name, now).await {
            error!("Scheduler error in namespace '{}': {e}", ns.name);
        }
    }
    Ok(())
}

async fn evaluate_namespace_schedules<S: WorkflowStore>(store: &S, namespace: &str, now: f64) -> Result<()> {
    let schedules = store.list_schedules(namespace).await?;

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

        // Parse cron to compute next run — interpreted in the schedule's
        // configured timezone (defaults to UTC).
        let next_run = match compute_next_run(&sched.cron_expr, &sched.timezone) {
            Some(t) => t,
            None => {
                warn!(
                    "Invalid cron expression or timezone for schedule '{}': expr={} tz={}",
                    sched.name, sched.cron_expr, sched.timezone
                );
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
                .update_schedule_last_run(namespace, &sched.name, now, next_run, last_wf_id)
                .await?;
            continue;
        }

        // Start a new workflow run
        let workflow_id = format!("{}-{}", sched.name, now as u64);
        let run_id = format!("run-{workflow_id}");

        let wf = WorkflowRecord {
            id: workflow_id.clone(),
            namespace: namespace.to_string(),
            run_id,
            workflow_type: sched.workflow_type.clone(),
            task_queue: sched.task_queue.clone(),
            status: "PENDING".to_string(),
            input: sched.input.clone(),
            result: None,
            error: None,
            parent_id: None,
            claimed_by: None,
            search_attributes: None,
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

        // Phase 9: scheduled workflows must enter the dispatch pool so a
        // worker can pick them up. Without this they'd sit PENDING forever.
        store.mark_workflow_dispatchable(&workflow_id).await?;

        store
            .update_schedule_last_run(namespace, &sched.name, now, next_run, &workflow_id)
            .await?;

        info!(
            "Schedule '{}': started workflow {workflow_id} (type: {})",
            sched.name, sched.workflow_type
        );
    }

    Ok(())
}

fn compute_next_run(cron_expr: &str, timezone: &str) -> Option<f64> {
    let schedule = Schedule::from_str(cron_expr).ok()?;
    // Empty string is treated as UTC so older schedules (pre-v0.11.3) keep
    // behaving identically even if they migrate in without the column set.
    if timezone.is_empty() || timezone.eq_ignore_ascii_case("UTC") {
        let next = schedule.upcoming(chrono::Utc).next()?;
        return Some(next.timestamp() as f64);
    }
    let tz: chrono_tz::Tz = timezone.parse().ok()?;
    let next = schedule.upcoming(tz).next()?;
    Some(next.timestamp() as f64)
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Daily at 02:00" computes a different UTC epoch depending on the
    /// schedule's timezone. UTC and Europe/Berlin produce different
    /// next-fire times outside of the months where they happen to align.
    #[test]
    fn compute_next_run_honors_timezone() {
        let utc_next = compute_next_run("0 0 2 * * *", "UTC").expect("utc next_run");
        let berlin_next =
            compute_next_run("0 0 2 * * *", "Europe/Berlin").expect("berlin next_run");
        assert_ne!(
            utc_next, berlin_next,
            "02:00 UTC and 02:00 Europe/Berlin should not coincide"
        );
    }

    #[test]
    fn compute_next_run_empty_timezone_defaults_to_utc() {
        let utc_next = compute_next_run("0 0 2 * * *", "UTC").expect("utc next_run");
        let default_next = compute_next_run("0 0 2 * * *", "").expect("empty next_run");
        assert_eq!(utc_next, default_next);
    }

    #[test]
    fn compute_next_run_invalid_timezone_returns_none() {
        assert!(compute_next_run("0 0 2 * * *", "Not/AZone").is_none());
    }

    #[test]
    fn compute_next_run_invalid_cron_returns_none() {
        assert!(compute_next_run("not a cron", "UTC").is_none());
    }
}
