//! Workload shaping — Poisson jitter + IMAGE/TEXT/IDLE route (humanlike_scheduler subset).

use chrono::{Local, Timelike};
use rand::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadRoute {
    Image,
    Text,
    Idle,
}

#[derive(Clone, Debug)]
pub struct WorkloadPolicy {
    pub poisson_lambda: f64,
    pub image_weight_boost: f64,
    pub image_reserve_pct: f64,
}

impl WorkloadPolicy {
    pub fn from_env() -> Self {
        let poisson_lambda = std::env::var("IMAGE_POISSON_LAMBDA")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(8.0)
            .clamp(0.0, 64.0);
        let image_weight_boost = std::env::var("IMAGE_WORKLOAD_RIMG_BOOST")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.15)
            .clamp(0.0, 2.0);
        let image_reserve_pct = std::env::var("IMAGE_WORKLOAD_IMAGE_RESERVE_PCT")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.70)
            .clamp(0.0, 1.0);
        Self {
            poisson_lambda,
            image_weight_boost,
            image_reserve_pct,
        }
    }

    pub fn image_score_multiplier(&self) -> f64 {
        1.0 + self.image_weight_boost
    }

    pub fn route_for_hour(&self, hour: u8) -> WorkloadRoute {
        match hour {
            0..=5 | 22..=23 => WorkloadRoute::Idle,
            9..=11 | 14..=17 => WorkloadRoute::Image,
            _ => WorkloadRoute::Text,
        }
    }

    pub fn current_route(&self) -> WorkloadRoute {
        let hour = Local::now().hour() as u8;
        self.route_for_hour(hour)
    }

    /// Effective global concurrency cap after workload routing.
    pub fn effective_global_cap(&self, base: usize, reserve_pct: Option<f64>) -> usize {
        let reserve = reserve_pct.unwrap_or(self.image_reserve_pct);
        let route = self.current_route();
        let factor = match route {
            WorkloadRoute::Image => 1.0,
            WorkloadRoute::Text => 1.0 - reserve * 0.5,
            WorkloadRoute::Idle => 1.0 - reserve,
        };
        ((base as f64) * factor).round().max(1.0) as usize
    }

    /// Whether an account should participate in image dispatch under current route.
    pub fn account_eligible_for_image(&self, image_inflight: i64, text_heavy: bool) -> bool {
        match self.current_route() {
            WorkloadRoute::Image => true,
            WorkloadRoute::Text => !text_heavy || image_inflight == 0,
            WorkloadRoute::Idle => image_inflight == 0 && !text_heavy,
        }
    }
}
/// Poisson-distributed delay in milliseconds for dispatch spacing.
pub fn poisson_delay_ms(lambda: f64) -> u64 {
    if lambda <= 0.0 {
        return 0;
    }
    let mut rng = rand::thread_rng();
    let u: f64 = rng.gen_range(0.0001..1.0);
    let delay_secs = -ln(u) / lambda;
    (delay_secs * 1000.0).round().clamp(0.0, 30_000.0) as u64
}

fn ln(x: f64) -> f64 {
    x.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisson_delay_bounded() {
        let d = poisson_delay_ms(8.0);
        assert!(d <= 30_000);
    }
}
