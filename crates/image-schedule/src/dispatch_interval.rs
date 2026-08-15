//! Per-account minimum interval between image dispatches.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct DispatchIntervalGate {
    interval_ms: Arc<parking_lot::RwLock<u64>>,
    last_dispatch: Arc<Mutex<HashMap<String, Instant>>>,
}

impl DispatchIntervalGate {
    pub fn from_env() -> Self {
        let interval_ms = std::env::var("IMAGE_DISPATCH_INTERVAL_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(800);
        Self::new(interval_ms)
    }

    pub fn new(interval_ms: u64) -> Self {
        Self {
            interval_ms: Arc::new(parking_lot::RwLock::new(interval_ms)),
            last_dispatch: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set_interval_ms(&self, ms: u64) {
        *self.interval_ms.write() = ms;
    }

    pub fn interval_ms(&self) -> u64 {
        *self.interval_ms.read()
    }

    pub fn since_last_dispatch_ms(&self, email: &str) -> Option<u64> {
        let key = email.trim().to_lowercase();
        if key.is_empty() {
            return None;
        }
        let map = self.last_dispatch.lock();
        map.get(&key).map(|t| {
            let elapsed = Instant::now().duration_since(*t);
            elapsed.as_millis() as u64
        })
    }

    pub fn mark_dispatch(&self, email: &str) {
        let key = email.trim().to_lowercase();
        if key.is_empty() {
            return;
        }
        self.last_dispatch.lock().insert(key, Instant::now());
    }

    pub fn reconcile_stale(&self, max_age: Duration) {
        let now = Instant::now();
        let mut map = self.last_dispatch.lock();
        map.retain(|_, t| now.duration_since(*t) < max_age);
    }
}

pub struct DispatchMarkGuard<'a> {
    gate: &'a DispatchIntervalGate,
    email: String,
}

impl<'a> DispatchMarkGuard<'a> {
    pub fn new(gate: &'a DispatchIntervalGate, email: &str) -> Self {
        gate.mark_dispatch(email);
        Self {
            gate,
            email: email.to_string(),
        }
    }
}

impl Drop for DispatchMarkGuard<'_> {
    fn drop(&mut self) {
        let _ = (&self.gate, &self.email);
    }
}
