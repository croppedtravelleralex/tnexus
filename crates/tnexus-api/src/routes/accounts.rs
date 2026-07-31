use crate::account_ops::{self, new_progress_id};
use crate::accounts_store::activity_daily;
use crate::gptimage_proxy::{admin_token, proxy_get, proxy_post};
use crate::middleware::AdminUser;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    200
}

#[derive(Debug, Deserialize)]
struct ActivityQuery {
    #[serde(default = "default_days")]
    days: usize,
}

fn default_days() -> usize {
    14
}

#[derive(Debug, Deserialize)]
struct UsageRecentQuery {
    #[serde(default = "default_usage_days")]
    days: usize,
}

fn default_usage_days() -> usize {
    6
}

#[derive(Debug, Deserialize)]
struct SchedulingBody {
    access_token: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    reason: String,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct SchedulingBulkBody {
    #[serde(default)]
    access_tokens: Vec<String>,
    #[serde(default)]
    emails: Vec<String>,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Deserialize)]
struct CreateAccountsBody {
    #[serde(default)]
    tokens: Vec<String>,
    #[serde(default)]
    accounts: Vec<Value>,
    #[serde(default)]
    skip_refresh: bool,
}

#[derive(Debug, Deserialize)]
struct ImportBatchBody {
    #[serde(default)]
    accounts: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct ExportBody {
    #[serde(default)]
    access_tokens: Vec<String>,
    #[serde(default = "default_json_format")]
    format: String,
}

#[derive(Debug, Deserialize)]
struct RefreshBody {
    #[serde(default)]
    access_tokens: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ReloginBody {
    #[serde(default)]
    access_tokens: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthStartBody {
    #[serde(default)]
    email_hint: String,
}

#[derive(Debug, Deserialize)]
struct OAuthFinishBody {
    session_id: String,
    callback: String,
}

#[derive(Debug, Deserialize)]
struct BindingSlotsQuery {
    #[serde(default)]
    week_offset: i64,
    #[serde(default = "default_timezone")]
    timezone: String,
}

fn default_timezone() -> String {
    "Asia/Shanghai".to_string()
}

fn default_json_format() -> String {
    "json".to_string()
}

#[derive(Debug, Deserialize)]
struct IncludeItemsQuery {
    #[serde(default)]
    include_items: bool,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_accounts).post(create_accounts))
        .route("/reload-from-storage", post(reload_from_storage))
        .route("/activity/daily", get(get_activity_daily))
        .route("/schedulable-breakdown", get(get_schedulable_breakdown))
        .route("/scheduling", post(set_scheduling))
        .route("/scheduling/bulk", post(scheduling_bulk))
        .route("/import-batch", post(import_batch))
        .route("/usage/recent", get(get_usage_recent))
        .route("/usage/binding-slots", get(get_binding_slots))
        .route("/export", post(export_accounts))
        .route("/refresh", post(refresh_accounts))
        .route("/refresh/progress/{progress_id}", get(get_refresh_progress))
        .route("/re-login", post(relogin_accounts))
        .route("/re-login/progress/{progress_id}", get(get_relogin_progress))
        .route("/oauth/start", post(oauth_start))
        .route("/oauth/finish", post(oauth_finish))
}

async fn list_accounts(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<ListQuery>,
) -> Json<Value> {
    Json(st.accounts.list(q.offset, q.limit).await)
}

async fn create_accounts(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<IncludeItemsQuery>,
    Json(body): Json<CreateAccountsBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut payloads = body.accounts;
    for token in body.tokens {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            payloads.push(json!({ "access_token": trimmed }));
        }
    }
    if payloads.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "tokens is required".into()));
    }
    let summary = st
        .accounts
        .import_payloads(payloads)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let list = st.accounts.list(0, usize::MAX).await;
    let stats = list.get("stats").cloned().unwrap_or(json!({}));
    let mut response = json!({
        "added": summary.added,
        "skipped": summary.skipped,
        "updated": summary.updated,
        "refreshed": 0,
        "errors": [],
        "stats": stats,
    });
    if q.include_items {
        response["items"] = list.get("items").cloned().unwrap_or(json!([]));
    }
    Ok(Json(response))
}

async fn import_batch(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<IncludeItemsQuery>,
    Json(body): Json<ImportBatchBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let summary = st
        .accounts
        .import_payloads(body.accounts)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let list = st.accounts.list(0, usize::MAX).await;
    let stats = list.get("stats").cloned().unwrap_or(json!({}));
    let mut response = json!({
        "added": summary.added,
        "skipped": summary.skipped,
        "updated": summary.updated,
        "stats": stats,
    });
    if q.include_items {
        response["items"] = list.get("items").cloned().unwrap_or(json!([]));
    }
    Ok(Json(response))
}

