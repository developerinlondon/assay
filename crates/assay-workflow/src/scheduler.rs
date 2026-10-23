use std::str::FromStr;
use std::sync::Arc;

use anyhow::Result;
use cron::Schedule;
use tokio::time::{Duration, interval};
use tracing::{debug, error, info, warn};

use crate::store::WorkflowStore;
use crate::types::{WorkflowEvent, WorkflowRecord, WorkflowSchedule};

const SCHEDULER_POLL_SECS: u64 = 15;

/// Evaluates cron schedules and starts workflow runs when they're due.
/// Runs as a background tokio task.
pub async fn run_scheduler<S: WorkflowStore>(store: Arc<S>) {
    let mut tick = interval(Duration::from_secs(SCHEDULER_POLL_SECS));
    info!("Cron scheduler started (poll every {SCHEDULER_POLL_SECS}s)");

    // Report never-fireable rows once, at boot, rather than on every tick.
    // Creation rejects such expressions now, so anything listed here predates
    // that validation or was written straight to the store by an embedder.
    // This runs before leader election, so under Postgres every pod reports
    // the same list — one line each is cheaper than coordinating the report.
    match unfireable_schedules(&*store).await {
        Ok(rows) if !rows.is_empty() => {
            warn!(
                "Scheduler: {} schedule(s) can never fire and will be skipped: {}",
                rows.len(),
                rows.join(", ")
            );
        }
        Ok(_) => {}
        Err(e) => error!("Scheduler: could not check for never-fireable schedules: {e}"),
    }

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
    evaluate_schedules_at(store, timestamp_now()).await
}

/// One scheduler pass at an injected `now` — public so integration tests can
/// drive cron evaluation deterministically without waiting on the poll tick.
pub async fn evaluate_schedules_at<S: WorkflowStore>(store: &S, now: f64) -> Result<()> {
    let namespaces = store.list_namespaces().await?;

    for ns in &namespaces {
        if let Err(e) = evaluate_namespace_schedules(store, &ns.name, now).await {
            error!("Scheduler error in namespace '{}': {e}", ns.name);
        }
    }
    Ok(())
}

