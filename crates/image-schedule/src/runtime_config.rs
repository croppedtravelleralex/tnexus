//! Hot-reloadable image runtime — gptimage `ConfigStore` scheduling keys.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ImageRuntimeSnapshot {
    #[serde(default)]
    pub image_generation_paused: bool,
    #[serde(default)]
    pub image_global_concurrency: Option<usize>,
    #[serde(default)]
    pub image_account_inflight_max: Option<u64>,
    #[serde(default)]
    pub image_binding_inflight_max: Option<u64>,
    #[serde(default)]
    pub image_dispatch_interval_ms: Option<u64>,
    #[serde(default)]
    pub image_estimated_slot_secs: Option<u32>,
    #[serde(default)]
    pub image_workload_image_reserve_pct: Option<f64>,
    #[serde(default)]
    pub image_ss_slot_cap: Option<u64>,
    #[serde(default)]
    pub image_account_slot_cap: Option<u64>,
    #[serde(default)]
    pub image_slot_ttl_secs: Option<u64>,
    #[serde(default)]
    pub image_ready_buffer_max_bytes: Option<u64>,
    #[serde(default)]
    pub image_ready_buffer_max_items: Option<u64>,
    #[serde(default)]
    pub image_return_window_max: Option<u64>,
    #[serde(default)]
    pub image_cooldown_rate_limit_secs: Option<u64>,
    #[serde(default)]
    pub image_cooldown_terminal_secs: Option<u64>,
    #[serde(default)]
    pub humanlike_epsilon: Option<f64>,
    #[serde(default)]
    pub image_deadlock_cpu_trip_pct: Option<f64>,
    #[serde(default)]
    pub image_pipeline_stall_secs: Option<u64>,
}

impl ImageRuntimeSnapshot {
    pub fn normalize(&mut self) {
        if let Some(v) = self.image_global_concurrency {
            self.image_global_concurrency = Some(v.clamp(1, 128));
        }
        if let Some(v) = self.image_binding_inflight_max {
            self.image_binding_inflight_max = Some(v.min(16));
        }
        if let Some(v) = self.image_workload_image_reserve_pct {
            self.image_workload_image_reserve_pct = Some(v.clamp(0.0, 1.0));
        }
        if let Some(v) = self.humanlike_epsilon {
            self.humanlike_epsilon = Some(v.clamp(0.0, 1.0));
        }
        if let Some(v) = self.image_deadlock_cpu_trip_pct {
            self.image_deadlock_cpu_trip_pct = Some(v.clamp(50.0, 100.0));
        }
    }
}

#[derive(Clone)]
pub struct ImageRuntimeConfig {
    path: PathBuf,
    inner: Arc<RwLock<ImageRuntimeSnapshot>>,
    poll_secs: u64,
    env_enabled: bool,
}

impl ImageRuntimeConfig {
    pub fn from_env(base_image_enabled: bool) -> Self {
        let path = std::env::var("IMAGE_RUNTIME_CONFIG_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data/pool/image_runtime.json"));
        let poll_secs = std::env::var("IMAGE_RUNTIME_POLL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);
        let cfg = Self {
            path,
            inner: Arc::new(RwLock::new(ImageRuntimeSnapshot::default())),
            poll_secs: poll_secs.max(5),
            env_enabled: base_image_enabled,
        };
        cfg.reload();
        cfg
    }

    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_secs)
    }

    pub fn reload(&self) {
        let mut snap = if self.path.exists() {
            fs::read_to_string(&self.path)
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default()
        } else {
            ImageRuntimeSnapshot::default()
        };
        if let Ok(v) = std::env::var("IMAGE_GENERATION_PAUSED") {
            if v == "1" || v.eq_ignore_ascii_case("true") {
                snap.image_generation_paused = true;
            }
        }
        snap.normalize();
        *self.inner.write() = snap;
    }

    pub fn snapshot(&self) -> ImageRuntimeSnapshot {
        self.inner.read().clone()
    }

    pub fn is_generation_allowed(&self) -> bool {
        self.env_enabled && !self.inner.read().image_generation_paused
    }

    pub fn is_paused(&self) -> bool {
        self.inner.read().image_generation_paused
    }

    pub fn effective_global_concurrency(&self, default: usize) -> usize {
        self.inner
            .read()
            .image_global_concurrency
            .unwrap_or(default)
            .max(1)
    }

    pub fn estimated_slot_secs(&self, default: u32) -> u32 {
        self.inner
            .read()
            .image_estimated_slot_secs
            .unwrap_or(default)
            .clamp(5, 300)
    }

    pub fn workload_image_reserve_pct(&self, default: f64) -> f64 {
        self.inner
            .read()
            .image_workload_image_reserve_pct
            .unwrap_or(default)
            .clamp(0.0, 1.0)
    }

    pub fn humanlike_epsilon(&self, default: f64) -> f64 {
        self.inner
            .read()
            .humanlike_epsilon
            .unwrap_or(default)
            .clamp(0.0, 1.0)
    }

    pub fn binding_inflight_max(&self, default: u64) -> u64 {
        self.inner
            .read()
            .image_binding_inflight_max
            .unwrap_or(default)
    }

    pub fn dispatch_interval_ms(&self, default: u64) -> u64 {
        self.inner
            .read()
            .image_dispatch_interval_ms
            .unwrap_or(default)
    }
}
