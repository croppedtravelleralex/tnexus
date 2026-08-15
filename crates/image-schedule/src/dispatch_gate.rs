//! Interval dispatch gate — mirrors gptimage `image_schedule_core::dispatch_gate`.

/// Returns true when callers should defer dispatch (respond 429, do not queue HTTP).
pub fn should_wait(
    interval_ms: u64,
    inflight: u64,
    cap: u64,
    queued: u64,
    since_last_dispatch_ms: Option<u64>,
) -> bool {
    if cap > 0 && inflight >= cap {
        return true;
    }
    if interval_ms == 0 {
        return false;
    }
    if let Some(elapsed) = since_last_dispatch_ms {
        if elapsed < interval_ms {
            return true;
        }
    }
    queued > 0 && inflight > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflight_at_cap_waits() {
        assert!(should_wait(0, 4, 4, 0, None));
    }

    #[test]
    fn interval_blocks_until_elapsed() {
        assert!(should_wait(800, 0, 4, 0, Some(100)));
        assert!(!should_wait(800, 0, 4, 0, Some(900)));
    }
}
