//! Admin 领域类型（对齐 Go `domain/admin`）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 系统唯一管理员（Go `admin.Admin`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admin {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 可轮换和删除的管理员刷新会话（Go `admin.Session`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub admin_id: i64,
    /// refresh token 的 SHA-256 hex 摘要（明文不落库）。
    pub refresh_token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Admin {
    pub fn new(id: i64, username: String, password_hash: String, now: DateTime<Utc>) -> Self {
        Self {
            id,
            username,
            password_hash,
            created_at: now,
            updated_at: now,
        }
    }
}

impl Session {
    pub fn new(
        id: i64,
        admin_id: i64,
        refresh_token_hash: String,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            admin_id,
            refresh_token_hash,
            expires_at,
            last_used_at: None,
            created_at: now,
        }
    }

    /// 是否未过期（Go `time.Now().UTC().Before(ExpiresAt)`）。
    pub fn not_expired(&self, now: DateTime<Utc>) -> bool {
        now < self.expires_at
    }
}
