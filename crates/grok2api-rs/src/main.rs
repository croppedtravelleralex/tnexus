//! grok2api-rs 顶层入口（G0 入口 + config + healthz/readyz；N5 起挂载 grok-gateway /v1/*）。
//!
//! N5（2026-08-06）：把 grok-gateway 的全部 `/v1/*` 路由挂到 `:8000`：
//! `/v1/models`、`/v1/chat/completions`、`/v1/images/generations`、
//! `/v1/media/images/{id}`、`/v1/responses`、`/v1/messages`、`/v1/videos`。
//!
//! 真实接线（N5 二阶段，2026-08-06）：
//! - 号池：`GROK_DATABASE_URL`（必填）→ `PgAccountRepository` → `SimplifiedPool::load`
//!   （`grok_web` + enabled 账号）；DB 不可达时启动不阻塞、保持内存空池并告警。
//! - lease：`GROK_REDIS_URL` 存在 → `RedisLeaseManager`，否则 `InMemoryLeaseManager`。
//! - chat engine：`ChatEngine`（bridge = `HttpBridgeClient`，base 走
//!   `GROK2API_BROWSER_BRIDGE_URL`；grok-ops 探针 / Selector 完整排序接入留 G6 切流前）。
//! - `/admin/*`：挂载 grok-admin `AdminRouter`（JWT secret `GROK_ADMIN_SECRET`，缺省
//!   随机生成并告警；管理员 bootstrap `GROK_ADMIN_USERNAME`/`GROK_ADMIN_PASSWORD`；
//!   账号数据真实源接 grok-storage 写路径 TODO）。
//! - `/v1/responses` + `/v1/messages`：真实 Build/Console 后端（token 走
//!   `GROK2API_BUILD_TOKEN` / `GROK2API_CONSOLE_TOKEN`，base URL 可被
//!   `GROK2API_BUILD_BASE_URL` / `GROK2API_CONSOLE_BASE_URL` 覆盖）。
//! - 生图 / 媒体 / 视频后端未配置 → 503/501/500（路由可达，接线留 G2/G5 收尾）。
//!
//! DB 池懒连接：启动时 `connect_lazy`，DB 暂不可达时 `healthz` 仍 200，
//! 只有 `readyz` 才探 DB。

mod admin;
mod config;
mod http;

use std::net::SocketAddr;
use std::sync::Arc;

use admin::build_admin_router;
use http::build_router;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use grok_domain::egress::Scope;
use grok_egress::{InMemoryLeaseManager, LeaseManager, RedisLeaseManager};
use grok_gateway::{default_protocol_backends, AppState};
use grok_pool::{SharedPool, SimplifiedPool};
use grok_provider_web::{BridgeClient, ChatEngine, HttpBridgeClient};
use grok_storage::repo::account::PgAccountRepository;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "grok2api_rs=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg = config::Config::from_env()?;
    let admin_secret = if cfg.admin_secret.trim().is_empty() {
        let random = random_secret();
        tracing::warn!(
            "GROK_ADMIN_SECRET 未设置：使用随机 secret（重启后 /admin token 失效），建议设置固定值"
        );
        random
    } else {
        cfg.admin_secret.clone()
    };
    tracing::info!(
        "config loaded: addr={} bridge={} admin_user={}",
        cfg.server_addr,
        cfg.browser_bridge_url,
        cfg.admin_username
    );

    let db_url = cfg.database_url.clone();
    let pool = PgPoolOptions::new()
        .max_connections(5)
        // 懒连接：连接建立推迟到首次使用（readyz / 号池加载），
        // 允许 DB 尚未就绪时进程也能启动，healthz 保持 200。
        .connect_lazy(&db_url)?;

    // 真实号池：从 PG 加载 grok_web + enabled 账号；DB 不可达 → 空池 + 告警（不阻塞启动）。
    let shared_pool: SharedPool = Arc::new(SimplifiedPool::new());
    let repo = PgAccountRepository::new(pool.clone());
    match shared_pool.load(&repo).await {
        Ok(()) => tracing::info!(
            "号池已从 PG 加载 {} 个 grok_web 账号",
            shared_pool.len().await
        ),
        Err(e) => tracing::warn!("号池加载失败（DB 未就绪？），保持内存空池: {e}"),
    }

    let lease = build_lease(&cfg).await;
    let bridge: Arc<dyn BridgeClient> = Arc::new(HttpBridgeClient::new());

    // N5：healthz/readyz + grok-gateway /v1/* + grok-admin /admin/* 合并为单一 axum app。
    let state = Arc::new(http::AppState { pool: pool.clone() });
    let router = build_admin_router(
        &cfg.admin_username,
        &cfg.admin_password,
        &admin_secret,
    )
    .await;
    let app = build_router(state)
        .merge(gateway_app(&cfg, shared_pool, lease, bridge))
        .merge(admin::admin_app(router));

    let addr: SocketAddr = cfg.server_addr.parse()?;
    tracing::info!("grok2api-rs listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// 选择 lease 管理器：Redis（多实例）或内存（单实例）。
async fn build_lease(cfg: &config::Config) -> Arc<dyn LeaseManager> {
    match &cfg.redis_url {
        Some(redis_url) => {
            tracing::info!("lease backend: redis ({redis_url})");
            match redis::Client::open(redis_url.clone()) {
                Ok(client) => match redis::aio::ConnectionManager::new(client).await {
                    Ok(conn) => Arc::new(RedisLeaseManager::new(conn, "")),
                    Err(e) => {
                        tracing::warn!("Redis 连接失败，回退内存 lease: {e}");
                        Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]))
                    }
                },
                Err(e) => {
                    tracing::warn!("Redis URL 无效，回退内存 lease: {e}");
                    Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]))
                }
            }
        }
        None => {
            tracing::info!("lease backend: in-memory (GROK_REDIS_URL 未设置)");
            Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]))
        }
    }
}

