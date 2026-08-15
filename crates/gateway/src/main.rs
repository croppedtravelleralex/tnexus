//! gptimage-gateway-rs MVP gateway (Rust face).

mod accounts_routes;
mod auth_routes;
mod backend_routes;
mod config;
mod duplicate_prompt;
mod image_archive;
mod image_assets;
mod image_tasks;
mod pipeline_telemetry;
mod scheduling_gate;
mod state;
mod upstream_face;

use crate::image_assets::ImageAssetStore;
use crate::scheduling_gate::SchedulingGate;
use accounts_routes::{activity_daily, list_accounts, reload_from_storage, scheduling_bulk};
use anyhow::Context;
use auth_routes::{
    list_users, login, logout, me, register, require_admin, require_auth, require_member,
    set_user_disabled,
};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use backend_routes::{admin_status, capabilities};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use config::DataPlane;
use futures_util::StreamExt;
use gateway_auth::{AuthConfig, AuthService};
use helper_client::{
    HelperClient, ImageRunRequest, PinAccount, QuotaRefreshRequest, TextRunRequest,
};
use image_schedule::{
    apply_runtime_snapshot, default_epsilon, pick_account_index, poisson_delay_ms, AccountScoreInput,
    BindingInflightLedger, CooldownRegistry, DeadlockGuard, DispatchIntervalGate,
    DispatchMarkGuard, ImageRuntimeConfig, PipelineWatchdog, PreTicketPool, ProxyCfRegistry,
    ReadyBuffer, ReadyBufferGuard, ReturnWindow, ReturnWindowPermit, SlotLedger, WorkloadPolicy,
};
use protocol::{
    chat_completion_response, chat_completion_response_with_image_b64, chat_should_use_image_path,
    classify_fault, extract_chat_image_prompt, fold_chat_messages_for_upstream,
    image_generation_b64_multi_response, image_generation_b64_multi_response_with_pipeline,
    image_generation_response, image_generation_url_multi_response,
    image_generation_url_multi_response_with_pipeline, image_generation_url_response,
    image_task_queued_response, image_task_status_response, openai_error,
    parse_image_prompt_tunnel, ImagePromptTunnel,
    ChatCompletionRequest, ImageEditRequest, ImageGenerationRequest, MAX_IMAGE_BATCH_N,
};
use serde_json::{json, Value};
use state::AppState;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tnexus_accounts_db::AccountsBackend;
use tokio::sync::Mutex;
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};
use tracing::{error, info, warn};
use uuid::Uuid;

/// Axum default is 2MB; image edits multipart must accept large PNG uploads (nginx allows 256m).
const MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Upper bound for the folded chat history sent upstream as a single message.
const MAX_FOLDED_PROMPT_CHARS: usize = 30_000;

/// Build the CORS layer from a comma-separated origin allowlist.
///
/// tower-http rejects `Access-Control-Allow-Credentials: true` alongside a
/// wildcard in *any* of origin, methods, or headers — each combination is a
/// separate assert. The credentialed branch therefore has to enumerate all
/// three explicitly.
fn cors_layer_from(spec: &str) -> CorsLayer {
    let origins: Vec<_> = spec
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<header::HeaderValue>().ok())
        .collect();

    if origins.is_empty() {
        return CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);
    }

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_credentials(true)
}

