use crate::config::AppConfig;
use redis::aio::ConnectionManager;
use sqlx::PgPool;
use std::sync::Arc;
use tnexus_auth::AuthService;
use tnexus_storage::SharedStorage;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub pool: PgPool,
    pub auth: AuthService,
    pub redis: ConnectionManager,
    pub storage: Option<SharedStorage>,
    pub http: reqwest::Client,
}

impl AppState {
    pub async fn new(config: AppConfig, pool: PgPool) -> anyhow::Result<Self> {
        let auth = AuthService::new(
            pool.clone(),
            config.jwt_secret.clone(),
            config.jwt_ttl_secs,
        )?;

        if let (Some(email), Some(password)) = (
            config.bootstrap_admin_email.clone(),
            config.bootstrap_admin_password.clone(),
        ) {
            auth.bootstrap_admin(&email, &password, "Admin").await?;
        }

        if let (Some(email), Some(password)) = (
            config.bootstrap_demo_email.clone(),
            config.bootstrap_demo_password.clone(),
        ) {
            auth.ensure_member_account(&email, &password, "Demo User")
                .await?;
        }

        let redis_client = redis::Client::open(config.redis_url.as_str())?;
        let redis = ConnectionManager::new(redis_client).await?;

        let storage = if let Some(r2_cfg) = &config.r2 {
            let storage = tnexus_storage::AssetStorage::from_config(r2_cfg).await?;
            Some(Arc::new(storage))
        } else {
            None
        };

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        Ok(Self {
            config,
            pool,
            auth,
            redis,
            storage,
            http,
        })
    }
}
