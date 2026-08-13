//! Background upkeep so the pool degrades on its own schedule rather than on a
//! user's request.
//!
//! Without this, a revoked account stays `active` until someone happens to
//! route traffic to it, and every such request pays a full upstream round trip
//! before failing over. The janitor finds those accounts on a timer instead.

use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

use crate::api::AppState;

/// Wait this long after boot before the first sweep, so startup and the sweep's
/// burst of upstream calls do not overlap.
const WARMUP_SECS: u64 = 60;

pub fn spawn(state: Arc<AppState>) {
    let interval_secs = state.config.sweep_interval_secs;
    if interval_secs == 0 {
        info!("janitor disabled (GROKPROXY_SWEEP_INTERVAL_SECS=0)");
        return;
    }
    let batch = state.config.sweep_batch;
    let concurrency = state.config.sweep_concurrency;
    info!(
        every_secs = interval_secs,
        batch, concurrency, "janitor started"
    );

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(WARMUP_SECS)).await;
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        // A slow sweep must not queue up catch-up ticks behind it.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match state.pool.sweep(batch, concurrency).await {
                Ok(report) => info!(
                    checked = report.checked,
                    alive = report.alive,
                    revoked = report.revoked,
                    other = report.other,
                    "sweep done"
                ),
                // Never break the loop: one bad sweep should not stop upkeep
                // for the lifetime of the process.
                Err(err) => warn!(error = %err, "sweep failed"),
            }
        }
    });
}