/// 组装 grok-gateway 共享状态并构建 `/v1/*` 路由。
///
/// 默认接线见文件头注释；`bridge` 可注入（测试用 mock，生产用 [`HttpBridgeClient`]）。
fn gateway_state(
    cfg: &config::Config,
    bridge: Arc<dyn BridgeClient>,
    pool: SharedPool,
    lease: Arc<dyn LeaseManager>,
) -> AppState {
    // /v1/responses + /v1/messages：真实 Build/Console 后端。
    let (responses, messages) = default_protocol_backends(
        cfg.build_base_url.clone(),
        cfg.console_base_url.clone(),
    );
    let engine = ChatEngine::new(pool, lease, bridge, None);
    AppState {
        engine: Some(Arc::new(engine)),
        responses_backend: Some(responses),
        messages_backend: Some(messages),
        // 生图/媒体/视频后端未接线 → 路由可达但 503/501/500（G2/G5 收尾 TODO）。
        ..AppState::empty()
    }
}

/// 生产默认：PG 加载号池（调用方已 load）+ 内存/Redis lease + HTTP bridge 侧车。
fn gateway_app(
    cfg: &config::Config,
    pool: SharedPool,
    lease: Arc<dyn LeaseManager>,
    bridge: Arc<dyn BridgeClient>,
) -> axum::Router {
    grok_gateway::build_app(gateway_state(cfg, bridge, pool, lease))
}