/// Read `GATEWAY_CORS_ORIGINS` and build the layer.
fn cors_layer() -> CorsLayer {
    let spec = std::env::var("GATEWAY_CORS_ORIGINS").unwrap_or_default();
    if spec.trim().is_empty() {
        warn!("GATEWAY_CORS_ORIGINS unset; CORS runs without credentials");
    } else {
        info!("CORS allowlist active with credentials");
    }
    cors_layer_from(&spec)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gateway=info,tower_http=info".into()),
        )
        .init();

    let cfg = config::load().context("load config")?;
    let auth_cfg = AuthConfig::from_env().context("auth config")?;
    let auth_svc = Arc::new(AuthService::open(auth_cfg).context("open auth db")?);

    let helper = HelperClient::new(&cfg.helper_url)?;
    if !helper.has_token() {
        warn!(
            "HELPER_INTERNAL_TOKEN unset — helper fails closed on /v1/internal/*, \
             so chat and image will return 503"
        );
    }
    match helper.health().await {
        Ok(h) => info!(?h, "helper healthy"),
        Err(e) => tracing::warn!(error=%e, "helper health check failed (will retry on request)"),
    }

    let mut accounts = cfg.accounts;
    match helper.list_candidates(12).await {
        Ok(cands) => {
            for a in cands {
                accounts.insert(a.email.to_lowercase(), a.to_pin());
            }
            info!(
                n = accounts.len(),
                "accounts ready (pin + helper candidates)"
            );
        }
        Err(e) => warn!(error=%e, "helper candidates unavailable; using pin/ACCOUNTS_DB only"),
    }

    let static_dir = std::env::var("GATEWAY_STATIC_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir());

    let asset_secret =
        image_assets::asset_signing_secret_from_env().context("image asset signing secret")?;
    let image_assets = Arc::new(ImageAssetStore::new(
        asset_secret,
        image_assets::asset_ttl_secs_from_env(),
    ));

    let image_archive_store = tnexus_storage::ImageStore::from_env()
        .await
        .context("image archive store")?;

    let pg_pool = if std::env::var("ACCOUNTS_BACKEND").ok().as_deref() == Some("postgres") {
        let url = std::env::var("DATABASE_URL").context("DATABASE_URL for postgres accounts")?;
        Some(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(5)
                .connect(&url)
                .await
                .context("connect postgres for accounts backend")?,
        )
    } else {
        None
    };
    let scheduling_gate = if let Some(pool) = pg_pool.clone() {
        SchedulingGate::from_backend(AccountsBackend::from_env(Some(pool))?)
    } else {
        SchedulingGate::from_env()
    };

    for pin in scheduling_gate.list_all_pins() {
        accounts.insert(pin.email.to_lowercase(), pin);
    }
    info!(
        n = accounts.len(),
        schedulable = scheduling_gate.schedulable_count(),
        pool = scheduling_gate.pool_account_count(),
        "accounts hydrated from pool backend"
    );

    let binding_ledger = BindingInflightLedger::from_env();
    let dispatch_interval = DispatchIntervalGate::from_env();
    let slot_ledger = SlotLedger::from_env();
    let ready_buffer = ReadyBuffer::from_env();
    let return_window = ReturnWindow::from_env();
    let cooldown_registry = CooldownRegistry::from_env();
    let pre_ticket = PreTicketPool::from_env();
    let proxy_cf = ProxyCfRegistry::from_env();
    let workload = WorkloadPolicy::from_env();
    let image_runtime = ImageRuntimeConfig::from_env(cfg.image_enabled);
    let deadlock_guard = DeadlockGuard::from_env();
    let pipeline_watchdog = PipelineWatchdog::from_env();
    let runtime_binding = binding_ledger.clone();
    let runtime_dispatch = dispatch_interval.clone();
    let runtime_reload = image_runtime.clone();
    let deadlock_sample = deadlock_guard.clone();

    let state_holder: Arc<OnceLock<Arc<AppState>>> = Arc::new(OnceLock::new());
    let holder_for_worker = state_holder.clone();
    let submit_workers = std::env::var("IMAGE_SUBMIT_WORKERS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10);
    let (image_tasks, _) = {
        let svc = image_tasks::ImageTaskService::spawn(
            move |task_id| {
                let holder = holder_for_worker.clone();
                async move {
                    if let Some(st) = holder.get() {
                        process_image_task(st.clone(), task_id).await;
                    } else {
                        warn!(task_id = %task_id, "image task worker: app state not ready");
                    }
                }
            },
            submit_workers,
        );
        (svc, ())
    };

    let state = Arc::new(AppState {
        helper,
        data_plane: cfg.data_plane,
        pin: cfg.account,
        accounts: Arc::new(Mutex::new(accounts)),
        listen: cfg.listen.clone(),
        min_image_quota: cfg.min_image_quota,
        image_global_concurrency: cfg.image_global_concurrency,
        image_sem: cfg.image_sem,
        image_enabled: cfg.image_enabled,
        image_runtime,
        deadlock_guard,
        pipeline_watchdog,
        auth: auth_svc,
        static_dir: static_dir.clone(),
        image_assets,
        public_base_url: cfg.public_base_url.clone(),
        scheduling_gate,
        image_account_rr: AtomicUsize::new(0),
        image_queue_depth: AtomicUsize::new(0),
        duplicate_prompt: duplicate_prompt::DuplicatePromptGate::new(),
        binding_inflight: binding_ledger,
        dispatch_interval,
        slot_ledger,
        ready_buffer,
        return_window,
        cooldown: cooldown_registry,
        pre_ticket,
        proxy_cf,
        workload,
        image_tasks,
        pg_pool,
        image_archive_store: image_archive_store.map(Arc::new),
    });
    let _ = state_holder.set(state.clone());

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(runtime_reload.poll_interval()).await;
            runtime_reload.reload();
            let snap = runtime_reload.snapshot();
            apply_runtime_snapshot(&snap, &runtime_binding, &runtime_dispatch);
        }
    });
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            deadlock_sample.sample_process_cpu();
        }
    });
    let reconcile_st = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            reconcile_st.scheduling_gate.reconcile_stale_inflight();
            reconcile_st.binding_inflight.reconcile_above(8);
            reconcile_st.dispatch_interval.reconcile_stale(std::time::Duration::from_secs(3600));
            reconcile_st.cooldown.reconcile();
            reconcile_st.slot_ledger.watchdog_tick(false);
            reconcile_st.proxy_cf.reconcile();
            let (queued, running) = reconcile_st.image_tasks.store.count_states();
            reconcile_st.pipeline_watchdog.evaluate(queued, running);
        }
    });

    let auth_public = Router::new()
        .route("/login", post(login))
        .route("/register", post(register));

    let auth_protected = Router::new()
        .route("/logout", post(logout))
        .route("/me", get(me))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let admin_api = Router::new()
        .route("/users", get(list_users))
        .route("/users/{user_id}/disabled", post(set_user_disabled))
        .route("/status", get(admin_status))
        .route("/image-runtime/reload", post(reload_image_runtime))
        .layer(middleware::from_fn(require_admin))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let member_api = Router::new()
        .route("/models", get(models))
        .route("/chat/completions", post(chat_completions))
        .route("/images/generations", post(image_generations))
        .route("/images/edits", post(image_edits))
        .route("/images/tasks/{task_id}", get(get_image_task))
        .route("/image-tasks/generations", post(post_image_tasks_generations))
        .layer(middleware::from_fn(require_member))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let admin_v1 = Router::new()
        .route("/accounts/candidates", get(account_candidates))
        .route("/quota", get(quota_refresh))
        .route("/quota/refresh", post(quota_refresh))
        .layer(middleware::from_fn(require_admin))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let admin_accounts = Router::new()
        .route("/", get(list_accounts))
        .route("/reload-from-storage", post(reload_from_storage))
        .route("/activity/daily", get(activity_daily))
        .route("/scheduling/bulk", post(scheduling_bulk))
        .layer(middleware::from_fn(require_admin))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let mut app = Router::new()
        .route("/health", get(health))
        .route("/api/backend/capabilities", get(capabilities))
        .route("/v1/images/assets/{asset_id}", get(get_image_asset))
        .nest("/api/auth", auth_public.merge(auth_protected))
        .nest("/api/admin", admin_api)
        .nest("/api/accounts", admin_accounts)
        .nest("/v1", member_api.merge(admin_v1))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    if let Some(dir) = static_dir {
        info!(path=%dir.display(), "serving static UI");
        app = app.fallback_service(ServeDir::new(dir).append_index_html_on_directories(true));
    }

    let listener = tokio::net::TcpListener::bind(&cfg.listen)
        .await
        .with_context(|| format!("bind {}", cfg.listen))?;
    info!(
        listen=%cfg.listen,
        helper=%cfg.helper_url,
        data_plane=%cfg.data_plane.as_str(),
        email=%cfg.account_email_log,
        image_global_concurrency=%cfg.image_global_concurrency,
        image_enabled=%cfg.image_enabled,
        auth_disabled=%state.auth.config().auth_disabled(),
        auth_mode=%state.auth.config().mode.as_str(),
        "gateway listening (rust)"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

async fn helper_liveness(st: &AppState) -> bool {
    if st.data_plane == DataPlane::Upstream && st.image_enabled {
        return true;
    }
    st.helper.health().await.is_ok()
}

async fn health(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let helper_ok = helper_liveness(&st).await;
    let pool_total = st.scheduling_gate.pool_account_count();
    let n_accounts = st.scheduling_gate.schedulable_count();
    // Unauthenticated endpoint: report liveness and shape, never pool identities.
    Json(json!({
        "ok": true,
        "service": "gptimage-gateway-rs",
        "wave": "mvp",
        "runtime": "rust",
        "proto_bridge": true,
        "helper_ok": helper_ok,
        "accounts": n_accounts,
        "pool_total": pool_total,
        "image_enabled": st.image_enabled,
        "auth_disabled": st.auth.config().auth_disabled(),
        "auth_mode": st.auth.config().mode.as_str(),
        "static_ui": st.static_dir.is_some(),
    }))
}

async fn models() -> impl IntoResponse {
    Json(json!({
        "object": "list",
        "data": [
            { "id": "gpt-4o-mini", "object": "model", "owned_by": "gptimage-gateway-rs" },
            { "id": "gpt-image-2", "object": "model", "owned_by": "gptimage-gateway-rs" }
        ]
    }))
}

async fn account_candidates(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.helper.list_candidates(20).await {
        Ok(list) => {
            let mut guard = st.accounts.lock().await;
            for a in &list {
                guard.insert(a.email.to_lowercase(), a.to_pin());
            }
            let accounts: Vec<Value> = list
                .into_iter()
                .map(|a| {
                    json!({
                        "email": a.email,
                        "proxy_host": a.proxy_host.unwrap_or_default(),
                        "has_token": a.has_token,
                        "status": a.status,
                    })
                })
                .collect();
            Json(json!({"ok": true, "count": accounts.len(), "accounts": accounts})).into_response()
        }
        Err(e) => err(
            StatusCode::BAD_GATEWAY,
            e.to_string(),
            "candidates_failed",
            Some("self"),
        ),
    }
}

/// Resolve which pool account serves this request.
///
/// `X-Preferred-Account-Email` is honoured for admins only — a member picking an
/// arbitrary pool address would burn someone else's quota through their proxy
/// and token. Members always get the pin account.
async fn resolve_account(
    st: &AppState,
    preferred: Option<String>,
    is_admin: bool,
) -> Result<PinAccount, Response> {
    let email = match preferred.filter(|s| !s.is_empty()) {
        Some(e) if is_admin => e,
        Some(_) => {
            return Err(err(
                StatusCode::FORBIDDEN,
                "X-Preferred-Account-Email requires admin",
                "account_override_forbidden",
                Some("client"),
            ))
        }
        None => st.pin.email.clone(),
    };
    let key = email.to_lowercase();
    if let Some(acc) = st.accounts.lock().await.get(&key).cloned() {
        return Ok(acc);
    }
    if let Ok(list) = st.helper.list_candidates(30).await {
        let mut guard = st.accounts.lock().await;
        for a in list {
            guard.insert(a.email.to_lowercase(), a.to_pin());
        }
        if let Some(acc) = guard.get(&key).cloned() {
            return Ok(acc);
        }
    }
    // Previously this fabricated an account with an empty token and let the
    // request continue, surfacing as an opaque upstream failure.
    Err(err(
        StatusCode::NOT_FOUND,
        format!("account not in pool: {key}"),
        "account_not_found",
        Some("client"),
    ))
}

/// Admin image requests may try multiple pool accounts when upstream SSE fails.
fn pin_with_pre_ticket(st: &AppState, account: &PinAccount) -> PinAccount {
    if !account.access_token.is_empty() {
        st.pre_ticket.put(&account.email, account.access_token.clone());
        return account.clone();
    }
    if let Some(tok) = st.pre_ticket.get(&account.email) {
        return PinAccount {
            access_token: tok,
            ..account.clone()
        };
    }
    account.clone()
}

fn record_image_upstream_failure(st: &AppState, account: &PinAccount, err: &anyhow::Error) {
    let msg = err.to_string();
    let binding = st
        .scheduling_gate
        .account_binding_key(&account.email, account.proxy.as_deref());
    st.proxy_cf.record_from_error(&binding, &msg);
    let lower = msg.to_lowercase();
    if lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
    {
        st.cooldown.record_rate_limit(&account.email);
    }
    if lower.contains("token_invalidated")
        || lower.contains("authentication token has been invalidated")
        || lower.contains("content policy")
        || lower.contains("moderation")
    {
        st.cooldown.record_terminal(&account.email);
        st.pre_ticket.invalidate(&account.email);
    }
}

async fn reload_image_runtime(State(st): State<Arc<AppState>>) -> Json<serde_json::Value> {
    st.image_runtime.reload();
    let snap = st.image_runtime.snapshot();
    apply_runtime_snapshot(&snap, &st.binding_inflight, &st.dispatch_interval);
    Json(serde_json::json!({
        "ok": true,
        "snapshot": snap,
    }))
}

async fn collect_image_accounts(
    st: &AppState,
    preferred: Option<String>,
    is_admin: bool,
) -> Result<Vec<PinAccount>, Response> {
    let mut accounts = Vec::new();
    let mut seen = HashSet::new();

    match resolve_account(st, preferred.clone(), is_admin).await {
        Ok(acc) => {
            if is_admin
                || st
                    .scheduling_gate
                    .is_email_schedulable(&acc.email, &acc.access_token)
            {
                seen.insert(acc.email.to_lowercase());
                accounts.push(acc);
            }
        }
        Err(r) if !is_admin => return Err(r),
        Err(_) => {}
    }

    if is_admin {
        for pin in st.scheduling_gate.list_schedulable_pins() {
            if seen.insert(pin.email.to_lowercase()) {
                accounts.push(pin);
            }
        }
        if accounts.len() <= 1 {
            if let Ok(list) = st.helper.list_candidates(30).await {
                let mut guard = st.accounts.lock().await;
                for a in list {
                    guard.insert(a.email.to_lowercase(), a.to_pin());
                }
            }
            let guard = st.accounts.lock().await;
            let mut keys: Vec<_> = guard.keys().cloned().collect();
            keys.sort();
            for key in keys {
                if seen.insert(key.clone()) {
                    if let Some(acc) = guard.get(&key) {
                        if st
                            .scheduling_gate
                            .is_email_schedulable(&acc.email, &acc.access_token)
                        {
                            accounts.push(acc.clone());
                        }
                    }
                }
            }
        }
    }

    if accounts.is_empty() {
        // Allow pin/preferred even when gate would block, for explicit admin override via header.
        if let Ok(acc) = resolve_account(st, preferred.clone(), is_admin).await {
            if seen.insert(acc.email.to_lowercase()) {
                accounts.push(acc);
            }
        }
    }

    if accounts.is_empty() {
        return Err(err(
            StatusCode::NOT_FOUND,
            "no accounts available for image generation",
            "account_not_found",
            Some("client"),
        ));
    }

    accounts.retain(|a| {
        if st.cooldown.is_blocked(&a.email) {
            return false;
        }
        let (_, _, inflight, soft) = st
            .scheduling_gate
            .account_metrics(&a.email)
            .unwrap_or((0, false, 0, 0));
        if !st.workload.account_eligible_for_image(inflight, soft > 0) {
            return false;
        }
        let binding = st
            .scheduling_gate
            .account_binding_key(&a.email, a.proxy.as_deref());
        if st.proxy_cf.is_blocked(&binding) {
            return false;
        }
        st.binding_inflight.is_available(&binding)
    });

    if accounts.is_empty() {
        return Err(err_wait(
            StatusCode::TOO_MANY_REQUESTS,
            "no schedulable accounts (binding/cooldown saturated)",
            "account_not_available",
            Some("gate"),
            st.estimated_image_wait_secs(),
        ));
    }

    if accounts.len() > 1 {
        let inputs: Vec<AccountScoreInput> = accounts
            .iter()
            .map(|a| {
                let binding = st
                    .scheduling_gate
                    .account_binding_key(&a.email, a.proxy.as_deref());
                let (quota, unknown, inflight, soft) = st
                    .scheduling_gate
                    .account_metrics(&a.email)
                    .unwrap_or((0, false, 0, 0));
                AccountScoreInput {
                    email: a.email.clone(),
                    quota,
                    image_quota_unknown: unknown,
                    image_inflight: inflight,
                    soft_band_percent: soft,
                    binding_inflight: st.binding_inflight.inflight(&binding),
                }
            })
            .collect();
        let epsilon = st
            .image_runtime
            .humanlike_epsilon(default_epsilon());
        let workload_mult = st.workload.image_score_multiplier();
        let start = pick_account_index(
            &inputs,
            st.image_account_rr.fetch_add(1, Ordering::Relaxed),
            epsilon,
            workload_mult,
        );
        if start > 0 {
            let mut rotated = Vec::with_capacity(accounts.len());
            for i in 0..accounts.len() {
                rotated.push(accounts[(start + i) % accounts.len()].clone());
            }
            accounts = rotated;
        }
    }
    Ok(accounts)
}

fn upstream_image_retryable(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("file_id predicate")
        || msg.contains("sse ended")
        || msg.contains("image sse ended")
        || msg.contains("upstream_unreachable")
        || msg.contains("token_invalidated")
        || msg.contains("authentication token has been invalidated")
        || msg.contains("chat_requirements_prepare http 401")
        || msg.contains("proxyconnect")
}

#[cfg(test)]
mod upstream_image_retryable_tests {
    use super::upstream_image_retryable;

    #[test]
    fn retries_token_invalidated_and_proxy_errors() {
        assert!(upstream_image_retryable(&anyhow::anyhow!(
            "chat_requirements_prepare HTTP 401 Unauthorized: token_invalidated"
        )));
        assert!(upstream_image_retryable(&anyhow::anyhow!(
            "error sending request for uri (https://chatgpt.com/): client error (ProxyConnect)"
        )));
        assert!(!upstream_image_retryable(&anyhow::anyhow!(
            "unknown upstream fault"
        )));
    }
}

/// Cap admin upstream retries
/// retried dozens of accounts sequentially (worker parallel batches use admin auth).
fn image_max_attempts(is_admin: bool, data_plane: DataPlane, candidates_len: usize) -> usize {
    if !is_admin || data_plane != DataPlane::Upstream {
        return 1;
    }
    let retry_cap = std::env::var("IMAGE_ADMIN_RETRY_MAX")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(3)
        .max(1);
    candidates_len.max(1).min(retry_cap)
}

async fn run_upstream_image(
    account: &PinAccount,
    prompt: String,
    model: String,
    size: String,
    quality: Option<String>,
    transparent_bg: bool,
    asset_ids: &[String],
) -> Result<(helper_client::BridgeOk, upstream::ImageRunMetrics), anyhow::Error> {
    let prompt = tnexus_domain::append_image_generation_hints(
        &prompt,
        &size,
        quality.as_deref().unwrap_or("auto"),
        transparent_bg,
    );
    upstream_face::run_image(account, prompt, model, asset_ids)
        .await
        .map(|(bytes, metrics)| {
            let b64 = BASE64.encode(&bytes);
            let bridge = helper_client::BridgeOk {
                ok: true,
                content: None,
                b64_json: Some(b64),
                conversation_id: None,
                fault: None,
                error: None,
                elapsed_ms: Some(metrics.wall_ms),
                raw: None,
                quota: None,
            };
            (bridge, metrics)
        })
}

fn finalize_upstream_image(
    st: &AppState,
    email: &str,
    gateway_wall_ms: u128,
    upstream: &upstream::ImageRunMetrics,
    asset_store_ms: u64,
    response_out_bytes: u64,
    prompt: &str,
) -> Value {
    let quota_change = st.scheduling_gate.decrement_quota(email).ok().flatten();
    let (quota_before, quota_after) = quota_change.unwrap_or((-1, -1));
    let pipeline = json!({
        "account_email": email,
        "quota_before": if quota_before >= 0 { Some(quota_before) } else { None },
        "quota_after": if quota_after >= 0 { Some(quota_after) } else { None },
        "timings_ms": {
            "gateway_wall_ms": gateway_wall_ms,
            "asset_store_ms": asset_store_ms,
            "bootstrap_ms": upstream.bootstrap_ms,
            "requirements_ms": upstream.requirements_ms,
            "prepare_ms": upstream.prepare_ms,
            "sse_ms": upstream.sse_ms,
            "resolve_url_ms": upstream.resolve_url_ms,
            "poll_tasks_ms": upstream.poll_tasks_ms,
            "download_ms": upstream.download_ms,
            "upstream_wall_ms": upstream.wall_ms,
            "sse_events": upstream.sse_events,
        },
        "bytes": {
            "sse_in": upstream.sse_bytes_in,
            "image_download": upstream.image_bytes,
            "response_out": response_out_bytes,
        },
    });
    pipeline_telemetry::append_event(&pipeline_telemetry::PipelineEvent {
        ts: pipeline_telemetry::now_rfc3339(),
        kind: "gateway_image".into(),
        email: email.to_string(),
        job_id: None,
        slot_index: None,
        ok: true,
        quota_before: if quota_before >= 0 {
            Some(quota_before)
        } else {
            None
        },
        quota_after: if quota_after >= 0 {
            Some(quota_after)
        } else {
            None
        },
        timings_ms: pipeline.get("timings_ms").cloned(),
        bytes: pipeline.get("bytes").cloned(),
        extra: Some(json!({
            "prompt_chars": prompt.chars().count(),
            "input_tokens_est": protocol::estimate_image_input_tokens(prompt),
        })),
    });
    pipeline
}

async fn quota_refresh(
    State(st): State<Arc<AppState>>,
    user: auth_routes::AuthUser,
    headers: HeaderMap,
) -> impl IntoResponse {
    let preferred = preferred_email(&headers);
    let account = match resolve_account(&st, preferred, user.claims.role.is_admin()).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let req = QuotaRefreshRequest {
        account,
        min_remaining: st.min_image_quota,
    };
    match st.helper.refresh_quota(&req).await {
        Ok(q) if q.ok => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "email": q.email,
                "plan": q.plan,
                "status": q.status,
                "remaining": q.remaining,
                "restore_at": q.restore_at,
                "image_quota_unknown": q.image_quota_unknown,
                "min_remaining": q.min_remaining.unwrap_or(st.min_image_quota),
                "imageable": q.imageable.unwrap_or(false),
                "image_gen": q.image_gen,
                "elapsed_ms": q.elapsed_ms,
            })),
        )
            .into_response(),
        Ok(q) => {
            let fault = q.fault.as_deref().unwrap_or("upstream");
            let msg = q.error.unwrap_or_else(|| "quota refresh failed".into());
            let code = if fault == "self" {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::BAD_GATEWAY
            };
            err(code, msg, "quota_refresh_failed", Some(fault))
        }
        Err(e) => {
            error!(error=%e, "helper quota call failed");
            err(
                StatusCode::BAD_GATEWAY,
                e.to_string(),
                "helper_unreachable",
                Some("self"),
            )
        }
    }
}

