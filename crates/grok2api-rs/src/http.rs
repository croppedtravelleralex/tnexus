//! grok2api-rs HTTP 路由（G0 最小集）。
//!
//! 39 主文档 §6.1 P0：`GET /healthz`、`GET /readyz`。分层 readyz
//! （依赖探测按依赖树展开，39 §6.3「易遗漏模块」）留待 G1+，G0 只探 DB。

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;

/// AXum 共享状态：DB 池 + 配置。
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

/// 构建最小路由。
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
}

/// 存活探针：进程活着即 200（不探依赖）。
async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"status": "ok"})))
}

/// 就绪探针：DB 可达才 200，否则 503。
async fn readyz(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // 轻量探测，不进入长事务；单次 SELECT 1 用短超时，避免挂死。
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("SELECT 1").execute(&state.pool),
    )
    .await
    {
        Ok(Ok(_)) => (StatusCode::OK, Json(json!({"status": "ready"}))),
        Ok(Err(e)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "not_ready", "db": "error", "detail": e.to_string()})),
        ),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "not_ready", "db": "timeout"})),
        ),
    }
}
