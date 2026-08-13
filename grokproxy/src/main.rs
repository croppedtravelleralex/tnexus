//! grokProxy — minimal Grok account pool with an OpenAI-compatible front door.

mod api;
mod config;
mod jwt;
mod model;
mod pool;
mod store;
mod upstream;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::api::AppState;
use crate::config::Config;
use crate::pool::Pool;
use crate::store::Store;
use crate::upstream::Upstream;

/// Unix seconds. Single source so tests and storage never disagree.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env();
    let store = Store::open(&config.database_path)?;
    let upstream = Upstream::new(&config.base_url, config.upstream_timeout_secs)
        .with_default_proxy(config.default_proxy.clone());
    let pool = Pool::new(store, upstream, config.max_attempts);

    if config.admin_key.is_empty() {
        // Loud, because an open ingest endpoint accepts anyone's credentials.
        tracing::warn!("GROKPROXY_ADMIN_KEY is empty — admin API is unauthenticated");
    }

    let listen = config.listen.clone();
    let state = Arc::new(AppState { pool, config });
    let app = api::router(state.clone()).layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    info!(
        addr = %listen,
        db = %state.config.database_path.display(),
        upstream = %state.config.base_url,
        // Logged without credentials so a misrouted egress is visible at a glance.
        egress = %redact_proxy(&state.config.default_proxy),
        "grokproxy listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("shutdown requested");
}

/// Strip `user:pass@` so proxy credentials never reach the log.
fn redact_proxy(proxy: &str) -> String {
    if proxy.is_empty() {
        return "(direct)".to_string();
    }
    match proxy.split_once("://") {
        Some((scheme, rest)) => match rest.rsplit_once('@') {
            Some((_, host)) => format!("{scheme}://***@{host}"),
            None => format!("{scheme}://{rest}"),
        },
        None => proxy.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::redact_proxy;

    #[test]
    fn credentials_never_reach_the_log() {
        assert_eq!(
            redact_proxy("http://user:secret@host:18100"),
            "http://***@host:18100"
        );
    }

    #[test]
    fn plain_proxy_and_empty_are_readable() {
        assert_eq!(
            redact_proxy("http://127.0.0.1:7897"),
            "http://127.0.0.1:7897"
        );
        assert_eq!(redact_proxy(""), "(direct)");
    }
}