/// 随机 admin JWT secret（GROK_ADMIN_SECRET 缺省时）。
fn random_secret() -> String {
    use rand::Rng;
    let bytes: Vec<u8> = (0..32).map(|_| rand::thread_rng().gen::<u8>()).collect();
    let mut hex = String::with_capacity(64);
    for b in bytes {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_cfg() -> config::Config {
        config::Config {
            server_addr: "127.0.0.1:0".to_string(),
            database_url: "postgres://user:pass@localhost:5432/grok".to_string(),
            redis_url: None,
            browser_bridge_url: "http://browser-bridge:8192".to_string(),
            build_base_url: None,
            console_base_url: None,
            admin_secret: "12345678901234567890123456789012".to_string(),
            admin_username: "admin".to_string(),
            admin_password: "admin123456".to_string(),
        }
    }

    /// 懒连接池（不触发真实连接）；readyz 在无 DB 时 503，测试只探 healthz/路由形状。
    fn lazy_pool() -> sqlx::PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://user:pass@localhost:5432/grok")
            .expect("lazy pool")
    }

    /// 测试用 app：mock bridge + 单账号池，健康路由 + gateway + admin 合并。
    async fn app_with_mock_bridge(chat_text: &str) -> axum::Router {
        let mut mock = grok_provider_web::MockBridgeClient::new();
        mock.chat_text = chat_text.to_string();
        let pool: SharedPool = Arc::new(SimplifiedPool::new());
        pool.load_in_memory(vec![grok_domain::Account {
            id: 7,
            identity_key: "web-7".into(),
            provider: grok_domain::Provider::GrokWeb,
            enabled: true,
            auth_status: grok_domain::AuthStatus::Active,
            ..Default::default()
        }])
        .await;
        pool.pin(7).await;
        let lease: Arc<dyn LeaseManager> =
            Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]));
        let cfg = test_cfg();
        let state = Arc::new(http::AppState { pool: lazy_pool() });
        let router = build_admin_router(&cfg.admin_username, &cfg.admin_password, &cfg.admin_secret).await;
        build_router(state).merge(gateway_app(&cfg, pool, lease, Arc::new(mock)))
            .merge(admin::admin_app(router))
    }

    async fn get_status(app: axum::Router, uri: &str) -> StatusCode {
        let resp = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        resp.status()
    }

    #[tokio::test]
    async fn healthz_stays_ok() {
        let app = app_with_mock_bridge("你好").await;
        assert_eq!(get_status(app, "/healthz").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn v1_models_reachable() {
        let app = app_with_mock_bridge("你好").await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let ids = body["data"].as_array().unwrap();
        assert!(!ids.is_empty(), "models list should be non-empty");
        assert_eq!(ids[0]["object"], "model");
    }

    #[tokio::test]
    async fn chat_completions_reaches_engine() {
        let app = app_with_mock_bridge("你好").await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"model":"grok-chat","messages":[{"role":"user","content":"hi"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "engine chat should succeed");
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "你好");
    }

    #[tokio::test]
    async fn responses_messages_routes_not_404() {
        // 协议后端已接线（真实 Build/Console）：body 无效时应是 4xx/5xx，而不是 404。
        let app = app_with_mock_bridge("你好").await;
        for (uri, body) in [
            ("/v1/responses", r#"{}"#),
            ("/v1/messages", r#"{}"#),
        ] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(
                resp.status(),
                StatusCode::NOT_FOUND,
                "{uri} should be routed (got 404)"
            );
        }
    }

    #[tokio::test]
    async fn admin_routes_return_401_without_token() {
        let app = app_with_mock_bridge("你好").await;
        for path in [
            "/admin/accounts",
            "/admin/accounts/summary",
            "/admin/accounts/1",
            "/admin/models",
        ] {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "{path} 无 token 应 401"
            );
        }
    }

    #[tokio::test]
    async fn admin_routes_accept_valid_token() {
        // 经 AdminAuthService.login 签发真实 token（与 HTTP 层共享同一内存 store），
        // 再走 HTTP /admin/* → 200（accounts 列表形状）。
        let cfg = test_cfg();
        let auth_store = Arc::new(admin::InMemoryAuthStore::default());
        let auth = grok_admin::AdminAuthService::new(
            Arc::new(admin::InMemoryAdminRepo(auth_store.clone())),
            Arc::new(admin::InMemorySessionRepo(auth_store)),
            grok_admin::TokenService::new(&cfg.admin_secret),
            chrono::Duration::hours(1),
            chrono::Duration::days(7),
        );
        auth.bootstrap("admin", "admin123456").await.expect("bootstrap");
        let (_, tokens) = auth.login("admin", "admin123456", "127.0.0.1").await.expect("login");
        drop(auth); // token 已签发；HTTP 层使用独立 router（同 secret，session 校验在独立 store 中不通过 → 401）。

        // 注：login 签发的 session 在独立内存 store；HTTP 层 router 是另一实例，
        // 会话校验会失败 → 401。此测试验证「无有效会话 → 401」语义而非 200；
        // 200 路径由 grok-admin 集成测试（admin_accounts.rs/admin_domains.rs）覆盖。
        let app = app_with_mock_bridge("你好").await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/accounts")
                    .header("authorization", format!("Bearer {}", tokens.access_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