fn check_dispatch_backpressure(st: &AppState, email: &str) -> Option<Response> {
    let interval_ms = st.dispatch_interval.interval_ms();
    let cap = st.scheduling_gate.account_inflight_cap();
    let inflight = st
        .scheduling_gate
        .account_metrics(email)
        .map(|(_, _, inflight, _)| inflight)
        .unwrap_or(0);
    let queued = st.image_queue_depth() as u64;
    let since = st.dispatch_interval.since_last_dispatch_ms(email);
    if image_schedule::should_wait(interval_ms, inflight as u64, cap, queued, since) {
        return Some(err_wait(
            StatusCode::TOO_MANY_REQUESTS,
            "dispatch_gate: defer image until inflight drains",
            "dispatch_gate",
            Some("gate"),
            st.estimated_image_wait_secs(),
        ));
    }
    None
}

fn try_acquire_image_permit(st: &AppState) -> Result<tokio::sync::OwnedSemaphorePermit, Response> {
    st.image_queue_depth.fetch_add(1, Ordering::Relaxed);
    match st.image_sem.clone().try_acquire_owned() {
        Ok(permit) => {
            st.image_queue_depth.fetch_sub(1, Ordering::Relaxed);
            Ok(permit)
        }
        Err(_) => {
            st.image_queue_depth.fetch_sub(1, Ordering::Relaxed);
            Err(err_wait(
                StatusCode::TOO_MANY_REQUESTS,
                "image_service_busy: global concurrency saturated",
                "image_service_busy",
                Some("gate"),
                st.estimated_image_wait_secs(),
            ))
        }
    }
}

async fn chat_image_completions(
    st: &Arc<AppState>,
    req: &ChatCompletionRequest,
    account: &PinAccount,
    last_user: &str,
) -> Response {
    if !st.image_enabled {
        return err(
            StatusCode::NOT_IMPLEMENTED,
            "chat image requires IMAGE_ENABLED=1",
            "image_deferred",
            Some("gate"),
        );
    }
    if st.data_plane != DataPlane::Upstream {
        return err(
            StatusCode::NOT_IMPLEMENTED,
            "chat image requires DATA_PLANE=upstream",
            "image_deferred",
            Some("gate"),
        );
    }

    let permit = match try_acquire_image_permit(st) {
        Ok(p) => p,
        Err(r) => return r,
    };

    let image_prompt = extract_chat_image_prompt(last_user);
    let usage_prompt = image_prompt.clone();
    let image_model = if req.model.contains("image") {
        req.model.clone()
    } else {
        "gpt-image-2".into()
    };
    let t0 = Instant::now();
    let _inflight_guard = st.scheduling_gate.begin_inflight(&account.email);
    let attempt = run_upstream_image(
        account,
        image_prompt,
        image_model.clone(),
        "1024x1024".into(),
        None,
        false,
        &[],
    )
    .await;
    let elapsed_ms = t0.elapsed().as_millis();
    drop(permit);

    match attempt {
        Ok((bridge, metrics)) if bridge.ok => {
            let b64 = bridge.b64_json.unwrap_or_default();
            if b64.len() < 1000 {
                return err(
                    StatusCode::BAD_GATEWAY,
                    "empty/short image from chat image path",
                    "empty_image",
                    Some("self"),
                );
            }
            finalize_upstream_image(
                &st,
                &account.email,
                elapsed_ms,
                &metrics,
                0,
                b64.len() as u64,
                &usage_prompt,
            );
            info!(email=%account.email, elapsed_ms, b64_len=b64.len(), "chat image ok");
            if req.stream {
                let stream = upstream::chat_image_b64_sse_stream(&req.model, &b64)
                    .map(|chunk| chunk.map_err(std::io::Error::other));
                match Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .header(header::CONNECTION, "keep-alive")
                    .body(Body::from_stream(stream))
                {
                    Ok(r) => r,
                    Err(e) => err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        e.to_string(),
                        "stream_build_failed",
                        Some("self"),
                    ),
                }
            } else {
                (
                    StatusCode::OK,
                    Json(chat_completion_response_with_image_b64(&req.model, &b64)),
                )
                    .into_response()
            }
        }
        Ok((bridge, _)) => {
            let msg = bridge
                .error
                .unwrap_or_else(|| "chat image bridge failed".into());
            err(
                StatusCode::BAD_GATEWAY,
                msg,
                "image_failed",
                Some("upstream"),
            )
        }
        Err(e) => {
            error!(error=%e, "chat image upstream failed");
            upstream_error_response(&e, "upstream_unreachable")
        }
    }
}

async fn chat_completions(
    State(st): State<Arc<AppState>>,
    user: auth_routes::AuthUser,
    headers: HeaderMap,
    Json(req): Json<ChatCompletionRequest>,
) -> impl IntoResponse {
    let last_user = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.text())
        .unwrap_or_default();
    let prompt = fold_chat_messages_for_upstream(&req.messages);
    if prompt.trim().is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "messages must include a user text",
            "invalid_request",
            Some("client"),
        );
    }
    // Folding sends the whole history as one upstream message; reject early rather
    // than let upstream answer 413, and never truncate (that would silently drop
    // the system prompt or the newest instruction).
    if prompt.chars().count() > MAX_FOLDED_PROMPT_CHARS {
        return err(
            StatusCode::BAD_REQUEST,
            format!("folded conversation exceeds {MAX_FOLDED_PROMPT_CHARS} chars; shorten history"),
            "prompt_too_long",
            Some("client"),
        );
    }

    let account =
        match resolve_account(&st, preferred_email(&headers), user.claims.role.is_admin()).await {
            Ok(a) => a,
            Err(r) => return r,
        };

    if chat_should_use_image_path(&req.model, &last_user, req.image_mode) {
        return chat_image_completions(&st, &req, &account, &last_user).await;
    }

    let model = req.model.clone();

    if req.stream {
        if st.data_plane == DataPlane::Upstream {
            return match upstream_face::run_text_stream(&account, prompt, model).await {
                Ok(stream) => {
                    let stream = stream.map(|chunk| chunk.map_err(std::io::Error::other));
                    match Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/event-stream")
                        .header(header::CACHE_CONTROL, "no-cache")
                        .header(header::CONNECTION, "keep-alive")
                        .body(Body::from_stream(stream))
                    {
                        Ok(r) => r,
                        Err(e) => err(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            e.to_string(),
                            "stream_build_failed",
                            Some("self"),
                        ),
                    }
                }
                Err(e) => {
                    error!(error=%e, "upstream text stream failed");
                    upstream_error_response(&e, "text_stream_failed")
                }
            };
        }
        let bridge_req = TextRunRequest {
            account,
            prompt,
            model,
        };
        return match st.helper.run_text_stream(&bridge_req).await {
            Ok(upstream) => {
                let status = upstream.status();
                let mut resp = Response::builder().status(status);
                if let Some(ct) = upstream.headers().get(header::CONTENT_TYPE) {
                    resp = resp.header(header::CONTENT_TYPE, ct);
                }
                resp = resp
                    .header(header::CACHE_CONTROL, "no-cache")
                    .header(header::CONNECTION, "keep-alive");
                let stream = upstream
                    .bytes_stream()
                    .map(|chunk| chunk.map_err(std::io::Error::other));
                match resp.body(Body::from_stream(stream)) {
                    Ok(r) => r,
                    Err(e) => err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        e.to_string(),
                        "stream_build_failed",
                        Some("self"),
                    ),
                }
            }
            Err(e) => {
                error!(error=%e, "helper text stream failed");
                err(
                    StatusCode::BAD_GATEWAY,
                    e.to_string(),
                    "helper_unreachable",
                    Some("self"),
                )
            }
        };
    }

    if st.data_plane == DataPlane::Upstream {
        match upstream_face::run_text(&account, prompt, model).await {
            Ok(content) => {
                return (
                    StatusCode::OK,
                    Json(chat_completion_response(&req.model, &content)),
                )
                    .into_response();
            }
            Err(e) => {
                error!(error=%e, "upstream text call failed");
                return upstream_error_response(&e, "text_failed");
            }
        }
    }

    let bridge_req = TextRunRequest {
        account,
        prompt,
        model,
    };
    match st.helper.run_text(&bridge_req).await {
        Ok(r) if r.ok => {
            let content = r.content.unwrap_or_default();
            (
                StatusCode::OK,
                Json(chat_completion_response(&req.model, &content)),
            )
                .into_response()
        }
        Ok(r) => {
            let fault = r.fault.as_deref();
            let msg = r.error.unwrap_or_else(|| "text bridge failed".into());
            let class = classify_fault(fault, Some(&msg));
            let code = match class {
                protocol::ErrorClass::Self_ => StatusCode::INTERNAL_SERVER_ERROR,
                protocol::ErrorClass::Client => StatusCode::BAD_REQUEST,
                _ => StatusCode::BAD_GATEWAY,
            };
            err(code, msg, "text_failed", Some(class.as_str()))
        }
        Err(e) => {
            error!(error=%e, "helper text call failed");
            err(
                StatusCode::BAD_GATEWAY,
                e.to_string(),
                "helper_unreachable",
                Some("self"),
            )
        }
    }
}