async fn evaluate_namespace_schedules<S: WorkflowStore>(
    store: &S,
    namespace: &str,
    now: f64,
) -> Result<()> {
    let schedules = store.list_schedules(namespace).await?;

    for sched in schedules {
        if sched.paused {
            continue;
        }

        // Creation seeds next_run_at (see `seed_next_run`) and refuses a cron
        // that can never fire (see `validate_cron_expr`), so a NULL here means
        // a row that predates one of those or one written straight to the
        // store. The seeding is not retroactive: a pre-seed row still fires
        // once, and that fire gives it a cadence.
        let is_due = match sched.next_run_at {
            Some(next) => now >= next,
            None => true,
        };

        if !is_due {
            continue;
        }

        // Parse cron to compute next run — interpreted in the schedule's
        // configured timezone (defaults to UTC).
        let next_run = match compute_next_run(&sched.cron_expr, &sched.timezone) {
            Some(t) => t,
            None => {
                // A NULL next_run_at reads as due, so this arm would otherwise
                // log on every poll for as long as the row exists. The boot
                // report in `run_scheduler` names these rows once instead.
                debug!(
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

        let workflow_id = start_scheduled_run(store, namespace, &sched, now).await?;

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

async fn start_scheduled_run<S: WorkflowStore>(
    store: &S,
    namespace: &str,
    sched: &WorkflowSchedule,
    now: f64,
) -> Result<String> {
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
        archived_at: None,
        archive_uri: None,
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

    // worker can pick them up. Without this they'd sit PENDING forever.
    store.mark_workflow_dispatchable(&workflow_id).await?;

    Ok(workflow_id)
}

/// `next_run_at` to persist for a schedule being created. The scheduler reads
/// a NULL `next_run_at` as due-now, so an unseeded schedule fires one run at
/// registration no matter what its cron says.
pub(crate) fn seed_next_run(sched: &WorkflowSchedule) -> Option<f64> {
    sched
        .next_run_at
        .or_else(|| compute_next_run(&sched.cron_expr, &sched.timezone))
}

pub(crate) fn compute_next_run(cron_expr: &str, timezone: &str) -> Option<f64> {
    next_occurrence(&Schedule::from_str(cron_expr).ok()?, timezone)
}

/// The next fire time of an already-parsed cron, as a UTC epoch.
fn next_occurrence(schedule: &Schedule, timezone: &str) -> Option<f64> {
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

/// Whether a cron expression can actually produce a run, with the reason it
/// cannot as the error. Create and patch call this so a schedule that could
/// never fire is refused rather than stored: such a row keeps `next_run_at`
/// NULL, which the scheduler reads as due on every pass, so the caller holds a
/// 201 for a schedule that does nothing.
///
/// Asks the same code the scheduler uses rather than counting fields, so
/// "accepted" and "fireable" cannot drift apart — this also catches an
/// expression that parses but whose year has passed. Validate the timezone
/// first: an unparseable zone surfaces here as a no-future-occurrence error,
/// which names the wrong field.
pub fn validate_cron_expr(cron_expr: &str, timezone: &str) -> Result<(), String> {
    let Ok(schedule) = Schedule::from_str(cron_expr) else {
        return Err(format!(
            "invalid cron expression '{cron_expr}': expected 6 or 7 whitespace-separated fields \
             (second minute hour day-of-month month day-of-week [year]); the 5-field crontab form \
             does not parse"
        ));
    };
    if next_occurrence(&schedule, timezone).is_none() {
        return Err(format!(
            "cron expression '{cron_expr}' has no future occurrence in timezone '{timezone}'"
        ));
    }
    Ok(())
}

/// Stored schedules that can never fire, as `"{namespace}/{name} cron=… tz=…"`
/// lines. Paused rows are left out — a paused schedule is not expected to fire,
/// and resuming it recomputes `next_run_at` anyway.
///
/// Public for the same reason `evaluate_schedules_at` is: integration tests
/// drive it directly across both backends.
pub async fn unfireable_schedules<S: WorkflowStore>(store: &S) -> Result<Vec<String>> {
    let mut unfireable = Vec::new();

    for ns in store.list_namespaces().await? {
        for sched in store.list_schedules(&ns.name).await? {
            if sched.paused {
                continue;
            }
            if compute_next_run(&sched.cron_expr, &sched.timezone).is_none() {
                unfireable.push(format!(
                    "{}/{} cron={} tz={}",
                    ns.name, sched.name, sched.cron_expr, sched.timezone
                ));
            }
        }
    }

    Ok(unfireable)
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

    fn schedule_in_timezone(timezone: &str) -> WorkflowSchedule {
        WorkflowSchedule {
            name: "sched".to_string(),
            namespace: "main".to_string(),
            workflow_type: "wf".to_string(),
            cron_expr: "0 0 8 * * *".to_string(),
            timezone: timezone.to_string(),
            input: None,
            task_queue: "main".to_string(),
            overlap_policy: "skip".to_string(),
            paused: false,
            last_run_at: None,
            next_run_at: None,
            last_workflow_id: None,
            created_at: 0.0,
        }
    }

    /// The seed must be computed in the schedule's own timezone: "daily at
    /// 08:00" is a different UTC instant in Auckland (UTC+12/+13) than in UTC.
    #[test]
    fn seed_next_run_honors_the_schedule_timezone() {
        let auckland =
            seed_next_run(&schedule_in_timezone("Pacific/Auckland")).expect("auckland seed");
        let utc = seed_next_run(&schedule_in_timezone("UTC")).expect("utc seed");
        assert_ne!(
            auckland, utc,
            "08:00 Pacific/Auckland and 08:00 UTC should not coincide"
        );
    }

    #[test]
    fn seed_next_run_preserves_an_explicit_next_run_at() {
        let mut sched = schedule_in_timezone("UTC");
        sched.next_run_at = Some(1.0);
        assert_eq!(seed_next_run(&sched), Some(1.0));
    }

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

    #[test]
    fn validate_cron_expr_accepts_the_six_field_form() {
        assert_eq!(validate_cron_expr("0 0 8 * * *", "Europe/Berlin"), Ok(()));
    }

    /// The optional 7th field is a year. 2099 is far enough out that this test
    /// does not expire; a nearer year would start failing once it passed.
    #[test]
    fn validate_cron_expr_accepts_a_seven_field_form_with_a_future_year() {
        assert_eq!(validate_cron_expr("0 0 8 1 1 * 2099", "UTC"), Ok(()));
    }

    /// The expression from the report: a 5-field crontab the cron crate
    /// rejects. The message has to name the field count, since the caller's
    /// mistake is invisible otherwise — it reads as a valid crontab line.
    #[test]
    fn validate_cron_expr_rejects_the_five_field_crontab_form() {
        let err = validate_cron_expr("0 8 1 1 *", "UTC").expect_err("5 fields must be rejected");
        assert!(
            err.contains("6 or 7") && err.contains("5-field"),
            "message should name the expected field count, got: {err}"
        );
    }

    /// Parses, but every occurrence is in the past, so it is as unfireable as
    /// one that does not parse — and would leave the same NULL next_run_at row.
    #[test]
    fn validate_cron_expr_rejects_a_year_that_has_passed() {
        let err =
            validate_cron_expr("0 0 8 1 1 * 2020", "UTC").expect_err("a past year cannot fire");
        assert!(
            err.contains("no future occurrence"),
            "message should say there is no future occurrence, got: {err}"
        );
    }

    #[test]
    fn validate_cron_expr_rejects_garbage() {
        assert!(validate_cron_expr("not a cron", "UTC").is_err());
    }
}
