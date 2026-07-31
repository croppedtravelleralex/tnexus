//! Shared gateway application state.

use crate::config::DataPlane;
use crate::image_assets::ImageAssetStore;
use gateway_auth::AuthService;
use helper_client::{HelperClient, PinAccount};
use std::collections::HashMap;
use std::path::PathBuf;
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
    pub image_enabled: bool,
    pub auth: Arc<AuthService>,
    pub static_dir: Option<PathBuf>,
    pub image_assets: Arc<ImageAssetStore>,
    pub public_base_url: String,
}
