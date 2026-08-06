mod auth;
mod nurture;
mod oauth;
mod ops;
mod pkce;
mod refresh;
mod relogin;
mod routes;
mod usage_events;
mod user_info;
mod workers;

use axum::Router;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("tnexus_account_ops=info".parse()?),
        )
        .init();

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let ops = Arc::new(ops::OpsServices::new());
    workers::spawn_all(ops.clone(), http.clone());

    let state = Arc::new(routes::AppState {
        oauth: Arc::new(oauth::OAuthLoginService::new(http.clone())),
        http,
        ops,
    });

    let protected = routes::api_router(state).layer(axum::middleware::from_fn(auth::require_token));
    let app = Router::new()
        .route("/health", axum::routing::get(routes::health))
        .merge(protected);

    let listen = auth::listen_addr();
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!(%listen, "tnexus-account-ops listening");
    axum::serve(listener, app).await?;
    Ok(())
}
