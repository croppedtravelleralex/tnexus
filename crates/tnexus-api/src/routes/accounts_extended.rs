//! Extended account admin routes — TNexus local only (no gptimage proxy).

use crate::middleware::AdminUser;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct DeleteAccountsBody {
    #[serde(default)]
    pub tokens: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAccountBody {
    pub access_token: String,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub quota: Option<i64>,
    #[serde(default)]
    pub proxy: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SoftBandBody {
    pub access_token: String,
    #[serde(default)]
    pub percent: Option<i64>,
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Deserialize)]
pub struct QuotaPrimeBody {
    #[serde(default)]
    pub access_tokens: Vec<String>,
    #[serde(default)]
    pub preferred_account_email: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize)]
pub struct RefreshAllStartBody {
    #[serde(flatten)]
    pub options: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct IncludeItemsQuery {
    #[serde(default)]
    pub include_items: bool,
}

#[derive(Debug, Deserialize)]
pub struct OutlookRecoveryEnableBody {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct RecoverOutlookBody {
    pub access_token: String,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", delete(delete_accounts))
        .route("/update", post(update_account))
        .route("/soft-band", post(soft_band))
        .route("/quota-window/prime", post(quota_window_prime))
        .route("/quota-window/prime/status", get(quota_window_prime_status))
        .route("/refresh-all/start", post(refresh_all_start))
        .route("/refresh-all/status", get(refresh_all_status))
        .route("/refresh-all/stop", post(refresh_all_stop))
        .route("/outlook-recovery/status", get(outlook_recovery_status))
        .route("/outlook-recovery/enable", post(outlook_recovery_enable))
        .route("/recover-outlook", post(recover_outlook))
        .route("/recover-outlook/progress/{progress_id}", get(recover_outlook_progress))
}

async fn delete_accounts(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<IncludeItemsQuery>,
    Json(body): Json<DeleteAccountsBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let tokens: Vec<String> = body
        .tokens
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "tokens is required".into()));
    }

    let removed = st
        .accounts
        .delete_by_tokens(&tokens)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let list = st.accounts.list(0, usize::MAX).await;
    let stats = list.get("stats").cloned().unwrap_or(json!({}));
    let mut response = json!({ "removed": removed, "stats": stats, "source": "tnexus-local" });
    if q.include_items {
        response["items"] = list.get("items").cloned().unwrap_or(json!([]));
    }
    Ok(Json(response))
}

async fn update_account(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<IncludeItemsQuery>,
    Json(body): Json<UpdateAccountBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let token = body.access_token.trim();
    if token.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "access_token is required".into()));
    }

    let mut patch = serde_json::Map::new();
    if let Some(v) = body.r#type {
        patch.insert("type".into(), json!(v));
    }
    if let Some(v) = body.status {
        patch.insert("status".into(), json!(v));
    }
    if let Some(v) = body.quota {
        patch.insert("quota".into(), json!(v));
    }
    if let Some(v) = body.proxy {
        patch.insert("proxy".into(), json!(v));
    }
    let item = st
        .accounts
        .update_by_token(token, &Value::Object(patch))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "account not found".into()))?;
    let list = st.accounts.list(0, usize::MAX).await;
    let stats = list.get("stats").cloned().unwrap_or(json!({}));
    let mut response = json!({ "item": item, "stats": stats, "source": "tnexus-local" });
    if q.include_items {
        response["items"] = list.get("items").cloned().unwrap_or(json!([]));
    }
    Ok(Json(response))
}

async fn soft_band(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<IncludeItemsQuery>,
    Json(body): Json<SoftBandBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let token = body.access_token.trim();
    if token.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "access_token is required".into()));
    }
    let patch = if body.clear {
        json!({ "soft_band_percent": null })
    } else {
        let pct = body.percent.unwrap_or(50).clamp(1, 99);
        json!({ "soft_band_percent": pct })
    };
    let item = st
        .accounts
        .update_by_token(token, &patch)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "account not found".into()))?;
    let list = st.accounts.list(0, usize::MAX).await;
    let stats = list.get("stats").cloned().unwrap_or(json!({}));
    let mut response = json!({ "item": item, "stats": stats, "source": "tnexus-local" });
    if q.include_items {
        response["items"] = list.get("items").cloned().unwrap_or(json!([]));
    }
    Ok(Json(response))
}