async fn export_accounts(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<ExportBody>,
) -> Result<Response, (StatusCode, String)> {
    if body.format != "json" {
        return Err((StatusCode::BAD_REQUEST, "only json export is supported".into()));
    }
    let tokens: Vec<String> = body
        .access_tokens
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let items = st.accounts.export_items(&tokens).await;
    if items.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "没有可导出的账号".into()));
    }
    let payload = if items.len() == 1 {
        items[0].clone()
    } else {
        json!(items)
    };
    let body = serde_json::to_string_pretty(&payload).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize export: {e}"),
        )
    })?;
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let filename = format!("tnexus-accounts-{timestamp}.json");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(Body::from(body))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn get_activity_daily(
    _admin: AdminUser,
    Query(q): Query<ActivityQuery>,
) -> Json<Value> {
    Json(activity_daily(q.days))
}

async fn get_usage_recent(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<UsageRecentQuery>,
) -> Json<Value> {
    Json(st.accounts.usage_recent(q.days).await)
}

async fn get_schedulable_breakdown(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Json<Value> {
    Json(st.accounts.schedulable_breakdown().await)
}

async fn reload_from_storage(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Json<Value> {
    match st.accounts.reload().await {
        Ok(total) => Json(json!({ "ok": true, "total": total })),
        Err(err) => Json(json!({ "ok": false, "error": err.to_string() })),
    }
}

async fn set_scheduling(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<SchedulingBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let token = body.access_token.trim();
    if token.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "access_token is required".into()));
    }
    let item = st
        .accounts
        .set_scheduling_by_token(token, body.enabled)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let Some(item) = item else {
        return Err((StatusCode::NOT_FOUND, "account not found".into()));
    };
    let list = st.accounts.list(0, usize::MAX).await;
    let stats = list.get("stats").cloned().unwrap_or(json!({}));
    Ok(Json(json!({
        "ok": true,
        "enabled": body.enabled,
        "item": item,
        "stats": stats,
    })))
}

async fn scheduling_bulk(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<SchedulingBulkBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut tokens = body
        .access_tokens
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() && !body.emails.is_empty() {
        let guard = st.accounts.list(0, usize::MAX).await;
        let items = guard
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let email_set: std::collections::HashSet<String> = body
            .emails
            .iter()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        tokens = items
            .into_iter()
            .filter_map(|row| {
                let email = row.get("email")?.as_str()?.to_lowercase();
                if email_set.contains(&email) {
                    row.get("access_token")?.as_str().map(str::to_string)
                } else {
                    None
                }
            })
            .collect();
    }
    if tokens.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "access_tokens is required".into()));
    }
    let updated = st
        .accounts
        .set_scheduling_bulk(&tokens, body.enabled)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "ok": true,
        "updated": updated,
        "enabled": body.enabled,
    })))
}

async fn get_binding_slots(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<BindingSlotsQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if admin_token(&st).is_some() {
        let query = format!(
            "week_offset={}&timezone={}",
            q.week_offset,
            urlencoding::encode(&q.timezone)
        );
        if let Ok(data) = proxy_get(&st, "/api/accounts/usage/binding-slots", &query).await {
            return Ok(Json(data));
        }
    }
    let map = st.accounts.email_to_binding_map().await;
    let data = crate::usage_metrics::get_binding_usage_slots(&map, q.week_offset, &q.timezone)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(data))
}

async fn refresh_accounts(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<RefreshBody>,
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
    if tokens.len() > 50 {
        return Err((StatusCode::BAD_REQUEST, "单次最多刷新 50 个账号".into()));
    }

    let progress_id = new_progress_id();
    let pid = progress_id.clone();
    let state = st.clone();
    let progress = st.refresh_progress.clone();
    let token_list = tokens.clone();
    tokio::spawn(async move {
        account_ops::spawn_refresh(state, token_list, progress, pid).await;
    });
    Ok(Json(json!({ "progress_id": progress_id })))
}

