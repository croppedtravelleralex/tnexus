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
        .route("/v1/messages", post(messages))
        .route("/api/v1/stats", get(stats))
        .route("/api/v1/accounts", get(list_accounts).post(import_accounts))
        .route("/api/v1/accounts/{id}/health", post(set_health))
        .route("/api/v1/sweep", post(sweep))
        .route("/api/v1/quota", post(quota))
        .route("/api/v1/mint", post(mint))
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

/// Which upstream endpoint a forwarded request should land on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Endpoint {
    Chat,
    Responses,
}

/// The outcome of a forwarded request, before it is dressed in whichever
/// protocol the caller speaks.
struct Forwarded {
    body: Value,
    model: String,
}

/// Send a request upstream, moving to the next account when the failure is the
/// account's fault rather than the caller's.
///
/// Every protocol the proxy speaks funnels through here, so failover, model
/// resolution and usage accounting cannot drift between them.
async fn forward(
    state: &Shared,
    endpoint: Endpoint,
    mut payload: Value,
) -> Result<Forwarded, (StatusCode, String)> {
    let requested_model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let mut last_error = "no account could serve the request".to_string();
    for _ in 0..state.config.max_attempts {
        let lease = match state.pool.acquire_build().await {
            Ok(lease) => lease,
            Err(err) => return Err((StatusCode::SERVICE_UNAVAILABLE, err.to_string())),
        };
        let account = &lease.account;
        let model = resolve_model(state, account, &requested_model).await;
        payload["model"] = Value::from(model.clone());

        let upstream = state.pool.upstream();
        let sent = match endpoint {
            Endpoint::Chat => {
                upstream
                    .chat_completions(
                        &account.access_token,
                        &account.proxy_url,
                        &account.headers,
                        &payload,
                    )
                    .await
            }
            Endpoint::Responses => {
                upstream
                    .responses(
                        &account.access_token,
                        &account.proxy_url,
                        &account.headers,
                        &payload,
                    )
                    .await
            }
        };

        match sent {
            Ok(outcome) => {
                let _ = state
                    .pool
                    .report_success_with_usage(account, &model, &outcome);
                return Ok(Forwarded {
                    body: outcome.body,
                    model,
                });
            }
            Err(err) => {
                let failure = downcast_failure(&err);
                last_error = err.to_string();
                warn!(account = %account.email, error = %last_error, "upstream call failed");
                let _ = state.pool.report_failure(account, &failure, &last_error);
            }
        }
    }
    Err((StatusCode::BAD_GATEWAY, last_error))
}

async fn chat_completions(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    if !Config::authorizes(&state.config.api_key, bearer(&headers)) {
        return deny();
    }
    match forward(&state, Endpoint::Chat, payload).await {
        Ok(forwarded) => Json(forwarded.body).into_response(),
        Err((status, message)) => upstream_error(status, message),
    }
}

async fn responses(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    if !Config::authorizes(&state.config.api_key, bearer(&headers)) {
        return deny();
    }
    match forward(&state, Endpoint::Responses, payload).await {
        Ok(forwarded) => Json(forwarded.body).into_response(),
        Err((status, message)) => upstream_error(status, message),
    }
}

/// Anthropic Messages API. The upstream speaks only the OpenAI shape, so the
/// request and reply are translated around the same forwarding path.
async fn messages(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    // Anthropic clients send the key as `x-api-key`, not a bearer header.
    let presented = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .or_else(|| bearer(&headers));
    if !Config::authorizes(&state.config.api_key, presented) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(crate::anthropic::error_body("unauthorized")),
        )
            .into_response();
    }

    let openai = crate::anthropic::request_to_openai(&payload);
    match forward(&state, Endpoint::Chat, openai).await {
        Ok(forwarded) => Json(crate::anthropic::response_to_anthropic(
            &forwarded.body,
            &forwarded.model,
        ))
        .into_response(),
        Err((status, message)) => {
            (status, Json(crate::anthropic::error_body(&message))).into_response()
        }
    }
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

