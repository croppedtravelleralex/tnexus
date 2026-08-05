//! Fire-and-forget persistence of gateway OpenAPI images into TNexus archive.

use crate::state::AppState;
use axum::http::HeaderMap;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use protocol::estimate_image_input_tokens;
use serde_json::{json, Value};
use std::sync::Arc;
use tnexus_image_archive::{parse_newapi_headers, persist_gateway_image, PersistGatewayImageInput};
use tracing::warn;

pub fn schedule_gateway_image_archive(
    st: Arc<AppState>,
    headers: HeaderMap,
    model: String,
    prompt: String,
    items: Vec<(String, u128, Option<Value>, Option<String>)>,
) {
    let Some(pool) = st.pg_pool.clone() else {
        return;
    };
    let store = st.image_archive_store.clone();
    let attribution = parse_newapi_headers(&headers);

    tokio::spawn(async move {
        for (b64, elapsed_ms, pipeline, source_url) in items {
            let Ok(bytes) = BASE64.decode(b64.as_str()) else {
                warn!("gateway archive: invalid b64");
                continue;
            };
            if bytes.len() < 256 {
                continue;
            }
            let text_tokens = estimate_image_input_tokens(&prompt);
            let usage = json!({
                "input_tokens": text_tokens,
                "output_tokens": 1650,
                "total_tokens": text_tokens + 1650,
            });
            let input = PersistGatewayImageInput {
                model: model.clone(),
                prompt: prompt.clone(),
                image_bytes: bytes,
                generation_ms: elapsed_ms.min(u128::from(u64::MAX)) as u64,
                source_url: source_url.clone(),
                pipeline: pipeline.clone(),
                usage: Some(usage),
                attribution: attribution.clone(),
            };
            if let Err(e) = persist_gateway_image(&pool, store.as_ref(), input).await {
                warn!(error = %e, "gateway image archive failed");
            }
        }
    });
}