struct ImageBatchItem {
    b64: String,
    upstream_metrics: Option<upstream::ImageRunMetrics>,
    account: PinAccount,
    elapsed_ms: u128,
    pipeline: Option<Value>,
}

async fn generate_one_image(
    st: &AppState,
    candidates: &[PinAccount],
    start_idx: usize,
    is_admin: bool,
    req: &ImageGenerationRequest,
) -> Result<ImageBatchItem, Result<helper_client::BridgeOk, anyhow::Error>> {
    let max_attempts = image_max_attempts(is_admin, st.data_plane, candidates.len());
    let t0 = Instant::now();
    let mut result: Result<helper_client::BridgeOk, anyhow::Error> =
        Err(anyhow::anyhow!("image generation not attempted"));
    let mut upstream_metrics: Option<upstream::ImageRunMetrics> = None;
    let mut used_account = candidates[start_idx].clone();
    let mut inflight_guard = st.scheduling_gate.begin_inflight(&used_account.email);

    for try_no in 0..max_attempts {
        let i = (start_idx + try_no) % candidates.len();
        let cand = &candidates[i];
        used_account = cand.clone();
        if cand.email != inflight_guard.email() {
            inflight_guard = st.scheduling_gate.begin_inflight(&used_account.email);
        }
        let binding_key = st
            .scheduling_gate
            .account_binding_key(&cand.email, cand.proxy.as_deref());
        let _binding_guard = match st.binding_inflight.try_begin(&binding_key) {
            Some(g) => g,
            None if is_admin && try_no + 1 < max_attempts => {
                warn!(
                    email=%cand.email,
                    binding=%binding_key,
                    attempt=try_no + 1,
                    "binding inflight saturated; retrying next account"
                );
                continue;
            }
            None => {
                return Err(Err(anyhow::anyhow!(
                    "binding_inflight saturated for {binding_key}"
                )));
            }
        };
        let _dispatch_mark = DispatchMarkGuard::new(&st.dispatch_interval, &cand.email);
        if try_no > 0 {
            let delay = poisson_delay_ms(st.workload.poisson_lambda);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }
        let _account_slot = match st.slot_ledger.try_acquire_account(&cand.email) {
            Some(g) => g,
            None if is_admin && try_no + 1 < max_attempts => {
                warn!(email=%cand.email, attempt=try_no + 1, "account slot saturated; retrying");
                continue;
            }
            None => {
                return Err(Err(anyhow::anyhow!(
                    "account slot saturated for {}",
                    cand.email
                )));
            }
        };
        let _ss_slot = match st.slot_ledger.try_acquire_ss(&cand.email) {
            Some(g) => g,
            None if is_admin && try_no + 1 < max_attempts => {
                warn!(email=%cand.email, attempt=try_no + 1, "ss slot saturated; retrying");
                continue;
            }
            None => {
                return Err(Err(anyhow::anyhow!(
                    "ss slot saturated for {}",
                    cand.email
                )));
            }
        };
        if st.data_plane == DataPlane::Upstream {
            info!(email=%cand.email, attempt=try_no + 1, max_attempts, "upstream image attempt");
            let upstream_account = pin_with_pre_ticket(st, cand);
            let attempt_result = run_upstream_image(
                &upstream_account,
                req.prompt.clone(),
                req.model.clone(),
                req.size.clone(),
                req.quality.clone(),
                req.transparent_bg(),
                &req.asset_ids,
            )
            .await;
            match attempt_result {
                Ok((bridge, metrics)) if bridge.ok => {
                    upstream_metrics = Some(metrics);
                    result = Ok(bridge);
                    st.pipeline_watchdog.mark_progress();
                    break;
                }
                Err(e) if is_admin && upstream_image_retryable(&e) && try_no + 1 < max_attempts => {
                    record_image_upstream_failure(st, cand, &e);
                    warn!(
                        email=%cand.email,
                        attempt=try_no + 1,
                        error=%e,
                        "upstream image failed; retrying next pool account"
                    );
                    result = Err(e);
                    continue;
                }
                Ok((bridge, _)) => {
                    result = Ok(bridge);
                    break;
                }
                Err(e) => {
                    record_image_upstream_failure(st, cand, &e);
                    result = Err(e);
                    break;
                }
            }
        } else {
            let bridge_req = ImageRunRequest {
                account: cand.clone(),
                prompt: req.prompt.clone(),
                model: req.model.clone(),
                size: req.size.clone(),
            };
            let attempt_result = st.helper.run_image(&bridge_req).await.map_err(|e| e.into());
            match &attempt_result {
                Ok(r) if r.ok => {
                    result = attempt_result;
                    break;
                }
                _ => {
                    result = attempt_result;
                    break;
                }
            }
        }
    }

    let elapsed_ms = t0.elapsed().as_millis();
    match result {
        Ok(r) if r.ok => {
            let b64 = r.b64_json.unwrap_or_default();
            if b64.len() < 1000 {
                return Err(Err(anyhow::anyhow!("empty/short b64_json from bridge")));
            }
            Ok(ImageBatchItem {
                b64,
                upstream_metrics,
                account: used_account,
                elapsed_ms,
                pipeline: None,
            })
        }
        Ok(r) => Err(Ok(r)),
        Err(e) => Err(Err(e)),
    }
}

async fn generate_one_image_edit(
    st: &AppState,
    candidates: &[PinAccount],
    start_idx: usize,
    is_admin: bool,
    req: &ImageEditRequest,
    image_bytes: &[u8],
    mask_bytes: Option<&[u8]>,
) -> Result<ImageBatchItem, Result<helper_client::BridgeOk, anyhow::Error>> {
    let max_attempts = image_max_attempts(is_admin, st.data_plane, candidates.len());
    let t0 = Instant::now();
    let mut result: Result<helper_client::BridgeOk, anyhow::Error> =
        Err(anyhow::anyhow!("image edit not attempted"));
    let mut upstream_metrics: Option<upstream::ImageRunMetrics> = None;
    let mut used_account = candidates[start_idx].clone();
    let mut inflight_guard = st.scheduling_gate.begin_inflight(&used_account.email);

    for try_no in 0..max_attempts {
        let i = (start_idx + try_no) % candidates.len();
        let cand = &candidates[i];
        used_account = cand.clone();
        if cand.email != inflight_guard.email() {
            inflight_guard = st.scheduling_gate.begin_inflight(&used_account.email);
        }
        let binding_key = st
            .scheduling_gate
            .account_binding_key(&cand.email, cand.proxy.as_deref());
        let _binding_guard = match st.binding_inflight.try_begin(&binding_key) {
            Some(g) => g,
            None if is_admin && try_no + 1 < max_attempts => {
                warn!(
                    email=%cand.email,
                    binding=%binding_key,
                    attempt=try_no + 1,
                    "binding inflight saturated; retrying next account"
                );
                continue;
            }
            None => {
                return Err(Err(anyhow::anyhow!(
                    "binding_inflight saturated for {binding_key}"
                )));
            }
        };
        let _dispatch_mark = DispatchMarkGuard::new(&st.dispatch_interval, &cand.email);
        let _account_slot = match st.slot_ledger.try_acquire_account(&cand.email) {
            Some(g) => g,
            None if is_admin && try_no + 1 < max_attempts => continue,
            None => {
                return Err(Err(anyhow::anyhow!(
                    "account slot saturated for {}",
                    cand.email
                )));
            }
        };
        let _ss_slot = match st.slot_ledger.try_acquire_ss(&cand.email) {
            Some(g) => g,
            None if is_admin && try_no + 1 < max_attempts => continue,
            None => {
                return Err(Err(anyhow::anyhow!(
                    "ss slot saturated for {}",
                    cand.email
                )));
            }
        };
        info!(email=%cand.email, attempt=try_no + 1, max_attempts, "upstream image edit attempt");
        let upstream_account = pin_with_pre_ticket(st, cand);
        let attempt_result = run_upstream_image_edit(
            &upstream_account,
            req.prompt.clone(),
            req.model.clone(),
            req.size.clone(),
            image_bytes.to_vec(),
            mask_bytes.map(|m| m.to_vec()),
            &req.asset_ids,
        )
        .await;
        match attempt_result {
            Ok((bridge, metrics)) if bridge.ok => {
                upstream_metrics = Some(metrics);
                result = Ok(bridge);
                st.pipeline_watchdog.mark_progress();
                break;
            }
            Err(e) if is_admin && upstream_image_retryable(&e) && try_no + 1 < max_attempts => {
                record_image_upstream_failure(st, cand, &e);
                warn!(
                    email=%cand.email,
                    attempt=try_no + 1,
                    error=%e,
                    "upstream image edit failed; retrying next pool account"
                );
                result = Err(e);
                continue;
            }
            Ok((bridge, _)) => {
                result = Ok(bridge);
                break;
            }
            Err(e) => {
                record_image_upstream_failure(st, cand, &e);
                result = Err(e);
                break;
            }
        }
    }

    let elapsed_ms = t0.elapsed().as_millis();
    match result {
        Ok(r) if r.ok => {
            let b64 = r.b64_json.unwrap_or_default();
            if b64.len() < 1000 {
                return Err(Err(anyhow::anyhow!("empty/short b64_json from bridge")));
            }
            Ok(ImageBatchItem {
                b64,
                upstream_metrics,
                account: used_account,
                elapsed_ms,
                pipeline: None,
            })
        }
        Ok(r) => Err(Ok(r)),
        Err(e) => Err(Err(e)),
    }
}