async fn quota_window_prime(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<QuotaPrimeBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let tokens: Vec<String> = body
        .access_tokens
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "access_tokens is required".into()));
    }
    let mut accounts = Vec::new();
    for token in &tokens {
        if let Some(row) = st.accounts.export_account_for_token(token).await {
            accounts.push(row);
        }
    }
    if crate::account_ops::ops_available(&st) {
        match crate::account_ops::quota_prime_enqueue(
            &st,
            json!({
                "access_tokens": tokens,
                "mode": if body.mode.is_empty() { "manual" } else { body.mode.as_str() },
                "accounts": accounts,
            }),
        )
        .await
        {
            Ok(data) => {
                for token in &tokens {
                    let patch = json!({
                        "quota_window_prime_state": "queued",
                        "quota_window_prime_last_error": null,
                    });
                    let _ = st.accounts.update_by_token(token, &patch).await;
                }
                return Ok(Json(data));
            }
            Err(err) => {
                tracing::warn!("account-ops quota prime failed, fallback to gateway: {err}");
            }
        }
    }
    st.quota_prime
        .enqueue_tokens(st.clone(), tokens.clone(), accounts)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))
}

async fn quota_window_prime_status(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    if crate::account_ops::ops_available(&st) {
        if let Ok(data) = crate::account_ops::quota_prime_status(&st).await {
            return Ok(Json(data));
        }
    }
    Ok(Json(st.quota_prime.status().await))
}

async fn refresh_all_start(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<RefreshAllStartBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    st.refresh_all
        .start(st.clone(), Value::Object(body.options))
        .await
        .map(Json)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))
}

async fn refresh_all_status(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    Ok(Json(st.refresh_all.status().await))
}

async fn refresh_all_stop(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    Ok(Json(st.refresh_all.stop().await))
}

async fn outlook_recovery_status(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    if crate::account_ops::ops_available(&st) {
        if let Ok(data) = crate::account_ops::outlook_auto_recovery_status(&st).await {
            return Ok(Json(data));
        }
    }
    Ok(Json(st.outlook_recovery.status().await))
}

async fn outlook_recovery_enable(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<OutlookRecoveryEnableBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if crate::account_ops::ops_available(&st) {
        if let Ok(data) = crate::account_ops::outlook_auto_recovery_settings(
            &st,
            json!({ "enabled": body.enabled }),
        )
        .await
        {
            return Ok(Json(data));
        }
    }
    Ok(Json(st.outlook_recovery.set_enabled(body.enabled).await))
}

pub async fn activity_daily_local(st: &Arc<AppState>, days: usize) -> Value {
    st.accounts.activity_daily_from_pool(days).await
}

async fn recover_outlook(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<RecoverOutlookBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let token = body.access_token.trim();
    if token.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "access_token is required".into()));
    }
    if !crate::account_ops::ops_available(&st) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Outlook 恢复未配置（需 ACCOUNT_OPS_TOKEN + GPTIMAGE_ROOT）".into(),
        ));
    }
    let account = st.accounts.export_account_for_token(token).await;
    let mut payload = json!({ "access_token": token });
    if let Some(row) = account {
        payload["account"] = row;
    }
    let data = crate::account_ops::outlook_recover_one(&st, payload)
        .await
        .map_err(|e| (StatusCode::CONFLICT, e))?;
    Ok(Json(data))
}

async fn recover_outlook_progress(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Path(progress_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if !crate::account_ops::ops_available(&st) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Outlook 恢复未配置（需 ACCOUNT_OPS_TOKEN + GPTIMAGE_ROOT）".into(),
        ));
    }
    let data = crate::account_ops::outlook_recover_progress(&st, &progress_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(data))
}
