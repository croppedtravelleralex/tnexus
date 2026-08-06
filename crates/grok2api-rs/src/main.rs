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
//! - 鉴权（安全红线）：`GROK_GATEWAY_AUTH_KEY`/`GATEWAY_AUTH_KEY` → `/v1` 写操作
//!   Bearer/X-API-Key 校验（见 `grok-gateway::router`）；`/admin/*` 独立监听
//!   `GROK_ADMIN_LISTEN`（默认 `0.0.0.0:8091`，仅内网），login/refresh 绕过 guard。
//! - `/v1/responses` + `/v1/messages`：真实 Build/Console 后端（token 走
//!   `GROK2API_BUILD_TOKEN` / `GROK2API_CONSOLE_TOKEN`，base URL 可被
//!   `GROK2API_BUILD_BASE_URL` / `GROK2API_CONSOLE_BASE_URL` 覆盖）。
//! - 生图 / 媒体 / 视频后端未配置 → 503/501/500（路由可达，接线留 G2/G5 收尾）。
//!
//! DB 池懒连接：启动时 `connect_lazy`，DB 暂不可达时 `healthz` 仍 200，
//! 只有 `readyz` 才探 DB（响应已脱敏，不回传 DSN/内部错误）。

mod admin;
mod config;
mod http;
mod pg_admin;
mod tasks;

use std::net::SocketAddr;
use std::sync::Arc;

use admin::build_admin_bundle;
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
    if cfg.gateway_auth_key.is_none() {
        tracing::warn!(
            "GROK_GATEWAY_AUTH_KEY/GATEWAY_AUTH_KEY 未配置：/v1 写操作无鉴权，生产前必须设置"
        );
    }

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

    // 安全红线（Critical-1）：/v1 写操作鉴权密钥挂进 gateway state。
    let gateway_key = cfg.gateway_auth_key.clone();
    let v1_app = grok_gateway::build_app(
        gateway_state(&cfg, bridge, shared_pool, lease).with_gateway_auth_key(gateway_key),
    );

    // 安全红线（Critical-2/3）：/admin 独立端口（GROK_ADMIN_LISTEN，默认 :8091 仅内网），
    // login/refresh 绕过 guard（见 admin.rs）。DB 已配置 → PG 数据面（真实号池）；
    // 否则内存实现（测试/无 DB 降级，账号列表空）。
    let admin_bundle = if cfg.database_url.trim().is_empty() {
        build_admin_bundle(
            &cfg.admin_username,
            cfg.admin_password.as_deref(),
            &admin_secret,
        )
        .await
    } else {
        pg_admin::build_admin_bundle_pg(
            pool.clone(),
            &cfg.admin_username,
            cfg.admin_password.as_deref(),
            &admin_secret,
        )
        .await
    };

    // 后台任务（G6 切流前置）：GROK_TASKS_ENABLED=1 且 DB 就绪时启动 Build 四池探针
    // （TaskScheduler 包装，panic 续跑）。无 DB → 不启动并日志提示。
    let task_cfg = tasks::TaskConfig::from_env();
    let _background = if task_cfg.enabled {
        let bt = tasks::spawn_background_tasks(&task_cfg, repo);
        tracing::info!(
            "后台任务已注册: {:?}",
            bt.status_snapshot()
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
        );
        bt
    } else {
        tracing::info!("GROK_TASKS_ENABLED 未设置：后台任务未启动");
        tasks::BackgroundTasks::empty()
    };

    let state = Arc::new(http::AppState { pool: pool.clone() });
    let health_app = build_router(state);

    let v1_addr: SocketAddr = cfg.server_addr.parse()?;
    let admin_addr: SocketAddr = cfg.admin_listen.parse()?;
    let v1_listener = tokio::net::TcpListener::bind(v1_addr).await?;
    let admin_listener = tokio::net::TcpListener::bind(admin_addr).await?;
    tracing::info!("grok2api-rs /v1+healthz listening on {v1_addr}");
    tracing::info!("grok2api-rs /admin listening on {admin_addr}（仅内网）");

    let v1_server = async { axum::serve(v1_listener, health_app.merge(v1_app)).await };
    let admin_server = async { axum::serve(admin_listener, admin::admin_app(admin_bundle)).await };
    tokio::select! {
        r = v1_server => r?,
        r = admin_server => r?,
    }
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
    // /v1/responses + /v1/messages：真实 Build/Console 后端（token 未配置时为 None → 503）。
    let (responses, messages) =
        default_protocol_backends(cfg.build_base_url.clone(), cfg.console_base_url.clone());
    let engine = ChatEngine::new(pool.clone(), lease.clone(), bridge.clone(), None);
    let mut state = AppState {
        engine: Some(Arc::new(engine)),
        responses_backend: responses,
        messages_backend: messages,
        ..AppState::empty()
    };
    // 生图：GROK_IMAGE_ENABLED=1 时接真实 ImageEngine（pool/lease/bridge 已就绪，
    // 与 chat 同链路）；未开启 → 路由可达但 500（明确错误，不外呼）。
    if cfg.image_enabled {
        let image_engine = grok_provider_web::ImageEngine::new(
            pool,
            lease,
            bridge,
            None,
            grok_image_pipeline::ImagePipeline::new(
                grok_image_pipeline::SlotManager::new(&[("ps", 2), ("ss", 1)]),
                Arc::new(grok_image_pipeline::InMemoryTraceRepository::new()),
            ),
        );
        state.image_engine = Some(Arc::new(image_engine));
        tracing::info!("GROK_IMAGE_ENABLED=1：/v1/images/generations 已接线真实引擎");
    } else {
        tracing::warn!("GROK_IMAGE_ENABLED 未设置：生图路由 500（需 bridge + 票池侧车）");
    }
    // 媒体/视频后端未接线 → 501/500（G2/G5 收尾 TODO：media fetcher 需存储 + 视频需上游轮询）。
    state
}