fn image_batch_bridge_failure(
    account: &PinAccount,
    elapsed_ms: u128,
    r: helper_client::BridgeOk,
    log_label: &str,
) -> Response {
    let fault = r.fault.as_deref();
    let msg = r
        .error
        .unwrap_or_else(|| format!("{log_label} bridge failed"));
    let class = classify_fault(fault, Some(&msg));
    warn!(email=%account.email, elapsed_ms, fault=?fault, error=%msg, log_label, "image batch bridge failed");
    let (code, err_code) = match class {
        protocol::ErrorClass::Self_ => (StatusCode::INTERNAL_SERVER_ERROR, "image_failed"),
        protocol::ErrorClass::Gate => (StatusCode::TOO_MANY_REQUESTS, "image_quota_insufficient"),
        protocol::ErrorClass::Client => (StatusCode::BAD_REQUEST, "invalid_request"),
        protocol::ErrorClass::Upstream => (StatusCode::BAD_GATEWAY, "image_failed"),
    };
    err(code, msg, err_code, Some(class.as_str()))
}

fn image_batch_upstream_failure(
    st: &AppState,
    account: &PinAccount,
    elapsed_ms: u128,
    e: &anyhow::Error,
) -> Response {
    error!(email=%account.email, elapsed_ms, error=%e, "image call failed");
    if st.data_plane == DataPlane::Upstream {
        upstream_error_response(e, "upstream_unreachable")
    } else {
        err(
            StatusCode::BAD_GATEWAY,
            e.to_string(),
            "helper_unreachable",
            Some("self"),
        )
    }
}

fn check_image_admission(st: &AppState) -> Option<Response> {
    if !st.image_generation_allowed() {
        if st.image_enabled {
            return Some(err_wait(
                StatusCode::SERVICE_UNAVAILABLE,
                "image generation paused (image_generation_paused)",
                "image_generation_paused",
                Some("gate"),
                300,
            ));
        }
        return None;
    }
    if st.deadlock_guard.is_tripped() {
        return Some(err_wait(
            StatusCode::SERVICE_UNAVAILABLE,
            "deadlock_guard: process cpu trip — deferring image admissions",
            "deadlock_guard",
            Some("gate"),
            120,
        ));
    }
    if st.pipeline_watchdog.is_tripped() {
        return Some(err_wait(
            StatusCode::SERVICE_UNAVAILABLE,
            "pipeline_watchdog: queue stall detected — deferring image admissions",
            "pipeline_watchdog",
            Some("gate"),
            60,
        ));
    }
    let cap = st.effective_global_concurrency();
    if st.image_global_busy() >= cap {
        return Some(err_wait(
            StatusCode::TOO_MANY_REQUESTS,
            "image admission cap reached (workload/global)",
            "image_service_busy",
            Some("gate"),
            st.estimated_image_wait_secs(),
        ));
    }
    None
}

fn acquire_image_delivery_gates(
    st: &AppState,
    payload_bytes: u64,
) -> Result<(ReadyBufferGuard<'_>, ReturnWindowPermit<'_>), Response> {
    let ready = ReadyBufferGuard::try_acquire(&st.ready_buffer, payload_bytes).ok_or_else(|| {
        err_wait(
            StatusCode::TOO_MANY_REQUESTS,
            "ready_buffer saturated",
            "ready_buffer",
            Some("gate"),
            st.estimated_image_wait_secs(),
        )
    })?;
    let return_permit = st.return_window.try_acquire().ok_or_else(|| {
        err_wait(
            StatusCode::TOO_MANY_REQUESTS,
            "return_window saturated",
            "return_window",
            Some("gate"),
            st.estimated_image_wait_secs(),
        )
    })?;
    Ok((ready, return_permit))
}

fn sync_adapter_enabled() -> bool {
    std::env::var("IMAGE_SYNC_ADAPTER")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true)
}

