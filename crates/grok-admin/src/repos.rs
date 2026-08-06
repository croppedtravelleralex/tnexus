//! 持久化抽象（对齐 Go `repository.AdminRepository` / `AdminSessionRepository` / `RateLimiter`）。
//!
//! Rust 用 trait + `Option`（None = 未找到）区分「记录不存在」与「运行时错误」，
//! 使 Service 能把 NotFound 映射为 `InvalidCredentials` / `InvalidSession`，其它错误
//! 映射为 `RuntimeUnavailable`（对齐 Go 的 `errors.Is(err, repository.ErrNotFound)`）。

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::domain::{Admin, Session};
use crate::error::AdminResult;

/// 管理员账号仓库（Go `AdminRepository`）。
#[async_trait]
pub trait AdminRepository: Send + Sync {
    /// 管理员总数（Go `Count`）。
    async fn count(&self) -> AdminResult<i64>;
    /// 创建管理员（Go `Create`）。
    async fn create(&self, admin: Admin) -> AdminResult<Admin>;
    /// 按用户名查（Go `GetByUsername`；None = 未找到）。
    async fn get_by_username(&self, username: &str) -> AdminResult<Option<Admin>>;
    /// 按 ID 查（Go `GetByID`；None = 未找到）。
    async fn get_by_id(&self, id: i64) -> AdminResult<Option<Admin>>;
    /// 改密码并撤销该管理员的全部 refresh session（Go `UpdatePasswordAndRevokeSessions`）。
    async fn update_password_and_revoke_sessions(
        &self,
        admin_id: i64,
        password_hash: &str,
    ) -> AdminResult<()>;
}

/// 管理员刷新会话仓库（Go `AdminSessionRepository`）。
#[async_trait]
pub trait AdminSessionRepository: Send + Sync {
    /// 按 token hash 查（Go `GetByTokenHash`；None = 未找到）。
    async fn get_by_token_hash(&self, token_hash: &str) -> AdminResult<Option<Session>>;
    /// 按 ID 查（Go `GetByID`；None = 未找到）。
    async fn get_by_id(&self, id: i64) -> AdminResult<Option<Session>>;
    /// 创建会话（Go `Create`）。
    async fn create(&self, session: Session) -> AdminResult<Session>;
    /// 轮换 refresh token（Go `Rotate`）。`Ok(true)` = 成功；`Ok(false)` = 冲突/不存在
    /// （hash 已被轮换），Service 映射为 `ErrInvalidSession`。
    async fn rotate(
        &self,
        session_id: i64,
        old_token_hash: &str,
        new_token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AdminResult<bool>;
    /// 撤销会话（Go `Revoke`；不存在视为成功）。
    async fn revoke(&self, id: i64) -> AdminResult<()>;
}

/// 登录限流器（Go `repository.RateLimiter`）。
#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// 是否允许本次请求（Go `Allow`）。
    async fn allow(&self, key: &str, limit: i32, now: DateTime<Utc>) -> AdminResult<bool>;
}

/// 固定窗口内存限流器（对齐 Go 内存 RateLimiter；多实例需换 Redis，见 docs/39g）。
pub struct MemoryRateLimiter {
    window: Duration,
    buckets: Mutex<HashMap<String, (DateTime<Utc>, i32)>>,
}

impl MemoryRateLimiter {
    /// `window`：窗口时长（默认建议 15 分钟，对齐 Go `checkLoginRate` 窗口）。
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            buckets: Mutex::new(HashMap::new()), // Mutex 无 Default → 显式构造
        }
    }
}

impl Default for MemoryRateLimiter {
    fn default() -> Self {
        Self::new(Duration::minutes(15))
    }
}

#[async_trait]
impl RateLimiter for MemoryRateLimiter {
    async fn allow(&self, key: &str, limit: i32, now: DateTime<Utc>) -> AdminResult<bool> {
        let mut buckets = self.buckets.lock().unwrap();
        let entry = buckets.entry(key.to_string()).or_insert((now, 0));
        if now - entry.0 >= self.window {
            *entry = (now, 0);
        }
        if entry.1 >= limit {
            return Ok(false);
        }
        entry.1 += 1;
        Ok(true)
    }
}
