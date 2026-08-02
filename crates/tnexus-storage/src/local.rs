use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::{generate_variants, StoredAsset};

#[derive(Clone)]
pub struct LocalAssetStorage {
    root: PathBuf,
}

impl LocalAssetStorage {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).with_context(|| format!("create image store {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn store_image_variants(
        &self,
        user_id: Uuid,
        job_id: Uuid,
        image_bytes: &[u8],
    ) -> Result<StoredAsset> {
        let asset_id = Uuid::new_v4();
        let base = format!("{user_id}/{job_id}");
        let original_key = format!("{base}/original/{asset_id}.png");
        let preview_key = format!("{base}/preview/{asset_id}.webp");
        let thumb_key = format!("{base}/thumb/{asset_id}.webp");

        let (preview_bytes, thumb_bytes) = generate_variants(image_bytes)?;

        self.put_bytes(&original_key, image_bytes, "image/png").await?;
        self.put_bytes(&preview_key, &preview_bytes, "image/webp").await?;
        self.put_bytes(&thumb_key, &thumb_bytes, "image/webp").await?;

        Ok(StoredAsset {
            original_key,
            preview_key,
            thumb_key,
        })
    }

    pub async fn read_bytes(&self, key: &str) -> Result<Vec<u8>> {
        let path = self.path_for_key(key);
        tokio::fs::read(&path)
            .await
            .with_context(|| format!("read image file {path}"))
    }

    pub async fn put_bytes(&self, key: &str, bytes: &[u8], _content_type: &str) -> Result<()> {
        let path = self.path_for_key(key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        tokio::fs::write(&path, bytes)
            .await
            .with_context(|| format!("write image file {}", path.display()))?;
        Ok(())
    }

    fn path_for_key(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }
}
