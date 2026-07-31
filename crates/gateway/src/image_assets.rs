//! Short-lived in-memory image assets for `response_format=url` (browser-displayable proxy URLs).

use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
struct StoredAsset {
    bytes: Vec<u8>,
    content_type: String,
    expires_at: u64,
}

#[derive(Clone)]
pub struct ImageAssetStore {
    secret: Arc<Vec<u8>>,
    ttl_secs: u64,
    assets: Arc<RwLock<HashMap<Uuid, StoredAsset>>>,
}

#[derive(Debug, Deserialize)]
pub struct AssetQuery {
    pub exp: u64,
    pub sig: String,
}

impl ImageAssetStore {
    pub fn new(secret: impl Into<Vec<u8>>, ttl_secs: u64) -> Self {
        Self {
            secret: Arc::new(secret.into()),
            ttl_secs: ttl_secs.max(60),
            assets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn ttl_secs(&self) -> u64 {
        self.ttl_secs
    }

    pub fn store(&self, bytes: Vec<u8>) -> (Uuid, u64, String) {
        let id = Uuid::new_v4();
        let exp = unix_now() + self.ttl_secs;
        let content_type = sniff_content_type(&bytes).to_string();
        let sig = sign_token(&self.secret, &id.to_string(), exp);
        let asset = StoredAsset {
            bytes,
            content_type,
            expires_at: exp,
        };
        self.assets.write().expect("asset lock").insert(id, asset);
        (id, exp, sig)
    }

    pub fn public_url(&self, base: &str, id: Uuid, exp: u64, sig: &str) -> String {
        format!(
            "{}/v1/images/assets/{}?exp={exp}&sig={sig}",
            base.trim_end_matches('/'),
            id
        )
    }

    fn verify_sig(&self, asset_id: &str, exp: u64, sig: &str) -> bool {
        let expected = sign_token(&self.secret, asset_id, exp);
        expected == sig
    }

    fn get_valid(&self, id: Uuid, exp: u64, sig: &str) -> Result<StoredAsset> {
        if exp < unix_now() {
            bail!("asset link expired");
        }
        if !self.verify_sig(&id.to_string(), exp, sig) {
            bail!("invalid asset signature");
        }
        let guard = self.assets.read().expect("asset lock");
        let Some(asset) = guard.get(&id) else {
            bail!("asset not found");
        };
        if asset.expires_at < unix_now() {
            bail!("asset expired");
        }
        Ok(StoredAsset {
            bytes: asset.bytes.clone(),
            content_type: asset.content_type.clone(),
            expires_at: asset.expires_at,
        })
    }

    pub fn prune_expired(&self) {
        let now = unix_now();
        let mut guard = self.assets.write().expect("asset lock");
        guard.retain(|_, asset| asset.expires_at >= now);
    }
}

pub fn serve_image_asset(
    store: &ImageAssetStore,
    asset_id: Uuid,
    query: AssetQuery,
) -> Response {
    match store.get_valid(asset_id, query.exp, &query.sig) {
        Ok(asset) => {
            let mut resp = Response::new(Body::from(asset.bytes));
            *resp.status_mut() = StatusCode::OK;
            if let Ok(ct) = HeaderValue::from_str(&asset.content_type) {
                resp.headers_mut()
                    .insert(header::CONTENT_TYPE, ct);
            }
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=3600"),
            );
            resp
        }
        Err(err) => {
            warn!(%asset_id, error = %err, "image asset fetch failed");
            (
                StatusCode::NOT_FOUND,
                format!("image asset unavailable: {err}"),
            )
                .into_response()
        }
    }
}

pub fn sign_token(secret: &[u8], asset_id: &str, exp: u64) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(format!("{asset_id}:{exp}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sniff_content_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
}

pub fn asset_signing_secret_from_env() -> Result<Vec<u8>> {
    if let Ok(key) = std::env::var("GATEWAY_ASSET_HMAC_KEY") {
        if !key.trim().is_empty() {
            return Ok(key.into_bytes());
        }
    }
    if let Ok(key) = std::env::var("GATEWAY_AUTH_KEY") {
        if !key.trim().is_empty() {
            return Ok(key.into_bytes());
        }
    }
    if let Ok(key) = std::env::var("AUTH_JWT_SECRET") {
        if key.len() >= 32 {
            return Ok(key.into_bytes());
        }
    }
    anyhow::bail!(
        "set GATEWAY_ASSET_HMAC_KEY, GATEWAY_AUTH_KEY, or AUTH_JWT_SECRET (>=32) for image asset URLs"
    );
}

pub fn asset_ttl_secs_from_env() -> u64 {
    std::env::var("GATEWAY_IMAGE_ASSET_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(86_400)
        .max(60)
}

pub fn public_base_url_from_env() -> String {
    std::env::var("GATEWAY_PUBLIC_BASE_URL")
        .unwrap_or_default()
        .trim()
        .trim_end_matches('/')
        .to_string()
}

pub fn resolve_public_base(configured: &str, headers: &axum::http::HeaderMap) -> Result<String> {
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .context("missing Host header; set GATEWAY_PUBLIC_BASE_URL")?;
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    Ok(format!("{proto}://{host}"))
}

pub fn wants_url_response(response_format: &str) -> bool {
    response_format.eq_ignore_ascii_case("url")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let secret = b"test-secret-key";
        let id = Uuid::new_v4().to_string();
        let exp = unix_now() + 3600;
        let sig = sign_token(secret, &id, exp);
        let store = ImageAssetStore::new(secret.to_vec(), 3600);
        assert!(store.verify_sig(&id, exp, &sig));
        assert!(!store.verify_sig(&id, exp, "bad"));
    }

    #[test]
    fn sniff_png() {
        assert_eq!(sniff_content_type(&[0x89, 0x50, 0x4E, 0x47]), "image/png");
    }
}
