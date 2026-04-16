//! Background task that recovers workflow tasks whose worker has died.
//!
//! When a worker claims a workflow task it sets `dispatch_claimed_by` and
//! `dispatch_last_heartbeat`. If the worker crashes (SIGKILL, OOM, network
//! partition), nothing in the worker's normal flow will release the lease.
//! This poller periodically scans for leases whose heartbeat is older than
//! `WORKFLOW_TASK_HEARTBEAT_TIMEOUT_SECS` and re-arms the task as
//! dispatchable so any other worker on the queue can take over.
//!
//! Combined with the deterministic-replay model, this gives crash safety:
//! the new worker reads the workflow's full event history and replays past
//! the same point the dead worker reached, with all completed activities /
//! timers / signals / side-effects short-circuiting from cache.

use std::sync::Arc;

use tokio::time::{interval, Duration};
use tracing::{debug, error, info};

use crate::store::WorkflowStore;

/// How long a worker can be silent before its dispatch lease is forcibly
/// released. Short enough to keep stuck workflows moving, long enough to
/// tolerate brief network blips. Override at runtime via the
/// `ASSAY_WF_DISPATCH_TIMEOUT_SECS` env var (used by tests to avoid
/// waiting 30s for crash-recovery).
pub const WORKFLOW_TASK_HEARTBEAT_TIMEOUT_SECS: f64 = 30.0;

/// How often the recovery poller runs. Should be << the timeout so a
/// single missed cycle doesn't double the recovery time.
const POLL_SECS: u64 = 1;

fn timeout_secs() -> f64 {
    std::env::var("ASSAY_WF_DISPATCH_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(WORKFLOW_TASK_HEARTBEAT_TIMEOUT_SECS)
}

pub async fn run_dispatch_recovery<S: WorkflowStore>(store: Arc<S>) {
    let mut tick = interval(Duration::from_secs(POLL_SECS));
    let t = timeout_secs();
    info!(
        "Dispatch-recovery poller started (poll every {POLL_SECS}s, \
         release stale leases older than {t}s)"
    );

    loop {
        tick.tick().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        match store.release_stale_dispatch_leases(now, timeout_secs()).await {
            Ok(0) => {}
            Ok(n) => debug!("Released {n} stale workflow dispatch lease(s)"),
            Err(e) => error!("Dispatch-recovery poller error: {e}"),
        }
    }
}
