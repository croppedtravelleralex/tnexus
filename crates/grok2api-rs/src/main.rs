//! grok2api-rs 顶层入口（G0，39a G0-3/G0-4，39e G0-P6）。
//!
//! G0 只做入口 + config + healthz/readyz，不启用任何 provider / 号池 /
//! 后台任务（那些属 G1+ / G4）。DB 池懒连接：启动时 `connect_lazy`，
//! DB 暂不可达健康检查 `healthz` 仍 200，只有 `readyz` 才探 DB。

mod config;
mod http;

use std::net::SocketAddr;
use std::sync::Arc;

use http::build_router;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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

    let state = Arc::new(http::AppState { pool });
    let app = build_router(state);

    let addr: SocketAddr = cfg.server_addr.parse()?;
    tracing::info!("grok2api-rs listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
