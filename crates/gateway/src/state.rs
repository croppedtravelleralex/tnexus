//! Shared gateway application state.

use crate::config::DataPlane;
use crate::duplicate_prompt::DuplicatePromptGate;
use crate::image_assets::ImageAssetStore;
use crate::image_tasks::ImageTaskService;
use crate::scheduling_gate::SchedulingGate;
use gateway_auth::AuthService;
use helper_client::{HelperClient, PinAccount};
use image_schedule::{
    BindingInflightLedger, CooldownRegistry, DeadlockGuard, DispatchIntervalGate, ImageRuntimeConfig,
    PipelineWatchdog, PreTicketPool, ProxyCfRegistry, ReadyBuffer, ReturnWindow, SharedSlotLedger,
    WorkloadPolicy,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

pub struct AppState {
    pub helper: HelperClient,
    pub data_plane: DataPlane,
    pub pin: PinAccount,
    pub accounts: Arc<Mutex<HashMap<String, PinAccount>>>,
    pub listen: String,
    pub min_image_quota: i64,
    pub image_global_concurrency: usize,
    pub image_sem: Arc<Semaphore>,
    /// Waiters blocked on global image semaphore (admission queue depth).
    pub image_queue_depth: AtomicUsize,
    /// Base capability from IMAGE_ENABLED env (hot pause layered via `image_runtime`).
    pub image_enabled: bool,
    pub image_runtime: ImageRuntimeConfig,
    pub deadlock_guard: DeadlockGuard,
    pub pipeline_watchdog: PipelineWatchdog,
    pub auth: Arc<AuthService>,
    pub static_dir: Option<PathBuf>,
    pub image_assets: Arc<ImageAssetStore>,
    pub public_base_url: String,
    pub scheduling_gate: SchedulingGate,
    pub image_account_rr: AtomicUsize,
    pub duplicate_prompt: DuplicatePromptGate,
    pub binding_inflight: BindingInflightLedger,
    pub dispatch_interval: DispatchIntervalGate,
    pub slot_ledger: SharedSlotLedger,
    pub ready_buffer: ReadyBuffer,
    pub return_window: ReturnWindow,
    pub cooldown: CooldownRegistry,
    pub pre_ticket: PreTicketPool,
    pub proxy_cf: ProxyCfRegistry,
    pub workload: WorkloadPolicy,
    pub image_tasks: ImageTaskService,
    pub pg_pool: Option<sqlx::PgPool>,
    pub image_archive_store: Option<tnexus_storage::SharedImageStore>,
}

impl AppState {
    pub fn image_generation_allowed(&self) -> bool {
        self.image_runtime.is_generation_allowed()
    }

    pub fn image_queue_depth(&self) -> usize {
        self.image_queue_depth.load(Ordering::Relaxed)
    }

    pub fn effective_global_concurrency(&self) -> usize {
        let base = self
            .image_runtime
            .effective_global_concurrency(self.image_global_concurrency);
        let reserve = self
            .image_runtime
            .workload_image_reserve_pct(self.workload.image_reserve_pct);
        self.workload.effective_global_cap(base, Some(reserve))
    }

    pub fn image_global_busy(&self) -> usize {
        let inflight = self
            .image_global_concurrency
            .saturating_sub(self.image_sem.available_permits());
        inflight.max(self.image_queue_depth())
    }

    pub fn estimated_image_wait_secs(&self) -> u32 {
        let depth = self.image_queue_depth();
        let global_busy = self.image_global_busy();
        let task_queued = self.image_tasks.store.snapshot_len() as usize;
        let (_, ready_items) = self.ready_buffer.stats();
        let return_busy = self.return_window.inflight() as usize;
        let busy = global_busy
            .max(depth)
            .max(task_queued / 2)
            .max(return_busy)
            .max(ready_items as usize);
        if busy == 0 {
            return 5;
        }
        let avg_secs = self.image_runtime.estimated_slot_secs(
            std::env::var("IMAGE_ESTIMATED_SLOT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(45),
        );
        let cap = self.effective_global_concurrency().max(1) as u32;
        ((busy as u32 * avg_secs) / cap).clamp(5, 180)
    }

    pub fn pipeline_snapshot(&self) -> serde_json::Value {
        let (ready_bytes, ready_items) = self.ready_buffer.stats();
        let (queued, running) = self.image_tasks.store.count_states();
        serde_json::json!({
            "image_runtime": self.image_runtime.snapshot(),
            "deadlock_guard": self.deadlock_guard.stats_json(),
            "pipeline_watchdog": self.pipeline_watchdog.stats_json(queued, running),
            "slot_ledger": self.slot_ledger.stats_json(),
            "ready_buffer": {
                "bytes": ready_bytes,
                "items": ready_items,
            },
            "return_window_inflight": self.return_window.inflight(),
            "workload_route": format!("{:?}", self.workload.current_route()),
            "effective_global_concurrency": self.effective_global_concurrency(),
            "image_global_busy": self.image_global_busy(),
            "image_tasks": self.image_tasks.store.snapshot_len(),
            "image_tasks_queued": queued,
            "image_tasks_running": running,
            "pre_ticket_cached": self.pre_ticket.stats(),
            "proxy_cf_blocked_bindings": self.proxy_cf.blocked_count(),
        })
    }
}