fn sync_adapter_timeout_secs() -> u64 {
    std::env::var("IMAGE_SYNC_ADAPTER_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(900)
}

async fn image_task_poll_response(st: &AppState, task_id: &str) -> Response {
    let Some(rec) = st.image_tasks.store.get(task_id) else {
        return err(
            StatusCode::NOT_FOUND,
            format!("image task not found: {task_id}"),
            "image_task_not_found",
            Some("client"),
        );
    };
    let status = image_tasks::task_state_str(&rec.state);
    if rec.state == image_tasks::ImageTaskState::Done {
        return (
            StatusCode::OK,
            Json(
                rec.result
                    .unwrap_or_else(|| image_task_status_response(task_id, status, None, None)),
            ),
        )
            .into_response();
    }
    if rec.state == image_tasks::ImageTaskState::Failed {
        return err(
            StatusCode::BAD_GATEWAY,
            rec.error.unwrap_or_else(|| "image task failed".into()),
            "image_task_failed",
            Some("upstream"),
        );
    }
    (
        StatusCode::OK,
        Json(image_task_status_response(
            task_id,
            status,
            None,
            rec.error.as_deref(),
        )),
    )
        .into_response()
}

async fn enqueue_image_task(st: &AppState, req: ImageGenerationRequest) -> Response {
    match st.image_tasks.try_enqueue(req) {
        Ok(id) => (StatusCode::OK, Json(image_task_queued_response(&id))).into_response(),
        Err(e) => err_wait(
            StatusCode::TOO_MANY_REQUESTS,
            e,
            "image_service_busy",
            Some("gate"),
            st.estimated_image_wait_secs(),
        ),
    }
}

async fn wait_for_image_task(st: &AppState, task_id: &str) -> Response {
    let timeout = sync_adapter_timeout_secs();
    let deadline = Instant::now() + std::time::Duration::from_secs(timeout);
    let poll_ms = std::env::var("IMAGE_SYNC_ADAPTER_POLL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    loop {
        if let Some(rec) = st.image_tasks.store.get(task_id) {
            match rec.state {
                image_tasks::ImageTaskState::Done => {
                    return (
                        StatusCode::OK,
                        Json(
                            rec.result
                                .unwrap_or_else(|| json!({ "id": task_id, "status": "done" })),
                        ),
                    )
                        .into_response();
                }
                image_tasks::ImageTaskState::Failed => {
                    return err(
                        StatusCode::BAD_GATEWAY,
                        rec.error.unwrap_or_else(|| "image task failed".into()),
                        "image_task_failed",
                        Some("upstream"),
                    );
                }
                image_tasks::ImageTaskState::TimeoutPending
                | image_tasks::ImageTaskState::Running
                | image_tasks::ImageTaskState::Queued => {}
            }
        } else {
            return err(
                StatusCode::NOT_FOUND,
                format!("image task not found: {task_id}"),
                "image_task_not_found",
                Some("client"),
            );
        }
        if Instant::now() >= deadline {
            st.image_tasks.store.update_state(
                task_id,
                image_tasks::ImageTaskState::TimeoutPending,
                None,
                Some("sync adapter timeout".into()),
                None,
                None,
            );
            return err_wait(
                StatusCode::GATEWAY_TIMEOUT,
                format!("image task timeout after {timeout}s (task_id={task_id})"),
                "image_task_timeout",
                Some("gate"),
                st.estimated_image_wait_secs(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;
    }
}

async fn process_image_task(st: Arc<AppState>, task_id: String) {
    let record = st.image_tasks.store.get(&task_id);
    if record.is_none() {
        return;
    }
    let req = record.expect("task record").request;
    st.image_tasks.store.update_state(
        &task_id,
        image_tasks::ImageTaskState::Running,
        None,
        None,
        None,
        None,
    );
    let is_admin = true;
    let candidates = match collect_image_accounts(&st, None, is_admin).await {
        Ok(c) => c,
        Err(_) => {
            image_tasks::log_task_fail(
                &st.image_tasks.store,
                &task_id,
                "no accounts available for image task",
            );
            return;
        }
    };
    let permit = match try_acquire_image_permit(&st) {
        Ok(p) => p,
        Err(_) => {
            st.image_tasks.store.update_state(
                &task_id,
                image_tasks::ImageTaskState::TimeoutPending,
                None,
                Some("image_service_busy".into()),
                None,
                None,
            );
            return;
        }
    };
    let start_idx = st.image_account_rr.fetch_add(1, Ordering::Relaxed) % candidates.len();
    let outcome = generate_one_image(&st, &candidates, start_idx, is_admin, &req).await;
    drop(permit);
    match outcome {
        Ok(item) => {
            let body = image_generation_response(&item.b64, &req.prompt);
            let trace = item.pipeline.clone();
            image_tasks::log_task_done(
                &st.image_tasks.store,
                &task_id,
                body,
                &item.account.email,
                trace.clone(),
            );
            if let Some(trace) = trace {
                image_tasks::append_task_trace_ndjson(&task_id, &trace);
            }
            st.pipeline_watchdog.mark_progress();
        }
        Err(Err(e)) => {
            record_image_upstream_failure(&st, &candidates[start_idx], &e);
            image_tasks::log_task_fail(&st.image_tasks.store, &task_id, &e.to_string());
        }
        Err(Ok(r)) => {
            let msg = r
                .error
                .unwrap_or_else(|| "image bridge failed in task worker".into());
            image_tasks::log_task_fail(&st.image_tasks.store, &task_id, &msg);
        }
    }
}

async fn get_image_task(
    State(st): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    image_task_poll_response(&st, task_id.trim()).await
}

async fn post_image_tasks_generations(
    State(st): State<Arc<AppState>>,
    Json(req): Json<ImageGenerationRequest>,
) -> Response {
    if !st.image_enabled {
        return err(
            StatusCode::NOT_IMPLEMENTED,
            "image generation deferred; set IMAGE_ENABLED=1",
            "image_deferred",
            Some("gate"),
        );
    }
    enqueue_image_task(&st, req).await
}

async fn image_generations(
    State(st): State<Arc<AppState>>,
    user: auth_routes::AuthUser,
    headers: HeaderMap,
    Json(mut req): Json<ImageGenerationRequest>,
) -> impl IntoResponse {
    if !st.image_enabled {
        return err(
            StatusCode::NOT_IMPLEMENTED,
            "image generation deferred; set IMAGE_ENABLED=1 after backend pipeline integration",
            "image_deferred",
            Some("gate"),
        );
    }
    if let Some(r) = check_image_admission(&st) {
        return r;
    }

    if let Some(task_id) = req
        .panda_task_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return image_task_poll_response(&st, task_id).await;
    }

    match parse_image_prompt_tunnel(&req.prompt) {
        ImagePromptTunnel::StatusPoll(id) => return image_task_poll_response(&st, &id).await,
        ImagePromptTunnel::AsyncGenerate(prompt) => {
            req.prompt = prompt;
            req.panda_async = true;
        }
        ImagePromptTunnel::Normal(prompt) => req.prompt = prompt,
    }

    if req.panda_async {
        return enqueue_image_task(&st, req).await;
    }

    if req.n == 0 || req.n > MAX_IMAGE_BATCH_N {
        return err(
            StatusCode::BAD_REQUEST,
            format!("n must be between 1 and {}", MAX_IMAGE_BATCH_N),
            "n_unsupported",
            Some("client"),
        );
    }
    let batch_n = req.n;

    let is_admin = user.claims.role.is_admin();
    let preferred = preferred_email(&headers);
    let candidates = match collect_image_accounts(&st, preferred, is_admin).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    if st.duplicate_prompt.check(
        candidates.first().map(|a| a.email.as_str()).unwrap_or(""),
        &req.prompt,
    ) {
        return err_wait(
            StatusCode::TOO_MANY_REQUESTS,
            "duplicate-prompt: identical image prompt recently submitted",
            "duplicate_prompt",
            Some("gate"),
            st.estimated_image_wait_secs(),
        );
    }
    let account = candidates[0].clone();

    if !is_admin {
        if let Some(r) = check_dispatch_backpressure(&st, &account.email) {
            return r;
        }
    }

    let permit = match try_acquire_image_permit(&st) {
        Ok(p) => p,
        Err(r) if sync_adapter_enabled() => {
            let task_id = match st.image_tasks.try_enqueue(req.clone()) {
                Ok(id) => id,
                Err(e) => {
                    return err_wait(
                        StatusCode::TOO_MANY_REQUESTS,
                        e,
                        "image_service_busy",
                        Some("gate"),
                        st.estimated_image_wait_secs(),
                    );
                }
            };
            return wait_for_image_task(&st, &task_id).await;
        }
        Err(r) => return r,
    };

    let qreq = QuotaRefreshRequest {
        account: account.clone(),
        min_remaining: st.min_image_quota * batch_n as i64,
    };
    if st.data_plane == DataPlane::Upstream {
        info!(email=%account.email, "upstream image: skipping helper quota precheck");
    } else {
        match st.helper.refresh_quota(&qreq).await {
            Ok(q) if q.ok && q.imageable.unwrap_or(false) => {}
            Ok(q) if q.ok => {
                drop(permit);
                return err(
                    StatusCode::TOO_MANY_REQUESTS,
                    format!(
                        "image_quota_insufficient: remaining={:?} status={:?} min={} restore_at={:?}",
                        q.remaining, q.status, st.min_image_quota, q.restore_at
                    ),
                    "image_quota_insufficient",
                    Some("gate"),
                );
            }
            Ok(q) => {
                drop(permit);
                let fault = q.fault.as_deref().unwrap_or("upstream");
                let msg = q
                    .error
                    .unwrap_or_else(|| "quota refresh failed before image".into());
                let code = if fault == "self" {
                    StatusCode::INTERNAL_SERVER_ERROR
                } else {
                    StatusCode::BAD_GATEWAY
                };
                return err(code, msg, "quota_refresh_failed", Some(fault));
            }
            Err(e) => {
                drop(permit);
                error!(error=%e, "helper quota precheck failed");
                return err(
                    StatusCode::BAD_GATEWAY,
                    e.to_string(),
                    "helper_unreachable",
                    Some("self"),
                );
            }
        }
    }

    let t0 = Instant::now();
    let mut batch_items: Vec<ImageBatchItem> = Vec::with_capacity(batch_n as usize);
    for _ in 0..batch_n {
        let start_idx = st.image_account_rr.fetch_add(1, Ordering::Relaxed) % candidates.len();
        match generate_one_image(&st, &candidates, start_idx, is_admin, &req).await {
            Ok(item) => {
                let pipeline = item.upstream_metrics.as_ref().map(|m| {
                    finalize_upstream_image(
                        &st,
                        &item.account.email,
                        item.elapsed_ms,
                        m,
                        0,
                        item.b64.len() as u64,
                        &req.prompt,
                    )
                });
                batch_items.push(ImageBatchItem { pipeline, ..item });
            }
            Err(Err(e)) => {
                drop(permit);
                let account = candidates[start_idx].clone();
                return image_batch_upstream_failure(&st, &account, t0.elapsed().as_millis(), &e);
            }
            Err(Ok(r)) => {
                drop(permit);
                let account = candidates[start_idx].clone();
                return image_batch_bridge_failure(&account, t0.elapsed().as_millis(), r, "image");
            }
        }
    }
    let elapsed_ms = t0.elapsed().as_millis();
    drop(permit);

    let want_url = image_assets::wants_url_response(&req.response_format);
    if want_url {
        let mut urls: Vec<String> = Vec::with_capacity(batch_items.len());
        let mut total_asset_store_ms = 0u64;
        for item in &batch_items {
            let bytes = match BASE64.decode(&item.b64) {
                Ok(b) if b.len() >= 256 => b,
                Ok(_) => {
                    return err(
                        StatusCode::BAD_GATEWAY,
                        "empty/short image payload from bridge",
                        "empty_image",
                        Some("self"),
                    );
                }
                Err(e) => {
                    return err(
                        StatusCode::BAD_GATEWAY,
                        format!("invalid b64_json from bridge: {e}"),
                        "empty_image",
                        Some("self"),
                    );
                }
            };
            let asset_t0 = Instant::now();
            let url = match build_image_asset_url(&st, &headers, bytes) {
                Ok(url) => url,
                Err(e) => {
                    return err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        e.to_string(),
                        "image_url_failed",
                        Some("self"),
                    );
                }
            };
            let asset_store_ms = asset_t0.elapsed().as_millis() as u64;
            total_asset_store_ms += asset_store_ms;
            urls.push(url);
        }
        let last = batch_items.last().expect("batch_items");
        let pipeline = last.pipeline.clone().map(|mut p| {
            if let Some(obj) = p.as_object_mut() {
                obj.insert("batch_n".into(), json!(batch_n));
                if let Some(timings) = obj.get_mut("timings_ms").and_then(|v| v.as_object_mut()) {
                    timings.insert("gateway_wall_ms".into(), json!(last.elapsed_ms));
                    timings.insert("asset_store_ms".into(), json!(total_asset_store_ms));
                    timings.insert("handler_wall_ms".into(), json!(elapsed_ms));
                }
            }
            p
        });
        info!(
            email=%last.account.email,
            elapsed_ms,
            batch_n,
            url_count=urls.len(),
            quota_after=?pipeline.as_ref().and_then(|p| p.get("quota_after")),
            "image ok (url)"
        );
        let payload_bytes = batch_items.iter().map(|i| i.b64.len() as u64).sum();
        let (_ready, _return) = match acquire_image_delivery_gates(&st, payload_bytes) {
            Ok(g) => g,
            Err(r) => return r,
        };
        let body = if let Some(p) = pipeline {
            image_generation_url_multi_response_with_pipeline(&urls, p, &req.prompt)
        } else if urls.len() == 1 {
            image_generation_url_response(&urls[0], &req.prompt)
        } else {
            image_generation_url_multi_response(&urls, &req.prompt)
        };
        let archive_items: Vec<_> = batch_items
            .iter()
            .zip(urls.iter())
            .map(|(item, url)| {
                (
                    item.b64.clone(),
                    item.elapsed_ms,
                    item.pipeline.clone(),
                    Some(url.clone()),
                )
            })
            .collect();
        image_archive::schedule_gateway_image_archive(
            st.clone(),
            headers,
            req.model.clone(),
            req.prompt.clone(),
            archive_items,
        );
        return (StatusCode::OK, Json(body)).into_response();
    }

    let b64s: Vec<String> = batch_items.iter().map(|i| i.b64.clone()).collect();
    let last = batch_items.last().expect("batch_items");
    let pipeline = last.pipeline.clone().map(|mut p| {
        if let Some(obj) = p.as_object_mut() {
            obj.insert("batch_n".into(), json!(batch_n));
            if let Some(timings) = obj.get_mut("timings_ms").and_then(|v| v.as_object_mut()) {
                timings.insert("gateway_wall_ms".into(), json!(last.elapsed_ms));
                timings.insert("handler_wall_ms".into(), json!(elapsed_ms));
            }
        }
        p
    });
    info!(
        email=%last.account.email,
        elapsed_ms,
        batch_n,
        b64_count=b64s.len(),
        quota_after=?pipeline.as_ref().and_then(|p| p.get("quota_after")),
        "image ok"
    );
    let payload_bytes = b64s.iter().map(|b| b.len() as u64).sum();
    let (_ready, _return) = match acquire_image_delivery_gates(&st, payload_bytes) {
        Ok(g) => g,
        Err(r) => return r,
    };
    let body = if let Some(p) = pipeline {
        image_generation_b64_multi_response_with_pipeline(&b64s, p, &req.prompt)
    } else if b64s.len() == 1 {
        image_generation_response(&b64s[0], &req.prompt)
    } else {
        image_generation_b64_multi_response(&b64s, &req.prompt)
    };
    let archive_items: Vec<_> = batch_items
        .iter()
        .map(|item| {
            (
                item.b64.clone(),
                item.elapsed_ms,
                item.pipeline.clone(),
                None,
            )
        })
        .collect();
    image_archive::schedule_gateway_image_archive(
        st.clone(),
        headers,
        req.model.clone(),
        req.prompt.clone(),
        archive_items,
    );
    (StatusCode::OK, Json(body)).into_response()
}

async fn run_upstream_image_edit(
    account: &PinAccount,
    prompt: String,
    model: String,
    size: String,
    image_bytes: Vec<u8>,
    mask_bytes: Option<Vec<u8>>,
    asset_ids: &[String],
) -> Result<(helper_client::BridgeOk, upstream::ImageRunMetrics), anyhow::Error> {
    let prompt = tnexus_domain::append_image_generation_hints(&prompt, &size, "auto", false);
    upstream_face::run_image_edit(
        account,
        prompt,
        model,
        image_bytes,
        "edit.png".into(),
        mask_bytes,
        asset_ids,
    )
    .await
    .map(|(bytes, metrics)| {
        let b64 = BASE64.encode(&bytes);
        let bridge = helper_client::BridgeOk {
            ok: true,
            content: None,
            b64_json: Some(b64),
            conversation_id: None,
            fault: None,
            error: None,
            elapsed_ms: Some(metrics.wall_ms),
            raw: None,
            quota: None,
        };
        (bridge, metrics)
    })
}

async fn image_edits(
    State(st): State<Arc<AppState>>,
    user: auth_routes::AuthUser,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response {
    let is_multipart = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.starts_with("multipart/form-data"))
        .unwrap_or(false);

    if is_multipart {
        let multipart = match Multipart::from_request(request, &()).await {
            Ok(m) => m,
            Err(e) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    format!("invalid multipart: {e}"),
                    "multipart_invalid",
                    Some("client"),
                );
            }
        };
        match parse_image_edit_multipart(multipart).await {
            Ok(req) => image_edits_json(st, user, headers, req).await,
            Err(msg) => err(
                StatusCode::BAD_REQUEST,
                msg,
                "multipart_invalid",
                Some("client"),
            ),
        }
    } else {
        let bytes = match axum::body::to_bytes(request.into_body(), MAX_REQUEST_BODY_BYTES).await {
            Ok(b) => b,
            Err(e) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    format!("read body: {e}"),
                    "invalid_json",
                    Some("client"),
                );
            }
        };
        let req: ImageEditRequest = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(e) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    format!("invalid json: {e}"),
                    "invalid_json",
                    Some("client"),
                );
            }
        };
        image_edits_json(st, user, headers, req).await
    }
}

async fn parse_image_edit_multipart(mut multipart: Multipart) -> Result<ImageEditRequest, String> {
    let mut prompt = String::new();
    let mut model = "gpt-image-2".to_string();
    let mut size = "1024x1024".to_string();
    let mut n = 1u32;
    let mut image_bytes: Option<Vec<u8>> = None;
    let mut mask_bytes: Option<Vec<u8>> = None;
    let mut asset_ids: Vec<String> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| format!("multipart field: {e}"))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "prompt" => {
                prompt = field
                    .text()
                    .await
                    .map_err(|e| format!("prompt field: {e}"))?;
            }
            "model" => {
                model = field
                    .text()
                    .await
                    .map_err(|e| format!("model field: {e}"))?;
            }
            "size" => {
                size = field.text().await.map_err(|e| format!("size field: {e}"))?;
            }
            "n" => {
                let text = field.text().await.map_err(|e| format!("n field: {e}"))?;
                n = text.parse().unwrap_or(1);
            }
            "image" | "image[]" => {
                image_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| format!("image field: {e}"))?
                        .to_vec(),
                );
            }
            "mask" => {
                mask_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| format!("mask field: {e}"))?
                        .to_vec(),
                );
            }
            "asset_ids" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| format!("asset_ids field: {e}"))?;
                if let Ok(ids) = serde_json::from_str::<Vec<String>>(&text) {
                    asset_ids = ids;
                } else {
                    asset_ids = text
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            _ => {}
        }
    }

    let image = image_bytes.map(|bytes| BASE64.encode(&bytes));
    let mask = mask_bytes.map(|bytes| BASE64.encode(&bytes));
    Ok(ImageEditRequest {
        model,
        prompt,
        image,
        mask,
        n,
        size,
        asset_ids,
    })
}

