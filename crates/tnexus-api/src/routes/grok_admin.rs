//! Grok 管理 API 代理：TNexus 管理员会话 → 服务端换取 grok-admin JWT → :8091。

use crate::middleware::AdminUser;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().fallback(proxy_grok_admin)
}

async fn proxy_grok_admin(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    req: Request,
) -> Result<Response, (StatusCode, String)> {
    let client = st.grok_admin.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Grok 管理代理未配置（需 GROK_ADMIN_PASSWORD + GROK_ADMIN_BASE）".into(),
    ))?;

    let (parts, body) = req.into_parts();
    let suffix = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target = format!("{}{}", client.base(), suffix);

    let bytes = axum::body::to_bytes(body, 32 * 1024 * 1024)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let method = parts.method.clone();
    let mut attempt = 0u8;
    loop {
        attempt += 1;
        let token = client
            .access_token()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

        let mut builder = st.http.request(method.clone(), &target);
        builder = builder.bearer_auth(&token);
        if let Some(ct) = parts.headers.get(header::CONTENT_TYPE) {
            builder = builder.header(header::CONTENT_TYPE, ct);
        }
        if !bytes.is_empty() {
            builder = builder.body(bytes.clone());
        }

        let upstream = builder
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("grok-admin upstream: {e}")))?;

        if upstream.status() == StatusCode::UNAUTHORIZED && attempt < 2 {
            client.invalidate();
            continue;
        }

        let status = upstream.status();
        let headers = upstream.headers().clone();
        let body = upstream
            .bytes()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

        let mut resp = Response::builder().status(status);
        if let Some(ct) = headers.get(header::CONTENT_TYPE) {
            resp = resp.header(header::CONTENT_TYPE, ct);
        }
        return Ok(resp
            .body(Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()));
    }
}
