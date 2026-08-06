//! grok2api-rs 顶层入口（G0 入口 + config + healthz/readyz；N5 起挂载 grok-gateway /v1/*）。
//!
//! N5（2026-08-06）：把 grok-gateway 的全部 `/v1/*` 路由挂到 `:8000`：
//! `/v1/models`、`/v1/chat/completions`、`/v1/images/generations`、
//! `/v1/media/images/{id}`、`/v1/responses`、`/v1/messages`、`/v1/videos`。
//!
//! 最小可用默认值（真实接线留 TODO）：
//! - chat engine：browser-bridge 侧车（`GROK2API_BROWSER_BRIDGE_URL`）+ 内存号池
//!   （`SimplifiedPool`，空池无账号）+ 内存 lease（`InMemoryLeaseManager`）；
//!   PG 号池 / Redis lease / grok-ops 探针接线留 G6 切流前。
//! - `/v1/responses` + `/v1/messages`：真实 Build/Console 后端（token 走
//!   `GROK2API_BUILD_TOKEN` / `GROK2API_CONSOLE_TOKEN`，base URL 可被
//!   `GROK2API_BUILD_BASE_URL` / `GROK2API_CONSOLE_BASE_URL` 覆盖）。
//! - 生图 / 媒体 / 视频后端未配置 → 503/501/500（路由可达，接线留 G2/G5 收尾）。
//!
//! DB 池懒连接：启动时 `connect_lazy`，DB 暂不可达时 `healthz` 仍 200，
//! 只有 `readyz` 才探 DB。

mod config;
mod http;

use std::net::SocketAddr;
use std::sync::Arc;

use http::build_router;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use grok_domain::egress::Scope;
use grok_egress::{InMemoryLeaseManager, LeaseManager};
use grok_gateway::{default_protocol_backends, AppState};
use grok_pool::{SharedPool, SimplifiedPool};
use grok_provider_web::{BridgeClient, ChatEngine, HttpBridgeClient};

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
    tracing::info!(
        "config loaded: addr={} bridge={}",
        cfg.server_addr,
        cfg.browser_bridge_url
    );

    let db_url = cfg.database_url.clone();
    let pool = PgPoolOptions::new()
        .max_connections(5)
        // 懒连接：连接建立推迟到首次使用（readyz / 未来查询），
        // 允许 DB 尚未就绪时进程也能启动，healthz 保持 200。
        .connect_lazy(&db_url)?;

    // N5：healthz/readyz 与 grok-gateway /v1/* 合并为单一 axum app。
    let state = Arc::new(http::AppState { pool });
    let app = build_router(state).merge(gateway_app(&cfg));

    let addr: SocketAddr = cfg.server_addr.parse()?;
    tracing::info!("grok2api-rs listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
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

/// 生产默认：内存号池 + 内存 lease + HTTP bridge 侧车。
fn gateway_app(cfg: &config::Config) -> axum::Router {
    let pool: SharedPool = Arc::new(SimplifiedPool::new());
    let lease: Arc<dyn LeaseManager> =
        Arc::new(InMemoryLeaseManager::new(&[(Scope::GrokWeb, 4)]));
    let bridge: Arc<dyn BridgeClient> = Arc::new(HttpBridgeClient::new());
    grok_gateway::build_app(gateway_state(cfg, bridge, pool, lease))
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
        }
    }

    /// 懒连接池（不触发真实连接）；readyz 在无 DB 时 503，测试只探 healthz/路由形状。
    fn lazy_pool() -> sqlx::PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://user:pass@localhost:5432/grok")
            .expect("lazy pool")
    }

    /// 测试用 app：mock bridge + 单账号池，健康路由与 gateway 合并。
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
        build_router(state).merge(grok_gateway::build_app(gateway_state(
            &cfg,
            Arc::new(mock),
            pool,
            lease,
        )))
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
}
