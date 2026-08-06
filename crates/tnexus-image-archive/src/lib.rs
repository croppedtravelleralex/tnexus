//! Gateway OpenAPI image persistence + NewAPI user attribution.

use anyhow::{Context, Result};
use axum::http::HeaderMap;
use chrono::{Duration, Utc};
use image::GenericImageView;
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::io::Cursor;
use tnexus_storage::SharedImageStore;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct NewApiAttribution {
    pub user_id: Option<i64>,
    pub token_name: Option<String>,
}

pub fn parse_newapi_headers(headers: &HeaderMap) -> NewApiAttribution {
    let user_id = ["new-api-user", "x-newapi-user-id"]
        .into_iter()
        .find_map(|name| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_newapi_user_id)
        });
    let token_name = ["x-newapi-token-name", "new-api-token-name"]
        .into_iter()
        .find_map(|name| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    NewApiAttribution {
        user_id,
        token_name,
    }
}

fn parse_newapi_user_id(raw: &str) -> Option<i64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    s.parse::<i64>().ok()
}

pub fn staging_retention_days() -> i64 {
    std::env::var("GATEWAY_IMAGE_STAGING_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7)
        .clamp(1, 365)
}

pub async fn resolve_owner_user_id(
    pool: &PgPool,
    attribution: &NewApiAttribution,
) -> Result<Option<Uuid>> {
    let Some(newapi_user_id) = attribution.user_id else {
        return Ok(None);
    };
    let row = sqlx::query("SELECT id FROM users WHERE newapi_user_id = $1 LIMIT 1")
        .bind(newapi_user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get("id")))
}

pub async fn bind_newapi_user_id(
    pool: &PgPool,
    tnexus_user_id: Uuid,
    newapi_user_id: i64,
) -> Result<()> {
    let taken = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM users WHERE newapi_user_id = $1 AND id <> $2 LIMIT 1",
    )
    .bind(newapi_user_id)
    .bind(tnexus_user_id)
    .fetch_optional(pool)
    .await?;
    if taken.is_some() {
        anyhow::bail!("newapi_user_id already bound to another TNexus user");
    }
    sqlx::query("UPDATE users SET newapi_user_id = $2 WHERE id = $1")
        .bind(tnexus_user_id)
        .bind(newapi_user_id)
        .execute(pool)
        .await
        .context("bind newapi_user_id")?;
    Ok(())
}

pub async fn get_newapi_user_id(pool: &PgPool, tnexus_user_id: Uuid) -> Result<Option<i64>> {
    let row = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT newapi_user_id FROM users WHERE id = $1",
    )
    .bind(tnexus_user_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub struct PersistGatewayImageInput {
    pub model: String,
    pub prompt: String,
    pub image_bytes: Vec<u8>,
    pub generation_ms: u64,
    pub source_url: Option<String>,
    pub pipeline: Option<Value>,
    pub usage: Option<Value>,
    pub attribution: NewApiAttribution,
}

pub async fn persist_gateway_image(
    pool: &PgPool,
    store: Option<&SharedImageStore>,
    input: PersistGatewayImageInput,
) -> Result<Uuid> {
    let owner_user_id = resolve_owner_user_id(pool, &input.attribution).await?;
    let record_id = Uuid::new_v4();
    let retention_days = staging_retention_days();
    let staging_expires_at = Utc::now() + Duration::days(retention_days);

    let (width, height, size_bytes) = image_dimensions(&input.image_bytes);
    let storage_user = owner_user_id.unwrap_or_else(Uuid::nil);

    let (r2_key_original, r2_key_preview, r2_key_thumb, inline_preview_b64) =
        if let Some(store) = store {
            let asset = store
                .store_image_variants(storage_user, record_id, &input.image_bytes)
                .await
                .context("store gateway image variants")?;
            (Some(asset.original_key), Some(asset.preview_key), Some(asset.thumb_key), None)
        } else {
            let b64 = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &input.image_bytes,
            );
            (None, None, None, Some(b64))
        };

    sqlx::query(
        r#"INSERT INTO user_image_records (
            id, owner_user_id, source, newapi_user_id, newapi_token_name, model, prompt,
            agent_prompt, r2_key_original, r2_key_preview, r2_key_thumb, inline_preview_b64,
            source_url, width, height, size_bytes, generation_ms, pipeline, usage,
            backup_status, staging_expires_at
        ) VALUES (
            $1, $2, 'gateway_openapi', $3, $4, $5, $6,
            $6, $7, $8, $9, $10,
            $11, $12, $13, $14, $15, $16, $17,
            'pending', $18
        )"#,
    )
    .bind(record_id)
    .bind(owner_user_id)
    .bind(input.attribution.user_id)
    .bind(input.attribution.token_name.as_deref())
    .bind(&input.model)
    .bind(&input.prompt)
    .bind(r2_key_original)
    .bind(r2_key_preview)
    .bind(r2_key_thumb)
    .bind(inline_preview_b64)
    .bind(input.source_url.as_deref())
    .bind(width)
    .bind(height)
    .bind(size_bytes)
    .bind(input.generation_ms as i64)
    .bind(input.pipeline)
    .bind(input.usage)
    .bind(staging_expires_at)
    .execute(pool)
    .await
    .context("insert user_image_records")?;

    Ok(record_id)
}

fn image_dimensions(bytes: &[u8]) -> (Option<i32>, Option<i32>, Option<i64>) {
    let dims = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.decode().ok())
        .map(|img| img.dimensions());
    (
        dims.map(|(w, _)| w as i32),
        dims.map(|(_, h)| h as i32),
        Some(bytes.len() as i64),
    )
}
