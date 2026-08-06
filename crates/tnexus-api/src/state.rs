use crate::account_ops;
use crate::accounts_store::AccountsStore;
use crate::config::AppConfig;
use crate::local_nurture::{LocalNurtureStore, OutlookRecoveryStore};
use crate::quota_prime_job::QuotaPrimeJob;
use crate::refresh_all::RefreshAllStore;
use redis::aio::ConnectionManager;
use sqlx::PgPool;
use std::sync::Arc;
use tnexus_auth::AuthService;
use tnexus_storage::SharedImageStore;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub pool: PgPool,
    pub auth: AuthService,
    pub redis: ConnectionManager,
    pub redis_client: redis::Client,
    pub image_store: Option<SharedImageStore>,
    pub http: reqwest::Client,
    pub accounts: AccountsStore,
    pub refresh_progress: account_ops::ProgressStore,
    pub relogin_progress: account_ops::ProgressStore,
    pub refresh_all: RefreshAllStore,
    pub quota_prime: QuotaPrimeJob,
    pub nurture_store: LocalNurtureStore,
    pub outlook_recovery: OutlookRecoveryStore,
}

impl AppState {
    pub async fn new(config: AppConfig, pool: PgPool) -> anyhow::Result<Self> {
        let auth = AuthService::new(pool.clone(), config.jwt_secret.clone(), config.jwt_ttl_secs)?;

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
            auth.ensure_member_account(&email, &password, "User")
                .await?;
        }

        let redis_client = redis::Client::open(config.redis_url.as_str())?;
        let redis = ConnectionManager::new(redis_client.clone()).await?;

        let image_store = tnexus_storage::ImageStore::from_env().await?.map(Arc::new);

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        let accounts = AccountsStore::from_env_with_pool(Some(pool.clone())).unwrap_or_default();

        Ok(Self {
            config,
            pool,
            auth,
            redis,
            redis_client,
            image_store,
            http,
            accounts,
            refresh_progress: account_ops::ProgressStore::new(),
            relogin_progress: account_ops::ProgressStore::new(),
            refresh_all: RefreshAllStore::new(),
            quota_prime: QuotaPrimeJob::new(),
            nurture_store: LocalNurtureStore::new(),
            outlook_recovery: OutlookRecoveryStore::new(),
        })
    }
}
