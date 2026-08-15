//! CPU deadlock guard — gptimage `image_deadlock_guard_service` subset.

use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::{Pid, System};

#[derive(Clone)]
pub struct DeadlockGuard {
    cpu_trip_pct: f64,
    window_secs: u64,
    trip_cooldown_secs: u64,
    samples: Arc<Mutex<Vec<(Instant, f32)>>>,
    tripped_until: Arc<Mutex<Option<Instant>>>,
}

impl DeadlockGuard {
    pub fn from_env() -> Self {
        let cpu_trip_pct = std::env::var("IMAGE_DEADLOCK_CPU_TRIP_PCT")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(90.0)
            .clamp(50.0, 100.0);
        let window_secs = std::env::var("IMAGE_DEADLOCK_WINDOW_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60);
        let trip_cooldown_secs = std::env::var("IMAGE_DEADLOCK_TRIP_COOLDOWN_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(120);
        Self::new(cpu_trip_pct, window_secs, trip_cooldown_secs)
    }

    pub fn new(cpu_trip_pct: f64, window_secs: u64, trip_cooldown_secs: u64) -> Self {
        Self {
            cpu_trip_pct,
            window_secs: window_secs.max(10),
            trip_cooldown_secs: trip_cooldown_secs.max(30),
            samples: Arc::new(Mutex::new(Vec::new())),
            tripped_until: Arc::new(Mutex::new(None)),
        }
    }

    pub fn sample_process_cpu(&self) {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        let pid = Pid::from_u32(std::process::id());
        let cpu = sys
            .process(pid)
            .map(|p| p.cpu_usage())
            .unwrap_or(0.0);
        self.record_sample(cpu);
        self.evaluate_trip();
    }

    pub fn record_sample(&self, cpu: f32) {
        let now = Instant::now();
        let window = Duration::from_secs(self.window_secs);
        let mut samples = self.samples.lock();
        samples.push((now, cpu));
        samples.retain(|(t, _)| now.duration_since(*t) <= window);
    }

    pub fn evaluate_trip(&self) {
        let samples = self.samples.lock();
        if samples.is_empty() {
            return;
        }
        let p95 = percentile_cpu(&samples, 0.95);
        if p95 >= self.cpu_trip_pct as f32 {
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

    pub fn stats_json(&self) -> serde_json::Value {
        let samples = self.samples.lock();
        let p95 = if samples.is_empty() {
            0.0
        } else {
            percentile_cpu(&samples, 0.95)
        };
        json!({
            "tripped": self.is_tripped(),
            "cpu_p95": p95,
            "cpu_trip_pct": self.cpu_trip_pct,
            "window_secs": self.window_secs,
            "sample_count": samples.len(),
        })
    }
}

fn percentile_cpu(samples: &[(Instant, f32)], pct: f64) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut vals: Vec<f32> = samples.iter().map(|(_, c)| *c).collect();
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((vals.len() as f64 - 1.0) * pct).round() as usize;
    vals[idx.min(vals.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trip_latches_on_high_p95_samples() {
        let guard = DeadlockGuard::new(90.0, 60, 30);
        for _ in 0..8 {
            guard.record_sample(95.0);
        }
        guard.evaluate_trip();
        assert!(guard.is_tripped());
    }
}