#[derive(Debug, Default, Deserialize)]
struct ListQuery {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    health: Option<String>,
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

async fn list_accounts(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Response {
    if !Config::authorizes(&state.config.admin_key, bearer(&headers)) {
        return deny();
    }
    let filter = crate::store::AccountQuery {
        provider: query.provider.as_deref().and_then(Provider::parse),
        health: query
            .health
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(Health::parse),
        search: query.search.filter(|value| !value.trim().is_empty()),
        limit: query.limit.unwrap_or(50),
        offset: query.offset.unwrap_or(0),
    };
    match state.pool.store().query(&filter) {
        Ok((accounts, total)) => {
            let views: Vec<AccountView> = accounts.iter().map(AccountView::from).collect();
            Json(json!({
                "accounts": views,
                "count": views.len(),
                "total": total,
                "offset": filter.offset,
                "limit": filter.limit,
            }))
            .into_response()
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

/// Keep-alive pass: the cheapest probe that still teaches us something about
/// each account, so a routine sweep does not spend the pool's own budget.
async fn sweep(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<SweepQuery>,
) -> Response {
    run_probe(state, headers, query, None).await
}

/// Chat-probe the pool: which accounts can actually generate right now, and
/// what entitlement the upstream reports for them.
async fn quota(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<SweepQuery>,
) -> Response {
    run_probe(state, headers, query, Some(crate::probe::Probe::Chat)).await
}

async fn run_probe(
    state: Shared,
    headers: HeaderMap,
    query: SweepQuery,
    probe: Option<crate::probe::Probe>,
) -> Response {
    if !Config::authorizes(&state.config.admin_key, bearer(&headers)) {
        return deny();
    }
    match state
        .pool
        .probe_pool(
            probe,
            query.limit.unwrap_or(0),
            query.concurrency.unwrap_or(8),
        )
        .await
    {
        Ok(report) => Json(json!({"ok": true, "report": report})).into_response(),
        Err(err) => upstream_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

/// Exchange a web SSO cookie for a Build credential and keep it.
///
/// The registration machine used to mint to a JSON file and ship it here as a
/// separate step. Doing the exchange in-process removes that hop entirely: the
/// browser's only remaining job is to produce the cookie.
async fn mint(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(request): Json<crate::xai::mint::MintRequest>,
) -> Response {
    if !Config::authorizes(&state.config.admin_key, bearer(&headers)) {
        return deny();
    }
    match crate::xai::mint::mint(state.pool.store(), &request).await {
        Ok(outcome) => Json(json!({"ok": true, "account": outcome})).into_response(),
        Err(err) => {
            warn!(email = %request.email, error = %format!("{err:#}"), "mint failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok": false, "error": format!("{err:#}")})),
            )
                .into_response()
        }
    }
}

async fn stats(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if !Config::authorizes(&state.config.admin_key, bearer(&headers)) {
        return deny();
    }
    match state.pool.store().stats() {
        Ok(mut value) => {
            let (queued, in_flight) = state.pool.ready_depth();
            value["scheduler"] = json!({"queued": queued, "in_flight": in_flight});
            Json(value).into_response()
        }
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
                sweep_interval_secs: 0,
                sweep_batch: 0,
                sweep_concurrency: 1,
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
            Query(ListQuery::default()),
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

        let listed =
            list_accounts(State(shared.clone()), headers, Query(ListQuery::default())).await;
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
    async fn the_anthropic_endpoint_accepts_an_x_api_key() {
        // Anthropic clients send `x-api-key`, so requiring a bearer header
        // would reject every well-formed request.
        let shared = state();
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "anything".parse().unwrap());
        let response = messages(
            State(shared),
            headers,
            Json(serde_json::json!({"model": "grok-4.6", "messages": []})),
        )
        .await;
        // The key is accepted (api_key is empty in tests); the pool is what is
        // empty, so this must not be a 401.
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn anthropic_failures_use_the_anthropic_error_envelope() {
        // An OpenAI-shaped error here reads to an Anthropic client as a
        // malformed response rather than a failure it can report.
        let shared = state();
        let response = messages(
            State(shared),
            HeaderMap::new(),
            Json(serde_json::json!({"messages": []})),
        )
        .await;
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["error"]["type"], "api_error");
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
