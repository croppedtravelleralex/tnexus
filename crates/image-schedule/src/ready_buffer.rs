//! Ready buffer backpressure — gptimage `ready_buffer.py` subset.

use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Clone)]
pub struct ReadyBuffer {
    max_bytes: u64,
    max_items: u64,
    bytes: Arc<Mutex<u64>>,
    items: Arc<Mutex<u64>>,
}

impl ReadyBuffer {
    pub fn from_env() -> Self {
        let max_bytes = std::env::var("IMAGE_READY_BUFFER_MAX_BYTES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(512 * 1024 * 1024);
        let max_items = std::env::var("IMAGE_READY_BUFFER_MAX_ITEMS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(32);
        Self::new(max_bytes, max_items)
    }

    pub fn new(max_bytes: u64, max_items: u64) -> Self {
        Self {
            max_bytes,
            max_items,
            bytes: Arc::new(Mutex::new(0)),
            items: Arc::new(Mutex::new(0)),
        }
    }

    pub fn try_admit(&self, payload_bytes: u64) -> bool {
        if self.max_items == 0 && self.max_bytes == 0 {
            return true;
        }
        let mut items = self.items.lock();
        let mut bytes = self.bytes.lock();
        if self.max_items > 0 && *items >= self.max_items {
            return false;
        }
        if self.max_bytes > 0 && *bytes + payload_bytes > self.max_bytes {
            return false;
        }
        *items += 1;
        *bytes += payload_bytes;
        true
    }

    pub fn release(&self, payload_bytes: u64) {
        let mut items = self.items.lock();
        let mut bytes = self.bytes.lock();
        *items = items.saturating_sub(1);
        *bytes = bytes.saturating_sub(payload_bytes);
    }

    pub fn stats(&self) -> (u64, u64) {
        (*self.bytes.lock(), *self.items.lock())
    }
}

pub struct ReadyBufferGuard<'a> {
    buffer: &'a ReadyBuffer,
    bytes: u64,
    admitted: bool,
}

impl<'a> ReadyBufferGuard<'a> {
    pub fn try_acquire(buffer: &'a ReadyBuffer, payload_bytes: u64) -> Option<Self> {
        if buffer.try_admit(payload_bytes) {
            Some(Self {
                buffer,
                bytes: payload_bytes,
                admitted: true,
            })
        } else {
            None
        }
    }
}

impl Drop for ReadyBufferGuard<'_> {
    fn drop(&mut self) {
        if self.admitted {
            self.buffer.release(self.bytes);
        }
    }
}