/// 生产默认：PG 加载号池（调用方已 load）+ 内存/Redis lease + HTTP bridge 侧车。
/// 测试复用：注入 mock bridge 构建 `/v1` 路由（无鉴权）。
#[allow(dead_code)]
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
            gateway_auth_key: None,
            admin_listen: "127.0.0.1:0".to_string(),
            admin_secret: "12345678901234567890123456789012".to_string(),
            admin_username: "admin".to_string(),
            admin_password: Some("admin123456".to_string()),
            image_enabled: false,
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
        let bundle = build_admin_bundle(
            &cfg.admin_username,
            cfg.admin_password.as_deref(),
            &cfg.admin_secret,
        )
        .await;
        build_router(state)
            .merge(gateway_app(&cfg, pool, lease, Arc::new(mock)))
            .merge(admin::admin_app(bundle))
    }

    /// 带 `/v1` 鉴权密钥的测试 app。
    async fn app_with_mock_bridge_and_key(chat_text: &str, key: &str) -> axum::Router {
        let app = app_with_mock_bridge(chat_text).await;
        // 复用同一 app：无法就地换 key，直接重新组装 gateway 段。
        // 这里仅用于鉴权行为验证（见 v1_auth_* 测试），直接构造带 key 的独立 app。
        drop(app);
        let mut mock = grok_provider_web::MockBridgeClient::new();
        mock.chat_text = chat_text.to_string();
        let pool: SharedPool = Arc::new(SimplifiedPool::new());
        let lease: Arc<dyn LeaseManager> =
            Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]));
        let cfg = test_cfg();
        grok_gateway::build_app(
            gateway_state(&cfg, Arc::new(mock), pool, lease)
                .with_gateway_auth_key(Some(key.to_string())),
        )
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
    async fn readyz_sanitized_when_db_down() {
        // 懒连接池连不上：readyz 应 503 且 body 不含 DSN/detail。
        let app = app_with_mock_bridge("你好").await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["db"], "error");
        assert!(
            body.get("detail").is_none(),
            "readyz 不得回传内部错误 detail"
        );
        assert!(
            !text.contains("postgres://"),
            "readyz 不得泄露 DB DSN: {text}"
        );
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
        for (uri, body) in [("/v1/responses", r#"{}"#), ("/v1/messages", r#"{}"#)] {
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
    async fn v1_auth_rejects_without_key_when_configured() {
        // 配置了 GATEWAY_AUTH_KEY：POST 无凭据 → 401；GET 仍开放。
        let app = app_with_mock_bridge_and_key("你好", "sekret").await;
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"grok-chat","messages":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "无凭据 POST 应 401"
        );
        // 错误响应是结构化 JSON（非纯文本/空）。
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "unauthorized");
        // GET /v1/models 不要求鉴权。
        assert_eq!(get_status(app.clone(), "/v1/models").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn v1_auth_accepts_bearer_and_api_key() {
        let app = app_with_mock_bridge_and_key("你好", "sekret").await;
        let body = r#"{"model":"grok-chat","messages":[{"role":"user","content":"hi"}]}"#;
        for header in [("authorization", "Bearer sekret"), ("x-api-key", "sekret")] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/chat/completions")
                        .header("content-type", "application/json")
                        .header(header.0, header.1)
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            // 鉴权放行的标志是「未被 401 拒绝」：空池会 503（无账号），同样证明已过鉴权。
            assert_ne!(
                resp.status(),
                StatusCode::UNAUTHORIZED,
                "{} 应放行（got {:?}）",
                header.0,
                resp.status()
            );
        }
    }

    #[tokio::test]
    async fn v1_auth_rejects_wrong_key() {
        let app = app_with_mock_bridge_and_key("你好", "sekret").await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer wrong")
                    .body(Body::from(
                        r#"{"model":"grok-chat","messages":[{"role":"user","content":"hi"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "错误密钥应 401");
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
    async fn admin_login_roundtrip_then_access() {
        // 安全红线（Critical-2）：login 绕过 guard 签发 token，随后携带该 token
        // 访问受保护端点应 200（共享同一内存 session store）。
        let cfg = test_cfg();
        let bundle = build_admin_bundle(
            &cfg.admin_username,
            cfg.admin_password.as_deref(),
            &cfg.admin_secret,
        )
        .await;
        let app = admin::admin_app(bundle);

        // 1) login（无任何 guard）→ 200 + access_token
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"admin123456"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "login 应 200（绕过 guard）");
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let token = body["tokens"]["access_token"].as_str().unwrap().to_string();
        assert!(!token.is_empty());

        // 2) 携带 token 访问受保护端点 → 200
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/admin/accounts")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "携带 token 应 200");
    }

    #[tokio::test]
    async fn admin_login_wrong_password_401() {
        let cfg = test_cfg();
        let bundle = build_admin_bundle(
            &cfg.admin_username,
            cfg.admin_password.as_deref(),
            &cfg.admin_secret,
        )
        .await;
        let app = admin::admin_app(bundle);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"wrong-pass"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "错误密码应 401");
    }

    #[tokio::test]
    async fn admin_login_disabled_without_password() {
        // GROK_ADMIN_PASSWORD 未配置：无管理员，login 恒 401（不 bootstrap）。
        let cfg = test_cfg();
        let bundle = build_admin_bundle(&cfg.admin_username, None, &cfg.admin_secret).await;
        let app = admin::admin_app(bundle);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"username":"admin","password":"whatever123"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "未 bootstrap 应 401"
        );
    }
    #[test]
    fn gateway_state_image_engine_switch() {
        let mut cfg = test_cfg();
        let mock = Arc::new(grok_provider_web::MockBridgeClient::new());
        let pool: SharedPool = Arc::new(SimplifiedPool::new());
        let lease = Arc::new(grok_egress::InMemoryLeaseManager::new(&[(
            grok_domain::Scope::GrokWeb,
            4,
        )]));
        // 默认关闭 → None
        let state = gateway_state(&cfg, mock.clone(), pool.clone(), lease.clone());
        assert!(state.image_engine.is_none(), "默认应无生图引擎");
        // 开启 → Some（真实 ImageEngine 组装成功）
        cfg.image_enabled = true;
        let state2 = gateway_state(&cfg, mock, pool, lease);
        assert!(
            state2.image_engine.is_some(),
            "GROK_IMAGE_ENABLED=1 应组装真实 ImageEngine"
        );
    }
}
