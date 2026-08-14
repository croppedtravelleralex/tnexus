//! Background upkeep so the pool degrades on its own schedule rather than on a
//! user's request.
//!
//! Three loops:
//! * measure — chat-probe unmeasured accounts so entitlements show up quickly
//! * sweep — cheap token keepalive on accounts we have already measured
//! * remint — try a sibling Web SSO before a revoked Build row is dropped
//!
//! After each pass, terminal rows (`needs_reauth` / `forbidden`) are deleted
//! so they cannot clog stats or steal scheduler slots.

use std::sync::Arc;
use std::time::Duration;

use tracing::{info, warn};

use crate::api::AppState;
use crate::probe::Probe;

const WARMUP_SECS: u64 = 45;
const MEASURE_INTERVAL_SECS: u64 = 60;
const MEASURE_BATCH: usize = 40;
const MEASURE_CONCURRENCY: usize = 8;
const REMINT_WARMUP_SECS: u64 = 90;
const REMINT_INTERVAL_SECS: u64 = 60;
const REMINT_BATCH: usize = 12;
const REMINT_CONCURRENCY: usize = 3;

pub fn spawn(state: Arc<AppState>) {
    spawn_measure(state.clone());
    spawn_sweep(state.clone());
    spawn_remint(state);
}

fn spawn_measure(state: Arc<AppState>) {
    info!(
        every_secs = MEASURE_INTERVAL_SECS,
        batch = MEASURE_BATCH,
        "measure loop started"
    );
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(WARMUP_SECS)).await;
        let mut ticker = tokio::time::interval(Duration::from_secs(MEASURE_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match state
                .pool
                .probe_pool(Some(Probe::Chat), MEASURE_BATCH, MEASURE_CONCURRENCY)
                .await
            {
                Ok(report) => {
                    let purged = state.pool.purge_unusable().unwrap_or(0);
                    if report.checked > 0 || purged > 0 {
                        info!(
                            checked = report.checked,
                            alive = report.alive,
                            no_permission = report.no_permission,
                            revoked = report.revoked,
                            budget_spent = report.budget_spent,
                            purged,
                            "measure done"
                        );
                    }
                }
                Err(err) => warn!(error = %err, "measure failed"),
            }
        }
    });
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
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match state
                .pool
                .probe_pool(Some(Probe::Token), batch, concurrency)
                .await
            {
                Ok(report) => {
                    let purged = state.pool.purge_unusable().unwrap_or(0);
                    info!(
                        checked = report.checked,
                        alive = report.alive,
                        revoked = report.revoked,
                        unreachable = report.unreachable,
                        purged,
                        "sweep done"
                    );
                }
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
                Ok(report) if report.attempted == 0 => {
                    let _ = state.pool.purge_unusable();
                }
                Ok(report) => {
                    let purged = state.pool.purge_unusable().unwrap_or(0);
                    info!(
                        attempted = report.attempted,
                        revived = report.revived,
                        sso_rejected = report.sso_rejected,
                        failed = report.failed,
                        remaining = report.remaining,
                        purged,
                        "remint done"
                    );
                }
                Err(err) => warn!(error = %err, "remint failed"),
            }
        }
    });
}