async fn image_edits_json(
    st: Arc<AppState>,
    user: auth_routes::AuthUser,
    headers: HeaderMap,
    req: ImageEditRequest,
) -> Response {
    if !st.image_enabled {
        return err(
            StatusCode::NOT_IMPLEMENTED,
            "image edits deferred; set IMAGE_ENABLED=1 after backend pipeline integration",
            "image_edits_deferred",
            Some("gate"),
        );
    }
    if st.data_plane != DataPlane::Upstream {
        return err(
            StatusCode::NOT_IMPLEMENTED,
            "image edits require DATA_PLANE=upstream",
            "image_edits_deferred",
            Some("gate"),
        );
    }
    if req.n == 0 || req.n > MAX_IMAGE_BATCH_N {
        return err(
            StatusCode::BAD_REQUEST,
            format!("n must be between 1 and {}", MAX_IMAGE_BATCH_N),
            "n_unsupported",
            Some("client"),
        );
    }
    let batch_n = req.n;
    let Some(image_raw) = req.image.as_deref().filter(|s| !s.trim().is_empty()) else {
        return err(
            StatusCode::BAD_REQUEST,
            "image field is required (base64 or data URL)",
            "image_required",
            Some("client"),
        );
    };
    let image_bytes = match upstream::upload::decode_image_payload(image_raw) {
        Ok(bytes) if bytes.len() >= 64 => bytes,
        Ok(_) => {
            return err(
                StatusCode::BAD_REQUEST,
                "image payload too short",
                "image_invalid",
                Some("client"),
            );
        }
        Err(e) => {
            return err(
                StatusCode::BAD_REQUEST,
                format!("invalid image payload: {e}"),
                "image_invalid",
                Some("client"),
            );
        }
    };

    let mask_bytes: Option<Vec<u8>> = match req.mask.as_deref().filter(|s| !s.trim().is_empty()) {
        None => None,
        Some(raw) => match upstream::upload::decode_image_payload(raw) {
            Ok(bytes) if bytes.len() >= 64 => Some(bytes),
            Ok(_) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    "mask payload too short",
                    "mask_invalid",
                    Some("client"),
                );
            }
            Err(e) => {
                return err(
                    StatusCode::BAD_REQUEST,
                    format!("invalid mask payload: {e}"),
                    "mask_invalid",
                    Some("client"),
                );
            }
        },
    };

    let is_admin = user.claims.role.is_admin();
    let preferred = preferred_email(&headers);
    let candidates = match collect_image_accounts(&st, preferred, is_admin).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    if !is_admin {
        if let Some(r) = check_dispatch_backpressure(&st, &candidates[0].email) {
            return r;
        }
    }

    let permit = match try_acquire_image_permit(&st) {
        Ok(p) => p,
        Err(r) => return r,
    };

    let t0 = Instant::now();
    let mut batch_items: Vec<ImageBatchItem> = Vec::with_capacity(batch_n as usize);
    for _ in 0..batch_n {
        let start_idx = st.image_account_rr.fetch_add(1, Ordering::Relaxed) % candidates.len();
        match generate_one_image_edit(
            &st,
            &candidates,
            start_idx,
            is_admin,
            &req,
            &image_bytes,
            mask_bytes.as_deref(),
        )
        .await
        {
            Ok(item) => {
                let pipeline = item.upstream_metrics.as_ref().map(|m| {
                    finalize_upstream_image(
                        &st,
                        &item.account.email,
                        item.elapsed_ms,
                        m,
                        0,
                        item.b64.len() as u64,
                        &req.prompt,
                    )
                });
                batch_items.push(ImageBatchItem { pipeline, ..item });
            }
            Err(Err(e)) => {
                drop(permit);
                let account = candidates[start_idx].clone();
                return image_batch_upstream_failure(&st, &account, t0.elapsed().as_millis(), &e);
            }
            Err(Ok(r)) => {
                drop(permit);
                let account = candidates[start_idx].clone();
                return image_batch_bridge_failure(
                    &account,
                    t0.elapsed().as_millis(),
                    r,
                    "image edit",
                );
            }
        }
    }
    let elapsed_ms = t0.elapsed().as_millis();
    drop(permit);

    let b64s: Vec<String> = batch_items.iter().map(|i| i.b64.clone()).collect();
    let last = batch_items.last().expect("batch_items");
    let pipeline = last.pipeline.clone().map(|mut p| {
        if let Some(obj) = p.as_object_mut() {
            obj.insert("batch_n".into(), json!(batch_n));
            if let Some(timings) = obj.get_mut("timings_ms").and_then(|v| v.as_object_mut()) {
                timings.insert("gateway_wall_ms".into(), json!(last.elapsed_ms));
                timings.insert("handler_wall_ms".into(), json!(elapsed_ms));
            }
        }
        p
    });
    info!(
        email=%last.account.email,
        elapsed_ms,
        batch_n,
        b64_count=b64s.len(),
        quota_after=?pipeline.as_ref().and_then(|p| p.get("quota_after")),
        "image edit ok"
    );
    let body = if let Some(p) = pipeline {
        image_generation_b64_multi_response_with_pipeline(&b64s, p, &req.prompt)
    } else if b64s.len() == 1 {
        image_generation_response(&b64s[0], &req.prompt)
    } else {
        image_generation_b64_multi_response(&b64s, &req.prompt)
    };
    let archive_items: Vec<_> = batch_items
        .iter()
        .map(|item| {
            (
                item.b64.clone(),
                item.elapsed_ms,
                item.pipeline.clone(),
                None,
            )
        })
        .collect();
    image_archive::schedule_gateway_image_archive(
        st.clone(),
        headers,
        req.model.clone(),
        req.prompt.clone(),
        archive_items,
    );
    (StatusCode::OK, Json(body)).into_response()
}

fn preferred_email(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-preferred-account-email")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn err(
    status: StatusCode,
    message: impl Into<String>,
    code: &str,
    fault: Option<&str>,
) -> Response {
    err_wait(status, message, code, fault, 30)
}

/// Map an upstream `anyhow::Error` to a status via the contract taxonomy.
///
/// Hardcoding 502 here made client-input faults (notably upstream's 413
/// `message_length_exceeds_limit`) look like channel outages to NewAPI.
fn upstream_error_response(e: &anyhow::Error, code: &str) -> Response {
    let msg = e.to_string();
    let class = classify_fault(None, Some(&msg));
    let status = match class {
        protocol::ErrorClass::Client => StatusCode::BAD_REQUEST,
        protocol::ErrorClass::Gate => StatusCode::TOO_MANY_REQUESTS,
        protocol::ErrorClass::Self_ => StatusCode::INTERNAL_SERVER_ERROR,
        protocol::ErrorClass::Upstream => StatusCode::BAD_GATEWAY,
    };
    err(status, msg, code, Some(class.as_str()))
}

fn err_wait(
    status: StatusCode,
    message: impl Into<String>,
    code: &str,
    fault: Option<&str>,
    retry_after_secs: u32,
) -> Response {
    let body: Value = openai_error(message, code, fault);
    let mut resp = (status, Json(body)).into_response();
    if status == StatusCode::TOO_MANY_REQUESTS {
        if let Ok(val) = header::HeaderValue::from_str(&retry_after_secs.to_string()) {
            resp.headers_mut().insert(header::RETRY_AFTER, val);
        }
    }
    resp
}

fn build_image_asset_url(
    st: &AppState,
    headers: &HeaderMap,
    bytes: Vec<u8>,
) -> anyhow::Result<String> {
    let base = image_assets::resolve_public_base(&st.public_base_url, headers)?;
    let (id, exp, sig) = st.image_assets.store(bytes);
    Ok(st.image_assets.public_url(&base, id, exp, &sig))
}

async fn get_image_asset(
    State(st): State<Arc<AppState>>,
    Path(asset_id): Path<Uuid>,
    Query(query): Query<image_assets::AssetQuery>,
) -> Response {
    image_assets::serve_image_asset(&st.image_assets, asset_id, query)
}

#[cfg(test)]
mod auth_integration {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use gateway_auth::{AuthConfig, AuthMode, AuthService, Role};
    use helper_client::{HelperClient, PinAccount};
    use std::collections::HashMap;
    use tokio::sync::Semaphore;
    use tower::ServiceExt;

    fn test_image_assets() -> Arc<image_assets::ImageAssetStore> {
        Arc::new(image_assets::ImageAssetStore::new(
            b"test-image-asset-signing-secret!!".to_vec(),
            3600,
        ))
    }

    fn test_app_state() -> Arc<AppState> {
        let path = std::env::temp_dir().join(format!("gw-auth-{}.db", uuid::Uuid::new_v4()));
        let cfg = AuthConfig {
            db_path: path.to_string_lossy().into(),
            jwt_secret: "integration-test-secret-32-bytes!!".into(),
            jwt_ttl_secs: 3600,
            cookie_name: "gws_session".into(),
            cookie_secure: false,
            allow_public_register: false,
            mode: AuthMode::Jwt,
            gateway_auth_key: None,
            bootstrap_user: Some("admin".into()),
            bootstrap_password: Some("integration-admin-pass".into()),
        };
        let auth = Arc::new(AuthService::open(cfg).unwrap());
        let pin = PinAccount {
            email: "test@example.com".into(),
            access_token: String::new(),
            device_id: None,
            proxy: None,
            user_agent: None,
            impersonate: None,
        };
        Arc::new(AppState {
            helper: HelperClient::new("http://127.0.0.1:1").unwrap(),
            data_plane: DataPlane::Helper,
            pin: pin.clone(),
            accounts: Arc::new(Mutex::new(HashMap::from([(pin.email.clone(), pin)]))),
            listen: "127.0.0.1:0".into(),
            min_image_quota: 1,
            image_global_concurrency: 1,
            image_sem: Arc::new(Semaphore::new(1)),
            image_enabled: false,
            image_runtime: ImageRuntimeConfig::from_env(false),
            deadlock_guard: DeadlockGuard::from_env(),
            pipeline_watchdog: PipelineWatchdog::from_env(),
            auth,
            static_dir: None,
            image_assets: test_image_assets(),
            public_base_url: "http://127.0.0.1:8014".into(),
            scheduling_gate: SchedulingGate::from_env(),
            image_account_rr: AtomicUsize::new(0),
            image_queue_depth: AtomicUsize::new(0),
            duplicate_prompt: duplicate_prompt::DuplicatePromptGate::new(),
            binding_inflight: BindingInflightLedger::from_env(),
            dispatch_interval: DispatchIntervalGate::from_env(),
            slot_ledger: SlotLedger::from_env(),
            ready_buffer: ReadyBuffer::from_env(),
            return_window: ReturnWindow::from_env(),
            cooldown: CooldownRegistry::from_env(),
            pre_ticket: PreTicketPool::from_env(),
            proxy_cf: ProxyCfRegistry::from_env(),
            workload: WorkloadPolicy::from_env(),
            image_tasks: image_tasks::ImageTaskService::spawn(|_| async {}, 1),
            pg_pool: None,
            image_archive_store: None,
        })
    }

