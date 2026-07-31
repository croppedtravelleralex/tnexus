mod account_ops;
mod accounts_store;
mod config;
mod gptimage_proxy;
mod jobs;
mod middleware;
mod models;
mod routes;
mod state;
mod usage_metrics;

use axum::{
    http::{header, Method, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use config::AppConfig;
use sqlx::postgres::PgPoolOptions;
use state::AppState;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "tnexus_api=info,tower_http=info".into()
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::from_env()?;
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;

    let state = Arc::new(AppState::new(config.clone(), pool).await?);
    let app = build_router(state.clone());

    let addr: SocketAddr = config.listen_addr.parse()?;
    tracing::info!("tnexus-api listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .nest("/api/auth", routes::auth::routes())
        .nest("/api/conversations", routes::conversations::routes())
        .nest("/api/jobs", routes::jobs::routes())
        .nest("/api/accounts", routes::accounts::routes())
        .nest("/api/logs", routes::media::routes())
        .nest("/api/images", routes::media::image_routes())
        .nest("/api/ops", routes::ops::routes())
        .with_state(state.clone());

    let cors = build_cors(&state.config.cors_origins);

    let mut app = Router::new().merge(api).layer(cors).layer(TraceLayer::new_for_http());

    if let Some(dir) = &state.config.static_dir {
        let index = format!("{dir}/index.html");
        app = app.fallback_service(
            ServeDir::new(dir)
                .not_found_service(tower::service_fn(move |_| {
                    let index = index.clone();
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            axum::response::Response::builder()
                                .status(StatusCode::OK)
                                .header(header::CONTENT_TYPE, "text/html")
                                .body(axum::body::Body::from(
                                    tokio::fs::read(index).await.unwrap_or_default(),
                                ))
                                .unwrap(),
                        )
                    }
                }))
                .append_index_html_on_directories(true),
        );
    }

    app
}

fn build_cors(origins: &[String]) -> CorsLayer {
    let allowed: Vec<axum::http::HeaderValue> = origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(allowed))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        .allow_credentials(true)
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "r2": state.storage.is_some(),
        "static_ui": state.config.static_dir.is_some(),
    }))
}

use axum::extract::State;
