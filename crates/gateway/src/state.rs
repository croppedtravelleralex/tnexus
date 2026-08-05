//! Shared gateway application state.

use crate::config::DataPlane;
use crate::duplicate_prompt::DuplicatePromptGate;
use crate::image_assets::ImageAssetStore;
use crate::scheduling_gate::SchedulingGate;
use gateway_auth::AuthService;
use helper_client::{HelperClient, PinAccount};
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
    pub image_enabled: bool,
    pub auth: Arc<AuthService>,
    pub static_dir: Option<PathBuf>,
    pub image_assets: Arc<ImageAssetStore>,
    pub public_base_url: String,
    pub scheduling_gate: SchedulingGate,
    pub image_account_rr: AtomicUsize,
    pub duplicate_prompt: DuplicatePromptGate,
    pub pg_pool: Option<sqlx::PgPool>,
    pub image_archive_store: Option<tnexus_storage::SharedImageStore>,
}

impl AppState {
    pub fn image_queue_depth(&self) -> usize {
        self.image_queue_depth.load(Ordering::Relaxed)
    }

    pub fn estimated_image_wait_secs(&self) -> u32 {
        let depth = self.image_queue_depth();
        let inflight = self
            .image_global_concurrency
            .saturating_sub(self.image_sem.available_permits());
        let busy = inflight.max(depth);
        if busy == 0 {
            return 5;
        }
        let avg_secs = 45u32;
        let cap = self.image_global_concurrency.max(1) as u32;
        ((busy as u32 * avg_secs) / cap).clamp(5, 180)
    }
}
