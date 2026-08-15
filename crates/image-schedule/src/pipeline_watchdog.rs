//! Pipeline stall watchdog — gptimage `pipeline_watchdog` subset.

use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct PipelineWatchdog {
    stall_secs: u64,
    last_progress: Arc<Mutex<Instant>>,
    tripped_until: Arc<Mutex<Option<Instant>>>,
    trip_cooldown_secs: u64,
}

impl PipelineWatchdog {
    pub fn from_env() -> Self {
        let stall_secs = std::env::var("IMAGE_PIPELINE_STALL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(180);
        let trip_cooldown_secs = std::env::var("IMAGE_PIPELINE_TRIP_COOLDOWN_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);
        Self::new(stall_secs, trip_cooldown_secs)
    }

    pub fn new(stall_secs: u64, trip_cooldown_secs: u64) -> Self {
        Self {
            stall_secs: stall_secs.max(30),
            last_progress: Arc::new(Mutex::new(Instant::now())),
            tripped_until: Arc::new(Mutex::new(None)),
            trip_cooldown_secs: trip_cooldown_secs.max(15),
        }
    }

    pub fn mark_progress(&self) {
        *self.last_progress.lock() = Instant::now();
        *self.tripped_until.lock() = None;
    }

    pub fn evaluate(&self, queued_tasks: usize, running_tasks: usize) {
        if queued_tasks == 0 && running_tasks == 0 {
            self.mark_progress();
            return;
        }
        let idle = Instant::now().duration_since(*self.last_progress.lock());
        if idle > Duration::from_secs(self.stall_secs) && (queued_tasks > 0 || running_tasks > 0) {
            let until = Instant::now() + Duration::from_secs(self.trip_cooldown_secs);
            *self.tripped_until.lock() = Some(until);
        }
    }

    pub fn is_tripped(&self) -> bool {
        let now = Instant::now();
        let mut trip = self.tripped_until.lock();
        if let Some(until) = *trip {
            if now < until {
                return true;
            }
            *trip = None;
        }
        false
    }

    pub fn stats_json(&self, queued: usize, running: usize) -> serde_json::Value {
        serde_json::json!({
            "tripped": self.is_tripped(),
            "queued_tasks": queued,
            "running_tasks": running,
            "stall_secs": self.stall_secs,
            "idle_secs": Instant::now().duration_since(*self.last_progress.lock()).as_secs(),
        })
    }
}
