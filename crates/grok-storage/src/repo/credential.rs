//! 凭据只读仓储（grok_credentials）。
//!
//! G0 只提供 SELECT 密文原样通道；解密逻辑不入本 crate（凭据加密密钥在
//! ETL / 启动期单独治理，docs/39b §1）。
//!
//! 列名对齐 Go `account_credentials` 的
//! `encrypted_primary` / `encrypted_refresh`（迁移 010 骨架扩展为 Go parity 后
//! 即为最终列名，见 39e / 39b §3 表 4）。

use async_trait::async_trait;
use sqlx::{PgPool, Row};

use crate::StorageError;

/// 凭据只读 repository（G0）。
#[async_trait]
pub trait CredentialRepository {
    /// 返回账号访问令牌密文（原样字节，AES-GCM 密文，不解密）。
    async fn get(&self, account_id: i64) -> Result<Vec<u8>, StorageError>;

    /// 存在性 + refresh 到期判断（只读，供刷新调度判断）。
    /// 返回 (access_ciphertext, Option<refresh_ciphertext>, Option<refresh_due_at>)。
    async fn refresh_due(
        &self,
        account_id: i64,
    ) -> Result<
        Option<(
            Vec<u8>,
            Option<Vec<u8>>,
            Option<chrono::DateTime<chrono::Utc>>,
        )>,
        StorageError,
    >;
}

/// PG 凭据只读实现。
pub struct PgCredentialRepository {
    pool: PgPool,
}

impl PgCredentialRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CredentialRepository for PgCredentialRepository {
    async fn get(&self, account_id: i64) -> Result<Vec<u8>, StorageError> {
        let row =
            sqlx::query("SELECT encrypted_primary FROM grok_credentials WHERE account_id = $1")
                .bind(account_id)
                .fetch_optional(&self.pool)
                .await?;
        let Some(row) = row else {
            return Err(StorageError::NotFound(format!("credential {account_id}")));
        };
        row.try_get::<Vec<u8>, _>("encrypted_primary")
            .map_err(StorageError::from)
    }

    async fn refresh_due(
        &self,
        account_id: i64,
    ) -> Result<
        Option<(
            Vec<u8>,
            Option<Vec<u8>>,
            Option<chrono::DateTime<chrono::Utc>>,
        )>,
        StorageError,
    > {
        let row = sqlx::query(
            "SELECT encrypted_primary, encrypted_refresh, refresh_due_at \
             FROM grok_credentials WHERE account_id = $1",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let primary: Vec<u8> = row.try_get("encrypted_primary")?;
        let refresh: Option<Vec<u8>> = row.try_get("encrypted_refresh")?;
        let due: Option<chrono::DateTime<chrono::Utc>> = row.try_get("refresh_due_at")?;
        Ok(Some((primary, refresh, due)))
    }
}
