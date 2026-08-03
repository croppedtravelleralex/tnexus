use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::sync::Arc;

use crate::nurture;
use crate::oauth::OAuthLoginService;
use crate::ops::OpsServices;
use crate::refresh;
use crate::relogin;
use crate::user_info;

#[derive(Clone)]
pub struct AppState {
    pub oauth: Arc<OAuthLoginService>,
    pub http: reqwest::Client,
    pub ops: Arc<OpsServices>,
}

#[derive(Deserialize)]
pub struct OAuthStartIn {
    #[serde(default)]
    email_hint: String,
}

#[derive(Deserialize)]
pub struct OAuthFinishIn {
    session_id: String,
    callback: String,
}

#[derive(Deserialize)]
pub struct AccountIn {
    #[serde(default)]
    account: Map<String, Value>,
}

#[derive(Deserialize)]
pub struct NurtureEnqueueIn {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    access_tokens: Vec<String>,
    #[serde(default)]
    accounts: Vec<Map<String, Value>>,
}

#[derive(Deserialize)]
pub struct QuotaPrimeIn {
    #[serde(default)]
    access_tokens: Vec<String>,
    #[serde(default)]
    accounts: Vec<Map<String, Value>>,
}

#[derive(Deserialize)]
pub struct OutlookRecoverIn {
    access_token: String,
    #[serde(default)]
    account: Map<String, Value>,
}

#[derive(Deserialize)]
pub struct NurtureEnableIn {
    enabled: bool,
}

#[derive(Deserialize)]
pub struct NurtureProcessOneIn {
    #[serde(default)]
    access_token: String,
}

fn token_email_map(accounts: &[Map<String, Value>]) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for row in accounts {
        let token = row
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let email = row
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if !token.is_empty() && !email.is_empty() {
            out.insert(token.to_string(), email.to_string());
        }
    }
    out
}

pub fn api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/oauth/start", post(oauth_start))
        .route("/v1/oauth/finish", post(oauth_finish))
        .route("/v1/accounts/refresh-one", post(refresh_one))
        .route("/v1/accounts/relogin-one", post(relogin_one))
        .route("/v1/nurture/status", get(nurture_status))
        .route("/v1/nurture/enable", post(nurture_enable))
        .route("/v1/nurture/enqueue", post(nurture_enqueue))
        .route("/v1/nurture/process-one", post(nurture_process_one))
        .route("/v1/outlook/auto-recovery/status", get(outlook_status))
        .route("/v1/outlook/auto-recovery/settings", post(outlook_settings))
        .route("/v1/outlook/recover-one", post(outlook_recover_one))
        .route("/v1/outlook/recover/progress/{id}", get(outlook_progress))
        .route("/v1/quota-window/prime", post(quota_prime))
        .route("/v1/quota-window/prime/status", get(quota_prime_status))
        .route("/v1/proxy/runtime", get(proxy_runtime_get).post(proxy_runtime_save))
        .route("/v1/proxy/test", post(proxy_test))
        .route("/v1/webshare-cf-scan/status", get(webshare_status))
        .route("/v1/webshare-cf-scan/inventory", get(webshare_inventory))
        .route("/v1/webshare-cf-scan/run-once", post(webshare_run_once))
        .with_state(state)
}

pub async fn health() -> Json<Value> {
    Json(json!({
        "ok": true,
        "service": "tnexus-account-ops",
        "runtime": "rust",
        "ops_bridge": true,
    }))
}

async fn oauth_start(
    State(st): State<Arc<AppState>>,
    Json(body): Json<OAuthStartIn>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match st.oauth.start(&body.email_hint) {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(oauth_err(e)),
    }
}

async fn oauth_finish(
    State(st): State<Arc<AppState>>,
    Json(body): Json<OAuthFinishIn>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match st.oauth.finish(&body.session_id, &body.callback).await {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(oauth_err(e)),
    }
}

async fn refresh_one(
    State(st): State<Arc<AppState>>,
    Json(body): Json<AccountIn>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let acc = refresh::refresh_access_token(&st.http, &body.account, false).await;
    let merged = user_info::merge_user_info(&st.http, &acc).await;
    Ok(Json(json!({ "ok": true, "account": merged })))
}

async fn relogin_one(
    State(_st): State<Arc<AppState>>,
    Json(body): Json<AccountIn>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match relogin::relogin_account(&body.account).await {
        Ok(merged) => Ok(Json(json!({ "ok": true, "account": merged }))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )),
    }
}

async fn nurture_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(st.ops.nurture_status())
}

async fn nurture_enable(
    State(st): State<Arc<AppState>>,
    Json(body): Json<NurtureEnableIn>,
) -> Json<Value> {
    Json(st.ops.nurture_enable(body.enabled))
}

async fn nurture_enqueue(
    State(st): State<Arc<AppState>>,
    Json(body): Json<NurtureEnqueueIn>,
) -> Json<Value> {
    let emails = token_email_map(&body.accounts);
    Json(st.ops.nurture_enqueue(&body.access_tokens, &body.prompt, &emails))
}

async fn nurture_process_one(
    State(st): State<Arc<AppState>>,
    Json(body): Json<NurtureProcessOneIn>,
) -> Json<Value> {
    let job = st.ops.nurture_process_one_sync(&body.access_token);
    if let Some(job) = job {
        match nurture::run_text_nurture(&st.http, &job.access_token, &job.prompt).await {
            Ok(v) => {
                st.ops.record_nurture_success();
                Json(v)
            }
            Err(e) => {
                st.ops.record_nurture_error(e.to_string());
                Json(json!({ "ok": false, "error": e.to_string() }))
            }
        }
    } else {
        Json(json!({ "ok": false, "error": "queue empty" }))
    }
}

async fn outlook_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(st.ops.outlook_status())
}

async fn outlook_settings(
    State(st): State<Arc<AppState>>,
    Json(body): Json<Map<String, Value>>,
) -> Json<Value> {
    Json(st.ops.outlook_settings(body))
}

async fn outlook_recover_one(
    State(st): State<Arc<AppState>>,
    Json(body): Json<OutlookRecoverIn>,
) -> Json<Value> {
    Json(st.ops.outlook_recover_one(&body.access_token, body.account))
}

async fn outlook_progress(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    st.ops
        .outlook_progress(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn quota_prime(
    State(st): State<Arc<AppState>>,
    Json(body): Json<QuotaPrimeIn>,
) -> Json<Value> {
    let emails = token_email_map(&body.accounts);
    Json(st.ops.quota_prime_enqueue(body.access_tokens, &emails))
}

async fn quota_prime_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(st.ops.quota_prime_status())
}

async fn proxy_runtime_get(State(st): State<Arc<AppState>>) -> Result<Json<Value>, StatusCode> {
    st.ops
        .proxy_runtime_get()
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn proxy_runtime_save(
    State(st): State<Arc<AppState>>,
    Json(body): Json<Map<String, Value>>,
) -> Result<Json<Value>, StatusCode> {
    st.ops
        .proxy_runtime_save(body)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

async fn proxy_test(
    State(st): State<Arc<AppState>>,
    Json(body): Json<Map<String, Value>>,
) -> Result<Json<Value>, StatusCode> {
    let url = body
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    st.ops
        .proxy_test(&st.http, url)
        .await
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

async fn webshare_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(st.ops.webshare_status())
}

async fn webshare_inventory(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(st.ops.webshare_inventory())
}

async fn webshare_run_once(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(st.ops.webshare_run_once(&st.http).await)
}

fn oauth_err(e: anyhow::Error) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": e.to_string() })),
    )
}
