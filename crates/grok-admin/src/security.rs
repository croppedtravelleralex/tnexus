//! 安全原语（对齐 Go `infra/security`）：
//! - [`TokenService`]：HS256 管理员 JWT 签发 / 校验（claims = adminId/sessionId/iss/sub/iat/exp）
//! - [`hash_token`] / [`new_opaque_token`]：refresh token 的 SHA-256 摘要与随机令牌
//! - [`hash_password`] / [`verify_password`]：bcrypt 密码哈希

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AdminError, AdminResult};

/// JWT issuer（对齐 Go `NewTokenService` 的 `"grok2api"`）。
pub const TOKEN_ISSUER: &str = "grok2api";

/// access token claims（对齐 Go `adminClaims`，字段名 camelCase 保持跨实现兼容）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminClaims {
    #[serde(rename = "adminId")]
    pub admin_id: i64,
    #[serde(rename = "sessionId")]
    pub session_id: i64,
    pub iss: String,
    pub sub: String,
    pub iat: i64,
    pub exp: i64,
}

/// 解析出的令牌身份（Go `AdminTokenIdentity`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminTokenIdentity {
    pub admin_id: i64,
    pub session_id: i64,
}

/// 管理员 access token 服务（Go `security.TokenService`）。
pub struct TokenService {
    secret: Vec<u8>,
    issuer: String,
}

impl TokenService {
    pub fn new(secret: &str) -> Self {
        Self {
            secret: secret.as_bytes().to_vec(),
            issuer: TOKEN_ISSUER.to_string(),
        }
    }

    /// 创建短期管理员 JWT（Go `CreateAccessToken`）。
    pub fn create_access_token(
        &self,
        admin_id: i64,
        session_id: i64,
        ttl: Duration,
    ) -> AdminResult<(String, DateTime<Utc>)> {
        let now = Utc::now();
        let expires_at = now + ttl;
        let claims = AdminClaims {
            admin_id,
            session_id,
            iss: self.issuer.clone(),
            sub: admin_id.to_string(),
            iat: now.timestamp(),
            exp: expires_at.timestamp(),
        };
        let signed = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(&self.secret),
        )
        .map_err(|e| AdminError::Token(e.to_string()))?;
        Ok((signed, expires_at))
    }

    /// 校验管理员 JWT（Go `ParseAccessToken`）：HS256 + issuer + 未过期 + 非零 ID。
    pub fn parse_access_token(&self, raw: &str) -> AdminResult<AdminTokenIdentity> {
        let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.set_issuer(&[&self.issuer]);
        validation.leeway = 0; // 对齐 Go jwt/v5 默认：exp 严格校验，无宽限
        let token = decode::<AdminClaims>(
            raw,
            &DecodingKey::from_secret(&self.secret),
            &validation,
        )
        .map_err(|_| AdminError::InvalidSession)?;
        let claims = token.claims;
        if claims.admin_id == 0 || claims.session_id == 0 {
            return Err(AdminError::InvalidSession);
        }
        Ok(AdminTokenIdentity {
            admin_id: claims.admin_id,
            session_id: claims.session_id,
        })
    }
}

/// 不可逆 SHA-256 十六进制摘要（Go `HashToken`）。
pub fn hash_token(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

/// 随机不透明令牌，base64url 无填充（Go `NewOpaqueToken`）。
pub fn new_opaque_token(bytes_length: usize) -> AdminResult<String> {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes_length];
    rand::thread_rng().fill_bytes(&mut buf);
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// bcrypt 密码哈希（Go `HashPassword`）。
pub fn hash_password(password: &str) -> AdminResult<String> {
    bcrypt::hash(password, bcrypt::DEFAULT_COST).map_err(|e| AdminError::Password(e.to_string()))
}

/// 校验密码（Go `VerifyPassword`）。
pub fn verify_password(hash: &str, password: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_token_is_sha256_hex() {
        let h = hash_token("abc");
        assert_eq!(h.len(), 64);
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_ne!(h, hash_token("abd"));
    }

    #[test]
    fn opaque_token_is_random_and_url_safe() {
        let a = new_opaque_token(32).unwrap();
        let b = new_opaque_token(32).unwrap();
        assert_ne!(a, b);
        assert!(!a.contains('+') && !a.contains('/'));
    }

    #[test]
    fn password_round_trip_and_mismatch() {
        let hash = hash_password("password123").unwrap();
        assert!(verify_password(&hash, "password123"));
        assert!(!verify_password(&hash, "wrong"));
    }
}