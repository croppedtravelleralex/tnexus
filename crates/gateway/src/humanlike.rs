//! Humanlike account scoring — simplified port of gptimage `humanlike_scheduler`.

use chrono::{Local, Timelike};
use rand::Rng;

/// Hour-of-day weight (0.0–1.0): lower at night, higher in business hours.
pub fn hour_weight(hour: u8) -> f64 {
    match hour {
        0..=5 => 0.35,
        6..=8 => 0.55,
        9..=11 => 0.85,
        12..=13 => 0.75,
        14..=17 => 0.90,
        18..=20 => 0.70,
        21..=23 => 0.45,
        _ => 0.6,
    }
}

#[derive(Debug, Clone)]
pub struct AccountScoreInput {
    pub email: String,
    pub quota: i64,
    pub image_quota_unknown: bool,
    pub image_inflight: i64,
    pub soft_band_percent: i64,
}

/// Score account for image dispatch (higher = prefer).
pub fn score_account(input: &AccountScoreInput, hour: u8, jitter: f64) -> f64 {
    let mut score = hour_weight(hour);
    if input.image_quota_unknown {
        score += 2.0;
    } else if input.quota > 0 {
        score += (input.quota as f64).min(50.0) / 25.0;
    } else {
        score *= 0.1;
    }
    if input.image_inflight > 0 {
        score /= 1.0 + input.image_inflight as f64;
    }
    if input.soft_band_percent > 0 {
        score *= 0.5;
    }
    score + jitter
}

/// Pick index into `candidates` using humanlike scores; falls back to `rr_start % len`.
pub fn pick_account_index(
    candidates: &[AccountScoreInput],
    rr_start: usize,
) -> usize {
    if candidates.is_empty() {
        return 0;
    }
    let hour = Local::now().hour() as u8;
    let mut rng = rand::thread_rng();
    let mut best_idx = rr_start % candidates.len();
    let mut best_score = -1.0;
    for (i, c) in candidates.iter().enumerate() {
        let jitter = rng.gen_range(0.0..0.15);
        let s = score_account(c, hour, jitter);
        if s > best_score {
            best_score = s;
            best_idx = i;
        }
    }
    best_idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_weight_lower_than_peak() {
        assert!(hour_weight(3) < hour_weight(10));
    }
}