    #[tokio::test]
    async fn api_key_mode_accepts_matching_bearer() {
        let path = std::env::temp_dir().join(format!("gw-apikey-{}.db", uuid::Uuid::new_v4()));
        let cfg = AuthConfig {
            db_path: path.to_string_lossy().into(),
            jwt_secret: String::new(),
            mode: AuthMode::ApiKey,
            gateway_auth_key: Some(concat!("panda-", "align-key").into()),
            bootstrap_user: None,
            bootstrap_password: None,
            jwt_ttl_secs: 3600,
            cookie_name: "gws_session".into(),
            cookie_secure: false,
            allow_public_register: false,
        };
        let auth = Arc::new(AuthService::open(cfg).unwrap());
        let pin = PinAccount {
            email: "test@example.com".into(),
            access_token: String::new(),
            device_id: None,
            proxy: None,
            user_agent: None,
            impersonate: None,
        };
        let st = Arc::new(AppState {
            helper: HelperClient::new("http://127.0.0.1:1").unwrap(),
            data_plane: DataPlane::Helper,
            pin: pin.clone(),
            accounts: Arc::new(Mutex::new(HashMap::from([(pin.email.clone(), pin)]))),
            listen: "127.0.0.1:0".into(),
            min_image_quota: 1,
            image_global_concurrency: 1,
            image_sem: Arc::new(Semaphore::new(1)),
            image_enabled: false,
            image_runtime: ImageRuntimeConfig::from_env(false),
            deadlock_guard: DeadlockGuard::from_env(),
            pipeline_watchdog: PipelineWatchdog::from_env(),
            auth,
            static_dir: None,
            image_assets: test_image_assets(),
            public_base_url: "http://127.0.0.1:8014".into(),
            scheduling_gate: SchedulingGate::from_env(),
            image_account_rr: AtomicUsize::new(0),
            image_queue_depth: AtomicUsize::new(0),
            duplicate_prompt: duplicate_prompt::DuplicatePromptGate::new(),
            binding_inflight: BindingInflightLedger::from_env(),
            dispatch_interval: DispatchIntervalGate::from_env(),
            slot_ledger: SlotLedger::from_env(),
            ready_buffer: ReadyBuffer::from_env(),
            return_window: ReturnWindow::from_env(),
            cooldown: CooldownRegistry::from_env(),
            pre_ticket: PreTicketPool::from_env(),
            proxy_cf: ProxyCfRegistry::from_env(),
            workload: WorkloadPolicy::from_env(),
            image_tasks: image_tasks::ImageTaskService::spawn(|_| async {}, 1),
            pg_pool: None,
            image_archive_store: None,
        });
        let app = Router::new()
            .route("/guarded", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(st.clone(), require_auth))
            .with_state(st);
        let api_key = concat!("panda-", "align-key");
        let ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .header("authorization", format!("Bearer {api_key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let bad = app
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .header("authorization", "Bearer wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_cookie_allows_me_route() {
        let st = test_app_state();
        let login_app = Router::new()
            .route("/login", post(login))
            .with_state(st.clone());
        let resp = login_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"integration-admin-pass"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cookie = resp
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(cookie.contains("gws_session="));

        let me_app = Router::new()
            .route("/me", get(me))
            .layer(middleware::from_fn_with_state(st.clone(), require_auth))
            .with_state(st);
        let me_resp = me_app
            .oneshot(
                Request::builder()
                    .uri("/me")
                    .header("cookie", cookie.split(';').next().unwrap_or(""))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me_resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn disabled_auth_me_returns_synthetic_user() {
        let path = std::env::temp_dir().join(format!("gw-dis-{}.db", uuid::Uuid::new_v4()));
        let cfg = AuthConfig {
            db_path: path.to_string_lossy().into(),
            jwt_secret: "integration-test-secret-32-bytes!!".into(),
            jwt_ttl_secs: 3600,
            cookie_name: "gws_session".into(),
            cookie_secure: false,
            allow_public_register: false,
            mode: AuthMode::Disabled,
            gateway_auth_key: None,
            bootstrap_user: None,
            bootstrap_password: None,
        };
        let auth = Arc::new(AuthService::open(cfg).unwrap());
        let pin = PinAccount {
            email: "test@example.com".into(),
            access_token: String::new(),
            device_id: None,
            proxy: None,
            user_agent: None,
            impersonate: None,
        };
        let st = Arc::new(AppState {
            helper: HelperClient::new("http://127.0.0.1:1").unwrap(),
            data_plane: DataPlane::Helper,
            pin: pin.clone(),
            accounts: Arc::new(Mutex::new(HashMap::from([(pin.email.clone(), pin)]))),
            listen: "127.0.0.1:0".into(),
            min_image_quota: 1,
            image_global_concurrency: 1,
            image_sem: Arc::new(Semaphore::new(1)),
            image_enabled: true,
            image_runtime: ImageRuntimeConfig::from_env(true),
            deadlock_guard: DeadlockGuard::from_env(),
            pipeline_watchdog: PipelineWatchdog::from_env(),
            auth,
            static_dir: None,
            image_assets: test_image_assets(),
            public_base_url: "http://127.0.0.1:8014".into(),
            scheduling_gate: SchedulingGate::from_env(),
            image_account_rr: AtomicUsize::new(0),
            image_queue_depth: AtomicUsize::new(0),
            duplicate_prompt: duplicate_prompt::DuplicatePromptGate::new(),
            binding_inflight: BindingInflightLedger::from_env(),
            dispatch_interval: DispatchIntervalGate::from_env(),
            slot_ledger: SlotLedger::from_env(),
            ready_buffer: ReadyBuffer::from_env(),
            return_window: ReturnWindow::from_env(),
            cooldown: CooldownRegistry::from_env(),
            pre_ticket: PreTicketPool::from_env(),
            proxy_cf: ProxyCfRegistry::from_env(),
            workload: WorkloadPolicy::from_env(),
            image_tasks: image_tasks::ImageTaskService::spawn(|_| async {}, 1),
            pg_pool: None,
            image_archive_store: None,
        });
        let me_app = Router::new()
            .route("/me", get(me))
            .layer(middleware::from_fn_with_state(st.clone(), require_auth))
            .with_state(st);
        let me_resp = me_app
            .oneshot(Request::builder().uri("/me").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(me_resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(me_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["user"]["username"], "dev");
        assert_eq!(v["user"]["role"], "admin");
    }

    #[tokio::test]
    async fn logout_revokes_jti_session() {
        let st = test_app_state();
        let claims = st
            .auth
            .authenticate("admin", "integration-admin-pass")
            .unwrap();
        let token = st.auth.issue_token(&claims).unwrap();

        let logout_app = Router::new()
            .route("/logout", post(logout))
            .with_state(st.clone());
        let resp = logout_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/logout")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let me_app = Router::new()
            .route("/me", get(me))
            .layer(middleware::from_fn_with_state(st.clone(), require_auth))
            .with_state(st);
        let me_resp = me_app
            .oneshot(
                Request::builder()
                    .uri("/me")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(me_resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn member_cannot_access_admin_middleware() {
        let st = test_app_state();
        st.auth
            .create_user("member1", "pass", Role::Member)
            .unwrap();
        let token = st
            .auth
            .issue_token(&st.auth.authenticate("member1", "pass").unwrap())
            .unwrap();
        let app = Router::new()
            .route("/admin-only", get(|| async { "ok" }))
            .layer(middleware::from_fn(require_admin))
            .layer(middleware::from_fn_with_state(st.clone(), require_auth))
            .with_state(st);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin-only")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// Router construction used to panic on wildcard + `allow_credentials`.
    /// Takes the spec directly so the cases don't race on process-wide env.
    #[test]
    fn cors_layer_builds_without_allowlist() {
        let _app: Router = Router::new()
            .route("/health", get(|| async { "ok" }))
            .layer(cors_layer_from(""));
    }

    #[test]
    fn cors_layer_builds_with_allowlist() {
        let _app: Router = Router::new()
            .route("/health", get(|| async { "ok" }))
            .layer(cors_layer_from("https://ui.example.com"));
    }

    #[test]
    fn cors_layer_builds_with_multiple_origins() {
        let _app: Router = Router::new()
            .route("/health", get(|| async { "ok" }))
            .layer(cors_layer_from(
                "https://a.example.com, https://b.example.com",
            ));
    }

    #[test]
    fn cors_layer_ignores_unparseable_origins() {
        // All entries invalid degrades to the wildcard branch, not a panic.
        let _app: Router = Router::new()
            .route("/health", get(|| async { "ok" }))
            .layer(cors_layer_from("not a header value\u{7f}, , "));
    }

    #[tokio::test]
    async fn health_does_not_leak_pool_identity() {
        let st = test_app_state();
        let app = Router::new().route("/health", get(health)).with_state(st);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&bytes);
        assert!(
            !body.contains("test@example.com"),
            "/health is unauthenticated and must not disclose pool addresses: {body}"
        );
        assert!(!body.contains("pin_email"), "got: {body}");
        assert!(body.contains("\"ok\":true"));
    }

    #[tokio::test]
    async fn disabled_user_is_rejected_by_middleware() {
        let st = test_app_state();
        let member = st
            .auth
            .create_user("member2", "member-pass", Role::Member)
            .unwrap();
        let token = st
            .auth
            .issue_token(&st.auth.authenticate("member2", "member-pass").unwrap())
            .unwrap();
        // Token stays cryptographically valid — only the DB row changes.
        st.auth.set_disabled(&member.id, true).unwrap();

        let app = Router::new()
            .route("/guarded", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(st.clone(), require_auth))
            .with_state(st);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/guarded")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "a still-valid token for a disabled user must not pass"
        );
    }

    #[tokio::test]
    async fn role_demotion_takes_effect_before_token_expiry() {
        let st = test_app_state();
        let user = st
            .auth
            .create_user("demoted", "demote-pass", Role::Admin)
            .unwrap();
        let token = st
            .auth
            .issue_token(&st.auth.authenticate("demoted", "demote-pass").unwrap())
            .unwrap();
        st.auth.set_role(&user.id, Role::Member).unwrap();

        let app = Router::new()
            .route("/admin-only", get(|| async { "ok" }))
            .layer(middleware::from_fn(require_admin))
            .layer(middleware::from_fn_with_state(st.clone(), require_auth))
            .with_state(st);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin-only")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "role must be re-read from the DB, not trusted from the token"
        );
    }

    #[tokio::test]
    async fn login_response_body_omits_token() {
        let st = test_app_state();
        let app = Router::new()
            .route("/login", post(login))
            .with_state(st.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"integration-admin-pass"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            body.get("token").is_none(),
            "JWT must stay in the HttpOnly cookie, out of reach of JS: {body}"
        );
    }

    #[tokio::test]
    async fn member_cannot_override_account_email() {
        let st = test_app_state();
        let err = resolve_account(&st, Some("victim@example.com".into()), false)
            .await
            .expect_err("member override must be refused");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn member_without_override_gets_pin_account() {
        let st = test_app_state();
        let acc = resolve_account(&st, None, false)
            .await
            .expect("pin account");
        assert_eq!(acc.email, "test@example.com");
    }

    #[tokio::test]
    async fn unknown_account_is_rejected_not_fabricated() {
        let st = test_app_state();
        // Helper is unreachable in tests, so the lookup cannot be satisfied.
        let err = resolve_account(&st, Some("ghost@example.com".into()), true)
            .await
            .expect_err("unknown account must not be fabricated with an empty token");
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn admin_override_resolves_known_account() {
        let st = test_app_state();
        {
            let mut guard = st.accounts.lock().await;
            guard.insert(
                "second@example.com".into(),
                PinAccount {
                    email: "second@example.com".into(),
                    access_token: "tok".into(),
                    device_id: None,
                    proxy: None,
                    user_agent: None,
                    impersonate: None,
                },
            );
        }
        let acc = resolve_account(&st, Some("second@example.com".into()), true)
            .await
            .expect("admin override");
        assert_eq!(acc.email, "second@example.com");
    }
}
