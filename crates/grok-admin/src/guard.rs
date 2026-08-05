//! HTTP 认证 guard 形态（供后续 Admin API 端点接线）。
//!
//! 接收 `Authorization: Bearer <token>` 头 → 校验 access token → 返回管理员与会话。
//! 纯函数 [`bearer_token`] 可单独测试 / 复用；[`authenticate_bearer`] 是
//! guard 组合（对齐 Go `AuthenticateAccess` 的语义，HTTP 层在 Go 侧由中间件调用）。

use crate::domain::Admin;
use crate::error::{AdminError, AdminResult};
use crate::service::AdminAuthService;

/// 解析 `Authorization: Bearer <token>`（大小写不敏感；无 / 非 Bearer → None）。
pub fn bearer_token(header: &str) -> Option<&str> {
    let (scheme, token) = header.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(token)
}

/// 认证 guard：解析 Bearer 头 → 校验 access token → 读取管理员与会话。
pub async fn authenticate_bearer(
    service: &AdminAuthService,
    authorization_header: &str,
) -> AdminResult<AuthContext> {
    let token = bearer_token(authorization_header).ok_or(AdminError::InvalidSession)?;
    let admin = service.authenticate_access(token).await?;
    let identity = service
        .parse_access_token(token)
        .map_err(|_| AdminError::InvalidSession)?;
    Ok(AuthContext {
        admin,
        session_id: identity.session_id,
    })
}

/// 认证上下文（admin + session）。
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub admin: Admin,
    pub session_id: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_token_parsing() {
        assert_eq!(bearer_token("Bearer abc.def"), Some("abc.def"));
        assert_eq!(bearer_token("bearer  abc.def "), Some("abc.def"));
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token("Bearer"), None);
        assert_eq!(bearer_token(""), None);
        assert_eq!(bearer_token("Bearer "), None);
    }
}