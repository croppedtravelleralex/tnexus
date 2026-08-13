//! Grok 运行时 API 代理：TNexus 登录用户 → GROK_GATEWAY_AUTH_KEY → grok2api-rs :8000。

use crate::middleware::AuthUser;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use futures::StreamExt;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().fallback(proxy_grok_gateway)
}

async fn proxy_grok_gateway(
    State(st): State<Arc<AppState>>,
    _user: AuthUser,
    req: Request,
) -> Result<Response, (StatusCode, String)> {
    let base = st.config.grok2api_base.trim_end_matches('/').to_string();
    let key = st.config.grok_gateway_auth_key.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Grok 网关未配置（需 GROK_GATEWAY_AUTH_KEY）".into(),
    ))?;

    let (parts, body) = req.into_parts();
    let suffix = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target = format!("{base}/v1{suffix}");

    let bytes = axum::body::to_bytes(body, 32 * 1024 * 1024)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let mut builder = st.http.request(parts.method, &target).bearer_auth(key);
    if let Some(ct) = parts.headers.get(header::CONTENT_TYPE) {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    if let Some(accept) = parts.headers.get(header::ACCEPT) {
        builder = builder.header(header::ACCEPT, accept);
    }
    if !bytes.is_empty() {
        builder = builder.body(bytes);
    }

    let upstream = builder
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("grok2api upstream: {e}")))?;

    let status = upstream.status();
    let mut resp = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CACHE_CONTROL,
        header::TRANSFER_ENCODING,
    ] {
        if let Some(v) = upstream.headers().get(&name) {
            resp = resp.header(name, v);
        }
    }

    let stream = upstream
        .bytes_stream()
        .map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
    Ok(resp
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}
