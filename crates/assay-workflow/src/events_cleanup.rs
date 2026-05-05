//! Hourly prune of `engine_events` older than the configured TTL.
//! Idempotent across nodes — `DELETE WHERE ts < cutoff` is a no-op
//! if another node already swept. No leader election needed.

use std::sync::Arc;
use std::time::Duration;

use assay_domain::events::EngineEventBus;

/// Run the cleanup loop forever. Callers spawn this as a tokio task
/// alongside the engine's other background housekeeping loops.
/// First tick is skipped so we don't prune at startup.
pub async fn run_events_cleanup(bus: Arc<dyn EngineEventBus>, cadence: Duration, ttl_secs: u64) {
    let mut tick = tokio::time::interval(cadence);
    tick.tick().await; // skip immediate first tick
    loop {
        tick.tick().await;
        let cutoff = now_secs() - ttl_secs as f64;
        match bus.prune(cutoff).await {
            Ok(n) if n > 0 => tracing::info!(pruned = n, "engine_events cleanup swept"),
            Ok(_) => tracing::debug!("engine_events cleanup: nothing to prune"),
            Err(e) => tracing::warn!(?e, "engine_events prune failed; will retry next tick"),
        }
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
