//! HTTP surface: OpenAI-compatible chat, admin ingest, and a status page.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;

use crate::config::Config;
use crate::model::{AccountView, Health, ImportRequest, Provider};
use crate::pool::{downcast_failure, Pool};

pub struct AppState {
    pub pool: Pool,
    pub config: Config,
}

pub type Shared = Arc<AppState>;

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/", get(admin_page))
        .route("/admin", get(admin_page))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses))
        .route("/api/v1/stats", get(stats))
        .route("/api/v1/accounts", get(list_accounts).post(import_accounts))
        .route("/api/v1/accounts/{id}/health", post(set_health))
        .route("/api/v1/sweep", post(sweep))
        .with_state(state)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get("authorization")?.to_str().ok()
}

fn deny() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": {"message": "unauthorized", "type": "invalid_request_error"}})),
    )
        .into_response()
}

fn upstream_error(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(json!({"error": {"message": message, "type": "upstream_error"}})),
    )
        .into_response()
}

async fn healthz() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

async fn readyz(State(state): State<Shared>) -> Response {
    match state.pool.healthy_count() {
        // Ready means "can serve a request now"; an empty pool must not pass a
        // load balancer's health gate.
        Ok(count) if count > 0 => {
            Json(json!({"status": "ready", "accounts": count})).into_response()
        }
        Ok(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "no_accounts", "accounts": 0})),
        )
            .into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "error", "detail": err.to_string()})),
        )
            .into_response(),
    }
}

async fn list_models(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if !Config::authorizes(&state.config.api_key, bearer(&headers)) {
        return deny();
    }
    let models = state.pool.advertised_models().unwrap_or_default();
    let now = crate::now();
    let data: Vec<Value> = models
        .into_iter()
        .map(|id| json!({"id": id, "object": "model", "created": now, "owned_by": "xai"}))
        .collect();
    Json(json!({"object": "list", "data": data})).into_response()
}

