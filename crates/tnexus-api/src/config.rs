use anyhow::{Context, Result};
use std::env;
use tnexus_storage::R2Config;

#[derive(Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub redis_url: String,
    pub jwt_secret: String,
    pub jwt_ttl_secs: u64,
    pub cookie_name: String,
    pub cookie_secure: bool,
    pub listen_addr: String,
    pub static_dir: Option<String>,
    pub cors_origins: Vec<String>,
    pub gptimage_base: String,
    pub gptimage_admin_token: Option<String>,
    pub account_ops_base: String,
    pub account_ops_token: Option<String>,
    pub grok2api_base: String,
    pub director_model: String,
    pub r2: Option<R2Config>,
    pub bootstrap_admin_email: Option<String>,
    pub bootstrap_admin_password: Option<String>,
    pub bootstrap_demo_email: Option<String>,
    pub bootstrap_demo_password: Option<String>,
    pub presign_ttl_secs: u64,
    pub gateway_base: String,
    pub gateway_internal_token: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let r2 = if env::var("R2_BUCKET").is_ok() {
            Some(R2Config {
                account_id: env_required("R2_ACCOUNT_ID")?,
                access_key_id: env_required("R2_ACCESS_KEY_ID")?,
                secret_access_key: env_required("R2_SECRET_ACCESS_KEY")?,
                bucket: env_required("R2_BUCKET")?,
                endpoint: env::var("R2_ENDPOINT").ok(),
            })
        } else {
            None
        };

        Ok(Self {
            database_url: env_required("DATABASE_URL")?,
            redis_url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
            jwt_secret: env_required("JWT_SECRET")?,
            jwt_ttl_secs: env::var("JWT_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(86400),
            cookie_name: env::var("AUTH_COOKIE_NAME").unwrap_or_else(|_| "tnexus_session".into()),
            cookie_secure: env::var("AUTH_COOKIE_SECURE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            listen_addr: env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:9000".into()),
            static_dir: env::var("GATEWAY_STATIC_DIR").ok(),
            cors_origins: env::var("CORS_ORIGINS")
                .unwrap_or_else(|_| "http://localhost:3000".into())
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            gptimage_base: env::var("GPTIMAGE_BASE")
                .or_else(|_| env::var("GATEWAY_BASE"))
                .unwrap_or_else(|_| "http://127.0.0.1:8014".into()),
            gptimage_admin_token: env::var("GPTIMAGE_ADMIN_TOKEN").ok(),
            account_ops_base: env::var("ACCOUNT_OPS_BASE")
                .unwrap_or_else(|_| "http://127.0.0.1:9011".into()),
            account_ops_token: env::var("ACCOUNT_OPS_TOKEN")
                .ok()
                .or_else(|| env::var("HELPER_INTERNAL_TOKEN").ok()),
            grok2api_base: env::var("GROK2API_BASE")
                .unwrap_or_else(|_| "http://127.0.0.1:18000".into()),
            director_model: env::var("DIRECTOR_MODEL").unwrap_or_else(|_| "gpt-5".into()),
            r2,
            bootstrap_admin_email: env::var("BOOTSTRAP_ADMIN_EMAIL").ok(),
            bootstrap_admin_password: env::var("BOOTSTRAP_ADMIN_PASSWORD").ok(),
            bootstrap_demo_email: env::var("BOOTSTRAP_DEMO_EMAIL").ok(),
            bootstrap_demo_password: env::var("BOOTSTRAP_DEMO_PASSWORD").ok(),
            presign_ttl_secs: env::var("PRESIGN_TTL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1800),
            gateway_base: env::var("GATEWAY_BASE")
                .or_else(|_| env::var("GPTIMAGE_GATEWAY_BASE"))
                .unwrap_or_else(|_| "http://127.0.0.1:8014".into()),
            gateway_internal_token: env::var("GATEWAY_AUTH_KEY")
                .ok()
                .or_else(|| env::var("GATEWAY_INTERNAL_TOKEN").ok()),
        })
    }
}

fn env_required(key: &str) -> Result<String> {
    env::var(key).with_context(|| format!("missing env {key}"))
}

pub const JOB_QUEUE_KEY: &str = "tnexus:jobs";
pub const JOB_EVENTS_PREFIX: &str = "tnexus:job_events:";