async fn get_refresh_progress(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    axum::extract::Path(progress_id): axum::extract::Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if let Some(progress) = st.refresh_progress.get(&progress_id).await {
        return Ok(Json(progress));
    }
    if admin_token(&st).is_some() {
        if let Ok(data) = proxy_get(
            &st,
            &format!("/api/accounts/refresh/progress/{progress_id}"),
            "",
        )
        .await
        {
            if data.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                if let Some(result) = data.get("result") {
                    if let Some(items) = result.get("items").and_then(|v| v.as_array()) {
                        let _ = st.accounts.merge_remote_items(items).await;
                    }
                }
            }
            return Ok(Json(data));
        }
    }
    Err((StatusCode::NOT_FOUND, "progress not found".into()))
}

async fn relogin_accounts(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<ReloginBody>,
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
    let progress_id = new_progress_id();
    let pid = progress_id.clone();
    let state = st.clone();
    let progress = st.relogin_progress.clone();
    tokio::spawn(async move {
        account_ops::spawn_relogin(state, tokens, progress, pid).await;
    });
    Ok(Json(json!({ "progress_id": progress_id })))
}

async fn get_relogin_progress(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    axum::extract::Path(progress_id): axum::extract::Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if let Some(progress) = st.relogin_progress.get(&progress_id).await {
        return Ok(Json(progress));
    }
    if admin_token(&st).is_some() {
        if let Ok(data) = proxy_get(
            &st,
            &format!("/api/accounts/re-login/progress/{progress_id}"),
            "",
        )
        .await
        {
            if data.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                if let Some(result) = data.get("result") {
                    if let Some(items) = result.get("items").and_then(|v| v.as_array()) {
                        let _ = st.accounts.merge_remote_items(items).await;
                    }
                }
            }
            return Ok(Json(data));
        }
    }
    Err((StatusCode::NOT_FOUND, "progress not found".into()))
}

async fn oauth_start(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<OAuthStartBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    match account_ops::oauth_start(&st, &body.email_hint).await {
        Ok(data) => return Ok(Json(data)),
        Err(native_err) => {
            if admin_token(&st).is_some() {
                let data = proxy_post(
                    &st,
                    "/api/accounts/oauth/start",
                    json!({ "email_hint": body.email_hint }),
                )
                .await?;
                return Ok(Json(data));
            }
            return Err((StatusCode::SERVICE_UNAVAILABLE, native_err));
        }
    }
}

async fn oauth_finish(
    State(st): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<OAuthFinishBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let native = account_ops::oauth_finish(&st, &body.session_id, &body.callback).await;
    let tokens = match native {
        Ok(data) => data,
        Err(native_err) => {
            if admin_token(&st).is_some() {
                let data = proxy_post(
                    &st,
                    "/api/accounts/oauth/finish",
                    json!({
                        "session_id": body.session_id,
                        "callback": body.callback,
                    }),
                )
                .await?;
                if let Some(items) = data.get("items").and_then(|v| v.as_array()) {
                    let summary = st.accounts.merge_remote_items(items).await.map_err(|e| {
                        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                    })?;
                    let list = st.accounts.list(0, usize::MAX).await;
                    let stats = list.get("stats").cloned().unwrap_or(json!({}));
                    return Ok(Json(json!({
                        "added": summary.added,
                        "skipped": summary.skipped,
                        "updated": summary.updated,
                        "refreshed": data.get("refreshed").cloned().unwrap_or(json!(0)),
                        "errors": data.get("errors").cloned().unwrap_or(json!([])),
                        "stats": stats,
                    })));
                }
                return Ok(Json(data));
            }
            return Err((StatusCode::BAD_REQUEST, native_err));
        }
    };
    let payload = json!({
        "access_token": tokens.get("access_token").and_then(|v| v.as_str()).unwrap_or(""),
        "refresh_token": tokens.get("refresh_token"),
        "id_token": tokens.get("id_token"),
        "source_type": "oauth_login",
        "email": format!(
            "oauth-{}@imported.local",
            tokens
                .get("access_token")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(8)
                .collect::<String>()
        ),
    });
    let summary = st
        .accounts
        .import_payloads(vec![payload.clone()])
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let refreshed = match account_ops::refresh_one(&st, payload).await {
        Ok(updated) => {
            let _ = st.accounts.merge_remote_items(&[updated]).await;
            1
        }
        Err(_) => 0,
    };
    let list = st.accounts.list(0, usize::MAX).await;
    let stats = list.get("stats").cloned().unwrap_or(json!({}));
    Ok(Json(json!({
        "added": summary.added,
        "skipped": summary.skipped,
        "updated": summary.updated,
        "refreshed": refreshed,
        "errors": [],
        "stats": stats,
    })))
}
