//! 管理员认证编排（对齐 Go `adminauth.Service`）。
//!
//! 编排单管理员登录、JWT access token 与可撤销 refresh session 生命周期。
//! IO 通过 [`crate::repos`] trait 注入（测试用内存 fake；真实 SQL 由 grok-storage 提供）。

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};

use crate::domain::{Admin, Session};
use crate::error::{AdminError, AdminResult};
use crate::repos::{AdminRepository, AdminSessionRepository, MemoryRateLimiter, RateLimiter};
use crate::security::{
    hash_password, hash_token, new_opaque_token, verify_password, AdminTokenIdentity, TokenService,
};

/// 登录对照组：防时序的虚拟密码哈希（Go `grok2api-invalid-admin-password`）。
const DUMMY_PASSWORD: &str = "grok2api-invalid-admin-password";

/// 一对 access/refresh token（Go `adminauth.Tokens`）。
#[derive(Debug, Clone)]
pub struct Tokens {
    pub access_token: String,
    pub access_token_expires_at: DateTime<Utc>,
    pub refresh_token: String,
    pub refresh_token_expires_at: DateTime<Utc>,
}

/// 管理员认证服务。
pub struct AdminAuthService {
    admins: Arc<dyn AdminRepository>,
    sessions: Arc<dyn AdminSessionRepository>,
    tokens: TokenService,
    access_ttl: Duration,
    refresh_ttl: Duration,
    login_limiter: Option<Arc<dyn RateLimiter>>,
    dummy_password_hash: String,
}

impl AdminAuthService {
    pub fn new(
        admins: Arc<dyn AdminRepository>,
        sessions: Arc<dyn AdminSessionRepository>,
        tokens: TokenService,
        access_ttl: Duration,
        refresh_ttl: Duration,
    ) -> Self {
        let dummy_password_hash = hash_password(DUMMY_PASSWORD).unwrap_or_default();
        Self {
            admins,
            sessions,
            tokens,
            access_ttl,
            refresh_ttl,
            login_limiter: Some(Arc::new(MemoryRateLimiter::default())),
            dummy_password_hash,
        }
    }

    pub fn set_login_rate_limiter(&mut self, limiter: Arc<dyn RateLimiter>) {
        self.login_limiter = Some(limiter);
    }

    /// 解析 access token 身份（guard 层复用；Go `TokenService.ParseAccessToken`）。
    pub fn parse_access_token(&self, raw: &str) -> AdminResult<AdminTokenIdentity> {
        self.tokens.parse_access_token(raw)
    }

    /// 首次启动在没有管理员时创建唯一管理员（Go `Bootstrap`）。
    pub async fn bootstrap(&self, username: &str, password: &str) -> AdminResult<()> {
        let count = self.admins.count().await?;
        if count > 0 {
            return Ok(());
        }
        let username = username.trim();
        if username.is_empty() || password.len() < 8 {
            return Err(AdminError::BootstrapRequired);
        }
        let hash = hash_password(password)?;
        let now = Utc::now();
        self.admins
            .create(Admin::new(0, username.to_string(), hash, now))
            .await?;
        Ok(())
    }

    /// 校验密码并创建新的可撤销 refresh session（Go `Login`）。
    pub async fn login(
        &self,
        username: &str,
        password: &str,
        remote_address: &str,
    ) -> AdminResult<(Admin, Tokens)> {
        let username = username.trim().to_string();
        self.check_login_rate(&username, remote_address).await?;
        let value = match self.admins.get_by_username(&username).await {
            Ok(Some(admin)) => admin,
            Ok(None) => {
                // 即使是未知用户也执行一次 dummy verify，避免时序侧信道。
                let _ = verify_password(&self.dummy_password_hash, password);
                return Err(AdminError::InvalidCredentials);
            }
            Err(e) => return Err(e),
        };
        if !verify_password(&value.password_hash, password) {
            return Err(AdminError::InvalidCredentials);
        }
        let (tokens, _session) = self.create_session(value.id).await?;
        Ok((value, tokens))
    }

    /// 轮换 refresh token，旧 token 立即失效（Go `Refresh`）。
    pub async fn refresh(&self, raw_refresh_token: &str) -> AdminResult<Tokens> {
        let token_hash = hash_token(raw_refresh_token);
        let session = match self.sessions.get_by_token_hash(&token_hash).await {
            Ok(Some(s)) => s,
            Ok(None) => return Err(AdminError::InvalidSession),
            Err(e) => return Err(e),
        };
        if !session.not_expired(Utc::now()) {
            return Err(AdminError::InvalidSession);
        }
        match self.admins.get_by_id(session.admin_id).await {
            Ok(None) => return Err(AdminError::InvalidSession),
            Ok(Some(_)) => {}
            Err(e) => return Err(e),
        }
        let (access_token, access_expires_at) =
            self.tokens
                .create_access_token(session.admin_id, session.id, self.access_ttl)?;
        let refresh_token = new_opaque_token(32)?;
        let refresh_expires_at = Utc::now() + self.refresh_ttl;
        match self
            .sessions
            .rotate(
                session.id,
                &token_hash,
                &hash_token(&refresh_token),
                refresh_expires_at,
            )
            .await?
        {
            true => Ok(Tokens {
                access_token,
                access_token_expires_at: access_expires_at,
                refresh_token,
                refresh_token_expires_at: refresh_expires_at,
            }),
            false => Err(AdminError::InvalidSession),
        }
    }

