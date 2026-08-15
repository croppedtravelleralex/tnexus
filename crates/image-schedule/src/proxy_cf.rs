//! Proxy / CF403 binding isolation — gptimage proxy sticky subset.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct ProxyCfRegistry {
    cooldown_secs: u64,
    blocked: Arc<Mutex<HashMap<String, Instant>>>,
}

impl ProxyCfRegistry {
    pub fn from_env() -> Self {
        let cooldown_secs = std::env::var("IMAGE_CF_BINDING_COOLDOWN_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(900);
        Self::new(cooldown_secs)
    }

    pub fn new(cooldown_secs: u64) -> Self {
        Self {
            cooldown_secs: cooldown_secs.max(60),
            blocked: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn is_blocked(&self, binding_key: &str) -> bool {
        let key = binding_key.trim();
        if key.is_empty() {
            return false;
        }
        let now = Instant::now();
        let mut map = self.blocked.lock();
        map.retain(|_, until| *until > now);
        map.get(key).is_some_and(|until| *until > now)
    }

    pub fn record_cf403(&self, binding_key: &str) {
        let key = binding_key.trim();
        if key.is_empty() {
            return;
        }
        let until = Instant::now() + Duration::from_secs(self.cooldown_secs);
        let mut map = self.blocked.lock();
        let entry = map.entry(key.to_string()).or_insert(until);
        if until > *entry {
            *entry = until;
        }
    }

    pub fn record_from_error(&self, binding_key: &str, err: &str) {
        let lower = err.to_lowercase();
        if lower.contains("cloudflare")
            || lower.contains("cf-ray")
            || lower.contains("403 forbidden")
            || lower.contains("cf_chl")
        {
            self.record_cf403(binding_key);
        }
    }

    pub fn reconcile(&self) {
        let now = Instant::now();
        self.blocked.lock().retain(|_, until| *until > now);
    }

    pub fn blocked_count(&self) -> usize {
        self.reconcile();
        self.blocked.lock().len()
    }
}