/// Forward a chat request, retrying on the next account when the failure is
/// the account's fault rather than the caller's.
async fn chat_completions(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(mut payload): Json<Value>,
) -> Response {
    if !Config::authorizes(&state.config.api_key, bearer(&headers)) {
        return deny();
    }
    let requested_model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let mut last_error = "no account could serve the request".to_string();
    for _ in 0..state.config.max_attempts {
        let lease = match state.pool.acquire_build().await {
            Ok(lease) => lease,
            Err(err) => return upstream_error(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
        };
        let account = lease.account;

        let model = resolve_model(&state, &account, &requested_model).await;
        payload["model"] = Value::from(model.clone());

        match state
            .pool
            .upstream()
            .chat_completions(
                &account.access_token,
                &account.proxy_url,
                &account.headers,
                &payload,
            )
            .await
        {
            Ok(body) => {
                let _ = state.pool.report_success(&account, &model);
                return Json(body).into_response();
            }
            Err(err) => {
                let failure = downcast_failure(&err);
                last_error = err.to_string();
                warn!(account = %account.email, error = %last_error, "chat failed");
                let _ = state.pool.report_failure(&account, &failure, &last_error);
            }
        }
    }
    upstream_error(StatusCode::BAD_GATEWAY, last_error)
}

async fn responses(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(mut payload): Json<Value>,
) -> Response {
    if !Config::authorizes(&state.config.api_key, bearer(&headers)) {
        return deny();
    }
    let requested_model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let mut last_error = "no account could serve the request".to_string();
    for _ in 0..state.config.max_attempts {
        let lease = match state.pool.acquire_build().await {
            Ok(lease) => lease,
            Err(err) => return upstream_error(StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
        };
        let account = lease.account;
        let model = resolve_model(&state, &account, &requested_model).await;
        payload["model"] = Value::from(model.clone());

        match state
            .pool
            .upstream()
            .responses(
                &account.access_token,
                &account.proxy_url,
                &account.headers,
                &payload,
            )
            .await
        {
            Ok(body) => {
                let _ = state.pool.report_success(&account, &model);
                return Json(body).into_response();
            }
            Err(err) => {
                let failure = downcast_failure(&err);
                last_error = err.to_string();
                let _ = state.pool.report_failure(&account, &failure, &last_error);
            }
        }
    }
    upstream_error(StatusCode::BAD_GATEWAY, last_error)
}

/// Decide which model id to send upstream.
///
/// Callers routinely ask for a stale id (`grok-4.5`) after upstream renamed it,
/// so an explicit request is honoured only when the account has never reported
/// something newer.
async fn resolve_model(state: &Shared, account: &crate::model::Account, requested: &str) -> String {
    if !account.last_model.is_empty() {
        return account.last_model.clone();
    }
    match state
        .pool
        .upstream()
        .list_models(&account.access_token, &account.proxy_url, &account.headers)
        .await
    {
        Ok(ids) => crate::upstream::pick_chat_model(&ids).unwrap_or_else(|| {
            if requested.is_empty() {
                crate::upstream::FALLBACK_MODEL.to_string()
            } else {
                requested.to_string()
            }
        }),
        Err(_) if !requested.is_empty() => requested.to_string(),
        Err(_) => crate::upstream::FALLBACK_MODEL.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(default)]
    provider: Option<String>,
}

async fn list_accounts(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Response {
    if !Config::authorizes(&state.config.admin_key, bearer(&headers)) {
        return deny();
    }
    let provider = query.provider.as_deref().and_then(Provider::parse);
    match state.pool.store().list(provider) {
        Ok(accounts) => {
            let views: Vec<AccountView> = accounts.iter().map(AccountView::from).collect();
            Json(json!({"accounts": views, "count": views.len()})).into_response()
        }
        Err(err) => upstream_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

async fn import_accounts(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(request): Json<ImportRequest>,
) -> Response {
    if !Config::authorizes(&state.config.admin_key, bearer(&headers)) {
        return deny();
    }
    let default_provider = request.provider.as_deref().and_then(Provider::parse);
    match state
        .pool
        .store()
        .import(default_provider, &request.accounts, crate::now())
    {
        Ok(outcome) => Json(json!({"ok": true, "result": outcome})).into_response(),
        Err(err) => upstream_error(StatusCode::BAD_REQUEST, err.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct HealthUpdate {
    health: String,
}

async fn set_health(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(update): Json<HealthUpdate>,
) -> Response {
    if !Config::authorizes(&state.config.admin_key, bearer(&headers)) {
        return deny();
    }
    let health = Health::parse(&update.health);
    if let Err(err) = state
        .pool
        .store()
        .mark_health(id, health, 0, "manual override", crate::now())
    {
        return upstream_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
    }
    // Read back rather than echoing the request: a bad id would otherwise
    // report success for an account that does not exist.
    match state.pool.store().get(id) {
        Ok(Some(account)) => {
            Json(json!({"ok": true, "account": AccountView::from(&account)})).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"message": format!("no account {id}")}})),
        )
            .into_response(),
        Err(err) => upstream_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct SweepQuery {
    /// 0 = every candidate.
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    concurrency: Option<usize>,
}

/// Refresh the pool once so dead credentials stop costing user requests.
async fn sweep(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<SweepQuery>,
) -> Response {
    if !Config::authorizes(&state.config.admin_key, bearer(&headers)) {
        return deny();
    }
    match state
        .pool
        .sweep(query.limit.unwrap_or(0), query.concurrency.unwrap_or(8))
        .await
    {
        Ok(report) => Json(json!({"ok": true, "report": report})).into_response(),
        Err(err) => upstream_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

async fn stats(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if !Config::authorizes(&state.config.admin_key, bearer(&headers)) {
        return deny();
    }
    match state.pool.store().stats() {
        Ok(value) => Json(value).into_response(),
        Err(err) => upstream_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

/// Single-file status page. Fetches through the same admin key the API uses,
/// so there is no second auth path to keep in sync.
async fn admin_page() -> Html<&'static str> {
    Html(include_str!("admin.html"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::upstream::Upstream;

    fn state() -> Shared {
        let store = Store::open_in_memory().unwrap();
        let upstream = Upstream::new(crate::upstream::DEFAULT_BASE_URL, 5);
        Arc::new(AppState {
            pool: Pool::new(store, upstream, 2),
            config: Config {
                listen: "127.0.0.1:0".into(),
                database_path: ":memory:".into(),
                base_url: crate::upstream::DEFAULT_BASE_URL.into(),
                api_key: String::new(),
                admin_key: "admin-key".into(),
                upstream_timeout_secs: 5,
                max_attempts: 2,
                default_proxy: String::new(),
                sticky_relay: String::new(),
            },
        })
    }

    #[tokio::test]
    async fn readyz_fails_while_the_pool_is_empty() {
        let response = readyz(State(state())).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn admin_endpoints_require_the_admin_key() {
        let response = list_accounts(
            State(state()),
            HeaderMap::new(),
            Query(ListQuery { provider: None }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn import_then_list_round_trips() {
        let shared = state();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer admin-key".parse().unwrap());

        let body: ImportRequest = serde_json::from_value(serde_json::json!({
            "provider": "build",
            "accounts": [{"email": "a@b.c", "refresh_token": "r1"}]
        }))
        .unwrap();
        let response = import_accounts(State(shared.clone()), headers.clone(), Json(body)).await;
        assert_eq!(response.status(), StatusCode::OK);

        let listed = list_accounts(
            State(shared.clone()),
            headers,
            Query(ListQuery { provider: None }),
        )
        .await;
        assert_eq!(listed.status(), StatusCode::OK);
        assert_eq!(shared.pool.store().list(None).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn account_view_never_serializes_tokens() {
        let shared = state();
        let import: ImportRequest = serde_json::from_value(serde_json::json!({
            "provider": "build",
            "accounts": [{"email": "a@b.c", "refresh_token": "super-secret"}]
        }))
        .unwrap();
        shared
            .pool
            .store()
            .import(Some(Provider::Build), &import.accounts, 1)
            .unwrap();

        let account = shared.pool.store().list(None).unwrap().remove(0);
        let rendered = serde_json::to_string(&AccountView::from(&account)).unwrap();
        assert!(!rendered.contains("super-secret"));
    }

    #[tokio::test]
    async fn models_endpoint_falls_back_before_any_traffic() {
        let response = list_models(State(state()), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn set_health_on_a_missing_account_is_404() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer admin-key".parse().unwrap());
        let response = set_health(
            State(state()),
            headers,
            Path(9_999),
            Json(HealthUpdate {
                health: "disabled".into(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn set_health_reports_the_stored_state() {
        let shared = state();
        let import: ImportRequest = serde_json::from_value(serde_json::json!({
            "provider": "build",
            "accounts": [{"email": "a@b.c", "refresh_token": "r1"}]
        }))
        .unwrap();
        shared
            .pool
            .store()
            .import(Some(Provider::Build), &import.accounts, 1)
            .unwrap();
        let id = shared.pool.store().list(None).unwrap()[0].id;

        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer admin-key".parse().unwrap());
        let response = set_health(
            State(shared.clone()),
            headers,
            Path(id),
            Json(HealthUpdate {
                health: "disabled".into(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            shared.pool.store().get(id).unwrap().unwrap().health,
            Health::Disabled
        );
    }

    #[tokio::test]
    async fn chat_without_accounts_is_503_not_500() {
        let response = chat_completions(
            State(state()),
            HeaderMap::new(),
            Json(serde_json::json!({"model": "grok-4.6"})),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
