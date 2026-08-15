//! Per-account + numbered sS slot ledger (gptimage `slot_ledger.rs` subset).

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct SlotLedgerConfig {
    pub account_cap: u64,
    pub ss_cap: u64,
    pub slot_ttl: Duration,
}

impl SlotLedgerConfig {
    pub fn from_env() -> Self {
        let account_cap = std::env::var("IMAGE_ACCOUNT_SLOT_CAP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        let ss_cap = std::env::var("IMAGE_SS_SLOT_CAP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);
        let ttl_secs = std::env::var("IMAGE_SLOT_TTL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(600);
        Self {
            account_cap,
            ss_cap,
            slot_ttl: Duration::from_secs(ttl_secs.max(30)),
        }
    }
}

#[derive(Clone)]
pub struct SlotLedger {
    cfg: SlotLedgerConfig,
    accounts: Arc<Mutex<HashMap<String, Instant>>>,
    ss_slots: Arc<Mutex<HashMap<u32, (String, Instant)>>>,
    ss_rr: Arc<Mutex<u32>>,
}

pub type SharedSlotLedger = SlotLedger;

impl SlotLedger {
    pub fn from_env() -> Self {
        Self::new(SlotLedgerConfig::from_env())
    }

    pub fn new(cfg: SlotLedgerConfig) -> Self {
        Self {
            cfg,
            accounts: Arc::new(Mutex::new(HashMap::new())),
            ss_slots: Arc::new(Mutex::new(HashMap::new())),
            ss_rr: Arc::new(Mutex::new(0)),
        }
    }

    pub fn account_inflight_for_token(&self, account_key: &str) -> u64 {
        let key = account_key.trim().to_lowercase();
        if key.is_empty() {
            return 0;
        }
        self.accounts
            .lock()
            .get(&key)
            .map(|_| 1)
            .unwrap_or(0)
    }

    pub fn ss_inflight(&self) -> u64 {
        self.ss_slots.lock().len() as u64
    }

    pub fn try_acquire_account(&self, account_key: &str) -> Option<AccountSlotGuard<'_>> {
        if self.cfg.account_cap == 0 {
            return Some(AccountSlotGuard {
                ledger: self,
                key: String::new(),
                active: false,
            });
        }
        let key = account_key.trim().to_lowercase();
        if key.is_empty() {
            return None;
        }
        let mut map = self.accounts.lock();
        if map.contains_key(&key) {
            return None;
        }
        map.insert(key.clone(), Instant::now());
        Some(AccountSlotGuard {
            ledger: self,
            key,
            active: true,
        })
    }

    pub fn release_account(&self, account_key: &str) {
        let key = account_key.trim().to_lowercase();
        if !key.is_empty() {
            self.accounts.lock().remove(&key);
        }
    }

    pub fn try_acquire_ss(&self, holder_key: &str) -> Option<SsSlotGuard<'_>> {
        if self.cfg.ss_cap == 0 {
            return Some(SsSlotGuard {
                ledger: self,
                slot_index: 0,
                active: false,
            });
        }
        let holder = holder_key.trim();
        if holder.is_empty() {
            return None;
        }
        self.watchdog_tick(false);
        let mut slots = self.ss_slots.lock();
        if slots.len() as u64 >= self.cfg.ss_cap {
            return None;
        }
        let mut rr = self.ss_rr.lock();
        let cap = self.cfg.ss_cap as u32;
        let mut chosen = None;
        for offset in 0..cap {
            let idx = (*rr + offset) % cap;
            if !slots.contains_key(&idx) {
                chosen = Some(idx);
                *rr = (idx + 1) % cap;
                break;
            }
        }
        let slot_index = chosen?;
        slots.insert(slot_index, (holder.to_string(), Instant::now()));
        Some(SsSlotGuard {
            ledger: self,
            slot_index,
            active: true,
        })
    }

    pub fn release_ss(&self, slot_index: u32) {
        self.ss_slots.lock().remove(&slot_index);
    }

    pub fn watchdog_tick(&self, force: bool) {
        let ttl = self.cfg.slot_ttl;
        let now = Instant::now();
        let mut accounts = self.accounts.lock();
        accounts.retain(|_, since| {
            if force {
                false
            } else {
                now.duration_since(*since) < ttl
            }
        });
        let mut ss = self.ss_slots.lock();
        ss.retain(|_, (_, since)| {
            if force {
                false
            } else {
                now.duration_since(*since) < ttl
            }
        });
    }

    pub fn stats_json(&self) -> serde_json::Value {
        serde_json::json!({
            "account_inflight": self.accounts.lock().len(),
            "ss_inflight": self.ss_slots.lock().len(),
            "account_cap": self.cfg.account_cap,
            "ss_cap": self.cfg.ss_cap,
            "slot_ttl_secs": self.cfg.slot_ttl.as_secs(),
        })
    }
}

pub struct AccountSlotGuard<'a> {
    ledger: &'a SlotLedger,
    key: String,
    active: bool,
}

impl Drop for AccountSlotGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            self.ledger.release_account(&self.key);
        }
    }
}

pub struct SsSlotGuard<'a> {
    ledger: &'a SlotLedger,
    slot_index: u32,
    active: bool,
}

impl SsSlotGuard<'_> {
    pub fn slot_index(&self) -> u32 {
        self.slot_index
    }
}

impl Drop for SsSlotGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            self.ledger.release_ss(self.slot_index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_and_ss_slots_exclusive() {
        let ledger = SlotLedger::new(SlotLedgerConfig {
            account_cap: 1,
            ss_cap: 2,
            slot_ttl: Duration::from_secs(60),
        });
        let a1 = ledger.try_acquire_account("a@x.com").expect("account");
        assert!(ledger.try_acquire_account("a@x.com").is_none());
        let s1 = ledger.try_acquire_ss("a@x.com").expect("ss1");
        let s2 = ledger.try_acquire_ss("b@x.com").expect("ss2");
        assert!(ledger.try_acquire_ss("c@x.com").is_none());
        drop(s1);
        drop(s2);
        drop(a1);
        assert!(ledger.try_acquire_account("a@x.com").is_some());
    }
}
