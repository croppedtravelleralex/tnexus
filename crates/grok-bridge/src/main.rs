//! grok-bridge 入口：`:8192` axum 服务（/health + /v1/*）。
//!
//! env：
//! - `GROK_BRIDGE_ADDR` 监听地址，缺省 `0.0.0.0:8192`
//! - `GROK_BRIDGE_KEY` 鉴权 key（缺省未配置 → 非 /health 一律 401）
//! - `GROK_BRIDGE_CHROME_PATH` Chrome/Edge 可执行路径（缺省自动探测）
//! - `BRIDGE_SIGNER_MODULE_ID` grok.com 签名器模块号（缺省 4629918）

use std::sync::Arc;

use grok_bridge::handlers::BridgeState;
use grok_bridge::session::{ChromeCdpFactory, SessionPool};

fn addr_from_env() -> String {
    std::env::var("GROK_BRIDGE_ADDR").unwrap_or_else(|_| "0.0.0.0:8192".to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let key = grok_bridge::auth::configured_key();
    let chrome_path = std::env::var("GROK_BRIDGE_CHROME_PATH").ok();
    let factory = Arc::new(ChromeCdpFactory::new(chrome_path));
    let pool = Arc::new(SessionPool::new(factory));
    let state = BridgeState {
        pool,
        key: Arc::new(key),
    };
    let addr = addr_from_env();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(
        addr = %addr,
        key_configured = !state.key.is_empty(),
        "grok-bridge listening"
    );
    axum::serve(listener, grok_bridge::handlers::build_router(state)).await?;
    Ok(())
}
