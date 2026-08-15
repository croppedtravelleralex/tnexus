//! Per-binding inflight cap — mirrors gptimage `image_binding_inflight_max`.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct BindingInflightLedger {
    max: Arc<parking_lot::RwLock<u64>>,
    counts: Arc<Mutex<HashMap<String, u64>>>,
}

impl BindingInflightLedger {
    pub fn from_env() -> Self {
        let max = std::env::var("IMAGE_BINDING_INFLIGHT_MAX")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1);
        Self::new(max)
    }

    pub fn new(max: u64) -> Self {
        Self {
            max: Arc::new(parking_lot::RwLock::new(max)),
            counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set_max(&self, max: u64) {
        *self.max.write() = max;
    }

    pub fn cap(&self) -> u64 {
        *self.max.read()
    }

    pub fn inflight(&self, binding_key: &str) -> u64 {
        let key = binding_key.trim();
        if key.is_empty() {
            return 0;
        }
        self.counts
            .lock()
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// Returns false when binding is already at cap.
    pub fn is_available(&self, binding_key: &str) -> bool {
        let max = self.cap();
        if max == 0 {
            return true;
        }
        let key = binding_key.trim();
        if key.is_empty() {
            return true;
        }
        self.inflight(key) < max
    }

    pub fn try_begin<'a>(&'a self, binding_key: &str) -> Option<BindingInflightGuard<'a>> {
        let max = self.cap();
        if max == 0 {
            return Some(BindingInflightGuard {
                ledger: self,
                key: binding_key.trim().to_string(),
                active: false,
            });
        }
        let key = binding_key.trim();
        if key.is_empty() {
            return Some(BindingInflightGuard {
                ledger: self,
                key: String::new(),
                active: false,
            });
        }
        let mut map = self.counts.lock();
        let entry = map.entry(key.to_string()).or_insert(0);
        if *entry >= max {
            return None;
        }
        *entry += 1;
        Some(BindingInflightGuard {
            ledger: self,
            key: key.to_string(),
            active: true,
        })
    }

    pub fn reconcile_above(&self, ceiling: u64) -> usize {
        let mut map = self.counts.lock();
        let mut reset = 0usize;
        for (k, v) in map.iter_mut() {
            if *v > ceiling {
                *v = 0;
                reset += 1;
                let _ = k;
            }
        }
        reset
    }
}

pub struct BindingInflightGuard<'a> {
    ledger: &'a BindingInflightLedger,
    key: String,
    active: bool,
}

impl Drop for BindingInflightGuard<'_> {
    fn drop(&mut self) {
        if !self.active || self.key.is_empty() {
            return;
        }
        let mut map = self.ledger.counts.lock();
        if let Some(v) = map.get_mut(&self.key) {
            *v = v.saturating_sub(1);
            if *v == 0 {
                map.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_cap_blocks_second_acquire() {
        let ledger = BindingInflightLedger::new(1);
        let g1 = ledger.try_begin("proxy:1.2.3.4:30000").expect("first");
        assert!(!ledger.is_available("proxy:1.2.3.4:30000"));
        assert!(ledger.try_begin("proxy:1.2.3.4:30000").is_none());
        drop(g1);
        assert!(ledger.is_available("proxy:1.2.3.4:30000"));
    }
}
