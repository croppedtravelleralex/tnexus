//! Background upkeep so the pool degrades on its own schedule rather than on a
//! user's request.
//!
//! Without this, a revoked account stays `active` until someone happens to
//! route traffic to it, and every such request pays a full upstream round trip
//! before failing over. The janitor finds those accounts on a timer instead.
//! A second loop remints `needs_reauth` Build rows from a still-valid Web SSO.

use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

use crate::api::AppState;

/// Wait this long after boot before the first sweep, so startup and the sweep's
/// burst of upstream calls do not overlap.
const WARMUP_SECS: u64 = 60;
/// Remint starts after the first sweep has had a chance to run, and then
/// chews through the `needs_reauth` backlog in small batches.
const REMINT_WARMUP_SECS: u64 = 90;
const REMINT_INTERVAL_SECS: u64 = 60;
const REMINT_BATCH: usize = 12;
const REMINT_CONCURRENCY: usize = 3;

pub fn spawn(state: Arc<AppState>) {
    spawn_sweep(state.clone());
    spawn_remint(state);
}

fn spawn_sweep(state: Arc<AppState>) {
    let interval_secs = state.config.sweep_interval_secs;
    if interval_secs == 0 {
        info!("janitor sweep disabled (GROKPROXY_SWEEP_INTERVAL_SECS=0)");
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
            // No fixed probe: each account gets the cheapest one that still
            // reveals something, so upkeep does not drain the pool it guards.
            match state.pool.probe_pool(None, batch, concurrency).await {
                Ok(report) => info!(
                    checked = report.checked,
                    alive = report.alive,
                    revoked = report.revoked,
                    no_permission = report.no_permission,
                    unreachable = report.unreachable,
                    budget_spent = report.budget_spent,
                    "sweep done"
                ),
                // Never break the loop: one bad sweep should not stop upkeep
                // for the lifetime of the process.
                Err(err) => warn!(error = %err, "sweep failed"),
            }
        }
    });
}

fn spawn_remint(state: Arc<AppState>) {
    info!(
        every_secs = REMINT_INTERVAL_SECS,
        batch = REMINT_BATCH,
        concurrency = REMINT_CONCURRENCY,
        "remint loop started"
    );
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(REMINT_WARMUP_SECS)).await;
        let mut ticker = tokio::time::interval(Duration::from_secs(REMINT_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match crate::xai::mint::remint_batch(
                state.pool.store(),
                &state.config.sticky_relay,
                REMINT_BATCH,
                REMINT_CONCURRENCY,
            )
            .await
            {
                Ok(report) if report.attempted == 0 => {}
                Ok(report) => info!(
                    attempted = report.attempted,
                    revived = report.revived,
                    sso_rejected = report.sso_rejected,
                    failed = report.failed,
                    remaining = report.remaining,
                    "remint done"
                ),
                Err(err) => warn!(error = %err, "remint failed"),
            }
        }
    });
}
