//! Interval dispatch gate — mirrors gptimage `image_schedule_core::dispatch_gate`.
//!
//! Used to defer new work when inflight is at cap or a prior slot is still draining.

/// Returns true when callers should wait before dispatching another image task.
pub fn should_wait(interval_ms: u64, inflight: u64, cap: u64, queued: u64) -> bool {
    if cap == 0 {
        return false;
    }
    if inflight >= cap {
        return true;
    }
    if interval_ms == 0 {
        return false;
    }
    queued > 0 && inflight > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cap_never_waits() {
        assert!(!should_wait(1000, 5, 0, 10));
    }

    #[test]
    fn inflight_at_cap_waits() {
        assert!(should_wait(0, 4, 4, 0));
    }

    #[test]
    fn interval_gate_with_queue() {
        assert!(should_wait(500, 1, 4, 2));
        assert!(!should_wait(500, 0, 4, 2));
        assert!(!should_wait(0, 1, 4, 2));
    }
}
