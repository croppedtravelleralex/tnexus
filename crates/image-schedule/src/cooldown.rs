//! Account cooldown registry — gptimage 429/终态冷却子集.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct CooldownRegistry {
    rate_limit_secs: u64,
    terminal_secs: u64,
    until: Arc<Mutex<HashMap<String, Instant>>>,
}

impl CooldownRegistry {
    pub fn from_env() -> Self {
        let rate_limit_secs = std::env::var("IMAGE_COOLDOWN_RATE_LIMIT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(900);
        let terminal_secs = std::env::var("IMAGE_COOLDOWN_TERMINAL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1800);
        Self::new(rate_limit_secs, terminal_secs)
    }

    pub fn new(rate_limit_secs: u64, terminal_secs: u64) -> Self {
        Self {
            rate_limit_secs,
            terminal_secs,
            until: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn is_blocked(&self, email: &str) -> bool {
        let key = email.trim().to_lowercase();
        if key.is_empty() {
            return false;
        }
        let now = Instant::now();
        let mut map = self.until.lock();
        map.retain(|_, t| *t > now);
        map.get(&key).is_some_and(|t| *t > now)
    }

    pub fn record_rate_limit(&self, email: &str) {
        self.record(email, self.rate_limit_secs);
    }

    pub fn record_terminal(&self, email: &str) {
        self.record(email, self.terminal_secs);
    }

    fn record(&self, email: &str, secs: u64) {
        let key = email.trim().to_lowercase();
        if key.is_empty() || secs == 0 {
            return;
        }
        let until = Instant::now() + Duration::from_secs(secs);
        let mut map = self.until.lock();
        let entry = map.entry(key).or_insert(until);
        if until > *entry {
            *entry = until;
        }
    }

    pub fn reconcile(&self) {
        let now = Instant::now();
        self.until.lock().retain(|_, t| *t > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_blocks_until_expiry() {
        let reg = CooldownRegistry::new(2, 10);
        reg.record_rate_limit("a@x.com");
        assert!(reg.is_blocked("a@x.com"));
    }
}