    /// 撤销当前 refresh session（Go `Logout`；不存在视为成功）。
    pub async fn logout(&self, raw_refresh_token: &str) -> AdminResult<()> {
        let token_hash = hash_token(raw_refresh_token);
        let session = match self.sessions.get_by_token_hash(&token_hash).await {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(()),
            Err(e) => return Err(e),
        };
        self.sessions.revoke(session.id).await
    }

    /// 校验 access token 并读取管理员（Go `AuthenticateAccess`）。
    pub async fn authenticate_access(&self, raw_access_token: &str) -> AdminResult<Admin> {
        let identity = match self.tokens.parse_access_token(raw_access_token) {
            Ok(id) => id,
            Err(_) => return Err(AdminError::InvalidSession),
        };
        let session = match self.sessions.get_by_id(identity.session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => return Err(AdminError::InvalidSession),
            Err(e) => return Err(e),
        };
        if session.admin_id != identity.admin_id || !session.not_expired(Utc::now()) {
            return Err(AdminError::InvalidSession);
        }
        match self.admins.get_by_id(identity.admin_id).await {
            Ok(Some(admin)) => Ok(admin),
            Ok(None) => Err(AdminError::InvalidSession),
            Err(e) => Err(e),
        }
    }

    /// 修改密码并撤销管理员的全部 refresh session（Go `ChangePassword`）。
    pub async fn change_password(
        &self,
        admin_id: i64,
        current_password: &str,
        new_password: &str,
    ) -> AdminResult<()> {
        if new_password.len() < 8 {
            return Err(AdminError::InvalidPassword);
        }
        let value = match self.admins.get_by_id(admin_id).await {
            Ok(Some(admin)) => admin,
            Ok(None) => return Err(AdminError::InvalidCredentials),
            Err(e) => return Err(e),
        };
        if !verify_password(&value.password_hash, current_password) {
            return Err(AdminError::InvalidCredentials);
        }
        let hash = hash_password(new_password)?;
        self.admins
            .update_password_and_revoke_sessions(admin_id, &hash)
            .await
    }

    /// 生成 access + refresh token 并落库会话（Go `createSession`）。
    async fn create_session(&self, admin_id: i64) -> AdminResult<(Tokens, Session)> {
        let refresh_token = random_refresh_token()?;
        let refresh_expires_at = Utc::now() + self.refresh_ttl;
        let session = self
            .sessions
            .create(Session::new(
                0,
                admin_id,
                hash_token(&refresh_token),
                refresh_expires_at,
                Utc::now(),
            ))
            .await?;
        let access = match self
            .tokens
            .create_access_token(admin_id, session.id, self.access_ttl)
        {
            Ok(v) => v,
            Err(e) => {
                let _ = self.sessions.revoke(session.id).await;
                return Err(e);
            }
        };
        Ok((
            Tokens {
                access_token: access.0,
                access_token_expires_at: access.1,
                refresh_token,
                refresh_token_expires_at: refresh_expires_at,
            },
            session,
        ))
    }

    /// 登录限流（Go `checkLoginRate`）：IP 30 次 / 用户 12 次 / 窗口。
    async fn check_login_rate(&self, username: &str, remote_address: &str) -> AdminResult<()> {
        let Some(limiter) = &self.login_limiter else {
            return Ok(());
        };
        let now = Utc::now();
        let keys = [
            (
                format!("admin-login:ip:{}", hash_token(remote_address.trim())),
                30,
            ),
            (
                format!("admin-login:user:{}", hash_token(&username.to_lowercase())),
                12,
            ),
        ];
        for (key, limit) in keys {
            let allowed = limiter.allow(&key, limit, now).await?;
            if !allowed {
                return Err(AdminError::LoginRateLimited);
            }
        }
        Ok(())
    }
}

fn random_refresh_token() -> AdminResult<String> {
    new_opaque_token(32)
}

#[cfg(test)]
mod inner_tests {
    use super::*;

    #[test]
    fn dummy_password_hash_is_precomputed() {
        let s = AdminAuthService::new(
            Arc::new(NoopAdmins),
            Arc::new(NoopSessions),
            TokenService::new("x".repeat(32).as_str()),
            Duration::minutes(1),
            Duration::hours(1),
        );
        assert!(!s.dummy_password_hash.is_empty());
    }

    struct NoopAdmins;
    #[async_trait::async_trait]
    impl AdminRepository for NoopAdmins {
        async fn count(&self) -> AdminResult<i64> {
            Ok(0)
        }
        async fn create(&self, _a: Admin) -> AdminResult<Admin> {
            Err(AdminError::RuntimeUnavailable("noop".into()))
        }
        async fn get_by_username(&self, _u: &str) -> AdminResult<Option<Admin>> {
            Ok(None)
        }
        async fn get_by_id(&self, _i: i64) -> AdminResult<Option<Admin>> {
            Ok(None)
        }
        async fn update_password_and_revoke_sessions(&self, _i: i64, _h: &str) -> AdminResult<()> {
            Ok(())
        }
    }
    struct NoopSessions;
    #[async_trait::async_trait]
    impl AdminSessionRepository for NoopSessions {
        async fn get_by_token_hash(&self, _h: &str) -> AdminResult<Option<Session>> {
            Ok(None)
        }
        async fn get_by_id(&self, _i: i64) -> AdminResult<Option<Session>> {
            Ok(None)
        }
        async fn create(&self, _s: Session) -> AdminResult<Session> {
            Err(AdminError::RuntimeUnavailable("noop".into()))
        }
        async fn rotate(
            &self,
            _i: i64,
            _o: &str,
            _n: &str,
            _e: DateTime<Utc>,
        ) -> AdminResult<bool> {
            Ok(true)
        }
        async fn revoke(&self, _i: i64) -> AdminResult<()> {
            Ok(())
        }
    }
}
