use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use image::imageops::FilterType;
use image::{GenericImageView, ImageFormat};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone)]
pub struct R2Config {
    pub account_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
    pub endpoint: Option<String>,
}

#[derive(Clone)]
pub struct AssetStorage {
    client: Client,
    bucket: String,
}

#[derive(Debug, Clone)]
pub struct StoredAsset {
    pub original_key: String,
    pub preview_key: String,
    pub thumb_key: String,
}

impl AssetStorage {
    pub async fn from_config(cfg: &R2Config) -> Result<Self> {
        let endpoint = cfg.endpoint.clone().unwrap_or_else(|| {
            format!(
                "https://{}.r2.cloudflarestorage.com",
                cfg.account_id.trim()
            )
        });
        let credentials = Credentials::new(
            cfg.access_key_id.trim(),
            cfg.secret_access_key.trim(),
            None,
            None,
            "tnexus",
        );
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new("auto"))
            .credentials_provider(credentials)
            .endpoint_url(endpoint)
            .load()
            .await;
        let client = Client::new(&sdk_config);
        Ok(Self {
            client,
            bucket: cfg.bucket.clone(),
        })
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

        self.put_bytes(&original_key, image_bytes, "image/png")
            .await?;
        self.put_bytes(&preview_key, &preview_bytes, "image/webp")
            .await?;
        self.put_bytes(&thumb_key, &thumb_bytes, "image/webp")
            .await?;

        Ok(StoredAsset {
            original_key,
            preview_key,
            thumb_key,
        })
    }

    pub async fn presign_get(&self, key: &str, ttl_secs: u64, download: bool) -> Result<String> {
        let mut req = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key);
        if download {
            let filename = key.rsplit('/').next().unwrap_or("image.png");
            req = req.response_content_disposition(format!(
                "attachment; filename=\"{filename}\""
            ));
        }
        let presigned = req
            .presigned(
                PresigningConfig::expires_in(Duration::from_secs(ttl_secs))
                    .context("presign ttl")?,
            )
            .await
            .context("presign get")?;
        Ok(presigned.uri().to_string())
    }

    async fn put_bytes(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from(bytes.to_vec()))
            .send()
            .await
            .with_context(|| format!("put object {key}"))?;
        Ok(())
    }
}

fn generate_variants(image_bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let img = image::load_from_memory(image_bytes).context("decode image")?;
    let preview = resize_to_webp(&img, 512)?;
    let thumb = resize_to_webp(&img, 256)?;
    Ok((preview, thumb))
}

fn resize_to_webp(img: &image::DynamicImage, max_side: u32) -> Result<Vec<u8>> {
    let (w, h) = img.dimensions();
    let scale = (max_side as f32 / w.max(h) as f32).min(1.0);
    let nw = ((w as f32) * scale).max(1.0) as u32;
    let nh = ((h as f32) * scale).max(1.0) as u32;
    let resized = img.resize(nw, nh, FilterType::Lanczos3);
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    resized
        .write_to(&mut cursor, ImageFormat::WebP)
        .context("encode webp")?;
    Ok(buf)
}

pub type SharedStorage = Arc<AssetStorage>;
