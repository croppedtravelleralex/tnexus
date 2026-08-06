use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use crate::{AssetStorage, LocalAssetStorage, R2Config, StoredAsset};

pub type SharedImageStore = Arc<ImageStore>;

#[derive(Clone)]
pub enum ImageStoreBackend {
    R2(AssetStorage),
    Local(LocalAssetStorage),
}

#[derive(Clone)]
pub struct ImageStore {
    backend: ImageStoreBackend,
}

impl ImageStore {
    pub async fn from_r2_config(cfg: &R2Config) -> Result<Self> {
        Ok(Self {
            backend: ImageStoreBackend::R2(AssetStorage::from_config(cfg).await?),
        })
    }

    pub fn from_local_path(path: PathBuf) -> Result<Self> {
        Ok(Self {
            backend: ImageStoreBackend::Local(LocalAssetStorage::new(path)?),
        })
    }

    /// R2 when `R2_BUCKET` is set; otherwise local disk at `IMAGE_STORE_PATH` (default `/data/images`).
    pub async fn from_env() -> Result<Option<Self>> {
        if std::env::var("R2_BUCKET")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
        {
            let cfg = R2Config {
                account_id: std::env::var("R2_ACCOUNT_ID").context("R2_ACCOUNT_ID")?,
                access_key_id: std::env::var("R2_ACCESS_KEY_ID").context("R2_ACCESS_KEY_ID")?,
                secret_access_key: std::env::var("R2_SECRET_ACCESS_KEY")
                    .context("R2_SECRET_ACCESS_KEY")?,
                bucket: std::env::var("R2_BUCKET").context("R2_BUCKET")?,
                endpoint: std::env::var("R2_ENDPOINT").ok(),
            };
            return Ok(Some(Self::from_r2_config(&cfg).await?));
        }

        let path = std::env::var("IMAGE_STORE_PATH").unwrap_or_else(|_| "/data/images".into());
        if path.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(Self::from_local_path(PathBuf::from(path))?))
    }

    pub fn backend_name(&self) -> &'static str {
        match &self.backend {
            ImageStoreBackend::R2(_) => "r2",
            ImageStoreBackend::Local(_) => "local",
        }
    }

    pub fn uses_remote_urls(&self) -> bool {
        matches!(self.backend, ImageStoreBackend::R2(_))
    }

    pub async fn store_image_variants(
        &self,
        user_id: Uuid,
        job_id: Uuid,
        image_bytes: &[u8],
    ) -> Result<StoredAsset> {
        match &self.backend {
            ImageStoreBackend::R2(s) => s.store_image_variants(user_id, job_id, image_bytes).await,
            ImageStoreBackend::Local(s) => {
                s.store_image_variants(user_id, job_id, image_bytes).await
            }
        }
    }

    pub async fn read_bytes(&self, key: &str) -> Result<Vec<u8>> {
        match &self.backend {
            ImageStoreBackend::R2(s) => {
                let url = s
                    .presign_get(key, 300, false)
                    .await
                    .context("presign for read")?;
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(120))
                    .build()
                    .context("http client")?;
                let bytes = client
                    .get(&url)
                    .send()
                    .await
                    .context("fetch r2 object")?
                    .bytes()
                    .await
                    .context("read r2 body")?;
                Ok(bytes.to_vec())
            }
            ImageStoreBackend::Local(s) => s.read_bytes(key).await,
        }
    }

    pub async fn presign_get(&self, key: &str, ttl_secs: u64, download: bool) -> Result<String> {
        match &self.backend {
            ImageStoreBackend::R2(s) => s.presign_get(key, ttl_secs, download).await,
            ImageStoreBackend::Local(_) => {
                anyhow::bail!("local image store does not support presigned URLs")
            }
        }
    }
}
