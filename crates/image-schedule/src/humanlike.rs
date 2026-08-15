//! Humanlike account scoring — gptimage `humanlike_scheduler` Rust port.

use chrono::{Local, Timelike};
use rand::Rng;

#[derive(Debug, Clone)]
pub struct AccountScoreInput {
    pub email: String,
    pub quota: i64,
    pub image_quota_unknown: bool,
    pub image_inflight: i64,
    pub soft_band_percent: i64,
    pub binding_inflight: u64,
}

/// Hour-of-day weight (night low, business hours high).
pub fn hour_weight(hour: u8) -> f64 {
    match hour {
        0..=5 => 0.40,
        6..=8 => 0.55,
        9..=11 => 0.85,
        12..=13 => 0.75,
        14..=17 => 0.90,
        18..=20 => 0.70,
        21..=23 => 0.45,
        _ => 0.60,
    }
}

/// Score account for image dispatch (higher = prefer).
pub fn score_account(input: &AccountScoreInput, hour: u8, jitter: f64, workload_mult: f64) -> f64 {
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
    if input.binding_inflight > 0 {
        score /= 1.0 + input.binding_inflight as f64;
    }
    if input.soft_band_percent > 0 {
        let burn = (input.soft_band_percent as f64 / 100.0).clamp(0.0, 0.95);
        score *= (1.0 - burn);
    }
    (score + jitter) * workload_mult
}

/// ε-greedy pick with humanlike scores.
pub fn pick_account_index(
    candidates: &[AccountScoreInput],
    rr_start: usize,
    epsilon: f64,
    workload_mult: f64,
) -> usize {
    if candidates.is_empty() {
        return 0;
    }
    let eps = epsilon.clamp(0.0, 1.0);
    let hour = Local::now().hour() as u8;
    let mut rng = rand::thread_rng();
    if eps > 0.0 && rng.gen::<f64>() < eps {
        return rng.gen_range(0..candidates.len());
    }
    let mut best_idx = rr_start % candidates.len();
    let mut best_score = -1.0;
    for (i, c) in candidates.iter().enumerate() {
        let jitter = rng.gen_range(0.0..0.15);
        let s = score_account(c, hour, jitter, workload_mult);
        if s > best_score {
            best_score = s;
            best_idx = i;
        }
    }
    best_idx
}

pub fn default_epsilon() -> f64 {
    std::env::var("HUMANLIKE_EPSILON")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.12)
        .clamp(0.0, 1.0)
}
