use std::sync::Arc;

use anyhow::Result;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info};

use crate::store::WorkflowStore;
use crate::types::WorkflowEvent;

const TIMER_POLL_SECS: u64 = 1;

/// Polls for due timers and records TimerFired events.
/// Runs as a background tokio task.
pub async fn run_timer_poller<S: WorkflowStore>(store: Arc<S>) {
    let mut tick = interval(Duration::from_secs(TIMER_POLL_SECS));
    info!("Timer poller started (poll every {TIMER_POLL_SECS}s)");

    loop {
        tick.tick().await;
        if let Err(e) = fire_timers(&*store).await {
            error!("Timer poller error: {e}");
        }
    }
}

async fn fire_timers<S: WorkflowStore>(store: &S) -> Result<()> {
    let now = timestamp_now();
    let fired = store.fire_due_timers(now).await?;

    for timer in fired {
        let event = WorkflowEvent {
            id: None,
            workflow_id: timer.workflow_id.clone(),
            seq: timer.seq,
            event_type: "TimerFired".to_string(),
            payload: Some(serde_json::json!({ "fire_at": timer.fire_at }).to_string()),
            timestamp: now,
        };

        if let Err(e) = store.append_event(&event).await {
            error!(
                "Failed to record TimerFired event for workflow {}: {e}",
                timer.workflow_id
            );
            continue;
        }

        debug!(
            "Timer fired: workflow={}, seq={}",
            timer.workflow_id, timer.seq
        );
    }

    Ok(())
}

fn timestamp_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
