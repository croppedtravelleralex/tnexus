//! Chat completions — proxy to TNexus gateway (对齐 gptimage 对话数据面).

use crate::middleware::AuthUser;
use crate::state::AppState;
use crate::usage_metrics::{self, UsageEvent};
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use futures::StreamExt;
use serde_json::Value;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/completions", post(chat_completions))
        .route("/models", get(chat_models))
}

async fn chat_models(State(st): State<Arc<AppState>>) -> Result<Json<Value>, (StatusCode, String)> {
    let url = format!(
        "{}/v1/models",
        st.config.gateway_base.trim_end_matches('/')
    );
    let client = reqwest::Client::new();
    let mut req = client.get(&url);
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
    let json: Value = resp
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(json))
}

async fn chat_completions(
    State(st): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let wants_image = body
        .get("image_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || chat_body_requests_image(&body);
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

    let metric = if wants_image {
        "images_chat"
    } else {
        "dialogues_real"
    };
    let _ = usage_metrics::record_event(&UsageEvent {
        ts: Utc::now().to_rfc3339(),
        email: user.claims.email.clone(),
        binding: String::new(),
        metric: metric.into(),
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

fn chat_body_requests_image(body: &Value) -> bool {
    let last_user = body
        .get("messages")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .rev()
                .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        });
    let text = last_user.and_then(|m| match m.get("content") {
        Some(Value::String(s)) => Some(s.as_str()),
        Some(other) => other.as_str(),
        None => None,
    });
    text.map(|t| {
        let trimmed = t.trim();
        let lower = trimmed.to_lowercase();
        lower.starts_with("@create image")
            || trimmed.starts_with("@Create image")
            || lower.starts_with("/image")
            || lower.starts_with("/img")
            || trimmed.contains("画一张")
            || trimmed.contains("画一幅")
            || trimmed.contains("画个")
            || trimmed.contains("帮我画")
            || trimmed.contains("生成图片")
            || trimmed.contains("生成一张图")
            || trimmed.contains("生图")
            || trimmed.contains("绘制")
    })
    .unwrap_or(false)
}
