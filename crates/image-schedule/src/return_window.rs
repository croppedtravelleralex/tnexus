//! Return window — bounded concurrent large payload returns.

use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Clone)]
pub struct ReturnWindow {
    max: u64,
    inflight: Arc<Mutex<u64>>,
}

impl ReturnWindow {
    pub fn from_env() -> Self {
        let max = std::env::var("IMAGE_RETURN_WINDOW_MAX")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8);
        Self::new(max)
    }

    pub fn new(max: u64) -> Self {
        Self {
            max,
            inflight: Arc::new(Mutex::new(0)),
        }
    }

    pub fn try_acquire(&self) -> Option<ReturnWindowPermit<'_>> {
        if self.max == 0 {
            return Some(ReturnWindowPermit {
                window: self,
                active: false,
            });
        }
        let mut n = self.inflight.lock();
        if *n >= self.max {
            return None;
        }
        *n += 1;
        Some(ReturnWindowPermit {
            window: self,
            active: true,
        })
    }

    fn release(&self) {
        let mut n = self.inflight.lock();
        *n = n.saturating_sub(1);
    }

    pub fn inflight(&self) -> u64 {
        *self.inflight.lock()
    }
}

pub struct ReturnWindowPermit<'a> {
    window: &'a ReturnWindow,
    active: bool,
}

impl Drop for ReturnWindowPermit<'_> {
    fn drop(&mut self) {
        if self.active {
            self.window.release();
        }
    }
}
