//! Forward selected admin APIs to gptimage when `GPTIMAGE_ADMIN_TOKEN` is set.

use crate::state::AppState;
use axum::http::{Method, StatusCode};
use serde_json::Value;
use std::sync::Arc;

pub fn admin_token(state: &AppState) -> Option<&str> {
    state
        .config
        .gptimage_admin_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub async fn proxy_json(
    state: &Arc<AppState>,
    method: Method,
    path: &str,
    query: &str,
    body: Option<Value>,
) -> Result<Value, (StatusCode, String)> {
    let token = admin_token(state).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "未配置 GPTIMAGE_ADMIN_TOKEN，无法代理到 gptimage".into(),
    ))?;
    let base = state.config.gptimage_base.trim_end_matches('/');
    let mut url = format!("{base}{path}");
    if !query.is_empty() {
        url.push('?');
        url.push_str(query);
    }

    let mut req = state.http.request(method, &url).header("Authorization", format!("Bearer {token}"));
    if let Some(payload) = body {
        req = req.json(&payload);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("gptimage 请求失败: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("gptimage 响应读取失败: {e}")))?;

    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("detail")
                    .and_then(|d| d.get("error"))
                    .or_else(|| v.get("error"))
                    .and_then(|e| e.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| text.clone());
        return Err((status, message));
    }

    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("gptimage 响应 JSON 解析失败: {e}"),
        )
    })
}

pub async fn proxy_get(state: &Arc<AppState>, path: &str, query: &str) -> Result<Value, (StatusCode, String)> {
    proxy_json(state, Method::GET, path, query, None).await
}

pub async fn proxy_post(
    state: &Arc<AppState>,
    path: &str,
    body: Value,
) -> Result<Value, (StatusCode, String)> {
    proxy_json(state, Method::POST, path, "", Some(body)).await
}
