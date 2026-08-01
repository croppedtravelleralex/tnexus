//! Chat completions — proxy to TNexus gateway (对齐 gptimage 对话数据面).

use crate::middleware::AuthUser;
use crate::state::AppState;
use crate::usage_metrics::{self, UsageEvent};
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use chrono::Utc;
use futures::StreamExt;
use serde_json::Value;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/completions", post(chat_completions))
}

async fn chat_completions(
    State(st): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let url = format!(
        "{}/v1/chat/completions",
        st.config.gateway_base.trim_end_matches('/')
    );
    let client = reqwest::Client::new();
    let mut req = client
        .post(&url)
        .header(header::CONTENT_TYPE, "application/json")
        .json(&body);
    if let Some(token) = &st.config.gateway_internal_token {
        req = req.bearer_auth(token);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("gateway unreachable: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err((status, text));
    }

    let _ = usage_metrics::record_event(&UsageEvent {
        ts: Utc::now().to_rfc3339(),
        email: user.claims.email.clone(),
        binding: String::new(),
        metric: "dialogues_real".into(),
        ok: true,
    });

    if stream {
        let stream = resp.bytes_stream().map(|chunk| {
            chunk.map_err(|e| std::io::Error::other(e))
        });
        let body = Body::from_stream(stream);
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(body)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?);
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(json).into_response())
}
