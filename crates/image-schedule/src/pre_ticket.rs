//! Per-account credential bundle cache — gptimage `pre_ticket_pool.py` subset.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct PreTicketPool {
    ttl: Duration,
    entries: Arc<Mutex<HashMap<String, (Instant, String)>>>,
}

impl PreTicketPool {
    pub fn from_env() -> Self {
        let ttl_secs = std::env::var("IMAGE_PRE_TICKET_TTL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(120);
        Self::new(Duration::from_secs(ttl_secs.max(30)))
    }

    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let k = key.trim().to_lowercase();
        if k.is_empty() {
            return None;
        }
        let now = Instant::now();
        let mut map = self.entries.lock();
        map.retain(|_, (exp, _)| *exp > now);
        map.get(&k).and_then(|(exp, v)| if *exp > now { Some(v.clone()) } else { None })
    }

    pub fn put(&self, key: &str, ticket: String) {
        let k = key.trim().to_lowercase();
        if k.is_empty() || ticket.trim().is_empty() {
            return;
        }
        let exp = Instant::now() + self.ttl;
        self.entries.lock().insert(k, (exp, ticket));
    }

    pub fn invalidate(&self, key: &str) {
        let k = key.trim().to_lowercase();
        if !k.is_empty() {
            self.entries.lock().remove(&k);
        }
    }

    pub fn stats(&self) -> usize {
        let now = Instant::now();
        let mut map = self.entries.lock();
        map.retain(|_, (exp, _)| *exp > now);
        map.len()
    }
}
