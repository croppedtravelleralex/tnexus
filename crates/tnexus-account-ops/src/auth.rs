use axum::{
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    extract::Request,
};
use std::env;

pub async fn require_token(request: Request, next: Next) -> Response {
    let expected = env::var("ACCOUNT_OPS_TOKEN")
        .or_else(|_| env::var("HELPER_INTERNAL_TOKEN"))
        .unwrap_or_default();
    if expected.trim().is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "ACCOUNT_OPS_TOKEN not configured"})),
        )
            .into_response();
    }
    let provided = request
        .headers()
        .get("x-account-ops-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided != expected {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"error": "invalid X-Account-Ops-Token"})),
        )
            .into_response();
    }
    next.run(request).await
}

pub fn listen_addr() -> String {
    env::var("ACCOUNT_OPS_LISTEN").unwrap_or_else(|_| "127.0.0.1:9011".to_string())
}
