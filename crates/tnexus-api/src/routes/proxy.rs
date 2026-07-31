//! Proxy / Webshare CF scan — delegated to account-ops (gptimage libs, no :8012 HTTP).

use crate::account_ops;
use crate::middleware::AdminUser;
use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct ProxyTestBody {
    #[serde(default)]
    url: String,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/runtime", get(proxy_runtime_get).post(proxy_runtime_save))
        .route("/test", post(proxy_test))
}

pub fn webshare_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(webshare_status))
        .route("/inventory", get(webshare_inventory))
        .route("/run-once", post(webshare_run_once))
}

async fn require_ops(st: &AppState) -> Result<(), (StatusCode, String)> {
    if account_ops::ops_available(st) {
        Ok(())
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "代理服务未配置（需 ACCOUNT_OPS_TOKEN + GPTIMAGE_ROOT）".into(),
        ))
    }
}

async fn proxy_runtime_get(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    require_ops(&st).await?;
    let data = account_ops::proxy_runtime_get(&st)
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(data))
}

async fn proxy_runtime_save(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    require_ops(&st).await?;
    let data = account_ops::proxy_runtime_save(&st, body)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(data))
}

async fn proxy_test(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<ProxyTestBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    require_ops(&st).await?;
    let data = account_ops::proxy_test(&st, &body.url)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(data))
}

async fn webshare_status(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    require_ops(&st).await?;
    let data = account_ops::webshare_cf_scan_status(&st)
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(data))
}

async fn webshare_inventory(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    require_ops(&st).await?;
    let data = account_ops::webshare_cf_scan_inventory(&st)
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    Ok(Json(data))
}

async fn webshare_run_once(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    require_ops(&st).await?;
    let data = account_ops::webshare_cf_scan_run_once(&st)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(data))
}
