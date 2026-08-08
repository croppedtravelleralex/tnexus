//! 服务端 grok-admin JWT（TNexus 代理用；前端不再持 token）。

use crate::config::AppConfig;
use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct GrokAdminClient {
    http: Client,
    base: String,
    username: String,
    password: String,
    cache: Arc<Mutex<Option<CachedToken>>>,
}

#[derive(Clone)]
struct CachedToken {
    access: String,
    expires_at: Instant,
}

#[derive(Deserialize)]
struct LoginResponse {
    tokens: TokenPair,
}

#[derive(Deserialize)]
struct TokenPair {
    access_token: String,
}

impl GrokAdminClient {
    pub fn from_config(config: &AppConfig, http: Client) -> Option<Self> {
        let password = config.grok_admin_password.as_ref()?;
        let username = config
            .grok_admin_username
            .clone()
            .unwrap_or_else(|| "admin".into());
        Some(Self {
            http,
            base: config.grok_admin_base.trim_end_matches('/').to_string(),
            username,
            password: password.clone(),
            cache: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn access_token(&self) -> Result<String> {
        {
            let guard = self.cache.lock().await;
            if let Some(c) = guard.as_ref() {
                if Instant::now() < c.expires_at {
                    return Ok(c.access.clone());
                }
            }
        }
        let token = self.login().await?;
        let mut guard = self.cache.lock().await;
        *guard = Some(CachedToken {
            access: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(50 * 60),
        });
        Ok(token)
    }

    pub fn invalidate(&self) {
        if let Ok(mut guard) = self.cache.try_lock() {
            *guard = None;
        }
    }

    async fn login(&self) -> Result<String> {
        let url = format!("{}/admin/auth/login", self.base);
        let res = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "username": self.username,
                "password": self.password,
            }))
            .send()
            .await
            .with_context(|| format!("grok-admin login request failed: {url}"))?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(anyhow!("grok-admin login {status}: {body}"));
        }
        let parsed: LoginResponse = res.json().await.context("parse grok-admin login")?;
        Ok(parsed.tokens.access_token)
    }

    pub fn base(&self) -> &str {
        &self.base
    }
}
